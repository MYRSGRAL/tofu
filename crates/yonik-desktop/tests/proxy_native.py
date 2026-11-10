"""Локальная TLS/VLESS приёмка DesktopHost с закреплённым native core."""

from __future__ import annotations

import os
import select
import socket
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from engine_compile import UUID  # noqa: E402
from engine_host import https_server, peer  # noqa: E402


def main() -> None:
    """Запустить готовый Rust test binary, не изменяя сеть или trust store хоста."""
    binary = Path(sys.argv[1]).resolve()
    engines = Path(sys.argv[2]).resolve()
    with tempfile.TemporaryDirectory(prefix="yonik-proxy-native-") as temporary:
        workspace = Path(temporary)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        bus = subprocess.Popen(
            ["dbus-daemon", "--session", "--nofork", "--print-address=1"],
            stdout=subprocess.PIPE,
            text=True,
        )
        try:
            assert bus.stdout is not None
            if not select.select([bus.stdout], [], [], 5)[0]:
                raise TimeoutError("private D-Bus не готов")
            address = bus.stdout.readline().strip()
            with https_server(workspace, "health", 0, "127.0.0.1") as https:
                for engine, directory in (
                    ("sing-box", "sing-box-1.14.4"),
                    ("xray", "xray-26.10.31"),
                ):
                    with peer(
                        engines / directory / engine,
                        engine,
                        workspace,
                        port=port,
                        target_ports="1-65535",
                    ):
                        env = dict(os.environ)
                        env.update(
                            DBUS_SYSTEM_BUS_ADDRESS=address,
                            DBUS_SESSION_BUS_ADDRESS=address,
                            SSL_CERT_FILE=str(workspace / "health.ca.pem"),
                            YONIK_PROXY_ENGINE_DIR=str(engines),
                            YONIK_PROXY_ENGINE=engine,
                            YONIK_PROXY_PROFILE=f"vless://{UUID}@127.0.0.1:{port}#Fixture",
                            YONIK_PROXY_PROBE_URL=f"https://127.0.0.1:{https.server_port}/",
                        )
                        subprocess.run(
                            [
                                str(binary),
                                "native_session_proxy_manual_and_scheduled_refresh",
                                "--ignored",
                                "--exact",
                                "--nocapture",
                            ],
                            env=env,
                            check=True,
                            timeout=90,
                        )
                        assert https.requested.is_set()

        finally:
            bus.terminate()
            bus.wait(timeout=5)


if __name__ == "__main__":
    main()
