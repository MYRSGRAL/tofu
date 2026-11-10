"""Воспроизводимый Linux notices inventory; не заключение о праве поставки."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import logging
import os
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


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def evidence(path: Path, base: Path) -> dict:
    return {"path": os.path.relpath(path, base), "sha256": digest(path)}


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
        base = Path(package["manifest_path"]).parent
        licenses: set[Path] = set()
        notices: set[Path] = set()
        for path in base.rglob("*"):
            if not path.is_file() or path.is_symlink():
                continue
            name = path.name.upper()
            if name.startswith(("LICENSE", "LICENCE", "COPYING")):
                licenses.add(path)
            if name.startswith(("NOTICE", "AUTHORS", "COPYRIGHT")):
                notices.add(path)
        if package.get("license_file"):
            path = base / package["license_file"]
            if path.is_file():
                licenses.add(path.resolve())
        rows.append(
            {
                "id": package_id,
                "name": package["name"],
                "version": package["version"],
                "source": package.get("source"),
                "spdx_declared": package.get("license"),
                "features": sorted(nodes[package_id]["features"]),
                "manifest": os.path.relpath(base / "Cargo.toml", root),
                "license_files": [evidence(p, base) for p in sorted(licenses)],
                "notice_files": [evidence(p, base) for p in sorted(notices)],
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--check", type=Path)
    args = parser.parse_args()
    if bool(args.output) == bool(args.check):
        parser.error("Укажите ровно один из --output/--check")
    root = Path(__file__).resolve().parents[1]
    metadata = json.loads(subprocess.check_output(METADATA_ARGS, cwd=root, text=True))
    report = (
        json.loads(args.check.read_text()) if args.check else inventory(metadata, root)
    )
    errors = validate(report, metadata, root)
    bundle = (args.output or args.check).parent / "texts"
    for row in report.get("packages", []):
        base = (root / row["manifest"]).parent
        for entry in row["license_files"] + row["notice_files"]:
            destination = bundle / entry["sha256"]
            if args.output:
                bundle.mkdir(exist_ok=True)
                destination.write_bytes((base / entry["path"]).read_bytes())
            elif not destination.is_file() or digest(destination) != entry["sha256"]:
                errors.append(f"missing_notice: bundled text {entry['sha256']}")
    if args.output:
        args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    for error in errors:
        logging.error("%s", error)
    logging.warning(
        f"packages={len(report['packages'])} errors={len(errors)}; runtime composition unknown"
    )
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
