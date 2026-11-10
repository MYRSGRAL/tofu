"""Воспроизводимый Linux notices inventory; не заключение о праве поставки."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import logging
import os
import stat
import subprocess
from pathlib import Path

TARGET = "x86_64-unknown-linux-gnu"
ARTIFACTS = [
    {
        "package": "yonik-desktop",
        "binary": "yonik-desktop",
        "path": "/usr/bin/yonik-desktop",
        "features": ["gui"],
    },
    {
        "package": "yonik-linux",
        "binary": "yonik-helper",
        "path": "/usr/lib/yonik/yonik-helper",
        "features": [],
    },
    {
        "package": "yonik-linux",
        "binary": "yonik-worker",
        "path": "/usr/lib/yonik/yonik-worker",
        "features": [],
    },
]
METADATA_ARGS = [
    "cargo",
    "+1.85.1",
    "metadata",
    "--locked",
    "--offline",
    "--format-version",
    "1",
    "--filter-platform",
    TARGET,
    "--features",
    "yonik-desktop/gui",
]


MAX_TEXT = 4 * 1024 * 1024


def read_bounded(path: Path, allowed: Path) -> bytes:
    """Читает regular file относительно разрешённого root без symlink-компонентов."""
    try:
        parts = path.relative_to(allowed).parts
    except ValueError as error:
        raise ValueError("путь вне допустимого корня") from error
    if not parts or any(part in ("..", ".") for part in parts):
        raise ValueError("неверный относительный путь")
    directory = os.open(allowed, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in parts[:-1]:
            nested = os.open(
                part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory
            )
            os.close(directory)
            directory = nested
        descriptor = os.open(
            parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory
        )
        with os.fdopen(descriptor, "rb") as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_TEXT:
                raise ValueError("не regular file или превышен лимит")
            data = stream.read(MAX_TEXT + 1)
            if len(data) > MAX_TEXT:
                raise ValueError("превышен лимит чтения")
            return data
    except OSError as error:
        raise ValueError("путь отсутствует, недоступен или содержит symlink") from error
    finally:
        os.close(directory)


def license_root(path: Path, base: Path, root: Path, source: str | None) -> Path:
    if path.is_relative_to(base):
        return base
    # Cargo workspace license-file у YonIk указывает на корневой LICENSE.
    if (
        source is None
        and base.is_relative_to(root)
        and path.parent == root
        and path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING"))
    ):
        return root
    raise ValueError("license_file вне допустимого корня")


def digest(path: Path) -> str:
    return hashlib.sha256(read_bounded(path, path.parent)).hexdigest()


def evidence(path: Path, base: Path, allowed: Path) -> dict:
    data = read_bounded(path, allowed)
    if not data.decode("utf-8", errors="replace").strip():
        raise ValueError("пустой лицензионный текст")
    return {
        "path": os.path.relpath(path, base),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def inventory(metadata: dict, root: Path) -> dict:
    """Отбирает normal/build closure; Cargo feature union остаётся верхней оценкой."""
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [
        p["id"]
        for p in packages.values()
        if p["name"] in {a["package"] for a in ARTIFACTS}
    ]
    seen: set[str] = set()
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        for dep in nodes[package_id]["deps"]:
            if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]):
                pending.append(dep["pkg"])
    rows = []
    for package_id in sorted(seen):
        package = packages[package_id]
        base = Path(package["manifest_path"]).parent.resolve()
        licenses: set[Path] = set()
        notices: set[Path] = set()
        for path in base.rglob("*"):
            if path.is_symlink() or not path.is_file():
                continue
            name = path.name.upper()
            if name.startswith(("LICENSE", "LICENCE", "COPYING")):
                licenses.add(path)
            if name.startswith(("NOTICE", "AUTHORS", "COPYRIGHT")):
                notices.add(path)
        if package.get("license_file"):
            relative = Path(package["license_file"])
            if relative.is_absolute():
                raise ValueError("абсолютный license_file запрещён")
            path = Path(os.path.normpath(base / relative))
            allowed = license_root(path, base, root, package.get("source"))
            read_bounded(path, allowed)
            licenses.add(path)
        rows.append(
            {
                "id": package_id,
                "name": package["name"],
                "version": package["version"],
                "source": package.get("source"),
                "spdx_declared": package.get("license"),
                "features": sorted(nodes[package_id]["features"]),
                "manifest": os.path.relpath(base / "Cargo.toml", root),
                "license_files": [
                    evidence(
                        p, base, license_root(p, base, root, package.get("source"))
                    )
                    for p in sorted(licenses)
                ],
                "notice_files": [evidence(p, base, base) for p in sorted(notices)],
            }
        )
    provenance = {
        str(p): digest(root / p)
        for p in [
            "Cargo.toml",
            "Cargo.lock",
            "packaging/arch/PKGBUILD",
            "tools/build_native_adapter.py",
            "crates/yonik-engine/src/lib.rs",
        ]
        if (root / p).is_file()
    }
    return {
        "schema": 1,
        "target": TARGET,
        "metadata_command": METADATA_ARGS,
        "scope": "normal/build closure выбранных packages; features — Cargo union, точный binary linkage unknown; dev-only edges исключены",
        "artifacts": copy.deepcopy(ARTIFACTS),
        "provenance": provenance,
        "packages": rows,
        "upstream_evidence": {
            str(p.relative_to(root)): digest(p)
            for p in sorted((root / "packaging/licenses/upstream").glob("*"))
            if p.is_file()
        },
        "runtime": {
            "qt_system_libraries": "unknown: требуется staged ELF + package database конкретной среды",
            "pinned_cores": "unknown: отдельные Xray/sing-box/wrapper, не часть Cargo closure; см. README.md",
            "binary_composition": "unknown: metadata не доказывает runtime binary composition",
        },
    }


def validate(report: dict, metadata: dict, root: Path) -> list[str]:
    expected = inventory(metadata, root)
    errors = []
    for key in expected.keys() - {"packages"}:
        if report.get(key) != expected[key]:
            errors.append(f"artifact_provenance: {key}")
    actual = {row["id"]: row for row in report.get("packages", [])}
    if len(actual) != len(report.get("packages", [])) or set(actual) != {
        row["id"] for row in expected["packages"]
    }:
        errors.append("artifact_provenance: package closure")
    for row in expected["packages"]:
        found = actual.get(row["id"], {})
        if (
            not row["license_files"]
            or found.get("license_files") != row["license_files"]
        ):
            errors.append(f"missing_license: {row['id']}")
        if found.get("notice_files") != row["notice_files"]:
            errors.append(f"missing_notice: {row['id']}")
        for key in row.keys() - {"license_files", "notice_files"}:
            if found.get(key) != row[key]:
                errors.append(f"artifact_provenance: {row['id']}: {key}")
    return errors


def store_text(bundle: Path, sha256: str, data: bytes) -> None:
    if len(sha256) != 64 or any(c not in "0123456789abcdef" for c in sha256):
        raise ValueError("неверный SHA-256")
    if hashlib.sha256(data).hexdigest() != sha256:
        raise ValueError("исходный текст изменился после inventory")
    bundle.mkdir(exist_ok=True)
    directory = os.open(bundle, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        try:
            descriptor = os.open(
                sha256,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                0o644,
                dir_fd=directory,
            )
        except FileExistsError:
            if digest(bundle / sha256) != sha256:
                raise ValueError("существующий bundled text повреждён") from None
            return
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
    finally:
        os.close(directory)


def write_report(path: Path, report: dict) -> None:
    data = (json.dumps(report, indent=2, ensure_ascii=False) + "\n").encode()
    if len(data) > MAX_TEXT:
        raise ValueError("слишком большой report")
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        descriptor = os.open(
            path.name,
            os.O_WRONLY | os.O_CREAT | os.O_NONBLOCK | os.O_NOFOLLOW,
            0o644,
            dir_fd=directory,
        )
        with os.fdopen(descriptor, "wb") as stream:
            if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                raise ValueError("report не regular file")
            stream.truncate(0)
            stream.write(data)
    finally:
        os.close(directory)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", type=Path)
    args = parser.parse_args()
    if bool(args.output) == bool(args.check):
        parser.error("Укажите ровно один из --output/--check")
    try:
        return run(args)
    except (OSError, ValueError, KeyError, TypeError) as error:
        logging.error("inventory отклонён: %s", type(error).__name__)
        return 1


def run(args: argparse.Namespace) -> int:
    root = Path(__file__).resolve().parents[1]
    metadata = json.loads(subprocess.check_output(METADATA_ARGS, cwd=root, text=True))
    report = (
        json.loads(read_bounded(args.check, args.check.parent))
        if args.check
        else inventory(metadata, root)
    )
    if not isinstance(report, dict):
        raise ValueError("report должен быть объектом")
    errors = validate(report, metadata, root)
    bundle = (args.output or args.check).parent / "texts"
    for row in inventory(metadata, root)["packages"]:
        base = (root / row["manifest"]).parent.resolve()
        for entry in row["license_files"] + row["notice_files"]:
            destination = bundle / entry["sha256"]
            if args.output:
                source = Path(os.path.normpath(base / entry["path"]))
                allowed = license_root(source, base, root, row.get("source"))
                store_text(bundle, entry["sha256"], read_bounded(source, allowed))
            elif not destination.is_file() or digest(destination) != entry["sha256"]:
                errors.append(f"missing_notice: bundled text {entry['sha256']}")
    if args.output:
        write_report(args.output, report)
    for error in errors:
        logging.error("%s", error)
    logging.warning(
        f"packages={len(report['packages'])} errors={len(errors)}; runtime composition unknown"
    )
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
