"""Actual mtree4 Qkk/RED/restore и real dependency/hook prefix acceptance."""

from __future__ import annotations

import fcntl
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

import a4_candidate_lifecycle as base
from a4_prefix_runtime import attempt, populate

OUT = base.BASE / "docs/evidence/a4-mtree4"
NEW = Path(
    "/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-mtree4/package.tar"
)


def qkk(args: list[str], expected: int, label: str) -> None:
    process = subprocess.Popen(
        [*args, "-Qkk", "yonik"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    output, _ = process.communicate()
    (OUT / (label + ".json")).write_text(
        json.dumps(
            {"pid": process.pid, "exit": process.returncode, "output": output}, indent=2
        )
        + "\n"
    )
    assert process.returncode == expected
    assert "no mtree" not in output
    if expected == 0:
        assert "0 altered files" in output
    else:
        assert "mismatch" in output.lower()


def main() -> None:
    base.OUT = OUT
    base.NEW = NEW
    assert (
        base.sha(NEW)
        == "26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237"
    )
    assert (
        base.sha(base.OLD)
        == "e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0"
    )
    assert (
        base.command(["vercmp", "0.1.0.r20261110-2", "0.1.0.r20261111-1"]).strip()
        == "-1"
    )
    task = Path(
        tempfile.mkdtemp(prefix="a4-mtree4-", dir="/var/tmp/codex-myrs/transient")
    )
    lock = os.open(task, os.O_RDONLY | os.O_DIRECTORY)
    fcntl.flock(lock, fcntl.LOCK_EX)
    root = task / "root"
    root.mkdir()
    try:
        populate(root, OUT)
        for name in (
            "var/lib/pacman",
            "var/log",
            "var/cache/pacman/pkg",
            "etc/pacman.d/hooks",
            "etc/pacman.d/gnupg",
        ):
            (root / name).mkdir(parents=True, exist_ok=True)
        config = root / "etc/pacman.conf"
        config.write_text("[options]\nArchitecture = x86_64\nSigLevel = Never\n")
        args = [
            "unshare",
            "--user",
            "--map-root-user",
            "--mount",
            "--fork",
            "pacman",
            "--config",
            str(config),
            "--root",
            str(root),
            "--dbpath",
            str(root / "var/lib/pacman"),
            "--logfile",
            str(root / "var/log/pacman.log"),
            "--cachedir",
            str(root / "var/cache/pacman/pkg"),
            "--gpgdir",
            str(root / "etc/pacman.d/gnupg"),
            "--hookdir",
            str(root / "etc/pacman.d/hooks"),
            "--noconfirm",
        ]
        states = {}
        for phase, package in [("install", base.OLD), ("upgrade", NEW)]:
            base.command([*args, "-U", "-dd", str(package)])
            qkk(args, 0, phase + "-qkk")
            states[phase] = base.snapshot(root, package)
        path = root / "usr/share/applications/cloud.highpriority.yonik.desktop"
        path.write_bytes(path.read_bytes() + b"\n# own corruption fixture\n")
        qkk(args, 1, "content-red")
        base.command([*args, "-U", "-dd", str(NEW)])
        qkk(args, 0, "content-restored-green")
        path.chmod(0o600)
        qkk(args, 1, "mode-red")
        path.chmod(0o644)
        qkk(args, 0, "mode-restored-green")
        attempt(root, OUT)
        for phase, package in [("rollback", base.OLD), ("uninstall", None)]:
            base.command(
                [*args, "-U", "-dd", str(package)]
                if package
                else [*args, "-R", "-dd", "yonik"]
            )
            if package:
                qkk(args, 0, phase + "-qkk")
            states[phase] = base.snapshot(root, package)
        assert not (root / "usr/bin/yonik-desktop").exists()
        (OUT / "snapshots.json").write_text(json.dumps(states, indent=2) + "\n")
    finally:
        if (root / "var/log/pacman.log").exists():
            shutil.copyfile(root / "var/log/pacman.log", OUT / "pacman.log")
        # bwrap PID namespace завершается вместе с waited supervisor; иных spawn нет.
        shutil.rmtree(task)
        (OUT / "cleanup.json").write_text(
            json.dumps(
                {"task": str(task), "exists": task.exists(), "children_waited": True}
            )
            + "\n"
        )
        os.close(lock)


if __name__ == "__main__":
    main()
