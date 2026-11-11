"""Связывает runtime Arch package BUILDINFO с exact-tag первичным PKGBUILD."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import subprocess
import sys
import urllib.parse
from pathlib import Path


def fetch(url: str, path: Path) -> bytes:
    subprocess.run(
        [
            "curl",
            "-4",
            "--http1.1",
            "-fLsS",
            "--connect-timeout",
            "20",
            "--max-time",
            "60",
            "--max-filesize",
            str(4 * 1024 * 1024),
            url,
            "-o",
            str(path),
        ],
        check=True,
        capture_output=True,
    )
    return path.read_bytes()


def collect(item: tuple[str, str, str], cache: Path, output: Path) -> dict:
    name, version, expected = item
    tag = version.replace(":", "-")
    project = urllib.parse.quote("archlinux/packaging/packages/" + name, safe="")
    api = f"https://gitlab.archlinux.org/api/v4/projects/{project}"
    tag_url = api + "/repository/tags/" + urllib.parse.quote(tag, safe="")
    try:
        metadata = json.loads(fetch(tag_url, cache / (name + "-tag.json")))
        commit = metadata["commit"]["id"]
        url = api + "/repository/files/PKGBUILD/raw?ref=" + commit
        raw = fetch(url, cache / (name + "-PKGBUILD"))
        digest = hashlib.sha256(raw).hexdigest()
        if digest != expected:
            raise ValueError("PKGBUILD hash не совпал с BUILDINFO")
        (output / "recipes" / digest).write_bytes(raw)
        return {
            "pkgbase": name,
            "version": version,
            "tag": tag,
            "commit": commit,
            "tag_url": tag_url,
            "recipe_url": url,
            "sha256": digest,
            "file": "recipes/" + digest,
            "status": "verified",
            "source_pins": "source/checksum arrays сохранены raw; PKGBUILD не исполнялся",
        }
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        return {
            "pkgbase": name,
            "version": version,
            "expected_sha256": expected,
            "tag_url": tag_url,
            "status": "unresolved",
            "error": str(error),
        }


def main() -> None:
    cache, output = map(Path, sys.argv[1:])
    (output / "recipes").mkdir(exist_ok=True)
    packages = json.loads((output / "arch/arch-packages.json").read_text())
    items = set()
    for record in packages["records"]:
        fields = dict(
            line.split(" = ", 1)
            for line in record["buildinfo"].splitlines()
            if line.startswith(("pkgver = ", "pkgbuild_sha256sum = "))
        )
        items.add((record["pkgbase"], fields["pkgver"], fields["pkgbuild_sha256sum"]))
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        records = list(
            pool.map(lambda item: collect(item, cache, output), sorted(items))
        )
    (output / "arch-recipes.json").write_text(
        json.dumps(records, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        {
            status: sum(x["status"] == status for x in records)
            for status in ("verified", "unresolved")
        }
    )


if __name__ == "__main__":
    main()
