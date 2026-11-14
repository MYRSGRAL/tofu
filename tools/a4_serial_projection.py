"""Чистая sanitized projection для proposed serial collector; без I/O/exec."""

from __future__ import annotations

import hashlib
import re

REQUIRED = (
    "sshd.service",
    "cloud-init-network.service",
    "cloud-init-main.service",
    "cloud-config.service",
    "cloud-final.service",
    "network.target",
    "network-online.target",
    "time-sync.target",
    "systemd-time-wait-sync.service",
)
# Только имена из reviewed bootstrap scope; неизвестная связь делает граф неполным.
NAMES = frozenset(REQUIRED) | {
    "cloud-init-local.service",
    "cloud-init.target",
    "cloud-config.target",
    "sshdgenkeys.service",
    "systemd-timesyncd.service",
    "basic.target",
    "sysinit.target",
    "shutdown.target",
    "sockets.target",
    "system.slice",
    "dbus.socket",
    "dbus.service",
}
RELATIONS = frozenset(
    {"After", "Before", "Requires", "Wants", "Requisite", "BindsTo", "Conflicts"}
)
STATES = {
    "LoadState": {
        "loaded",
        "not-found",
        "error",
        "bad-setting",
        "masked",
        "merged",
        "stub",
    },
    "ActiveState": {
        "active",
        "reloading",
        "inactive",
        "failed",
        "activating",
        "deactivating",
        "maintenance",
        "refreshing",
    },
    "SubState": {
        "dead",
        "running",
        "exited",
        "start",
        "start-pre",
        "start-post",
        "stop",
        "stop-sigterm",
        "failed",
        "waiting",
        "listening",
    },
    "Result": {
        "success",
        "exit-code",
        "timeout",
        "dependency",
        "signal",
        "resources",
        "start-limit-hit",
        "protocol",
        "assert",
        "condition",
    },
}
MAX_BYTES = 16384


def project_definition(unit: str, data: bytes) -> dict:
    if unit not in REQUIRED:
        raise ValueError("Unit вне required allowlist")
    if not data or len(data) > MAX_BYTES or not data.endswith(b"\n") or b"\0" in data:
        return {"unit": unit, "status": "RED_INVALID_OR_LIMIT"}
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return {"unit": unit, "status": "RED_ENCODING"}
    relations: dict[str, list[str]] = {}
    unknown = 0
    section = ""
    pending = ""
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith(("#", ";")):
            continue
        line = pending + line
        pending = ""
        if line.endswith("\\"):
            pending = line[:-1] + " "
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            continue
        if "=" not in line or not section:
            return {"unit": unit, "status": "RED_SYNTAX"}
        key, value = line.split("=", 1)
        if section != "Unit" or key not in RELATIONS:
            continue
        if not value.strip():
            relations[key] = []
            continue
        values = value.split()
        if len(values) > 128:
            return {"unit": unit, "status": "RED_LIMIT"}
        unknown += sum(name not in NAMES for name in values)
        relations.setdefault(key, []).extend(name for name in values if name in NAMES)
    if pending:
        return {"unit": unit, "status": "RED_SYNTAX"}
    return {
        "unit": unit,
        "status": "PROJECTED_NOT_FULL_DEFINITION",
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "relations": {key: sorted(set(values)) for key, values in relations.items()},
        "redacted_dependency_tokens": unknown,
        "graph_complete": False,
    }


def project_properties(unit: str, raw: dict[str, str]) -> dict:
    if unit not in REQUIRED:
        raise ValueError("Unit вне required allowlist")
    result: dict = {"unit": unit, "fields": {}, "unknown_fields": 0}
    for key in sorted(STATES.keys() | RELATIONS | {"Job"}):
        value = raw.get(key)
        if value is None or len(value) > MAX_BYTES:
            result["unknown_fields"] += 1
        elif key in STATES and value in STATES[key]:
            result["fields"][key] = value
        elif key in RELATIONS:
            names = value.split()
            if len(names) <= 128 and all(name in NAMES for name in names):
                result["fields"][key] = sorted(set(names))
            else:
                result["unknown_fields"] += 1
        elif key == "Job" and re.fullmatch(
            r"[0-9]{1,10}(?: /org/freedesktop/systemd1/job/[0-9]{1,10})?", value
        ):
            result["fields"][key] = int(value.split()[0])
        else:
            result["unknown_fields"] += 1
    return result


def project_job(raw: dict[str, object]) -> dict | None:
    if (
        raw.get("unit") not in REQUIRED
        or type(raw.get("id")) is not int
        or not 0 < raw["id"] < 2**32
        or raw.get("type")
        not in {"start", "stop", "reload", "restart", "verify-active", "nop"}
        or raw.get("state") not in {"waiting", "running"}
    ):
        return None
    return {key: raw[key] for key in ("unit", "id", "type", "state")}
