"""Статические наблюдения над поставкой Linux; unknown не означает complete."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import re
import stat
import subprocess
import tarfile
import tempfile
from pathlib import Path, PurePosixPath

MAX_FILE = 128 * 1024 * 1024
MAX_ARCHIVE = 256 * 1024 * 1024
MAX_MEMBERS = 4096
ROOT = Path(__file__).resolve().parents[1]


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_file(path: Path, limit: int = MAX_FILE) -> bytes:
    """Не следует symlink ни в одном компоненте входного пути."""
    if ".." in path.parts:
        raise ValueError("недопустимый путь")
    path = path.absolute()
    directory = os.open(path.anchor, os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:-1]:
            descriptor = os.open(
                part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory
            )
            os.close(directory)
            directory = descriptor
        descriptor = os.open(
            path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory
        )
        with os.fdopen(descriptor, "rb") as stream:
            if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                raise ValueError("вход должен быть regular file")
            data = stream.read(limit + 1)
            if len(data) > limit:
                raise ValueError("превышен лимит входа")
            return data
    finally:
        os.close(directory)


def members(data: bytes) -> dict[str, bytes]:
    result = {}
    seen = set()
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
        for index, member in enumerate(archive):
            name = member.name
            path = PurePosixPath(name)
            if (
                index >= MAX_MEMBERS
                or path.is_absolute()
                or ".." in path.parts
                or "\\" in name
                or not path.parts
            ):
                raise ValueError("недопустимый путь/число archive members")
            normalized = str(path)
            if normalized in seen:
                raise ValueError("повтор archive member")
            seen.add(normalized)
            if member.isdir():
                continue
            if not member.isfile() or member.size > MAX_FILE:
                raise ValueError("symlink/special member или слишком большой файл")
            stream = archive.extractfile(member)
            assert stream is not None
            result[normalized] = stream.read(MAX_FILE + 1)
    return result


def package_files(path: Path) -> tuple[bytes, dict[str, bytes]]:
    data = read_file(path)
    # zstd читает нашу копию, поэтому хеш и сканирование связаны с одними байтами.
    with tempfile.TemporaryFile() as source:
        source.write(data)
        source.seek(0)
        process = subprocess.Popen(
            ["zstd", "-dc"],
            stdin=source,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        try:
            assert process.stdout is not None
            unpacked = process.stdout.read(MAX_ARCHIVE + 1)
            if len(unpacked) > MAX_ARCHIVE:
                raise ValueError("превышен лимит распакованного архива")
            if process.wait(timeout=10):
                raise ValueError("повреждён zstd archive")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            if process.stdout:
                process.stdout.close()
    return data, members(unpacked)


def elf(data: bytes) -> dict:
    with tempfile.TemporaryDirectory(prefix="yonik-composition-") as temporary:
        binary = Path(temporary) / "artifact"
        binary.write_bytes(data)
        output = subprocess.run(
            ["readelf", "-W", "-d", "-l", str(binary)],
            capture_output=True,
            check=True,
            timeout=20,
        ).stdout.decode()
    return {
        "sha256": sha(data),
        "needed": sorted(re.findall(r"\(NEEDED\).*?\[(.*?)\]", output)),
        "interpreter": re.findall(r"Requesting program interpreter: ([^\]]+)", output),
        "search_paths": re.findall(r"\((?:RUNPATH|RPATH)\).*?\[(.*?)\]", output),
        "ownership": "unresolved: DT_NEEDED не устанавливает пакет/версию поставщика",
    }


def go_info(data: bytes) -> dict:
    with tempfile.TemporaryDirectory(prefix="yonik-go-info-") as temporary:
        binary = Path(temporary) / "artifact"
        binary.write_bytes(data)
        lines = (
            subprocess.run(
                ["go", "version", "-m", str(binary)],
                capture_output=True,
                check=True,
                timeout=20,
                env={
                    **os.environ,
                    "GOTOOLCHAIN": "local",
                    "GOPROXY": "off",
                    "GOSUMDB": "off",
                },
            )
            .stdout.decode()
            .splitlines()
        )
    rows = [line.strip().split("\t") for line in lines[1:]]
    # Не публикуем произвольные ldflags и локальные пути из build settings.
    allowed = {
        "vcs",
        "vcs.revision",
        "vcs.modified",
        "vcs.time",
        "CGO_ENABLED",
        "GOARCH",
        "GOOS",
        "GOAMD64",
        "-tags",
        "-trimpath",
        "-buildmode",
    }
    return {
        "toolchain": lines[0].rsplit(": ", 1)[-1] if lines else "unknown",
        "modules": [row for row in rows if row[0] in {"mod", "dep", "=>"}],
        "settings": [
            row[1]
            for row in rows
            if row[0] == "build"
            and len(row) == 2
            and row[1].split("=", 1)[0] in allowed
        ],
        "limits": "module buildinfo не доказывает включение каждого символа; тексты зависимостей unresolved",
    }


def scan(package: Path, cores: list[Path]) -> dict:
    packed, files = package_files(package)
    inventory = read_file(ROOT / "packaging/licenses/linux-inventory.json")
    primary = json.loads(read_file(ROOT / "packaging/licenses/upstream/sources.json"))
    texts = []
    for record in primary:
        filename = record["name"] + ".LICENSE"
        if PurePosixPath(filename).name != filename or "\\" in filename:
            raise ValueError("недопустимое имя license")
        raw = read_file(ROOT / "packaging/licenses/upstream" / filename)
        if sha(raw) != record["sha256"]:
            raise ValueError("license hash mismatch")
        texts.append(
            {
                "name": record["name"],
                "url": record["url"],
                "sha256": sha(raw),
                "binary_binding": "unresolved: совпадение версии/commit недостаточно для source checksum",
            }
        )
    artifacts = {
        name: elf(data)
        for name, data in sorted(files.items())
        if data.startswith(b"\x7fELF")
    }
    required = {
        "usr/bin/yonik-desktop",
        "usr/lib/yonik/yonik-helper",
        "usr/lib/yonik/yonik-worker",
    }
    if not required.issubset(artifacts):
        raise ValueError("отсутствует обязательный ELF поставки")
    engines = {}
    for core in cores:
        data = read_file(core)
        if not data.startswith(b"\x7fELF"):
            raise ValueError("ядро не ELF")
        key = core.parent.name + "/" + core.name
        if key in engines:
            raise ValueError("повтор ядра")
        engines[key] = {**elf(data), "go": go_info(data)}
    return {
        "schema": 1,
        "status": "partial",
        "package_sha256": sha(packed),
        "package_metadata": {
            name: files[name].decode() for name in (".PKGINFO", ".BUILDINFO")
        },
        "rust_inventory": {
            "sha256": sha(inventory),
            "relation": "source normal/build closure; dev-only исключены; не состав этих ELF",
            "static_inclusion": "unresolved: нужны exact-build provenance/link map; dead code unknown",
        },
        "elf": artifacts,
        "cores": engines,
        "primary_license_texts": texts,
        "qt_plugins": {
            "packaged": [name for name in files if "/plugins/" in name],
            "runtime": "unresolved: dlopen/Qt paths и версии требуют изолированного runtime",
        },
        "limits": [
            "Пакет source eec7 — исторический, не latest product",
            "Arch ownership и runtime resolution требуют базы целевой среды; host не подменяет её",
            "Go main (devel), patches и source checksum требуют отдельного provenance",
            "Артефактный scan не юридическое заключение и не полный P25",
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--core", action="append", default=[], type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--check", type=Path)
    args = parser.parse_args()
    result = scan(args.package, args.core)
    if args.check and json.loads(read_file(args.check)) != result:
        raise ValueError("composition manifest не соответствует входам")
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
