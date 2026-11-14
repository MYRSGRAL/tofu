"""Runtime-only TLS downloads с SHA sync DB и existing trusted keyring."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import quote

BASE = Path(__file__).resolve().parents[1]
SAVED = BASE / "local/validation/a4-userdata-142"
OUT = BASE / "docs/evidence/a4-userdata-142"
ENV = {"PATH": "/usr/bin", "LANG": "C", "HOME": str(SAVED)}


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(args: list[str]) -> str:
    process = subprocess.Popen(
        args, env=ENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
    )
    output, _ = process.communicate()
    with (OUT / "download-commands.jsonl").open("a") as log:
        log.write(
            json.dumps(
                {
                    "argv": args,
                    "pid": process.pid,
                    "exit": process.returncode,
                    "output": output,
                }
            )
            + "\n"
        )
    if process.returncode:
        raise RuntimeError(f"Команда отказала: {args[0]}, exit={process.returncode}")
    return output


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", default="fresh-runtime-plan.json")
    parser.add_argument("--record", default="runtime-inputs.json")
    parser.add_argument("--cleanup", default="fetch-cleanup.json")
    args = parser.parse_args()
    for value in (args.plan, args.record, args.cleanup):
        assert Path(value).name == value
    assert not (OUT / args.record).exists()
    SAVED.mkdir(mode=0o700, parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)
    cache = SAVED / "runtime-cache"
    cache.mkdir(mode=0o700, exist_ok=True)
    task = Path(
        tempfile.mkdtemp(
            prefix="a4-runtime-fetch-", dir="/var/tmp/codex-myrs/transient"
        )
    )
    lock = os.open(task, os.O_RDONLY | os.O_DIRECTORY)
    fcntl.flock(lock, fcntl.LOCK_EX)
    try:
        keyring = task / "gnupg"
        keyring.mkdir(mode=0o700)
        for name in ("pubring.gpg", "trustdb.gpg"):
            shutil.copyfile("/etc/pacman.d/gnupg/" + name, keyring / name)
        pins = {name: sha(keyring / name) for name in ("pubring.gpg", "trustdb.gpg")}
        plan = json.loads((SAVED / args.plan).read_text())
        rows = []
        for metadata in plan:
            name = metadata["NAME"][0]
            version = metadata["VERSION"][0]
            repo = metadata.get("A4_REPO", ["cachyos-extra-v3"])[0]
            repositories = {
                "cachyos-extra-v3": "https://mirror.cachyos.org/repo/x86_64_v3/cachyos-extra-v3/",
                "cachyos-core-v3": "https://mirror.cachyos.org/repo/x86_64_v3/cachyos-core-v3/",
                "cachyos-v3": "https://mirror.cachyos.org/repo/x86_64_v3/cachyos-v3/",
                "arch-core": "https://archlinux.cachyos.org/repo/core/os/x86_64/",
            }
            assert repo in repositories
            url = repositories[repo] + quote(metadata["FILENAME"][0], safe="")
            checksum = metadata["SHA256SUM"][0]
            assert len(checksum) == 64
            filename = metadata["FILENAME"][0]
            assert Path(filename).name == filename
            package = cache / filename
            assert not package.is_symlink()
            for address, target in (
                (url, package),
                (url + ".sig", package.with_name(package.name + ".sig")),
            ):
                if not target.exists():
                    run(
                        [
                            "curl",
                            "--proto",
                            "=https",
                            "--proto-redir",
                            "=https",
                            "--tlsv1.2",
                            "--fail",
                            "--location",
                            "--max-time",
                            "60",
                            "--output",
                            str(target),
                            address,
                        ]
                    )
            assert sha(package) == checksum
            sig = run(
                [
                    "gpg",
                    "--homedir",
                    str(keyring),
                    "--batch",
                    "--no-auto-key-retrieve",
                    "--no-auto-check-trustdb",
                    "--status-fd",
                    "1",
                    "--verify",
                    str(package) + ".sig",
                    str(package),
                ]
            )
            assert "[GNUPG:] VALIDSIG " in sig and any(
                "[GNUPG:] " + level in sig
                for level in ("TRUST_FULLY", "TRUST_ULTIMATE")
            )
            info = run(["bsdtar", "-xOf", str(package), ".PKGINFO"])
            assert f"pkgname = {name}\n" in info and f"pkgver = {version}\n" in info
            rows.append(
                {
                    "name": name,
                    "version": version,
                    "repository": repo,
                    "repository_db_sha256": sha(SAVED / (repo + ".db")),
                    "url": url,
                    "path": str(package),
                    "sha256": checksum,
                    "signature_sha256": sha(Path(str(package) + ".sig")),
                    "signature_status": sig,
                    "pkginfo": info,
                }
            )
        (OUT / args.record).write_text(
            json.dumps(
                {
                    "trusted_keyring": pins,
                    "repository_db_url": "https://mirror.cachyos.org/repo/x86_64_v3/cachyos-extra-v3/cachyos-extra-v3.db",
                    "repository_db_sha256": sha(SAVED / "cachyos-extra-v3.db"),
                    "packages": rows,
                },
                indent=2,
            )
            + "\n"
        )
    finally:
        run(["gpgconf", "--homedir", str(task / "gnupg"), "--kill", "all"])
        shutil.rmtree(task)
        (OUT / args.cleanup).write_text(
            json.dumps({"task": str(task), "exists": task.exists()}) + "\n"
        )
        os.close(lock)


if __name__ == "__main__":
    main()
