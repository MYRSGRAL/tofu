"""Actual host SIGKILL; PID-1 supervisor наблюдает core до teardown namespace."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import select
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))

from engine_compile import UUID
from engine_host import https_server, peer
from local_socks_alternating import command
from local_socks_resource import children, identity

CORES = {
    "sing-box": (
        "sing-box-1.14.4/sing-box",
        "1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a",
    ),
    "xray": (
        "xray-26.10.31/xray",
        "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1",
    ),
}


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def available(port: int) -> bool:
    with socket.socket() as listener:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            listener.bind(("127.0.0.1", port))
        except OSError:
            return False
    return True


def owned_listener(pid: int, port: int) -> bool:
    inodes = {os.readlink(fd) for fd in Path(f"/proc/{pid}/fd").iterdir()}
    for row in Path(f"/proc/{pid}/net/tcp").read_text().splitlines()[1:]:
        fields = row.split()
        if fields[1] == f"0100007F:{port:04X}" and fields[3] == "0A":
            if f"socket:[{fields[9]}]" in inodes:
                return True
    return False


def capture(pid: int) -> dict:
    return dict(
        pid=pid,
        starttime=identity(pid),
        exe_sha256=hashlib.sha256(Path(f"/proc/{pid}/exe").read_bytes()).hexdigest(),
    )


def observe(core: dict, fd: int, port: int) -> dict:
    exited = bool(select.select([fd], [], [], 0)[0])
    return dict(
        exited=exited,
        same_identity=not exited and capture(core["pid"]) == core,
        listener_owned=not exited and owned_listener(core["pid"], port),
        port_available=available(port),
        supervisor_alive=True,
    )


def run(binary: Path, engines: Path, output: Path, engine: str = "sing-box") -> None:
    assert os.getpid() == 1, "Supervisor должен быть namespace init"
    relative, pin = CORES[engine]
    core_path = engines / relative
    assert hashlib.sha256(core_path.read_bytes()).hexdigest() == pin
    result: dict = dict(engine=engine, pin=pin, observations=[])
    try:
        with tempfile.TemporaryDirectory(prefix="p09-owned-") as temporary:
            root = Path(temporary)
            env = os.environ | {
                "HOME": str(root),
                "XDG_RUNTIME_DIR": str(root),
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
                    with https_server(root, "health", 0, "127.0.0.1") as https:
                        env["SSL_CERT_FILE"] = str(root / "health.ca.pem")
                        peer_port = free_port()
                        with peer(
                            core_path,
                            engine,
                            root,
                            port=peer_port,
                            target_ports=str(https.server_port),
                        ):
                            for mode in ("stop", "sigkill"):
                                port = free_port()
                                data = root / mode
                                data.mkdir(mode=0o700)
                                with (output / f"{mode}.stderr").open("w") as errors:
                                    host = subprocess.Popen(
                                        [
                                            str(binary),
                                            str(data),
                                            str(engines),
                                            engine,
                                            f"vless://{UUID}@127.0.0.1:{peer_port}#Synthetic",
                                            f"https://127.0.0.1:{https.server_port}/",
                                            str(port),
                                        ],
                                        env=env,
                                        stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE,
                                        stderr=errors,
                                        text=True,
                                    )
                                    tracked: list[tuple[dict, int]] = []
                                    case: dict = dict(mode=mode, port=port)
                                    result[mode] = case
                                    try:
                                        command(host, "", "ready")
                                        case["connected"] = command(
                                            host, "connect", "connect"
                                        )
                                        assert (
                                            case["connected"]["status"] == "Connected"
                                        )
                                        pids = children(host.pid)
                                        assert len(pids) == 1, pids
                                        core = capture(pids.pop())
                                        assert core["exe_sha256"] == pin
                                        fd = os.pidfd_open(core["pid"])
                                        tracked.append((core, fd))
                                        case["host"] = capture(host.pid)
                                        case["core"] = core
                                        case["live_negative_canary"] = observe(
                                            core, fd, port
                                        )
                                        assert case["live_negative_canary"] == dict(
                                            exited=False,
                                            same_identity=True,
                                            listener_owned=True,
                                            port_available=False,
                                            supervisor_alive=True,
                                        )
                                        if mode == "stop":
                                            case["stop_ack"] = command(
                                                host, "stop", "stop"
                                            )
                                            assert select.select([fd], [], [], 5)[0]
                                            assert not Path(
                                                f"/proc/{core['pid']}"
                                            ).exists()
                                            assert not children(host.pid)
                                            assert available(port)
                                            case["exit_reap_rebind"] = True
                                            command(host, "shutdown", "closed")
                                            assert host.wait(timeout=5) == 0
                                        else:
                                            host.kill()
                                            assert (
                                                host.wait(timeout=5) == -signal.SIGKILL
                                            )
                                            case["host_exit"] = -signal.SIGKILL
                                            start = time.monotonic()
                                            for offset in (0, 1, 2, 3):
                                                time.sleep(
                                                    max(
                                                        0,
                                                        start
                                                        + offset
                                                        - time.monotonic(),
                                                    )
                                                )
                                                observation = observe(core, fd, port)
                                                observation["elapsed"] = (
                                                    time.monotonic() - start
                                                )
                                                result["observations"].append(
                                                    observation
                                                )
                                            last = result["observations"][-1]
                                            assert last["elapsed"] <= 3.2, (
                                                "Превышена граница observation"
                                            )
                                            case["verdict"] = (
                                                "survives"
                                                if last["same_identity"]
                                                and last["listener_owned"]
                                                else "exits"
                                                if last["exited"]
                                                and last["port_available"]
                                                else "unknown"
                                            )
                                    finally:
                                        # На ошибке Connect сначала учитываем только собственных детей host.
                                        if host.poll() is None:
                                            known = {
                                                entry[0]["pid"] for entry in tracked
                                            }
                                            for pid in children(host.pid) - known:
                                                core = capture(pid)
                                                assert core["exe_sha256"] == pin
                                                tracked.append(
                                                    (core, os.pidfd_open(pid))
                                                )
                                            host.kill()
                                            host.wait(timeout=5)
                                        for core, fd in tracked:
                                            try:
                                                if not select.select([fd], [], [], 0)[
                                                    0
                                                ]:
                                                    assert capture(core["pid"]) == core
                                                    signal.pidfd_send_signal(
                                                        fd, signal.SIGKILL
                                                    )
                                                assert select.select([fd], [], [], 5)[0]
                                                try:
                                                    reaped, status = os.waitpid(
                                                        core["pid"], 0
                                                    )
                                                    case["supervisor_reaped"] = dict(
                                                        pid=reaped, status=status
                                                    )
                                                except ChildProcessError:
                                                    assert not Path(
                                                        f"/proc/{core['pid']}"
                                                    ).exists()
                                            finally:
                                                os.close(fd)
                                        assert available(port)
                                        case["final_port_available"] = True
                                        case["remaining_host_children"] = []
                                        host.stdin.close()
                                        host.stdout.close()
                finally:
                    bus.terminate()
                    bus.wait(timeout=5)
        assert not children(1), children(1)
        result["final_children_empty"] = True
    finally:
        (output / "result.json").write_text(json.dumps(result, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("engines", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--engine", choices=CORES, default="sing-box")
    parser.add_argument("--inside", action="store_true")
    args = parser.parse_args()
    binary, engines, output = (
        value.resolve() for value in (args.binary, args.engines, args.output)
    )
    output.mkdir(parents=True, exist_ok=True)
    if args.inside:
        run(binary, engines, output, args.engine)
    else:
        subprocess.run(
            [
                "unshare",
                "--map-current-user",
                "--pid",
                "--fork",
                "--mount-proc",
                "--",
                sys.executable,
                str(Path(__file__).resolve()),
                str(binary),
                str(engines),
                str(output),
                "--engine",
                args.engine,
                "--inside",
            ],
            check=True,
            timeout=120,
        )
