"""Детерминированная отмена actual normal Host consumer; только owned loopback."""

from __future__ import annotations

import json
import os
import select
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path

from local_socks_alternating import command
from local_socks_resource import children
from p09_host_sigkill import CORES, available, capture, free_port

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from engine_compile import UUID
from engine_host import https_server, peer


@contextmanager
def private_secrets(env: dict[str, str], root: Path, provider: Path):
    daemon_env = env | {"LD_LIBRARY_PATH": str(provider / "lib")}
    with subprocess.Popen(
        [
            str(provider / "gnome-keyring-daemon"),
            "--foreground",
            "--unlock",
            "--components=secrets",
            "--control-directory",
            str(root / "keyring"),
        ],
        env=daemon_env,
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ) as daemon:
        try:
            daemon.stdin.write(os.urandom(32).hex().encode())
            daemon.stdin.close()
            deadline = time.monotonic() + 8
            while True:
                assert daemon.poll() is None, "Private Secret Service завершился"
                ready = subprocess.run(
                    [
                        "gdbus",
                        "call",
                        "--address",
                        env["DBUS_SESSION_BUS_ADDRESS"],
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
                assert time.monotonic() < deadline, "Private Secret Service не готов"
                time.sleep(0.05)
            yield
        finally:
            daemon.terminate()
            daemon.wait(timeout=5)


def run(binary: Path, engines: Path, output: Path, provider: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    results = []
    for engine, (relative, pin) in CORES.items():
        for stage in ("validate", "apply"):
            for cancel in (True, False):
                case = dict(engine=engine, stage=stage, cancel=cancel)
                with tempfile.TemporaryDirectory(prefix="p09-cancel-") as tmp:
                    root = Path(tmp)
                    env = os.environ | {
                        "HOME": tmp,
                        "XDG_RUNTIME_DIR": tmp,
                        "XDG_CONFIG_HOME": str(root / "config"),
                        "XDG_DATA_HOME": str(root / "data"),
                        "XDG_CACHE_HOME": str(root / "cache"),
                    }
                    with subprocess.Popen(
                        ["dbus-daemon", "--session", "--nofork", "--print-address=1"],
                        stdout=subprocess.PIPE,
                        text=True,
                    ) as bus:
                        try:
                            assert select.select([bus.stdout], [], [], 5)[0]
                            address = bus.stdout.readline().strip()
                            env |= {
                                "DBUS_SESSION_BUS_ADDRESS": address,
                                "DBUS_SYSTEM_BUS_ADDRESS": address,
                            }
                            with (
                                private_secrets(env, root, provider),
                                https_server(root, "health", 0, "127.0.0.1") as https,
                            ):
                                env["SSL_CERT_FILE"] = str(root / "health.ca.pem")
                                port, peer_port = free_port(), free_port()
                                with peer(
                                    engines / relative,
                                    engine,
                                    root,
                                    port=peer_port,
                                    target_ports=str(https.server_port),
                                ):
                                    data = root / "host"
                                    data.mkdir(mode=0o700)
                                    with (
                                        output / f"{engine}-{stage}-{cancel}.stderr"
                                    ).open("w") as err:
                                        host = subprocess.Popen(
                                            [
                                                str(binary),
                                                str(data),
                                                str(engines),
                                                engine,
                                                f"vless://{UUID}@127.0.0.1:{peer_port}#Synthetic",
                                                f"https://127.0.0.1:{https.server_port}/",
                                                str(port),
                                                stage,
                                            ],
                                            env=env,
                                            stdin=subprocess.PIPE,
                                            stdout=subprocess.PIPE,
                                            stderr=err,
                                            text=True,
                                        )
                                        fd = None
                                        try:
                                            command(host, "", "ready")
                                            boundary = command(host, "begin", "begin")
                                            assert boundary["generation"] == 1
                                            core = capture(boundary["pid"])
                                            assert core["exe_sha256"] == pin
                                            assert core["pid"] in children(host.pid)
                                            stdin_pipe = os.readlink(
                                                f"/proc/{core['pid']}/fd/0"
                                            )
                                            assert stdin_pipe.startswith("pipe:[")
                                            assert stdin_pipe in {
                                                os.readlink(p)
                                                for p in Path(
                                                    f"/proc/{host.pid}/fd"
                                                ).iterdir()
                                            }
                                            fd = os.pidfd_open(core["pid"])
                                            assert not select.select([fd], [], [], 0)[
                                                0
                                            ], "RED: child должен быть жив на границе"
                                            case |= dict(boundary=boundary, core=core)
                                            if cancel:
                                                case["stop"] = command(
                                                    host, "stop", "stop"
                                                )
                                                assert (
                                                    case["stop"]["status"]
                                                    == "Disconnected"
                                                )
                                                assert (
                                                    case["stop"][
                                                        "acknowledged_generation"
                                                    ]
                                                    == boundary["generation"]
                                                )
                                                assert select.select([fd], [], [], 5)[0]
                                                assert not Path(
                                                    f"/proc/{core['pid']}"
                                                ).exists()
                                                assert not children(
                                                    host.pid
                                                ) and available(port)
                                                assert stdin_pipe not in {
                                                    os.readlink(p)
                                                    for p in Path(
                                                        f"/proc/{host.pid}/fd"
                                                    ).iterdir()
                                                }
                                                case["stdin_fd_closed"] = True
                                            else:
                                                assert command(
                                                    host, "release", "release"
                                                )["delivered"]
                                                # Барьер one-shot: повторный Connect не нужен для контроля.
                                                case["connected"] = command(
                                                    host, "settled", "settled"
                                                )
                                                assert (
                                                    case["connected"]["status"]
                                                    == "Connected"
                                                )
                                                case["stop"] = command(
                                                    host, "stop", "stop"
                                                )
                                            # Новое поколение после late completion обязано реально подключиться.
                                            case["replacement"] = command(
                                                host, "connect", "connect"
                                            )
                                            assert (
                                                case["replacement"]["status"]
                                                == "Connected"
                                            )
                                            if cancel:
                                                replacement = children(host.pid)
                                                assert len(replacement) == 1
                                                replacement_core = capture(
                                                    next(iter(replacement))
                                                )
                                                assert (
                                                    replacement_core["exe_sha256"]
                                                    == pin
                                                )
                                                case["late_release"] = command(
                                                    host, "release", "release"
                                                )
                                                assert not case["late_release"][
                                                    "delivered"
                                                ], (
                                                    "RED: старый consumer ещё принимает completion"
                                                )
                                                assert children(host.pid) == replacement
                                                assert (
                                                    capture(replacement_core["pid"])
                                                    == replacement_core
                                                )
                                                case["replacement_core"] = (
                                                    replacement_core
                                                )
                                            command(host, "stop", "stop")
                                            assert not children(host.pid) and available(
                                                port
                                            )
                                            command(host, "shutdown", "closed")
                                            assert host.wait(timeout=5) == 0
                                        finally:
                                            if fd is not None:
                                                os.close(fd)
                                            if host.poll() is None:
                                                host.terminate()
                                                host.wait(timeout=10)
                        finally:
                            bus.terminate()
                            bus.wait(timeout=5)
                results.append(case)
                (output / "result.json").write_text(json.dumps(results, indent=2))
    assert len(results) == 8


if __name__ == "__main__":
    run(*(Path(arg).resolve() for arg in sys.argv[1:5]))
