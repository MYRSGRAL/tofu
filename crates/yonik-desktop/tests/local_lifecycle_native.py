"""Production GUI lifecycle local source с обоими закреплёнными cores."""

from __future__ import annotations

import json
import os
import select
import socket
import ssl
import subprocess
import sys
import tempfile
from pathlib import Path
from urllib.request import HTTPSHandler, ProxyHandler, build_opener

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from engine_compile import UUID  # noqa: E402
from engine_host import https_server, peer  # noqa: E402


def main() -> None:
    """Запустить GUI с приватными D-Bus, VLESS peer и HTTPS probe."""
    binary = Path(sys.argv[1]).resolve()
    engines = Path(sys.argv[2]).resolve()
    with (
        tempfile.TemporaryDirectory(prefix="yonik-proxy-native-") as temporary,
        tempfile.TemporaryDirectory(prefix="y137-") as runtime_directory,
    ):
        workspace = Path(temporary)
        # Короткий путь оставляет место для имени Unix socket из SHA256.
        runtime = Path(runtime_directory)
        home = workspace / "home"
        home.mkdir(mode=0o700)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        bus_config = workspace / "bus.conf"
        bus_config.write_text(
            "<busconfig><type>session</type><listen>unix:tmpdir="
            + str(runtime)
            + '</listen><auth>EXTERNAL</auth><policy context="default">'
            '<allow send_destination="*"/><allow receive_sender="*"/>'
            '<allow own="*"/></policy></busconfig>'
        )
        bus = subprocess.Popen(
            [
                "dbus-daemon",
                "--config-file=" + str(bus_config),
                "--nofork",
                "--print-address=1",
            ],
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
                        url = f"https://127.0.0.1:{https.server_port}/"
                        context = ssl.create_default_context(
                            cafile=str(workspace / "health.ca.pem")
                        )
                        direct = build_opener(
                            ProxyHandler({}), HTTPSHandler(context=context)
                        )
                        for case, identity, status, expected_request in (
                            (
                                "denied",
                                "00000000-0000-4000-8000-000000000001",
                                "Limited",
                                False,
                            ),
                            ("known", UUID, "Connected", True),
                        ):
                            # Endpoint доступен даже в отказном контроле; direct
                            # bypass с неверным UUID поэтому провалит oracle.
                            with direct.open(url, timeout=3) as response:
                                if response.status != 204:
                                    raise RuntimeError("HTTPS fixture недоступна")
                            https.requested.clear()
                            env = dict(os.environ)
                            env.update(
                                HOME=str(home),
                                XDG_RUNTIME_DIR=str(runtime),
                                XDG_CONFIG_HOME=str(workspace / "config"),
                                XDG_DATA_HOME=str(workspace / "data"),
                                XDG_CACHE_HOME=str(workspace / "cache"),
                                DBUS_SYSTEM_BUS_ADDRESS=address,
                                DBUS_SESSION_BUS_ADDRESS=address,
                                SSL_CERT_FILE=str(workspace / "health.ca.pem"),
                                YONIK_LIFECYCLE_ENGINES=str(engines),
                                YONIK_LIFECYCLE_ENGINE=engine,
                                YONIK_LIFECYCLE_PROFILE=f"vless://{identity}@127.0.0.1:{port}#Fixture",
                                YONIK_LIFECYCLE_PROBE=url,
                                YONIK_LIFECYCLE_STATUS=status,
                            )
                            output = Path(sys.argv[3]).resolve() / f"{engine}-{case}"
                            subprocess.run(
                                [
                                    sys.executable,
                                    str(
                                        Path(__file__).with_name(
                                            "local_lifecycle_smoke.py"
                                        )
                                    ),
                                    str(binary),
                                    str(output),
                                ],
                                env=env,
                                check=True,
                                timeout=150,
                            )
                            requested = https.requested.is_set()
                            if requested != expected_request:
                                raise RuntimeError("HTTPS probe обошёл VLESS admission")
                            (output / "peer-control.json").write_text(
                                json.dumps(
                                    {
                                        "direct_endpoint_available": True,
                                        "valid_peer_credentials": expected_request,
                                        "https_requested": requested,
                                        "expected_status": status,
                                        "normal_stop_only": True,
                                    },
                                    indent=2,
                                )
                            )
        finally:
            bus.terminate()
            bus.wait(timeout=5)


if __name__ == "__main__":
    main()
