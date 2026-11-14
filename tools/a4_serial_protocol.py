"""Закрытый bounded serial protocol; parser не исполняет guest payload."""

from __future__ import annotations

import json
import re

import a4_serial_projection as projection

PREFIX = b"A4SERIAL1 "
MAX_STREAM = 1024 * 1024
MAX_RECORD = 4096
MAX_RECORDS = 128
MAX_PAYLOAD = 256 * 1024
HASH = re.compile(r"[0-9a-f]{64}")
TOKEN = re.compile(r"[0-9a-f]{32}")
UUID = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}")


def integer(value: object, maximum: int = 2**63 - 1) -> bool:
    return type(value) is int and 0 <= value <= maximum


def exact(value: object, keys: set[str]) -> dict:
    if not isinstance(value, dict) or set(value) != keys:
        raise ValueError("Protocol schema")
    return value


def checked_payload(kind: str, payload: object) -> dict:
    """Валидация с построением нового объекта, без переноса unknown полей."""
    if kind == "start":
        p = exact(payload, {"collector_sha256", "projection_sha256"})
        if not all(isinstance(v, str) and HASH.fullmatch(v) for v in p.values()):
            raise ValueError("Source digest")
    elif kind == "identity":
        p = exact(payload, {"pid1", "dmi", "cgroup2", "masks"})
        if p != {"pid1": "systemd", "dmi": "QEMU", "cgroup2": True, "masks": 5}:
            raise ValueError("Guest identity")
    elif kind == "properties":
        p = exact(payload, {"unit", "fields", "unknown_fields"})
        if not integer(p["unknown_fields"], 64) or not isinstance(p["fields"], dict):
            raise ValueError("Properties schema")
        allowed = projection.STATES.keys() | projection.RELATIONS | {"Job"}
        if not set(p["fields"]) <= allowed:
            raise ValueError("Unknown property")
        for key, value in p["fields"].items():
            if key in projection.STATES:
                valid = (
                    value in projection.STATES[key] if isinstance(value, str) else False
                )
            elif key in projection.RELATIONS:
                valid = (
                    isinstance(value, list)
                    and len(value) <= 128
                    and all(isinstance(v, str) and v in projection.NAMES for v in value)
                )
            else:
                valid = integer(value, 2**32 - 1)
            if not valid:
                raise ValueError("Property value")
        if p["unit"] not in projection.REQUIRED:
            raise ValueError("Unknown unit")
    elif kind == "definition":
        p = exact(payload, {"unit", "index", "definition"})
        if p["unit"] not in projection.REQUIRED or not integer(p["index"], 4):
            raise ValueError("Definition unit/index")
        d = p["definition"]
        if not isinstance(d, dict) or d.get("unit") != p["unit"]:
            raise ValueError("Definition unit mismatch")
        status = d.get("status")
        red = {
            "RED_INVALID_OR_LIMIT",
            "RED_ENCODING",
            "RED_SYNTAX",
            "RED_LIMIT",
            "RED_READ",
            "MASKED",
        }
        if status in red:
            exact(d, {"unit", "status"})
        elif status == "PROJECTED_NOT_FULL_DEFINITION":
            exact(
                d,
                {
                    "unit",
                    "status",
                    "bytes",
                    "sha256",
                    "relations",
                    "redacted_dependency_tokens",
                    "graph_complete",
                },
            )
            if (
                not integer(d["bytes"], projection.MAX_BYTES)
                or d["bytes"] == 0
                or not isinstance(d["sha256"], str)
                or not HASH.fullmatch(d["sha256"])
                or d["graph_complete"] is not False
                or not integer(d["redacted_dependency_tokens"], projection.MAX_BYTES)
            ):
                raise ValueError("Definition digest/limit")
            if (
                not isinstance(d["relations"], dict)
                or not set(d["relations"]) <= projection.RELATIONS
            ):
                raise ValueError("Definition relations")
            for values in d["relations"].values():
                if (
                    not isinstance(values, list)
                    or len(values) > 128
                    or not all(
                        isinstance(v, str) and v in projection.NAMES for v in values
                    )
                ):
                    raise ValueError("Definition targets")
        else:
            raise ValueError("Definition status")
    elif kind == "jobs":
        p = exact(payload, {"jobs", "foreign_count", "unknown"})
        if (
            not isinstance(p["jobs"], list)
            or len(p["jobs"]) > 128
            or not integer(p["foreign_count"], 4096)
            or type(p["unknown"]) is not bool
        ):
            raise ValueError("Job limits")
        for job in p["jobs"]:
            if not isinstance(job, dict) or projection.project_job(job) != job:
                raise ValueError("Job projection")
    elif kind == "masks":
        p = exact(payload, {"inactive", "unknown"})
        if type(p["inactive"]) is not bool or type(p["unknown"]) is not bool:
            raise ValueError("Masks schema")
    elif kind == "spawn":
        p = exact(payload, {"pid", "starttime"})
        if not integer(p["pid"], 2**31 - 1) or not integer(p["starttime"]):
            raise ValueError("Child identity")
    elif kind == "end":
        p = exact(payload, {"outcome"})
        if p["outcome"] not in {"CAPTURED", "UNKNOWN"}:
            raise ValueError("End status")
    elif kind == "failure":
        p = exact(payload, {"code"})
        if p["code"] not in {
            "GUARD_RED",
            "LIMIT",
            "READ_UNKNOWN",
            "PRODUCT_JOB_FAIL",
            "DEADLINE",
            "CHILD_UNKNOWN",
        }:
            raise ValueError("Failure code")
    else:
        raise ValueError("Unknown record kind")
    return json.loads(json.dumps(p))


def decode_unique(text: str) -> dict:
    def unique(pairs: list[tuple[str, object]]) -> dict:
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("Duplicate JSON key")
            result[key] = value
        return result

    return json.loads(text, object_pairs_hook=unique)


class Stream:
    def __init__(self, token: str, sources: dict[str, str]) -> None:
        if not TOKEN.fullmatch(token):
            raise ValueError("Token format")
        self.token = token
        self.sources = sources
        self.buffer = b""
        self.raw_bytes = 0
        self.payload_bytes = 0
        self.records: list[dict] = []
        self.boot = ""
        self.phase = "initial"
        self.ended: set[str] = set()
        self.units: dict[str, set[str]] = {"initial": set(), "delayed": set()}
        self.kinds: dict[str, set[str]] = {"initial": set(), "delayed": set()}
        self.identities: dict[str, tuple[int, int]] = {}
        self.definitions: set[tuple[str, int]] = set()
        self.child: tuple[int, int] | None = None
        self.last_monotonic = 0

    def feed(self, data: bytes) -> None:
        self.raw_bytes += len(data)
        if self.raw_bytes > MAX_STREAM:
            raise ValueError("Raw serial limit")
        self.buffer += data
        while b"\n" in self.buffer:
            line, self.buffer = self.buffer.split(b"\n", 1)
            if line.startswith(PREFIX):
                self._accept(line.rstrip(b"\r"))
            elif PREFIX in line:
                raise ValueError("Interleaved serial frame")
        if len(self.buffer) > MAX_RECORD and self.buffer.startswith(PREFIX):
            raise ValueError("Frame limit")
        if len(self.buffer) > MAX_STREAM:
            raise ValueError("Serial line limit")

    def _accept(self, line: bytes) -> None:
        if len(line) + 1 > MAX_RECORD or len(self.records) >= MAX_RECORDS:
            raise ValueError("Frame count/size")
        self.payload_bytes += len(line) + 1
        if self.payload_bytes > MAX_PAYLOAD:
            raise ValueError("Payload limit")
        frame = exact(
            decode_unique(line[len(PREFIX) :].decode("utf-8")),
            {
                "seq",
                "phase",
                "monotonic_ns",
                "run_token",
                "boot_id",
                "pid",
                "starttime",
                "kind",
                "payload",
            },
        )
        if (
            frame["run_token"] != self.token
            or frame["seq"] != len(self.records) + 1
            or type(frame["seq"]) is not int
        ):
            raise ValueError("Token/sequence")
        if (
            not isinstance(frame["boot_id"], str)
            or not UUID.fullmatch(frame["boot_id"])
            or (self.boot and frame["boot_id"] != self.boot)
        ):
            raise ValueError("Boot identity")
        for name in ("pid", "starttime", "monotonic_ns"):
            if not integer(frame[name]) or frame[name] == 0:
                raise ValueError("Process/time identity")
        if frame["monotonic_ns"] < self.last_monotonic:
            raise ValueError("Monotonic regression")
        phase = frame["phase"]
        if (
            phase not in {"initial", "delayed"}
            or phase in self.ended
            or (phase == "initial" and self.phase == "delayed")
            or (phase == "delayed" and "initial" not in self.ended)
        ):
            raise ValueError("Phase order")
        kinds = self.kinds[phase]
        kind = frame["kind"]
        if not kinds and kind != "start":
            raise ValueError("Missing START")
        identity = (frame["pid"], frame["starttime"])
        if phase in self.identities and self.identities[phase] != identity:
            raise ValueError("Collector identity changed")
        if phase == "delayed" and identity != self.child:
            raise ValueError("Child identity mismatch")
        payload = checked_payload(kind, frame["payload"])
        if kind == "start" and payload != self.sources:
            raise ValueError("Source binding")
        if (
            kind in {"start", "identity", "jobs", "masks", "spawn", "end"}
            and kind in kinds
        ):
            raise ValueError("Duplicate record")
        if kind == "spawn":
            if phase != "initial":
                raise ValueError("Second child")
            self.child = (payload["pid"], payload["starttime"])
        if kind == "properties":
            if payload["unit"] in self.units[phase]:
                raise ValueError("Duplicate unit")
            self.units[phase].add(payload["unit"])
        if kind == "definition":
            key = (payload["unit"], payload["index"])
            if phase != "initial" or key in self.definitions:
                raise ValueError("Duplicate/delayed definition")
            self.definitions.add(key)
        if (kind == "failure" and payload["code"] == "PRODUCT_JOB_FAIL") or (
            kind == "masks" and not payload["inactive"] and not payload["unknown"]
        ):
            raise ValueError("PRODUCT_JOB_FAIL")
        self.boot = frame["boot_id"]
        self.phase = phase
        self.last_monotonic = frame["monotonic_ns"]
        self.identities[phase] = identity
        kinds.add(kind)
        if kind == "end":
            self.ended.add(phase)
        # Новый public объект никогда не содержит private token.
        self.records.append(
            {
                key: frame[key]
                for key in (
                    "seq",
                    "phase",
                    "monotonic_ns",
                    "boot_id",
                    "pid",
                    "starttime",
                    "kind",
                )
            }
            | {"payload": payload}
        )

    def result(self) -> dict:
        captured = self.ended == {"initial", "delayed"} and self.child is not None
        for phase in ("initial", "delayed"):
            captured &= (
                self.units[phase] == set(projection.REQUIRED)
                and {"identity", "masks", "jobs"} <= self.kinds[phase]
            )
        captured &= {unit for unit, index in self.definitions if index == 0} == set(
            projection.REQUIRED
        )
        unknown = any(
            r["kind"] == "failure"
            or (r["kind"] == "end" and r["payload"]["outcome"] != "CAPTURED")
            for r in self.records
        )
        return {
            "capture": "CAPTURED"
            if captured and not unknown and not self.buffer.startswith(PREFIX)
            else "UNKNOWN",
            "graph_status": "UNKNOWN",
            "records": self.records,
            "fullA4": False,
            "finalLinux": False,
            "factory": "CLOSED",
        }
