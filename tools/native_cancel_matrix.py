#!/usr/bin/env python3
"""Изолированная bounded приёмка; oracle учитывает все процессы private PID namespace."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path


def snapshot(only_pid: int | None = None) -> dict[str, dict[str, int | str | None]]:
    rows = {}
    for path in Path("/proc").iterdir():
        if (
            not path.name.isdigit()
            or int(path.name) == os.getpid()
            or (only_pid is not None and int(path.name) != only_pid)
        ):
            continue
        try:
            stat = (path / "stat").read_text().rsplit(") ", 1)[1].split()
        except (FileNotFoundError, ProcessLookupError):
            continue
        # Zombie сохраняет identity, хотя smaps_rollup уже недоступен.
        row: dict[str, int | str | None] = {
            "state": stat[0],
            "fd": None,
            "threads": None,
            "pss": None,
            "private_dirty": None,
        }
        rows[f"{path.name}:{stat[19]}"] = row
        try:
            row["fd"] = len(list((path / "fd").iterdir()))
            row["threads"] = len(list((path / "task").iterdir()))
            memory = dict(
                line.split(":", 1)
                for line in (path / "smaps_rollup").read_text().splitlines()
                if ":" in line
            )
            row["pss"] = int(memory.get("Pss", "0").split()[0]) * 1024
            row["private_dirty"] = (
                int(memory.get("Private_Dirty", "0").split()[0]) * 1024
            )
        except (FileNotFoundError, ProcessLookupError):
            # Недоступные метрики не означают отсутствие процесса.
            pass
    return rows


def remaining(rows: dict, baseline: dict) -> dict:
    return {key: value for key, value in rows.items() if key not in baseline}


def inside(args: argparse.Namespace) -> None:
    # Отдельный proc mount обязателен: PID 1 — unshare supervisor, host PID невидимы.
    if Path("/proc/1/ns/pid").readlink() != Path("/proc/self/ns/pid").readlink():
        raise RuntimeError("Нет private proc")
    if os.readlink("/proc/self/ns/net") == args.parent_net:
        raise RuntimeError("Нет private net")
    if sorted(
        line.split(":", 1)[0].strip()
        for line in Path("/proc/net/dev").read_text().splitlines()[2:]
    ) != ["lo"]:
        raise RuntimeError("Ожидался только loopback")
    baseline = snapshot()
    results = []
    for engine in range(2):
        for axis in ("positive", "startup_cancel", "traffic_cancel", "peer_failure"):
            samples = []
            seen = set()
            with (args.output / f"{engine}-{axis}.log").open("w") as log:
                process = subprocess.Popen(
                    [args.binary, args.cores, axis, str(engine)], stdout=log, stderr=log
                )
                deadline = time.monotonic() + 20
                while process.poll() is None:
                    rows = remaining(snapshot(), baseline)
                    samples.append(rows)
                    seen.update(rows)
                    if time.monotonic() > deadline:
                        # Popen держит ещё не reaped child: PID reuse исключён.
                        process.kill()
                        process.wait()
                        raise RuntimeError("Deadline")
                    time.sleep(0.001)
                terminal = remaining(snapshot(), baseline)
                record = {
                    "engine": engine,
                    "axis": axis,
                    "exit": process.returncode,
                    "tracked_pid_starttime": sorted(seen),
                    "samples": samples,
                    "terminal_owned_group": terminal,
                }
                results.append(record)
                (args.output / "matrix.json").write_text(json.dumps(results, indent=2))
                if process.returncode or terminal:
                    raise RuntimeError(
                        f"Не принят {engine}/{axis}: {process.returncode}, {terminal}"
                    )
    fixture = subprocess.Popen(
        [
            sys.executable,
            "-c",
            'import time; data=bytearray(8*1024*1024); f=open("/dev/null"); time.sleep(10)',
        ]
    )
    try:
        time.sleep(0.1)
        retained = remaining(snapshot(), baseline)
        if not retained or not any(
            (row["private_dirty"] or 0) >= 8 * 1024 * 1024 for row in retained.values()
        ):
            raise RuntimeError("Negative oracle пропустил собственный retainer")
        (args.output / "negative.json").write_text(
            json.dumps({"detected": True, "owned_fixture": retained}, indent=2)
        )
    finally:
        fixture.terminate()
        fixture.wait()
    if remaining(snapshot(), baseline):
        raise RuntimeError("Fixture не убран")
    (args.output / "terminal.json").write_text(
        json.dumps(
            {
                "baseline": baseline,
                "terminal": snapshot(),
                "owned_resources_zero": True,
            },
            indent=2,
        )
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    parser.add_argument("cores")
    parser.add_argument("output", type=Path)
    parser.add_argument("--inside", action="store_true")
    parser.add_argument("--parent-net")
    args = parser.parse_args()
    if args.inside:
        inside(args)
        return
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {
        "source_head": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], text=True
        ).strip(),
        "sha256": {},
    }
    for path in [
        Path(args.binary),
        Path(__file__),
        Path("crates/yonik-desktop/examples/local_lifecycle_resource.rs"),
        *Path(args.cores).glob("*/*"),
    ]:
        if path.is_file():
            manifest["sha256"][str(path)] = hashlib.sha256(
                path.read_bytes()
            ).hexdigest()
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    command = [
        "unshare",
        "--user",
        "--map-root-user",
        "--net",
        "--pid",
        "--fork",
        "--kill-child=SIGKILL",
        "--mount-proc",
        "sh",
        "-c",
        'ip link set lo up && exec dbus-run-session -- "$@"',
        "matrix",
        sys.executable,
        str(Path(__file__).resolve()),
        args.binary,
        args.cores,
        str(args.output.resolve()),
        "--inside",
        "--parent-net",
        os.readlink("/proc/self/ns/net"),
    ]
    subprocess.run(command, check=True, timeout=190)


if __name__ == "__main__":
    main()
