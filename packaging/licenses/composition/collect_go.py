"""Сбор первичных Go notices из versioned module zip; никаких Go builds."""

from __future__ import annotations

import base64
import concurrent.futures
import hashlib
import io
import json
import re
import stat
import sys
import urllib.error
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath

LIMIT = 128 * 1024 * 1024


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def escaped(value: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9._~+/@!\-]+", value) or ".." in value.split("/"):
        raise ValueError("недопустимая module identity")
    return "".join("!" + char.lower() if char.isupper() else char for char in value)


def collect(row: tuple[str, str, str], cache: Path, output: Path) -> dict:
    module, version, expected = row
    url = f"https://proxy.golang.org/{escaped(module)}/@v/{escaped(version)}.zip"
    try:
        with urllib.request.urlopen(url, timeout=90) as response:
            data = response.read(LIMIT + 1)
        if len(data) > LIMIT:
            raise ValueError("module zip превышает лимит")
        (cache / (sha(data) + ".zip")).write_bytes(data)
        prefix = module + "@" + version + "/"
        lines = []
        licenses = []
        pending_texts = {}
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            infos = archive.infolist()
            if (
                len(infos) > 100000
                or sum(i.file_size for i in infos) > 512 * 1024 * 1024
            ):
                raise ValueError("zip bounds")
            seen = set()
            for info in sorted(infos, key=lambda i: i.filename):
                path = PurePosixPath(info.filename)
                if (
                    not info.filename.startswith(prefix)
                    or ".." in path.parts
                    or "\\" in info.filename
                    or info.filename in seen
                    or stat.S_ISLNK(info.external_attr >> 16)
                ):
                    raise ValueError("zip path/duplicate/symlink")
                seen.add(info.filename)
                if info.is_dir():
                    continue
                raw = archive.read(info)
                lines.append(f"{sha(raw)}  {info.filename}\n".encode())
                if re.match(
                    r"^(?:LICEN[CS]ES?|COPYING|NOTICES?|AUTHORS|COPYRIGHTS?|PATENTS)(?:$|[._-]|[0-9])",
                    path.name.upper(),
                ):
                    if len(raw) > 4 * 1024 * 1024:
                        raise ValueError("notice превышает лимит")
                    digest = sha(raw)
                    pending_texts[digest] = raw
                    licenses.append(
                        {
                            "source_path": info.filename[len(prefix) :],
                            "file": "texts/" + digest,
                            "sha256": digest,
                        }
                    )
        actual = (
            "h1:" + base64.b64encode(hashlib.sha256(b"".join(lines)).digest()).decode()
        )
        if actual != expected:
            raise ValueError(f"module h1 mismatch: {actual}")
        for digest, raw in pending_texts.items():
            (output / "texts" / digest).write_bytes(raw)
        return {
            "module": module,
            "version": version,
            "h1": expected,
            "zip_url": url,
            "zip_sha256": sha(data),
            "h1_verified": True,
            "texts": licenses,
            "status": "observed" if licenses else "missing_text",
            "limits": "Имена notices не доказывают полноту встроенных уведомлений/статического кода",
        }
    except (OSError, ValueError, urllib.error.URLError, zipfile.BadZipFile) as error:
        return {
            "module": module,
            "version": version,
            "h1": expected,
            "zip_url": url,
            "status": "unresolved",
            "error": str(error),
        }


def main() -> None:
    manifest, cache_name, output_name = sys.argv[1:]
    cache, output = Path(cache_name), Path(output_name)
    cache.mkdir(exist_ok=True)
    (output / "texts").mkdir(parents=True, exist_ok=True)
    report = json.loads(Path(manifest).read_text())
    rows = sorted(
        {
            tuple(row[1:4])
            for core in report["cores"].values()
            for row in core["go"]["modules"]
            if row[0] == "dep"
        }
    )
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        records = list(pool.map(lambda row: collect(row, cache, output), rows))
    result = {
        "schema": 1,
        "input_manifest_sha256": sha(Path(manifest).read_bytes()),
        "records": records,
        "main_sources": "unresolved: main не связан с zip через dependency h1",
        "legal_status": "не юридическое заключение",
    }
    (output / "go-sources.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        json.dumps(
            {
                status: sum(r["status"] == status for r in records)
                for status in ("observed", "missing_text", "unresolved")
            }
        )
    )


if __name__ == "__main__":
    main()
