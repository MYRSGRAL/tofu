"""Ограниченный post-build evidence capture; не production admission/legal gate."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys
from contextlib import ExitStack
from pathlib import Path
from typing import Any

MAX_FILES = 4096
MAX_FILE = 512 * 1024 * 1024
MAX_TOTAL = 1024 * 1024 * 1024
MAX_METADATA = 1024 * 1024
MAX_TEXT = 8 * 1024 * 1024
MAX_GENERATED = 16 * 1024 * 1024
KINDS = {
    "source": "source",
    "sdk-source": "source",
    "primary": "source",
    "header": "sdk",
    "tool": "sdk",
    "generated": "build",
    "elf": "build",
    "depfile": "build",
    "qt-version": "build",
}
IDENTITY = re.compile(r"[A-Za-z0-9_.-]{1,128}")
HASH = re.compile(r"[0-9a-f]{64}")


class CaptureError(ValueError):
    """Некорректные или изменившиеся входы; evidence отвергнуто."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CaptureError(message)


def object_fields(value: Any, allowed: set[str], required: set[str]) -> dict:
    require(isinstance(value, dict), "Ожидался object")
    require(required <= value.keys() <= allowed, "Неверные поля object")
    return value


def sequence(value: Any) -> list:
    require(
        isinstance(value, list) and len(value) <= MAX_FILES, "Неверный список/лимит"
    )
    return value


def identity(value: Any) -> str:
    require(
        isinstance(value, str) and IDENTITY.fullmatch(value) is not None, "Неверный ID"
    )
    return value


def ids(value: Any) -> list[str]:
    result = [identity(v) for v in sequence(value)]
    require(len(set(result)) == len(result), "Повтор ID")
    return result


def sha(value: Any) -> str:
    require(
        isinstance(value, str) and HASH.fullmatch(value) is not None, "Неверный SHA256"
    )
    return value


def relative(value: Any) -> str:
    require(isinstance(value, str) and 0 < len(value) <= 4096, "Неверный путь")
    require(
        not value.startswith("/")
        and all(p not in ("", ".", "..") for p in value.split("/")),
        "Путь вне закрытого root",
    )
    require(not any(ord(c) < 32 or ord(c) == 127 for c in value), "Control в пути")
    require(
        not set(value.split("/")) & {".ssh", ".gnupg", ".config", ".aws", ".env"},
        "Запрещённый private/config путь",
    )
    return value


def parse(raw: bytes) -> Any:
    def pairs(items: list[tuple[str, Any]]) -> dict:
        result: dict = {}
        for key, value in items:
            require(key not in result, "Повтор JSON key")
            result[key] = value
        return result

    try:
        return json.loads(
            raw,
            object_pairs_hook=pairs,
            parse_constant=lambda _: (_ for _ in ()).throw(
                CaptureError("Нефинитный JSON")
            ),
        )
    except (ValueError, UnicodeError, RecursionError) as error:
        raise CaptureError("Некорректный JSON") from error


class Reader:
    """FD-relative NOFOLLOW; roots открываются от / без symlink ancestors."""

    def __init__(self, roots: dict[str, Path], stack: ExitStack) -> None:
        require(set(roots) == {"source", "build", "sdk"}, "Нужны три explicit roots")
        self.fds: dict[str, int] = {}
        self.total = 0
        paths = list(roots.values())
        for i, path in enumerate(paths):
            require(
                path.is_absolute() and ".." not in path.parts, "Неверный absolute root"
            )
            require(path not in (Path("/"), Path.home()), "Слишком широкий root")
            require(
                all(
                    not path.is_relative_to(other) and not other.is_relative_to(path)
                    for other in paths[:i]
                ),
                "Roots пересекаются",
            )
        for name, path in roots.items():
            fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
            try:
                for part in path.parts[1:]:
                    nested = os.open(
                        part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd
                    )
                    os.close(fd)
                    fd = nested
            except OSError:
                os.close(fd)
                raise
            stack.callback(os.close, fd)
            self.fds[name] = fd

    def _consume(
        self, root: str, path: str, limit: int, content: bool
    ) -> tuple[bytes, str]:
        parts = relative(path).split("/")
        fd = os.dup(self.fds[root])
        try:
            for part in parts[:-1]:
                nested = os.open(
                    part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd
                )
                os.close(fd)
                fd = nested
            file_fd = os.open(
                parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd
            )
        finally:
            os.close(fd)
        with os.fdopen(file_fd, "rb") as stream:
            before = os.fstat(stream.fileno())
            require(stat.S_ISREG(before.st_mode), "Не regular file")
            require(before.st_size <= min(limit, MAX_FILE), "Размер файла превышен")
            require(self.total + before.st_size <= MAX_TOTAL, "Общий размер превышен")
            digest = hashlib.sha256()
            chunks: list[bytes] = []
            count = 0
            while chunk := stream.read(65536):
                count += len(chunk)
                self.total += len(chunk)
                require(
                    count <= min(limit, MAX_FILE) and self.total <= MAX_TOTAL,
                    "Размер чтения превышен",
                )
                digest.update(chunk)
                if content:
                    chunks.append(chunk)
            after = os.fstat(stream.fileno())
            require(
                (
                    before.st_dev,
                    before.st_ino,
                    before.st_size,
                    before.st_mtime_ns,
                    before.st_ctime_ns,
                )
                == (
                    after.st_dev,
                    after.st_ino,
                    after.st_size,
                    after.st_mtime_ns,
                    after.st_ctime_ns,
                )
                and count == before.st_size,
                "Файл изменился при чтении",
            )
            return b"".join(chunks), digest.hexdigest()

    def read(self, root: str, path: str, limit: int) -> bytes:
        return self._consume(root, path, limit, True)[0]


def capture(roots: dict[str, Path], record_path: str, record_sha256: str) -> dict:
    """Читает только pin-verified record и его конечный whitelist; ничего не запускает."""
    try:
        with ExitStack() as stack:
            result = _capture(Reader(roots, stack), record_path, sha(record_sha256))
            result["roots"] = {name: str(path) for name, path in roots.items()}
            result["record_path"] = record_path
            return result
    except (TypeError, KeyError, RecursionError) as error:
        raise CaptureError("Некорректная metadata structure") from error
    except OSError as error:
        raise CaptureError(f"Отказ безопасного чтения: {error.errno}") from error


def _capture(reader: Reader, record_path: str, expected: str) -> dict:
    raw = reader.read("build", record_path, MAX_METADATA)
    require(hashlib.sha256(raw).hexdigest() == expected, "Изменён record pin")
    record = object_fields(
        parse(raw),
        {
            "schema",
            "source",
            "profile",
            "files",
            "elfs",
            "qt_version",
            "units",
            "generated",
            "primary_refs",
        },
        {"schema", "source", "profile", "files", "elfs", "primary_refs"},
    )
    require(type(record["schema"]) is int and record["schema"] == 1, "Неверная schema")
    source = object_fields(
        record["source"],
        {"commit", "tree", "archive_sha256", "tracked_inputs"},
        {"commit", "tree", "archive_sha256", "tracked_inputs"},
    )
    for key in ("commit", "tree"):
        require(
            isinstance(source[key], str)
            and re.fullmatch("[0-9a-f]{40}", source[key]) is not None,
            "Неверный Git identity",
        )
    sha(source["archive_sha256"])
    tracked = set(ids(source["tracked_inputs"]))
    profile = object_fields(
        record["profile"],
        {"target", "commands", "features"},
        {"target", "commands", "features"},
    )
    identity(profile["target"])
    ids(profile["features"])
    require(bool(sequence(profile["commands"])), "Нет profile commands")
    for command in profile["commands"]:
        require(
            bool(sequence(command))
            and all(
                isinstance(arg, str) and 0 < len(arg) <= 4096 and "\x00" not in arg
                for arg in command
            ),
            "Неверный command record",
        )
    files: dict[str, dict] = {}
    contents: dict[str, bytes] = {}
    missing: list[str] = []
    paths: set[tuple[str, str]] = set()
    generated_total = 0
    for row in sequence(record["files"]):
        row = object_fields(
            row,
            {"id", "root", "path", "kind", "sha256", "source_ref"},
            {"id", "root", "path", "kind"},
        )
        key = identity(row["id"])
        require(key not in files, "Повтор file ID")
        require(
            isinstance(row["kind"], str)
            and row["kind"] in KINDS
            and (
                row["root"] == KINDS[row["kind"]]
                or row["kind"] == "tool"
                and row["root"] == "build"
            ),
            "Неверный file kind/root",
        )
        path = relative(row["path"])
        require((row["root"], path) not in paths, "Повтор file path")
        paths.add((row["root"], path))
        require(
            row["root"] != "source" or key in tracked,
            "Source input не в tracked whitelist",
        )
        files[key] = dict(row, verified=False)
        if "sha256" not in row:
            missing.append(f"identity:{key}")
            continue
        checksum = sha(row["sha256"])
        kind = row["kind"]
        limit = (
            MAX_FILE
            if kind in ("elf", "tool")
            else MAX_GENERATED
            if kind == "generated"
            else MAX_TEXT
            if kind in ("header", "source", "sdk-source", "primary")
            else MAX_METADATA
        )
        try:
            if kind in ("elf", "tool"):
                data, observed = reader._consume(row["root"], path, limit, False)
            else:
                data = reader.read(row["root"], path, limit)
                observed = hashlib.sha256(data).hexdigest()
        except FileNotFoundError:
            missing.append(f"file:{key}")
            continue
        require(observed == checksum, f"Hash mismatch: {key}")
        files[key]["verified"] = True
        if kind == "header":
            contents[key] = data[:16384]
        elif kind == "primary":
            contents[key] = b"x" if data.strip() else b""
        elif kind not in ("elf", "tool", "source", "sdk-source"):
            contents[key] = data
        if kind == "generated":
            generated_total += len(data)
            require(
                generated_total <= MAX_GENERATED, "Общий generated capture превышен"
            )
    require(
        tracked == {key for key, row in files.items() if row["root"] == "source"},
        "Tracked whitelist не совпал с source inputs",
    )

    def get(key: Any, kind: str | None = None) -> dict | None:
        key = identity(key)
        require(key in files, "Ссылка вне file whitelist")
        row = files[key]
        require(kind is None or row["kind"] == kind, "Неверный reference kind")
        return row if row["verified"] else None

    primary: dict[str, dict] = {}
    for ref in sequence(record["primary_refs"]):
        ref = object_fields(ref, {"identifier", "file"}, {"identifier", "file"})
        identifier = identity(ref["identifier"])
        require(identifier not in primary, "Повтор primary identifier")
        row = get(ref["file"], "primary")
        if row:
            require(bool(contents[row["id"]].strip()), "Пустой primary text")
            primary[identifier] = row

    def conditions(row: dict) -> dict:
        key = row["id"]
        text = contents[key][:16384].decode("utf-8", errors="replace")
        expressions = re.findall(r"SPDX-License-Identifier:[ \t]*([^\r\n]+)", text)
        expressions = [e.removesuffix("*/").strip() for e in expressions]
        tokens = {
            t
            for expression in expressions
            for t in re.findall(r"[A-Za-z0-9_.+-]+", expression)
        } - {"OR", "AND", "WITH"}
        if not expressions or not tokens:
            missing.append(f"SPDX:{key}")
        for token in sorted(tokens - primary.keys()):
            missing.append(f"primary:{key}:{token}")
        return {
            "expressions": expressions,
            "primary_refs": {
                token: primary[token] for token in sorted(tokens) if token in primary
            },
            "applicability": "NOTPROVEN",
            "copyright_excerpt": [
                line for line in text.splitlines() if "copyright" in line.lower()
            ],
        }

    headers = []
    for key, row in files.items():
        if row["kind"] != "header" or not row["verified"]:
            continue
        source_ref = (
            get(row["source_ref"], "sdk-source") if row.get("source_ref") else None
        )
        if source_ref is None:
            missing.append(f"header_source:{key}")
        headers.append({"source_ref": source_ref, "file": row, **conditions(row)})
    if not headers:
        missing.append("headeridentity")
    qt_version = None
    qt_binding = None
    if record.get("qt_version"):
        row = get(record["qt_version"], "qt-version")
        if row:
            qt = object_fields(
                parse(contents[row["id"]]),
                {"schema", "version", "sdk_files", "query_argv", "query_tool", "exit"},
                {"schema", "version", "sdk_files", "query_argv", "exit"},
            )
            require(
                type(qt["schema"]) is int
                and qt["schema"] == 1
                and type(qt["exit"]) is int
                and qt["exit"] == 0
                and isinstance(qt["version"], str)
                and re.fullmatch(r"\d+\.\d+\.\d+", qt["version"]) is not None,
                "Неверный Qt version query record",
            )
            require(
                bool(sequence(qt["query_argv"]))
                and all(isinstance(arg, str) for arg in qt["query_argv"]),
                "Неверный Qt query argv",
            )
            sdk_ids = set(ids(qt["sdk_files"]))
            require(
                sdk_ids
                == {key for key, file in files.items() if file["root"] == "sdk"},
                "Qt SDK binding не покрывает whitelist",
            )
            query_tool = get(qt["query_tool"], "tool") if qt.get("query_tool") else None
            if query_tool is None:
                missing.append("qt_query_tool")
            if all(get(key) for key in sdk_ids):
                qt_version = qt["version"]
                qt_binding = {"record": row, "metadata": qt, "query_tool": query_tool}
    if qt_version is None:
        missing.append("qt_version")
    generated = []
    generated_ids: set[str] = set()
    for item in sequence(record.get("generated", [])):
        item = object_fields(
            item, {"file", "inputs", "generator"}, {"file", "inputs", "generator"}
        )
        row = get(item["file"], "generated")
        require(item["file"] not in generated_ids, "Повтор generated record")
        generated_ids.add(item["file"])
        generator = get(item["generator"], "tool")
        inputs = ids(item["inputs"])
        require(
            bool(inputs) and item["file"] not in inputs,
            "Пустые/циклические generation inputs",
        )
        verified_inputs = [get(key) for key in inputs]
        if row and generator and all(verified_inputs):
            try:
                text = contents[row["id"]].decode("utf-8")
            except UnicodeError as error:
                raise CaptureError("Generated source не UTF-8") from error
            generated.append(
                {
                    "file": row,
                    "text": text,
                    "inputs": verified_inputs,
                    "generator": generator,
                    **conditions(row),
                }
            )
    if not generated or generated_ids != {
        key for key, row in files.items() if row["kind"] == "generated"
    }:
        missing.append("generatedrecord")
    units = sequence(record.get("units", []))
    if not units:
        missing.append("depfiles")
    unit_ids: set[str] = set()
    dependency_headers: set[str] = set()
    compile_units = []
    for unit in units:
        unit = object_fields(unit, {"file", "depfile", "argv"}, {"file", "depfile"})
        if not unit.get("argv"):
            missing.append(f"compile_metadata:{unit['file']}")
        else:
            require(
                bool(sequence(unit["argv"]))
                and all(
                    isinstance(arg, str) and 0 < len(arg) <= 4096 and "\x00" not in arg
                    for arg in unit["argv"]
                ),
                "Неверный CXX argv",
            )
        get(unit["file"])
        require(files[unit["file"]]["kind"] in ("source", "generated"), "Неверный unit")
        require(unit["file"] not in unit_ids, "Повтор unit")
        unit_ids.add(unit["file"])
        captured_unit = dict(unit, normalized_depfile=None)
        compile_units.append(captured_unit)
        if unit["depfile"] is None:
            missing.append(f"depfile:{unit['file']}")
            continue
        row = get(unit["depfile"], "depfile")
        if row:
            dep = object_fields(
                parse(contents[row["id"]]),
                {"schema", "unit", "dependencies"},
                {"schema", "unit", "dependencies"},
            )
            require(
                type(dep["schema"]) is int
                and dep["schema"] == 1
                and dep["unit"] == unit["file"],
                "Неверный normalized depfile binding",
            )
            captured_unit["normalized_depfile"] = dep
            dependencies = ids(dep["dependencies"])
            require(bool(dependencies), "Пустой depfile")
            for key in dependencies:
                get(key)
                require(
                    files[key]["kind"]
                    in ("source", "sdk-source", "header", "generated"),
                    "Неверный dependency kind",
                )
                if files[key]["kind"] == "header":
                    dependency_headers.add(key)
    for key, row in files.items():
        if row["kind"] == "header" and key not in dependency_headers:
            missing.append(f"header_depfile_coverage:{key}")
    if not generated_ids <= unit_ids:
        missing.append("generated_compile_units")
    elfs = []
    elf_ids = ids(record["elfs"])
    require(
        set(elf_ids) == {key for key, row in files.items() if row["kind"] == "elf"},
        "ELF binding не покрывает whitelist",
    )
    for key in elf_ids:
        row = get(key, "elf")
        if row:
            elfs.append(row)
    if not elfs:
        missing.append("elf_profile_binding")
    require(
        reader.read("build", record_path, MAX_METADATA) == raw,
        "Metadata изменились во время capture",
    )
    return {
        "schema": 1,
        "status": "MISSING" if missing else "CAPTURED",
        "missing": sorted(set(missing)),
        "legal_status": "NOTPROVEN",
        "final_linux_acceptance": False,
        "production_admission_gate": False,
        "record_sha256": expected,
        "source": source,
        "elf_profile_binding": {"profile": profile, "elfs": elfs},
        "qt_version": qt_version,
        "qt_binding": qt_binding,
        "compile_units": compile_units,
        "binding_scope": "caller-pinned-record; independent source/provenance authority required",
        "headers": headers,
        "generated_sources": generated,
        "files": list(files.values()),
        "read_bytes": reader.total,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "build", "sdk"):
        parser.add_argument("--" + name + "-root", type=Path, required=True)
    parser.add_argument("--record", required=True, help="Относительно build root")
    parser.add_argument(
        "--record-sha256",
        required=True,
        help="Pin из independently verified build record",
    )
    args = parser.parse_args()
    try:
        result = capture(
            {
                name: getattr(args, name + "_root")
                for name in ("source", "build", "sdk")
            },
            args.record,
            args.record_sha256,
        )
    except (CaptureError, TypeError, KeyError) as error:
        json.dump(
            {
                "status": "REJECTED",
                "error": str(error),
                "final_linux_acceptance": False,
            },
            sys.stdout,
            ensure_ascii=False,
        )
        sys.stdout.write("\n")
        return 2
    json.dump(result, sys.stdout, ensure_ascii=False, indent=2)
    sys.stdout.write("\n")
    return 1 if result["status"] == "MISSING" else 0


if __name__ == "__main__":
    sys.exit(main())
