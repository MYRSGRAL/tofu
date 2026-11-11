#!/usr/bin/env python3
"""Ограниченный CLI экспорта support metadata v1: stdin JSON → stdout JSON.

Это самостоятельный срез, не конвертер внутренних логов или native-конфигов.
Неизвестные поля отклоняются целиком; текст пользователя не экспортируется.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import NoReturn

MAX_BYTES = 65_536
MAX_DEPTH = 4
MAX_ERRORS = 128
CATALOG = json.loads(
    (
        Path(__file__).resolve().parents[1]
        / "docs/specs/diagnostic-message-catalog.json"
    ).read_text(encoding="utf-8")
)
MESSAGES = {
    item["message_key"]: (item["ru"], item["en"]) for item in CATALOG["messages"]
}
CODES = {item["code"]: item["message_key"] for item in CATALOG["messages"]}


class ExportError(ValueError):
    """Безопасный отказ: сообщение никогда не содержит входные данные."""


def reject() -> NoReturn:
    raise ExportError("Диагностические метаданные отклонены")


def fields(value: object, expected: set[str]) -> dict:
    if type(value) is not dict or value.keys() != expected:
        reject()
    return value


def unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            reject()
        result[key] = value
    return result


def export_metadata(raw: bytes) -> bytes:
    """Проверяет весь документ до выдачи результата, с фиксированными лимитами."""
    if type(raw) is not bytes or len(raw) > MAX_BYTES:
        reject()
    # Ограничение глубины до json.loads, включая документы с неверной схемой.
    depth = 0
    quoted = escaped = False
    for byte in raw:
        if quoted:
            if escaped:
                escaped = False
            elif byte == 92:
                escaped = True
            elif byte == 34:
                quoted = False
        elif byte == 34:
            quoted = True
        elif byte in (91, 123):
            depth += 1
            if depth > MAX_DEPTH:
                reject()
        elif byte in (93, 125):
            depth -= 1
            if depth < 0:
                reject()
    try:
        value = json.loads(
            raw, object_pairs_hook=unique_object, parse_constant=lambda _: reject()
        )
    except (ValueError, UnicodeError, RecursionError):
        reject()
    root = fields(value, {"schema_version", "platform", "errors"})
    if type(root["schema_version"]) is not int or root["schema_version"] != 1:
        reject()
    if root["platform"] != "linux":
        reject()
    errors = root["errors"]
    if type(errors) is not list or len(errors) > MAX_ERRORS:
        reject()
    for item in errors:
        error = fields(item, {"code", "message_key", "stage", "retryable", "params"})
        code = error["code"]
        if type(code) is not str or code not in CODES:
            reject()
        if error["message_key"] != CODES[code]:
            reject()
        if error["stage"] != "operation" or type(error["retryable"]) is not bool:
            reject()
        # Свободные параметры и native cause пока не имеют безопасной схемы.
        fields(error["params"], set())
    result = (
        json.dumps(root, ensure_ascii=True, separators=(",", ":")) + "\n"
    ).encode()
    if len(result) > MAX_BYTES:
        reject()
    return result


def main() -> int:
    try:
        result = export_metadata(sys.stdin.buffer.read(MAX_BYTES + 1))
    except ExportError:
        sys.stderr.write("Диагностические метаданные отклонены\n")
        return 2
    sys.stdout.buffer.write(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
