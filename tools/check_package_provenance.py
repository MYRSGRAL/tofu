#!/usr/bin/env python3
"""Локальный source→ELF preflight; не выполняет сборку или установку."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath
from typing import NamedTuple, Never


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


def _validate_materials(
    candidate: dict, expected: dict, repo: Path, inputs: Path
) -> dict:
    """Только структурная проверка недоверенных материалов, НЕ provenance.

    Функция не создаёт trusted context и не вызывается production CLI.
    Положительный результат не доказывает core/compliance build binding.
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
        "status": "MATERIALS_CHECKED",
        "scope": "untrusted-materials-only",
        "final_linux_acceptance": False,
        "candidate": candidate,
    }


class NotReady(ValueError):
    """Нет авторизованного полного build closure; финальный PASS запрещён."""


class FrozenBuildRequest(NamedTuple):
    """Вход отдельной controller operation, выбранный root, не candidate CLI."""

    release_id: str
    source_commit: str
    source_tree: str
    source_archive: Path


def collect_release(request: FrozenBuildRequest) -> Never:
    """Будущий producer сам запускает fixed recipe, не импортирует attestation.

    До реализации исполнения и сбора actual core/compliance metadata никакая
    регистрация frozen request не может создать trusted release context.
    """
    raise NotReady(
        "Producer не реализован: нужны fixed recipe execution, toolchains, "
        "ELF/GoBuildInfo, frozen core pins/patch packages и packaged compliance"
    )


def canonical_repo() -> Path:
    """Registry выводится из common Git root reviewed tooling, не caller/env."""
    env = {
        key: value for key, value in os.environ.items() if not key.startswith("GIT_")
    }
    common = (
        subprocess.check_output(
            [
                "git",
                "-C",
                str(Path(__file__).resolve().parent),
                "rev-parse",
                "--path-format=absolute",
                "--git-common-dir",
            ],
            env=env,
        )
        .decode()
        .strip()
    )
    repo = Path(common).parent
    if repo != Path("/home/myrs/projects/yonik") or Path(common).name != ".git":
        raise NotReady("Неизвестный canonical Git anchor")
    origin = (
        subprocess.check_output(
            ["git", "-C", str(repo), "remote", "get-url", "origin"], env=env
        )
        .decode()
        .strip()
    )
    if origin not in (
        "https://github.com/MYRSGRAL/yonik.git",
        "git@github.com:MYRSGRAL/yonik.git",
    ):
        raise NotReady("Неверный origin canonical anchor")
    subprocess.check_output(
        [
            "git",
            "-C",
            str(repo),
            "cat-file",
            "-e",
            "128a070884e82bf21b5c22a74ba908dadd7daf5a^{commit}",
        ],
        env=env,
    )
    return repo


def _private_directory(path: Path | str, parent: int | None = None) -> int:
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
    metadata = os.fstat(fd)
    if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        os.close(fd)
        raise NotReady("Registry требует own-UID каталог 0700")
    return fd


def _read_request(registry: Path, release_id: str) -> dict:
    """Descriptor-relative NOFOLLOW; shared lock корня удерживается до чтения."""
    for parent in reversed(registry.parents):
        if parent.is_symlink():
            raise NotReady("Ссылка в пути registry")
    root_fd = _private_directory(registry)
    try:
        fcntl.flock(root_fd, fcntl.LOCK_SH)
        release_fd = _private_directory(release_id, root_fd)
        try:
            fd = os.open(
                "frozen.json",
                os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                dir_fd=release_fd,
            )
            with os.fdopen(fd, "rb") as stream:
                metadata = os.fstat(stream.fileno())
                if (
                    not stat.S_ISREG(metadata.st_mode)
                    or metadata.st_uid != os.getuid()
                    or stat.S_IMODE(metadata.st_mode) != 0o600
                    or metadata.st_nlink != 1
                    or metadata.st_size > 1024 * 1024
                ):
                    raise NotReady(
                        "Registry record требует own-UID regular 0600 nlink=1"
                    )
                record = json.loads(stream.read(1024 * 1024 + 1))
                if not isinstance(record, dict):
                    raise NotReady("Некорректный registry request")
                return record
        finally:
            os.close(release_fd)
    finally:
        os.close(root_fd)


def verify_release(release_id: str, package: Path, inputs: Path) -> Never:
    """Единственный production entrypoint; arbitrary JSON не является authority."""
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,63}", release_id):
        raise NotReady("Некорректный release-id")
    registry = canonical_repo() / "local/release-controller"
    try:
        _read_request(registry, release_id)
    except (OSError, ValueError) as error:
        raise NotReady(f"Нет доверенного release context: {error}") from error
    # Даже FROZEN в приватном registry не заменяет ещё отсутствующий producer.
    # package/inputs намеренно не читаются до получения trusted build context.
    raise NotReady(
        "Registry request не является build authority: controlled producer "
        "и actual core/compliance binding ещё не реализованы"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-id", required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--inputs", type=Path, required=True)
    args = parser.parse_args()
    try:
        verify_release(args.release_id, args.package, args.inputs)
    except (NotReady, OSError, subprocess.CalledProcessError) as error:
        sys.stdout.write(
            json.dumps(
                {
                    "status": "BLOCKED",
                    "final_linux_acceptance": False,
                    "reason": str(error),
                },
                ensure_ascii=False,
            )
            + "\n"
        )
        raise SystemExit(2) from error


if __name__ == "__main__":
    main()
