"""Guest-only closed readonly collector; host запуск запрещён guard до subprocess."""

from __future__ import annotations

import hashlib
import json
import os
import posixpath
import re
import select
import signal
import stat
import subprocess
import time
from pathlib import Path

import a4_serial_projection as projection
import a4_serial_protocol as protocol

MASKS = (
    "yonik-helper.service",
    "yonik-worker.service",
    "yonik-worker.socket",
    "yonik-worker-check.service",
    "yonik-worker-check.socket",
)
ROOTS = (
    "/etc/systemd/system",
    "/run/systemd/system",
    "/usr/lib/systemd/system",
    "/run/systemd/generator",
    "/run/systemd/generator.early",
    "/run/systemd/generator.late",
)
SOURCE_NAMES = (
    "a4_serial_collector.py",
    "a4_serial_projection.py",
    "a4_serial_protocol.py",
)
FIELDS = tuple(sorted(projection.STATES.keys() | projection.RELATIONS | {"Job"}))
ENV = {"PATH": "/usr/bin", "LC_ALL": "C", "SYSTEMD_PAGER": "cat", "SYSTEMD_COLORS": "0"}


def _valid_path(path: str, unit: str) -> bool:
    if unit not in projection.REQUIRED or not isinstance(path, str) or len(path) > 512:
        return False
    for root in ROOTS:
        prefix = root + "/"
        if path.startswith(prefix):
            relative = path[len(prefix) :]
            return relative == unit or bool(
                re.fullmatch(
                    re.escape(unit) + r"\.d/[A-Za-z0-9_:-][A-Za-z0-9_.:-]{0,127}\.conf",
                    relative,
                )
            )
    return False


def _read_unit(
    path: str, unit: str, rootfd: int, remaining: int = projection.MAX_BYTES
) -> bytes:
    """Каждый компонент через dirfd; разрешены только bounded in-root aliases."""
    for _ in range(9):
        if not _valid_path(path, unit):
            raise ValueError("Path outside allowlist")
        components = path.lstrip("/").split("/")
        directory = os.dup(rootfd)
        retry = False
        try:
            for index, component in enumerate(components):
                info = os.stat(component, dir_fd=directory, follow_symlinks=False)
                if stat.S_ISLNK(info.st_mode):
                    target = os.readlink(component, dir_fd=directory)
                    parent = "/" + "/".join(components[:index])
                    path = posixpath.normpath(
                        posixpath.join(parent, target, *components[index + 1 :])
                    )
                    retry = True
                    break
                final = index == len(components) - 1
                if final and not stat.S_ISREG(info.st_mode):
                    raise ValueError("Special definition rejected before open")
                flags = os.O_NOFOLLOW | (
                    os.O_PATH if final else os.O_RDONLY | os.O_DIRECTORY
                )
                descriptor = os.open(component, flags, dir_fd=directory)
                os.close(directory)
                directory = descriptor
            if retry:
                continue
            before = os.fstat(directory)
            if (
                not stat.S_ISREG(before.st_mode)
                or before.st_nlink != 1
                or before.st_size > min(projection.MAX_BYTES, remaining)
            ):
                raise ValueError("Not bounded regular definition")
            # Удерживаем O_PATH inode; reopen только проверенного regular fd.
            readable = os.open(
                "/proc/self/fd/" + str(directory), os.O_RDONLY | os.O_NONBLOCK
            )
            os.close(directory)
            directory = readable
            data = bytearray()
            while len(data) <= projection.MAX_BYTES:
                block = os.read(
                    directory, min(4096, projection.MAX_BYTES + 1 - len(data))
                )
                if not block:
                    break
                data.extend(block)
            after = os.fstat(directory)
            if (
                before.st_dev,
                before.st_ino,
                before.st_size,
                before.st_mtime_ns,
                before.st_ctime_ns,
            ) != (
                after.st_dev,
                after.st_ino,
                after.st_size,
                after.st_mtime_ns,
                after.st_ctime_ns,
            ) or len(data) != before.st_size:
                raise ValueError("Definition changed/truncated")
            return bytes(data)
        finally:
            os.close(directory)
    raise ValueError("Alias depth")


def _argv(kind: str, unit: str = "") -> list[str]:
    if kind in {"properties", "paths"} and unit in projection.REQUIRED:
        fields = FIELDS if kind == "properties" else ("FragmentPath", "DropInPaths")
        return [
            "/usr/bin/systemctl",
            "--no-pager",
            "show",
            unit,
            *[v for field in fields for v in ("-p", field)],
        ]
    if kind == "jobs" and not unit:
        return [
            "/usr/bin/systemctl",
            "--no-pager",
            "--no-legend",
            "--plain",
            "list-jobs",
        ]
    if kind == "masks" and not unit:
        return [
            "/usr/bin/systemctl",
            "--no-pager",
            "show",
            *MASKS,
            *[
                v
                for field in (
                    "Id",
                    "LoadState",
                    "ActiveState",
                    "SubState",
                    "MainPID",
                    "Job",
                    "InvocationID",
                )
                for v in ("-p", field)
            ],
        ]
    raise ValueError("No generic guest exec")


def _command(kind: str, deadline: float, unit: str = "") -> str | None:
    remaining = min(1.0, deadline - time.monotonic())
    if remaining <= 0:
        return None
    process = subprocess.Popen(
        _argv(kind, unit),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=ENV,
    )
    assert process.stdout is not None
    os.set_blocking(process.stdout.fileno(), False)
    data = bytearray()
    end = time.monotonic() + remaining
    try:
        while time.monotonic() < end:
            ready, _, _ = select.select(
                [process.stdout], [], [], max(0, end - time.monotonic())
            )
            if ready:
                block = os.read(process.stdout.fileno(), 4096)
                if not block:
                    process.wait(timeout=max(0.001, end - time.monotonic()))
                    return (
                        data.decode("utf-8", errors="strict")
                        if process.returncode == 0
                        else None
                    )
                data.extend(block)
                if len(data) > 32768:
                    return None
        return None
    except (UnicodeError, subprocess.TimeoutExpired):
        return None
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=1)
        process.stdout.close()


def _properties(text: str | None) -> dict[str, str]:
    result = {}
    if text is None:
        return result
    for line in text.splitlines():
        if not line:
            continue
        if "=" not in line:
            return {}
        key, value = line.split("=", 1)
        if key in result:
            return {}
        result[key] = value
    return result


def _jobs(text: str | None) -> tuple[dict, bool]:
    result = {"jobs": [], "foreign_count": 0, "unknown": text is None}
    product = False
    for line in (text or "").splitlines():
        fields = line.split()
        if len(fields) != 4 or not fields[0].isdigit():
            result["unknown"] = True
            continue
        if fields[1] in MASKS:
            product = True
        if fields[1] not in projection.REQUIRED:
            result["foreign_count"] += 1
            continue
        job = projection.project_job(
            {
                "id": int(fields[0]),
                "unit": fields[1],
                "type": fields[2],
                "state": fields[3],
            }
        )
        if job is None or len(result["jobs"]) >= 128:
            result["unknown"] = True
        else:
            result["jobs"].append(job)
    return result, product


def _mask_state(text: str | None) -> dict:
    seen = set()
    if text is None:
        return {"inactive": False, "unknown": True}
    for block in text.strip().split("\n\n"):
        fields = _properties(block)
        unit = fields.get("Id")
        if unit not in MASKS or unit in seen:
            return {"inactive": False, "unknown": True}
        seen.add(unit)
        if (
            fields.get("LoadState") != "masked"
            or fields.get("ActiveState") != "inactive"
            or fields.get("SubState") != "dead"
            or fields.get("MainPID") != "0"
            or fields.get("Job", "").split(" ")[0] != "0"
            or fields.get("InvocationID", "").strip("0")
        ):
            return {"inactive": False, "unknown": False}
    return {"inactive": len(seen) == 5, "unknown": len(seen) != 5}


def _starttime(pid: int) -> int:
    data = Path(f"/proc/{pid}/stat").read_text()
    return int(data[data.rfind(")") + 2 :].split()[19])


def _identity() -> str:
    if (
        os.getuid() != 0
        or os.geteuid() != 0
        or Path("/proc/1/exe").resolve() != Path("/usr/lib/systemd/systemd")
        or Path("/sys/class/dmi/id/sys_vendor").read_text().strip() != "QEMU"
        or not Path("/sys/fs/cgroup/cgroup.controllers").is_file()
    ):
        raise ValueError("Guest identity guard")
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    if not protocol.UUID.fullmatch(boot):
        raise ValueError("Boot UUID")
    return boot


class ProductFailure(ValueError):
    """Наблюдён product process: fixed FAIL, немедленный host stop."""


def _runtime_guard() -> None:
    for unit in MASKS:
        if os.readlink("/etc/systemd/system/" + unit) != "/dev/null":
            raise ValueError("Mask guard")
    for index, entry in enumerate(Path("/proc").iterdir()):
        if index > 8192:
            raise ValueError("Process inventory limit")
        if entry.name.isdigit():
            try:
                if (entry / "comm").read_text().strip() in {
                    "yonik-helper",
                    "yonik-worker",
                    "yonik-desktop",
                    "xray",
                    "sing-box",
                }:
                    raise ProductFailure("Product process")
            except FileNotFoundError:
                continue


class Emitter:
    def __init__(self, fd: int, contract: dict, boot: str) -> None:
        self.fd, self.contract, self.boot = fd, contract, boot
        self.seq = 0
        self.total = 0
        self.phase = "initial"

    def emit(self, kind: str, payload: dict) -> None:
        payload = protocol.checked_payload(kind, payload)
        frame = {
            "seq": self.seq + 1,
            "phase": self.phase,
            "monotonic_ns": time.monotonic_ns(),
            "run_token": self.contract["token"],
            "boot_id": self.boot,
            "pid": os.getpid(),
            "starttime": _starttime(os.getpid()),
            "kind": kind,
            "payload": payload,
        }
        data = (
            protocol.PREFIX + json.dumps(frame, separators=(",", ":")).encode() + b"\n"
        )
        if (
            len(data) > protocol.MAX_RECORD
            or self.seq >= protocol.MAX_RECORDS
            or self.total + len(data) > protocol.MAX_PAYLOAD
        ):
            raise ValueError("Serial limit")
        end = time.monotonic() + 2
        pending = memoryview(data)
        while pending:
            _, ready, _ = select.select(
                [], [self.fd], [], max(0, end - time.monotonic())
            )
            if not ready:
                raise TimeoutError("Serial write")
            count = os.write(self.fd, pending)
            pending = pending[count:]
        self.seq += 1
        self.total += len(data)


def _snapshot(emitter: Emitter, deadline: float, initial: bool) -> None:
    if _identity() != emitter.boot:
        raise ValueError("Boot changed")
    emitter.emit(
        "start",
        {
            key: emitter.contract["sources"][name]
            for key, name in [
                ("collector_sha256", SOURCE_NAMES[0]),
                ("projection_sha256", SOURCE_NAMES[1]),
            ]
        },
    )
    try:
        _runtime_guard()
    except ProductFailure:
        emitter.emit("failure", {"code": "PRODUCT_JOB_FAIL"})
        raise
    except (OSError, ValueError):
        emitter.emit("failure", {"code": "GUARD_RED"})
        raise
    emitter.emit(
        "identity", {"pid1": "systemd", "dmi": "QEMU", "cgroup2": True, "masks": 5}
    )
    masks = _mask_state(_command("masks", deadline))
    emitter.emit("masks", masks)
    if masks["unknown"]:
        emitter.emit("failure", {"code": "GUARD_RED"})
        raise ValueError("Mask state UNKNOWN")
    if not masks["inactive"]:
        emitter.emit("failure", {"code": "PRODUCT_JOB_FAIL"})
        raise ValueError("Product active")
    rootfd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    total = 0
    try:
        for unit in projection.REQUIRED:
            emitter.emit(
                "properties",
                projection.project_properties(
                    unit, _properties(_command("properties", deadline, unit))
                ),
            )
            if not initial:
                continue
            paths = _properties(_command("paths", deadline, unit))
            fragment = paths.get("FragmentPath", "")
            dropins = paths.get("DropInPaths", "").split()
            if len(dropins) > 4 or len(set(dropins)) != len(dropins):
                dropins = []
                fragment = ""
            for index, path in enumerate([fragment, *dropins]):
                try:
                    if time.monotonic() >= deadline:
                        raise TimeoutError("Snapshot deadline")
                    data = _read_unit(path, unit, rootfd, 256 * 1024 - total)
                    total += len(data)
                    if total > 256 * 1024:
                        raise ValueError("Read total limit")
                    definition = projection.project_definition(unit, data)
                except (OSError, ValueError, TimeoutError):
                    definition = {"unit": unit, "status": "RED_READ"}
                emitter.emit(
                    "definition",
                    {"unit": unit, "index": index, "definition": definition},
                )
        jobs, product = _jobs(_command("jobs", deadline))
        emitter.emit("jobs", jobs)
        if product:
            emitter.emit("failure", {"code": "PRODUCT_JOB_FAIL"})
            raise ValueError("Product job")
    finally:
        os.close(rootfd)
    if time.monotonic() >= deadline:
        emitter.emit("failure", {"code": "DEADLINE"})
        raise TimeoutError("Snapshot deadline")


def _contract() -> dict:
    descriptor = os.open("/run/a4-serial/contract.json", os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        if (
            info.st_size > 4096
            or info.st_uid != 0
            or stat.S_IMODE(info.st_mode) != 0o600
            or not stat.S_ISREG(info.st_mode)
        ):
            raise ValueError("Contract owner/mode")
        contract = protocol.exact(json.loads(stream.read(4097)), {"token", "sources"})
    if (
        not isinstance(contract["token"], str)
        or not protocol.TOKEN.fullmatch(contract["token"])
        or set(contract["sources"]) != set(SOURCE_NAMES)
    ):
        raise ValueError("Contract schema")
    for name in SOURCE_NAMES:
        if (
            hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
            != contract["sources"][name]
        ):
            raise ValueError("Collector source drift")
    return contract


def main() -> int:
    # До guard нет subprocess, serial open, fork, чтения seed или произвольных paths.
    boot = _identity()
    contract = _contract()
    entered = time.monotonic()
    serial = os.open(
        "/dev/ttyS0", os.O_WRONLY | os.O_NOCTTY | os.O_NONBLOCK | os.O_NOFOLLOW
    )
    emitter = Emitter(serial, contract, boot)
    signal.signal(signal.SIGALRM, lambda *_: os._exit(2))
    signal.alarm(25)
    try:
        _snapshot(emitter, entered + 25, True)
        signal.alarm(0)
        child = os.fork()
        if child:
            emitter.emit("spawn", {"pid": child, "starttime": _starttime(child)})
            emitter.emit("end", {"outcome": "CAPTURED"})
            return 0
        # Child не держит cloud-init stdout/stderr. Sequence родителя: SPAWN, END.
        null = os.open("/dev/null", os.O_RDWR)
        for fd in (0, 1, 2):
            os.dup2(null, fd)
        if null > 2:
            os.close(null)
        emitter.seq += 2
        emitter.total += 2048  # консервативный reserve для SPAWN/END родителя
        emitter.phase = "delayed"
        signal.signal(signal.SIGALRM, lambda *_: os._exit(2))
        signal.alarm(max(1, int(entered + 50 - time.monotonic())))
        time.sleep(max(0, entered + 30 - time.monotonic()))
        try:
            _snapshot(emitter, min(entered + 50, time.monotonic() + 15), False)
            emitter.emit("end", {"outcome": "CAPTURED"})
            os._exit(0)
        except Exception:
            emitter.emit("failure", {"code": "READ_UNKNOWN"})
            emitter.emit("end", {"outcome": "UNKNOWN"})
            os._exit(1)
    except Exception:
        emitter.emit("failure", {"code": "READ_UNKNOWN"})
        emitter.emit("end", {"outcome": "UNKNOWN"})
        return 1
    finally:
        os.close(serial)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception:
        raise SystemExit(1)
