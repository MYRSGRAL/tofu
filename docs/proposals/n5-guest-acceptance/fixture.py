"""Закрытая подготовка DATA fixture; модуль не исполняет guest/provisioning."""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import Any

SCHEMA = "SEALED_GUEST_FIXTURE_V1"
RECEIPT = "SEALED_GUEST_RECEIPT_V1"
FILES = {
    "driver": (
        "n5-guest-driver",
        "/usr/lib/yonik-acceptance/n5-guest-driver",
        0o555,
    ),
    "helper": ("yonik-helper", "/usr/lib/yonik/yonik-helper", 0o555),
    "worker": ("yonik-worker", "/usr/lib/yonik/yonik-worker", 0o555),
    "desktop": ("yonik-desktop", "/usr/bin/yonik-desktop", 0o555),
    "core_a": ("sing-box", "/usr/lib/yonik/engines/sing-box", 0o555),
    "core_b": ("xray", "/usr/lib/yonik/engines/xray", 0o555),
}
UNIT_PINS = {
    "yonik-helper.service": "d1fc8b14578e7ed96a68208794bcd4320dbb043c6223fa6088cccb0d05056c4b",
    "yonik-worker.service": "a6669a5c1be21b70a5dc950611881e8fa823976660993840724a4b2ae497e3d0",
    "yonik-worker.socket": "80f5d6aaeb742c0da4ab58944a7977ba4c802cbf5e025e98cb46bb8d7a2d02d4",
    "yonik-worker-check.service": "14966746029044e65691b0529c8a4e849eb04babf4a6185a83c1181047b40424",
    "yonik-worker-check.socket": "4c667e20d0ebc738193f3bdd3b1b0beff7d38930d45c8d86deb879b841a4e904",
}
UNITS = tuple(UNIT_PINS)
COVERAGE_POLICY_SHA256 = (
    "bd5d7474108a2a1e051734ea4c9097d43380341025ed9cd60e0f1442527d2983"
)
POLICY = "yonik-own-namespace-fixed16-v1"
ABI = "linux-native-abi260-ynkch004-ynkns004-v1"
LIMIT = 512 * 1024 * 1024
ELF_LIMIT = 128 * 1024 * 1024


class Missing(ValueError):
    """Недостающие DATA не равны успешно наблюдённым фактам."""


class Invalid(ValueError):
    """Закрытая схема или binding нарушены."""


@dataclass(frozen=True)
class Validation:
    status: str
    reasons: tuple[str, ...]
    manifest_sha256: str


def _object(value: Any, keys: set[str]) -> dict[str, Any]:
    if type(value) is not dict or set(value) - keys:
        raise Invalid("Неизвестные или неверные поля")
    if keys - set(value):
        raise Missing("Недостающие поля: " + ",".join(sorted(keys - set(value))))
    return value


def _number(value: Any, low: int = 1, high: int = 2**64 - 1) -> int:
    if type(value) is not int or not low <= value <= high:
        raise Invalid("Недопустимое целое число")
    return value


def _pin(value: Any, missing: list[str], name: str, length: int = 64) -> None:
    if value is None:
        missing.append(name)
    elif (
        type(value) is not str
        or not re.fullmatch(f"[0-9a-f]{{{length}}}", value)
        or set(value) == {"0"}
    ):
        raise Invalid(f"Неверный pin: {name}")


def canonical(value: Any) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False
    ).encode()


def _unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise Invalid("Duplicate JSON field")
        result[key] = value
    return result


def load(path: Path) -> dict[str, Any]:
    """Только bounded JSON DATA, без разрешения FD/команд/окружения."""
    with path.open("rb") as file:
        raw = file.read(1024 * 1024 + 1)
    if len(raw) > 1024 * 1024:
        raise Invalid("JSON превышает лимит")
    try:
        result = json.loads(
            raw,
            object_pairs_hook=_unique,
            parse_constant=lambda _: (_ for _ in ()).throw(Invalid("JSON constant")),
        )
    except (ValueError, UnicodeError, RecursionError) as error:
        raise Invalid("Некорректный JSON") from error
    if type(result) is not dict:
        raise Invalid("Ожидался JSON object")
    return result


def _profile(value: Any, manifest: dict[str, Any], missing: list[str]) -> None:
    p = _object(
        value,
        {
            "schema",
            "worker_sha256",
            "source_archive_sha256",
            "policy_id",
            "policy_sha256",
            "kernel_release",
            "kernel_build_id",
            "kernel_machine",
            "kernel_abi",
        },
    )
    if (
        type(p["schema"]) is not int
        or p["schema"] != 1
        or p["policy_id"] != POLICY
        or p["kernel_machine"] != "x86_64"
        or p["kernel_abi"] != ABI
    ):
        raise Invalid("Неподдержанный native profile DATA")
    for name, length in [
        ("worker_sha256", 32),
        ("source_archive_sha256", 32),
        ("policy_sha256", 32),
        ("kernel_build_id", 20),
    ]:
        field = p[name]
        if field is None:
            missing.append("profile." + name)
        elif type(field) is not list or len(field) != length or not any(field):
            raise Invalid("Неверный byte-array profile")
        else:
            for byte in field:
                _number(byte, 0, 255)
    if p["kernel_release"] is None:
        missing.append("profile.kernel_release")
    elif not isinstance(p["kernel_release"], str) or not re.fullmatch(
        r"[a-zA-Z0-9._+-]{1,128}", p["kernel_release"]
    ):
        raise Invalid("Неверный kernel release")
    bindings = {
        "worker_sha256": manifest["artifacts"]["worker"]["sha256"],
        "source_archive_sha256": manifest["source_archive_sha256"],
        "kernel_build_id": manifest["kernel_build_id"],
        "policy_sha256": manifest["coverage_policy_sha256"],
    }
    for name, expected in bindings.items():
        if (
            p[name] is not None
            and expected is not None
            and bytes(p[name]).hex() != expected
        ):
            raise Invalid("Profile DATA не связан с immutable inputs")


def _manifest(value: Any) -> list[str]:
    m = _object(
        value,
        {
            "schema",
            "source_commit",
            "source_archive_sha256",
            "guest_base_sha256",
            "guest_provenance_sha256",
            "kernel_build_id",
            "coverage_policy_sha256",
            "artifacts",
            "units",
            "profile",
            "environment",
        },
    )
    if m["schema"] != SCHEMA:
        raise Invalid("Неверная версия fixture")
    missing: list[str] = []
    for name in (
        "source_archive_sha256",
        "guest_base_sha256",
        "guest_provenance_sha256",
        "coverage_policy_sha256",
    ):
        _pin(m[name], missing, name)
    if (
        m["coverage_policy_sha256"] is not None
        and m["coverage_policy_sha256"] != COVERAGE_POLICY_SHA256
    ):
        raise Invalid("Не supported frozen coverage policy DATA")
    _pin(m["source_commit"], missing, "source_commit", 40)
    _pin(m["kernel_build_id"], missing, "kernel_build_id", 40)
    artifacts = _object(m["artifacts"], set(FILES))
    for name, (_, installed, mode) in FILES.items():
        a = _object(
            artifacts[name],
            {
                "sha256",
                "size",
                "source_archive_sha256",
                "source_commit",
                "build_receipt_sha256",
                "installed_path",
                "uid",
                "gid",
                "mode",
            },
        )
        _pin(a["source_commit"], missing, name + ".source_commit", 40)
        for key in ("sha256", "source_archive_sha256", "build_receipt_sha256"):
            _pin(a[key], missing, name + "." + key)
        if a["size"] is None:
            missing.append(name + ".size")
        else:
            _number(a["size"], 64, ELF_LIMIT)
        _number(a["mode"], 0, 0o7777)
        if (
            (a["installed_path"], a["uid"], a["gid"], a["mode"])
            != (installed, 0, 0, mode)
            or type(a["uid"]) is not int
            or type(a["gid"]) is not int
        ):
            raise Invalid("Нарушен installed file contract")
        if name not in ("core_a", "core_b"):
            for key in ("source_archive_sha256", "source_commit"):
                if a[key] is not None and m[key] is not None and a[key] != m[key]:
                    raise Invalid("ELF из другого source archive/commit")
    units = _object(m["units"], set(UNITS))
    for unit in UNITS:
        _pin(units[unit], missing, unit)
        if units[unit] is not None and units[unit] != UNIT_PINS[unit]:
            raise Invalid("Canonical systemd unit pin изменён")
    _profile(m["profile"], m, missing)
    env = _object(
        m["environment"],
        {
            "pid1_sha256",
            "worker_uid",
            "worker_gid",
            "topology_sha256",
            "uid_caps_sha256",
            "cgroup_controllers_sha256",
        },
    )
    for name in (
        "pid1_sha256",
        "topology_sha256",
        "uid_caps_sha256",
        "cgroup_controllers_sha256",
    ):
        _pin(env[name], missing, name)
    for name in ("worker_uid", "worker_gid"):
        if env[name] is None:
            missing.append(name)
        else:
            _number(env[name], 1, 2**32 - 2)
    return missing


def preflight(manifest: dict[str, Any]) -> Validation:
    """READY означает лишь полноту DATA, никогда не future grant."""
    try:
        missing = _manifest(manifest)
        digest = hashlib.sha256(canonical(manifest)).hexdigest()
        return Validation("MISSING" if missing else "READY", tuple(missing), digest)
    except Missing as error:
        return Validation("MISSING", (str(error),), "")
    except (Invalid, ValueError, TypeError, RecursionError) as error:
        return Validation("REJECTED", (str(error),), "")


def _capture(value: Any, expected_sha: str) -> None:
    if type(value) is not str or not 0 < len(value) <= 1024 * 1024:
        raise Invalid("Отсутствует bounded raw capture")
    if hashlib.sha256(value.encode()).hexdigest() != expected_sha:
        raise Invalid("Raw capture не соответствует pinned closure")


def _receipt_values(value: Any) -> None:
    if value is None:
        raise Missing("Отсутствует after-setup observation")
    if type(value) is dict:
        for item in value.values():
            _receipt_values(item)
    elif type(value) is list:
        for item in value:
            _receipt_values(item)
    elif type(value) not in (str, int):
        raise Invalid("Receipt допускает только exact integer/string DATA")


def validate_receipt(
    manifest: dict[str, Any], receipt: dict[str, Any] | None
) -> Validation:
    check = preflight(manifest)
    if check.status != "READY":
        return check
    if receipt is None:
        return Validation("MISSING", ("after_setup receipt",), check.manifest_sha256)
    try:
        _receipt_values(receipt)
        r = _object(
            receipt,
            {
                "schema",
                "manifest_sha256",
                "guest_base_sha256",
                "guest_provenance_sha256",
                "setup_finished_ns",
                "captured_ns",
                "source_archive_sha256",
                "kernel",
                "files",
                "units",
                "profile",
                "profile_registration",
                "pid1",
                "system_bus",
                "identities",
                "cgroup",
                "topology",
            },
        )
        if r["schema"] != RECEIPT or r["manifest_sha256"] != check.manifest_sha256:
            raise Invalid("Receipt другого manifest/schema")
        for name in (
            "guest_base_sha256",
            "guest_provenance_sha256",
            "source_archive_sha256",
            "units",
            "profile",
        ):
            if r[name] != manifest[name]:
                raise Invalid("Receipt input binding mismatch: " + name)
        if _number(r["captured_ns"]) <= _number(r["setup_finished_ns"]):
            raise Invalid("Capture должен следовать setup")
        kernel = _object(r["kernel"], {"build_id", "release", "machine"})
        if kernel != {
            "build_id": manifest["kernel_build_id"],
            "release": manifest["profile"]["kernel_release"],
            "machine": "x86_64",
        }:
            raise Invalid("Guest kernel mismatch")
        files = _object(r["files"], {*FILES, "source", "profile"})
        for name in FILES:
            expected = manifest["artifacts"][name]
            actual = _object(
                files[name],
                {
                    "sha256",
                    "size",
                    "installed_path",
                    "uid",
                    "gid",
                    "mode",
                    "kind",
                    "ancestors",
                },
            )
            for key in ("sha256", "size", "installed_path", "uid", "gid", "mode"):
                if (
                    type(actual[key]) is not type(expected[key])
                    or actual[key] != expected[key]
                ):
                    raise Invalid("Installed ELF receipt mismatch")
            if actual["kind"] != "regular":
                raise Invalid("Symlink/special file запрещён")
            parents = list(Path(expected["installed_path"]).parents)
            if type(actual["ancestors"]) is not list or len(actual["ancestors"]) != len(
                parents
            ):
                raise Invalid("Не полная installed ancestor chain")
            for ancestor, path in zip(actual["ancestors"], parents, strict=True):
                _object(ancestor, {"path", "uid", "gid", "mode", "kind"})
                mode = _number(ancestor["mode"], 0, 0o7777)
                if (
                    ancestor["path"] != str(path)
                    or ancestor["uid"] != 0
                    or ancestor["gid"] != 0
                    or mode & 0o7022
                    or mode & 0o111 != 0o111
                    or type(ancestor["uid"]) is not int
                    or type(ancestor["gid"]) is not int
                    or ancestor["kind"] != "directory"
                ):
                    raise Invalid("Untrusted installed ancestor")
        for name, installed, digest in [
            (
                "source",
                "/usr/lib/yonik/native-source-v1.tar",
                manifest["source_archive_sha256"],
            ),
            (
                "profile",
                "/usr/lib/yonik/native-profile-v1.json",
                hashlib.sha256(canonical(manifest["profile"])).hexdigest(),
            ),
        ]:
            actual = _object(
                files[name], {"installed_path", "sha256", "uid", "gid", "mode", "kind"}
            )
            if actual != {
                "installed_path": installed,
                "sha256": digest,
                "uid": 0,
                "gid": 0,
                "mode": 0o444,
                "kind": "regular",
            }:
                raise Invalid("Native readonly DATA contract mismatch")
        registration = _object(
            r["profile_registration"],
            {"scope", "grant_manifest_sha256", "grant_source_sha256"},
        )
        if registration != {
            "scope": "OWN_GUEST_ONLY",
            "grant_manifest_sha256": check.manifest_sha256,
            "grant_source_sha256": manifest["source_archive_sha256"],
        }:
            raise Invalid("Регистрация не связана с будущим guest-only grant")
        env = manifest["environment"]
        pid1 = _object(r["pid1"], {"pid", "uid", "comm", "exe_sha256", "start"})
        if (
            pid1["pid"] != 1
            or pid1["uid"] != 0
            or pid1["comm"] != "systemd"
            or pid1["exe_sha256"] != env["pid1_sha256"]
        ):
            raise Invalid("PID1 не совпал с trusted guest base")
        _number(pid1["start"])
        bus = _object(
            r["system_bus"], {"address", "uid", "pid", "start", "socket_inode", "owner"}
        )
        if (
            bus["address"] != "unix:path=/run/dbus/system_bus_socket"
            or bus["uid"] != 0
            or not isinstance(bus["owner"], str)
            or not re.fullmatch(r":1\.[0-9]+", bus["owner"])
        ):
            raise Invalid("Не actual system bus")
        for key in ("pid", "start", "socket_inode"):
            _number(bus[key])
        ids = _object(
            r["identities"],
            {"helper_uid", "worker_uid", "worker_gid", "uid_caps_sha256", "capture"},
        )
        if {key: value for key, value in ids.items() if key != "capture"} != {
            "helper_uid": 0,
            "worker_uid": env["worker_uid"],
            "worker_gid": env["worker_gid"],
            "uid_caps_sha256": env["uid_caps_sha256"],
        }:
            raise Invalid("UID/capabilities closure mismatch")
        _capture(ids["capture"], env["uid_caps_sha256"])
        cg = _object(
            r["cgroup"],
            {"version", "helper", "worker", "controllers_sha256", "capture"},
        )
        if {key: value for key, value in cg.items() if key != "capture"} != {
            "version": 2,
            "helper": "/system.slice/yonik-helper.service",
            "worker": "/system.slice/yonik-worker.service",
            "controllers_sha256": env["cgroup_controllers_sha256"],
        }:
            raise Invalid("Cgroup closure mismatch")
        _capture(cg["capture"], env["cgroup_controllers_sha256"])
        topology = _object(
            r["topology"],
            {
                "sha256",
                "network_namespace",
                "pid_namespace",
                "mount_namespace",
                "capture",
            },
        )
        _capture(topology["capture"], env["topology_sha256"])
        if topology["sha256"] != env["topology_sha256"]:
            raise Invalid("Topology closure mismatch")
        for name in ("network_namespace", "pid_namespace", "mount_namespace"):
            pair = topology[name]
            if type(pair) is not list or len(pair) != 2:
                raise Invalid("Namespace identity отсутствует")
            for value in pair:
                _number(value)
        return Validation("READY", (), check.manifest_sha256)
    except Missing as error:
        return Validation("MISSING", (str(error),), check.manifest_sha256)
    except (Invalid, ValueError, TypeError, RecursionError) as error:
        return Validation("REJECTED", (str(error),), check.manifest_sha256)


def _read_file(directory: int, name: str, limit: int = LIMIT) -> bytes:
    fd = os.open(
        name,
        os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK,
        dir_fd=directory,
    )
    try:
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or not 0 < before.st_size <= limit:
            raise Invalid("Неверный staging input")
        chunks: list[bytes] = []
        total = 0
        while chunk := os.read(fd, min(65536, limit + 1 - total)):
            total += len(chunk)
            if total > limit:
                raise Invalid("Staging input слишком велик")
            chunks.append(chunk)
        after = os.fstat(fd)
        if (
            before.st_ino,
            before.st_dev,
            before.st_size,
            before.st_mtime_ns,
            before.st_ctime_ns,
        ) != (
            after.st_ino,
            after.st_dev,
            after.st_size,
            after.st_mtime_ns,
            after.st_ctime_ns,
        ) or total != before.st_size:
            raise Invalid("Staging input изменился во время чтения")
        return b"".join(chunks)
    finally:
        os.close(fd)


def _copy_input(
    directory: int,
    filename: str,
    destination: Path,
    expected_sha: str,
    expected_size: int | None,
    elf: bool = False,
    limit: int = LIMIT,
) -> None:
    fd = os.open(
        filename,
        os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK,
        dir_fd=directory,
    )
    try:
        before = os.fstat(fd)
        limit = ELF_LIMIT if elf else limit
        if not stat.S_ISREG(before.st_mode) or not 0 < before.st_size <= limit:
            raise Invalid("Неверный bounded source input")
        if expected_size is not None and before.st_size != expected_size:
            raise Invalid("Input size mismatch")
        sha = hashlib.sha256()
        total = 0
        with destination.open("xb") as output:
            while chunk := os.read(fd, 65536):
                if (
                    total == 0
                    and elf
                    and (
                        len(chunk) < 64
                        or chunk[:7] != b"\x7fELF\x02\x01\x01"
                        or chunk[18:20] != b"\x3e\x00"
                    )
                ):
                    raise Invalid("Неверный ELF64/x86_64 header")
                total += len(chunk)
                if total > limit:
                    raise Invalid("Input grew past limit")
                sha.update(chunk)
                output.write(chunk)
            after = os.fstat(fd)
            if (
                (
                    before.st_ino,
                    before.st_dev,
                    before.st_size,
                    before.st_mtime_ns,
                    before.st_ctime_ns,
                )
                != (
                    after.st_ino,
                    after.st_dev,
                    after.st_size,
                    after.st_mtime_ns,
                    after.st_ctime_ns,
                )
                or total != before.st_size
                or sha.hexdigest() != expected_sha
            ):
                raise Invalid("Input changed/hash mismatch")
            output.flush()
            os.fchmod(output.fileno(), 0o400)
    finally:
        os.close(fd)


def stage_inputs(
    manifest: dict[str, Any], inputs: Path, destination: Path
) -> Validation:
    """Новый локальный DATA-каталог; partial без seal не является fixture."""
    check = preflight(manifest)
    if check.status != "READY":
        return check
    try:
        fd = os.open(
            inputs, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
        )
    except FileNotFoundError:
        return Validation(
            "MISSING", ("trusted inputs directory",), check.manifest_sha256
        )
    except OSError as error:
        return Validation("REJECTED", (str(error),), check.manifest_sha256)
    try:
        destination.mkdir(mode=0o700)
        _copy_input(
            fd,
            "source.tar",
            destination / "source.tar",
            manifest["source_archive_sha256"],
            None,
        )
        _copy_input(
            fd,
            "guest-base.img",
            destination / "guest-base.img",
            manifest["guest_base_sha256"],
            None,
            limit=16 * 1024 * 1024 * 1024,
        )
        provenance = _read_file(fd, "guest-base.receipt.json", 4096)
        if (
            hashlib.sha256(provenance).hexdigest()
            != manifest["guest_provenance_sha256"]
        ):
            raise Invalid("Guest provenance pin mismatch")
        base_receipt = json.loads(provenance, object_pairs_hook=_unique)
        _object(base_receipt, {"schema", "base_sha256", "origin", "lineage", "state"})
        if base_receipt != {
            "schema": "FRESH_GUEST_BASE_V1",
            "base_sha256": manifest["guest_base_sha256"],
            "origin": "TRUSTED_FRESH_BASE",
            "lineage": "NEW_ACCEPTANCE_N5",
            "state": "NEVER_RUN",
        }:
            raise Invalid("Недопустимая/старая A4 guest base")
        records: dict[str, bytes] = {"guest-base.receipt.json": provenance}
        for name, (filename, _, _) in FILES.items():
            expected = manifest["artifacts"][name]
            _copy_input(
                fd,
                filename,
                destination / filename,
                expected["sha256"],
                expected["size"],
                True,
            )
            raw = _read_file(fd, filename + ".receipt.json", 4096)
            if hashlib.sha256(raw).hexdigest() != expected["build_receipt_sha256"]:
                raise Invalid("Build receipt pin mismatch")
            if name in ("core_a", "core_b"):
                _copy_input(
                    fd,
                    filename + ".source.tar",
                    destination / (filename + ".source.tar"),
                    expected["source_archive_sha256"],
                    None,
                )
            receipt = json.loads(raw, object_pairs_hook=_unique)
            _object(
                receipt,
                {"schema", "source_commit", "source_archive_sha256", "elf_sha256"},
            )
            if receipt != {
                "schema": "SOURCE_BOUND_ELF_V1",
                "source_commit": expected["source_commit"],
                "source_archive_sha256": expected["source_archive_sha256"],
                "elf_sha256": expected["sha256"],
            }:
                raise Invalid("ELF receipt из другого source")
            records[filename + ".receipt.json"] = raw
        for unit in UNITS:
            blob = _read_file(fd, unit, 65536)
            if hashlib.sha256(blob).hexdigest() != manifest["units"][unit]:
                raise Invalid("Canonical unit изменён")
            records[unit] = blob
        records["manifest.json"] = canonical(manifest)
        records["native-profile-v1.json"] = canonical(manifest["profile"])
        # Seal появляется последним, только после полной верификации всех inputs.
        records["SEALED_GUEST_FIXTURE_V1.json"] = canonical(
            {"schema": SCHEMA, "manifest_sha256": check.manifest_sha256}
        )
        for name, blob in records.items():
            with (destination / name).open("xb") as target:
                target.write(blob)
                target.flush()
                os.fchmod(target.fileno(), 0o400)
        return check
    except (FileNotFoundError, Missing):
        return Validation(
            "MISSING",
            ("trusted fresh guest inputs отсутствуют",),
            check.manifest_sha256,
        )
    except (Invalid, OSError, ValueError, TypeError) as error:
        return Validation("REJECTED", (str(error),), check.manifest_sha256)
    finally:
        os.close(fd)


def setup_plan(manifest: dict[str, Any]) -> tuple[dict[str, Any], ...]:
    """Только DATA будущего guest-only grant, без dispatch/exec API."""
    if preflight(manifest).status != "READY":
        return ()
    steps: list[dict[str, Any]] = []

    def install(source: str, target: str, mode: str) -> None:
        steps.append(
            {
                "timeout_seconds": 10,
                "argv": [
                    "/usr/bin/install",
                    "-D",
                    "-o",
                    "0",
                    "-g",
                    "0",
                    "-m",
                    mode,
                    "/run/yonik-n5-inputs/" + source,
                    target,
                ],
            }
        )

    for filename, installed, _ in FILES.values():
        install(filename, installed, "0555")
    install("source.tar", "/usr/lib/yonik/native-source-v1.tar", "0444")
    install("native-profile-v1.json", "/usr/lib/yonik/native-profile-v1.json", "0444")
    for unit in UNITS:
        install(unit, "/usr/lib/systemd/system/" + unit, "0644")
    return tuple(steps)
