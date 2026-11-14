"""Owned private SS и реальные loopback HTTPS/SOCKS; только synthetic fixture."""

from __future__ import annotations

import argparse
import http.server
import json
import os
import re
import select
import signal
import socket
import socketserver
import ssl
import subprocess
import tempfile
import threading
import time
from pathlib import Path


class Guard:
    def __init__(self, argv: list[str], **kwargs: object) -> None:
        self.process = subprocess.Popen(argv, start_new_session=True, **kwargs)
        self.fd = os.pidfd_open(self.process.pid)
        self.closed = False

    def stop(self) -> None:
        if self.closed:
            return
        self.closed = True
        # WNOWAIT сохраняет reservation PID/PGID до последнего group signal.
        os.waitid(os.P_PID, self.process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(self.process.pid, sig)
            except ProcessLookupError:
                pass
            if sig == signal.SIGTERM:
                time.sleep(0.2)
        self.process.wait(timeout=5)
        os.close(self.fd)


class State:
    def __init__(self) -> None:
        self.mode = "full"
        self.origin_requests = 0
        self.socks_requests = 0
        self.held_started = False
        self.held_closed = False
        self.lock = threading.Lock()
        self.proxy: Server | None = None

    def stats(self) -> dict:
        with self.lock:
            return {
                k: getattr(self, k)
                for k in (
                    "origin_requests",
                    "socks_requests",
                    "held_started",
                    "held_closed",
                )
            }


class Server(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


def start(server: Server) -> None:
    threading.Thread(target=server.serve_forever, daemon=True).start()


def read_exact(sock: socket.socket, size: int) -> bytes:
    data = b""
    while len(data) < size:
        part = sock.recv(size - len(data))
        if not part:
            raise EOFError
        data += part
    return data


def run(args: argparse.Namespace, root: Path, output: Path) -> None:
    state = State()
    cert = root / "cert.pem"
    key = root / "key.pem"
    ca = root / "ca.pem"
    ca_key = root / "ca-key.pem"
    csr = root / "leaf.csr"
    extensions = root / "extensions"
    extensions.write_text(
        "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1\n"
    )
    commands = [
        [
            "/usr/bin/openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            str(ca_key),
            "-out",
            str(ca),
            "-days",
            "1",
            "-subj",
            "/CN=OwnedTestCA",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
        ],
        [
            "/usr/bin/openssl",
            "req",
            "-new",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            str(key),
            "-out",
            str(csr),
            "-subj",
            "/CN=fixture.invalid",
        ],
        [
            "/usr/bin/openssl",
            "x509",
            "-req",
            "-in",
            str(csr),
            "-CA",
            str(ca),
            "-CAkey",
            str(ca_key),
            "-set_serial",
            "1",
            "-out",
            str(cert),
            "-days",
            "1",
            "-extfile",
            str(extensions),
        ],
    ]
    for command in commands:
        subprocess.run(
            command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)

    class Origin(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args: object) -> None:
            pass

        def do_GET(self) -> None:
            if self.path == "/health":
                self.send_response(204)
                self.end_headers()
                return
            with state.lock:
                state.origin_requests += 1
                mode = state.mode
            if mode == "error":
                self.send_response(503)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            body = b"trojan://invented-priority-secret@example.invalid:443#Synthetic"
            if mode == "partial":
                body += b"\nnot-a-uri"
            self.send_response(200)
            self.send_header(
                "Content-Length", str(999 if mode == "held" else len(body))
            )
            self.end_headers()
            if mode == "held":
                with state.lock:
                    state.held_started = True
                self.connection.settimeout(4)
                try:
                    self.wfile.write(b"x")
                    self.wfile.flush()
                    closed = self.connection.recv(1) == b""
                except (ssl.SSLEOFError, ConnectionResetError):
                    closed = True
                except TimeoutError:
                    closed = False
                with state.lock:
                    state.held_closed = closed
            else:
                self.wfile.write(body)

    origin = Server(("127.0.0.1", 0), Origin)
    origin.socket = context.wrap_socket(origin.socket, server_side=True)
    start(origin)
    target = origin.server_address

    class Socks(socketserver.BaseRequestHandler):
        def handle(self) -> None:
            peer = self.request
            peer.settimeout(5)
            try:
                version, methods = read_exact(peer, 2)
                assert version == 5
                read_exact(peer, methods)
                peer.sendall(b"\5\0")
                head = read_exact(peer, 4)
                assert head == b"\5\1\0\1"
                address = socket.inet_ntoa(read_exact(peer, 4))
                port = int.from_bytes(read_exact(peer, 2), "big")
                assert (address, port) == target
                with state.lock:
                    state.socks_requests += 1
                with socket.create_connection(target, timeout=5) as upstream:
                    peer.sendall(b"\5\0\0\1\0\0\0\0\0\0")
                    deadline = time.monotonic() + 15
                    while time.monotonic() < deadline:
                        ready, _, _ = select.select([peer, upstream], [], [], 0.1)
                        for source in ready:
                            payload = source.recv(16384)
                            if not payload:
                                return
                            (upstream if source is peer else peer).sendall(payload)
            except (OSError, EOFError):
                return

    proxy = Server(("127.0.0.1", 0), Socks)
    state.proxy = proxy
    start(proxy)

    class Control(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args: object) -> None:
            pass

        def do_GET(self) -> None:
            route = self.path.strip("/")
            if route.startswith("mode/"):
                with state.lock:
                    state.mode = route.split("/")[1]
                    if state.mode == "held":
                        state.held_started = False
                        state.held_closed = False
            elif route == "proxy/stop":
                proxy.shutdown()
                proxy.server_close()
                state.proxy = None
            elif route == "origin-control":
                client = ssl.create_default_context(cafile=str(ca))
                with client.wrap_socket(
                    socket.create_connection(target, timeout=3),
                    server_hostname="127.0.0.1",
                ) as connection:
                    connection.sendall(b"GET /health HTTP/1.0\r\nHost: fixture\r\n\r\n")
                    assert b"204" in connection.recv(2048)
                result = dict(available=True)
            else:
                result = state.stats()
            if route != "origin-control":
                result = state.stats()
            body = json.dumps(result).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    control = Server(("127.0.0.1", 0), Control)
    start(control)
    owned = []
    env = os.environ.copy()
    home = root / "home"
    home.mkdir(mode=0o700)
    runtime = root / "runtime"
    runtime.mkdir(mode=0o700)
    data = root / "data"
    data.mkdir(mode=0o700)
    env.update(
        HOME=str(home),
        XDG_RUNTIME_DIR=str(runtime),
        XDG_DATA_HOME=str(home / "data"),
        XDG_CONFIG_HOME=str(home / "config"),
        XDG_CACHE_HOME=str(home / "cache"),
        SSL_CERT_FILE=str(ca),
        P05_DATA=str(data),
        P05_URL=f"https://127.0.0.1:{target[1]}/subscription",
        P05_SOCKS=f"127.0.0.1:{proxy.server_address[1]}",
        P05_CONTROL=f"127.0.0.1:{control.server_address[1]}",
    )
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    try:
        bus = Guard(
            [
                "/usr/bin/dbus-daemon",
                "--session",
                "--nofork",
                "--nopidfile",
                f"--address=unix:path={runtime}/bus",
                "--print-address=1",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        owned.append(bus)
        assert select.select([bus.process.stdout], [], [], 5)[0]
        address = bus.process.stdout.readline().decode().strip()
        assert address.startswith("unix:")
        env["DBUS_SESSION_BUS_ADDRESS"] = address
        password = root / "password"
        with password.open("xb") as stream:
            stream.write(os.urandom(32).hex().encode())
        password.chmod(0o600)
        daemon_env = env | {"LD_LIBRARY_PATH": str(args.runtime / "lib")}
        with password.open("rb") as stream:
            daemon = Guard(
                [
                    str(args.runtime / "gnome-keyring-daemon"),
                    "--foreground",
                    "--unlock",
                    "--components=secrets",
                    "--control-directory",
                    str(runtime / "keyring"),
                ],
                env=daemon_env,
                stdin=stream,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            owned.append(daemon)
        deadline = time.monotonic() + 8
        while True:
            ready = subprocess.run(
                [
                    "/usr/bin/gdbus",
                    "call",
                    "--address",
                    address,
                    "--dest",
                    "org.freedesktop.DBus",
                    "--object-path",
                    "/org/freedesktop/DBus",
                    "--method",
                    "org.freedesktop.DBus.NameHasOwner",
                    "org.freedesktop.secrets",
                ],
                capture_output=True,
                timeout=2,
            )
            if b"true" in ready.stdout:
                break
            assert time.monotonic() < deadline
            time.sleep(0.05)
        with (output / "raw-test.log").open("xb") as log:
            child = Guard(
                [
                    str(args.binary),
                    "--ignored",
                    "--exact",
                    "actual_normal_host_subscription_priority",
                    "--nocapture",
                ],
                env=env,
                stdout=log,
                stderr=log,
            )
            owned.append(child)
            deadline = time.monotonic() + 90
            while (
                os.waitid(
                    os.P_PID, child.process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT
                )
                is None
            ):
                assert time.monotonic() < deadline
                time.sleep(0.05)
            status = os.waitid(os.P_PID, child.process.pid, os.WEXITED | os.WNOWAIT)
            assert status.si_status == 0
        result = dict(
            valid=True,
            normal_host_private_ss=True,
            fixture_counts=state.stats(),
            native_engine=False,
            hardware_power=None,
        )
    finally:
        for process in reversed(owned):
            process.stop()
        (output / "cleanup.json").write_text(
            json.dumps(
                {
                    "owned_count": len(owned),
                    "all_owned_reaped": all(
                        p.closed and p.process.returncode is not None for p in owned
                    ),
                }
            )
            + "\n"
        )
        for server in [control, origin, state.proxy]:
            if server is not None:
                server.shutdown()
                server.server_close()
    result["all_owned_reaped"] = all(p.process.returncode is not None for p in owned)
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    args.output.mkdir(parents=True, exist_ok=False)
    try:
        with tempfile.TemporaryDirectory(prefix="p05-p10-actual-") as directory:
            run(args, Path(directory), args.output)
        return 0
    except BaseException as error:
        (args.output / "failure.json").write_text(
            json.dumps(dict(valid=False, error_type=type(error).__name__)) + "\n"
        )
        return 1
    finally:
        raw = args.output / "raw-test.log"
        if raw.exists():
            text = raw.read_text(errors="replace")
            text = re.sub(r"https?://\S+", "<fixture-url>", text).replace(
                "invented-priority-secret", "<synthetic-redacted>"
            )
            raw.write_text(text)


if __name__ == "__main__":
    raise SystemExit(main())
