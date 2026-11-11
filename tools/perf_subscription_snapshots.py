"""P-04 delta: production subscription/repository в isolated ephemeral DB."""

import argparse
import hashlib
import json
import os
import platform
import resource
import selectors
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path


def run_child(command: list[str], body: bytes, timeout: float) -> dict:
    """Вывод ограничен pipe-reader; file limit отдельно разрешает ephemeral SQLite."""
    if body or not 0 < timeout <= 60:
        raise ValueError("Неверный вход resource child")

    def limits() -> None:
        resource.setrlimit(resource.RLIMIT_FSIZE, (64 * 1024 * 1024,) * 2)
        resource.setrlimit(resource.RLIMIT_CPU, (60, 60))

    started = time.monotonic()
    child = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
        preexec_fn=limits,
    )
    data = bytearray()
    cancelled = False
    with selectors.DefaultSelector() as selector:
        selector.register(child.stdout, selectors.EVENT_READ)
        while True:
            if selector.select(0.005):
                chunk = os.read(child.stdout.fileno(), 65537 - len(data))
                data.extend(chunk)
            if len(data) > 65536 or time.monotonic() - started >= timeout:
                if not cancelled:
                    os.killpg(child.pid, signal.SIGKILL)
                    cancelled = True
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                child.returncode = os.waitstatus_to_exitcode(status)
                break
        data.extend(child.stdout.read(65537 - len(data)))
    child.stdout.close()
    # Только каталог этого reaped child; SIGKILL не выполняет Rust cleanup.
    leftover = Path(tempfile.gettempdir()) / f"yonik-p04-snapshots-{child.pid}"
    if leftover.exists():
        shutil.rmtree(leftover)
    if child.returncode or cancelled or len(data) > 65536:
        raise RuntimeError(
            f"child exit={child.returncode}, cancelled={cancelled}: {data.decode(errors='replace')}"
        )
    return {
        "wall_s": time.monotonic() - started,
        "cpu_s": usage.ru_utime + usage.ru_stime,
        "peak_rss_kib": usage.ru_maxrss,
        "cancelled": False,
        "exit": child.returncode,
        "result": json.loads(data),
    }


def validate(result: dict, count: int) -> None:
    """Refusal/частичный прогон никогда не считается успешным commit/import."""
    if (
        result["sources"] != 20
        or result["successful_commits"] != 80
        or result["disk_imported"] != count * 4
        or result["staged_imported"] != count * 4
        or result["compiled"] != [count * 4, count * 4]
        or not result["refusal_preserved"]
        or not result["cancel_probe"]["aborted_future_committed"]
    ):
        raise ValueError("Неполный actual прогон")
    if len(result["phases"]) != 4:
        raise ValueError("Неполные раунды")
    retained = [p["state"]["rss_kib"] for p in result["phases"][1:]]
    if max(retained) > 192 * 1024 or max(retained) - min(retained) > 8 * 1024:
        raise ValueError("Retained memory вышла за budget/диапазон после прогрева")
    for phase in result["phases"]:
        if phase["state"]["snapshots"] != 20 or phase["state"]["profiles"] != count:
            raise ValueError("Накопление snapshots/profiles")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    files = (
        subprocess.check_output(["git", "ls-files", "crates", "Cargo.lock"])
        .decode()
        .splitlines()
    )
    files += [
        __file__,
        "tools/tests/test_perf_subscription_snapshots.py",
        "crates/yonik-linux/examples/subscription_snapshot_resource.rs",
    ]
    manifest = {
        "seed": 407,
        "fixture_sha256": {
            f"{n}/{r}/{s}": hashlib.sha256(
                "".join(
                    f"trojan://fixture-407@r{r}-s{s}-{i}.example.invalid:443\n"
                    for i in range(n // 20)
                ).encode()
            ).hexdigest()
            for n in (1000, 10000)
            for r in (0, 1, 2, 3, 99, 100)
            for s in range(20)
        },
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"]).decode().strip(),
        "platform": platform.platform(),
        "loadavg": os.getloadavg(),
        "process_names": subprocess.check_output(["ps", "-eo", "comm"])
        .decode()
        .splitlines(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "sources": {p: hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in files},
    }
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    rows = []
    for count in (1000, 10000):
        for repeat in range(4):
            row = {
                "count": count,
                "repeat": repeat,
                "warmup": repeat == 0,
                **run_child([str(binary), str(count)], b"", 60),
            }
            if row["cancelled"]:
                raise RuntimeError("Actual прогон превысил timeout")
            validate(row["result"], count)
            rows.append(row)
            (args.output / "runs.json").write_text(json.dumps(rows, indent=2) + "\n")


if __name__ == "__main__":
    main()
