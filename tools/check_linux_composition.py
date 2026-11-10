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


def retained_archives(linkmap: str) -> dict:
    """Ненулевые input sections только после gold Memory map, исключая discarded."""
    headers = ("\nMemory map\n", "\nLinker script and memory map\n")
    header = next((candidate for candidate in headers if candidate in linkmap), None)
    if header is None:
        raise ValueError("неподдерживаемый link map: нет gold/bfd memory map")
    retained = linkmap.split(header, 1)[1]
    pattern = re.compile(
        r"0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+(\S+\.(?:rlib|a))\(([^)]+)\)"
    )
    records: dict[str, dict] = {}
    for match in pattern.finditer(retained):
        address, size, archive, member = match.groups()
        if not int(address, 16) or not int(size, 16):
            continue
        entry = records.setdefault(
            Path(archive).name, {"sections": 0, "sample_members": []}
        )
        entry["sections"] += 1
        if len(entry["sample_members"]) < 4 and member not in entry["sample_members"]:
            entry["sample_members"].append(member)
    return records


def dependency_roles(metadata: dict, package_id: str) -> dict[str, list[str]]:
    """Normal closure отделена от build/proc-macro, dev рёбра не обходятся."""
    packages = {row["id"]: row for row in metadata["packages"]}
    nodes = {row["id"]: row for row in metadata["resolve"]["nodes"]}
    pending = [(package_id, "normal")]
    seen: set[tuple[str, str]] = set()
    while pending:
        current, role = pending.pop()
        if (current, role) in seen:
            continue
        seen.add((current, role))
        for dependency in nodes[current]["deps"]:
            target = dependency["pkg"]
            macro = any(
                "proc-macro" in row["kind"] for row in packages[target]["targets"]
            )
            for kind in dependency["dep_kinds"]:
                if kind["kind"] == "dev":
                    continue
                next_role = (
                    "build"
                    if role == "build" or kind["kind"] == "build" or macro
                    else "normal"
                )
                pending.append((target, next_role))
    result: dict[str, list[str]] = {}
    for identity, role in sorted(seen):
        result.setdefault(identity, []).append(role)
    return result


def validate_go_sources(path: Path, composition: dict) -> dict:
    """Сверяет закреплённые module identity и raw notices без сети."""
    report = json.loads(read_file(path, 16 * 1024 * 1024))
    expected = {
        tuple(row[1:4])
        for core in composition["cores"].values()
        for row in core["go"]["modules"]
        if row[0] == "dep"
    }
    found = set()
    for record in report["records"]:
        identity = (record["module"], record["version"], record["h1"])
        if identity not in expected or identity in found:
            raise ValueError("несовпадение/повтор Go module identity")
        found.add(identity)
        if record["status"] not in {"observed", "missing_text", "unresolved"}:
            raise ValueError("недопустимый Go status")

        def escape(value: str) -> str:
            return "".join("!" + c.lower() if c.isupper() else c for c in value)

        url = f"https://proxy.golang.org/{escape(identity[0])}/@v/{escape(identity[1])}.zip"
        if record["zip_url"] != url:
            raise ValueError("подмена primary module URL")
        if record["status"] != "observed":
            continue
        if record.get("h1_verified") is not True or not record.get("texts"):
            raise ValueError("нет подтверждения h1/texts")
        for text in record["texts"]:
            name = text["file"]
            if (
                not re.fullmatch(r"texts/[0-9a-f]{64}", name)
                or name.split("/")[1] != text["sha256"]
            ):
                raise ValueError("недопустимый путь Go notice")
            raw = read_file(path.parent / name, 4 * 1024 * 1024)
            if sha(raw) != text["sha256"]:
                raise ValueError("подмена Go notice")
    if found != expected:
        raise ValueError("неполный набор Go module records")
    manifest = ROOT / "packaging/licenses/linux-composition.json"
    if report["input_manifest_sha256"] != sha(read_file(manifest)):
        raise ValueError("Go sources связаны с другим binary manifest")
    return {
        "records": len(found),
        "observed": sum(r["status"] == "observed" for r in report["records"]),
    }


def validate_rust_evidence(directory: Path, report: dict) -> None:
    """Проверяет реальные новые ELF/maps вне TTL по опубликованному manifest."""
    for name, expected in report["evidence_sha256"].items():
        if Path(name).name != name or "\\" in name or name in {".", ".."}:
            raise ValueError("небезопасный путь Rust evidence")
        if sha(read_file(directory / name)) != expected:
            raise ValueError("Rust evidence hash mismatch")
    names = {row["binary"] for row in report["binaries"]}
    if (
        names != {"yonik-desktop", "yonik-helper", "yonik-worker"}
        or len(report["binaries"]) != 3
    ):
        raise ValueError("неверный набор новых Rust ELF")
    for row in report["binaries"]:
        if sha(read_file(directory / "new-binaries" / row["binary"])) != row["sha256"]:
            raise ValueError("Rust ELF hash mismatch")
        mapfile = directory / (row["binary"].replace("-", "_") + ".map")
        raw = read_file(mapfile)
        if sha(raw) != row["map_sha256"]:
            raise ValueError("Rust map hash mismatch")
        retained = retained_archives(raw.decode())
        for archive in row["archive_mentions"]:
            if archive.get("retained_input_sections") != retained.get(
                archive["archive"]
            ):
                raise ValueError("Rust retained section evidence mismatch")


def validate_bundle(directory: Path, composition: dict) -> dict:
    """Проверяет сохранённые raw bytes и cross-links; status partial не повышается."""
    names = (
        "go-sources.json",
        "go-toolchains.json",
        "go-toolchain-sources.json",
        "core-sources.json",
        "qt-sources.json",
        "rust-toolchain.json",
        "arch-recipes.json",
        "arch/arch-packages.json",
        "qt-runtime.json",
        "rust-build.json",
    )
    reports = {}
    count = 0

    def visit(value: object, base: Path) -> None:
        nonlocal count
        if isinstance(value, list):
            for row in value:
                visit(row, base)
        elif isinstance(value, dict):
            if "file" in value:
                name = value["file"]
                if not isinstance(name, str) or not re.fullmatch(
                    r"(?:texts|recipes)/[0-9a-f]{64}", name
                ):
                    raise ValueError("небезопасный путь evidence")
                if sha(read_file(base / name, 4 * 1024 * 1024)) != value["sha256"]:
                    raise ValueError("evidence hash mismatch")
                count += 1
            for child in value.values():
                if isinstance(child, (list, dict)):
                    visit(child, base)

    for name in names:
        path = directory / name
        report = json.loads(read_file(path, 16 * 1024 * 1024))
        visit(report, path.parent)
        reports[name] = report
    validate_go_sources(directory / "go-sources.json", composition)
    toolchains = {core["go"]["toolchain"] for core in composition["cores"].values()}
    if {row["ref"] for row in reports["go-toolchain-sources.json"]} != toolchains:
        raise ValueError("Go toolchain source version mismatch")
    sing = next(
        core
        for name, core in composition["cores"].items()
        if name.startswith("sing-box-")
    )
    revision = next(
        value.split("=", 1)[1]
        for value in sing["go"]["settings"]
        if value.startswith("vcs.revision=")
    )
    source = next(
        row for row in reports["core-sources.json"] if row["core"] == "sing-box"
    )
    if source["commit"] != revision:
        raise ValueError("sing-box source/buildinfo revision mismatch")
    runtime = reports["qt-runtime.json"]
    desktop = composition["elf"]["usr/bin/yonik-desktop"]["sha256"]
    if runtime["binary_sha256"] != desktop:
        raise ValueError("runtime относится к другому desktop")
    arch = reports["arch/arch-packages.json"]
    if arch["runtime_binary_sha256"] != desktop:
        raise ValueError("Arch evidence относится к другому desktop")
    owners = {row["name"]: row for row in arch["records"]}
    recipes = {row["pkgbase"]: row for row in reports["arch-recipes.json"]}
    for row in runtime["files"]:
        for owner in row.get("owner", []):
            package = owners[owner]
            if (
                package["runtime_payload_sha256"].get(row["resolved_path"].lstrip("/"))
                != row["sha256"]
            ):
                raise ValueError("runtime/package payload mismatch")
            if row["package_version"] != owner + " " + package["version"]:
                raise ValueError("runtime/package version mismatch")
    for package in owners.values():
        recipe = recipes[package["pkgbase"]]
        expected = next(
            line.split(" = ", 1)[1]
            for line in package["buildinfo"].splitlines()
            if line.startswith("pkgbuild_sha256sum = ")
        )
        if recipe["status"] != "verified" or recipe["sha256"] != expected:
            raise ValueError("Arch BUILDINFO/primary recipe mismatch")
    for row in reports["qt-sources.json"]:
        if row["checksum_matches"] != (
            row["git_archive_sha256"] == row["arch_source_checksum"]
        ):
            raise ValueError("Qt source checksum status неверен")
        recipe = recipes[row["pkgbase"]]
        raw = read_file(directory / recipe["file"]).decode()
        expected = re.search(r"sha256sums=\('([0-9a-f]{64})'", raw)
        if (
            expected is None
            or expected.group(1) != row["arch_source_checksum"]
            or recipe["sha256"] != row["arch_recipe_sha256"]
        ):
            raise ValueError("Qt source/Arch recipe checksum mismatch")
    rust = reports["rust-build.json"]
    if rust["status"] != "observed_new_build":
        raise ValueError("неверный статус новой Rust сборки")
    return {
        "status": "partial",
        "raw_references_checked": count,
        "arch_packages": len(owners),
        "go_modules": len(reports["go-sources.json"]["records"]),
        "qt_source_checksum_mismatches": sum(
            not row["checksum_matches"] for row in reports["qt-sources.json"]
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--core", action="append", default=[], type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--check", type=Path)
    parser.add_argument("--go-sources", type=Path)
    parser.add_argument("--evidence-bundle", type=Path)
    parser.add_argument("--rust-evidence", type=Path)
    args = parser.parse_args()
    result = scan(args.package, args.core)
    if args.rust_evidence:
        if not args.evidence_bundle:
            raise ValueError("--rust-evidence требует --evidence-bundle")
        report = json.loads(read_file(args.evidence_bundle / "rust-build.json"))
        validate_rust_evidence(args.rust_evidence, report)
    if args.evidence_bundle:
        validate_bundle(args.evidence_bundle, result)
    if args.go_sources:
        validate_go_sources(args.go_sources, result)
    if args.check and json.loads(read_file(args.check)) != result:
        raise ValueError("composition manifest не соответствует входам")
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
