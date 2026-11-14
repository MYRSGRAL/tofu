"""Ограниченный file/DB lifecycle accepted архивов в собственном user namespace."""

from __future__ import annotations

import fcntl
import hashlib
import io
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
OUT = BASE / "docs/evidence/a4-candidate-lifecycle"
REGISTRY = Path(
    "/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-3"
)
OLD = Path(
    "/home/myrs/.t3/worktrees/yonik/t3-0e71b13a-c31bb75af8e3d15e/local/validation/arch-clean/yonik-0.1.0.r20261110-2-x86_64.pkg.tar.zst"
)
NEW = REGISTRY / "package.tar"


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def command(args: list[str]) -> str:
    process = subprocess.Popen(
        args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
    )
    output, _ = process.communicate()
    with (OUT / "commands.jsonl").open("a") as stream:
        stream.write(
            json.dumps(
                {
                    "argv": args,
                    "pid": process.pid,
                    "exit": process.returncode,
                    "output": output,
                },
                ensure_ascii=False,
            )
            + "\n"
        )
    if process.returncode:
        raise RuntimeError(f"Команда завершилась с кодом {process.returncode}: {args}")
    return output


def snapshot(root: Path, archive: Path | None) -> dict:
    files = {}
    if archive:
        listing = command(["bsdtar", "-tf", str(archive)]).splitlines()
        for name in listing:
            name = name.removeprefix("./")
            if not name.startswith("usr/") or name.endswith("/"):
                continue
            path = root / name
            if path.is_file():
                info = path.stat()
                files[name] = {
                    "sha256": sha(path),
                    "mode": oct(info.st_mode & 0o7777),
                    "uid": info.st_uid,
                    "gid": info.st_gid,
                }
        # Полная сверка archive member bytes/modes; UID 0 namespace == own UID host.
        with (
            tarfile.open(archive)
            if archive == NEW
            else tarfile.open(
                fileobj=io.BytesIO(
                    subprocess.check_output(["zstd", "-dc", str(archive)])
                )
            ) as source
        ):
            for member in source:
                if member.isfile() and member.name.removeprefix("./").startswith(
                    "usr/"
                ):
                    name = member.name.removeprefix("./")
                    payload = source.extractfile(member)
                    assert payload is not None
                    assert (
                        files[name]["sha256"]
                        == hashlib.file_digest(payload, "sha256").hexdigest()
                    )
                    assert files[name]["mode"] == oct(member.mode)
                    assert files[name]["uid"] == os.getuid()
                    assert files[name]["gid"] == os.getgid()
    return {
        "files": files,
        "data": {
            str(p.relative_to(root)): {
                "sha256": sha(p),
                "mode": oct(p.stat().st_mode & 0o777),
                "uid": p.stat().st_uid,
            }
            for p in (root / "home/fixture").rglob("*")
            if p.is_file()
        },
    }


def main() -> None:
    OUT.mkdir(exist_ok=True)
    assert (
        command(
            ["git", "merge-base", "HEAD", "2599fe70ca93ebd14baaa8932e9de69fc5d66a94"]
        ).strip()
        == "2599fe70ca93ebd14baaa8932e9de69fc5d66a94"
    )
    assert (
        sha(OLD) == "e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0"
    )
    assert (
        sha(NEW) == "229a71a54cf2d0e6e6b83396476acb3ea54138ff0b988b93110ce6b7a3f5c740"
    )
    inventory = json.loads(
        (
            BASE
            / "docs/evidence/linux-controller-rehearsal-14229843-msrv85-3/registry-inventory.json"
        ).read_text()
    )
    for row in inventory:
        assert sha(Path(row["path"])) == row["sha256"]
    assert command(["vercmp", "0.1.0.r20261110-2", "0.1.0.r20261111-1"]).strip() == "-1"
    versions = {
        str(p): command(["bsdtar", "-xOf", str(p), ".PKGINFO"]) for p in (OLD, NEW)
    }
    assert "pkgver = 0.1.0.r20261111-1" in versions[str(NEW)]
    task = Path(
        tempfile.mkdtemp(prefix="a4-lifecycle-", dir="/var/tmp/codex-myrs/transient")
    )
    lock = os.open(task, os.O_RDONLY)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        root = task / "root"
        for name in (
            "var/lib/pacman",
            "var/log",
            "var/cache/pacman/pkg",
            "etc/pacman.d/hooks",
            "etc/pacman.d/gnupg",
            "home/fixture",
        ):
            (root / name).mkdir(parents=True, exist_ok=True)
        config = root / "etc/pacman.conf"
        config.write_text("[options]\nArchitecture = x86_64\nSigLevel = Never\n")
        (root / "home/fixture/data-sentinel").write_text(
            "Только fixture: не production profile/Secret Service.\n"
        )
        (root / "home/fixture/data-sentinel").chmod(0o600)
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
        states = {"before": snapshot(root, None)}
        try:
            for label, archive in [
                ("install", OLD),
                ("upgrade", NEW),
                ("rollback", OLD),
            ]:
                command([*pacman, "-U", "-dd", str(archive)])
                command([*pacman, "-Qkk", "yonik"])
                states[label] = snapshot(root, archive)
            command([*pacman, "-R", "-dd", "yonik"])
            states["uninstall"] = snapshot(root, None)
            assert not (root / "usr/bin/yonik-desktop").exists()
            assert all(
                state["data"] == states["before"]["data"] for state in states.values()
            )
            (OUT / "snapshots.json").write_text(
                json.dumps(states, indent=2, ensure_ascii=False) + "\n"
            )
            (OUT / "result.json").write_text(
                json.dumps(
                    {
                        "file_db_lifecycle": "PASS",
                        "final_linux_acceptance": False,
                        "versions": versions,
                        "registry_files_verified": len(inventory),
                        "candidate_qkk": "exit 0, no mtree file; archive bytes/modes checked independently",
                        "scope": "same-UID userns pacman -dd; no installed dependencies/system hooks/PID1; synthetic data only",
                        "blockers": [
                            "Real installed system hooks/PID1/cgroup not exercised",
                            "Candidate GUI profile/original/revision/Secret Service continuity not exercised; PR147 witness is source-specific",
                        ],
                    },
                    indent=2,
                    ensure_ascii=False,
                )
                + "\n"
            )
        finally:
            shutil.copyfile(root / "var/log/pacman.log", OUT / "pacman.log") if (
                root / "var/log/pacman.log"
            ).exists() else None
            # Все синхронные дочерние процессы завершены до удаления собственного дерева.
            shutil.rmtree(task)
            (OUT / "cleanup.json").write_text(
                json.dumps(
                    {
                        "task": str(task),
                        "exists": task.exists(),
                        "child_processes": "all waited",
                    }
                )
                + "\n"
            )

    finally:
        os.close(lock)


if __name__ == "__main__":
    main()
