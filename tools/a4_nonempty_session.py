"""Связанный lifecycle на копии production-created schema4 userdata."""

from __future__ import annotations

import fcntl
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

import a4_candidate_lifecycle as common
from a4_nonempty_control import snapshot
from a4_userdata_prefix import populate, sha
from a4_userdata_session import NEW

BASE = Path(__file__).resolve().parents[1]
OUT = BASE / "docs/evidence/a4-nonempty-lifecycle-142/run1"
PRIVATE = BASE / "local/validation/a4-userdata-142/diagnostic-a4-backup-cause-5thkiqwh"


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=False)
    assert PRIVATE.stat().st_mode & 0o7777 == 0o700
    original_public, original_opaque = snapshot(PRIVATE)
    registry = NEW.parent
    frozen = json.loads((registry / "frozen.json").read_text())
    assert frozen["source"] == {
        "commit": "14229843f9a11bf98ec4f5992df00f3247102ea2",
        "tree": "7daa4ebb7cc25d3ceafab76809afc78753dc5ec3",
    }
    assert (
        subprocess.check_output(
            ["git", "rev-parse", frozen["source"]["commit"] + "^{tree}"], text=True
        ).strip()
        == frozen["source"]["tree"]
    )
    subprocess.run(
        [
            "git",
            "merge-base",
            "--is-ancestor",
            "17c9dcbc586d2bcf3a70f66ca200d9acb8ccd5ee",
            "HEAD",
        ],
        check=True,
    )
    (OUT / "original-state-before.json").write_text(
        json.dumps(original_public, indent=2) + "\n"
    )
    assert (
        sha(NEW) == "26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237"
    )
    task = Path(
        tempfile.mkdtemp(prefix="a4-nonempty-", dir="/var/tmp/codex-myrs/transient")
    )
    lock = os.open(task, os.O_RDONLY | os.O_DIRECTORY)
    fcntl.flock(lock, fcntl.LOCK_EX)
    root = task / "root"
    root.mkdir()
    worker = None
    try:
        setup = OUT / "setup"
        setup.mkdir()
        populate(root, setup)
        home = root / "home/fixture"
        for name in ("data", "storage", "config"):
            shutil.copytree(PRIVATE / name, home / name, dirs_exist_ok=True)
        shutil.copyfile(PRIVATE / "keyring-password", home / "keyring-password")
        (home / "keyring-password").chmod(0o600)
        # Legitimate existing production-created data, не fresh install и не injection.
        source = (BASE / "tools/a4_userdata_worker.py").read_text()
        old = "password = secrets.token_hex(32).encode()"
        assert source.count(old) == 1
        source = source.replace(
            old, 'password = (HOME / "keyring-password").read_bytes()'
        )
        old = 'elif action == "inspect":'
        assert source.count(old) == 1
        source = source.replace(
            old,
            'elif action == "clipboard-diagnostics":\n                        from a4_x11_diagnostics import read_safe_diagnostics\n                        result = read_safe_diagnostics()\n                    '
            + old,
        )
        witness = BASE / "local/validation/a4-userdata-142/a4_prefix_witness"
        assert (
            sha(witness)
            == "a059c0572bcb8df741dba63fa088ee40b43260141bf122582954e9e6bfc0207e"
        )
        destination = root / "usr/local/bin/a4_prefix_witness"
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(witness, destination)
        (home / "worker.py").write_text(source)
        shutil.copyfile(
            BASE / "tools/a4_x11_diagnostics.py", home / "a4_x11_diagnostics.py"
        )
        verify = home / "verification"
        (verify / "tools").mkdir(parents=True)
        (verify / "docs/specs").mkdir(parents=True)
        for name in ("check_diagnostic_export.py", "check_diagnostic_export_v2.py"):
            shutil.copyfile(BASE / "tools" / name, verify / "tools" / name)
        for name in (
            "diagnostic-message-catalog.json",
            "diagnostic-context-v2.json",
        ):
            shutil.copyfile(BASE / "docs/specs" / name, verify / "docs/specs" / name)
        additions = json.loads(
            (
                BASE
                / "docs/evidence/a4-userdata-142/session7/screenshot-tool-additions.json"
            ).read_text()
        )
        for row in additions:
            path = Path(row["path"])
            assert sha(path) == row["sha256"]
            target = root / str(path).lstrip("/")
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
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
        common.OUT = OUT
        assert (
            sha(common.OLD)
            == "e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0"
        )
        metadata = {
            label: common.command(["bsdtar", "-xOf", str(package), ".PKGINFO"])
            for label, package in [("old", common.OLD), ("candidate", NEW)]
        }
        versions = {
            label: next(
                line.split(" = ", 1)[1]
                for line in info.splitlines()
                if line.startswith("pkgver = ")
            )
            for label, info in metadata.items()
        }
        assert (
            common.command(["vercmp", versions["old"], versions["candidate"]]).strip()
            == "-1"
        )
        (OUT / "package-versions.json").write_text(
            json.dumps({"versions": versions, "metadata": metadata}, indent=2) + "\n"
        )
        common.command([*args, "-U", "-dd", str(common.OLD)])
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
        for key, value in {
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
        }.items():
            command.extend(["--setenv", key, value])
        command += ["/usr/bin/python", "/home/fixture/worker.py"]
        (OUT / "case-binding.json").write_text(
            json.dumps(
                {
                    "source": "14229843f9a11bf98ec4f5992df00f3247102ea2",
                    "candidate_sha256": sha(NEW),
                    "worker_sha256": sha(home / "worker.py"),
                    "python_reader_sha256": sha(home / "a4_x11_diagnostics.py"),
                    "retained_state": str(PRIVATE),
                    "diagnostic_readers_explicitly_closed": True,
                    "witness_sha256": sha(witness),
                    "old_package_sha256": sha(common.OLD),
                    "purpose": "legitimate pre-existing schema4 with 2 normalGUI profiles; no synthesis/rebuild",
                },
                indent=2,
            )
            + "\n"
        )
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
                    },
                    indent=2,
                )
                + "\n"
            )
            code = worker.wait(timeout=1800)
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
            shutil.copytree(home / "evidence", OUT / "guest")
        saved = BASE / "local/validation/a4-userdata-142" / ("retained-" + task.name)
        if home.exists():
            saved.mkdir(mode=0o700)
            for name in ("storage", "data", "config"):
                if (home / name).exists():
                    shutil.copytree(home / name, saved / name)
            if (home / "keyring-password").exists():
                shutil.copyfile(home / "keyring-password", saved / "keyring-password")
                (saved / "keyring-password").chmod(0o600)
        original_after, opaque_after = snapshot(PRIVATE)
        unchanged = (
            original_public == original_after and original_opaque == opaque_after
        )
        (OUT / "original-retained-check.json").write_text(
            json.dumps(
                {
                    "path": str(PRIVATE),
                    "unchanged": unchanged,
                    "mode": oct(PRIVATE.stat().st_mode & 0o7777),
                }
            )
            + "\n"
        )
        assert unchanged
        shutil.rmtree(task)
        (OUT / "cleanup.json").write_text(
            json.dumps(
                {
                    "task": str(task),
                    "exists": task.exists(),
                    "private_saved": str(saved),
                    "children_waited": True,
                }
            )
            + "\n"
        )
        os.close(lock)


if __name__ == "__main__":
    main()
