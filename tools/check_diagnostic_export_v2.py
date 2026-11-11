#!/usr/bin/env python3
"""Строгая самостоятельная проверка versioned V2, без изменения V1."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from check_diagnostic_export import ExportError, fields, unique_object

ROOT = Path(__file__).resolve().parents[1]
CONTEXTS = json.loads((ROOT / "docs/specs/diagnostic-context-v2.json").read_text())
SLOTS = {
    "error",
    "connection",
    "source",
    "routing_ack",
    "maintenance_ack",
    "logs_read",
    "logs_save",
}


def reject() -> None:
    raise ExportError("Диагностические метаданные V2 отклонены")


def decimal(value: object) -> None:
    if value is None:
        return
    if (
        not isinstance(value, str)
        or not value.isascii()
        or not value.isdigit()
        or len(value) > 20
    ):
        reject()
    if str(int(value)) != value or int(value) > 2**64 - 1:
        reject()


def export_metadata(raw: bytes) -> bytes:
    """Отклоняет весь документ при любом нарушении закрытой схемы."""
    if len(raw) > 65536:
        reject()
    try:
        root = json.loads(
            raw, object_pairs_hook=unique_object, parse_constant=lambda _: reject()
        )
    except (ValueError, UnicodeError, RecursionError):
        reject()
    root = fields(root, {"schema_version", "platform", "errors"})
    if (
        type(root["schema_version"]) is not int
        or root["schema_version"] != 2
        or root["platform"] != "linux"
    ):
        reject()
    if type(root["errors"]) is not list or len(root["errors"]) > 16:
        reject()
    seen = set()
    for item in root["errors"]:
        item = fields(
            item,
            {
                "slot",
                "code",
                "message_key",
                "request_id",
                "generation",
                "stage",
                "reason",
                "retryable",
                "params",
            },
        )
        slot = item["slot"]
        reason = item["reason"]
        if (
            type(slot) is not str
            or slot not in SLOTS
            or slot in seen
            or type(reason) is not str
            or reason not in CONTEXTS
        ):
            reject()
        seen.add(slot)
        if slot not in CONTEXTS[reason]["slots"] or (
            CONTEXTS[reason]["generation_required"] and item["generation"] is None
        ):
            reject()
        for field in ("code", "message_key", "stage"):
            if item[field] != CONTEXTS[reason][field]:
                reject()
        if item["retryable"] is not False:
            reject()
        fields(item["params"], set())
        decimal(item["request_id"])
        decimal(item["generation"])
        if (slot in {"routing_ack", "maintenance_ack", "logs_read", "logs_save"}) != (
            item["request_id"] is not None
        ):
            reject()
    result = (
        json.dumps(root, ensure_ascii=True, separators=(",", ":")) + "\n"
    ).encode()
    if len(result) > 65536:
        reject()
    return result


def main() -> int:
    try:
        result = export_metadata(sys.stdin.buffer.read(65537))
    except ExportError:
        sys.stderr.write("Диагностические метаданные V2 отклонены\n")
        return 2
    sys.stdout.buffer.write(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
