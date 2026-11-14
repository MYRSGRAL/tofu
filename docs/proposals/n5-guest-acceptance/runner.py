"""Закрытый guest-only N5 runner. Импорт и --plan не запускают процессы.

Receipt observer не выдаёт production authority. Исполнение возможно только после
отдельного USER grant, sealing fixture/driver и удаления source blockers review-ом.
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

PACKAGE = Path(__file__).parent
DRIVER = "/usr/lib/yonik-acceptance/n5-guest-driver"
DRIVER_ARGV = (DRIVER, "--receipt-protocol=n5-v1")
FIXTURE = Path("/usr/lib/yonik-acceptance/sealed-guest-fixture-v1.json")
GRANT = Path("/usr/lib/yonik-acceptance/n5-user-grant.json")
SOURCE = "3e3127d30f66ac4d41aa5d5e7ff643e08bef5fb8"
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
    if (
        fixture.get("schema") != "SEALED_GUEST_FIXTURE_V1"
        or fixture.get("status") != "SEALED"
    ):
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
    n5 = fixture.get("n5_acceptance", {})
    if set(n5.get("artifacts", {})) != ARTIFACTS or not all(
        digest(v) for v in n5["artifacts"].values()
    ):
        raise Refused("MISSING_ARTIFACTS")
    if (
        not digest(n5.get("kernel_build_id"), 40)
        or type(n5.get("caller_uid")) is not int
        or not 0 < n5["caller_uid"] < 2**32
    ):
        raise Refused("FIXTURE_IDENTITY")
    if grant.get("cases") != [f"{core}:{case}" for core in CORES for case in CASES]:
        raise Refused("CASE_GRANT")
    controls = n5.get("controls", {})
    if set(controls) != set(CASES) or not all(digest(v) for v in controls.values()):
        raise Refused("MISSING_SEALED_CONTROLS")


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
                    "schema": 1,
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


def check_receipt(
    receipt: dict[str, Any],
    stage: str,
    previous: dict[str, Any] | None,
    expected: dict[str, Any],
) -> dict[str, Any] | None:
    if (
        set(receipt)
        != {
            "schema",
            "seq",
            "core",
            "case",
            "stage",
            "facts",
            "binding",
            "coverage",
            "artifacts",
            "kernel_build_id",
        }
        or type(receipt["schema"]) is not int
        or receipt["schema"] != 1
        or receipt["stage"] != stage
    ):
        raise Refused("STAGE_PROTOCOL")
    facts = receipt["facts"]
    if (
        not isinstance(facts, dict)
        or set(facts) != COUNTERS
        or any(type(v) is not int or v not in (0, 1) for v in facts.values())
    ):
        raise Refused("COUNTER_PROTOCOL")
    if facts["health"] and (
        not facts["ack"] or not facts["admitted"] or facts["revoked"]
    ):
        raise Refused("FALSE_READINESS")
    if (
        facts["ack"]
        and not facts["role_sends"]
        or facts["role_sends"]
        and not facts["admitted"]
    ):
        raise Refused("ROLE_ORDER")
    if facts["admitted"] and facts["apply_sends"] != 1:
        raise Refused("APPLY_ORDER")
    binding = receipt["binding"]
    if binding is not None:
        if not isinstance(binding, dict) or set(binding) != BINDING:
            raise Refused("BINDING_PROTOCOL")
        for key, value in binding.items():
            if key in {"worker_pid", "server_pid", "owner_uid"}:
                valid = type(value) is int and 0 < value < 2**32
            elif key in {"worker_start", "server_start", "generation", "epoch"}:
                valid = type(value) is int and 0 < value < 2**64
            else:
                valid = digest(value, 32 if key == "invocation" else 64)
            if not valid:
                raise Refused("BINDING_PROTOCOL")
        if (
            previous is not None
            and previous != binding
            or binding["owner_uid"] != expected["caller_uid"]
        ):
            raise Refused("CANONICAL_CHANGED")
    elif previous is not None or facts["admitted"]:
        raise Refused("MISSING_CUSTODY")
    if stage in PREFIX:
        if (
            binding is None
            or receipt["artifacts"] != expected["artifacts"]
            or receipt["kernel_build_id"] != expected["kernel_build_id"]
        ):
            raise Refused("ACTUAL_ARTIFACT_BINDING")
    if stage in PREFIX:
        index = PREFIX.index(stage)
        for key, minimum in (
            ("admitted", 2),
            ("role_sends", 3),
            ("ack", 4),
            ("health", 5),
        ):
            if facts[key] != int(index >= minimum):
                raise Refused("STAGE_FACTS")
    if stage in PREFIX[1:]:
        coverage = receipt["coverage"]
        if not isinstance(coverage, dict) or set(coverage) != {
            "baseline",
            "current",
            "errno",
            "receipt_sha256",
            "report_sha256",
        }:
            raise Refused("COVERAGE_PROTOCOL")
        b, c = coverage["baseline"], coverage["current"]
        if (
            type(b) is not int
            or type(c) is not int
            or not 0 <= b < 2**32 - 1
            or c != b + 1
            or not isinstance(coverage["errno"], list)
            or any(type(value) is not int for value in coverage["errno"])
            or coverage["errno"] != [1, 1, 38] + [1] * 16
            or any(not digest(coverage[k]) for k in ("receipt_sha256", "report_sha256"))
        ):
            raise Refused("COVERAGE_MISMATCH")
    if stage in {"profile_unavailable", "profile_rejected", "report_rejected"} and any(
        facts[k] for k in ("admitted", "role_sends", "ack", "health")
    ):
        raise Refused("DENIAL_BYPASS")
    if stage in {"revoked", "cancelled", "host_lost", "unknown"} and (
        not facts["revoked"]
        or facts["health"]
        or not facts["retained"]
        or facts["empty"]
        or facts["closed"]
    ):
        raise Refused("EARLY_RELEASE")
    if (
        stage == "healthy"
        and facts["health"] != 1
        or stage == "attach_ack"
        and facts["ack"] != 1
    ):
        raise Refused("MISSING_ACK_HEALTH")
    if stage == "closed" and (
        not facts["empty"]
        or not facts["reaped"]
        or not facts["closed"]
        or facts["retained"]
        or facts["health"]
    ):
        raise Refused("UNCERTAIN_CLEANUP")
    return binding


def run_case(
    driver: Driver,
    core: str,
    case: str,
    expected: dict[str, Any],
    campaign_deadline: float,
) -> dict[str, Any]:
    deadline = min(campaign_deadline - CLEANUP_SECONDS, time.monotonic() + CASE_SECONDS)
    binding = None
    if deadline <= time.monotonic():
        return {"core": core, "case": case, "status": "BLOCKED", "reason": "DEADLINE"}
    failure = None
    stopped = False
    try:
        for action, stage in CASES[case]:
            stopped |= action == "stop"
            binding = check_receipt(
                driver.exchange(core, case, action, deadline), stage, binding, expected
            )
    except (Refused, OSError) as error:
        failure = str(error) if isinstance(error, Refused) else "TRANSPORT_FAILURE"
    cleanup_deadline = min(campaign_deadline, time.monotonic() + CLEANUP_SECONDS)
    try:
        # Stop ровно один раз. Unknown не разрешает новый Apply или release owner.
        if not stopped:
            stopped = True
            check_receipt(
                driver.exchange(core, case, "stop", cleanup_deadline),
                "stopping",
                binding,
                expected,
            )
        check_receipt(
            driver.exchange(core, case, "cleanup", cleanup_deadline),
            "closed",
            binding,
            expected,
        )
    except (Refused, OSError):
        return {
            "core": core,
            "case": case,
            "status": "QUARANTINED",
            "reason": failure or "UNCERTAIN_CLEANUP",
        }
    return {
        "core": core,
        "case": case,
        "status": "FAIL" if failure else "PASS",
        "reason": failure,
    }


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
    expected = fixture["n5_acceptance"]
    if sha_file(Path(DRIVER)) != expected["artifacts"]["driver"]:
        raise Refused("DRIVER_CHANGED")
    deadline = time.monotonic() + CAMPAIGN_SECONDS
    driver = None
    try:
        driver = ProcessDriver(expected["caller_uid"])
        # Один case на один owned guest campaign. UNKNOWN никогда не ведёт к Apply G2.
        return [run_case(driver, core, case, expected, deadline)]
    except (Refused, OSError, subprocess.SubprocessError):
        raise RuntimeFailure("DRIVER_OR_CLEANUP_UNKNOWN") from None
    finally:
        if driver is not None:
            try:
                driver.close()
            except (OSError, subprocess.SubprocessError):
                raise RuntimeFailure("DRIVER_REAP_UNKNOWN") from None


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
