"""Фиксированный opt-in профиль build-time CXX metadata; без shell/discovery."""

from __future__ import annotations

import fcntl
import hashlib
import os
import re
import selectors
import signal
import stat
import subprocess
import time
from contextlib import ExitStack, contextmanager
from pathlib import Path
from typing import Callable, Iterator

from _p25_capture_producer import _directory, _write
from p25_capture_build_closure import (
    MAX_FILE,
    MAX_FILES,
    MAX_METADATA,
    Reader,
    object_fields,
    parse,
    relative,
    require,
)

PROFILE = {
    "id": "p25-final-cxx-v1",
    "compiler": "/usr/bin/g++",
    "qt_query": ["/usr/bin/qtpaths6", "--qt-version"],
    "sdk_root": "/usr",
    "registry_source_root": "/home/myrs/.cargo/registry/src",
    "dependency_flags": ["-MD", "-MF"],
    "deadline_seconds": 300,
    "stream_cap": MAX_METADATA,
    "observed_metadata_cap": 8 * MAX_METADATA,
    "dependency_row_cap": 10000,
}
Runner = Callable[[int, list[str], Path, dict[str, str]], tuple[int, bytes, bytes]]


def dependencies(raw: bytes) -> list[str]:
    """Одна Make rule: escapes, continuation и $$; никогда Make execution."""
    require(len(raw) <= MAX_METADATA, "Depfile превышает cap")
    text = raw.decode("utf-8", errors="strict")
    words: list[str] = []
    token = ""
    colon = False
    i = 0
    while i < len(text):
        char = text[i]
        if char == "\\":
            i += 1
            require(i < len(text), "Оборванный Make escape")
            if text[i] == "\n":
                if token:
                    words.append(token)
                    token = ""
            else:
                require(text[i] in " \\#:$\t", "Неизвестный Make escape")
                token += text[i]
        elif char == "$":
            require(text[i : i + 2] == "$$", "Make expansion запрещён")
            token += "$"
            i += 1
        elif char == ":":
            require(not colon and bool(token or words), "Несколько/пустой Make rule")
            colon = True
            words = []
            token = ""
        elif char in " \t\r\n":
            if token:
                words.append(token)
                token = ""
            if char == "\n":
                require(not text[i:].strip(), "Несколько Make rules")
        else:
            require(ord(char) >= 32 and char != "#", "Неэкранированный Make token")
            token += char
        require(len(words) <= MAX_FILES, "Слишком много dependencies")
        i += 1
    if token:
        words.append(token)
    require(colon and bool(words) and len(words) <= MAX_FILES, "Пустой depfile")
    return list(dict.fromkeys(words))


def compile_arguments(argv: list[str], depfile: Path) -> list[str]:
    require(0 < len(argv) <= MAX_FILES, "Неверный CXX argv count")
    require(
        all(
            isinstance(arg, str)
            and 0 < len(arg) <= 4096
            and "\x00" not in arg
            and not arg.startswith("@")
            for arg in argv
        ),
        "Response/неверный argv",
    )
    require(sum(map(len, argv)) <= MAX_METADATA, "CXX argv cap")
    require(
        not any(
            arg.startswith(("-MD", "-MMD", "-MF", "-MT", "-MQ", "-MP"))
            or arg in ("-M", "-MM")
            or arg.startswith("-Wp,-M")
            for arg in argv
        ),
        "Конфликт dependency flags",
    )
    return [*argv, "-MD", "-MF", str(depfile)] if "-c" in argv else list(argv)


@contextmanager
def trusted_tool(alias: str) -> Iterator[tuple[int, dict]]:
    require(alias in (PROFILE["compiler"], PROFILE["qt_query"][0]), "Не fixed tool")
    path = Path(alias).resolve(strict=True)
    require(path.is_relative_to(Path(PROFILE["sdk_root"])), "Tool вне trusted SDK root")
    parent = _directory(path.parent)
    try:
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=parent)
    finally:
        os.close(parent)
    try:
        before = os.fstat(fd)
        require(
            stat.S_ISREG(before.st_mode) and before.st_size <= MAX_FILE,
            "Неверный compiler/query file",
        )
        digest = hashlib.sha256()
        count = 0
        while chunk := os.read(fd, 65536):
            count += len(chunk)
            require(count <= MAX_FILE, "Tool bytes cap")
            digest.update(chunk)
        after = os.fstat(fd)
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
            "Tool изменился",
        )
        checksum = digest.hexdigest()
        os.lseek(fd, 0, os.SEEK_SET)
        yield fd, {"path": str(path), "sha256": checksum}
    finally:
        os.close(fd)


def execute(
    fd: int, argv: list[str], cwd: Path, env: dict[str, str]
) -> tuple[int, bytes, bytes]:
    """Pipe drain с deadline/caps; всегда завершает собственную process group."""
    outputs = [bytearray(), bytearray()]
    with subprocess.Popen(
        argv,
        executable=f"/proc/self/fd/{fd}",
        pass_fds=(fd,),
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    ) as process:
        try:
            deadline = time.monotonic() + PROFILE["deadline_seconds"]
            with selectors.DefaultSelector() as selector:
                for index, stream in enumerate((process.stdout, process.stderr)):
                    require(stream is not None, "Нет compiler pipe")
                    os.set_blocking(stream.fileno(), False)
                    selector.register(stream, selectors.EVENT_READ, index)
                while selector.get_map():
                    require(time.monotonic() < deadline, "Compiler/query deadline")
                    for key, _ in selector.select(
                        min(0.1, max(0, deadline - time.monotonic()))
                    ):
                        chunk = os.read(key.fileobj.fileno(), 65536)
                        if not chunk:
                            selector.unregister(key.fileobj)
                            continue
                        output = outputs[key.data]
                        require(
                            len(output) + len(chunk) <= PROFILE["stream_cap"],
                            "Compiler/query log cap",
                        )
                        output.extend(chunk)
            code = process.wait(timeout=max(0.01, deadline - time.monotonic()))
            return code, bytes(outputs[0]), bytes(outputs[1])
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()


def _reserve(build: Path) -> int:
    directory = _directory(build / "p25capture")
    try:
        fd = os.open(
            "count",
            os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK,
            0o600,
            dir_fd=directory,
        )
    finally:
        os.close(directory)
    try:
        info = os.fstat(fd)
        require(
            stat.S_ISREG(info.st_mode)
            and info.st_uid == os.getuid()
            and info.st_nlink == 1
            and info.st_size <= 16,
            "Неверный counter",
        )
        deadline = time.monotonic() + 5
        while True:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                require(time.monotonic() < deadline, "Counter lock deadline")
                time.sleep(0.01)
        raw = os.read(fd, 16)
        count = int(raw or b"0")
        require(0 <= count < MAX_FILES, "Invocation cap")
        os.pwrite(fd, str(count + 1).encode(), 0)
        os.fsync(fd)
        return count
    finally:
        os.close(fd)


def compiler_capture(
    config: dict,
    argv: list[str],
    cwd: Path,
    env: dict[str, str],
    runner: Runner = execute,
) -> tuple[int, bytes, bytes]:
    build = Path(config["build"])
    number = _reserve(build)
    dep = build / "p25capture" / f"{number}.d"
    actual = compile_arguments(argv, dep)
    require(
        cwd.is_relative_to(Path(config["source"])) or cwd.is_relative_to(build),
        "CXX cwd вне frozen roots",
    )
    with trusted_tool(PROFILE["compiler"]) as (fd, compiler):
        command = [PROFILE["compiler"], *actual]
        code, out, err = runner(fd, command, cwd, env)
    row = {
        "schema": 1,
        "config_sha256": env.get("YONIK_P25_CONFIG_SHA256"),
        "argv": command,
        "original_argv": argv,
        "cwd": str(cwd),
        "compiler": compiler,
        "exit": code,
        "stdout_sha256": hashlib.sha256(out).hexdigest(),
        "stderr_sha256": hashlib.sha256(err).hexdigest(),
        "depfile": None,
        "unit": None,
    }
    if "-c" in argv and code == 0:
        units = [
            arg for arg in argv if arg.endswith((".cpp", ".cxx", ".cc", ".C", ".c++"))
        ]
        require(len(units) == 1, "Неоднозначный CXX unit")
        row["unit"] = (
            str(cwd / units[0]) if not Path(units[0]).is_absolute() else units[0]
        )
        row["depfile"] = f"p25capture/{number}.d"
    require(
        len(out) <= PROFILE["stream_cap"] and len(err) <= PROFILE["stream_cap"],
        "Compiler log cap",
    )
    _write(build / "p25capture" / f"{number}.json", row)
    return code, out, err


def prepare(
    source: Path, build: Path, env: dict[str, str], runner: Runner = execute
) -> None:
    """Только внутри будущей build phase. Один observed query, не host audit."""
    build.mkdir(mode=0o700, exist_ok=True)
    (build / "p25capture").mkdir(mode=0o700)
    config = {
        "schema": 1,
        "source": str(source),
        "build": str(build),
        "sdk": PROFILE["sdk_root"],
    }
    pin = _write(build / "p25capture/config.json", config)
    env["YONIK_P25_CONFIG"] = str(build / "p25capture/config.json")
    env["YONIK_P25_CONFIG_SHA256"] = pin
    env["CXX"] = str(source / "tools/p25_cxx_capture_wrapper.py")
    env["CXX_x86_64_unknown_linux_gnu"] = env["CXX"]
    env["CXX_x86_64-unknown-linux-gnu"] = env["CXX"]
    env["TARGET_CXX"] = env["CXX"]
    with trusted_tool(PROFILE["qt_query"][0]) as (fd, tool):
        code, out, err = runner(fd, list(PROFILE["qt_query"]), source, env)
    require(
        len(out) <= PROFILE["stream_cap"] and len(err) <= PROFILE["stream_cap"],
        "Qt query log cap",
    )
    version = out.decode("ascii").strip()
    require(
        code == 0 and re.fullmatch(r"\d+\.\d+\.\d+", version) is not None,
        "Неуспешный fixed Qt query",
    )
    _write(
        build / "p25capture/qt-observed.json",
        {
            "schema": 1,
            "query_argv": PROFILE["qt_query"],
            "version": version,
            "exit": code,
            "tool": tool,
            "stdout": out.decode("ascii"),
            "stderr_sha256": hashlib.sha256(err).hexdigest(),
        },
    )


def wrapper_config(env: dict[str, str]) -> dict:
    path = Path(env["YONIK_P25_CONFIG"])
    parent = _directory(path.parent)
    try:
        fd = os.open(
            path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent
        )
    finally:
        os.close(parent)
    with os.fdopen(fd, "rb") as stream:
        require(stat.S_ISREG(os.fstat(stream.fileno()).st_mode), "Config не regular")
        raw = stream.read(MAX_METADATA + 1)
    require(
        len(raw) <= MAX_METADATA
        and hashlib.sha256(raw).hexdigest() == env["YONIK_P25_CONFIG_SHA256"],
        "Config pin mismatch/cap",
    )
    config = parse(raw)
    require(
        set(config) == {"schema", "source", "build", "sdk"} and config["schema"] == 1,
        "Неверный config",
    )
    require(
        config["sdk"] == PROFILE["sdk_root"]
        and path == Path(config["build"]) / "p25capture/config.json",
        "Config root mismatch",
    )
    with ExitStack() as stack:
        Reader({key: Path(config[key]) for key in ("source", "build", "sdk")}, stack)
    return config


def enrich(roots: dict[str, Path], record: dict, retained: Path) -> dict:
    """Закрытый join собственных wrapper records; inventory SDK не сканируется."""
    from p25_capture_build_closure import MAX_GENERATED, MAX_TEXT

    observed: list[dict] = []
    missing: list[str] = []
    rows = record["files"]
    known = {(row["root"], row["path"]): row for row in rows}
    origins: dict[str, str] = {}
    compiled_units: set[str] = set()
    with ExitStack() as stack:
        reader = Reader(roots, stack)
        external_readers: dict[str, Reader] = {}

        def stage(absolute: Path, kind: str) -> str:
            origin = str(absolute)
            if origin in origins:
                return origins[origin]
            allowed = [Path(PROFILE["sdk_root"]), Path(PROFILE["registry_source_root"])]
            matches = [root for root in allowed if absolute.is_relative_to(root)]
            require(len(matches) == 1, f"Dependency вне fixed input roots: {absolute}")
            base = matches[0]
            key = str(base)
            if key not in external_readers:
                external_readers[key] = Reader(
                    {"source": roots["source"], "build": roots["build"], "sdk": base},
                    stack,
                )
            external = external_readers[key]
            limit = MAX_FILE if kind == "tool" else MAX_TEXT
            local = relative(absolute.relative_to(base).as_posix())
            # Reader проверяет ancestors/type/stat/caps до сохранения точных bytes.
            _, expected = external._consume("sdk", local, limit, False)
            require(
                sum(item.total for item in external_readers.values())
                <= 1024 * 1024 * 1024,
                "External input total cap",
            )
            name = hashlib.sha256(origin.encode()).hexdigest()
            parent = _directory(absolute.parent)
            try:
                incoming = os.open(
                    absolute.name,
                    os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                    dir_fd=parent,
                )
            finally:
                os.close(parent)
            with os.fdopen(incoming, "rb") as stream:
                before = os.fstat(stream.fileno())
                require(
                    stat.S_ISREG(before.st_mode) and before.st_size <= limit,
                    "Staging type/cap",
                )
                directory = _directory(roots["sdk"])
                try:
                    fd = os.open(
                        name,
                        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                        0o600,
                        dir_fd=directory,
                    )
                    with os.fdopen(fd, "wb") as output:
                        checksum = hashlib.sha256()
                        size = 0
                        while chunk := stream.read(65536):
                            size += len(chunk)
                            require(size <= limit, "Staging byte cap")
                            checksum.update(chunk)
                            output.write(chunk)
                        output.flush()
                        os.fsync(output.fileno())
                    os.fsync(directory)
                finally:
                    os.close(directory)
                after = os.fstat(stream.fileno())
                require(
                    checksum.hexdigest() == expected
                    and size == before.st_size
                    and (
                        before.st_ino,
                        before.st_size,
                        before.st_mtime_ns,
                        before.st_ctime_ns,
                    )
                    == (
                        after.st_ino,
                        after.st_size,
                        after.st_mtime_ns,
                        after.st_ctime_ns,
                    ),
                    "Staging changed input",
                )
            origins[origin] = name
            return name

        def add(
            path: str, cwd: Path, kind: str | None = None, expected: str | None = None
        ) -> str:
            original = Path(path) if Path(path).is_absolute() else cwd / path
            absolute = Path(os.path.normpath(original))
            if ".." in original.parts:
                # Проверяем пропускаемые normalization ancestors до hash, чтобы
                # symlink/../ не подменил фактически прочитанный compiler input.
                require(
                    not set(original.parts)
                    & {".ssh", ".gnupg", ".config", ".aws", ".env"},
                    "Private dependency path",
                )
                directory = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
                try:
                    for part in original.parts[1:-1]:
                        nested = os.open(
                            part,
                            os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=directory,
                        )
                        os.close(directory)
                        directory = nested
                    canonical = _directory(absolute.parent)
                    try:
                        actual_stat, canonical_stat = (
                            os.fstat(directory),
                            os.fstat(canonical),
                        )
                        require(
                            (actual_stat.st_dev, actual_stat.st_ino)
                            == (canonical_stat.st_dev, canonical_stat.st_ino),
                            "Ambiguous dependency parent",
                        )
                    finally:
                        os.close(canonical)
                finally:
                    os.close(directory)
            matches = [
                (name, root)
                for name, root in roots.items()
                if absolute.is_relative_to(root)
            ]
            origin = str(absolute)
            if not matches:
                inferred_kind = kind or (
                    "sdk-unit" if origin in compiled_units else "header"
                )
                staged = stage(absolute, inferred_kind)
                absolute = roots["sdk"] / staged
                matches = [("sdk", roots["sdk"])]
                kind = inferred_kind
            require(len(matches) == 1, f"Input вне roots: {absolute}")
            root, base = matches[0]
            local = relative(absolute.relative_to(base).as_posix())
            key = (root, local)
            if key in known:
                row = known[key]
                if origin in compiled_units and row["kind"] == "header":
                    row["kind"] = "sdk-unit"
                require(
                    expected is None or row["sha256"] == expected,
                    "Изменён observed input",
                )
                return row["id"]
            actual_kind = (
                kind
                or {"source": "source", "build": "generated", "sdk": "header"}[root]
            )
            limit = (
                MAX_FILE
                if actual_kind == "tool"
                else MAX_GENERATED
                if actual_kind == "generated"
                else MAX_TEXT
            )
            _, checksum = reader._consume(root, local, limit, False)
            require(
                expected is None or checksum == expected, "Observed tool hash mismatch"
            )
            require(len(rows) < MAX_FILES, "File whitelist cap")
            row = {
                "id": f"input-{len(rows)}",
                "root": root,
                "path": local,
                "kind": actual_kind,
                "sha256": checksum,
            }
            rows.append(row)
            known[key] = row
            return row["id"]

        try:
            count = int(reader.read("build", "p25capture/count", 16))
        except FileNotFoundError:
            count = 0
        require(0 <= count <= MAX_FILES, "Wrapper invocation count cap")
        config_raw = reader.read("build", "p25capture/config.json", MAX_METADATA)
        config_pin = hashlib.sha256(config_raw).hexdigest()
        metadata_bytes = len(config_raw)
        dependency_rows = 0
        for number in range(count):
            raw = reader.read("build", f"p25capture/{number}.json", MAX_METADATA)
            metadata_bytes += len(raw)
            require(
                metadata_bytes <= PROFILE["observed_metadata_cap"],
                "Observed metadata total cap",
            )
            row = parse(raw)
            fields = {
                "schema",
                "config_sha256",
                "argv",
                "original_argv",
                "cwd",
                "compiler",
                "exit",
                "depfile",
                "unit",
                "stdout_sha256",
                "stderr_sha256",
            }
            object_fields(row, fields, fields)
            require(row["config_sha256"] == config_pin, "Observed config pin mismatch")
            require(
                type(row["schema"]) is int
                and row["schema"] == 1
                and type(row["exit"]) is int
                and row["exit"] == 0,
                "Неуспешный CXX record",
            )
            object_fields(row["compiler"], {"path", "sha256"}, {"path", "sha256"})
            require(
                row["argv"]
                == [
                    PROFILE["compiler"],
                    *compile_arguments(
                        row["original_argv"],
                        roots["build"] / "p25capture" / f"{number}.d",
                    ),
                ],
                "Observed argv binding mismatch",
            )
            if "-c" not in row["original_argv"]:
                require(
                    row["unit"] is None and row["depfile"] is None,
                    "Probe не compile record",
                )
            else:
                units = [
                    arg
                    for arg in row["original_argv"]
                    if arg.endswith((".cpp", ".cxx", ".cc", ".C", ".c++"))
                ]
                require(
                    len(units) == 1 and row["unit"] == str(Path(row["cwd"]) / units[0]),
                    "Observed unit binding mismatch",
                )
            compiler_id = add(
                row["compiler"]["path"], roots["sdk"], "tool", row["compiler"]["sha256"]
            )
            observed.append({**row, "compiler_id": compiler_id})
            if row["unit"] is None:
                continue
            require(
                row["depfile"] == f"p25capture/{number}.d", "Depfile binding mismatch"
            )
            cwd = Path(row["cwd"])
            compiled_units.add(os.path.normpath(row["unit"]))
            unit = add(row["unit"], cwd)
            raw_dep = reader.read("build", row["depfile"], MAX_METADATA)
            dep_paths = dependencies(raw_dep)
            dependency_rows += len(dep_paths)
            require(
                dependency_rows <= PROFILE["dependency_row_cap"], "Dependency rows cap"
            )
            deps = [add(path, cwd) for path in dep_paths]
            require(unit in deps, "Depfile не содержит actual unit")
            dep_path = f"p25capture/{number}-normalized.json"
            normalized = {
                "schema": 1,
                "unit": unit,
                "dependencies": list(dict.fromkeys(deps)),
            }
            dep_hash = _write(roots["build"] / dep_path, normalized)
            dep_id = add(str(roots["build"] / dep_path), cwd, "depfile", dep_hash)
            record["units"].append(
                {"file": unit, "depfile": dep_id, "argv": row["argv"]}
            )
            observed[-1]["raw_depfile_sha256"] = hashlib.sha256(raw_dep).hexdigest()
            observed[-1]["normalized_depfile"] = normalized
        qt = parse(reader.read("build", "p25capture/qt-observed.json", MAX_METADATA))
        require(
            qt["query_argv"] == PROFILE["qt_query"]
            and qt["exit"] == 0
            and re.fullmatch(r"\d+\.\d+\.\d+", qt["version"]) is not None,
            "Неверный observed fixed Qt query",
        )
        query_tool = add(qt["tool"]["path"], roots["sdk"], "tool", qt["tool"]["sha256"])
        qt_normalized = {
            "schema": 1,
            "version": qt["version"],
            "sdk_files": [row["id"] for row in rows if row["root"] == "sdk"],
            "query_argv": qt["query_argv"],
            "query_tool": query_tool,
            "exit": 0,
        }
        qt_path = roots["build"] / "p25capture/qt-normalized.json"
        qt_hash = _write(qt_path, qt_normalized)
        record["qt_version"] = add(str(qt_path), roots["build"], "qt-version", qt_hash)
        record["source"]["tracked_inputs"] = [
            row["id"] for row in rows if row["root"] == "source"
        ]
    missing.append("qt:query-to-header-version-join")
    if not record["units"]:
        missing.append("cxx:invocations")
    for row in rows:
        if row["kind"] == "generated":
            missing.append(f"generation-provenance:{row['id']}")
        if row["kind"] in ("header", "sdk-unit"):
            missing.append(f"source-offer-primary-join:{row['id']}")
    observed_hash = _write(
        retained / "build-cxx-observed.json",
        {
            "schema": 1,
            "records": observed,
            "qt": qt,
            "input_origins": origins,
            "config": parse(config_raw),
            "config_sha256": config_pin,
        },
        PROFILE["observed_metadata_cap"],
    )
    return {
        "producer_missing": missing,
        "compiled_units": len(record["units"]),
        "observed_invocations": len(observed),
        "qt_version": qt["version"],
        "observed_sha256": observed_hash,
    }
