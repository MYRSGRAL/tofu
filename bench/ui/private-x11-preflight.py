#!/usr/bin/env python3
"""Короткая частная X11-проверка; не физический benchmark и не idle acceptance."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import select
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
import ui_benchmark as driver


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--xvfb", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads((args.artifacts / "manifest.json").read_text())
    for toolkit in ("qt", "gtk", "slint"):
        name = f"yonik-bench-{toolkit}"
        if digest(args.artifacts / name) != manifest["binary"][name]["sha256"]:
            parser.error(f"SHA-256: {name}")
    args.output.mkdir(parents=True, exist_ok=False)
    os.environ.update(
        DISPLAY="",
        XDG_SESSION_TYPE="x11",
        QT_QPA_PLATFORM="xcb",
        QT_QUICK_BACKEND="software",
        GDK_BACKEND="x11",
        GSK_RENDERER="cairo",
        GTK_A11Y="none",
        LIBGL_ALWAYS_SOFTWARE="1",
        GALLIUM_DRIVER="llvmpipe",
        MESA_LOADER_DRIVER_OVERRIDE="llvmpipe",
        WINIT_UNIX_BACKEND="x11",
    )
    os.environ.pop("WAYLAND_DISPLAY", None)
    records = []
    evidence = dict(
        scope="private-X11 software-only; P01/P02 background potential noise",
        spontaneous_frames_observable=False,
        source_tree_scope="bench/ui subtree only",
        xvfb_sha256=digest(args.xvfb),
        environment={
            k: os.environ[k]
            for k in (
                "QT_QPA_PLATFORM",
                "QT_QUICK_BACKEND",
                "GDK_BACKEND",
                "GSK_RENDERER",
                "LIBGL_ALWAYS_SOFTWARE",
                "GALLIUM_DRIVER",
            )
        },
        records=records,
    )
    cancellation = driver.Cancellation()
    read_fd, write_fd = os.pipe()
    server = None
    try:
        with (args.output / "xvfb.stderr").open("xb") as log:
            server = subprocess.Popen(
                [
                    str(args.xvfb),
                    "-displayfd",
                    str(write_fd),
                    "-screen",
                    "0",
                    "1024x768x24",
                    "-nolisten",
                    "tcp",
                    "-ac",
                    "-noreset",
                ],
                pass_fds=(write_fd,),
                stdout=log,
                stderr=log,
                start_new_session=True,
            )
        os.close(write_fd)
        write_fd = -1
        if not select.select([read_fd], [], [], 10)[0]:
            raise RuntimeError("Xvfb displayfd timeout")
        display = os.read(read_fd, 64).decode().strip()
        if not display.isdecimal():
            raise RuntimeError("Xvfb displayfd invalid")
        os.environ["DISPLAY"] = ":" + display
        evidence["display"] = os.environ["DISPLAY"]
        for toolkit in ("qt", "gtk", "slint"):
            for rows in (1000, 10000):
                cancellation.check()
                record = dict(
                    toolkit=toolkit,
                    rows=rows,
                    valid=False,
                    reports=[],
                    negative_controls=[],
                )
                records.append(record)
                child = None
                try:
                    child = driver.Child(
                        [
                            str(args.artifacts / f"yonik-bench-{toolkit}"),
                            "--rows",
                            str(rows),
                        ],
                        args.output / f"{toolkit}-{rows}.stderr",
                    )
                    record["pid"] = child.process.pid
                    oracle = driver.Oracle(rows)
                    report, _ = child.ack(15)
                    oracle.validate(report, "frame")
                    record["first_frame_ms"] = (
                        time.monotonic_ns() - child.spawn_ns
                    ) / 1e6
                    record["reports"].append(report)
                    for field, value in (
                        ("page_hash", "0000000000000000"),
                        ("event", "hidden"),
                    ):
                        corrupted = dict(report, **{field: value})
                        try:
                            oracle.validate(corrupted, "frame")
                        except driver.BenchmarkError:
                            record["negative_controls"].append(field)
                        else:
                            raise RuntimeError(f"Oracle accepted corruption: {field}")
                    for command in (
                        driver.query(),
                        driver.query("server-0.example.invalid"),
                        dict(op="select", id=1),
                        dict(op="favorite", id=1, value=True),
                        driver.query(favorites_only=True),
                        driver.query(),
                        driver.query(sort="name"),
                        dict(op="scroll", index=199),
                        dict(op="hide"),
                        dict(op="show"),
                    ):
                        event = oracle.apply(command)
                        start = time.monotonic_ns()
                        child.send(command, 15)
                        report, _ = child.ack(15)
                        oracle.validate(report, event)
                        if report["toolkit"] != toolkit:
                            raise RuntimeError("Toolkit mismatch")
                        record["reports"].append(
                            dict(report, latency_ms=(time.monotonic_ns() - start) / 1e6)
                        )
                    record["process"] = driver.sample_process(child.process.pid)
                    child.send(dict(op="quit"), 15)
                    child.finish(15)
                    record["valid"] = True
                except (driver.BenchmarkError, OSError, RuntimeError) as error:
                    record["error"] = str(error)
                finally:
                    if child is not None:
                        child.close()
                        record["reaped"] = child.process.returncode is not None
                        record["proc_absent"] = not Path(
                            f"/proc/{child.process.pid}"
                        ).exists()
    finally:
        if server is not None:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=5)
            evidence["xvfb_reaped"] = server.returncode is not None
            evidence["xvfb_proc_absent"] = not Path(f"/proc/{server.pid}").exists()
        os.close(read_fd)
        if write_fd >= 0:
            os.close(write_fd)
        cancellation.restore()
        (args.output / "evidence.json").write_text(
            json.dumps(evidence, ensure_ascii=False, indent=2) + "\n"
        )
    return 0 if len(records) == 6 and all(r["valid"] for r in records) else 1


if __name__ == "__main__":
    raise SystemExit(main())
