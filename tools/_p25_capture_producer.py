"""Private capture текущего controller profile без discovery и дополнительных probes."""

from __future__ import annotations

import hashlib
import json
import os
from contextlib import ExitStack
from pathlib import Path

from p25_capture_build_closure import MAX_METADATA, Reader, capture, require


def _directory(path: Path) -> int:
    require(path.is_absolute() and ".." not in path.parts, "Неверный retained root")
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            nested = os.open(
                part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd
            )
            os.close(fd)
            fd = nested
        return fd
    except BaseException:
        os.close(fd)
        raise


def _write(path: Path, value: dict) -> str:
    data = (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()
    require(len(data) <= MAX_METADATA, "Capture metadata превышает cap")
    directory = _directory(path.parent)
    try:
        fd = os.open(
            path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600, dir_fd=directory,
        )
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.fsync(directory)
    finally:
        os.close(directory)
    return hashlib.sha256(data).hexdigest()


def retain(
    source: Path,
    build: Path,
    retained: Path,
    frozen: dict,
    commands: list[dict],
    elfs: list[str],
    target: str,
) -> dict:
    """Вызывать только после новых Rust builds под controller lock, до clean.

    Текущий profile не экспортирует CXX/query metadata. Отсутствие этих records
    нельзя заменять SDK inventory, командами recipe или diagnostic include chain.
    """
    require(bool(commands) and all(row.get("exit") == 0 for row in commands),
            "Нет успешных observed Rust commands")
    require(all(row.get("cwd") == str(source) for row in commands), "Command cwd mismatch")
    require(bool(elfs) and len(elfs) == len(set(elfs)), "Неполный ELF whitelist")
    require(not retained.is_relative_to(build.parent), "Retention внутри transient task")
    sdk = build.parent / "capture-empty-sdk"
    sdk.mkdir(mode=0o700)
    roots = {"source": source, "build": build, "sdk": sdk}
    files = []
    with ExitStack() as stack:
        reader = Reader(roots, stack)
        for index, name in enumerate(elfs):
            _, checksum = reader._consume("build", "release/" + name, 512 * 1024 * 1024, False)
            files.append({"id": f"elf-{index}", "root": "build", "path": "release/" + name,
                          "kind": "elf", "sha256": checksum})
    record = {
        "schema": 1,
        "source": {**frozen["source"], "archive_sha256": frozen["archive_sha256"], "tracked_inputs": []},
        "profile": {"target": target, "commands": [row["argv"] for row in commands], "features": ["gui"]},
        "files": files, "elfs": [row["id"] for row in files],
        "qt_version": None, "units": [], "generated": [], "primary_refs": [],
    }
    pin = _write(build / "capture-record.json", record)
    report = capture(roots, "capture-record.json", pin)
    require(report["status"] == "MISSING", "Текущий producer не доказывает CXX/Qt closure")
    _write(retained / "build-capture-record.json", record)
    report_pin = _write(retained / "build-capture-report.json", report)
    manifest = {
        "schema": 1, "status": report["status"], "missing": report["missing"],
        "producer_missing": ["cxx:invocations", "cxx:normalized-depfiles",
                             "generated:paths-and-generation-records", "qt:observed-query-and-tool",
                             "qt:header-source-primary-join"],
        "source": record["source"], "observed_commands": commands,
        "record_sha256": pin, "report_sha256": report_pin,
        "elfs": files, "final_linux_acceptance": False,
        "legal_status": "NOTPROVEN", "production_admission_gate": False,
        "invocation": ["python3", "tools/p25_capture_build_closure.py",
                       "--source-root", str(source), "--build-root", str(build),
                       "--sdk-root", str(sdk), "--record", "capture-record.json",
                       "--record-sha256", pin],
    }
    _write(retained / "build-capture-manifest.json", manifest)
    fd = _directory(retained)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    return manifest
