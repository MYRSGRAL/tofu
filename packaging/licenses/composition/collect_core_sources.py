"""Первичные exact-ref исходники ядер; не доказательство воспроизводимости ELF."""

from __future__ import annotations

import hashlib
import io
import json
import re
import subprocess
import sys
import tarfile
from pathlib import Path, PurePosixPath


def main() -> None:
    cache, output = map(Path, sys.argv[1:3])
    toolchains = len(sys.argv) > 3 and sys.argv[3] == "toolchains"
    records = []
    items = [
        ("sing-box", "SagerNet/sing-box", "c005f35300ce76427dc285481ec5695f5b179cd2"),
        ("xray", "XTLS/Xray-core", "v26.10.31"),
    ]
    if toolchains:
        items = [
            (version, "golang/go", version) for version in ("go1.27.3", "go1.26.10")
        ]
    for name, repo, ref in items:
        api = f"https://api.github.com/repos/{repo}/commits/{ref}"
        meta = cache / (name + "-commit.json")
        subprocess.run(
            [
                "curl",
                "-4",
                "--http1.1",
                "-fLsS",
                "--max-time",
                "90",
                api,
                "-o",
                str(meta),
            ],
            check=True,
        )
        commit = json.loads(meta.read_text())["sha"]
        if len(commit) != 40 or any(c not in "0123456789abcdef" for c in commit):
            raise ValueError("неверный source commit")
        url = f"https://codeload.github.com/{repo}/tar.gz/{commit}"
        archive_path = cache / (name + "-source.tar.gz")
        subprocess.run(
            [
                "curl",
                "-4",
                "--http1.1",
                "-fLsS",
                "--max-time",
                "90",
                "--max-filesize",
                str(128 * 1024 * 1024),
                url,
                "-o",
                str(archive_path),
            ],
            check=True,
        )
        data = archive_path.read_bytes()
        texts = []
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            total = 0
            for index, member in enumerate(archive):
                total += member.size
                path = PurePosixPath(member.name)
                if (
                    index > 100000
                    or total > 512 * 1024 * 1024
                    or path.is_absolute()
                    or ".." in path.parts
                ):
                    raise ValueError("source archive bounds")
                if member.isfile() and re.match(
                    r"^(?:LICEN[CS]ES?|COPYING|NOTICES?|AUTHORS|COPYRIGHTS?|PATENTS)(?:$|[._-]|[0-9])",
                    path.name.upper(),
                ):
                    if member.size > 4 * 1024 * 1024:
                        raise ValueError("source notice limit")
                    stream = archive.extractfile(member)
                    assert stream is not None
                    raw = stream.read()
                    sha = hashlib.sha256(raw).hexdigest()
                    (output / "texts" / sha).write_bytes(raw)
                    texts.append(
                        {
                            "source_path": member.name.split("/", 1)[1],
                            "sha256": sha,
                            "file": "texts/" + sha,
                        }
                    )
        records.append(
            {
                "core": name,
                "ref": ref,
                "commit": commit,
                "commit_api": api,
                "source_url": url,
                "source_sha256": hashlib.sha256(data).hexdigest(),
                "texts": texts,
                "binary_binding": "buildinfo Go version; custom stdlib patches не исключены"
                if toolchains
                else "buildinfo vcs.revision, modified=false"
                if name == "sing-box"
                else "unresolved: main (devel), no VCS",
            }
        )
        (
            output
            / ("go-toolchain-sources.json" if toolchains else "core-sources.json")
        ).write_text(json.dumps(records, ensure_ascii=False, indent=2) + "\n")
        print(name, commit, len(texts), flush=True)


if __name__ == "__main__":
    main()
