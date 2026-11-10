"""Guest-only provenance файлов runtime и первичных Arch package notices."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import tarfile
from pathlib import Path


def run(args: list[str]) -> str:
    return subprocess.run(
        args,
        check=True,
        capture_output=True,
        text=True,
        env={**os.environ, "LC_ALL": "C"},
    ).stdout


def main() -> None:
    root = Path("/home/builder/arch-evidence")
    (root / "texts").mkdir(parents=True, exist_ok=True)
    runtime = json.loads(Path("/home/builder/runtime/runtime.json").read_text())
    owners = sorted(
        {owner for row in runtime["files"] for owner in row.get("owner", [])}
    )
    if not os.environ.get("YONIK_ARCH_CACHE_READY"):
        subprocess.run(["pacman", "-Sw", "--noconfirm", *owners], check=True)
    records = []
    for owner in owners:
        identity = run(["pacman", "-Q", owner]).strip()
        name, version = identity.split()
        candidates = sorted(
            Path("/var/cache/pacman/pkg").glob(name + "-" + version + "-*.pkg.tar.zst")
        )
        if len(candidates) != 1:
            raise ValueError(f"нет точного archive {identity}: {candidates}")
        archive = candidates[0]
        with archive.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        sync = run(["pacman", "-Sii", owner])
        fields = dict(
            line.split(" : ", 1) for line in sync.splitlines() if " : " in line
        )
        fields = {key.strip(): value.strip() for key, value in fields.items()}
        if fields.get("Version") != version or fields.get("SHA-256 Sum") != digest:
            raise ValueError("sync database/archive mismatch: " + identity)
        pkginfo = run(["bsdtar", "-xOf", str(archive), ".PKGINFO"])
        buildinfo = run(["bsdtar", "-xOf", str(archive), ".BUILDINFO"])
        sourcebase = next(
            line.split(" = ", 1)[1]
            for line in pkginfo.splitlines()
            if line.startswith("pkgbase = ")
        )
        texts = []
        for line in run(["pacman", "-Qql", owner]).splitlines():
            path = Path(line)
            if not line.startswith("/usr/share/licenses/") or not path.is_file():
                continue
            data = path.read_bytes()
            if len(data) > 4 * 1024 * 1024:
                raise ValueError("license file limit")
            sha = hashlib.sha256(data).hexdigest()
            (root / "texts" / sha).write_bytes(data)
            texts.append(
                {
                    "source_path": line,
                    "resolved_path": str(path.resolve()),
                    "file": "texts/" + sha,
                    "sha256": sha,
                }
            )
        wanted = {
            row["resolved_path"].lstrip("/"): row["sha256"]
            for row in runtime["files"]
            if owner in row.get("owner", [])
        }
        payloads = {}
        with tarfile.open(archive, mode="r|zst") as package:
            for member in package:
                if member.name not in wanted:
                    continue
                if not member.isfile() or member.size > 256 * 1024 * 1024:
                    raise ValueError("runtime payload type/size")
                stream = package.extractfile(member)
                assert stream is not None
                actual = hashlib.file_digest(stream, "sha256").hexdigest()
                if actual != wanted[member.name]:
                    raise ValueError("runtime file differs from package payload")
                payloads[member.name] = actual
        if set(payloads) != set(wanted):
            raise ValueError("runtime payload missing from archive: " + owner)
        records.append(
            {
                "name": name,
                "version": version,
                "pkgbase": sourcebase,
                "package_sha256": digest,
                "runtime_payload_sha256": payloads,
                "archive": archive.name,
                "sync_metadata": fields,
                "pkginfo": pkginfo,
                "buildinfo": buildinfo,
                "texts": texts,
                "limits": "Arch notices из package payload; source recipe checksum проверяется отдельно",
            }
        )
    # Common licenses являются отдельно атрибутированными текстами, без выбора ветви лицензии.
    common = []
    for path in sorted(Path("/usr/share/licenses/common").rglob("*")):
        if path.is_file():
            data = path.read_bytes()
            digest = hashlib.sha256(data).hexdigest()
            (root / "texts" / digest).write_bytes(data)
            common.append(
                {"source_path": str(path), "sha256": digest, "file": "texts/" + digest}
            )
    (root / "arch-packages.json").write_text(
        json.dumps(
            {
                "schema": 1,
                "runtime_binary_sha256": runtime["binary_sha256"],
                "records": records,
                "common_licenses": common,
                "common_licenses_owner": run(["pacman", "-Q", "licenses"]),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
