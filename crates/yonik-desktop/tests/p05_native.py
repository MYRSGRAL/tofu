"""P05: собственные CA/upstream, оба pinned core, private namespace/D-Bus."""

from __future__ import annotations

import contextlib
import http.server
import json
import os
import select
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from dns_fixture import Reply, make_response, read_question
from engine_compile import UUID
from engine_host import https_server, peer


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_: object) -> None:
        pass

    def do_GET(self) -> None:
        case = self.path.split("?", 1)[0].strip("/")
        body = self.server.profile
        status = 200
        headers = {}
        if case not in {"seed", "old"}:
            body = body.replace("#Fixture", "#rotated")
        if case == "credentials":
            body = body.replace(UUID, "22222222-2222-4222-8222-222222222222")
        if case == "partial":
            body += "\nunknown://broken"
        if case == "empty":
            body = ""
        if case == "corrupt":
            body = "unknown://broken"
        if case == "http":
            status, body = 503, ""
        if case == "304":
            status, body = 304, ""
        if case in {"redirect", "cross"}:
            status, body = 302, ""
            headers["Location"] = (
                self.server.cross if case == "cross" else ""
            ) + "/full"
        if case == "full" and self.server.cross_check:
            assert "new-canary" not in self.path
            assert not self.headers.get("Authorization")
        self.send_response(status)
        for name, value in headers.items():
            self.send_header(name, value)
        self.send_header("ETag", "p05")
        self.send_header(
            "Content-Length", str(999999999 if case == "size" else len(body.encode()))
        )
        self.end_headers()
        self.wfile.write(body.encode())
        self.server.count += 1


@contextlib.contextmanager
def upstream(profile: str, context: ssl.SSLContext | None = None):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.profile, server.cross, server.cross_check, server.count = (
        profile,
        "",
        False,
        0,
    )
    if context:
        server.socket = context.wrap_socket(server.socket, server_side=True)
    thread = threading.Thread(
        target=server.serve_forever, kwargs={"poll_interval": 0.05}
    )
    thread.start()
    try:
        yield server
    finally:
        server.shutdown()
        thread.join(5)
        server.server_close()
        assert not thread.is_alive()


def main() -> None:
    binary, engines, evidence = map(lambda x: Path(x).resolve(), sys.argv[1:4])
    assert socket.if_nameindex() == [(1, "lo")], "только isolated loopback"
    with tempfile.TemporaryDirectory(prefix="p05-native-") as temporary:
        work = Path(temporary)
        work.chmod(0o700)
        with socket.socket() as reserve:
            reserve.bind(("127.0.0.1", 0))
            peer_port = reserve.getsockname()[1]
        profile = f"vless://{UUID}@127.0.0.1:{peer_port}#Fixture"
        with (
            https_server(work, "good", 0, "127.0.0.1") as health,
            https_server(work, "bad", 0, "127.0.0.1"),
        ):
            good = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            good.load_cert_chain(work / "good.pem", work / "good.key")
            bad = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            bad.load_cert_chain(work / "bad.pem", work / "bad.key")
            with (
                upstream(profile) as seed,
                upstream(profile) as target,
                upstream(profile, good) as secure,
                upstream(profile, bad) as untrusted,
                upstream(profile) as cross,
            ):
                target.cross = f"http://127.0.0.1:{cross.server_port}"
                cross.cross_check = True
                bus = subprocess.Popen(
                    ["dbus-daemon", "--session", "--nofork", "--print-address=1"],
                    stdout=subprocess.PIPE,
                    text=True,
                )
                try:
                    assert select.select([bus.stdout], [], [], 5)[0]
                    address = bus.stdout.readline().strip()
                    with socket.socket() as dns:
                        dns.bind(("127.0.0.1", 0))
                        dns.listen()
                        dns.settimeout(0.1)
                        stopped, queries, errors = threading.Event(), [], []

                        def serve_dns() -> None:
                            while not stopped.is_set():
                                try:
                                    stream, _ = dns.accept()
                                except socket.timeout:
                                    continue
                                try:
                                    with stream:
                                        stream.settimeout(5)

                                        def exact(size: int) -> bytes:
                                            data = b""
                                            while len(data) < size:
                                                chunk = stream.recv(size - len(data))
                                                if not chunk:
                                                    raise ValueError("DNS EOF")
                                                data += chunk
                                            return data

                                        packet = exact(struct.unpack("!H", exact(2))[0])
                                        name, kind, _ = read_question(packet)
                                        assert name == "subscription.example"
                                        queries.append(kind)
                                        reply = make_response(
                                            packet,
                                            Reply(
                                                addresses=("127.0.0.1",)
                                                if kind == 1
                                                else ()
                                            ),
                                        )
                                        stream.sendall(
                                            struct.pack("!H", len(reply)) + reply
                                        )
                                except Exception as error:
                                    errors.append(type(error).__name__)

                        thread = threading.Thread(target=serve_dns)
                        thread.start()
                        try:
                            for engine, folder in [
                                ("sing-box", "sing-box-1.14.4"),
                                ("xray", "xray-26.10.31"),
                            ]:
                                with peer(
                                    engines / folder / engine,
                                    engine,
                                    work,
                                    port=peer_port,
                                    target_ports="1-65535",
                                ):
                                    env = dict(
                                        os.environ,
                                        SSL_CERT_FILE=str(work / "good.ca.pem"),
                                        DBUS_SYSTEM_BUS_ADDRESS=address,
                                        DBUS_SESSION_BUS_ADDRESS=address,
                                        YONIK_PROXY_ENGINE_DIR=str(engines),
                                        YONIK_PROXY_ENGINE=engine,
                                        YONIK_PROXY_PROBE_URL=f"https://127.0.0.1:{health.server_port}/",
                                        YONIK_PROXY_FETCH_DNS=f"127.0.0.1:{dns.getsockname()[1]}",
                                        P05_SEED_URL=f"http://127.0.0.1:{seed.server_port}/seed",
                                        P05_PROXY_FAIL_URL="http://subscription.example:1/full",
                                        P05_TARGET_URL=f"http://subscription.example:{target.server_port}",
                                        P05_TLS_URL=f"https://127.0.0.1:{secure.server_port}",
                                        P05_BAD_TLS_URL=f"https://127.0.0.1:{untrusted.server_port}",
                                    )
                                    env["P05_FORBIDDEN_PORTS"] = (
                                        ","
                                        + ",,".join(
                                            str(server.server_port)
                                            for server in (
                                                target,
                                                cross,
                                                secure,
                                                untrusted,
                                                health,
                                            )
                                        )
                                        + ","
                                    )
                                    env["P05_FORBIDDEN_PORTS"] += (
                                        f",{dns.getsockname()[1]},,1,"
                                    )
                                    guard = Path(__file__).with_name("p05_guard.py")
                                    with (evidence / f"{engine}.log").open("w") as log:
                                        subprocess.run(
                                            [
                                                sys.executable,
                                                str(guard),
                                                str(binary),
                                                "p05_native_matrix",
                                                "--ignored",
                                                "--exact",
                                                "--nocapture",
                                            ],
                                            env=env,
                                            stdout=log,
                                            stderr=subprocess.STDOUT,
                                            timeout=180,
                                            check=True,
                                        )
                        finally:
                            stopped.set()
                            thread.join(6)
                        assert not errors and not thread.is_alive(), errors
                        (evidence / "counts.json").write_text(
                            json.dumps(
                                {
                                    "dns_queries": len(queries),
                                    "target_requests": target.count,
                                    "cross_requests": cross.count,
                                    "tls_requests": secure.count,
                                }
                            )
                        )
                finally:
                    bus.terminate()
                    bus.wait(5)


if __name__ == "__main__":
    main()
