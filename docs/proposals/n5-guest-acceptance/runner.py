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
LAUNCH = Path("/usr/lib/yonik-acceptance/n5-trusted-launch.json")
GRANT = Path("/usr/lib/yonik-acceptance/n5-user-grant.json")
SOURCE = "7000c59bca687a53cfa35e45137d0299cd1440f0"
MAX_JSON = 65536
MAX_FRAME = 4194304
MAX_TOTAL = 8388608
CASE_SECONDS = 180
CLEANUP_SECONDS = 45
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


def read_json(path: Path, limit: int = MAX_JSON) -> dict[str, Any]:
    with path.open("rb") as stream:
        return decode(stream.read(limit + 1), limit)


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
    for name in ("runner_sha256", "fixture_sha256", "grant_sha256", "launch_sha256"):
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
        not digest(authorization.get("source_commit"), 40)
        or fixture.get("source_commit") != authorization["source_commit"]
        or grant.get("source_commit") != authorization["source_commit"]
        or grant.get("launch_sha256") != authorization["launch_sha256"]
    ):
        raise Refused("SOURCE_BINDING")
    if grant.get("cases") != [f"{core}:{case}" for core in CORES for case in CASES]:
        raise Refused("CASE_GRANT")


def environment(uid: int) -> dict[str, str]:
    return {
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "PATH": "/usr/bin",
        "XDG_RUNTIME_DIR": f"/run/user/{uid}",
        "WAYLAND_DISPLAY": "wayland-0",
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path=/run/user/{uid}/yonik-n5-private-bus",
    }


class Driver(Protocol):
    def before_start(self, deadline: float) -> None: ...

    def after_await(self, deadline: float) -> None: ...

    def after_close_desktop_owner(self, deadline: float) -> None: ...

    def exchange(
        self, core: str, case: str, action: str, deadline: float
    ) -> dict[str, Any]: ...


class ProcessDriver:
    """Будущий sealed guest observer. Никаких shell/argv/config/env override."""

    def __init__(self, uid: int, gid: int) -> None:
        self.process = subprocess.Popen(
            DRIVER_ARGV,
            cwd="/",
            env=environment(uid),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
            user=uid,
            group=gid,
            extra_groups=(),
            close_fds=True,
        )
        try:
            self.pidfd = os.pidfd_open(self.process.pid)
        except OSError:
            # Без pidfd ownership не подтверждено: child сохраняется owner.
            raise RuntimeFailure("PIDFD_REQUIRED_AFTER_LAUNCH") from None
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
        if select.select([reader], [], [], 0)[0]:
            raise Refused("UNSOLICITED_FRAME_OR_EOF")
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
        except BaseException:
            # Неподтверждённый reap: сохранить pidfd и pipes для quarantine.
            raise
        os.close(self.pidfd)
        if self.process.stdin:
            self.process.stdin.close()
        if self.process.stdout:
            self.process.stdout.close()


# Исторические CASES/PREFIX выше сохраняют исходные IDs proposal, не runtime gate.
ACTUAL = {
    "desktop_system_connected",
    "trusted_execution_tuple",
    "tcp_udp_policy",
    "app_owner_policy",
    "worker_reaped",
    "cgroup_empty",
    "network_restored",
    "foreign_resources_preserved",
}
INFERRED = {"full_v4_report", "custody_roles_attach_ack"}
UNAVAILABLE = {"tampered_internal_report", "revoke_between_internal_stages"}


def _check_receipt(receipt: dict[str, Any], expected: dict[str, Any]) -> None:
    """v2: evidence class не взаимозаменяемы; legacy counters отвергаются.

    expected поступает от отдельно доверенного sealed capture verifier. Совпадение
    JSON само по себе не аутентифицирует capture и не открывает execute().
    """
    if (
        set(receipt)
        != {"schema", "seq", "core", "case", "stage", "claims", "observations"}
        or type(receipt["schema"]) is not int
        or receipt["schema"] != 2
        or type(receipt["seq"]) is not int
        or not 0 < receipt["seq"] < 2**64
        or type(receipt["core"]) is not str
        or type(receipt["case"]) is not str
        or receipt["core"] not in CORES
        or receipt["case"] not in CASES
        or receipt["stage"] != "witness"
    ):
        raise Refused("V2_PROTOCOL")
    observations = receipt["observations"]
    if type(observations) is not dict or set(observations) != {
        "flows",
        "root_captures",
    }:
        raise Refused("WITNESS_OBSERVATIONS")
    witness_sha256 = hashlib.sha256(
        json.dumps(
            observations, sort_keys=True, separators=(",", ":"), ensure_ascii=False
        ).encode()
    ).hexdigest()
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
            if (
                set(claim) != {"class", "capture_sha256", "observation"}
                or claim["class"] != "ACTUAL"
                or claim["capture_sha256"] != capture
            ):
                raise Refused("MISSING_ACTUAL_OBSERVATION")
            observation = claim["observation"]
            if name == "desktop_system_connected":
                if (
                    type(observation) is not dict
                    or observation.get("status") != "Connected"
                    or observation.get("connection_mode") != "system"
                    or observation.get("policy_requires_attribution") is not True
                    or observation.get("selected_profile")
                    != expected["controls"]["profile_id"]
                    or observation.get("source_id") != expected["controls"]["source_id"]
                    or observation.get("engine") != receipt["core"]
                ):
                    raise Refused("NOT_SYSTEM_TUN")
            elif name == "trusted_execution_tuple":
                if observation != {
                    "source_sha256": source,
                    "launch_sha256": capture,
                    "live_verified": True,
                }:
                    raise Refused("PROVENANCE_MISMATCH")
            elif name in {"tcp_udp_policy", "app_owner_policy"}:
                if observation != {"witness_sha256": witness_sha256}:
                    raise Refused("FLOW_COMMITMENT")
                check_flows(observations["flows"], expected["controls"])
            else:
                if observation != {"witness_sha256": witness_sha256}:
                    raise Refused("CLEANUP_COMMITMENT")
                check_cleanup(
                    {"class": "ACTUAL", "root_captures": observations["root_captures"]},
                    expected,
                )
        elif name in INFERRED:
            if (
                set(claim) != {"class", "source_sha256", "anchor_sha256"}
                or claim["class"] != "SOURCE_INVARIANT"
                or claim["source_sha256"] != source
                or claim["anchor_sha256"] != capture
            ):
                raise Refused("INFERENCE_BINDING")
        elif claim != {"class": "UNAVAILABLE", "reason": "SOURCE_ONLY"}:
            raise Refused("UNAVAILABLE_PROTOCOL")


def check_receipt(receipt: dict[str, Any], expected: dict[str, Any]) -> None:
    try:
        _check_receipt(receipt, expected)
    except (KeyError, TypeError, ValueError, AttributeError, IndexError) as error:
        raise Refused("V2_PROTOCOL") from error


def check_cleanup(observation: Any, expected: dict[str, Any]) -> None:
    if (
        type(observation) is not dict
        or set(observation) != {"class", "root_captures"}
        or observation["class"] != "ACTUAL"
    ):
        raise Refused("MISSING_ROOT_CAPTURE")
    captures = observation["root_captures"]
    if type(captures) is not list or len(captures) != 3:
        raise Refused("ROOT_CAPTURE_SET")
    controls = expected["controls"]
    for capture, stage in zip(
        captures, ("baseline", "connected", "stopped"), strict=True
    ):
        provenance = capture.get("provenance", {})
        if (
            capture.get("schema") != 1
            or capture.get("status") != "ACTUAL"
            or provenance.get("source") != expected["source_commit"]
            or provenance.get("manifest_sha256") != expected["manifest_sha256"]
            or provenance.get("stage") != stage
            or provenance.get("nonce") != controls["capture_nonce"]
            or provenance.get("authorization_sha256")
            != controls["capture_authorization_sha256"]
            or provenance.get("case")
            != expected["core"].replace("-", "_") + "_positive"
            or capture.get("observations", {}).get("artifacts") != expected["artifacts"]
        ):
            raise Refused("ROOT_CAPTURE_PROVENANCE")
    before, live, after = [c["observations"] for c in captures]
    network_keys = {"links", "addresses", "routes", "rules", "routes6", "rules6", "nft"}
    for item in (before, live, after):
        network = item.get("network")
        if type(network) is not dict or set(network) != network_keys:
            raise Refused("MISSING_IPV4_IPV6_CAPTURE")
        if any(
            type(values) is not list or any(not digest(value) for value in values)
            for values in network.values()
        ):
            raise Refused("NETWORK_HASH_PROTOCOL")
    if before["network"] != after["network"] or before["network"] == live["network"]:
        raise Refused("ROOT_NETWORK_NOT_RESTORED")
    worker = after["units"]["yonik-worker.service"]
    if (
        worker["main"] is not None
        or worker["members"] != []
        or type(worker["populated"]) is not int
        or worker["populated"] != 0
    ):
        raise Refused("WORKER_NONEMPTY")
    if (
        before["units"]["yonik-helper.service"]
        != after["units"]["yonik-helper.service"]
    ):
        raise Refused("HELPER_NOT_PRESERVED")
    members = live["units"]["yonik-worker.service"]["members"]
    if not members or any(
        not any(
            p.get("pid") == m["pid"]
            and p.get("start_ticks") == m["start_ticks"]
            and p.get("absent") is True
            for p in after["tracked_processes"]
        )
        for m in members
    ):
        raise Refused("MISSING_PROCESS_REAP")


def check_flows(observations: Any, controls: dict[str, Any]) -> None:
    flows = controls["flows"]
    if type(observations) is not list or len(observations) != len(flows):
        raise Refused("FLOW_MATRIX")
    for observed, flow in zip(observations, flows, strict=True):
        if (
            type(observed) is not dict
            or observed.get("flow") != flow
            or observed.get("class") != "ACTUAL"
            or observed.get("child_reaped") is not True
            or type(observed.get("pid")) is not int
            or observed["pid"] <= 0
            or type(observed.get("start")) is not int
            or observed["start"] <= 0
            or observed.get("uid") != controls["caller_uid"]
        ):
            raise Refused("APP_OWNER_OBSERVATION")
        capture, client = observed.get("endpoint_capture"), observed.get("client")
        if type(capture) is not dict or type(client) is not dict:
            raise Refused("FLOW_CAPTURE")
        token = capture.get("token")
        if (
            type(token) is not str
            or not token.startswith(flow["token"] + ":")
            or not digest(token[len(flow["token"]) + 1 :], 32)
            or capture.get("transport") != flow["transport"]
            or capture.get("endpoint") != flow["endpoint"]
            or client.get("remote") != flow["endpoint"]
            or client.get("transport") != flow["transport"]
        ):
            raise Refused("FLOW_BINDING")
        if type(capture.get("seen")) is not int:
            raise Refused("FLOW_COUNTER")
        if flow["action"] == "block":
            if capture["seen"] != 0 or client.get("received") is not False:
                raise Refused("BLOCK_REACHED_ENDPOINT")
        elif (
            capture["seen"] != 1
            or capture.get("server") != flow["response"]
            or capture.get("peer_ip") != flow["expected_peer_ip"]
            or client.get("received") is not True
            or client.get("reply") != token + ":" + flow["response"]
        ):
            raise Refused("FLOW_PATH")


def run_case(
    driver: Driver,
    core: str,
    case: str,
    expected: dict[str, Any],
    campaign_deadline: float,
) -> dict[str, Any]:
    # Недоступные interleaving controls не заменяются synthetic PASS.
    if core not in CORES or case not in CASES:
        raise Refused("CASE_NOT_ALLOWED")
    if case != "positive":
        return {
            "core": core,
            "case": case,
            "status": "UNAVAILABLE",
            "reason": "SOURCE_ONLY",
        }
    deadline = min(campaign_deadline, time.monotonic() + CASE_SECONDS)
    if deadline <= time.monotonic():
        return {"core": core, "case": case, "status": "BLOCKED", "reason": "DEADLINE"}
    # После неизвестного результата команды boundary утрачен: recovery запрещён.
    try:
        for seq, action in enumerate(
            ("start", "await", "stop", "close_desktop_owner"), 1
        ):
            if action == "start":
                driver.before_start(deadline)
            progress = driver.exchange(core, case, action, deadline)
            if (
                progress
                != {
                    "schema": 2,
                    "seq": seq,
                    "core": core,
                    "case": case,
                    "stage": action,
                }
                or type(progress.get("seq")) is not int
            ):
                raise Refused("PROGRESS_PROTOCOL")
            if action == "await":
                driver.after_await(deadline)
            elif action == "close_desktop_owner":
                driver.after_close_desktop_owner(deadline)
        cleanup_deadline = min(deadline, time.monotonic() + CLEANUP_SECONDS)
        receipt = driver.exchange(core, case, "cleanup", cleanup_deadline)
        if receipt.get("seq") != 5:
            raise Refused("RECEIPT_BINDING")
        check_receipt(receipt, expected)
    except (Refused, OSError):
        return {
            "core": core,
            "case": case,
            "status": "QUARANTINED",
            "reason": "MISSING_ACTUAL_WITNESS",
        }
    return {"core": core, "case": case, "status": "PASS", "reason": None}


def execute(
    authorization: dict[str, Any], core: str = "xray", case: str = "positive"
) -> list[dict[str, Any]]:
    if authorization.get("execution") != "AUTHORIZED":
        raise Refused("EXECUTION_CLOSED")
    # Переданный JSON не является root authority; только sealed coordinator.
    from coordinator import run_owned_guest

    return [run_owned_guest(core, case)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--core", choices=CORES)
    parser.add_argument("--case", choices=tuple(CASES))
    args = parser.parse_args(argv)
    if args.execute and (args.core is None or args.case is None):
        parser.error("--execute требует --core и --case")
    try:
        authorization = read_json(
            Path("/usr/lib/yonik-acceptance/n5-authorization.json")
            if args.execute
            else PACKAGE / "authorization.json"
        )
    except (Refused, OSError):
        print(json.dumps({"status": "BLOCKED", "executed": False}))
        return 2
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
