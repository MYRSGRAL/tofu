"""Закрытый guest-only N5 runner. Импорт и --plan не запускают процессы.

Receipt observer не выдаёт production authority. Исполнение возможно только после
отдельного USER grant и sealing настоящих fixture/driver/observer.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import select
import signal
import subprocess
import time
from pathlib import Path
from typing import Any, Protocol

from fixture import preflight

PACKAGE = Path(__file__).parent
DRIVER = "/usr/lib/yonik-acceptance/n5-guest-driver"
DRIVER_ARGV = (DRIVER, "--receipt-protocol=n5-v1")
FIXTURE = Path("/usr/lib/yonik-acceptance/sealed-guest-fixture-v1.json")
GRANT = Path("/usr/lib/yonik-acceptance/n5-user-grant.json")
SOURCE = "7000c59bca687a53cfa35e45137d0299cd1440f0"
MAX_JSON = 65536
MAX_FRAME = 4096
MAX_TOTAL = 1048576
CASE_SECONDS = 60
CLEANUP_SECONDS = 15
CAMPAIGN_SECONDS = 900
CORES = ("xray", "sing-box")
COUNTERS = {
    "apply_sends",
    "role_sends",
    "admitted",
    "ack",
    "health",
    "revoked",
    "retained",
    "empty",
    "reaped",
    "closed",
}
BINDING = {
    "worker_pid",
    "worker_start",
    "server_pid",
    "server_start",
    "generation",
    "epoch",
    "owner_uid",
    "invocation",
    "session_sha256",
    "nonce_sha256",
    "queue_sha256",
    "topology_sha256",
    "digest",
}
ARTIFACTS = {
    "desktop",
    "helper",
    "worker",
    "source_archive",
    "profile",
    "kernel_notes",
    "xray",
    "sing-box",
    "driver",
}
PREFIX = ("profile", "data4", "issuer", "roles", "attach_ack", "healthy")
# action, ожидаемый stage. await только читает следующий bounded observer receipt.
CASES: dict[str, tuple[tuple[str, str], ...]] = {
    "positive": tuple(
        ("start" if i == 0 else "await", s) for i, s in enumerate(PREFIX)
    ),
    "missing-profile": (("start", "profile_unavailable"),),
    "tampered-profile": (("start", "profile_rejected"),),
    "tampered-report": (("start", "profile"), ("await", "report_rejected")),
    "revoke": tuple(
        ("start" if i == 0 else "await", s) for i, s in enumerate(PREFIX[:4])
    )
    + (("revoke", "revoked"),),
    "cancel-before-handoff": tuple(
        ("start" if i == 0 else "await", s) for i, s in enumerate(PREFIX[:3])
    )
    + (("cancel", "cancelled"),),
    "cancel-after-handoff": tuple(
        ("start" if i == 0 else "await", s) for i, s in enumerate(PREFIX[:4])
    )
    + (("cancel", "cancelled"),),
    "host-loss": tuple(
        ("start" if i == 0 else "await", s) for i, s in enumerate(PREFIX)
    )
    + (("close_desktop_owner", "host_lost"),),
    "unknown-stop": tuple(
        ("start" if i == 0 else "await", s) for i, s in enumerate(PREFIX)
    )
    + (("stop", "unknown"),),
}


class RuntimeFailure(Exception):
    """Driver мог стартовать: исход нельзя объявлять неисполненным."""


class Refused(Exception):
    """Только фиксированный диагностический код, без raw driver bytes."""


def digest(value: Any, size: int = 64) -> bool:
    return (
        isinstance(value, str)
        and re.fullmatch(rf"[0-9a-f]{{{size}}}", value) is not None
        and value != "0" * size
    )


def object_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise Refused("DUPLICATE_FIELD")
        result[key] = value
    return result


def decode(data: bytes, limit: int = MAX_JSON) -> dict[str, Any]:
    if not data or len(data) > limit:
        raise Refused("BYTE_BOUND")
    try:
        value = json.loads(data, object_pairs_hook=object_pairs)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise Refused("JSON_PROTOCOL") from error
    if not isinstance(value, dict):
        raise Refused("JSON_OBJECT")
    return value


def read_json(path: Path) -> dict[str, Any]:
    with path.open("rb") as stream:
        return decode(stream.read(MAX_JSON + 1))


def sha_file(path: Path, limit: int = 512 * 1024 * 1024) -> str:
    # Никакого query/probe. В будущем читаются только exact sealed file inputs.
    digestor = hashlib.sha256()
    count = 0
    with path.open("rb") as stream:
        before = os.fstat(stream.fileno())
        while chunk := stream.read(65536):
            count += len(chunk)
            if count > limit:
                raise Refused("ARTIFACT_BOUND")
            digestor.update(chunk)
        after = os.fstat(stream.fileno())
    if count == 0 or (
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
    ):
        raise Refused("ARTIFACT_CHANGED")
    return digestor.hexdigest()


def authorize(
    authorization: dict[str, Any], fixture: dict[str, Any], grant: dict[str, Any]
) -> None:
    if (
        authorization.get("schema") != "N5_AUTHORIZATION_V1"
        or authorization.get("execution") != "AUTHORIZED"
    ):
        raise Refused("EXECUTION_CLOSED")
    if authorization.get("source_blockers") != []:
        raise Refused("SOURCE_BLOCKED")
    for name in ("runner_sha256", "fixture_sha256", "grant_sha256"):
        if not digest(authorization.get(name)):
            raise Refused("MISSING_PINS")
    # Строгая DATA-схема fixture.py не допускает status/n5_acceptance.
    if preflight(fixture).status != "READY":
        raise Refused("MISSING_FIXTURE")
    if (
        grant.get("schema") != "N5_USER_GRANT_V1"
        or grant.get("explicit_user_runtime_grant") is not True
    ):
        raise Refused("MISSING_USER_GRANT")
    if (
        grant.get("fixture_sha256") != authorization["fixture_sha256"]
        or grant.get("runner_sha256") != authorization["runner_sha256"]
    ):
        raise Refused("GRANT_BINDING")
    if (
        fixture.get("source_commit") != SOURCE
        or authorization.get("source_commit") != SOURCE
        or grant.get("source_commit") != SOURCE
    ):
        raise Refused("SOURCE_BINDING")
    if grant.get("cases") != [f"{core}:{case}" for core in CORES for case in CASES]:
        raise Refused("CASE_GRANT")
    # READY DATA не доказывает after-setup closure и actual observer provenance.
    # Совместимого sealed driver/control binding пока нет; не изобретаем extension.
    raise Refused("MISSING_ACTUAL_DRIVER_AND_FIXTURE")


def environment(uid: int) -> dict[str, str]:
    return {
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "PATH": "/usr/bin",
        "XDG_RUNTIME_DIR": f"/run/user/{uid}",
        "WAYLAND_DISPLAY": "wayland-0",
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path=/run/user/{uid}/bus",
    }


class Driver(Protocol):
    def exchange(
        self, core: str, case: str, action: str, deadline: float
    ) -> dict[str, Any]: ...


class ProcessDriver:
    """Будущий sealed guest observer. Никаких shell/argv/config/env override."""

    def __init__(self, uid: int) -> None:
        self.process = subprocess.Popen(
            DRIVER_ARGV,
            cwd="/",
            env=environment(uid),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        try:
            self.pidfd = os.pidfd_open(self.process.pid)
        except OSError:
            self.process.kill()
            self.process.wait(timeout=5)
            raise Refused("PIDFD_REQUIRED") from None
        assert self.process.stdin is not None and self.process.stdout is not None
        os.set_blocking(self.process.stdin.fileno(), False)
        os.set_blocking(self.process.stdout.fileno(), False)
        self.seq = 0
        self.total = 0

    def exchange(
        self, core: str, case: str, action: str, deadline: float
    ) -> dict[str, Any]:
        self.seq += 1
        request = (
            json.dumps(
                {
                    "schema": 2,
                    "seq": self.seq,
                    "core": core,
                    "case": case,
                    "action": action,
                },
                separators=(",", ":"),
            ).encode()
            + b"\n"
        )
        assert self.process.stdin is not None and self.process.stdout is not None
        writer, reader = self.process.stdin.fileno(), self.process.stdout.fileno()
        while request:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([], [writer], [], remaining)[1]:
                raise Refused("DEADLINE")
            try:
                request = request[os.write(writer, request) :]
            except BlockingIOError:
                continue
        data = bytearray()
        while b"\n" not in data:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([reader], [], [], remaining)[0]:
                raise Refused("DEADLINE")
            chunk = os.read(reader, MAX_FRAME + 1 - len(data))
            if not chunk:
                raise Refused("DRIVER_EOF")
            data.extend(chunk)
            self.total += len(chunk)
            if len(data) > MAX_FRAME or self.total > MAX_TOTAL:
                raise Refused("BYTE_BOUND")
        if data.count(b"\n") != 1 or not data.endswith(b"\n"):
            raise Refused("UNSOLICITED_FRAME")
        receipt = decode(bytes(data[:-1]), MAX_FRAME)
        if (
            type(receipt.get("seq")) is not int
            or receipt.get("seq") != self.seq
            or receipt.get("core") != core
            or receipt.get("case") != case
        ):
            raise Refused("RECEIPT_BINDING")
        return receipt

    def close(self) -> None:
        # Только собственный child pidfd; никакой host/worker PID из receipt.
        try:
            if self.process.poll() is None:
                signal.pidfd_send_signal(self.pidfd, signal.SIGTERM)
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    signal.pidfd_send_signal(self.pidfd, signal.SIGKILL)
                    self.process.wait(timeout=5)
        finally:
            os.close(self.pidfd)
            if self.process.stdin:
                self.process.stdin.close()
            if self.process.stdout:
                self.process.stdout.close()


# Исторические CASES/PREFIX выше сохраняют исходные IDs proposal, не runtime gate.
ACTUAL = {"desktop_system_connected", "trusted_execution_tuple", "tcp_udp_policy",
          "app_owner_policy", "worker_reaped", "cgroup_empty", "network_restored",
          "foreign_resources_preserved"}
INFERRED = {"full_v4_report", "custody_roles_attach_ack"}
UNAVAILABLE = {"tampered_internal_report", "revoke_between_internal_stages"}


def check_receipt(receipt: dict[str, Any], expected: dict[str, Any]) -> None:
    """v2: evidence class не взаимозаменяемы; legacy counters отвергаются.

    expected поступает от отдельно доверенного sealed capture verifier. Совпадение
    JSON само по себе не аутентифицирует capture и не открывает execute().
    """
    if (set(receipt) != {"schema", "seq", "core", "case", "stage", "claims"}
        or type(receipt["schema"]) is not int or receipt["schema"] != 2
        or type(receipt["seq"]) is not int or not 0 < receipt["seq"] < 2**64
        or type(receipt["core"]) is not str or type(receipt["case"]) is not str
        or receipt["core"] not in CORES or receipt["case"] not in CASES
        or receipt["stage"] != "witness"):
        raise Refused("V2_PROTOCOL")
    claims = receipt["claims"]
    if type(claims) is not dict or set(claims) != ACTUAL | INFERRED | UNAVAILABLE:
        raise Refused("CLAIM_SET")
    capture = expected.get("trusted_capture_sha256")
    source = expected.get("executing_source_sha256")
    if not digest(capture) or not digest(source):
        raise Refused("MISSING_TRUSTED_PROVENANCE")
    for name, claim in claims.items():
        if type(claim) is not dict:
            raise Refused("CLAIM_PROTOCOL")
        if name in ACTUAL:
            if (set(claim) != {"class", "capture_sha256", "observation_id"}
                or claim["class"] != "ACTUAL" or claim["capture_sha256"] != capture
                or type(claim["observation_id"]) is not int
                or claim["observation_id"] <= 0
                or expected.get("verified_observations", {}).get(name) != claim["observation_id"]):
                raise Refused("MISSING_ACTUAL_OBSERVATION")
        elif name in INFERRED:
            if (set(claim) != {"class", "source_sha256", "anchor_sha256"}
                or claim["class"] != "SOURCE_INVARIANT"
                or claim["source_sha256"] != source
                or claim["anchor_sha256"] != capture):
                raise Refused("INFERENCE_BINDING")
        elif claim != {"class": "UNAVAILABLE", "reason": "SOURCE_ONLY"}:
            raise Refused("UNAVAILABLE_PROTOCOL")


def run_case(driver: Driver, core: str, case: str, expected: dict[str, Any],
             campaign_deadline: float) -> dict[str, Any]:
    # Недоступные interleaving controls не заменяются synthetic PASS.
    if core not in CORES or case not in CASES:
        raise Refused("CASE_NOT_ALLOWED")
    if case != "positive":
        return {"core": core, "case": case, "status": "UNAVAILABLE", "reason": "SOURCE_ONLY"}
    deadline = min(campaign_deadline, time.monotonic() + CASE_SECONDS)
    if deadline <= time.monotonic():
        return {"core": core, "case": case, "status": "BLOCKED", "reason": "DEADLINE"}
    # Полный witness обязан включать actual cleanup; Disconnected не empty fence.
    failure = False
    stopped = False
    try:
        for action in ("start", "await", "stop"):
            stopped |= action == "stop"
            progress = driver.exchange(core, case, action, deadline)
            if (progress != {"schema": 2, "seq": progress.get("seq"), "core": core,
                             "case": case, "stage": action}
                or type(progress.get("seq")) is not int or progress["seq"] <= 0):
                raise Refused("PROGRESS_PROTOCOL")
    except (Refused, OSError):
        failure = True
    cleanup_deadline = min(campaign_deadline, time.monotonic() + CLEANUP_SECONDS)
    try:
        if not stopped:
            stopped = True
            driver.exchange(core, case, "stop", cleanup_deadline)
        receipt = driver.exchange(core, case, "cleanup", cleanup_deadline)
        if receipt.get("core") != core or receipt.get("case") != case:
            raise Refused("RECEIPT_BINDING")
        check_receipt(receipt, expected)
    except (Refused, OSError):
        return {"core": core, "case": case, "status": "QUARANTINED", "reason": "MISSING_ACTUAL_WITNESS"}
    return {"core": core, "case": case, "status": "FAIL" if failure else "PASS", "reason": "DRIVER_FAILURE" if failure else None}


def execute(
    authorization: dict[str, Any], core: str = "xray", case: str = "positive"
) -> list[dict[str, Any]]:
    # Closed status проверяется ДО чтения любых guest/runtime путей.
    if authorization.get("execution") != "AUTHORIZED":
        raise Refused("EXECUTION_CLOSED")
    if core not in CORES or case not in CASES:
        raise Refused("CASE_NOT_ALLOWED")
    fixture, grant = read_json(FIXTURE), read_json(GRANT)
    authorize(authorization, fixture, grant)
    for path, key in (
        (Path(__file__), "runner_sha256"),
        (FIXTURE, "fixture_sha256"),
        (GRANT, "grant_sha256"),
    ):
        if sha_file(path) != authorization[key]:
            raise Refused("SEALED_INPUT_CHANGED")
    raise Refused("MISSING_ACTUAL_DRIVER_AND_FIXTURE")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--core", choices=CORES)
    parser.add_argument("--case", choices=tuple(CASES))
    args = parser.parse_args(argv)
    if args.execute and (args.core is None or args.case is None):
        parser.error("--execute требует --core и --case")
    authorization = read_json(PACKAGE / "authorization.json")
    if not args.execute:
        print(
            json.dumps(
                {
                    "executed": False,
                    "execution": authorization["execution"],
                    "source_blockers": authorization["source_blockers"],
                    "missing": authorization["missing"],
                    "cases": [f"{core}:{case}" for core in CORES for case in CASES],
                },
                ensure_ascii=False,
            )
        )
        return 0
    try:
        results = execute(authorization, args.core, args.case)
        print(json.dumps({"results": results}, ensure_ascii=False))
        return int(any(r["status"] != "PASS" for r in results))
    except RuntimeFailure:
        print(json.dumps({"status": "QUARANTINED", "executed": True}))
        return 3
    except (Refused, OSError):
        print(json.dumps({"status": "BLOCKED", "executed": False}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
