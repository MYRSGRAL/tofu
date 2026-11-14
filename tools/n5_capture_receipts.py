#!/usr/bin/env python3
"""Проекция внешне доверенного Rust capture; не compile attestation/admission."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
from pathlib import Path

from n5_build_manifest import (
    MAX_ELF,
    MAX_SOURCE,
    open_regular,
    open_regular_directory,
    read_artifact,
    stamp,
)
from p25_capture_build_closure import (
    MAX_METADATA,
    _capture,
    object_fields,
    parse,
    require,
    sha,
)
from release_controller import CONTROLLED_ENV, RECIPE, RUST, SLOT, elf_header

METADATA = (
    "rust-captured-only.json",
    "build-capture-manifest.json",
    "build-capture-record.json",
    "build-capture-report.json",
    "frozen.json",
)
ROLES = dict(zip(("desktop", "helper", "worker"), RUST, strict=True))
PRODUCER_MISSING = [
    "cxx:invocations",
    "cxx:normalized-depfiles",
    "generated:paths-and-generation-records",
    "qt:observed-query-and-tool",
    "qt:header-source-primary-join",
]


def encode(value: dict) -> bytes:
    """Детерминированные байты receipt/report."""
    raw = (
        json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(",", ":"))
        + "\n"
    ).encode()
    require(len(raw) <= 32 * MAX_METADATA, "Выход превышает cap")
    return raw


class RetainedReader:
    """Адаптер конечного Rust whitelist для существующего capture validator."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.total = 0
        self.seen: dict[str, str] = {}
        self.identities: dict[str, tuple[int, ...]] = {}

    def _consume(
        self, root: str, path: str, limit: int, content: bool
    ) -> tuple[bytes, str]:
        require(root == "build", "Rust-only capture не содержит иных roots")
        names = {"capture-record.json": "build-capture-record.json"}
        names.update({"release/" + name: name for name in RUST})
        require(path in names, "Путь вне retained whitelist")
        digest, raw = self.read_file(names[path], limit, content)
        return raw, digest

    def read_file(
        self, name: str, limit: int, content: bool = True
    ) -> tuple[str, bytes]:
        fd = open_regular(self.root / name)
        try:
            before = stamp(os.fstat(fd))
        finally:
            os.close(fd)
        require(
            name not in self.identities or self.identities[name] == before,
            "Путь подменён между чтениями",
        )
        digest, raw = read_artifact(self.root / name, limit, collect=content)
        fd = open_regular(self.root / name)
        try:
            require(stamp(os.fstat(fd)) == before, "Путь изменился при join")
        finally:
            os.close(fd)
        self.identities[name] = before
        require(name not in self.seen or self.seen[name] == digest, "Вход изменился")
        self.seen[name] = digest
        self.total += before[2]
        require(self.total <= 2 * 1024**3, "Общий cap чтения превышен")
        return digest, raw

    def read(self, root: str, path: str, limit: int) -> bytes:
        return self._consume(root, path, limit, True)[0]


def exact(value: object, fields: set[str]) -> dict:
    return object_fields(value, fields, fields)


def produce(
    retained: Path, pins: dict[str, str], source_commit: str
) -> tuple[dict, dict]:
    """Pins передаются из независимого доверенного канала, никогда из самого marker."""
    require(set(pins) == {*METADATA, "source.tar"}, "Нужны все independent pins")
    require(re.fullmatch(r"[0-9a-f]{40}", source_commit) is not None, "Неверный commit")
    reader = RetainedReader(retained)
    values = {}
    for name in METADATA:
        digest, raw = reader.read_file(
            name, 32 * MAX_METADATA if name.endswith("report.json") else MAX_METADATA
        )
        require(digest == sha(pins[name]), "Independent pin mismatch: " + name)
        values[name] = parse(raw)
    archive, _ = reader.read_file("source.tar", MAX_SOURCE, False)
    require(archive == sha(pins["source.tar"]), "Source archive pin mismatch")
    frozen = values["frozen.json"]
    fields = {
        "schema",
        "release_id",
        "state",
        "source",
        "archive_sha256",
        "recipe",
        "tooling_sha256",
    }
    exact(frozen, fields)
    require(
        type(frozen["schema"]) is int
        and frozen["schema"] == 2
        and frozen["state"] == "FROZEN",
        "Неверный frozen schema/state",
    )
    exact(frozen["source"], {"commit", "tree"})
    require(
        frozen["source"]["commit"] == source_commit
        and re.fullmatch(r"[0-9a-f]{40}", frozen["source"]["tree"]) is not None,
        "Stale source",
    )
    require(
        frozen["recipe"] == RECIPE and frozen["archive_sha256"] == archive,
        "Неверный frozen profile/archive",
    )
    require(
        isinstance(frozen["tooling_sha256"], dict) and bool(frozen["tooling_sha256"]),
        "Нет tooling pins",
    )
    for pin in frozen["tooling_sha256"].values():
        sha(pin)
    exact(values["rust-captured-only.json"]["toolchains"], {"cargo", "rustc"})
    marker = values["rust-captured-only.json"]
    require(
        type(marker["schema"]) is int and marker["schema"] == 2,
        "Неверный marker schema",
    )
    for version in marker["toolchains"].values():
        require(
            isinstance(version, str) and 0 < len(version) <= 4096, "Неверный toolchain"
        )
    require(
        isinstance(marker["controlled_env"], dict)
        and set(marker["controlled_env"]) <= set(CONTROLLED_ENV),
        "Неверный environment",
    )
    require(type(frozen["recipe"]["schema"]) is int, "Неверный recipe schema")
    require(
        isinstance(marker["commands"], list) and len(marker["commands"]) <= 4096,
        "Command bounds",
    )
    for command in marker["commands"]:
        exact(command, {"argv", "cwd", "exit", "controlled_env"})
        require(
            isinstance(command["argv"], list)
            and 0 < len(command["argv"]) <= 4096
            and all(
                isinstance(arg, str) and 0 < len(arg) <= 4096 and "\x00" not in arg
                for arg in command["argv"]
            ),
            "Неверные command args",
        )
        require(
            isinstance(command["controlled_env"], dict)
            and set(command["controlled_env"]) <= set(CONTROLLED_ENV)
            and all(
                isinstance(value, str) for value in command["controlled_env"].values()
            ),
            "Неверный command environment",
        )
    exact(
        marker,
        fields
        | {
            "producer",
            "toolchains",
            "controlled_env",
            "commands",
            "elf",
            "build_capture",
            "rust_lineage",
            "final_linux_acceptance",
        },
    )
    require(
        all(marker[key] == value for key, value in frozen.items() if key != "state"),
        "Stale frozen marker",
    )
    require(
        marker["state"] == "RUST_CAPTURED_ONLY"
        and marker["producer"] == "local-release-controller-v1"
        and marker["rust_lineage"] is None
        and marker["final_linux_acceptance"] is False,
        "Не actual Rust capture",
    )
    manifest = values["build-capture-manifest.json"]
    exact(
        manifest,
        {
            "schema",
            "status",
            "missing",
            "producer_missing",
            "source",
            "observed_commands",
            "record_sha256",
            "report_sha256",
            "elfs",
            "final_linux_acceptance",
            "legal_status",
            "production_admission_gate",
            "invocation",
        },
    )
    require(marker["build_capture"] == manifest, "Capture marker conflict")
    require(
        manifest["record_sha256"] == pins["build-capture-record.json"]
        and manifest["report_sha256"] == pins["build-capture-report.json"],
        "Capture pins conflict",
    )
    record = values["build-capture-record.json"]
    # Этот seam намеренно поддерживает только исходный Rust-only producer, без final-profile enrichment.
    require(
        record["source"]
        == {**frozen["source"], "archive_sha256": archive, "tracked_inputs": []},
        "Source binding отсутствует",
    )
    commands = [[SLOT, *command] for command in RECIPE["rust_commands"]]
    require(
        record["profile"]
        == {"target": RECIPE["target"], "commands": commands, "features": ["gui"]},
        "Profile mismatch",
    )
    for key, expected in {
        "units": [],
        "generated": [],
        "primary_refs": [],
        "qt_version": None,
    }.items():
        require(record.get(key, "MISSING") == expected, "Неподдерживаемая closure")
    observed = manifest["observed_commands"]
    require(
        isinstance(observed, list) and len(observed) == len(commands),
        "Нет observed closure",
    )
    for row, command in zip(observed, commands, strict=True):
        exact(row, {"argv", "cwd", "exit", "controlled_env"})
        require(
            row["argv"] == command
            and type(row["exit"]) is int
            and row["exit"] == 0
            and isinstance(row["cwd"], str)
            and row["cwd"].startswith("/"),
            "Неверный observed build",
        )
    require(
        len({row["cwd"] for row in observed}) == 1
        and marker["commands"][-len(observed) :] == observed,
        "Command closure conflict",
    )
    expected_files = [
        {
            "id": f"elf-{i}",
            "root": "build",
            "path": "release/" + name,
            "kind": "elf",
            "sha256": sha(row["sha256"]),
        }
        for i, (name, row) in enumerate(zip(RUST, record["files"], strict=True))
    ]
    require(
        record["files"] == expected_files and manifest["elfs"] == expected_files,
        "ELF whitelist conflict",
    )
    replay = _capture(reader, "capture-record.json", pins["build-capture-record.json"])
    report = values["build-capture-report.json"]
    require(
        set(report) == set(replay) | {"roots", "record_path"}, "Report schema mismatch"
    )
    require(
        all(
            report[key] == value for key, value in replay.items() if key != "read_bytes"
        ),
        "Closure replay mismatch",
    )
    require(report["record_path"] == "capture-record.json", "Report record mismatch")
    require(
        manifest["schema"] == 1
        and type(manifest["schema"]) is int
        and manifest["status"] == replay["status"]
        and manifest["missing"] == replay["missing"]
        and manifest["source"] == record["source"]
        and manifest["producer_missing"] == PRODUCER_MISSING
        and manifest["final_linux_acceptance"] is False
        and manifest["production_admission_gate"] is False
        and manifest["legal_status"] == "NOTPROVEN",
        "Closure manifest mismatch",
    )
    exact(report["roots"], {"source", "build", "sdk"})
    require(
        all(
            isinstance(path, str) and path.startswith("/")
            for path in report["roots"].values()
        ),
        "Неверные report roots",
    )
    require(
        type(report["schema"]) is int
        and type(report["read_bytes"]) is int
        and 0 < report["read_bytes"] <= 1024**3,
        "Неверный report schema/bounds",
    )
    require(
        manifest["invocation"]
        == [
            "python3",
            "tools/p25_capture_build_closure.py",
            "--source-root",
            report["roots"]["source"],
            "--build-root",
            report["roots"]["build"],
            "--sdk-root",
            report["roots"]["sdk"],
            "--record",
            "capture-record.json",
            "--record-sha256",
            pins["build-capture-record.json"],
        ],
        "Invocation mismatch",
    )
    require(observed[0]["cwd"] == report["roots"]["source"], "Source cwd mismatch")
    receipts = {}
    require(set(marker["elf"]) == set(RUST), "Marker ELF whitelist mismatch")
    for role, name in ROLES.items():
        digest, _ = reader.read_file(name, MAX_ELF, False)
        fd = open_regular(retained / name)
        try:
            header = elf_header(os.read(fd, 64), os.fstat(fd).st_size)
        finally:
            os.close(fd)
        require(header == marker["elf"][name], "ELF header mismatch")
        receipts[role + ".receipt.json"] = {
            "schema": "SOURCE_BOUND_ELF_V1",
            "source_commit": source_commit,
            "source_archive_sha256": archive,
            "elf_sha256": digest,
        }
    receipts["worker-build-receipt.json"] = {
        "schema": 1,
        "worker_sha256": receipts["worker.receipt.json"]["elf_sha256"],
        "source_archive_sha256": archive,
    }
    # Повторно проверяем все пути после join, включая подмену ранее прочитанного файла.
    for name, digest in list(reader.seen.items()):
        actual, _ = reader.read_file(
            name, MAX_SOURCE if name == "source.tar" else MAX_ELF, False
        )
        require(actual == digest, "Вход изменился после join")
    return receipts, {
        "status": "UNREGISTERED",
        "admission": "NOT_ADMISSION",
        "binding": "projection_of_externally_trusted_controller_build_evidence",
        "independent_compile_attestation": False,
        "independent_pins": pins,
        "source_commit": source_commit,
        "source_archive_sha256": archive,
        "closure_status": replay["status"],
        "closure_missing": replay["missing"],
        "driver": "MISSING",
        "receipt_sha256": {
            name: hashlib.sha256(encode(value)).hexdigest()
            for name, value in receipts.items()
        },
    }


def emit(output: Path, receipts: dict, report: dict) -> None:
    """Новый каталог в workspace, fsync; существующее имя никогда не заменяется."""
    require(
        set(receipts)
        == {*(role + ".receipt.json" for role in ROLES), "worker-build-receipt.json"},
        "Неверный набор outputs",
    )
    output = Path(os.path.abspath(output))
    workspace = Path.cwd()
    require(
        output.is_relative_to(workspace) and output != workspace, "Выход вне workspace"
    )
    require(
        not any(
            output.is_relative_to(Path(p))
            for p in (
                "/etc",
                "/usr",
                "/dev",
                "/proc",
                "/sys",
                "/run",
                "/boot",
                "/var/lib",
                "/var/cache",
            )
        ),
        "Системный output",
    )
    payloads = {
        name: encode(value)
        for name, value in {**receipts, "report.json": report}.items()
    }
    parent = open_regular_directory(output.parent)
    try:
        require(os.fstat(parent).st_uid == os.geteuid(), "Чужой parent")
        os.mkdir(output.name, 0o700, dir_fd=parent)
        directory = os.open(
            output.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent
        )
        try:
            for name, raw in payloads.items():
                fd = os.open(
                    name,
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                    0o600,
                    dir_fd=directory,
                )
                with os.fdopen(fd, "wb") as stream:
                    stream.write(raw)
                    stream.flush()
                    os.fsync(stream.fileno())
            os.fsync(directory)
            os.fsync(parent)
        finally:
            os.close(directory)
    finally:
        os.close(parent)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--retained", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--output", type=Path, required=True)
    for name in (*METADATA, "source.tar"):
        parser.add_argument(
            "--" + name.replace(".", "-") + "-sha256", dest=name, required=True
        )
    args = vars(parser.parse_args())
    try:
        receipts, report = produce(
            args["retained"],
            {name: args[name] for name in (*METADATA, "source.tar")},
            args["source_commit"],
        )
        emit(args["output"], receipts, report)
    except (OSError, ValueError, TypeError, KeyError, RecursionError) as error:
        parser.exit(2, f"Capture отклонён: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
