#!/usr/bin/env python3
"""Авторизованные freeze/collect операции; candidate CLI не управляет producer."""

from __future__ import annotations

import argparse
import fcntl
import gzip
import hashlib
import io
import json
import os
import re
import shlex
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
from contextlib import contextmanager
from pathlib import Path, PurePosixPath
from typing import Iterator, Protocol
from urllib.parse import urlsplit

from _p25_capture_producer import retain as retain_build_capture
from check_package_provenance import (
    NotReady,
    _private_directory,
    _read_request,
    canonical_repo,
    git,
    input_path,
    sha,
    validate_source_members,
)

SPEC = "260bc50eeb4f3a40ac97cd9a54ff0ea943a833ef"
VERSIONS = {"xray": "26.10.31", "sing-box": "1.14.4"}
SLOT = "/home/myrs/projects/yonik/local/orchestration/with-rust-slot"
RUST_PROFILE = "1.85.1-x86_64-unknown-linux-gnu"
RUST_BIN = Path("/home/myrs/.rustup/toolchains") / RUST_PROFILE / "bin"
CONTROLLED_ENV = (
    "PATH",
    "RUSTUP_TOOLCHAIN",
    "RUSTC",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_JOBS",
    "YONIK_BUILD_ID",
    "GOMAXPROCS",
    "GOENV",
    "GOWORK",
    "GOTOOLCHAIN",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_CONFIG_GLOBAL",
    "GIT_TERMINAL_PROMPT",
    "TMPDIR",
)
RUST_REUSE_PRIOR = "linux-controller-rehearsal-14229843-msrv85-2"
RUST_REUSE_TARGET = "linux-controller-rehearsal-14229843-msrv85-3"
RUST_REUSE_REVIEW = "969f65b68f9283f81d77d9c6f1f2c00d8f26934e"
COLLECTED_REUSE_PRIOR = "linux-controller-rehearsal-14229843-msrv85-3"
COLLECTED_REUSE_TARGET = "linux-controller-rehearsal-14229843-msrv85-mtree4"
COLLECTED_REUSE_REVIEW = "2599fe70ca93ebd14baaa8932e9de69fc5d66a94"
TRANSIENT = Path("/var/tmp/codex-myrs/transient")
JOB_SOURCE = "ce356b452675036fab3388ef64ea03f8f00910ce"
JOB_BINARIES = {
    "sing-box": "8a1683227b52a94ef904f8cdfbb676b16f9cf4409e68c046d3a5c7acebe591b5",
    "xray": "68d802801b6be344f6fda5546803f2f38d49ace1cc50c4e37155fd0818d6083d",
}
CORE_JOB = Path("/home/myrs/projects/yonik/local/evidence/full-pinned-158-20261113")
RUST = {
    "yonik-desktop": "usr/bin/yonik-desktop",
    "yonik-helper": "usr/lib/yonik/yonik-helper",
    "yonik-worker": "usr/lib/yonik/yonik-worker",
}
LINK_FLAGS = [
    flag
    for symbol in (
        "qml_module_cloud_highpriority_yonik",
        "crate_yonik_desktop",
        "crate_cxx_qt_lib",
        "crate_cxx_qt",
    )
    for flag in ("-C", "link-arg=-Wl,--undefined=cxx_qt_init_" + symbol)
]
RECIPE = {
    "schema": 1,
    "target": "x86_64-unknown-linux-gnu",
    "native_spec_commit": SPEC,
    "rust_commands": [
        [
            "cargo",
            "rustc",
            "--locked",
            "--release",
            "-p",
            "yonik-desktop",
            "--features",
            "gui",
            "--bin",
            "yonik-desktop",
            "--",
            *LINK_FLAGS,
        ],
        [
            "cargo",
            "build",
            "--locked",
            "--release",
            "-p",
            "yonik-linux",
            "--bin",
            "yonik-helper",
            "--bin",
            "yonik-worker",
        ],
    ],
    "core_members": {engine: f"usr/lib/yonik/cores/{engine}" for engine in VERSIONS},
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def release_name(value: str) -> None:
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,63}", value):
        raise NotReady("Некорректный release-id")


def registry_path(path: Path) -> None:
    for component in (*reversed(path.parents), path):
        if component.is_symlink():
            raise NotReady("Ссылка в пути registry")


def write_record(path: Path, record: dict) -> None:
    data = json.dumps(record, ensure_ascii=False, indent=2) + "\n"
    if len(data.encode()) > 1024 * 1024:
        raise NotReady("Registry record превышает 1 MiB")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


@contextmanager
def release_lock(registry: Path, release: str, *, writer: bool) -> Iterator[Path]:
    release_name(release)
    registry_path(registry)
    root_fd = _private_directory(registry)
    try:
        fcntl.flock(root_fd, fcntl.LOCK_SH)
        fd = _private_directory(release, root_fd)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX if writer else fcntl.LOCK_SH)
            yield registry / release
        finally:
            os.close(fd)
    finally:
        os.close(root_fd)


def read_context(registry: Path, release: str) -> dict:
    release_name(release)
    try:
        return _read_request(registry, release, "collected.json")
    except FileNotFoundError:
        return _read_request(registry, release)


def snapshot(repo: Path, record: dict, archive: Path, destination: Path) -> None:
    validate_source_members(archive)
    if sha(archive) != record["archive_sha256"]:
        raise NotReady("Подменён frozen source archive")
    commit = record["source"]["commit"]
    if (
        git(repo, "rev-parse", commit + "^{tree}").decode().strip()
        != record["source"]["tree"]
    ):
        raise NotReady("Подменён frozen tree")
    expected = {}
    for row in git(repo, "ls-tree", "-rz", commit).split(b"\0"):
        if not row:
            continue
        metadata, name = row.split(b"\t", 1)
        mode, kind, oid = metadata.split()
        if kind != b"blob" or mode not in (b"100644", b"100755"):
            raise NotReady("Frozen tree содержит ссылку/submodule")
        expected[name.decode()] = (mode, oid)
    seen = set()
    with tarfile.open(archive) as stream:
        for entry in stream:
            if entry.isdir():
                continue
            name = entry.name.split("/", 1)[1]
            if name in seen or name not in expected:
                raise NotReady("Лишний frozen source member")
            mode, oid = expected[name]
            data = git(repo, "cat-file", "blob", oid.decode())
            incoming = stream.extractfile(entry)
            if (
                incoming is None
                or entry.size != len(data)
                or incoming.read(len(data) + 1) != data
                or bool(entry.mode & 0o111) != (mode == b"100755")
            ):
                raise NotReady("Подменён frozen source blob")
            seen.add(name)
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            target.chmod(0o755 if mode == b"100755" else 0o644)
    if seen != set(expected):
        raise NotReady("Неполный frozen source archive")


def tooling_hashes() -> dict[str, str]:
    base = Path(__file__).parent
    return {
        name: sha(base / name)
        for name in (
            "release_controller.py",
            "_p25_capture_producer.py",
            "p25_capture_build_closure.py",
            "check_package_provenance.py",
            "check_arch_package.py",
            "compile_notices.py",
            "check_linux_notices.py",
        )
    }


def final_capture_profile() -> dict:
    from _p25_final_profile import PROFILE

    names = (
        "_p25_final_profile.py",
        "p25_cxx_capture_wrapper.py",
        "_p25_capture_producer.py",
        "p25_capture_build_closure.py",
    )
    body = {
        "spec": PROFILE,
        "tooling_sha256": {name: sha(Path(__file__).parent / name) for name in names},
    }
    return {**body, "sha256": digest(json.dumps(body, sort_keys=True).encode())}


def verify_final_profile(record: dict) -> bool:
    if "final_profile" not in record:
        return False
    if record["release_id"] in (RUST_REUSE_TARGET, COLLECTED_REUSE_TARGET):
        raise NotReady("Final instrumentation запрещён для historical reuse")
    if record["final_profile"] != final_capture_profile():
        raise NotReady("Final instrumentation profile/tooling hash mismatch")
    return True


def _freeze(
    repo: Path,
    registry: Path,
    release: str,
    commit: str,
    final_profile: str | None = None,
) -> dict:
    release_name(release)
    registry_path(registry)
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise NotReady("Root должен выбрать полный source SHA")
    selected = None
    if final_profile is not None:
        if final_profile != "p25-final-cxx-v1" or release in (
            RUST_REUSE_TARGET,
            COLLECTED_REUSE_TARGET,
        ):
            raise NotReady("Неверный final profile/reuse")
        selected = final_capture_profile()
        for name, checksum in selected["tooling_sha256"].items():
            if digest(git(repo, "show", commit + ":tools/" + name)) != checksum:
                raise NotReady("Final source не содержит frozen capture tooling")
        wrapper_entry = git(repo, "ls-tree", commit, "tools/p25_cxx_capture_wrapper.py")
        if not wrapper_entry.startswith(b"100755 blob "):
            raise NotReady("Frozen CXX wrapper должен быть executable")
    source = {
        "commit": git(repo, "rev-parse", commit + "^{commit}").decode().strip(),
        "tree": git(repo, "rev-parse", commit + "^{tree}").decode().strip(),
    }
    registry.mkdir(mode=0o700, exist_ok=True)
    fd = _private_directory(registry)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        directory = registry / release
        directory.mkdir(mode=0o700)
        archive = git(repo, "archive", "--format=tar", "--prefix=src/", commit)
        with (directory / "source.tar").open("xb") as stream:
            stream.write(archive)
        (directory / "source.tar").chmod(0o600)
        record = {
            "schema": 2,
            "release_id": release,
            "state": "FROZEN",
            "source": source,
            "archive_sha256": digest(archive),
            "recipe": RECIPE,
            "tooling_sha256": tooling_hashes(),
        }
        if selected is not None:
            record["final_profile"] = selected
        write_record(directory / "frozen.json", record)
        return record
    finally:
        os.close(fd)


def freeze_release(release: str, commit: str, final_profile: str | None = None) -> dict:
    """Root/CI выбирает SHA; архив всегда получается самим controller из Git."""
    repo = canonical_repo()
    return _freeze(
        repo, repo / "local/release-controller", release, commit, final_profile
    )


def core_metadata(root: Path) -> dict:
    if json.loads((root / "packaging/arch/release-recipe.json").read_text()) != RECIPE:
        raise NotReady("Непроверенный frozen release recipe")
    result = {}
    for engine, version in VERSIONS.items():
        folder = root / f"adapters/{engine}/v{version}"
        lock = json.loads((folder / "source-lock.json").read_text())
        codec = json.loads((folder / "native-broker-codec.json").read_text())
        if (
            lock["tag"] != "v" + version
            or not re.fullmatch(r"[a-f0-9]{40}", lock["commit"])
            or not re.fullmatch(r"[a-f0-9]{64}", lock["archive_sha256"])
            or codec["frozen_spec_commit"] != SPEC
            or sha(folder / "native-broker-codec.patch") != codec["patch_sha256"]
        ):
            raise NotReady("Несовместимые core pins/spec/patch")
        for name, expected in codec["frozen_sources_sha256"].items():
            if sha(input_path(root, name)) != expected:
                raise NotReady("Core ABI не соответствует frozen sources")
        result[engine] = {"lock": lock, "codec": codec, "folder": str(folder)}
    return result


def source_offers(root: Path) -> dict:
    rows = json.loads(
        (root / "packaging/licenses/composition/go-sources.json").read_text()
    )["records"]
    result = {}
    for row in rows:
        url = urlsplit(row["zip_url"])
        if (
            not isinstance(row["module"], str)
            or not isinstance(row["version"], str)
            or not re.fullmatch(r"[a-f0-9]{64}", row["zip_sha256"])
            or url.scheme != "https"
            or not url.hostname
            or url.username
        ):
            raise NotReady("Некорректный frozen Go source offer")
        key = (row["module"], row["version"])
        if key in result and result[key] != row:
            raise NotReady("Конфликт frozen Go source offer module/version")
        result[key] = row
    return result


def elf_header(data: bytes, file_size: int | None = None) -> dict:
    if len(data) < 64:
        raise NotReady("Обрезанный ELF")
    fields = struct.unpack("<16sHHIQQQIHHHHHH", data[:64])
    ident, kind, machine, version, _, offset, _, _, header, size, count, *_ = fields
    if (
        ident[:7] != b"\x7fELF\x02\x01\x01"
        or machine != 62
        or kind not in (2, 3)
        or version != 1
        or header != 64
        or size != 56
        or not count
        or offset < 64
        or offset + size * count > (file_size if file_size is not None else len(data))
    ):
        raise NotReady("Несовместимый ELF64 little-endian x86_64")
    return {"class": 64, "endian": "little", "machine": machine, "type": kind}


def elf_file(path: Path) -> dict:
    with path.open("rb") as stream:
        return elf_header(stream.read(64), os.fstat(stream.fileno()).st_size)


class TrustedRunner(Protocol):
    commands: list[dict]

    def run(self, argv: list[str], cwd: Path, env: dict[str, str]) -> bytes: ...


class LocalRunner:
    """Без shell; команды/выходы/exit сохраняются controller, не импортируются."""

    def __init__(self, logs: Path) -> None:
        self.logs = logs
        self.logs.mkdir(mode=0o700)
        self.commands: list[dict] = []

    def run(self, argv: list[str], cwd: Path, env: dict[str, str]) -> bytes:
        index = len(self.commands)
        row = {
            "argv": argv,
            "cwd": str(cwd),
            "exit": None,
            "controlled_env": {key: env[key] for key in CONTROLLED_ENV if key in env},
        }
        self.commands.append(row)
        with (
            (self.logs / f"{index}.stdout").open("xb") as out,
            (self.logs / f"{index}.stderr").open("xb") as err,
        ):
            try:
                process = subprocess.Popen(
                    argv,
                    cwd=cwd,
                    env=env,
                    stdout=out,
                    stderr=err,
                    stdin=subprocess.DEVNULL,
                    start_new_session=True,
                )
                try:
                    row["exit"] = process.wait(timeout=3600)
                    if row["exit"]:
                        raise subprocess.CalledProcessError(row["exit"], argv)
                finally:
                    # Native builder сам владеет отдельными Go process groups:
                    # даём его SIGTERM handler выполнить cleanup до SIGKILL.
                    if process.poll() is None:
                        try:
                            os.killpg(process.pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                        try:
                            process.wait(timeout=15)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                    if row["exit"] is None:
                        row["exit"] = process.returncode
            finally:
                write_record(self.logs / f"{index}.command.json", row)
        return (self.logs / f"{index}.stdout").read_bytes()


def build_env(task: Path, commit: str) -> dict[str, str]:
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("CARGO_", "RUST", "GO", "GIT_", "YONIK_"))
        and key not in ("RUSTFLAGS", "RUSTDOCFLAGS", "LD_PRELOAD", "LD_LIBRARY_PATH")
    }
    env.update(
        PATH=str(RUST_BIN) + os.pathsep + env.get("PATH", os.defpath),
        RUSTUP_TOOLCHAIN=RUST_PROFILE,
        RUSTC=str(RUST_BIN / "rustc"),
        CARGO_TARGET_DIR=str(task / "target"),
        CARGO_BUILD_JOBS="1",
        YONIK_BUILD_ID="0.1.0+" + commit,
        GOMAXPROCS="1",
        GOENV="off",
        GOWORK="off",
        GOTOOLCHAIN="local",
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_TERMINAL_PROMPT="0",
        TMPDIR=str(task / "tmp"),
    )
    (task / "tmp").mkdir()
    return env


def archive_tree(root: Path, output: Path) -> None:
    with tarfile.open(output, "x") as archive:
        for folder, directories, names in os.walk(root, followlinks=False):
            directories[:] = sorted(name for name in directories if name != ".git")
            for name in directories:
                if (Path(folder) / name).is_symlink():
                    raise NotReady("Ссылка в patched source")
            for name in sorted(names):
                path = Path(folder) / name
                relative = path.relative_to(root).as_posix()
                input_path(root, relative)
                entry = archive.gettarinfo(str(path), "src/" + relative)
                entry.uid = entry.gid = 0
                entry.uname = entry.gname = ""
                with path.open("rb") as incoming:
                    archive.addfile(entry, incoming)


def check_core(
    root: Path,
    engine: str,
    output: Path,
    source: Path,
    runner: TrustedRunner,
    env: dict[str, str],
) -> dict:
    metadata = core_metadata(root)[engine]
    lock, codec = metadata["lock"], metadata["codec"]
    binary = input_path(output, engine)
    manifest = json.loads(input_path(output, "manifest.json").read_text())
    if (
        manifest["engine"] != engine
        or manifest["engine_version"] != VERSIONS[engine]
        or manifest["source_commit"] != lock["commit"]
        or manifest["source_tag"] != lock["tag"]
        or manifest["source_url"] != lock["source_url"]
        or manifest["module_sha256"] != lock["module_sha256"]
        or manifest["binary_sha256"] != sha(binary)
        or manifest["go_env"]["GOOS"] != "linux"
        or manifest["go_env"]["GOARCH"] != "amd64"
        or manifest["native_codec_component"]["frozen_spec_commit"] != SPEC
        or manifest["native_codec_component"]["patch_sha256"] != codec["patch_sha256"]
    ):
        raise NotReady("Core manifest не соответствует frozen pins/ABI")
    component = manifest["native_codec_component"]
    for field in (
        "callsite_files_sha256",
        "runtime_hooks_connected",
        "owned_tun_constructor_connected",
        "routing_entry_guard_connected",
    ):
        default = {} if field == "callsite_files_sha256" else False
        if component.get(field, default) != codec.get(field, default):
            raise NotReady("Core component metadata не соответствует frozen codec")
    prebuild = input_path(output, "prebuild.json")
    if sha(prebuild) != manifest["prebuild_sha256"]:
        raise NotReady("Подменён native prebuild record")
    before = json.loads(prebuild.read_text())
    if (
        before["source_commit"] != lock["commit"]
        or before["module_sha256"] != lock["module_sha256"]
    ):
        raise NotReady("Prebuild не соответствует frozen source/module")
    expected_patches = {path.name for path in Path(metadata["folder"]).glob("*.patch")}
    expected_patches.discard("native-broker-codec.patch")
    if {row["name"] for row in manifest["patches"]} != expected_patches:
        raise NotReady("Неполный/лишний core patch closure")
    for record in manifest["patches"]:
        if (
            sha(input_path(Path(metadata["folder"]), record["name"]))
            != record["sha256"]
        ):
            raise NotReady("Подменён core patch")
    for name, expected in lock["module_sha256"].items():
        if sha(input_path(source, name)) != expected:
            raise NotReady("Подменён core module")
    for name, expected in {
        **{
            f"common/yonikbroker/{name}": value
            for name, value in codec["package_files_sha256"].items()
        },
        **codec.get("callsite_files_sha256", {}),
    }.items():
        if sha(input_path(source, name)) != expected:
            raise NotReady("Patched source package не соответствует codec metadata")
    build_info = runner.run(["go", "version", "-m", str(binary)], root, env).decode()
    if (
        "\tbuild\tGOOS=linux" not in build_info
        or "\tbuild\tGOARCH=amd64" not in build_info
    ):
        raise NotReady("GoBuildInfo несовместим с Linux amd64")
    dependencies = resolve_dependencies(build_info, source_offers(root))
    return {
        "manifest": manifest,
        "elf": elf_file(binary),
        "go_build_info": build_info,
        "abi": codec,
        "dependencies": dependencies,
    }


def resolve_dependencies(build_info: str, pinned: dict) -> list[dict]:
    dependencies = {}
    for line in build_info.splitlines():
        fields = line.strip().split("\t")
        if fields[0] == "=>":
            raise NotReady("Go replacement требует отдельного frozen source binding")
        if fields[0] != "dep":
            continue
        if len(fields) != 4:
            raise NotReady("Неполный Go dependency BuildInfo")
        _, module, version, checksum = fields
        row = pinned.get((module, version))
        if row is None or row["h1"] != checksum:
            raise NotReady("Go dependency отсутствует в frozen compliance")
        resolved = {key: row[key] for key in ("module", "version", "h1", "zip_sha256")}
        if module in dependencies and dependencies[module] != resolved:
            raise NotReady("Конфликт Go dependency в одном core")
        dependencies[module] = resolved
    return [dependencies[name] for name in sorted(dependencies)]


def required_offers(root: Path, cores: dict) -> list[dict]:
    pinned = source_offers(root)
    required = {}
    for core in cores.values():
        resolved = resolve_dependencies(core["go_build_info"], pinned)
        if resolved != core["dependencies"]:
            raise NotReady("Подменён core/source offer binding")
        for row in resolved:
            key = (row["module"], row["version"])
            required[key] = pinned[key]
    return [required[key] for key in sorted(required)]


def pkginfo(root: Path) -> bytes:
    from check_arch_package import validate_recipe

    validate_recipe(root / "packaging/arch/PKGBUILD")
    fields = {}
    for line in (root / "packaging/arch/PKGBUILD").read_text().splitlines():
        name, separator, value = line.partition("=")
        if separator and name in ("pkgname", "pkgver", "pkgrel", "arch", "depends"):
            fields[name] = shlex.split(value.strip("()"))
    rows = [
        ("pkgname", fields["pkgname"][0]),
        ("pkgver", fields["pkgver"][0] + "-" + fields["pkgrel"][0]),
        ("arch", fields["arch"][0]),
        *(("depend", value) for value in fields["depends"]),
    ]
    return "".join(f"{key} = {value}\n" for key, value in rows).encode()


def package_files(
    root: Path, artifacts: Path, core_records: dict
) -> dict[str, tuple[int, bytes | Path]]:
    from check_arch_package import expected_files

    files = {}
    reverse = {member: name for name, member in RUST.items()}
    for member, (mode, data) in expected_files(root).items():
        files[member] = (mode, artifacts / reverse[member] if data is None else data)
    for engine in VERSIONS:
        files[RECIPE["core_members"][engine]] = (0o755, artifacts / engine / engine)
        for name in ("upstream.tar", "patched.tar", "manifest.json", "prebuild.json"):
            files[f"usr/share/yonik/sources/{engine}/{name}"] = (
                0o644,
                artifacts / engine / name,
            )
        folder = Path(core_metadata(root)[engine]["folder"])
        for path in folder.iterdir():
            if path.suffix in (".json", ".patch"):
                files[f"usr/share/yonik/sources/{engine}/{path.name}"] = (0o644, path)
    for row in required_offers(root, core_records):
        files[f"usr/share/yonik/sources/go/{row['zip_sha256']}.zip"] = (
            0o644,
            artifacts / "go-sources" / (row["zip_sha256"] + ".zip"),
        )
    files["usr/share/yonik/sources/release-source.tar"] = (
        0o644,
        artifacts / "source.tar",
    )
    return files


def write_package(
    path: Path, files: dict[str, tuple[int, bytes | Path]], info: bytes
) -> dict:
    # Metadata берутся из тех же TarInfo, которые записываются в архив.
    # Host stat UID/mtime не участвуют; текущий package contract запрещает links.
    if len(files) > 4096 or len(info) > 64 * 1024:
        raise NotReady("Превышен лимит package metadata")
    for name, (mode, content) in files.items():
        parts = name.split("/")
        if (
            name in (".MTREE", ".PKGINFO")
            or any(part in ("", ".", "..") for part in parts)
            or len(name.encode()) > 4096
            or "\0" in name
            or mode not in (0o644, 0o755)
        ):
            raise NotReady("Недопустимый package path/mode")
    bindings = {}
    lines = ["#mtree"]
    with tarfile.open(path, "x") as archive:
        for name, (mode, content) in {".PKGINFO": (0o644, info), **files}.items():
            entry = tarfile.TarInfo(name)
            entry.uid = entry.gid = entry.mtime = 0
            entry.mode = mode
            if isinstance(content, Path):
                entry.size = content.stat().st_size
                checksum = sha(content)
                with content.open("rb") as incoming:
                    archive.addfile(entry, incoming)
            else:
                entry.size = len(content)
                checksum = digest(content)
                archive.addfile(entry, io.BytesIO(content))
            escaped = "".join(
                chr(byte)
                if 33 <= byte <= 126 and byte not in (35, 61, 92)
                else f"\\{byte:03o}"
                for byte in ("./" + name).encode()
            )
            lines.append(
                f"{escaped} type=file uid={entry.uid} gid={entry.gid} "
                f"mode={entry.mode:o} time={entry.mtime} size={entry.size} "
                f"sha256digest={checksum}"
            )
            bindings[name] = {"mode": mode, "sha256": checksum}
        plain = ("\n".join(lines) + "\n").encode("ascii")
        if len(plain) > 1024 * 1024:
            raise NotReady("Превышен лимит .MTREE")
        data = gzip.compress(plain, mtime=0)
        entry = tarfile.TarInfo(".MTREE")
        entry.uid = entry.gid = entry.mtime = 0
        entry.mode, entry.size = 0o644, len(data)
        archive.addfile(entry, io.BytesIO(data))
        bindings[".MTREE"] = {"mode": entry.mode, "sha256": digest(data)}
    return bindings


def _ingest_core_job(repo: Path, registry: Path, release: str, job: Path) -> dict:
    """Только root/controller operation; production job path фиксирован в tooling."""
    with release_lock(registry, release, writer=True) as artifacts:
        request = _read_request(registry, release)
        with tempfile.TemporaryDirectory(prefix="yonik-ingest-") as temporary:
            root = Path(temporary)
            snapshot(repo, request, input_path(artifacts, "source.tar"), root)
            metadata = core_metadata(root)
            acceptance = json.loads(
                input_path(job, "acceptance-context.json").read_text()
            )
            expected_builder = digest(
                git(repo, "show", JOB_SOURCE + ":tools/build_native_adapter.py")
            )
            if acceptance["builder_sha256"] != expected_builder:
                raise NotReady("Trusted job использовал другой builder revision")
            for name, checksum in acceptance["patches"].items():
                if sha(input_path(root, name)) != checksum:
                    raise NotReady("Trusted native job не соответствует frozen patches")
            capability = json.loads(
                input_path(job, "capability-abi-manifest.json").read_text()
            )
            destination = artifacts / "accepted-cores"
            destination.mkdir(mode=0o700)
            for engine in VERSIONS:
                incoming = job / engine
                binary = input_path(incoming, engine)
                manifest = json.loads(input_path(incoming, "manifest.json").read_text())
                if (
                    sha(binary) != JOB_BINARIES[engine]
                    or capability[engine]["binary_sha256"] != sha(binary)
                    or capability[engine]["frozen_spec_commit"] != SPEC
                    or manifest["source_commit"] != metadata[engine]["lock"]["commit"]
                ):
                    raise NotReady("Неверный actual accepted core job")
                elf_file(binary)
                target = destination / engine
                target.mkdir(mode=0o700)
                for name in (engine, "manifest.json", "prebuild.json"):
                    with (
                        input_path(incoming, name).open("rb") as source,
                        (target / name).open("xb") as output,
                    ):
                        shutil.copyfileobj(source, output)
            record = {
                "source_review_commit": JOB_SOURCE,
                "job_path": str(job),
                "binary_sha256": JOB_BINARIES,
                "acceptance_context_sha256": sha(job / "acceptance-context.json"),
            }
            write_record(artifacts / "trusted-core-job.json", record)
            return record


def ingest_core_job(release: str) -> dict:
    repo = canonical_repo()
    return _ingest_core_job(repo, repo / "local/release-controller", release, CORE_JOB)


def restore_job_source(
    root: Path,
    source: Path,
    engine: str,
    manifest: dict,
    runner: TrustedRunner,
    env: dict[str, str],
) -> None:
    metadata = core_metadata(root)[engine]
    lock = metadata["lock"]
    source.parent.mkdir()
    runner.run(
        [
            "git",
            "clone",
            "--depth=1",
            "--branch",
            lock["tag"],
            "--",
            lock["source_url"],
            str(source),
        ],
        root,
        env,
    )
    head = runner.run(["git", "rev-parse", "HEAD"], source, env).decode().strip()
    if head != lock["commit"]:
        raise NotReady("Trusted core job source reconstruction: неверный HEAD")
    folder = Path(metadata["folder"])
    patches = [input_path(folder, row["name"]) for row in manifest["patches"]]
    patches.append(input_path(folder, "native-broker-codec.patch"))
    for patch in patches:
        runner.run(["git", "apply", "--check", str(patch)], source, env)
        runner.run(["git", "apply", str(patch)], source, env)


def rust_reuse_pins(repo: Path) -> dict:
    # Root согласовал именно эти partial facts, не произвольный caller record.
    path = f"docs/evidence/{RUST_REUSE_PRIOR}/registry-inventory.json"
    inventory = json.loads(git(repo, "show", RUST_REUSE_REVIEW + ":" + path))
    prefix = (
        "/home/myrs/projects/yonik/local/release-controller/" + RUST_REUSE_PRIOR + "/"
    )
    pins = {}
    for row in inventory:
        if not row["path"].startswith(prefix):
            raise NotReady("Неверный fixed partial-build inventory")
        relative = row["path"][len(prefix) :]
        parts = PurePosixPath(relative).parts
        if not parts or ".." in parts or relative in pins:
            raise NotReady("Некорректный partial-build inventory path")
        pins[relative] = {"size": row["size"], "sha256": row["sha256"]}
    return pins


def read_pinned_partial(directory: Path, relative: str, pin: dict) -> bytes:
    parts = PurePosixPath(relative).parts
    if not parts or relative.startswith("/") or ".." in parts:
        raise NotReady("Недопустимый partial-build path")
    parent = _private_directory(directory)
    try:
        for part in parts[:-1]:
            child = _private_directory(part, parent)
            os.close(parent)
            parent = child
        fd = os.open(
            parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent
        )
        with os.fdopen(fd, "rb") as stream:
            metadata = os.fstat(stream.fileno())
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != os.getuid()
                or metadata.st_nlink != 1
                or metadata.st_mode & 0o022
                or metadata.st_size != pin["size"]
                or (
                    (
                        relative
                        in ("frozen.json", "failure.json", "trusted-core-job.json")
                        or relative.endswith(".command.json")
                    )
                    and stat.S_IMODE(metadata.st_mode) != 0o600
                )
            ):
                raise NotReady("Небезопасный/изменённый partial-build artifact")
            if hashlib.file_digest(stream, "sha256").hexdigest() != pin["sha256"]:
                raise NotReady("Partial-build SHA mismatch")
            # Большие core/source archives только хешируются; Rust и records читаются.
            if (
                relative in RUST
                or relative.endswith(".json")
                or relative in ("controller-logs/0.stdout", "controller-logs/1.stdout")
            ):
                stream.seek(0)
                return stream.read()
            return b""
    finally:
        os.close(parent)


def rust_toolchain_hashes() -> dict[str, str]:
    return {name: sha(RUST_BIN / name) for name in ("rustc", "cargo")}


def reuse_rust(
    repo: Path,
    registry: Path,
    release: str,
    record: dict,
    artifacts: Path,
    toolchains: dict,
) -> dict:
    if release != RUST_REUSE_TARGET:
        raise NotReady("Rust reuse разрешён только fixed root-selected rehearsal ID")
    pins = rust_reuse_pins(repo)
    required = {
        "frozen.json",
        "failure.json",
        "source.tar",
        *RUST,
        *{f"controller-logs/{i}.command.json" for i in range(4)},
        "controller-logs/0.stdout",
        "controller-logs/1.stdout",
    }
    if not required <= pins.keys():
        raise NotReady("Нет fixed partial-build facts")
    with release_lock(registry, RUST_REUSE_PRIOR, writer=False) as prior:
        data = {
            name: read_pinned_partial(prior, name, pin) for name, pin in pins.items()
        }
        frozen = json.loads(data["frozen.json"])
        if any(
            frozen[key] != record[key] for key in ("source", "recipe", "archive_sha256")
        ):
            raise NotReady("Rust reuse source/tree/archive/recipe mismatch")
        # Tooling мог измениться только после prior job: его старые hashes проверяются
        # по согласованному Git revision, а новый freeze связывает новые hashes.
        for name, checksum in frozen["tooling_sha256"].items():
            if (
                digest(git(repo, "show", RUST_REUSE_REVIEW + ":tools/" + name))
                != checksum
            ):
                raise NotReady("Неверный prior controller tooling hash")
        failure = json.loads(data["failure.json"])
        if (
            failure.get("status") != "BLOCKED"
            or failure.get("final_linux_acceptance") is not False
            or failure.get("reason") != "Некорректный frozen Go source offer"
        ):
            raise NotReady("Неверный prior partial-build failure boundary")
        prior_task = TRANSIENT / ("release-controller-" + RUST_REUSE_PRIOR)
        commands = [
            ["rustc", "--version"],
            ["cargo", "--version"],
            *[[SLOT, *command] for command in RECIPE["rust_commands"]],
        ]
        for index, expected in enumerate(commands):
            row = json.loads(data[f"controller-logs/{index}.command.json"])
            controlled = row.get("controlled_env", {})
            expected_env = {
                "RUSTUP_TOOLCHAIN": RUST_PROFILE,
                "RUSTC": str(RUST_BIN / "rustc"),
                "CARGO_BUILD_JOBS": "1",
                "CARGO_TARGET_DIR": str(prior_task / "target"),
                "TMPDIR": str(prior_task / "tmp"),
                "YONIK_BUILD_ID": "0.1.0+" + record["source"]["commit"],
            }
            if (
                row["argv"] != expected
                or row["exit"] != 0
                or row["cwd"] != str(prior_task / "source")
                or any(
                    controlled.get(key) != value for key, value in expected_env.items()
                )
                or controlled.get("PATH", "").split(os.pathsep)[0] != str(RUST_BIN)
            ):
                raise NotReady("Нет успешных exact Rust commands/profile в prior job")
        for index, name in enumerate(("rustc", "cargo")):
            if toolchains[name] != data[f"controller-logs/{index}.stdout"].decode():
                raise NotReady("Rust reuse toolchain version mismatch")
        elf_records = {}
        for name in RUST:
            elf_records[name] = elf_header(data[name][:64], len(data[name]))
            with (artifacts / name).open("xb") as stream:
                stream.write(data[name])
        lineage = {
            "schema": 1,
            "build_performed": False,
            "prior_release_id": RUST_REUSE_PRIOR,
            "reviewed_evidence_commit": RUST_REUSE_REVIEW,
            "source": record["source"],
            "archive_sha256": record["archive_sha256"],
            "prior_artifacts": pins,
            "toolchains": toolchains,
            "elf": elf_records,
            "toolchain_binary_sha256_observed_at_reuse": rust_toolchain_hashes(),
        }
        write_record(artifacts / "rust-lineage.json", lineage)
        return lineage


def collected_reuse_pins(repo: Path) -> dict:
    rows = json.loads(
        git(
            repo,
            "show",
            COLLECTED_REUSE_REVIEW
            + ":docs/evidence/"
            + COLLECTED_REUSE_PRIOR
            + "/registry-inventory.json",
        )
    )
    prefix = (
        "/home/myrs/projects/yonik/local/release-controller/"
        + COLLECTED_REUSE_PRIOR
        + "/"
    )
    pins = {}
    for row in rows:
        if not row["path"].startswith(prefix):
            raise NotReady("Неверный fixed COLLECTED inventory")
        name = row["path"][len(prefix) :]
        if (
            name in pins
            or not name
            or name != PurePosixPath(name).as_posix()
            or ".." in PurePosixPath(name).parts
            or name.startswith("/")
        ):
            raise NotReady("Неверный COLLECTED inventory path")
        pins[name] = {"size": row["size"], "sha256": row["sha256"]}
    return pins


def reuse_collected(
    repo: Path, registry: Path, release: str, record: dict, artifacts: Path, root: Path
) -> dict:
    if release != COLLECTED_REUSE_TARGET:
        raise NotReady("COLLECTED reuse разрешён только fixed root-selected ID")
    pins = collected_reuse_pins(repo)
    if (
        not {"collected.json", "frozen.json", "source.tar", "package.tar", *RUST}
        <= pins.keys()
    ):
        raise NotReady("Неполный COLLECTED inventory")
    with release_lock(registry, COLLECTED_REUSE_PRIOR, writer=False) as prior:
        for name, pin in pins.items():
            read_pinned_partial(prior, name, pin)
        context = json.loads(
            read_pinned_partial(prior, "collected.json", pins["collected.json"])
        )
        if (
            context.get("state") != "COLLECTED"
            or context.get("final_linux_acceptance") is not False
            or context.get("release_id") != COLLECTED_REUSE_PRIOR
        ):
            raise NotReady("Нет prior NONFINAL COLLECTED")
        if any(
            context[key] != record[key]
            for key in ("source", "recipe", "archive_sha256")
        ):
            raise NotReady("COLLECTED reuse source/tree/archive/recipe mismatch")
        for name, checksum in context["tooling_sha256"].items():
            if (
                digest(git(repo, "show", COLLECTED_REUSE_REVIEW + ":tools/" + name))
                != checksum
            ):
                raise NotReady("Неверный prior COLLECTED tooling hash")
        for name, checksum in context["artifact_sha256"].items():
            if name not in pins or pins[name]["sha256"] != checksum:
                raise NotReady("Неверный prior COLLECTED artifact binding")
        validate_payload(prior / "package.tar", root, context)
        # Копируются только controller-owned artifacts, не prior status/commands.
        excluded = {
            "source.tar",
            "package.tar",
            "frozen.json",
            "collected.json",
            "rust-lineage.json",
        }
        for name in pins:
            if name in excluded or name.startswith("controller-logs/"):
                continue
            target = artifacts / name
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            with (
                input_path(prior, name).open("rb") as incoming,
                target.open("xb") as outgoing,
            ):
                shutil.copyfileobj(incoming, outgoing)
            target.chmod(0o600)
            if sha(target) != pins[name]["sha256"]:
                raise NotReady("COLLECTED copy SHA mismatch")
        for name in RUST:
            elf_file(artifacts / name)
        lineage = {
            "build_performed": False,
            "prior_release_id": COLLECTED_REUSE_PRIOR,
            "reviewed_evidence_commit": COLLECTED_REUSE_REVIEW,
            "source": record["source"],
            "prior_artifacts": pins,
        }
        write_record(artifacts / "collected-lineage.json", lineage)
        files = package_files(root, artifacts, context["cores"])
        payload = write_package(artifacts / "package.tar", files, pkginfo(root))
        result = {
            **context,
            **record,
            "state": "COLLECTED",
            "commands": [],
            "controlled_env": {},
            "payload": payload,
            "collected_lineage": lineage,
            "package_sha256": sha(artifacts / "package.tar"),
            "artifact_sha256": {
                p.relative_to(artifacts).as_posix(): sha(p)
                for p in artifacts.rglob("*")
                if p.is_file()
            },
            "final_linux_acceptance": False,
        }
        validate_payload(artifacts / "package.tar", root, result)
        write_record(artifacts / "collected.json", result)
        return result


def _collect(
    repo: Path, registry: Path, release: str, task: Path, runner: TrustedRunner
) -> dict:
    with release_lock(registry, release, writer=True) as artifacts:
        record = _read_request(registry, release)
        if (artifacts / "collected.json").exists():
            raise NotReady("Release уже collected; перезапись запрещена")
        if record["tooling_sha256"] != tooling_hashes():
            raise NotReady("Controller tooling изменился после freeze")
        final_profile = verify_final_profile(record)
        task.mkdir(mode=0o700)
        task_fd = os.open(task, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            fcntl.flock(task_fd, fcntl.LOCK_SH)
            root = task / "source"
            snapshot(repo, record, input_path(artifacts, "source.tar"), root)
            core_metadata(root)
            if release == COLLECTED_REUSE_TARGET:
                return reuse_collected(repo, registry, release, record, artifacts, root)
            env = build_env(task, record["source"]["commit"])
            toolchains = {
                name: runner.run([name, "--version"], root, env).decode()
                for name in ("rustc", "cargo")
            }
            try:
                capture_pending = False
                build_capture = None
                rust_lineage = None
                if release == RUST_REUSE_TARGET:
                    rust_lineage = reuse_rust(
                        repo, registry, release, record, artifacts, toolchains
                    )
                    elf_records = rust_lineage["elf"]
                else:
                    capture_pending = True
                    if final_profile:
                        from _p25_final_profile import prepare

                        for name, checksum in record["final_profile"][
                            "tooling_sha256"
                        ].items():
                            if sha(input_path(root / "tools", name)) != checksum:
                                raise NotReady("Snapshot capture tooling mismatch")
                        prepare(root, task / "target", env)
                    command_start = len(runner.commands)
                    for command in RECIPE["rust_commands"]:
                        runner.run([SLOT, *command], root, env)
                    elf_records = {}
                    for name in RUST:
                        binary = input_path(task / "target/release", name)
                        elf_records[name] = elf_file(binary)
                        shutil.copyfile(binary, artifacts / name)
                    build_capture = retain_build_capture(
                        root,
                        task / "target",
                        artifacts,
                        record,
                        runner.commands[command_start:],
                        list(RUST),
                        RECIPE["target"],
                        final_profile=final_profile,
                    )
                    capture_pending = False
                cores = {}
                for engine in VERSIONS:
                    build = task / engine
                    source = build / f"build-source-{engine}"
                    try:
                        accepted = _read_request(
                            registry, release, "trusted-core-job.json"
                        )
                    except FileNotFoundError:
                        accepted = None
                    if accepted is None:
                        runner.run(
                            [
                                sys.executable,
                                str(root / "tools/build_native_adapter.py"),
                                "--engine",
                                engine,
                                "--native-codec",
                                "--build-root",
                                str(build),
                                "--output-dir",
                                str(artifacts / engine),
                            ],
                            root,
                            env,
                        )
                    else:
                        if (
                            accepted["source_review_commit"] != JOB_SOURCE
                            or accepted["binary_sha256"] != JOB_BINARIES
                        ):
                            raise NotReady("Неверный controller-ingested native job")
                        output = artifacts / engine
                        output.mkdir(mode=0o700)
                        for name in (engine, "manifest.json", "prebuild.json"):
                            incoming = input_path(
                                artifacts / "accepted-cores" / engine, name
                            )
                            with (
                                incoming.open("rb") as stream,
                                (output / name).open("xb") as target,
                            ):
                                shutil.copyfileobj(stream, target)
                        manifest = json.loads((output / "manifest.json").read_text())
                        restore_job_source(root, source, engine, manifest, runner, env)
                    cores[engine] = check_core(
                        root, engine, artifacts / engine, source, runner, env
                    )
                    data = runner.run(
                        [
                            "git",
                            "archive",
                            "--format=tar",
                            cores[engine]["manifest"]["source_commit"],
                        ],
                        source,
                        env,
                    )
                    if (
                        digest(data)
                        != core_metadata(root)[engine]["lock"]["archive_sha256"]
                    ):
                        lock = core_metadata(root)[engine]["lock"]
                        url = lock.get("archive_url")
                        if not url:
                            raise NotReady("Upstream source archive hash mismatch")
                        runner.run(
                            [
                                "curl",
                                "--fail",
                                "--location",
                                "--output",
                                str(artifacts / engine / "upstream.tar"),
                                url,
                            ],
                            root,
                            env,
                        )
                    else:
                        (artifacts / engine / "upstream.tar").write_bytes(data)
                    if (
                        sha(artifacts / engine / "upstream.tar")
                        != core_metadata(root)[engine]["lock"]["archive_sha256"]
                    ):
                        raise NotReady("Pinned source archive hash mismatch")
                    archive_tree(source, artifacts / engine / "patched.tar")
                    logs = build / "build-native-adapter-logs"
                    if logs.exists():
                        shutil.copytree(logs, artifacts / engine / "builder-logs")
                (artifacts / "go-sources").mkdir(mode=0o700)
                for row in required_offers(root, cores):
                    target = artifacts / "go-sources" / (row["zip_sha256"] + ".zip")
                    if not target.exists():
                        runner.run(
                            [
                                "curl",
                                "--fail",
                                "--location",
                                "--output",
                                str(target),
                                row["zip_url"],
                            ],
                            root,
                            env,
                        )
                    if sha(target) != row["zip_sha256"]:
                        raise NotReady("Go source offer hash mismatch")
                files = package_files(root, artifacts, cores)
                payload = write_package(artifacts / "package.tar", files, pkginfo(root))
                context = {
                    **record,
                    "state": "COLLECTED",
                    "producer": "local-release-controller-v1",
                    "toolchains": toolchains,
                    "controlled_env": {key: env[key] for key in CONTROLLED_ENV},
                    "commands": runner.commands,
                    "elf": elf_records,
                    "cores": cores,
                    "payload": payload,
                    "trusted_native_job": accepted,
                    "rust_lineage": rust_lineage,
                    "build_capture": build_capture,
                    "artifact_sha256": {
                        path.relative_to(artifacts).as_posix(): sha(path)
                        for path in artifacts.rglob("*")
                        if path.is_file()
                    },
                    "package_sha256": sha(artifacts / "package.tar"),
                    "final_linux_acceptance": False,
                }
                validate_payload(artifacts / "package.tar", root, context)
            finally:
                if not capture_pending and (task / "target").exists():
                    runner.run(
                        [SLOT, "cargo", "clean", "--target-dir", str(task / "target")],
                        root,
                        env,
                    )
            write_record(artifacts / "collected.json", context)
            return context
        finally:
            os.close(task_fd)


def validate_payload(package: Path, frozen: Path, context: dict) -> None:
    from check_arch_package import expected_files, validate_archive

    if sha(package) != context["package_sha256"]:
        raise NotReady("Package отличается от actual controller build")
    for row in required_offers(frozen, context["cores"]):
        member = f"usr/share/yonik/sources/go/{row['zip_sha256']}.zip"
        if context["payload"].get(member) != {
            "mode": 0o644,
            "sha256": row["zip_sha256"],
        }:
            raise NotReady("Нет точного source offer в package binding")
    ordinary = expected_files(frozen)
    bound = {
        name: (row["mode"], row["sha256"])
        for name, row in context["payload"].items()
        if name not in ordinary and name != ".PKGINFO"
    }
    validate_archive(package, frozen, bound_files=bound)
    with tarfile.open(package) as archive:
        for name, row in context["payload"].items():
            entry = archive.getmember(name)
            stream = archive.extractfile(entry)
            if (
                stream is None
                or hashlib.file_digest(stream, "sha256").hexdigest() != row["sha256"]
            ):
                raise NotReady("Actual package payload отличается от build binding")
            if name in RUST.values() or name in RECIPE["core_members"].values():
                stream = archive.extractfile(entry)
                assert stream is not None
                elf_header(stream.read(64), entry.size)


def _verify(
    repo: Path, registry: Path, release: str, package: Path, inputs: Path
) -> dict:
    try:
        with release_lock(registry, release, writer=False) as artifacts:
            context = read_context(registry, release)
            if (
                context.get("state") != "COLLECTED"
                or context.get("producer") != "local-release-controller-v1"
            ):
                raise NotReady("Нет controller-collected build")
            if (
                context.get("recipe") != RECIPE
                or context.get("tooling_sha256") != tooling_hashes()
            ):
                raise NotReady("Неверная controller recipe")
            verify_final_profile(context)
            candidate_source = input_path(inputs, "source.tar")
            if sha(candidate_source) != context["archive_sha256"]:
                raise NotReady("Candidate inputs не соответствуют frozen source")
            with tempfile.TemporaryDirectory(prefix="yonik-verify-") as temporary:
                frozen = Path(temporary)
                snapshot(repo, context, input_path(artifacts, "source.tar"), frozen)
                core_metadata(frozen)
                validate_payload(
                    input_path(package.parent, package.name), frozen, context
                )
            return {
                "status": "PASS",
                "scope": "local-TCB-provenance-preflight",
                "release_id": release,
                "source": context["source"],
                "final_linux_acceptance": False,
            }
    except (OSError, KeyError, TypeError, ValueError, tarfile.TarError) as error:
        raise NotReady(f"Неполный trusted release context: {error}") from error


def collect_release(release: str) -> dict:
    """Production runner не принимает commands, manifests или outputs от caller."""
    repo = canonical_repo()
    registry = repo / "local/release-controller"
    release_name(release)
    artifacts = registry / release
    task = TRANSIENT / f"release-controller-{release}"
    with release_lock(registry, release, writer=True):
        runner = LocalRunner(artifacts / "controller-logs")
    preexisting = task.exists()
    try:
        result = _collect(repo, registry, release, task, runner)
    except Exception as error:
        if not preexisting and task.exists():
            size = sum(path.lstat().st_size for path in task.rglob("*"))
            failure = {
                "status": "BLOCKED",
                "final_linux_acceptance": False,
                "left_task": str(task),
                "bytes": size,
                "reason": str(error),
            }
            write_record(artifacts / "failure.json", failure)
            raise NotReady(f"{error}; сохранён task {task}, {size} bytes") from error
        raise
    # Только успешный собственный запуск: не трогаем pre-existing task при ошибке.
    descriptor = os.open(task, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_SH)
        shutil.rmtree(task)
    finally:
        os.close(descriptor)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("freeze", "collect", "ingest-cores"))
    parser.add_argument("--release-id", required=True)
    parser.add_argument("--commit")
    parser.add_argument("--final-profile", choices=("p25-final-cxx-v1",))
    args = parser.parse_args()
    try:
        if args.operation == "freeze":
            if not args.commit:
                parser.error("freeze требует root-selected --commit")
            result = freeze_release(args.release_id, args.commit, args.final_profile)
        else:
            if args.commit or args.final_profile:
                parser.error("collect/ingest-cores не принимают source/records")
            if args.operation == "ingest-cores":
                ingest_core_job(args.release_id)
                result = {"state": "CORES_INGESTED"}
            else:
                result = collect_release(args.release_id)
    except (ValueError, OSError, subprocess.SubprocessError, tarfile.TarError) as error:
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
    sys.stdout.write(
        json.dumps({"state": result["state"], "final_linux_acceptance": False}) + "\n"
    )


if __name__ == "__main__":
    main()
