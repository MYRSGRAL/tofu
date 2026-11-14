"""Ограниченная host-кампания: три полных P01, по одной private VM на повтор."""

import glob
import json
import os
import shutil
import signal
import subprocess
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
EVIDENCE = Path("/home/myrs/projects/yonik/local/validation/p01-178")
BINARY = Path(
    "/home/myrs/projects/yonik/local/artifacts/p01-178/yonik-desktop-observer"
)
FIRST = Path("/var/tmp/codex-myrs/transient/p01-178-credential-vm")


def call(
    ssh: list[str], command: str, payload: bytes | None = None, timeout: int = 60
) -> bytes:
    return subprocess.run(
        ssh + [command], input=payload, capture_output=True, check=True, timeout=timeout
    ).stdout


def context() -> dict:
    result = {"time": time.time(), "loadavg": Path("/proc/loadavg").read_text().strip()}
    for key, pattern in [
        ("power", "/sys/class/power_supply/*/online"),
        ("governors", "/sys/devices/system/cpu/cpu*/cpufreq/scaling_governor"),
        ("temperature", "/sys/class/thermal/thermal_zone*/temp"),
    ]:
        result[key] = {}
        for name in glob.glob(pattern):
            try:
                result[key][name] = Path(name).read_text().strip()
            except OSError:
                result[key][name] = None
    return result


def read_connection(path: Path) -> dict:
    info = json.loads(path.read_text())
    if (
        not isinstance(info, dict)
        or not isinstance(info.get("ssh"), list)
        or not info["ssh"]
        or any(
            not isinstance(arg, str) or not arg or "\0" in arg for arg in info["ssh"]
        )
        or type(info.get("vm_pid")) is not int
        or not 0 < info["vm_pid"] < 2**31
    ):
        raise ValueError("Invalid VM connection metadata")
    return info


def main() -> None:
    terminal = {"state": "FAILED", "repeats": []}
    try:
        for index in range(1, 4):
            root = (
                FIRST
                if index == 1
                else Path(f"/var/tmp/codex-myrs/transient/p01-178-vm-{index}")
            )
            output = EVIDENCE / f"repeat-{index}"
            output.mkdir()
            vm = None
            vm_log = (output / "vm.log").open("wb")
            if index >= 1:
                env = dict(
                    os.environ, YONIK_VM_ROOT=str(root), YONIK_VM_EVIDENCE=str(output)
                )
                vm = subprocess.Popen(
                    ["python", "tools/format4_gui_vm.py"],
                    cwd=REPO,
                    env=env,
                    stdout=vm_log,
                    stderr=vm_log,
                )
            complete = False
            ssh = None
            info = None
            try:
                connection = root / "connection.json"
                deadline = time.monotonic() + 600
                while not connection.exists():
                    if time.monotonic() > deadline or (
                        vm is not None and vm.poll() is not None
                    ):
                        raise TimeoutError("VM setup deadline")
                    time.sleep(5)
                info = read_connection(connection)
                ssh = info["ssh"]
                for source, destination in [
                    (BINARY, "/usr/local/bin/yonik-desktop"),
                    (
                        REPO / "tools/format4_gui_guest.py",
                        "/home/fixture/format4_gui_guest.py",
                    ),
                    (
                        REPO / "tools/p01_observer_measure.py",
                        "/home/fixture/p01_observer_measure.py",
                    ),
                ]:
                    call(
                        ssh,
                        "cat > " + destination + "; chmod 755 " + destination,
                        source.read_bytes(),
                    )
                if index >= 1:
                    call(
                        ssh,
                        "runuser -u fixture -- python /home/fixture/format4_gui_guest.py",
                        b'{"action":"init"}',
                    )
                call(ssh, "sysctl -w kernel.sched_schedstats=1")
                (output / "versions.txt").write_bytes(
                    call(ssh, "uname -a; pacman -Q; sysctl kernel.sched_schedstats")
                )
                # Fixture имеет уже существующий Secret Service и настоящий tray.
                with (output / "measurement.stderr").open("wb") as stderr:
                    task = subprocess.Popen(
                        ssh
                        + [
                            "runuser -u fixture -- python /home/fixture/p01_observer_measure.py"
                        ],
                        stdout=subprocess.DEVNULL,
                        stderr=stderr,
                    )
                    deadline = time.monotonic() + 2050
                    with (output / "host-context.jsonl").open("w") as samples:
                        while task.poll() is None:
                            samples.write(json.dumps(context()) + "\n")
                            samples.flush()
                            if time.monotonic() > deadline:
                                task.terminate()
                                task.wait(timeout=10)
                                raise TimeoutError("Measurement deadline")
                            try:
                                task.wait(timeout=5)
                            except subprocess.TimeoutExpired:
                                pass
                    if task.returncode:
                        raise RuntimeError("Measurement failed")
                for name in ["terminal.json", "samples.jsonl", "gui.log"]:
                    (output / name).write_bytes(
                        call(ssh, "cat /home/fixture/p01-result/" + name)
                    )
                state = json.loads((output / "terminal.json").read_text())
                if state["state"] != "COMPLETE":
                    raise RuntimeError("Incomplete repeat")
                terminal["repeats"].append({"repeat": index, "state": state["state"]})
                complete = True
            finally:
                if not complete:
                    terminal["preserved_root"] = str(root)
                try:
                    if not complete and ssh is not None:
                        for name in ["terminal.json", "samples.jsonl", "gui.log"]:
                            try:
                                (output / name).write_bytes(
                                    call(ssh, "cat /home/fixture/p01-result/" + name)
                                )
                            except (subprocess.SubprocessError, OSError) as error:
                                terminal.setdefault("harvest_errors", []).append(
                                    type(error).__name__
                                )
                finally:
                    if root.is_dir():
                        (root / "finish").touch()
                    if vm is not None:
                        try:
                            try:
                                vm.wait(timeout=60)
                            except subprocess.TimeoutExpired:
                                # Wrapper: poweroff ≤300 с, затем wait20 + TERM/wait15.
                                # 360 с включает весь его finally и reap QEMU.
                                vm.send_signal(signal.SIGINT)
                                vm.wait(timeout=360)
                        finally:
                            vm_log.close()
                    else:
                        deadline = time.monotonic() + 60
                        while Path(f"/proc/{info['vm_pid']}").exists():
                            if time.monotonic() > deadline:
                                raise TimeoutError("First VM reap deadline")
                            time.sleep(1)
                    vm_log.close()
                    if info is not None and Path(f"/proc/{info['vm_pid']}").exists():
                        raise RuntimeError("VM remains active; refusing cleanup")
                    # Только собственный восстановимый overlay; ELF/evidence вне root.
                    if complete:
                        shutil.rmtree(root)
        terminal["state"] = "COMPLETE"
    finally:
        (EVIDENCE / "campaign-terminal.json").write_text(
            json.dumps(terminal, indent=2) + "\n"
        )


if __name__ == "__main__":
    main()
