#!/usr/bin/env python3
"""Controlled A-08 correlation campaign; reused own runtime inputs, no installs."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def prepare(inputs: Path, observer: Path, work: Path, case: str) -> None:
    binding = json.loads(
        (ROOT / "docs/evidence/a08/runtime-bindings.json").read_text()
    )["sha256"]
    work.mkdir()
    for name in ("Xvfb", "helper"):
        source = inputs / name
        if (
            hashlib.sha256(source.read_bytes()).hexdigest()
            != binding[f"local/validation/a08-runtime/{name}"]
        ):
            raise ValueError("Retained runtime input mismatch")
        shutil.copy2(source, work / name)
    shutil.copy2(observer, work / "observer.so")
    shutil.copytree(inputs / "xdg", work / "xdg")
    (work / "empty-apps").mkdir()
    applications = work / "xdg/applications"
    for index in range(40):
        (applications / f"page-{index:02}.desktop").write_text(
            f"[Desktop Entry]\nType=Application\nName=A08 page {index:02}\n"
        )
    if case == "complete":
        (applications / "oversized.desktop").unlink()


def run(binary: Path, inputs: Path, observer: Path, output: Path) -> dict:
    results = {}
    for name, case in (
        ("limited", "limited"),
        ("complete", "complete"),
        ("drop", "drop"),
        ("hidden-scan", "hidden-scan"),
    ):
        work = output / name
        prepare(inputs, observer, work, case)
        command = [
            "/usr/bin/env",
            "-i",
            "PATH=/usr/bin:/bin",
            "/usr/bin/unshare",
            "--user",
            "--map-root-user",
            "--mount",
            "--pid",
            "--net",
            "--fork",
            "--kill-child=SIGKILL",
            "--mount-proc",
            "/usr/bin/dbus-run-session",
            "--",
            "/usr/bin/bash",
            str(ROOT / "tools/a08_runtime.sh"),
            str(work),
            str(binary),
            str(ROOT),
            case,
        ]
        with (work / "gui.log").open("w") as log:
            completed = subprocess.run(
                command, stdout=log, stderr=log, timeout=45, check=False
            )
        phases = [
            json.loads(line[4:])
            for line in (work / "gui.log").read_text().splitlines()
            if line.startswith("A08 {")
        ]
        expected_failure = {
            "drop": "deadline",
            "hidden-scan": "close/reopen no NEW admission",
        }.get(case)
        if expected_failure:
            valid = completed.returncode != 0 and any(
                p.get("reason") == expected_failure for p in phases
            )
        else:
            valid = (
                completed.returncode == 0
                and phases[-1]["phase"] == "completed"
                and not any(p["phase"] == "FAIL" for p in phases)
            )
        results[name] = {"exit": completed.returncode, "expected_result": valid}
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("inputs", type=Path)
    parser.add_argument("observer", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir()
    results = run(
        args.binary.resolve(),
        args.inputs.resolve(),
        args.observer.resolve(),
        args.output.resolve(),
    )
    (args.output / "runs.json").write_text(json.dumps(results, indent=2) + "\n")
    return 0 if all(r["expected_result"] for r in results.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
