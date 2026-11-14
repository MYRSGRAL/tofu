"""Будущая sealed-подготовка N5. Импорт модуля не запускает runtime."""

from __future__ import annotations

import hashlib
import json
import os
import secrets
import select
import signal
import stat
import subprocess
import time
from pathlib import Path
from typing import Any

ROOT = Path("/usr/lib/yonik-acceptance")
INPUT = ROOT / "n5-prepare.json"
GRANT = ROOT / "n5-prepare-grant.json"
HOME = Path("/run/yonik-acceptance")
HOST = str(ROOT / "startup-host")
DRIVER = str(ROOT / "n5-guest-driver")
PYTHON = "/usr/bin/python3"
BUS = "/usr/bin/dbus-daemon"
SS = "/usr/bin/gnome-keyring-daemon"
BUSCTL = "/usr/bin/busctl"
CONFIG = ROOT / "n5-source.conf"
ARCHIVE = ROOT / "product-source.tar"
LIMIT = 262144


class Closed(RuntimeError):
    """Закрытый gate или UNKNOWN; сообщения не содержат секретов."""


def require(value: Any) -> None:
    if not value:
        raise Closed("CLOSED: проверка подготовки")


def strict_json(raw: bytes) -> Any:
    require(len(raw) <= LIMIT)

    def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in items:
            require(key not in result)
            result[key] = value
        return result

    try:
        return json.loads(
            raw, object_pairs_hook=pairs, parse_constant=lambda _: require(False)
        )
    except (ValueError, RecursionError, UnicodeError) as exc:
        raise Closed("CLOSED: JSON") from exc


def fields(obj: Any, names: set[str]) -> None:
    require(type(obj) is dict and set(obj) == names)


def digest(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def sealed(path: Path, maximum: int = LIMIT) -> bytes:
    """Открытие каждого компонента через dirfd: NOFOLLOW, root, без write bits."""
    require(path.is_absolute())
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for index, part in enumerate(path.parts[1:]):
            last = index == len(path.parts) - 2
            child = os.open(
                part,
                os.O_RDONLY | os.O_NOFOLLOW | (0 if last else os.O_DIRECTORY),
                dir_fd=fd,
            )
            os.close(fd)
            fd = child
            info = os.fstat(fd)
            require(info.st_uid == 0 and not info.st_mode & (0o222 if last else 0o022))
            require(stat.S_ISREG(info.st_mode) if last else stat.S_ISDIR(info.st_mode))
        raw = bytearray()
        while len(raw) <= maximum:
            block = os.read(fd, min(65536, maximum + 1 - len(raw)))
            if not block:
                break
            raw.extend(block)
        require(len(raw) <= maximum)
        return bytes(raw)
    finally:
        os.close(fd)


def load(read: Any = sealed, uid: Any = os.getuid) -> dict[str, Any]:
    """Все pins проверяются до создания backend и mutable I/O."""
    raw = read(INPUT)
    data = strict_json(raw)
    fields(
        data,
        {
            "schema",
            "caller_uid",
            "tool_source_commit",
            "product_source_commit",
            "pins",
            "profile_name",
            "rules",
            "max_seconds",
        },
    )
    require(data["schema"] == "N5_PREPARATION_V1")
    require(type(data["caller_uid"]) is int and 0 < data["caller_uid"] == uid())
    require(type(data["max_seconds"]) is int and 1 <= data["max_seconds"] <= 3600)
    for key in ("tool_source_commit", "product_source_commit"):
        require(
            type(data[key]) is str
            and len(data[key]) == 40
            and all(c in "0123456789abcdef" for c in data[key])
        )
    require(type(data["profile_name"]) is str and 0 < len(data["profile_name"]) <= 256)
    grant = strict_json(read(GRANT))
    fields(grant, {"schema", "permission", "input_sha256", "caller_uid"})
    require(
        grant
        == {
            "schema": "N5_PREPARATION_GRANT_V1",
            "permission": "USER_GRANTED_GUEST_PREPARATION",
            "input_sha256": digest(raw),
            "caller_uid": data["caller_uid"],
        }
    )
    paths = {
        HOST,
        DRIVER,
        PYTHON,
        BUS,
        SS,
        BUSCTL,
        str(CONFIG),
        str(ARCHIVE),
        str(ROOT / "n5_guest_prepare.py"),
    }
    fields(data["pins"], paths)
    for path, expected in data["pins"].items():
        require(type(expected) is str and len(expected) == 64)
        require(digest(read(Path(path), 128 * 1024 * 1024)) == expected)
    require(
        digest(read(Path(__file__).absolute(), 128 * 1024 * 1024))
        == data["pins"][str(ROOT / "n5_guest_prepare.py")]
    )
    require(type(data["rules"]) is list and 1 <= len(data["rules"]) <= 16)
    ids = set()
    for rule in data["rules"]:
        fields(
            rule, {"id", "enabled", "executables", "domains", "destinations", "action"}
        )
        require(
            type(rule["id"]) is str
            and 0 < len(rule["id"]) <= 64
            and rule["id"] not in ids
        )
        ids.add(rule["id"])
        require(
            rule["enabled"] is True
            and rule["domains"] == []
            and rule["destinations"] == []
        )
        require(
            rule["executables"] in ([DRIVER], [PYTHON])
            and rule["action"] in ("direct", "proxy", "block")
        )
    return data


class Child:
    def __init__(self, argv: list[str], env: dict[str, str], deadline: float) -> None:
        self.deadline = deadline
        self.process = subprocess.Popen(
            argv,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        try:
            self.pidfd = os.pidfd_open(self.process.pid)
        except BaseException:
            self.process.kill()
            self.process.wait(timeout=3)
            raise
        self.buffer = bytearray()
        self.total = 0

    def send(self, raw: bytes) -> None:
        require(len(raw) <= 65536 and self.process.stdin is not None)
        fd = self.process.stdin.fileno()
        os.set_blocking(fd, False)
        while raw:
            require(time.monotonic() < self.deadline)
            if select.select(
                [], [fd], [], min(1, max(0, self.deadline - time.monotonic()))
            )[1]:
                count = os.write(fd, raw)
                raw = raw[count:]

    def line(self) -> bytes:
        require(self.process.stdout is not None)
        while b"\n" not in self.buffer:
            require(time.monotonic() < self.deadline)
            if not select.select(
                [self.process.stdout],
                [],
                [],
                min(1, max(0, self.deadline - time.monotonic())),
            )[0]:
                continue
            block = os.read(self.process.stdout.fileno(), 4096)
            require(block)
            self.total += len(block)
            require(self.total <= 8 * LIMIT and len(self.buffer) + len(block) <= LIMIT)
            self.buffer.extend(block)
        line, _, rest = self.buffer.partition(b"\n")
        self.buffer = bytearray(rest)
        return bytes(line)

    def until(self, predicate: Any) -> dict[str, Any]:
        while True:
            state = strict_json(self.line())
            require(type(state) is dict and "dispatch_error" not in state)
            require(not state.get("error") and not state.get("source_error"))
            if predicate(state):
                return state

    def command(self, command: dict[str, Any]) -> None:
        self.send(json.dumps(command, separators=(",", ":")).encode() + b"\n")

    def close(self, graceful: bool = False) -> None:
        try:
            if self.process.stdin:
                self.process.stdin.close()
            if not graceful and self.process.poll() is None:
                signal.pidfd_send_signal(self.pidfd, signal.SIGTERM)
            try:
                code = self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                signal.pidfd_send_signal(self.pidfd, signal.SIGKILL)
                self.process.wait(timeout=3)
                raise Closed("UNKNOWN: cleanup timeout") from None
            require(not graceful or code == 0)
        finally:
            if self.process.stdout:
                self.process.stdout.close()
            os.close(self.pidfd)


class Backend:
    def __init__(self, data: dict[str, Any]) -> None:
        self.data = data
        self.children: list[Child] = []
        self.deadline = time.monotonic() + data["max_seconds"]
        uid = data["caller_uid"]
        self.runtime = Path(f"/run/user/{uid}")
        self.address = f"unix:path={self.runtime}/yonik-n5-private-bus"
        self.env = {
            "PATH": "/usr/bin",
            "LANG": "C.UTF-8",
            "HOME": str(HOME),
            "XDG_RUNTIME_DIR": str(self.runtime),
            "DBUS_SESSION_BUS_ADDRESS": self.address,
        }
        for name in ("CONFIG", "DATA", "CACHE"):
            self.env[f"XDG_{name}_HOME"] = str(HOME / name.lower())

    def spawn(self, argv: list[str]) -> Child:
        child = Child(argv, self.env, self.deadline)
        self.children.append(child)
        return child

    def start(self) -> None:
        for path in (
            self.runtime,
            HOME,
            HOME / "data",
            HOME / "config",
            HOME / "cache",
            HOME / "keyring",
        ):
            fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
            try:
                for part in path.parts[1:]:
                    child = os.open(
                        part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd
                    )
                    os.close(fd)
                    fd = child
                    info = os.fstat(fd)
                    require(
                        info.st_uid in (0, self.data["caller_uid"])
                        and not info.st_mode & 0o022
                    )
                require(
                    info.st_uid == self.data["caller_uid"]
                    and stat.S_IMODE(info.st_mode) == 0o700
                )
            finally:
                os.close(fd)
        require(not any((HOME / "data").iterdir()))
        require(not any((HOME / "keyring").iterdir()))
        require(not os.path.lexists(HOME / "before-import.sqlite"))
        require(not (self.runtime / "yonik-n5-private-bus").exists())
        # Все mutable файлы — исключительно новые, caller-owned, без symlink.
        config = (
            "<busconfig><type>session</type><listen>"
            + self.address
            + '</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
        )
        for path, raw in (
            (HOME / "private-bus.conf", config.encode()),
            (HOME / "startup-fixture.marker", b"N5"),
        ):
            fd = os.open(
                path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600
            )
            with os.fdopen(fd, "wb") as stream:
                stream.write(raw)
        bus = self.spawn(
            [
                BUS,
                "--nofork",
                "--print-address=1",
                "--config-file=" + str(HOME / "private-bus.conf"),
            ]
        )
        require(bus.line().decode().split(",guid=")[0] == self.address)
        keyring = self.spawn(
            [
                SS,
                "--foreground",
                "--unlock",
                "--components=secrets",
                "--control-directory",
                str(HOME / "keyring"),
            ]
        )
        keyring.send(secrets.token_hex(32).encode() + b"\n")
        require(keyring.process.stdin is not None)
        keyring.process.stdin.close()
        # busctl — только metadata owner, никакого GetSecret/activation.
        while True:
            require(time.monotonic() < self.deadline)
            owner = self.spawn(
                [
                    BUSCTL,
                    "--address=" + self.address,
                    "--json=short",
                    "call",
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "GetConnectionUnixProcessID",
                    "s",
                    "org.freedesktop.secrets",
                ]
            )
            try:
                found = strict_json(owner.line())["data"] == [keyring.process.pid]
            except Closed:
                found = False
            self.children.remove(owner)
            owner.close()
            if found:
                break
            require(keyring.process.poll() is None)
            time.sleep(0.05)
        alias = self.spawn(
            [
                BUSCTL,
                "--address=" + self.address,
                "--json=short",
                "call",
                "org.freedesktop.secrets",
                "/org/freedesktop/secrets",
                "org.freedesktop.Secret.Service",
                "ReadAlias",
                "s",
                "default",
            ]
        )
        collection = strict_json(alias.line())["data"][0]
        require(
            type(collection) is str
            and collection.startswith("/org/freedesktop/secrets/collection/")
        )
        self.finish(alias)
        locked = self.spawn(
            [
                BUSCTL,
                "--address=" + self.address,
                "--json=short",
                "get-property",
                "org.freedesktop.secrets",
                collection,
                "org.freedesktop.Secret.Collection",
                "Locked",
            ]
        )
        require(strict_json(locked.line())["data"] is False)
        self.finish(locked)

    def host(self) -> Child:
        return self.spawn([HOST, str(HOME), "/usr/lib/yonik/engines"])

    def finish(self, child: Child) -> None:
        self.children.remove(child)
        child.close(True)

    def close(self) -> None:
        failed = False
        while self.children:
            try:
                self.children.pop().close()
            except Exception:
                failed = True
        if failed:
            raise Closed("UNKNOWN: cleanup")


class PreparedLease:
    """Bus/SS живы до close; receipt не является runtime grant."""

    def __init__(self, backend: Any, receipt: dict[str, Any]) -> None:
        self.backend = backend
        self.receipt = receipt
        self.closed = False

    def __enter__(self) -> PreparedLease:
        return self

    def close(self) -> None:
        if not self.closed:
            self.backend.close()
            self.closed = True

    def __exit__(self, *args: Any) -> None:
        self.close()


def prepare(
    read: Any = sealed, uid: Any = os.getuid, factory: Any = Backend
) -> PreparedLease:
    data = load(read, uid)
    backend = factory(data)
    try:
        backend.start()
        host = backend.host()
        state = host.until(lambda s: s.get("ready") is True)
        require(not state.get("profiles") and not state.get("sources"))
        host.command(
            {"command": "import_file", "name": "N5 owned fixture", "path": str(CONFIG)}
        )
        state = host.until(lambda s: bool(s.get("pending")))
        host.command(
            {
                "command": "maintenance_start",
                "request_id": 90,
                "operation": "backup",
                "destination": str(HOME / "before-import.sqlite"),
                "revision": state["revision"],
            }
        )
        state = host.until(
            lambda s: (
                s.get("maintenance_ack", {}).get("request_id") == 90
                and s["maintenance_ack"].get("phase") in ("complete", "failed")
            )
        )
        require(state["maintenance_ack"]["phase"] == "complete")
        host.command({"command": "approve_refresh"})
        state = host.until(
            lambda s: (
                not s.get("pending") and not s.get("busy") and bool(s.get("profiles"))
            )
        )
        profiles = [p for p in state["profiles"] if p["name"] == data["profile_name"]]
        require(len(profiles) == 1 and len(state["sources"]) == 1)
        profile = profiles[0]["id"]
        source = state["sources"][0]["id"]
        require(
            type(profile) is int and profile > 0 and type(source) is int and source > 0
        )
        host.command(
            {"command": "select_profile", "id": profile, "revision": state["revision"]}
        )
        state = host.until(lambda s: s.get("selected_profile") == profile)
        for request, rule in enumerate(data["rules"], 100):
            revision = state["routing_revision"]
            require(type(revision) is int and revision >= 0)
            host.command(
                {
                    "command": "edit_routing",
                    "expected_revision": revision,
                    "request_id": request,
                    "create": True,
                    "confirm_broad": False,
                    "rule": rule,
                }
            )
            state = host.until(
                lambda s: s.get("routing_ack", {}).get("request_id") == request
            )
            require(
                state["routing_ack"].get("error") is None
                and state["routing_revision"] > revision
            )
        routing = state["routing"]
        revision = state["routing_revision"]
        backend.finish(host)
        reopened = backend.host()
        state = reopened.until(lambda s: s.get("ready") is True)
        require(
            state.get("selected_profile") == profile
            and state.get("source_id") == source
        )
        require(any(p["id"] == profile for p in state["profiles"]))
        require(state["routing"] == routing and state["routing_revision"] == revision)
        backend.finish(reopened)
        receipt = {
            "schema": "N5_PREPARED_IDS_V1",
            "runtime_grant": "CLOSED",
            "source_id": source,
            "profile_id": profile,
            "routing_revision": revision,
            "tool_source_commit": data["tool_source_commit"],
            "product_source_commit": data["product_source_commit"],
            "pins": data["pins"],
        }
        return PreparedLease(backend, receipt)
    except BaseException:
        backend.close()
        raise
