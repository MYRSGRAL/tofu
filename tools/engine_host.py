"""Linux host: настоящие ядра, HTTPS/SOCKS, ошибки и отмена в отдельной сети."""

from __future__ import annotations

import argparse
import hashlib
import http.server
import json
import logging
import os
import selectors
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from engine_compile import UUID, stop, wait_listener
from engine_smoke import install_signal_handlers, links, run_sandbox

LOG = logging.getLogger("yonik.host")
ROOT = Path(__file__).resolve().parents[1]
PROFILE = f"vless://{UUID}@127.0.0.1:24443"


class HttpsHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self.server.requested.set()
        if self.path == "/slow":
            time.sleep(3)
        status = {"/ok200": 200, "/status": 503, "/redirect": 302}.get(self.path, 204)
        self.send_response(status)
        if status == 302:
            self.send_header("Location", "https://redirect.invalid/")
        self.send_header("Content-Length", "0")
        try:
            self.end_headers()
        except (BrokenPipeError, ConnectionResetError, ssl.SSLEOFError):
            LOG.debug("Тестовый клиент закрыл соединение после cancel/crash")

    def log_message(self, _format: str, *args: object) -> None:
        pass


@contextmanager
def https_server(
    workspace: Path, name: str, port: int, san: str
) -> Iterator[http.server.ThreadingHTTPServer]:
    key = workspace / f"{name}.key"
    certificate = workspace / f"{name}.pem"
    ca = workspace / f"{name}.ca.pem"
    ca_key = workspace / f"{name}.ca.key"
    csr = workspace / f"{name}.csr"
    extensions = workspace / f"{name}.extensions"
    extensions.write_text(
        f"basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:{san}\n"
    )
    for command in [
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=yonik-test-ca",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
            "-keyout",
            str(ca_key),
            "-out",
            str(ca),
        ],
        [
            "openssl",
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=yonik-test",
            "-keyout",
            str(key),
            "-out",
            str(csr),
        ],
        [
            "openssl",
            "x509",
            "-req",
            "-in",
            str(csr),
            "-CA",
            str(ca),
            "-CAkey",
            str(ca_key),
            "-CAcreateserial",
            "-days",
            "1",
            "-extfile",
            str(extensions),
            "-out",
            str(certificate),
        ],
    ]:
        subprocess.run(command, check=True, capture_output=True, timeout=10)
    with http.server.ThreadingHTTPServer(("127.0.0.1", port), HttpsHandler) as server:
        server.requested = threading.Event()
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(certificate, key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(
            target=server.serve_forever, kwargs={"poll_interval": 0.05}
        )
        thread.start()
        try:
            yield server
        finally:
            server.shutdown()
            thread.join(timeout=3)
            if thread.is_alive():
                raise RuntimeError("HTTPS server не завершился")


def arguments(
    host: Path,
    engine: str,
    binary: Path,
    endpoint: str = "https://127.0.0.1:24444/",
    once: bool = True,
) -> list[str]:
    command = [
        str(host),
        engine,
        "--binary",
        str(binary),
        "--sha256",
        hashlib.sha256(binary.read_bytes()).hexdigest(),
        "--check-url",
        endpoint,
        "--check-timeout",
        "1",
    ]
    return command + (["--once"] if once else [])


def environment(
    workspace: Path, trusted: bool = True, mode: str = "empty-roots"
) -> dict[str, str]:
    env = dict(os.environ)
    env.update(
        SSL_CERT_FILE=str(workspace / "roots.pem") if trusted else "/dev/null",
        SSL_CERT_DIR=str(workspace / mode),
        HTTP_PROXY="http://127.0.0.1:9",
        HTTPS_PROXY="http://127.0.0.1:9",
        ALL_PROXY="http://127.0.0.1:9",
        NO_PROXY="*",
    )
    return env


def once(
    command: list[str],
    env: dict[str, str],
    expected: str,
    code: int,
    status: int | None = None,
    kind: str | None = None,
    partial: bool = False,
) -> None:
    source = PROFILE.encode() + (b"\nunsupported://synthetic" if partial else b"")
    result = subprocess.run(
        command, input=source, env=env, capture_output=True, timeout=15
    )
    events = [json.loads(line) for line in result.stdout.splitlines()]
    if result.returncode != code or expected not in [
        event.get("state") for event in events
    ]:
        raise RuntimeError(
            f"Неверный результат host: {result.returncode}, {events}, {result.stderr!r}"
        )
    if events[-1]["state"] != "Disconnected":
        raise RuntimeError("Cleanup не подтверждён")
    if status is not None and not any(
        event.get("probe", {}).get("status") == status for event in events
    ):
        raise RuntimeError("HTTP status потерян или выполнен redirect")
    if kind is not None and not any(
        event.get("probe", {}).get("kind") == kind for event in events
    ):
        raise RuntimeError("Потерян тип ошибки HTTPS проверки")
    for event in events:
        if event.get("pid") and Path(f"/proc/{event['pid']}").exists():
            raise RuntimeError("После --once остался процесс ядра")


class Events:
    def __init__(self, process: subprocess.Popen[bytes]) -> None:
        if process.stdout is None:
            raise RuntimeError("Нет stdout host")
        self.stdout = process.stdout
        self.buffer = bytearray()

    def until(self, state: str) -> dict[str, object]:
        deadline = time.monotonic() + 10
        with selectors.DefaultSelector() as selector:
            selector.register(self.stdout, selectors.EVENT_READ)
            while time.monotonic() < deadline:
                while b"\n" in self.buffer:
                    line, _, rest = self.buffer.partition(b"\n")
                    self.buffer = bytearray(rest)
                    event = json.loads(line)
                    if event["state"] == state:
                        return event
                if not selector.select(timeout=0.2):
                    continue
                chunk = os.read(self.stdout.fileno(), 4096)
                if not chunk:
                    break
                self.buffer.extend(chunk)
        raise RuntimeError(f"Не получено состояние {state}")


def signal_runtime(
    command: list[str],
    env: dict[str, str],
    https: http.server.ThreadingHTTPServer,
    crash: bool,
    phase: str = "Checking",
    interrupt: bool = False,
) -> None:
    https.requested.clear()
    with subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        start_new_session=True,
    ) as host:
        try:
            host.stdin.write(PROFILE.encode())
            host.stdin.close()
            host.stdin = None
            reader = Events(host)
            event = reader.until("Checking")
            pid = int(event["pid"])
            if phase != "Checking":
                reader.until(phase)
            if crash:
                if not https.requested.wait(timeout=5):
                    raise RuntimeError("HTTPS не начался")
                os.kill(pid, signal.SIGKILL)
            else:
                if interrupt:
                    os.killpg(host.pid, signal.SIGINT)
                else:
                    host.send_signal(signal.SIGTERM)
            stdout, _ = host.communicate(timeout=10)
            states = [json.loads(line)["state"] for line in stdout.splitlines()]
            if host.returncode != (1 if crash else 0) or "Connected" in states:
                raise RuntimeError("Exit/cancel ошибочно подтвердил Connected")
            if not crash and "Disconnected" not in states:
                raise RuntimeError("SIGTERM не подтвердил cleanup")
            if Path(f"/proc/{pid}").exists():
                raise RuntimeError("Процесс ядра не reap после exit/cancel")
        finally:
            stop(host)


def fake_preparation(host: Path, fake: Path, workspace: Path) -> None:
    for mode, stage in [("version-hang", "version"), ("check-hang", "-test")]:
        command = arguments(host, "xray", fake)
        with subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment(workspace, mode=mode),
        ) as process:
            try:
                process.stdin.write(PROFILE.encode())
                process.stdin.close()
                process.stdin = None
                Events(process).until("Preparing")
                deadline = time.monotonic() + 5
                owned = None
                while time.monotonic() < deadline:
                    children = (
                        Path(f"/proc/{process.pid}/task/{process.pid}/children")
                        .read_text()
                        .split()
                    )
                    for child in children:
                        try:
                            args = (
                                Path(f"/proc/{child}/cmdline")
                                .read_bytes()
                                .split(b"\x00")
                            )
                        except FileNotFoundError:
                            continue
                        if stage.encode() in args:
                            owned = child
                            break
                    if owned:
                        break
                    time.sleep(0.02)
                if owned is None:
                    raise RuntimeError(
                        "Fixture не дошёл до ожидаемого subprocess этапа"
                    )
                process.send_signal(signal.SIGTERM)
                process.communicate(timeout=5)
                if process.returncode != 0 or Path(f"/proc/{owned}").exists():
                    raise RuntimeError("Отмена подготовки не выполнила kill+wait")
            finally:
                stop(process)
    result = subprocess.run(
        arguments(host, "xray", fake),
        input=PROFILE.encode(),
        env=environment(workspace, mode="version-hang"),
        capture_output=True,
        timeout=10,
    )
    if result.returncode != 1:
        raise RuntimeError("Timeout version не завершил host")
    LOG.info(
        "Подготовка: SIGTERM во время version/config-check; bounded version timeout"
    )


@contextmanager
def peer(
    binary: Path,
    engine: str,
    workspace: Path,
    port: int = 24443,
    target_ports: str = "24444-24445",
) -> Iterator[None]:
    if engine == "xray":
        config = {
            "log": {"loglevel": "none"},
            "inbounds": [
                {
                    "listen": "127.0.0.1",
                    "port": port,
                    "protocol": "vless",
                    "settings": {"decryption": "none", "clients": [{"id": UUID}]},
                }
            ],
            "outbounds": [
                {
                    "protocol": "freedom",
                    "settings": {
                        "finalRules": [
                            {
                                "action": "allow",
                                "ip": ["127.0.0.1/32"],
                                "port": target_ports,
                            }
                        ]
                    },
                }
            ],
        }
    else:
        config = {
            "log": {"disabled": True},
            "inbounds": [
                {
                    "type": "vless",
                    "listen": "127.0.0.1",
                    "listen_port": port,
                    "users": [{"uuid": UUID}],
                }
            ],
            "outbounds": [{"type": "direct"}],
        }
    path = workspace / f"{engine}-peer.json"
    path.write_text(json.dumps(config))
    with subprocess.Popen(
        [str(binary), "run", "-c", str(path)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ) as process:
        try:
            wait_listener(process, port)
            yield
        finally:
            stop(process)


def worker(args: argparse.Namespace) -> None:
    if os.stat("/proc/self/ns/net").st_ino == args.worker or links() != {"lo"}:
        raise RuntimeError("Нужен новый изолированный network namespace")
    subprocess.run(
        ["ip", "link", "set", "lo", "up"], check=True, capture_output=True, timeout=5
    )
    workspace = args.workspace
    (workspace / "empty-roots").mkdir()
    with (
        https_server(workspace, "good", 24444, "127.0.0.1") as https,
        https_server(workspace, "wrong", 24445, "127.0.0.2"),
    ):
        (workspace / "roots.pem").write_bytes(
            (workspace / "good.ca.pem").read_bytes()
            + (workspace / "wrong.ca.pem").read_bytes()
        )
        for engine, binary in [("xray", args.xray), ("sing-box", args.sing_box)]:
            env = environment(workspace)
            with peer(binary, engine, workspace):
                once(arguments(args.host, engine, binary), env, "Connected", 0, 204)
                once(
                    arguments(args.host, engine, binary)
                    + ["--accept-partial", "--server", "0"],
                    env,
                    "Connected",
                    0,
                    204,
                    partial=True,
                )
                once(
                    arguments(
                        args.host, engine, binary, "https://127.0.0.1:24444/status"
                    ),
                    env,
                    "Limited",
                    2,
                    503,
                )
                once(
                    arguments(
                        args.host, engine, binary, "https://127.0.0.1:24444/redirect"
                    ),
                    env,
                    "Limited",
                    2,
                    302,
                )
                once(
                    arguments(args.host, engine, binary),
                    environment(workspace, trusted=False),
                    "Limited",
                    2,
                    kind="tls",
                )
                once(
                    arguments(args.host, engine, binary, "https://127.0.0.1:24445/"),
                    env,
                    "Limited",
                    2,
                    kind="tls",
                )
                command = arguments(
                    args.host,
                    engine,
                    binary,
                    "https://127.0.0.1:24444/slow",
                    once=False,
                )
                signal_runtime(command, env, https, crash=False)
                signal_runtime(command, env, https, crash=True)
                live = arguments(args.host, engine, binary, once=False)
                signal_runtime(live, env, https, crash=False, phase="Connected")
                signal_runtime(
                    live, env, https, crash=False, phase="Connected", interrupt=True
                )
                signal_runtime(live, env, https, crash=True, phase="Connected")
                limited = arguments(
                    args.host,
                    engine,
                    binary,
                    "https://127.0.0.1:24444/status",
                    once=False,
                )
                signal_runtime(limited, env, https, crash=True, phase="Limited")
            once(arguments(args.host, engine, binary), env, "Limited", 2)
            with socket.socket() as busy:
                busy.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                busy.bind(("127.0.0.1", 10808))
                busy.listen()
                result = subprocess.run(
                    arguments(args.host, engine, binary),
                    input=PROFILE.encode(),
                    env=env,
                    capture_output=True,
                    timeout=10,
                )
                if (
                    result.returncode != 1
                    or b'"state":"Connected"' in result.stdout
                    or b'"state":"Checking"' in result.stdout
                ):
                    raise RuntimeError("Чужой listener принят за собственный")
            LOG.info(
                "%s: HTTPS204, HTTP503/302, TLS CA/name, без direct fallback, SIGTERM/crash, busy port",
                engine,
            )
        fake_preparation(args.host, args.fake, workspace)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host", type=Path)
    parser.add_argument("xray", type=Path)
    parser.add_argument("sing_box", type=Path)
    parser.add_argument("--worker", type=int)
    parser.add_argument("--workspace", type=Path)
    parser.add_argument("--fake", type=Path)
    args = parser.parse_args()
    if args.worker is not None:
        worker(args)
        return 0
    with tempfile.TemporaryDirectory(prefix="yonik-host-") as directory:
        fake = Path(directory) / "fake-core"
        subprocess.run(
            ["rustc", str(ROOT / "tests/engines/host_fixture.rs"), "-o", str(fake)],
            check=True,
            capture_output=True,
            timeout=30,
        )
        run_sandbox(
            [
                "unshare",
                "--user",
                "--map-root-user",
                "--net",
                "--pid",
                "--fork",
                "--mount-proc",
                "--kill-child",
                sys.executable,
                str(Path(__file__).resolve()),
                str(args.host.resolve()),
                str(args.xray.resolve()),
                str(args.sing_box.resolve()),
                "--worker",
                str(os.stat("/proc/self/ns/net").st_ino),
                "--workspace",
                directory,
                "--fake",
                str(fake),
            ],
            timeout=90,
        )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    raise SystemExit(main())
