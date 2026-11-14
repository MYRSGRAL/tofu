"""Directory-flocked disposable stand для remaining A4 GUI/data gates."""

from __future__ import annotations

import fcntl
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

import a4_candidate_lifecycle as c
from a4_userdata_prefix import populate, sha

BASE = Path(__file__).resolve().parents[1]
OUT = BASE / "docs/evidence/a4-userdata-142/session7"
SAVED = BASE / "local/validation/a4-userdata-142"
NEW = Path(
    "/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-mtree4/package.tar"
)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=False)
    assert (
        sha(NEW) == "26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237"
    )
    assert (
        sha(c.OLD) == "e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0"
    )
    task = Path(
        tempfile.mkdtemp(prefix="a4-userdata-", dir="/var/tmp/codex-myrs/transient")
    )
    fd = os.open(task, os.O_RDONLY | os.O_DIRECTORY)
    fcntl.flock(fd, fcntl.LOCK_EX)
    root = task / "root"
    root.mkdir()
    worker = None
    try:
        populate(root, OUT)
        home = root / "home/fixture"
        for name in ("text", "file"):
            (home / "storage" / (name + ".txt")).write_text(
                f"trojan://fixture-{name}@example.invalid:443#{name}"
            )
        shutil.copyfile(
            Path(__file__).with_name("a4_userdata_worker.py"), home / "worker.py"
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
            (root / name).mkdir(mode=0o700, parents=True, exist_ok=True)
        config = root / "etc/pacman.conf"
        # Только accepted SHA-pinned Yonik archives; downloaded runtimes отдельно проверены GPG.
        config.write_text(
            "[options]\nArchitecture = x86_64\nSigLevel = Required DatabaseOptional\nLocalFileSigLevel = Optional TrustedOnly\n"
        )
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
        c.OUT = OUT
        c.command([*args, "-U", "-dd", str(c.OLD)])
        host = {
            name: os.readlink("/proc/self/ns/" + name)
            for name in ("user", "mnt", "pid", "net")
        }
        command = [
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
        env = {
            "PATH": "/usr/bin",
            "HOME": "/home/fixture",
            "XDG_RUNTIME_DIR": "/home/fixture/runtime",
            "XDG_CONFIG_HOME": "/home/fixture/config",
            "XDG_DATA_HOME": "/home/fixture/data",
            "XDG_CACHE_HOME": "/home/fixture/cache",
            "DISPLAY": ":99",
            "QT_QPA_PLATFORM": "xcb",
            "QT_QUICK_BACKEND": "software",
            "QT_QUICK_CONTROLS_STYLE": "Basic",
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/home/fixture/runtime/bus",
            "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/home/fixture/runtime/no-system-bus",
            "A4_HOST_NAMESPACES": json.dumps(host),
        }
        for key, value in env.items():
            command.extend(["--setenv", key, value])
        command += ["/usr/bin/python", "/home/fixture/worker.py"]
        with (OUT / "worker.log").open("w") as log:
            worker = subprocess.Popen(
                command, stdout=log, stderr=log, start_new_session=True
            )
            (OUT / "active.json").write_text(
                json.dumps(
                    {
                        "task": str(task),
                        "root": str(root),
                        "pid": worker.pid,
                        "argv": command,
                        "pacman": args,
                        "host_namespace": host,
                    },
                    indent=2,
                )
                + "\n"
            )
            code = worker.wait(timeout=2400)
            (OUT / "worker-exit.json").write_text(
                json.dumps({"pid": worker.pid, "exit": code}) + "\n"
            )
    finally:
        if worker is not None and worker.poll() is None:
            worker.terminate()
            try:
                worker.wait(timeout=10)
            except subprocess.TimeoutExpired:
                worker.kill()
                worker.wait()
        home = root / "home/fixture"
        if (home / "evidence").exists():
            shutil.copytree(home / "evidence", OUT / "guest", dirs_exist_ok=False)
        private = SAVED / ("private-" + task.name)
        if home.exists():
            private.mkdir(mode=0o700)
            for name in ("storage", "data", "config"):
                if (home / name).exists():
                    shutil.copytree(home / name, private / name)
            if (home / "keyring-password").exists():
                shutil.copyfile(home / "keyring-password", private / "keyring-password")
                (private / "keyring-password").chmod(0o600)
        if (root / "var/log/pacman.log").exists():
            shutil.copyfile(root / "var/log/pacman.log", OUT / "pacman.log")
        shutil.rmtree(task)
        (OUT / "cleanup.json").write_text(
            json.dumps(
                {
                    "task": str(task),
                    "exists": task.exists(),
                    "private_saved": str(private),
                    "children_waited": True,
                }
            )
            + "\n"
        )
        os.close(fd)


if __name__ == "__main__":
    main()
