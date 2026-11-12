"""Production GUI lifecycle local source с обоими закреплёнными cores."""

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
    """Запустить GUI с приватными D-Bus, VLESS peer и HTTPS probe."""
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
                        https.requested.clear()
                        env = dict(os.environ)
                        env.update(
                            DBUS_SYSTEM_BUS_ADDRESS=address,
                            DBUS_SESSION_BUS_ADDRESS=address,
                            SSL_CERT_FILE=str(workspace / "health.ca.pem"),
                            YONIK_LIFECYCLE_ENGINES=str(engines),
                            YONIK_LIFECYCLE_ENGINE=engine,
                            YONIK_LIFECYCLE_PROFILE=f"vless://{UUID}@127.0.0.1:{port}#Fixture",
                            YONIK_LIFECYCLE_PROBE=f"https://127.0.0.1:{https.server_port}/",
                        )
                        subprocess.run(
                            [
                                sys.executable,
                                str(
                                    Path(__file__).with_name("local_lifecycle_smoke.py")
                                ),
                                str(binary),
                                str(Path(sys.argv[3]).resolve() / engine),
                            ],
                            env=env,
                            check=True,
                            timeout=150,
                        )
                        if not https.requested.is_set():
                            raise RuntimeError(
                                "Core не передал HTTPS probe локальному серверу"
                            )
                        (Path(sys.argv[3]) / engine / "traffic.txt").write_text(
                            "HTTPS probe reached local server; GUI Connected and normal Stop completed\n"
                        )
        finally:
            bus.terminate()
            bus.wait(timeout=5)


if __name__ == "__main__":
    main()
