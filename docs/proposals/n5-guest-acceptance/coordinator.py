"""SOURCE: закрытый координатор одного свежего owned guest; не runtime grant."""

from __future__ import annotations

import concurrent.futures
import fcntl
import hashlib
import json
import os
import re
import select
import stat
import time
from typing import Any

import publish_captures as publisher
import runner
from fixture import validate_receipt

_BASE = "/usr/lib/yonik-acceptance/"
_ROOT = "/run/yonik-acceptance"
_RETAINED: list[_Owner] = []


def _require(value: bool) -> None:
    if not value:
        raise runner.Refused("COORDINATOR_CLOSED")


def _read(path: str, mode: int, limit: int = 4 * 1024 * 1024) -> bytes:
    """Root-owned openat chain; bounded regular inode, без path reopen."""
    _require(
        path.startswith("/")
        and all(p not in ("", ".", "..") for p in path[1:].split("/"))
    )
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        parts = path[1:].split("/")
        for index, part in enumerate(parts):
            parent = os.fstat(fd)
            _require(parent.st_uid == 0 and not parent.st_mode & 0o022)
            flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
            if index < len(parts) - 1:
                flags |= os.O_DIRECTORY
            child = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = child
        before = os.fstat(fd)
        _require(stat.S_ISREG(before.st_mode) and before.st_uid == 0)
        _require(stat.S_IMODE(before.st_mode) == mode and before.st_nlink == 1)
        _require(before.st_size <= limit)
        raw = bytearray()
        while len(raw) <= limit:
            chunk = os.read(fd, min(65536, limit + 1 - len(raw)))
            if not chunk:
                break
            raw.extend(chunk)
        after = os.fstat(fd)

        def identity(s: os.stat_result) -> tuple[int, int, int, int, int]:
            return (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)

        _require(identity(before) == identity(after) and len(raw) == after.st_size)
        return bytes(raw)
    finally:
        os.close(fd)


def _root_directory() -> int:
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in ("", "run", "yonik-acceptance"):
            if part:
                child = os.open(
                    part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd
                )
                os.close(fd)
                fd = child
            info = os.fstat(fd)
            _require(info.st_uid == 0 and not info.st_mode & 0o022)
        return fd
    except BaseException:
        os.close(fd)
        raise


def _absent(fd: int, name: str) -> None:
    try:
        os.stat(name, dir_fd=fd, follow_symlinks=False)
    except FileNotFoundError:
        return
    raise runner.Refused("FRESH_GUEST_REQUIRED")


def _json(raw: bytes) -> dict[str, Any]:
    def invalid(value: str) -> None:
        raise runner.Refused("NONFINITE_JSON")

    data = json.loads(
        raw, object_pairs_hook=runner.object_pairs, parse_constant=invalid
    )
    _require(type(data) is dict)
    return data


def _sha(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def _gate(core: str) -> tuple[dict[str, Any], int, int]:
    # Только trusted material reads до завершения ОБОИХ grants.
    auth = _json(_read(_BASE + "n5-authorization.json", 0o444))
    fixture_raw = _read(str(runner.FIXTURE), 0o444)
    grant_raw = _read(str(runner.GRANT), 0o444)
    launch_raw = _read(str(runner.LAUNCH), 0o444)
    fixture, grant, launch = map(_json, (fixture_raw, grant_raw, launch_raw))
    _require(
        set(auth)
        == {
            "schema",
            "execution",
            "source_commit",
            "runner_sha256",
            "fixture_sha256",
            "grant_sha256",
            "launch_sha256",
            "source_blockers",
            "missing",
        }
    )
    _require(
        set(grant)
        == {
            "schema",
            "explicit_user_runtime_grant",
            "source_commit",
            "runner_sha256",
            "fixture_sha256",
            "launch_sha256",
            "cases",
        }
    )
    _require(set(launch) == {"schema", "manifest_sha256", "after_setup", "controls"})
    runner.authorize(auth, fixture, grant)
    for raw, key in (
        (fixture_raw, "fixture_sha256"),
        (grant_raw, "grant_sha256"),
        (launch_raw, "launch_sha256"),
    ):
        _require(_sha(raw) == auth[key])
    _require(_sha(_read(_BASE + "runner.py", 0o444)) == auth["runner_sha256"])
    _require(launch.get("schema") == "N5_TRUSTED_LAUNCH_V1")
    _require(validate_receipt(fixture, launch.get("after_setup")).status == "READY")
    _require(runner.preflight(fixture).manifest_sha256 == launch["manifest_sha256"])
    controls = launch["controls"]
    _require(
        set(controls)
        == {
            "caller_uid",
            "source_id",
            "profile_id",
            "private_bus",
            "flows",
            "kernel_boot_id",
            "capture_nonce",
            "capture_authorization_sha256",
            "collector_sha256",
            "flow_observer",
            "worker_cgroup",
            "python_sha256",
            "python_client_sha256",
        }
    )
    _require(
        _sha(_read(_BASE + "collect_observations.py", 0o444))
        == controls["collector_sha256"]
    )
    for key in (
        "capture_nonce",
        "capture_authorization_sha256",
        "collector_sha256",
        "python_sha256",
        "python_client_sha256",
    ):
        _require(runner.digest(controls[key]))
    _require(
        type(controls["kernel_boot_id"]) is str
        and re.fullmatch(
            r"[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}", controls["kernel_boot_id"]
        )
        is not None
    )
    for key in ("source_id", "profile_id"):
        _require(type(controls[key]) is int and 0 < controls[key] < 2**63)
    _require(type(controls["flows"]) is list and 0 < len(controls["flows"]) <= 128)
    root_raw = _read(_ROOT + "/authorization.json", 0o444)
    root = _json(root_raw)
    _require(
        set(root) == {"schema", "execution", "nonce", "case", "stages", "input_sha256"}
    )
    _require(
        root["schema"] == "ROOT_CAPTURE_AUTH_V1"
        and root["execution"] == "USER_GRANTED_GUEST_CAPTURE"
    )
    _require(
        root["nonce"] == controls["capture_nonce"]
        and root["case"] == core.replace("-", "_") + "_positive"
    )
    _require(root["stages"] == ["baseline", "connected", "stopped"])
    _require(_sha(root_raw) == controls["capture_authorization_sha256"])
    inputs_raw = _read(_ROOT + "/input.json", 0o444)
    inputs = _json(inputs_raw)
    _require(_sha(inputs_raw) == root["input_sha256"])
    _require(
        inputs
        == {
            "schema": "ROOT_CAPTURE_INPUT_V1",
            "source": fixture["source_commit"],
            "manifest_sha256": launch["manifest_sha256"],
        }
    )
    for artifact in fixture["artifacts"].values():
        raw = _read(
            artifact["installed_path"],
            artifact["mode"],
            512 * 1024 * 1024,
        )
        _require(len(raw) == artifact["size"] and _sha(raw) == artifact["sha256"])
    uid = controls["caller_uid"]
    _require(type(uid) is int and 0 < uid < 2**32)
    _require(
        controls["private_bus"] == f"unix:path=/run/user/{uid}/yonik-n5-private-bus"
    )
    # Только локальная root-owned passwd, без NSS и без предположения UID == GID.
    matches = []
    for line in _read("/etc/passwd", 0o644).decode().splitlines():
        fields = line.split(":")
        if len(fields) == 7 and fields[2].isdigit() and int(fields[2]) == uid:
            _require(fields[3].isdigit())
            matches.append(int(fields[3]))
    _require(len(matches) == 1 and 0 < matches[0] < 2**32)
    expected = {
        "trusted_capture_sha256": auth["launch_sha256"],
        "executing_source_sha256": fixture["source_archive_sha256"],
        "controls": controls,
        "source_commit": fixture["source_commit"],
        "manifest_sha256": launch["manifest_sha256"],
        "core": core,
        "artifacts": {
            a["installed_path"]: {"sha256": a["sha256"], "size": a["size"]}
            for a in fixture["artifacts"].values()
        },
    }
    return expected, uid, matches[0]


class Quarantined(runner.RuntimeFailure):
    """Owner удерживает ресурсы до внешнего authorized разбора VM."""

    def __init__(self, owner: _Owner) -> None:
        super().__init__("OUTER_VM_OWNER_REQUIRED")
        self.owner = owner


class _Owner:
    def __init__(self, core: str, expected: dict[str, Any]) -> None:
        self.core, self.expected = core, expected
        self.fd: int | None = None
        self.driver: runner.ProcessDriver | None = None
        self.worker = concurrent.futures.ThreadPoolExecutor(max_workers=1)
        self.future: concurrent.futures.Future[None] | None = None
        self.journal: list[dict[str, Any]] = []
        self.unknown = False
        self.deadline = time.monotonic() + runner.CAMPAIGN_SECONDS

    def quarantine(self) -> None:
        self.unknown = True
        if self not in _RETAINED:
            _RETAINED.append(self)

    def publish(self, stage: str | None, deadline: float) -> None:
        _require(not self.unknown)
        expected, _, _ = _gate(self.core)
        _require(expected == self.expected)
        end = min(deadline, self.deadline, time.monotonic() + 15)
        _require(time.monotonic() < end)
        nonce = expected["controls"]["capture_nonce"]
        case = self.core.replace("-", "_") + "_positive"
        call = (
            publisher.publish_unit_closure
            if stage is None
            else publisher.publish_capture
        )
        args = (nonce, case) if stage is None else (stage, nonce, case)
        entry = {
            "stage": stage,
            "nonce": nonce,
            "case": case,
            "deadline": end,
            "submitted": time.monotonic(),
            "outcome": "PENDING",
        }
        self.journal.append(entry)
        self.future = self.worker.submit(call, *args)
        try:
            while True:
                remaining = end - time.monotonic()
                _require(remaining > 0)
                if self.driver is not None:
                    _require(self.driver.process.poll() is None)
                    # Между командами любой stdout (EOF или extra frame) запрещён.
                    reader = self.driver.process.stdout
                    _require(reader is not None)
                    _require(not select.select([reader.fileno()], [], [], 0)[0])
                try:
                    self.future.result(timeout=min(remaining, 0.1))
                    break
                except concurrent.futures.TimeoutError:
                    if self.future.done():
                        raise

            _require(time.monotonic() < end)
            if self.driver is not None:
                _require(self.driver.process.poll() is None)
            entry.update(returned=time.monotonic(), outcome="DURABLE")
        except BaseException:
            entry.update(returned=time.monotonic(), outcome="UNKNOWN")
            self.quarantine()
            raise Quarantined(self) from None

    def before_start(self, deadline: float) -> None:
        self.publish("baseline", deadline)

    def after_await(self, deadline: float) -> None:
        self.publish("connected", deadline)

    def after_close_desktop_owner(self, deadline: float) -> None:
        self.publish("stopped", deadline)

    def exchange(
        self, core: str, case: str, action: str, deadline: float
    ) -> dict[str, Any]:
        _require(not self.unknown and self.driver is not None)
        return self.driver.exchange(core, case, action, min(deadline, self.deadline))


def _validate_child(
    driver: runner.ProcessDriver, uid: int, gid: int, expected: dict[str, Any]
) -> None:
    """Только собственный ещё живой child, удержанный pidfd; до первой команды."""
    process = driver.process
    _require(process.poll() is None)
    root = f"/proc/{process.pid}"

    def read(name: str) -> bytes:
        fd = os.open(root + "/" + name, os.O_RDONLY | os.O_NOFOLLOW)
        try:
            raw = os.read(fd, 65537)
            _require(len(raw) <= 65536)
            return raw
        finally:
            os.close(fd)

    before = read("stat").rsplit(b")", 1)[1].split()
    status = dict(line.split(":", 1) for line in read("status").decode().splitlines())
    _require(status["Uid"].split() == [str(uid)] * 4)
    _require(status["Gid"].split() == [str(gid)] * 4)
    _require(not status["Groups"].split())
    for name in ("CapInh", "CapPrm", "CapEff", "CapAmb"):
        _require(int(status[name], 16) == 0)
    _require(os.readlink(root + "/exe") == runner.DRIVER)
    fd = os.open(root + "/exe", os.O_RDONLY)
    try:
        pin = expected["artifacts"][runner.DRIVER]
        info = os.fstat(fd)
        _require(info.st_uid == 0 and stat.S_IMODE(info.st_mode) == 0o555)
        _require(info.st_size == pin["size"])
        with os.fdopen(os.dup(fd), "rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        _require(digest == pin["sha256"])
    finally:
        os.close(fd)
    after = read("stat").rsplit(b")", 1)[1].split()
    _require(before[19] == after[19] and int(after[19]) > 0 and process.poll() is None)


def run_owned_guest(core: str, case: str) -> dict[str, Any]:
    """Один core × positive; selectors никогда не дают root authority."""
    _require(type(core) is str and type(case) is str)
    _require(core in runner.CORES and case in runner.CASES)
    if case != "positive":
        return {
            "core": core,
            "case": case,
            "status": "UNAVAILABLE",
            "reason": "SOURCE_ONLY",
        }
    expected, uid, gid = _gate(core)
    _require(os.geteuid() == 0)
    owner = _Owner(core, expected)
    try:
        owner.fd = _root_directory()
        fcntl.flock(owner.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for name in (
            "lifecycle-owned",
            "unit-closure.json",
            "capture-baseline.json",
            "capture-connected.json",
            "capture-stopped.json",
        ):
            _absent(owner.fd, name)
        marker = os.open(
            "lifecycle-owned",
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
            dir_fd=owner.fd,
        )
        try:
            os.write(marker, b"OWNED: outer VM owner required; no resume\n")
            os.fsync(marker)
        finally:
            os.close(marker)
        os.fsync(owner.fd)
        owner.publish(None, owner.deadline)
        _require(_gate(core) == (expected, uid, gid))
        owner.driver = runner.ProcessDriver.__new__(runner.ProcessDriver)
        owner.driver.__init__(uid, gid)
        _validate_child(owner.driver, uid, gid, expected)
        result = runner.run_case(owner, core, case, expected, owner.deadline)
        if result["status"] != "PASS":
            raise runner.Refused("MISSING_ACTUAL_WITNESS")
        owner.driver.close()
        owner.worker.shutdown(wait=True)
        os.close(owner.fd)
        owner.fd = None
        return result
    except BaseException:
        owner.quarantine()
        raise Quarantined(owner) from None
