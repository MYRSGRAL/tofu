"""VM-only proc witness с security properties production worker unit."""

from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path


def guard() -> None:
    assert os.environ.get("YONIK_PRODUCTION_HELPER_VM") == "1"
    assert Path("/sys/class/dmi/id/sys_vendor").read_text().strip() == "QEMU"
    if sys.argv[1] != "--probe":
        assert Path("/proc/1/comm").read_text().strip() == "systemd"
    marker = Path("/etc/yonik-production-helper-vm")
    assert marker.stat().st_uid == 0 and marker.stat().st_mode & 0o022 == 0
    assert marker.read_text() == "yonik-production-helper-disposable-v1\n"


def client() -> None:
    sockets = []
    for family, address in [(socket.AF_INET, "127.0.0.1"), (socket.AF_INET6, "::1")]:
        for kind in [socket.SOCK_STREAM, socket.SOCK_DGRAM]:
            item = socket.socket(family, kind)
            item.bind((address, 0))
            if kind == socket.SOCK_STREAM:
                item.listen()
            sockets.append(item)
    Path("/run/yonik-attribution-client/ready").write_text(str(os.getpid()))
    time.sleep(30)


def probe(pid: int) -> None:
    result = {}
    for name in ["exe", "fd", "ns/net"]:
        try:
            path = Path(f"/proc/{pid}/{name}")
            list(path.iterdir()) if name == "fd" else path.readlink()
        except OSError as error:
            result[name] = error.errno
    fd_cost = []
    for attempts in [1, 10]:
        started = time.perf_counter_ns()
        failures = 0
        for _ in range(attempts):
            try:
                list(Path(f"/proc/{pid}/fd").iterdir())
            except OSError:
                failures += 1
        fd_cost.append(
            {
                "attempts": attempts,
                "denied": failures,
                "elapsed_ns": time.perf_counter_ns() - started,
            }
        )
    kernel_tables = {}
    for name in ["tcp", "udp", "tcp6", "udp6"]:
        lines = Path(f"/proc/net/{name}").read_text().splitlines()
        kernel_tables[name] = {"header": lines[0], "rows": len(lines) - 1}
    status = Path("/proc/self/status").read_text()
    assert "CapEff:\t0000000000001000" in status
    assert "NoNewPrivs:\t1" in status
    assert len(result) == 3
    print(
        json.dumps(
            {
                "denied_errno": result,
                "captured_ns": time.time_ns(),
                "denied_fd_cost": fd_cost,
                "kernel_tables": kernel_tables,
                "worker_uid": os.getuid(),
                "status": status,
                "cgroup": Path("/proc/self/cgroup").read_text(),
                "mountinfo": Path("/proc/self/mountinfo").read_text(),
            }
        ),
        flush=True,
    )


def controller(output: Path) -> None:
    assert os.geteuid() == 0
    source = Path("/usr/lib/systemd/system/yonik-worker.service").read_text()
    script = str(Path(__file__).resolve())
    units = []
    try:
        subprocess.run(
            [
                "systemd-run",
                "--unit=yonik-attribution-client",
                "--property=User=yonik-vm-allowed",
                "--property=RuntimeDirectory=yonik-attribution-client",
                "--property=Environment=YONIK_PRODUCTION_HELPER_VM=1",
                sys.executable,
                script,
                "--client",
            ],
            check=True,
        )
        deadline = time.monotonic() + 5
        while not Path("/run/yonik-attribution-client/ready").exists():
            assert time.monotonic() < deadline
            time.sleep(0.05)
        pid = int(
            subprocess.check_output(
                [
                    "systemctl",
                    "show",
                    "--property=MainPID",
                    "--value",
                    "yonik-attribution-client.service",
                ]
            )
        )
        assert pid > 1 and Path(f"/proc/{pid}").stat().st_uid != 0
        for view in ["default", "invisible"]:
            name = f"yonik-attribution-probe-{view}.service"
            content = "\n".join(
                line
                for line in source.splitlines()
                if not line.startswith(
                    (
                        "Sockets=",
                        "StandardInput=",
                        "StandardOutput=",
                        "StandardError=",
                        "ExecStart=",
                    )
                )
            )
            content += f"\nExecStart={sys.executable} {script} --probe {pid}\nStandardOutput=journal\nStandardError=journal\nEnvironment=YONIK_PRODUCTION_HELPER_VM=1\nProtectProc={view}\n"
            path = Path("/run/systemd/system") / name
            assert not path.exists()
            path.write_text(content)
            units.append(path)
            output.mkdir(exist_ok=True)
            (output / f"{view}-unit-before.service").write_text(content)
            (output / f"{view}-manifest-before.json").write_text(
                json.dumps(
                    {
                        "production_unit_sha256": hashlib.sha256(
                            source.encode()
                        ).hexdigest(),
                        "fixture_unit_sha256": hashlib.sha256(
                            content.encode()
                        ).hexdigest(),
                        "script_sha256": hashlib.sha256(
                            Path(script).read_bytes()
                        ).hexdigest(),
                        "client_pid": pid,
                        "client_uid": Path(f"/proc/{pid}").stat().st_uid,
                    }
                )
            )
            subprocess.run(["systemctl", "daemon-reload"], check=True)
            started_ns = time.time_ns()
            subprocess.run(
                ["systemctl", "start", "--wait", name], check=True, timeout=10
            )
            journal = subprocess.check_output(
                ["journalctl", "-u", name, "--no-pager", "-o", "cat"], text=True
            )
            rows = [
                json.loads(line)
                for line in journal.splitlines()
                if line.startswith('{"denied_errno"')
            ]
            rows = [row for row in rows if row.get("captured_ns", 0) >= started_ns]
            assert len(rows) == 1
            (output / f"{view}-result.json").write_text(json.dumps(rows[0], indent=2))
    finally:
        subprocess.run(
            ["systemctl", "stop", "yonik-attribution-client.service"], check=True
        )
        for path in units:
            subprocess.run(["systemctl", "stop", path.name], check=True)
            path.unlink()
        subprocess.run(["systemctl", "daemon-reload"], check=True)


if __name__ == "__main__":
    guard()
    if sys.argv[1:] == ["--client"]:
        client()
    elif sys.argv[1] == "--probe":
        probe(int(sys.argv[2]))
    else:
        controller(Path(sys.argv[1]))
