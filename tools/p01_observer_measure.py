"""Один полный P01 повтор в credential VM: 900 hidden + 900 static-open."""

import json
import os
import signal
import socket
import subprocess
import time
from pathlib import Path


def measure(pid: int) -> dict:
    root = Path(f"/proc/{pid}")
    stat = (root / "stat").read_text().rsplit(")", 1)[1].split()
    memory = {}
    for line in (root / "smaps_rollup").read_text().splitlines():
        if line.startswith(("Pss:", "Private_Dirty:")):
            key, value, _unit = line.split()
            memory[key.rstrip(":")] = int(value)
    switches = 0
    wakeups = 0
    wakeups_supported = True
    children: set[str] = set()
    threads: dict[str, int | None] = {}
    for task in (root / "task").iterdir():
        try:
            children.update((task / "children").read_text().split())
            sched = (task / "sched").read_text().splitlines()
            keys = {
                a.strip(): b.strip()
                for line in sched
                if ":" in line
                for a, b in [line.split(":", 1)]
            }
            thread_stat = (task / "stat").read_text().rsplit(")", 1)[1].split()
            identity = f"{task.name}:{thread_stat[19]}"
            threads[identity] = (
                int(keys["nr_wakeups"]) if "nr_wakeups" in keys else None
            )
            switches += int(keys["nr_switches"])
            if "nr_wakeups" in keys:
                wakeups += int(keys["nr_wakeups"])
            else:
                wakeups_supported = False
        except FileNotFoundError:
            continue
    return {
        "pid": pid,
        "threads": threads,
        "cpu_ticks": int(stat[11]) + int(stat[12]),
        "start_ticks": int(stat[19]),
        "memory_kib": memory,
        "context_switches_proxy": switches,
        "scheduler_wakeups": wakeups if wakeups_supported else None,
        "wakeup_source": "/proc/PID/task/TID/sched nr_wakeups; guest sched_schedstats=1"
        if wakeups_supported
        else "unavailable; switches are proxy only",
        "children": sorted(children),
    }


def verify_frame_canary(before: dict, after: dict) -> None:
    if (
        not before["visible"]
        or not after["visible"]
        or after["frames"] <= before["frames"]
    ):
        raise RuntimeError("Frame oracle нечувствителен")


def verify_phase(phase: str, start: dict, end: dict) -> None:
    visible = {"hidden": False, "static-open": True}[phase]
    if (
        start["visible"] != visible
        or end["visible"] != visible
        or end["visibility_events"] != start["visibility_events"]
    ):
        raise RuntimeError("Visibility changed during phase")
    if phase == "hidden" and end["frames"] != start["frames"]:
        raise RuntimeError("Hidden frame gate")


def main() -> None:
    home = Path("/home/fixture")
    if Path.home() != home or os.getuid() == 0:
        raise RuntimeError("Только отдельный fixture user в credential VM")
    output = home / "p01-result"
    output.mkdir()
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(home / "runtime"),
        XDG_DATA_HOME=str(home / "data"),
        XDG_CONFIG_HOME=str(home / "config"),
        DBUS_SESSION_BUS_ADDRESS=f"unix:path={home}/runtime/bus",
        DISPLAY=":99",
        QT_QPA_PLATFORM="xcb",
        QT_QUICK_BACKEND="software",
        YONIK_CREDENTIAL_VM="disposable-v1",
    )
    client, child = socket.socketpair()
    client.settimeout(10)
    env["YONIK_P01_CONTROL_FD"] = str(child.fileno())
    infrastructure = json.loads((home / "pids.json").read_text())
    # tint2 — фактический tray host. Остальная инфраструктура отдельно.
    tracked = {"gui": None, **infrastructure}
    terminal = {"state": "FAILED", "stats_gate": "N_A", "phase_seconds": 900}
    with (output / "gui.log").open("wb") as log:
        process = subprocess.Popen(
            [
                "/usr/local/bin/yonik-desktop",
                "--data-dir",
                str(home / "p01-measure-storage"),
            ],
            env=env,
            pass_fds=(child.fileno(),),
            stdout=log,
            stderr=log,
            start_new_session=True,
        )
        tracked["gui"] = process.pid
        child.close()
        stream = client.makefile("rb")

        def command(value: str) -> dict:
            client.sendall(value.encode() + b"\n")
            reply = json.loads(stream.readline(4096))
            if reply["command"] != value or not reply["ok"]:
                raise RuntimeError("Control rejected")
            return reply

        try:
            deadline = time.monotonic() + 40
            while not command("Snapshot").get("ready"):
                if time.monotonic() > deadline:
                    raise TimeoutError("Ready deadline")
                time.sleep(0.5)
            command("Open")
            time.sleep(1)
            canary_before = command("Snapshot")
            command("VisibleFrameCanary")
            time.sleep(1)
            canary_after = command("Snapshot")
            verify_frame_canary(canary_before, canary_after)
            terminal["frame_canary"] = {"before": canary_before, "after": canary_after}
            phases = []
            with (output / "samples.jsonl").open("w") as samples:
                for phase, action, visible in [
                    ("hidden", "Hide", False),
                    ("static-open", "Open", True),
                ]:
                    command(action)
                    time.sleep(60)  # startup Qt thread pool expiry вне измерения
                    start_state = command("Snapshot")
                    if start_state["visible"] != visible or start_state.get("can_stop"):
                        raise RuntimeError("Visibility/no-connect precondition")
                    start = time.monotonic()
                    first = {name: measure(pid) for name, pid in tracked.items()}
                    next_sample = start
                    while True:
                        now = time.monotonic()
                        row = {
                            "phase": phase,
                            "elapsed": now - start,
                            "processes": {
                                name: measure(pid) for name, pid in tracked.items()
                            },
                            "loadavg": Path("/proc/loadavg").read_text().strip(),
                        }
                        samples.write(json.dumps(row) + "\n")
                        samples.flush()
                        if (
                            process.poll() is not None
                            or row["processes"]["gui"]["children"]
                        ):
                            raise RuntimeError("GUI exited or spawned child")
                        if now - start >= 900:
                            break
                        next_sample += 5
                        time.sleep(
                            max(0, min(next_sample, start + 900) - time.monotonic())
                        )
                    end_state = command("Snapshot")
                    verify_phase(phase, start_state, end_state)
                    phases.append(
                        {
                            "phase": phase,
                            "duration": time.monotonic() - start,
                            "start": start_state,
                            "end": end_state,
                            "first": first,
                            "last": row["processes"],
                        }
                    )
            command("Quit")
            code = process.wait(timeout=20)
            if code:
                raise RuntimeError("GUI quit failed")
            terminal.update(
                state="COMPLETE",
                phases=phases,
                reaped=True,
                returncode=code,
                clock_ticks=os.sysconf("SC_CLK_TCK"),
                tracked_pids=tracked,
                overhead="test-support observer ELF; VM xcb/software; external sampler every 5s",
            )
        finally:
            client.close()
            stream.close()
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
            (output / "terminal.json").write_text(json.dumps(terminal, indent=2) + "\n")


if __name__ == "__main__":
    main()
