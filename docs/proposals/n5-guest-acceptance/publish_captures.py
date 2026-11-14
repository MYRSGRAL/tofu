"""Закрытый source publisher: данные наблюдений, не acceptance и не grant."""

from __future__ import annotations

import hashlib
import json
import os
import re
import secrets
import stat
from typing import Any

import collect_observations as c
import unit_closure_capture as u


class PublicationError(Exception):
    """Публикация не подтверждена; существующий capture не заменяется."""


class PublicationUnknown(PublicationError):
    """Имя уже связано с inode; durability неизвестна, файл сохраняется."""


def _hash(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def _parent() -> int:
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in ("", *c.ROOT.strip("/").split("/")):
            if part:
                child = os.open(
                    part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd
                )
                os.close(fd)
                fd = child
            info = os.fstat(fd)
            c.require(stat.S_ISDIR(info.st_mode))
            c.require(info.st_uid == 0 and not info.st_mode & 0o022)
        return fd
    except BaseException:
        os.close(fd)
        raise


def _identity(fd: int) -> None:
    other = _parent()
    try:
        a, b = os.fstat(fd), os.fstat(other)
        c.require((a.st_dev, a.st_ino) == (b.st_dev, b.st_ino))
    finally:
        os.close(other)


def _absent(fd: int, name: str) -> None:
    try:
        os.stat(name, dir_fd=fd, follow_symlinks=False)
    except FileNotFoundError:
        return
    raise PublicationError("Capture уже существует; повторное использование запрещено")


def _closure(data: dict[str, Any]) -> None:
    c.require(set(data) == {"schema", "provenance", "units", "helper"})
    c.require(data["schema"] == "N5_UNIT_CLOSURE_V1")
    p = data["provenance"]
    c.require(
        set(p)
        == {
            "source",
            "manifest_sha256",
            "nonce",
            "case",
            "kernel_boot_id",
            "collector_sha256",
            "authorization_sha256",
            "manager_client_sha256",
        }
    )
    c.require(re.fullmatch(r"[0-9a-f]{64}", p["manager_client_sha256"]) is not None)
    c.require(type(data["units"]) is dict and set(data["units"]) == set(u.UNITS))
    for name, props in data["units"].items():
        c.require(
            type(props) is dict
            and set(props)
            == set(u.COMMON + (u.SERVICE if name.endswith(".service") else u.SOCKET))
        )
        c.require(all(type(value) is str for value in props.values()))
        c.require(props["Id"] == name and props["LoadState"] == "loaded")
        c.require(props["FragmentPath"] == f"/usr/lib/systemd/system/{name}")
        c.require(props["DropInPaths"] == props["SourcePath"] == "")
        c.require(props["Transient"] == props["NeedDaemonReload"] == "no")
    h = data["helper"]
    c.require(
        type(h) is dict
        and set(h)
        == {
            "pid",
            "start_ticks",
            "uid",
            "exe_sha256",
            "exe_size",
            "invocation",
            "exec_main_start_monotonic_usec",
        }
    )
    for key in ("pid", "start_ticks", "exe_size", "exec_main_start_monotonic_usec"):
        c.require(type(h[key]) is int and 0 < h[key] < 2**64)
    c.require(1 < h["pid"] < 2**31 and h["exe_size"] <= 64 * c.LIMIT)
    c.require(type(h["uid"]) is int and h["uid"] == 0)
    c.require(re.fullmatch(r"[0-9a-f]{64}", h["exe_sha256"]) is not None)
    c.require(re.fullmatch(r"[0-9a-f]{32}", h["invocation"]) is not None)
    service = data["units"][u.UNITS[0]]
    c.require(service["ActiveState"] == "active" and service["SubState"] == "running")
    c.require(service["User"] == service["Group"] == "root")
    c.require(service["MainPID"] == str(h["pid"]))
    c.require(service["InvocationID"] == h["invocation"])
    c.require(
        service["ExecMainStartTimestampMonotonic"]
        == str(h["exec_main_start_monotonic_usec"])
    )


def _store(fd: int, name: str, raw: bytes) -> None:
    c.require(
        name in {"unit-closure.json", *(f"capture-{stage}.json" for stage in c.STAGES)}
    )
    c.require(len(raw) <= c.LIMIT)
    _identity(fd)
    _absent(fd, name)
    temp = ".publisher-" + secrets.token_hex(24)
    out = os.open(
        temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd
    )
    linked = False
    try:
        info = os.fstat(out)
        c.require(
            stat.S_ISREG(info.st_mode) and info.st_uid == 0 and info.st_nlink == 1
        )
        os.fchmod(out, 0o600)
        remaining = memoryview(raw)
        while remaining:
            count = os.write(out, remaining)
            c.require(count > 0)
            remaining = remaining[count:]
        os.fchmod(out, 0o444)
        os.fsync(out)
        _identity(fd)
        current = os.stat(temp, dir_fd=fd, follow_symlinks=False)
        c.require((current.st_dev, current.st_ino) == (info.st_dev, info.st_ino))
        c.require(current.st_nlink == 1 and stat.S_IMODE(current.st_mode) == 0o444)
        os.link(temp, name, src_dir_fd=fd, dst_dir_fd=fd, follow_symlinks=False)
        linked = True
        os.unlink(temp, dir_fd=fd)
        c.require(os.fstat(out).st_nlink == 1)
        _identity(fd)
        os.fsync(fd)
    except BaseException as exc:
        if linked:
            raise PublicationUnknown(
                "Capture связан; durability неизвестна, сохранён " + name
            ) from exc
        raise
    finally:
        try:
            os.close(out)
        finally:
            # Только своё временное имя; связанный capture сохраняется при отказе.
            try:
                os.unlink(temp, dir_fd=fd)
            except FileNotFoundError:
                pass
            except OSError as exc:
                if linked:
                    raise PublicationUnknown(
                        "Capture сохранён; cleanup не подтверждён"
                    ) from exc
                raise


def _publish(stage: str | None, nonce: str, case: str) -> None:
    fd = None
    try:
        c.require(type(nonce) is str and type(case) is str)
        c.require(stage is None or (type(stage) is str and stage in c.STAGES))
        backend = c.SystemBackend()
        auth, inputs_raw, inputs = c._authorize(backend, nonce, case)
        collector = backend.read(
            "/usr/lib/yonik-acceptance/collect_observations.py", sealed=True
        )
        fd = _parent()
        name = "unit-closure.json" if stage is None else f"capture-{stage}.json"
        _absent(fd, name)
        data = (
            u.collect_unit_closure(nonce, case)
            if stage is None
            else c.collect(stage, nonce, case)
        )
        raw = json.dumps(
            data, allow_nan=False, sort_keys=True, separators=(",", ":")
        ).encode()
        data = c.decode(raw)
        c.require(type(data) is dict)
        if stage is None:
            _closure(data)
            client = backend.read("/usr/bin/systemctl", trusted=True, cap=64 * c.LIMIT)
            c.require(data["provenance"]["manager_client_sha256"] == _hash(client))
        else:
            c.require(
                set(data)
                == {"schema", "status", "provenance", "observations", "inference"}
            )
            c.require(
                set(data["observations"])
                == {"collector", "units", "network", "artifacts"}
                | ({"tracked_processes"} if stage == "stopped" else set())
            )
            c.validate_capture(data)
            c.require(data["provenance"]["stage"] == stage)
        p = data["provenance"]
        for key, expected in {
            **{key: inputs[key] for key in ("source", "manifest_sha256")},
            "nonce": nonce,
            "case": case,
            "authorization_sha256": _hash(auth),
            "collector_sha256": _hash(collector),
        }.items():
            c.require(p[key] == expected)
        c.require(
            re.fullmatch(
                r"[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}", p["kernel_boot_id"]
            )
            is not None
        )
        c.require(c._authorize(backend, nonce, case) == (auth, inputs_raw, inputs))
        c.require(
            backend.read(
                "/usr/lib/yonik-acceptance/collect_observations.py", sealed=True
            )
            == collector
        )
        _store(fd, name, raw)
    except PublicationError:
        raise
    except (
        OSError,
        ValueError,
        TypeError,
        KeyError,
        AttributeError,
        RecursionError,
        c.Unavailable,
    ) as exc:
        raise PublicationError("Публикация не подтверждена") from exc
    finally:
        if fd is not None:
            os.close(fd)


def publish_capture(stage: str, nonce: str, case: str) -> None:
    """Опубликовать единственный fixed capture; успех означает завершённый fsync."""
    if stage is None:
        raise PublicationError("Неизвестная стадия")
    _publish(stage, nonce, case)


def publish_unit_closure(nonce: str, case: str) -> None:
    """Опубликовать fixed unit closure после того же grant gate."""
    _publish(None, nonce, case)
