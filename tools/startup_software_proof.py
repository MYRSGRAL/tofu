"""Normal Host/restart на собственных private bus/SS и pinned loopback cores.

Не устанавливает XDG/services и не использует VM, TUN, cross-UID или physical DE.
Все бинарники должны быть заранее retained вне build-root вместе с исходниками.
"""

from __future__ import annotations

import argparse
import hashlib
import http.server
import json
import os
import queue
import secrets
import socket
import subprocess
import tempfile
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Any

from engine_compile import UUID
from engine_host import https_server

PINS = {
    "xray": (
        "xray-26.10.31/xray",
        "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1",
    ),
    "sing-box": (
        "sing-box-1.14.4/sing-box",
        "1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a",
    ),
}


def wait_for(predicate: Any, timeout: float = 10) -> Any:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.02)
    raise TimeoutError("fixture deadline")


def owns_listener(process: subprocess.Popen[Any], port: int) -> bool:
    if process.poll() is not None:
        raise RuntimeError("Собственный peer завершился до listener")
    sockets = set()
    for fd in Path(f"/proc/{process.pid}/fd").iterdir():
        try:
            target = str(fd.readlink())
        except FileNotFoundError:
            continue
        if target.startswith("socket:["):
            sockets.add(target[8:-1])
    for line in Path(f"/proc/{process.pid}/net/tcp").read_text().splitlines()[1:]:
        fields = line.split()
        if (
            fields[1] == f"0100007F:{port:04X}"
            and fields[3] == "0A"
            and fields[9] in sockets
        ):
            return True
    return False


@contextmanager
def owned_peer(
    binary: Path,
    engine: str,
    root: Path,
    port: int,
    target_port: int,
    env: dict[str, str],
) -> Iterator[subprocess.Popen[Any]]:
    # Минимальная VLESS-конфигурация существующего engine_host.peer; readiness
    # проверяется по собственным FD, без дополнительного сетевого probe.
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
                                "port": str(target_port),
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
    path = root / "peer.json"
    path.write_text(json.dumps(config))
    with (root / "peer.log").open("w") as log:
        process = subprocess.Popen(
            [str(binary), "run", "-c", str(path)], stdout=log, stderr=log, env=env
        )
        try:
            wait_for(lambda: owns_listener(process, port))
            yield process
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


class Host:
    def __init__(
        self,
        binary: Path,
        root: Path,
        engines: Path,
        env: dict[str, str],
        label: str,
        barrier: bool = False,
    ) -> None:
        self.trace = (root / f"{label}.jsonl").open("w")
        self.stderr = (root / f"{label}.stderr").open("w")
        self.process = subprocess.Popen(
            [str(binary), str(root), str(engines)] + (["barrier"] if barrier else []),
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr,
            text=True,
            bufsize=1,
        )
        self.events: queue.Queue[dict[str, Any]] = queue.Queue()
        self.last: dict[str, Any] = {}
        self.thread = threading.Thread(target=self.read, daemon=True)
        self.thread.start()

    def read(self) -> None:
        assert self.process.stdout
        for line in self.process.stdout:
            self.trace.write(line)
            self.trace.flush()
            self.events.put(json.loads(line))

    def send(self, command: dict[str, Any]) -> None:
        assert self.process.stdin
        self.process.stdin.write(json.dumps(command) + "\n")
        self.process.stdin.flush()

    def until(self, predicate: Any, timeout: float = 20) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        while True:
            self.last = self.events.get(timeout=max(0.001, deadline - time.monotonic()))
            if predicate(self.last):
                return self.last

    def close(self) -> None:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            assert self.process.wait(timeout=15) == 0
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait()
            self.thread.join(timeout=3)
            self.trace.close()
            self.stderr.close()


def bus_call(env: dict[str, str], method: str, value: str) -> Any:
    address = env["DBUS_SESSION_BUS_ADDRESS"]
    if not address.startswith("unix:path=/tmp/yonik-startup-"):
        raise ValueError("Разрешена только собственная private bus, без fallback")
    result = subprocess.run(
        [
            "busctl",
            "--address=" + env["DBUS_SESSION_BUS_ADDRESS"],
            "--json=short",
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            method,
            "s",
            value,
        ],
        env=env,
        capture_output=True,
        text=True,
        timeout=3,
    )
    if result.returncode:
        return None
    return json.loads(result.stdout)["data"][0]


def campaign(args: argparse.Namespace, engine: str) -> dict[str, Any]:
    root = (args.output / engine).resolve()
    root.mkdir(mode=0o700)
    for name in ["runtime", "config", "xdg-data", "data"]:
        (root / name).mkdir(mode=0o700)
    (root / "startup-fixture.marker").write_text("owned software fixture\n")
    runtime_context = tempfile.TemporaryDirectory(prefix="yonik-startup-", dir="/tmp")
    runtime = Path(runtime_context.name)
    env = dict(
        os.environ,
        HOME=str(root),
        XDG_RUNTIME_DIR=str(runtime),
        XDG_CONFIG_HOME=str(root / "config"),
        XDG_DATA_HOME=str(root / "xdg-data"),
        DBUS_SYSTEM_BUS_ADDRESS="unix:path=" + str(runtime / "no-system-bus"),
    )
    env.pop("DBUS_SESSION_BUS_ADDRESS", None)
    env.pop("GNOME_KEYRING_CONTROL", None)
    env["LD_LIBRARY_PATH"] = str(args.keyring.parent / "lib")
    conf = root / "bus.conf"
    conf.write_text(
        f'<busconfig><type>session</type><listen>unix:path={runtime}/bus</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
    )
    bus_log = (root / "bus.stderr").open("w")
    bus = subprocess.Popen(
        [
            "dbus-daemon",
            "--nofork",
            "--nopidfile",
            "--print-address=1",
            "--config-file",
            str(conf),
        ],
        stdout=subprocess.PIPE,
        stderr=bus_log,
        text=True,
        env=env,
    )
    assert bus.stdout
    address = bus.stdout.readline().strip()
    if not address.startswith(f"unix:path={runtime}/bus"):
        if bus.poll() is None:
            bus.terminate()
        bus.wait(timeout=3)
        bus_log.close()
        runtime_context.cleanup()
        raise RuntimeError("Собственная bus не стартовала; fallback запрещён")
    env["DBUS_SESSION_BUS_ADDRESS"] = address
    children: list[subprocess.Popen[Any]] = [bus]
    logs: list[Any] = [bus_log]
    hosts: list[Host] = []
    password = secrets.token_hex(32).encode()
    keyring_log = (root / "keyring.stderr").open("w")
    logs.append(keyring_log)

    def keyring() -> subprocess.Popen[Any]:
        daemon = subprocess.Popen(
            [
                str(args.keyring),
                "--foreground",
                "--unlock",
                "--components=secrets",
                "--control-directory",
                str(runtime / "control"),
            ],
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.DEVNULL,
            stderr=keyring_log,
        )
        assert daemon.stdin
        daemon.stdin.write(password)
        daemon.stdin.close()
        children.append(daemon)
        owner = wait_for(
            lambda: bus_call(env, "GetNameOwner", "org.freedesktop.secrets")
        )
        assert bus_call(env, "GetConnectionUnixProcessID", owner) == daemon.pid
        return daemon

    hits: list[str] = []

    class Health(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            hits.append(self.path)
            self.send_response(204)
            self.end_headers()

        def log_message(self, format: str, *args: object) -> None:
            pass

    health_context = None
    peer_context = None
    try:
        health_context = https_server(root, "startup-health", 0, "127.0.0.1")
        health_server = health_context.__enter__()
        health_server.RequestHandlerClass = Health
        env["SSL_CERT_FILE"] = str(root / "startup-health.ca.pem")
        daemon = keyring()
        with socket.socket() as reserve:
            reserve.bind(("127.0.0.1", 0))
            server_port = reserve.getsockname()[1]
        peer_context = owned_peer(
            args.engines / PINS[engine][0],
            engine,
            root,
            port=server_port,
            target_port=health_server.server_port,
            env=env,
        )
        peer_context.__enter__()
        seed = Host(args.binary, root, args.engines, env, "seed")
        hosts.append(seed)
        seed.until(lambda s: s.get("ready") is True)
        profile = f"vless://{UUID}@127.0.0.1:{server_port}#startup-fixture"
        seed.send(
            {"command": "import_text", "name": "startup fixture", "text": profile}
        )
        seed.until(lambda s: bool(s.get("pending")))
        seed.send(
            {
                "command": "maintenance_start",
                "request_id": 90,
                "operation": "backup",
                "destination": str(root / "before-import.sqlite"),
                "revision": 0,
            }
        )
        backup = seed.until(
            lambda s: (
                s.get("maintenance_ack", {}).get("request_id") == 90
                and s.get("maintenance_ack", {}).get("phase") in ["complete", "failed"]
            )
        )
        assert backup["maintenance_ack"]["phase"] == "complete", backup[
            "maintenance_ack"
        ]
        seed.send({"command": "approve_refresh"})
        selected = seed.until(lambda s: bool(s.get("profiles")) and not s.get("busy"))
        profile_id = selected["profiles"][0]["id"]
        seed.send(
            {
                "command": "select_profile",
                "id": profile_id,
                "revision": selected["revision"],
            }
        )
        seed.until(lambda s: s.get("selected_profile") == profile_id)
        seed.send({"command": "set_engine", "engine": engine})
        seed.until(lambda s: s.get("engine") == engine)
        seed.send(
            {
                "command": "set_probe_target",
                "url": f"https://127.0.0.1:{health_server.server_port}/normal-connect",
                "expected_status": 204,
            }
        )
        seed.until(lambda s: s.get("probe_custom") is True)
        seed.send(
            {
                "command": "autoconnect_target_set",
                "request_id": "1",
                "expected_revision": "0",
            }
        )
        capture = seed.until(lambda s: s.get("startup", {}).get("request_id") == "1")
        assert capture["startup"]["save"] == "durable", capture.get("startup")
        assert capture["startup"]["error"] is None
        seed.send(
            {
                "command": "startup_preferences_set",
                "request_id": "2",
                "expected_revision": "1",
                "autostart_into_tray": False,
                "autoconnect": True,
            }
        )
        saved = seed.until(lambda s: s.get("startup", {}).get("request_id") == "2")
        assert saved["startup"]["save"] == "durable" and saved["can_stop"] is False
        seed.close()
        hosts.remove(seed)
        restarted = Host(args.binary, root, args.engines, env, "restart")
        hosts.append(restarted)
        connected = restarted.until(
            lambda s: (
                s.get("startup", {}).get("attempt") == "attempted"
                and s.get("status") in ["Connected", "Limited"]
                and s.get("can_stop") is True
            )
        )
        assert connected["active_engine"] == engine and connected["ephemeral"] is False
        assert hits == ["/normal-connect"], hits
        restarted.send({"command": "stop"})
        restarted.until(
            lambda s: (
                s.get("can_stop") is False
                and s.get("startup", {}).get("attempt") == "attempted"
            )
        )
        time.sleep(0.3)
        assert len(hits) == 1
        restarted.close()
        hosts.remove(restarted)
        (root / "barrier-ready").unlink(missing_ok=True)
        (root / "barrier-release").unlink(missing_ok=True)
        stopped = Host(
            args.binary, root, args.engines, env, "stop-before-barrier", True
        )
        hosts.append(stopped)
        wait_for(lambda: (root / "barrier-ready").exists())
        stopped.send({"command": "stop"})
        time.sleep(0.05)
        (root / "barrier-release").write_text("release")
        stopped.until(lambda s: s.get("startup", {}).get("attempt") == "blocked")
        stopped.close()
        hosts.remove(stopped)
        assert len(hits) == 1
        (root / "barrier-ready").unlink()
        (root / "barrier-release").unlink()
        lost = Host(args.binary, root, args.engines, env, "owner-loss", True)
        hosts.append(lost)
        wait_for(lambda: (root / "barrier-ready").exists())
        daemon.terminate()
        daemon.wait(timeout=5)
        wait_for(lambda: not bus_call(env, "GetNameOwner", "org.freedesktop.secrets"))
        (root / "barrier-release").write_text("release")
        lost.until(lambda s: s.get("startup", {}).get("attempt") == "blocked")
        keyring()
        time.sleep(0.3)
        assert len(hits) == 1
        lost.close()
        hosts.remove(lost)
        # Копия БД с тем же содержимым получает новый inode: target не переносится.
        import shutil

        live = root / "data" / "subscriptions.sqlite"
        replacement = live.with_suffix(".replacement")
        shutil.copyfile(live, replacement)
        replacement.chmod(0o600)
        replacement.replace(live)
        replaced = Host(args.binary, root, args.engines, env, "repository-replacement")
        hosts.append(replaced)
        blocked = replaced.until(
            lambda s: s.get("startup", {}).get("attempt") == "blocked"
        )
        assert blocked["startup"]["error"] == "startup_binding"
        replaced.close()
        hosts.remove(replaced)
        assert len(hits) == 1
        return {
            "engine": engine,
            "normal_custody": True,
            "logical_restart": True,
            "actual_loopback_connect": True,
            "stop_no_rearm": True,
            "stop_before_barrier": True,
            "owner_loss_late_ready": True,
            "repository_replacement": True,
            "health_requests": len(hits),
        }
    finally:
        cleanup_errors = []
        for host in hosts:
            try:
                host.close()
            except Exception as error:
                cleanup_errors.append(str(error))
        if peer_context is not None:
            peer_context.__exit__(None, None, None)
        if health_context is not None:
            health_context.__exit__(None, None, None)
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        for log in logs:
            log.close()
        runtime_context.cleanup()
        if cleanup_errors:
            raise RuntimeError(
                "Ошибка закрытия собственных hosts: " + repr(cleanup_errors)
            )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--engines", type=Path, required=True)
    parser.add_argument("--keyring", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for relative, digest in PINS.values():
        assert (
            hashlib.sha256((args.engines / relative).read_bytes()).hexdigest() == digest
        )
    results = [campaign(args, engine) for engine in PINS]
    (args.output / "result.json").write_text(json.dumps(results, indent=2))
    print(json.dumps(results))


if __name__ == "__main__":
    main()
