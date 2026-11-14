"""Независимый read-only guest capture. Импорт ничего не измеряет."""

from __future__ import annotations

import hashlib
import json
import os
import re
import selectors
import stat
import subprocess
import time
from typing import Any

ROOT = "/run/yonik-acceptance"
LIMIT = 1024 * 1024
DEADLINE = 5.0
STAGES = ("baseline", "connected", "stopped")
ARTIFACTS = (
    "/usr/lib/yonik/yonik-helper",
    "/usr/lib/yonik/yonik-worker",
    "/usr/bin/yonik-desktop",
    "/usr/lib/yonik/engines/sing-box",
    "/usr/lib/yonik/engines/xray",
)
UNITS = ("yonik-helper.service", "yonik-worker.service")
COMMANDS = {
    "units": (
        "/usr/bin/systemctl",
        "list-units",
        "--all",
        "--no-pager",
        "--plain",
        "--output=json",
    ),
    "links": ("/usr/bin/ip", "-j", "link", "show"),
    "addresses": ("/usr/bin/ip", "-j", "address", "show"),
    "routes": ("/usr/bin/ip", "-j", "route", "show", "table", "all"),
    "rules": ("/usr/bin/ip", "-j", "rule", "show"),
    "nft": ("/usr/bin/nft", "-j", "list", "ruleset"),
}
for _unit in UNITS:
    COMMANDS[_unit] = (
        "/usr/bin/systemctl",
        "show",
        _unit,
        "--no-pager",
        "--property=MainPID,InvocationID,ControlGroup,ActiveState",
    )


class Unavailable(Exception):
    """Недостаточно надёжных наблюдений; текст не экспортируется."""


def decode(raw: bytes) -> Any:
    """Отклонить дубликаты, nonfinite и слишком большие документы."""
    if len(raw) > LIMIT:
        raise Unavailable("size")

    def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in items:
            if key in result:
                raise Unavailable("duplicate")
            result[key] = value
        return result

    def constant(value: str) -> None:
        raise Unavailable("nonfinite")

    return json.loads(raw, object_pairs_hook=pairs, parse_constant=constant)


def digest(value: Any) -> str:
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def require(condition: bool) -> None:
    if not condition:
        raise Unavailable("validation")


class SystemBackend:
    """Реальные чтения доступны только после collect gate; без shell и RPC."""

    def uid(self) -> int:
        return os.geteuid()

    def pid(self) -> int:
        return os.getpid()

    def read(self, path: str, *, sealed: bool = False, cap: int = LIMIT) -> bytes:
        require(path.startswith("/") and ".." not in path.split("/"))
        fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
        try:
            parts = path.strip("/").split("/")
            for index, part in enumerate(parts):
                flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
                if index < len(parts) - 1:
                    flags |= os.O_DIRECTORY
                child = os.open(part, flags, dir_fd=fd)
                os.close(fd)
                fd = child
                info = os.fstat(fd)
                if sealed:
                    require(info.st_uid == 0 and not info.st_mode & 0o022)
            before = os.fstat(fd)
            require(stat.S_ISREG(before.st_mode))
            if sealed:
                require(not before.st_mode & 0o222 and before.st_nlink == 1)
            raw = bytearray()
            while len(raw) <= cap:
                chunk = os.read(fd, min(65536, cap + 1 - len(raw)))
                if not chunk:
                    break
                raw.extend(chunk)
            after = os.fstat(fd)
            require(len(raw) <= cap)
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
            )
            return bytes(raw)
        finally:
            os.close(fd)

    def command(self, name: str) -> bytes:
        require(name in COMMANDS)
        child = subprocess.Popen(
            COMMANDS[name],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env={"PATH": "/usr/bin", "LC_ALL": "C"},
            close_fds=True,
        )
        try:
            require(child.stdout is not None)
            output = bytearray()
            end = time.monotonic() + DEADLINE
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                while True:
                    remaining = end - time.monotonic()
                    require(remaining > 0)
                    require(bool(selector.select(remaining)))
                    chunk = os.read(
                        child.stdout.fileno(), min(65536, LIMIT + 1 - len(output))
                    )
                    if not chunk:
                        break
                    output.extend(chunk)
                    require(len(output) <= LIMIT)
            require(child.wait(timeout=max(0.001, end - time.monotonic())) == 0)
            return bytes(output)
        finally:
            # Только собственный child; чужие PID/группы никогда не сигнализируются.
            try:
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=DEADLINE)
            finally:
                if child.stdout is not None:
                    child.stdout.close()


def process(backend: SystemBackend, pid: int) -> dict[str, Any]:
    require(type(pid) is int and 0 < pid < 2**31)
    path = f"/proc/{pid}"
    first = backend.read(path + "/stat")
    require(first.startswith(str(pid).encode() + b" (") and first.rfind(b")") > 0)
    fields = first[first.rfind(b")") + 2 :].split()
    require(len(fields) >= 20)
    start = int(fields[19])
    uid_lines = [
        line.split()[1:]
        for line in backend.read(path + "/status").splitlines()
        if line.startswith(b"Uid:")
    ]
    require(len(uid_lines) == 1 and len(uid_lines[0]) == 4)
    uids = [int(item) for item in uid_lines[0]]
    require(backend.read(path + "/stat") == first)
    return {"pid": pid, "start_ticks": start, "uids": uids}


def unit(backend: SystemBackend, name: str) -> dict[str, Any]:
    raw = backend.command(name)
    props = dict(line.split("=", 1) for line in raw.decode().splitlines())
    require(set(props) == {"MainPID", "InvocationID", "ControlGroup", "ActiveState"})
    pid = int(props["MainPID"])
    invocation = props["InvocationID"]
    require(invocation == "" or re.fullmatch(r"[0-9a-f]{32}", invocation) is not None)
    group = props["ControlGroup"]
    require(group == "" or group == f"/system.slice/{name}")
    populated: int | None = None
    members: list[dict[str, Any]] = []
    if group:
        events = dict(
            line.split()
            for line in backend.read("/sys/fs/cgroup" + group + "/cgroup.events")
            .decode()
            .splitlines()
        )
        populated = int(events["populated"])
        require(populated in (0, 1))
        pids = backend.read("/sys/fs/cgroup" + group + "/cgroup.procs").split()
        require(len(pids) <= 256)
        members = [process(backend, int(item)) for item in pids]
        require(
            backend.read("/sys/fs/cgroup" + group + "/cgroup.procs").split() == pids
        )
        require(
            dict(
                line.split()
                for line in backend.read("/sys/fs/cgroup" + group + "/cgroup.events")
                .decode()
                .splitlines()
            )
            == events
        )
        # Нельзя принять снимок, если canonical unit изменился за время чтения.
    require(backend.command(name) == raw)
    return {
        "main": process(backend, pid) if pid else None,
        "invocation": invocation,
        "cgroup": group,
        "populated": populated,
        "members": members,
    }


def network(backend: SystemBackend) -> dict[str, list[str]]:
    result = {}
    for key in ("links", "addresses", "routes", "rules", "nft"):
        data = decode(backend.command(key))
        if key == "nft":
            require(type(data) is dict and set(data) == {"nftables"})
            data = data["nftables"]
            require(type(data) is list)
            data = [
                item for item in data if not (type(item) is dict and "metainfo" in item)
            ]
        require(type(data) is list and all(type(item) is dict for item in data))
        # Не экспортируем URI, адреса, правила или потенциальные секреты.
        result[key] = sorted(digest(item) for item in data)
    return result


def collect(
    stage: str, nonce: str, case: str, *, backend: SystemBackend | None = None
) -> dict[str, Any]:
    """Фиксированные inputs подписывает доверием root outer fixture, не caller."""
    backend = backend if backend is not None else SystemBackend()
    failure = {"schema": 1, "status": "UNAVAILABLE"}
    try:
        require(stage in STAGES and re.fullmatch(r"[0-9a-f]{64}", nonce) is not None)
        require(re.fullmatch(r"[A-Za-z0-9_-]{1,96}", case) is not None)
        require(backend.uid() == 0)
        auth_raw = backend.read(ROOT + "/authorization.json", sealed=True)
        auth = decode(auth_raw)
        require(
            type(auth) is dict
            and set(auth)
            == {"schema", "execution", "nonce", "case", "stages", "input_sha256"}
        )
        require(
            auth["schema"] == "ROOT_CAPTURE_AUTH_V1"
            and auth["execution"] == "USER_GRANTED_GUEST_CAPTURE"
        )
        require(
            auth["nonce"] == nonce
            and auth["case"] == case
            and auth["stages"] == list(STAGES)
        )
        input_raw = backend.read(ROOT + "/input.json", sealed=True)
        require(hashlib.sha256(input_raw).hexdigest() == auth["input_sha256"])
        inputs = decode(input_raw)
        require(
            type(inputs) is dict
            and set(inputs) == {"schema", "source", "manifest_sha256"}
        )
        require(inputs["schema"] == "ROOT_CAPTURE_INPUT_V1")
        require(re.fullmatch(r"[0-9a-f]{40}", inputs["source"]) is not None)
        require(re.fullmatch(r"[0-9a-f]{64}", inputs["manifest_sha256"]) is not None)
        # До этой точки запрещены proc, systemd, netlink и subprocess.
        observations = {
            "collector": process(backend, backend.pid()),
            "units": {name: unit(backend, name) for name in UNITS},
            "network": network(backend),
            "artifacts": {},
        }
        for path in ARTIFACTS:
            elf = backend.read(path, sealed=True, cap=64 * LIMIT)
            require(elf.startswith(b"\x7fELF"))
            observations["artifacts"][path] = {
                "sha256": hashlib.sha256(elf).hexdigest(),
                "size": len(elf),
            }
        if stage == "stopped":
            previous = read_capture("connected", nonce, case, backend=backend)
            require(previous["status"] == "ACTUAL")
            reaped = []
            for previous_unit in previous["observations"]["units"].values():
                for member in previous_unit["members"] + (
                    [previous_unit["main"]] if previous_unit["main"] else []
                ):
                    try:
                        backend.read(f"/proc/{member['pid']}/stat")
                    except FileNotFoundError:
                        gone = True
                    else:
                        now = process(backend, member["pid"])
                        gone = now["start_ticks"] != member["start_ticks"]
                    reaped.append(
                        {
                            "pid": member["pid"],
                            "start_ticks": member["start_ticks"],
                            "absent": gone,
                        }
                    )
            observations["tracked_processes"] = reaped
        require(backend.read(ROOT + "/authorization.json", sealed=True) == auth_raw)
        require(backend.read(ROOT + "/input.json", sealed=True) == input_raw)
        result = {
            "schema": 1,
            "status": "ACTUAL",
            "provenance": {
                "manifest_sha256": inputs["manifest_sha256"],
                "source": inputs["source"],
                "case": case,
                "stage": stage,
                "nonce": nonce,
                "collector_pid": backend.pid(),
                "authorization_sha256": hashlib.sha256(auth_raw).hexdigest(),
                "kernel_boot_id": backend.read("/proc/sys/kernel/random/boot_id")
                .decode()
                .strip(),
                "collector_sha256": hashlib.sha256(
                    backend.read(
                        "/usr/lib/yonik-acceptance/collect_observations.py", sealed=True
                    )
                ).hexdigest(),
            },
            "observations": observations,
            "inference": {"native_report_and_roles": "SOURCE_INVARIANT"},
        }
        validate_capture(result)
        require(len(json.dumps(result).encode()) <= LIMIT)
        return result
    except (
        OSError,
        ValueError,
        TypeError,
        KeyError,
        IndexError,
        RecursionError,
        Unavailable,
        subprocess.SubprocessError,
    ):
        return failure


def compare_cleanup(
    baseline: dict[str, Any], connected: dict[str, Any], stopped: dict[str, Any]
) -> dict[str, Any]:
    """Консервативное сравнение: изменение неизвестного ресурса не означает успех."""
    try:
        for capture, stage in zip((baseline, connected, stopped), STAGES, strict=True):
            validate_capture(capture)
            require(
                capture["status"] == "ACTUAL"
                and capture["provenance"]["stage"] == stage
            )
            for key in (
                "source",
                "manifest_sha256",
                "case",
                "nonce",
                "authorization_sha256",
                "collector_sha256",
                "kernel_boot_id",
            ):
                require(capture["provenance"][key] == baseline["provenance"][key])
        before, live, after = [
            item["observations"] for item in (baseline, connected, stopped)
        ]
        restored = before["network"] == after["network"]
        empty = True
        for name in UNITS:
            current = after["units"][name]
            empty = (
                empty
                and current["populated"] == 0
                and current["main"] is None
                and current["members"] == []
            )
            # Empty parent cgroup alone не доказывает reap/отсутствие escaped descendants.
        tracked = after["tracked_processes"]
        expected = {
            (member["pid"], member["start_ticks"])
            for item in live["units"].values()
            for member in item["members"] + ([item["main"]] if item["main"] else [])
        }
        require(
            bool(expected)
            and {(item["pid"], item["start_ticks"]) for item in tracked} == expected
        )
        return {
            "schema": 1,
            "status": "ACTUAL",
            "network_restored": restored,
            "canonical_cgroups_empty": empty,
            "tracked_processes_absent": all(item["absent"] is True for item in tracked),
            "own_network_delta": {
                key: sorted(set(live["network"][key]) - set(before["network"][key]))
                for key in before["network"]
            },
            "foreign_missing": {
                key: sorted(set(before["network"][key]) - set(after["network"][key]))
                for key in before["network"]
            },
            "escaped_descendants": "UNAVAILABLE",
        }
    except (KeyError, TypeError, ValueError, AttributeError, Unavailable):
        return {"schema": 1, "status": "UNAVAILABLE"}


def validate_capture(data: dict[str, Any]) -> None:
    """Проверить вложенные типы, не превращая наличие JSON в измерение."""
    require(
        type(data) is dict
        and type(data["schema"]) is int
        and data["schema"] == 1
        and data["status"] == "ACTUAL"
    )
    provenance = data["provenance"]
    require(
        type(provenance) is dict
        and set(provenance)
        == {
            "source",
            "manifest_sha256",
            "case",
            "stage",
            "nonce",
            "collector_pid",
            "authorization_sha256",
            "kernel_boot_id",
            "collector_sha256",
        }
    )
    for key, length in (
        ("source", 40),
        ("manifest_sha256", 64),
        ("nonce", 64),
        ("authorization_sha256", 64),
        ("collector_sha256", 64),
    ):
        require(
            type(provenance[key]) is str
            and re.fullmatch(r"[0-9a-f]{" + str(length) + "}", provenance[key])
            is not None
        )
    require(
        re.fullmatch(
            r"[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}", provenance["kernel_boot_id"]
        )
        is not None
    )

    def identity(value: Any) -> None:
        require(type(value) is dict and set(value) == {"pid", "start_ticks", "uids"})
        require(type(value["pid"]) is int and 0 < value["pid"] < 2**31)
        require(type(value["start_ticks"]) is int and value["start_ticks"] > 0)
        require(type(value["uids"]) is list and len(value["uids"]) == 4)
        require(all(type(uid) is int and 0 <= uid < 2**32 for uid in value["uids"]))

    observations = data["observations"]
    identity(observations["collector"])
    require(
        type(provenance["collector_pid"]) is int
        and provenance["collector_pid"] == observations["collector"]["pid"]
    )
    require(set(observations["units"]) == set(UNITS))
    for name, value in observations["units"].items():
        require(
            type(value) is dict
            and set(value) == {"main", "invocation", "cgroup", "populated", "members"}
        )
        require(value["cgroup"] in ("", f"/system.slice/{name}"))
        require(
            value["populated"] is None
            or (type(value["populated"]) is int and value["populated"] in (0, 1))
        )
        require(
            type(value["invocation"]) is str
            and (
                value["invocation"] == ""
                or re.fullmatch(r"[0-9a-f]{32}", value["invocation"]) is not None
            )
        )
        require(type(value["members"]) is list and len(value["members"]) <= 256)
        for member in value["members"]:
            identity(member)
        if value["main"] is not None:
            identity(value["main"])
    require(
        set(observations["network"]) == {"links", "addresses", "routes", "rules", "nft"}
    )
    for values in observations["network"].values():
        require(type(values) is list)
        require(
            all(
                type(item) is str and re.fullmatch(r"[0-9a-f]{64}", item) is not None
                for item in values
            )
        )
    require(set(observations["artifacts"]) == set(ARTIFACTS))
    for artifact in observations["artifacts"].values():
        require(
            set(artifact) == {"sha256", "size"}
            and type(artifact["size"]) is int
            and 4 <= artifact["size"] <= 64 * LIMIT
        )
        require(re.fullmatch(r"[0-9a-f]{64}", artifact["sha256"]) is not None)
    require(data["inference"] == {"native_report_and_roles": "SOURCE_INVARIANT"})
    if provenance["stage"] == "stopped":
        require(type(observations["tracked_processes"]) is list)
        for item in observations["tracked_processes"]:
            require(
                type(item) is dict and set(item) == {"pid", "start_ticks", "absent"}
            )
            require(
                type(item["pid"]) is int
                and item["pid"] > 0
                and type(item["start_ticks"]) is int
                and item["start_ticks"] > 0
            )
            require(type(item["absent"]) is bool)


def read_capture(
    stage: str, nonce: str, case: str, *, backend: SystemBackend | None = None
) -> dict[str, Any]:
    """Потребитель читает только fixed root-owned sealed capture; не выдаёт grant."""
    try:
        require(stage in STAGES)
        data = decode(
            (backend or SystemBackend()).read(
                ROOT + f"/capture-{stage}.json", sealed=True
            )
        )
        require(
            type(data) is dict
            and set(data)
            == {"schema", "status", "provenance", "observations", "inference"}
        )
        require(data["schema"] == 1 and data["status"] == "ACTUAL")
        require(
            data["provenance"]["stage"] == stage
            and data["provenance"]["nonce"] == nonce
            and data["provenance"]["case"] == case
        )
        require(
            set(data["observations"])
            == (
                {"collector", "units", "network", "artifacts", "tracked_processes"}
                if stage == "stopped"
                else {"collector", "units", "network", "artifacts"}
            )
        )
        require(
            data["provenance"]["collector_pid"]
            == data["observations"]["collector"]["pid"]
        )
        validate_capture(data)
        reader = backend or SystemBackend()
        auth_raw = reader.read(ROOT + "/authorization.json", sealed=True)
        auth = decode(auth_raw)
        require(
            auth["execution"] == "USER_GRANTED_GUEST_CAPTURE"
            and auth["nonce"] == nonce
            and auth["case"] == case
        )
        require(
            hashlib.sha256(auth_raw).hexdigest()
            == data["provenance"]["authorization_sha256"]
        )
        input_raw = reader.read(ROOT + "/input.json", sealed=True)
        require(hashlib.sha256(input_raw).hexdigest() == auth["input_sha256"])
        inputs = decode(input_raw)
        require(
            inputs["source"] == data["provenance"]["source"]
            and inputs["manifest_sha256"] == data["provenance"]["manifest_sha256"]
        )
        return data
    except (
        OSError,
        ValueError,
        TypeError,
        KeyError,
        AttributeError,
        RecursionError,
        Unavailable,
    ):
        return {"schema": 1, "status": "UNAVAILABLE"}
