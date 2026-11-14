"""Будущий root producer; импорт не читает хост и не запускает процессы."""

from __future__ import annotations

import hashlib
import json
import os
import re
import select
import stat
import subprocess
import time
from typing import Any

import collect_observations as c

CAPTURE = c.ROOT + "/unit-closure.json"
UNITS = (
    "yonik-helper.service",
    "yonik-worker.service",
    "yonik-worker-check.service",
    "yonik-worker.socket",
    "yonik-worker-check.socket",
)
COMMON = "Id,LoadState,FragmentPath,DropInPaths,SourcePath,Transient,NeedDaemonReload,ActiveState,SubState,InvocationID".split(
    ","
)
SERVICE = "ExecStart,ExecStartPre,ExecStartPost,ExecCondition,ExecReload,ExecStop,ExecStopPost,User,Group,Type,BusName,ControlGroup,MainPID,ExecMainStartTimestampMonotonic,Environment,EnvironmentFiles,RootDirectory,RootImage,WorkingDirectory,DynamicUser,PAMName,NoNewPrivileges,CapabilityBoundingSet,AmbientCapabilities,KillMode,PrivateNetwork,PrivateDevices,ProtectSystem,ProtectHome".split(
    ","
)
SOCKET = "Listen,SocketUser,SocketGroup,SocketMode,DirectoryMode,Accept,RemoveOnStop,Triggers".split(
    ","
)
for _unit in UNITS:
    c.COMMANDS["closure:" + _unit] = (
        "/usr/bin/systemctl",
        "--system",
        "--no-pager",
        "--all",
        "show",
        _unit,
        "--property",
        ",".join(COMMON + (SERVICE if _unit.endswith(".service") else SOCKET)),
    )


def _snapshot(backend: c.SystemBackend) -> dict[str, dict[str, str]]:
    result = {}
    for name in UNITS:
        raw = backend.command("closure:" + name)
        c.require(len(raw) <= 65536 and raw.endswith(b"\n"))
        props: dict[str, str] = {}
        for line in raw.decode().splitlines():
            key, value = line.split("=", 1)
            c.require(key not in props)
            props[key] = value
        c.require(
            set(props)
            == set(COMMON + (SERVICE if name.endswith(".service") else SOCKET))
        )
        c.require(props["Id"] == name and props["LoadState"] == "loaded")
        c.require(props["FragmentPath"] == f"/usr/lib/systemd/system/{name}")
        c.require(props["DropInPaths"] == props["SourcePath"] == "")
        c.require(props["Transient"] == props["NeedDaemonReload"] == "no")
        result[name] = props
    return result


def _identity(directory: int, pid: int) -> tuple[int, tuple[int, ...]]:
    def read(name: str) -> bytes:
        fd = os.open(
            name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory
        )
        try:
            raw = os.read(fd, 65537)
            c.require(len(raw) <= 65536)
            return raw
        finally:
            os.close(fd)

    raw = read("stat")
    c.require(raw.startswith(f"{pid} (".encode()) and raw.rfind(b")") > 0)
    start = int(raw[raw.rfind(b")") + 2 :].split()[19])
    rows = [
        line.split()[1:]
        for line in read("status").splitlines()
        if line.startswith(b"Uid:")
    ]
    c.require(len(rows) == 1 and len(rows[0]) == 4 and start > 0)
    uids = tuple(int(value) for value in rows[0])
    c.require(uids == (0, 0, 0, 0))
    return start, uids


def _stamp(info: os.stat_result) -> tuple[int, ...]:
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


class ClosureBackend(c.SystemBackend):
    """pidfd удерживается вокруг чтения именно executing image, не installed ELF."""

    def helper_image(self, pid: int) -> dict[str, Any]:
        c.require(type(pid) is int and 1 < pid < 2**31)
        pidfd = os.pidfd_open(pid, 0)
        try:
            poller = select.poll()
            poller.register(pidfd, select.POLLIN)
            c.require(not poller.poll(0))
            root = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
            try:
                proc = os.open(
                    "proc", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root
                )
            finally:
                os.close(root)
            try:
                c.require(
                    os.fstat(proc).st_uid == 0 and not os.fstat(proc).st_mode & 0o022
                )
                directory = os.open(
                    str(pid), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=proc
                )
            finally:
                os.close(proc)
            try:
                c.require(os.fstat(directory).st_uid == 0)
                before = _identity(directory, pid)
                # Единственное намеренное magic-link исключение: literal exe в
                # удерживаемом trusted procdir после root identity + live pidfd.
                fd = os.open("exe", os.O_RDONLY | os.O_NONBLOCK, dir_fd=directory)
                try:
                    initial = os.fstat(fd)
                    c.require(
                        stat.S_ISREG(initial.st_mode)
                        and 0 < initial.st_size <= 64 * c.LIMIT
                    )
                    c.require(initial.st_uid == 0 and not initial.st_mode & 0o022)
                    digest = hashlib.sha256()
                    size = 0
                    end = time.monotonic() + c.DEADLINE
                    while True:
                        c.require(time.monotonic() < end)
                        chunk = os.read(fd, min(65536, 64 * c.LIMIT + 1 - size))
                        if not chunk:
                            break
                        size += len(chunk)
                        c.require(size <= 64 * c.LIMIT)
                        digest.update(chunk)
                    c.require(
                        size == initial.st_size
                        and _stamp(initial) == _stamp(os.fstat(fd))
                    )
                    # Exec того же task после open тоже не должен пройти.
                    check = os.open(
                        "exe", os.O_RDONLY | os.O_NONBLOCK, dir_fd=directory
                    )
                    try:
                        c.require(_stamp(os.fstat(check)) == _stamp(initial))
                    finally:
                        os.close(check)
                    c.require(
                        _identity(directory, pid) == before and not poller.poll(0)
                    )
                    return {
                        "pid": pid,
                        "start_ticks": before[0],
                        "uid": 0,
                        "exe_sha256": digest.hexdigest(),
                        "exe_size": size,
                    }
                finally:
                    os.close(fd)
            finally:
                os.close(directory)
        finally:
            os.close(pidfd)


def collect_unit_closure(
    nonce: str, case: str, backend: ClosureBackend | None = None
) -> dict[str, Any]:
    """Возвращает exact consumer schema; запись sealed файла принадлежит outer fixture."""
    backend = backend if backend is not None else ClosureBackend()
    try:
        auth, inputs_raw, inputs = c._authorize(backend, nonce, case)
        client = backend.read("/usr/bin/systemctl", trusted=True, cap=64 * c.LIMIT)
        collector = backend.read(
            "/usr/lib/yonik-acceptance/collect_observations.py", sealed=True
        )
        boot = backend.read("/proc/sys/kernel/random/boot_id").decode().strip()
        c.require(
            re.fullmatch(r"[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}", boot) is not None
        )
        units = _snapshot(backend)
        service = units[UNITS[0]]
        c.require(
            service["ActiveState"] == "active" and service["SubState"] == "running"
        )
        c.require(service["User"] == service["Group"] == "root")
        pid = int(service["MainPID"])
        invocation = service["InvocationID"]
        started = int(service["ExecMainStartTimestampMonotonic"])
        c.require(
            re.fullmatch(r"[0-9a-f]{32}", invocation) is not None
            and 0 < started < 2**64
        )
        helper = backend.helper_image(pid)
        helper.update(invocation=invocation, exec_main_start_monotonic_usec=started)
        c.require(_snapshot(backend) == units)
        c.require(
            backend.read("/usr/bin/systemctl", trusted=True, cap=64 * c.LIMIT) == client
        )
        c.require(
            backend.read(
                "/usr/lib/yonik-acceptance/collect_observations.py", sealed=True
            )
            == collector
        )
        c.require(
            backend.read("/proc/sys/kernel/random/boot_id").decode().strip() == boot
        )
        c.require(backend.read(c.ROOT + "/authorization.json", sealed=True) == auth)
        c.require(backend.read(c.ROOT + "/input.json", sealed=True) == inputs_raw)
        result = {
            "schema": "N5_UNIT_CLOSURE_V1",
            "provenance": {
                **{key: inputs[key] for key in ("source", "manifest_sha256")},
                "nonce": nonce,
                "case": case,
                "kernel_boot_id": boot,
                "collector_sha256": hashlib.sha256(collector).hexdigest(),
                "authorization_sha256": hashlib.sha256(auth).hexdigest(),
                "manager_client_sha256": hashlib.sha256(client).hexdigest(),
            },
            "units": units,
            "helper": helper,
        }
        c.require(len(json.dumps(result).encode()) <= c.LIMIT)
        return result
    except (
        OSError,
        ValueError,
        TypeError,
        KeyError,
        IndexError,
        RecursionError,
        c.Unavailable,
        subprocess.SubprocessError,
    ):
        return {"schema": "N5_UNIT_CLOSURE_V1", "status": "UNAVAILABLE"}
