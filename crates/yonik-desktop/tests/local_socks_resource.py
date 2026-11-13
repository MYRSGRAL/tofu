"""P-05 component witness: 100 циклов actual DesktopHost, без host services."""

from __future__ import annotations

import hashlib
import json
import os
import select
import socket
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from engine_compile import UUID
from engine_host import https_server, peer


def children(pid: int) -> set[int]:
    result: set[int] = set()
    for task in Path(f"/proc/{pid}/task").iterdir():
        try:
            result.update(map(int, (task / "children").read_text().split()))
        except FileNotFoundError:
            continue
    return result


def identity(pid: int) -> str:
    return Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()[19]


def sample(pid: int, cycle: int) -> dict:
    root = Path(f"/proc/{pid}")
    rollup = {
        line.split(":")[0]: int(line.split()[1])
        for line in (root / "smaps_rollup").read_text().splitlines()
        if line.split()[0].endswith(":")
    }
    heap_rss = 0
    heap = False
    for line in (root / "smaps").read_text().splitlines():
        if line and line[0] in "0123456789abcdef" and "-" in line.split()[0]:
            heap = line.endswith("[heap]")
        elif heap and line.startswith("Rss:"):
            heap_rss += int(line.split()[1])
    return dict(
        cycle=cycle,
        starttime=identity(pid),
        fd=len(list((root / "fd").iterdir())),
        threads=len(list((root / "task").iterdir())),
        children=sorted(children(pid)),
        rss_kib=rollup["Rss"],
        pss_kib=rollup["Pss"],
        private_dirty_kib=rollup["Private_Dirty"],
        heap_rss_kib=heap_rss,
    )


def oracle(rows: list[dict]) -> dict:
    warm = rows[10:]
    metrics = {}
    for key in (
        "fd",
        "threads",
        "rss_kib",
        "pss_kib",
        "private_dirty_kib",
        "heap_rss_kib",
    ):
        values = [row[key] for row in warm]
        slope = statistics.linear_regression(
            [row["cycle"] for row in warm], values
        ).slope
        metrics[key] = dict(
            min=min(values),
            max=max(values),
            end=values[-1],
            delta=values[-1] - values[0],
            slope_per_cycle=slope,
        )
    red = (
        any(row["children"] for row in rows)
        or metrics["fd"]["max"] > warm[0]["fd"]
        or metrics["threads"]["max"] > warm[0]["threads"]
    )
    # Это triage-порог стенда, не абсолютный budget спецификации.
    memory_review = any(
        metrics[key]["delta"] > 8192 and metrics[key]["slope_per_cycle"] > 32
        for key in ("pss_kib", "private_dirty_kib", "heap_rss_kib")
    )
    return dict(
        result="RED"
        if red
        else "REVIEW"
        if memory_review
        else "BOUNDED_COMPONENT_WITNESS",
        metrics=metrics,
        memory_limit="8 MiB end delta AND 32 KiB/cycle; рост ниже порога также сохранён, это не доказательство отсутствия любых leaks",
    )


def command(process: subprocess.Popen, text: str, expected: str) -> None:
    if text:
        process.stdin.write(text + "\n")
        process.stdin.flush()
    if not select.select([process.stdout], [], [], 30)[0]:
        raise TimeoutError(expected)
    if process.stdout.readline().strip() != expected:
        raise RuntimeError("Нет ACK " + expected)


def main() -> None:
    binary, engines, output = (Path(arg).resolve() for arg in sys.argv[1:4])
    output.mkdir(parents=True, exist_ok=True)
    # Канарейка использует реальные собственные FD; production не получает fault.
    held = []
    canary = []
    try:
        for cycle in range(101):
            held.append(open("/dev/null", "rb"))
            canary.append(sample(os.getpid(), cycle))
        result = oracle(canary)
        assert result["result"] == "RED"
        (output / "canary.json").write_text(
            json.dumps(dict(summary=result, raw=canary), indent=2)
        )
    finally:
        for handle in held:
            handle.close()
    with tempfile.TemporaryDirectory(prefix="y-p05-") as temporary:
        root = Path(temporary)
        bus = subprocess.Popen(
            ["dbus-daemon", "--session", "--nofork", "--print-address=1"],
            stdout=subprocess.PIPE,
            text=True,
        )
        try:
            assert select.select([bus.stdout], [], [], 5)[0]
            address = bus.stdout.readline().strip()
            env = os.environ | {
                "HOME": str(root),
                "XDG_RUNTIME_DIR": str(root),
                "XDG_CONFIG_HOME": str(root / "config"),
                "XDG_DATA_HOME": str(root / "data"),
                "XDG_CACHE_HOME": str(root / "cache"),
                "DBUS_SESSION_BUS_ADDRESS": address,
                "DBUS_SYSTEM_BUS_ADDRESS": address,
            }
            with https_server(root, "health", 0, "127.0.0.1") as https:
                env["SSL_CERT_FILE"] = str(root / "health.ca.pem")
                for engine, directory in (
                    ("sing-box", "sing-box-1.14.4"),
                    ("xray", "xray-26.10.31"),
                ):
                    with socket.socket() as reserve:
                        reserve.bind(("127.0.0.1", 0))
                        peer_port = reserve.getsockname()[1]
                    with socket.socket() as reserve:
                        reserve.bind(("127.0.0.1", 0))
                        port = reserve.getsockname()[1]
                    core = engines / directory / engine
                    with peer(
                        core,
                        engine,
                        root,
                        port=peer_port,
                        target_ports=str(https.server_port),
                    ):
                        data = root / engine
                        data.mkdir(mode=0o700)
                        rows = []
                        tracked = []
                        connected = []
                        with (output / f"{engine}.stderr").open("w") as errors:
                            process = subprocess.Popen(
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
                            try:
                                command(process, "", "ready")
                                rows.append(sample(process.pid, 0))
                                for cycle in range(1, 101):
                                    https.requested.clear()
                                    command(process, "connect", "connect")
                                    assert https.requested.is_set(), (
                                        "Нет известного HTTPS через pinned peer"
                                    )
                                    owned = children(process.pid)
                                    assert len(owned) == 1
                                    pid = owned.pop()
                                    start = identity(pid)
                                    connected.append(
                                        dict(
                                            cycle=cycle,
                                            host=sample(process.pid, cycle),
                                            core=sample(pid, cycle),
                                        )
                                    )
                                    (output / f"{engine}.connected.json").write_text(
                                        json.dumps(connected, indent=2)
                                    )
                                    pidfd = os.pidfd_open(pid)
                                    try:
                                        command(process, "stop", "stop")
                                        assert select.select([pidfd], [], [], 5)[0], (
                                            "Core жив после Stop"
                                        )
                                        assert (
                                            not Path(f"/proc/{pid}").exists()
                                            or identity(pid) != start
                                        ), "Core не reaped"
                                    finally:
                                        os.close(pidfd)
                                    with socket.socket() as rebound:
                                        rebound.setsockopt(
                                            socket.SOL_SOCKET, socket.SO_REUSEADDR, 1
                                        )
                                        rebound.bind(("127.0.0.1", port))
                                    time.sleep(0.05)
                                    rows.append(sample(process.pid, cycle))
                                    tracked.append(
                                        dict(
                                            cycle=cycle,
                                            pid=pid,
                                            starttime=start,
                                            reaped=True,
                                            rebound=True,
                                            https=True,
                                        )
                                    )
                                    (output / f"{engine}.raw.json").write_text(
                                        json.dumps(rows, indent=2)
                                    )
                                process.stdin.write("logs\n")
                                process.stdin.flush()
                                assert select.select([process.stdout], [], [], 20)[0]
                                logs = json.loads(process.stdout.readline())
                                (output / f"{engine}.logs.json").write_text(
                                    json.dumps(logs, indent=2)
                                )
                                command(process, "shutdown", "closed")
                                assert process.wait(timeout=10) == 0
                            finally:
                                if process.poll() is None:
                                    # Только собственный Popen; EOF инициирует Guard shutdown.
                                    process.stdin.close()
                                    process.wait(timeout=25)
                                process.stdout.close()
                                if not process.stdin.closed:
                                    process.stdin.close()
                        summary = oracle(rows)
                        (output / f"{engine}.json").write_text(
                            json.dumps(
                                dict(
                                    summary=summary,
                                    tracked=tracked,
                                    core_sha256=hashlib.sha256(
                                        core.read_bytes()
                                    ).hexdigest(),
                                    closed=True,
                                ),
                                indent=2,
                            )
                        )
                        assert summary["result"] != "RED", summary
        finally:
            bus.terminate()
            bus.wait(timeout=5)
            bus.stdout.close()
    (output / "binary.sha256").write_text(
        hashlib.sha256(binary.read_bytes()).hexdigest() + "\n"
    )


if __name__ == "__main__":
    main()
