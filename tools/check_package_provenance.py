#!/usr/bin/env python3
"""Локальный source→ELF preflight; не выполняет сборку или установку."""

from __future__ import annotations

import argparse
import hashlib
import json
import stat
import subprocess
import tarfile
import tempfile
from pathlib import Path, PurePosixPath


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def git(repo: Path, *args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(repo), *args])


def validate_source_members(path: Path) -> None:
    """Проверяет все заголовки до чтения payload; ссылки не входят в closure."""
    prefix = None
    seen: set[str] = set()
    files: set[str] = set()
    directories: set[str] = set()
    with tarfile.open(path) as archive:
        for entry in archive:
            name = entry.name.removesuffix("/") if entry.isdir() else entry.name
            parts = name.split("/")
            if (
                not name
                or any(part in ("", ".", "..") for part in parts)
                or name in seen
                or entry.type
                not in (tarfile.DIRTYPE, tarfile.REGTYPE, tarfile.AREGTYPE)
                or entry.linkname
            ):
                raise ValueError("Недопустимый source путь/тип")
            seen.add(name)
            if prefix is None:
                prefix = parts[0]
            if parts[0] != prefix or (entry.isfile() and len(parts) < 2):
                raise ValueError("Неверный source prefix")
            (directories if entry.isdir() else files).add(name)
    parents = {str(parent) for name in files for parent in PurePosixPath(name).parents}
    if not files or not directories <= parents:
        raise ValueError("Лишний source каталог")


def input_path(inputs: Path, name: str) -> Path:
    """Отказывает ссылкам во всех компонентах до resolve/hash (same-UID TCB)."""
    relative = PurePosixPath(name)
    if (
        relative.is_absolute()
        or name != relative.as_posix()
        or any(part in (".", "..") for part in relative.parts)
        or not relative.parts
    ):
        raise ValueError("Недопустимый путь артефакта")
    path = inputs.absolute()
    for component in (*reversed(path.parents), path):
        if component.is_symlink():
            raise ValueError("Недопустимый путь: ссылка")
    for part in relative.parts:
        path /= part
        if path.is_symlink():
            raise ValueError("Недопустимый путь: ссылка")
    if not stat.S_ISREG(path.stat().st_mode):
        raise ValueError("Недопустимый путь: не обычный файл")
    return path


def verify(candidate: dict, expected: dict, repo: Path, inputs: Path) -> dict:
    """Expected — независимо review-закреплённый контракт, не часть кандидата.

    Build record является свидетельством доверенного локального builder (TCB),
    а не доказательством исполнения произвольного compiler вне этой модели.
    """
    if expected.get("status") != "FROZEN" or candidate != expected:
        raise ValueError("Нет frozen контракта или кандидат отличается от него")
    if candidate["schema"] != 1 or candidate["arch"] != "x86_64":
        raise ValueError("Неверная schema/архитектура")
    if candidate["abi"] != 260:
        raise ValueError("Неверный ABI")
    if candidate["capabilities"] != {"file_capabilities": [], "setuid": False}:
        raise ValueError("Непроверенные capabilities")
    if candidate["safety_gates"] != {
        "host_install": False,
        "autostart": False,
        "runtime_core_download": False,
    }:
        raise ValueError("Неверные safety gates")
    source = candidate["source"]
    commit = source["commit"]
    if len(commit) != 40 or any(c not in "0123456789abcdef" for c in commit):
        raise ValueError("Нужен полный commit SHA")
    if git(repo, "rev-parse", commit + "^{tree}").decode().strip() != source["tree"]:
        raise ValueError("Подменён source tree")
    artifacts = candidate["artifacts"]
    required = {
        "source_archive",
        "gui",
        "helper",
        "worker",
        "xray",
        "sing_box",
        "xray_patched_package",
        "sing_box_patched_package",
        "notices",
        "licenses",
        "source_offers",
        "build_record",
        "package",
    }
    if set(artifacts) != required:
        raise ValueError("Неполный closure артефактов")
    paths = {}
    for name, record in artifacts.items():
        path = input_path(inputs, record["path"])
        paths[name] = path
    validate_source_members(paths["source_archive"])
    for name, path in paths.items():
        if sha(path) != artifacts[name]["sha256"]:
            raise ValueError(f"Подменён артефакт: {name}")
    # Сравниваем каждый blob архива с точным Git tree, а не доверяем checksum.
    blobs = {}
    for row in git(repo, "ls-tree", "-rz", commit).split(b"\0"):
        if not row:
            continue
        metadata, name = row.split(b"\t", 1)
        mode, kind, oid = metadata.split()
        if kind != b"blob":
            raise ValueError("Submodule требует отдельного source closure")
        blobs[name.decode()] = (mode, git(repo, "cat-file", "blob", oid.decode()))
    seen = set()
    with tarfile.open(paths["source_archive"]) as archive:
        for entry in archive:
            if entry.isdir():
                continue
            name = entry.name.split("/", 1)[-1]
            if name in seen or name not in blobs:
                raise ValueError("Лишний/повторный source member")
            seen.add(name)
            mode, data = blobs[name]
            if mode not in (b"100644", b"100755"):
                raise ValueError("Недопустимый source mode/link")
            else:
                stream = archive.extractfile(entry) if entry.isfile() else None
                valid = (
                    stream is not None
                    and entry.size == len(data)
                    and stream.read(len(data) + 1) == data
                    and bool(entry.mode & 0o111) == (mode == b"100755")
                )
            if not valid:
                raise ValueError("Подменён source blob/mode")
    if seen != set(blobs):
        raise ValueError("Неполный source archive")
    build = json.loads(paths["build_record"].read_text())
    binding = {
        "source": source,
        "source_archive_sha256": artifacts["source_archive"]["sha256"],
        "command": candidate["build"]["command"],
        "toolchain": candidate["build"]["toolchain"],
        "target": candidate["build"]["target"],
        "outputs": {
            key: artifacts[key]["sha256"]
            for key in (
                "gui",
                "helper",
                "worker",
                "xray",
                "sing_box",
                "xray_patched_package",
                "sing_box_patched_package",
            )
        },
        "abi": 260,
    }
    if (
        build != binding
        or not binding["command"]
        or not binding["toolchain"]
        or binding["target"] != "x86_64-unknown-linux-gnu"
    ):
        raise ValueError("Отсутствует exact source/build/output binding")
    from check_arch_package import validate_archive

    # Snapshot содержит только проверенные Git blobs. Код из snapshot не запускается.
    with tempfile.TemporaryDirectory(prefix="yonik-frozen-") as temporary:
        frozen = Path(temporary)
        for name, (mode, data) in blobs.items():
            target = frozen / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        validate_archive(paths["package"], frozen)
    for key, member in (
        ("gui", "usr/bin/yonik-desktop"),
        ("helper", "usr/lib/yonik/yonik-helper"),
        ("worker", "usr/lib/yonik/yonik-worker"),
    ):
        data = subprocess.check_output(
            ["bsdtar", "-xOf", str(paths["package"]), member]
        )
        if hashlib.sha256(data).hexdigest() != artifacts[key]["sha256"]:
            raise ValueError("ELF пакета отличается от build closure")
    return {
        "status": "PASS",
        "scope": "local-TCB-provenance-preflight",
        "final_linux_acceptance": False,
        "candidate": candidate,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("candidate", "expected", "repo", "inputs", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    result = verify(
        json.loads(args.candidate.read_text()),
        json.loads(args.expected.read_text()),
        args.repo,
        args.inputs,
    )
    # O_EXCL: предыдущий final manifest нельзя заменить даже новым PASS.
    with args.output.open("x") as stream:
        stream.write(json.dumps(result, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
