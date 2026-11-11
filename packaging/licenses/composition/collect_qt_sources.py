"""Primary Qt git data; git archive checksum как в makepkg, без сборки Qt."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
import tarfile
from pathlib import Path


def main() -> None:
    cache, output = map(Path, sys.argv[1:])
    recipes = json.loads((output / "arch-recipes.json").read_text())
    records = []
    for name, repo in [
        ("qt6-base", "qtbase"),
        ("qt6-declarative", "qtdeclarative"),
        ("qt6-svg", "qtsvg"),
    ]:
        recipe = next(row for row in recipes if row["pkgbase"] == name)
        raw_recipe = (output / recipe["file"]).read_text()
        expected = re.search(r"sha256sums=\('([0-9a-f]{64})'", raw_recipe).group(1)
        directory = cache / repo
        url = f"https://code.qt.io/qt/{repo}.git"
        if not directory.exists():
            subprocess.run(
                [
                    "git",
                    "clone",
                    "--bare",
                    "--depth=1",
                    "--branch",
                    "v6.12.0",
                    url,
                    str(directory),
                ],
                check=True,
            )
        commit = subprocess.check_output(
            ["git", "-C", str(directory), "rev-parse", "v6.12.0^{}"], text=True
        ).strip()
        archive_path = cache / (repo + ".tar")
        with archive_path.open("wb") as archive:
            subprocess.run(
                [
                    "git",
                    "-c",
                    "core.abbrev=no",
                    "-C",
                    str(directory),
                    "archive",
                    "--format",
                    "tar",
                    "v6.12.0",
                ],
                check=True,
                stdout=archive,
            )
        with archive_path.open("rb") as archive:
            actual = hashlib.file_digest(archive, "sha256").hexdigest()

        texts = []
        with tarfile.open(archive_path) as archive:
            for member in archive:
                path = Path(member.name)
                if member.isfile() and (
                    re.match(
                        r"^(?:LICEN[CS]ES?|COPYING|NOTICES?|AUTHORS|COPYRIGHTS?|PATENTS)(?:$|[._-]|[0-9])",
                        path.name.upper(),
                    )
                    or "LICENSES" in path.parts
                    or path.name in {"qt_attribution.json", "qt_attributions.json"}
                ):
                    if member.size > 4 * 1024 * 1024:
                        raise ValueError("Qt notice size")
                    stream = archive.extractfile(member)
                    assert stream is not None
                    raw = stream.read()
                    digest = hashlib.sha256(raw).hexdigest()
                    (output / "texts" / digest).write_bytes(raw)
                    texts.append(
                        {
                            "source_path": member.name,
                            "sha256": digest,
                            "file": "texts/" + digest,
                        }
                    )
        records.append(
            {
                "pkgbase": name,
                "version": "6.12.0",
                "source_url": url,
                "tag": "v6.12.0",
                "commit": commit,
                "git_archive_sha256": actual,
                "arch_source_checksum": expected,
                "checksum_matches": actual == expected,
                "arch_recipe_sha256": recipe["sha256"],
                "texts": texts,
                "limits": "prepare patches закреплены в raw recipe; archive checksum относится к upstream до patches",
            }
        )
        (output / "qt-sources.json").write_text(
            json.dumps(records, ensure_ascii=False, indent=2) + "\n"
        )
        print(name, commit, len(texts), flush=True)


if __name__ == "__main__":
    main()
