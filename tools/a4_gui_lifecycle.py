"""Полный owned prefix для actual GUI/private provider, без host data/bus."""

from __future__ import annotations

import fcntl
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

import a4_candidate_lifecycle as base
from a4_mtree4_lifecycle import NEW
from a4_prefix_runtime import populate

OUT = base.BASE / "docs/evidence/a4-mtree4/gui-run"


def main() -> None:
    OUT.mkdir(exist_ok=True)
    base.OUT = OUT
    base.NEW = NEW
    assert (
        base.sha(NEW)
        == "26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237"
    )
    task = Path(tempfile.mkdtemp(prefix="a4-gui-", dir="/var/tmp/codex-myrs/transient"))
    lock = os.open(task, os.O_RDONLY | os.O_DIRECTORY)
    fcntl.flock(lock, fcntl.LOCK_EX)
    root = task / "root"
    root.mkdir()
    worker = None
    try:
        populate(root, OUT)
        home = root / "home/fixture"
        shutil.copyfile(
            Path(__file__).with_name("a4_gui_worker.py"), home / "worker.py"
        )
        for name in ("text", "file"):
            (home / "storage" / (name + ".txt")).write_text(
                f"trojan://fixture-{name}@example.invalid:443#{name}"
            )
        (home / "bus.conf").write_text(
            '<busconfig><type>session</type><listen>unix:path=/home/fixture/runtime/bus</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
        )
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
        pacman = [
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
            "--cachedir",
            str(root / "var/cache/pacman/pkg"),
            "--logfile",
            str(root / "var/log/pacman.log"),
            "--gpgdir",
            str(root / "etc/pacman.d/gnupg"),
            "--hookdir",
            str(root / "etc/pacman.d/hooks"),
            "--noconfirm",
        ]
        for name, package in [("install", base.OLD), ("upgrade", NEW)]:
            base.command([*pacman, "-U", "-dd", str(package)])
            (OUT / (name + ".json")).write_text(
                json.dumps(base.snapshot(root, package), indent=2) + "\n"
            )
        args = [
            "bwrap",
            "--unshare-all",
            "--die-with-parent",
            "--bind",
            str(root),
            "/",
            "--ro-bind",
            str(root / "usr"),
            "/usr",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--clearenv",
        ]
        for key, value in {
            "PATH": "/usr/bin",
            "HOME": "/home/fixture",
            "XDG_RUNTIME_DIR": "/home/fixture/runtime",
            "XDG_CONFIG_HOME": "/home/fixture/config",
            "XDG_DATA_HOME": "/home/fixture/data",
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/home/fixture/runtime/bus",
            "QT_QUICK_BACKEND": "software",
            "QT_QUICK_CONTROLS_STYLE": "Basic",
        }.items():
            args.extend(["--setenv", key, value])
        args.extend(["/usr/bin/python", "/home/fixture/worker.py"])
        with (OUT / "worker.log").open("w") as log:
            worker = subprocess.Popen(
                args, stdout=log, stderr=log, start_new_session=True
            )
            (OUT / "active.json").write_text(
                json.dumps(
                    {
                        "task": str(task),
                        "root": str(root),
                        "pid": worker.pid,
                        "argv": args,
                    },
                    indent=2,
                )
                + "\n"
            )
            code = worker.wait(timeout=900)
            (OUT / "worker-exit.json").write_text(
                json.dumps({"pid": worker.pid, "exit": code}) + "\n"
            )
        for name, package in [("rollback", base.OLD), ("uninstall", None)]:
            base.command(
                [*pacman, "-U", "-dd", str(package)]
                if package
                else [*pacman, "-R", "-dd", "yonik"]
            )
            (OUT / (name + ".json")).write_text(
                json.dumps(base.snapshot(root, package), indent=2) + "\n"
            )
    finally:
        if worker is not None and worker.poll() is None:
            worker.terminate()
            try:
                worker.wait(timeout=10)
            except subprocess.TimeoutExpired:
                worker.kill()
                worker.wait()
        if (root / "home/fixture").exists():
            for p in (root / "home/fixture").glob("*.log"):
                shutil.copyfile(p, OUT / p.name)
            for name in ("pids.jsonl", "exits.json"):
                if (root / "home/fixture" / name).exists():
                    shutil.copyfile(root / "home/fixture" / name, OUT / name)
            saved = base.BASE / "local/validation/a4-mtree4-private"
            saved.mkdir(mode=0o700, parents=True, exist_ok=False)
            for name in ("storage", "data", "config"):
                shutil.copytree(root / "home/fixture" / name, saved / name)
        if (root / "var/log/pacman.log").exists():
            shutil.copyfile(root / "var/log/pacman.log", OUT / "pacman.log")
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
