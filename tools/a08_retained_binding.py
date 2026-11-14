#!/usr/bin/env python3
"""Fail-closed source gate A-08; не является GUI/runtime приёмкой."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def compare(root: Path, archive: Path, paths: list[str]) -> list[dict[str, str]]:
    """Читает архив без извлечения; missing/duplicate/link отклоняются."""
    differences = []
    with tarfile.open(archive, "r:gz") as source:
        members: dict[str, list[tarfile.TarInfo]] = {}
        for member in source.getmembers():
            members.setdefault(member.name, []).append(member)
        for name in paths:
            current = digest((root / name).read_bytes())
            candidates = members.get(name, [])
            archived = "missing-or-ambiguous"
            if len(candidates) == 1 and candidates[0].isfile():
                stream = source.extractfile(candidates[0])
                assert stream is not None
                with stream:
                    archived = digest(stream.read())
            if current != archived:
                differences.append(
                    {"path": name, "current": current, "archived": archived}
                )
    return differences


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("retained", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    bindings = json.loads(
        (ROOT / "docs/validation/p03-integrated-bindings.json").read_text()
    )["sha256"]
    archive = args.retained / "source.tar.gz"
    binary = args.retained / "p03_integrated"
    integrity = all(
        digest(path.read_bytes()) == bindings["local/validation/p03-integrated/" + path.name]
        for path in (archive, binary)
    )
    tracked = subprocess.check_output(
        ["git", "ls-files", "-z", "crates", "Cargo.toml", "Cargo.lock"], cwd=ROOT
    ).decode().strip("\0").split("\0")
    differences = compare(ROOT, archive, tracked)
    result = {
        "acceptance": False,
        "kind": "retained-source-preflight-only",
        "base": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "retained_integrity": integrity,
        "binary_sha256": digest(binary.read_bytes()),
        "archive_sha256": digest(archive.read_bytes()),
        "checked_paths": len(tracked),
        "differences": differences,
        "reuse_admitted": integrity and not differences,
        "runtime_executed": False,
    }
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    return 0 if result["reuse_admitted"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
