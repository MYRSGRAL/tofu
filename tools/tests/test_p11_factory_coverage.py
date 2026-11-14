"""Gate закрытых ключей текущих DesktopHost factories и QML errorText."""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def dto_match_keys(source: str) -> set[str]:
    # Сериализация Action не создаёт diagnostic key. Остальные arms проверяем.
    for variant, value in (
        ("Direct", "direct"),
        ("Proxy", "proxy"),
        ("Block", "block"),
    ):
        source = re.sub(rf'\bAction::{variant}\s*=>\s*"{value}"', "", source)
    return set(re.findall(r'=>\s*"([a-z_]+)"', source))


class FactoryCoverage(unittest.TestCase):
    def test_action_serialization_does_not_hide_error_keys(self) -> None:
        source = """
            Action::Direct => "direct",
            Action::Proxy => "proxy",
            Action::Block => "block",
            Failure::Direct => "direct",
            Failure::Other => "missing_diagnostic",
            Action::Direct => "unexpected_action",
        """
        self.assertEqual(
            dto_match_keys(source),
            {"direct", "missing_diagnostic", "unexpected_action"},
        )

    def test_producer_keys_have_closed_catalog(self) -> None:
        """Извлекает литералы фабрик, включая match arms и operation ACK."""
        source = (ROOT / "crates/yonik-desktop/src/backend.rs").read_text()
        catalog = json.loads(
            (ROOT / "docs/specs/diagnostic-message-catalog.json").read_text()
        )
        keys = {item["message_key"] for item in catalog["messages"]}
        factories = set(re.findall(r'DesktopError\("([a-z_]+)"\)', source))
        factories.update(
            re.findall(
                r'(?:error|connection_error|source_error)\s*(?:=|:)\s*Some\("([a-z_]+)"\)',
                source,
            )
        )
        for name in ("map_subscription", "system_error"):
            body = source.split("fn " + name + "(", 1)[1].split("\n}\n", 1)[0]
            factories.update(re.findall(r'=>\s*"([a-z_]+)"', body))
        # Остальные match arms включают probe/review DTO и нейтральные категории.
        neutral = {
            "credentials",
            "endpoint",
            "not_applicable",
            "parameters",
            "payload",
            "reachable",
            "source",
            "xray",
        }
        dto_keys = dto_match_keys(source) - neutral
        self.assertEqual(dto_keys - keys, set())
        # Startup вынесен в приватный модуль, но его operation errors остаются
        # частью того же закрытого catalog/GUI контракта.
        startup = (ROOT / "crates/yonik-desktop/src/startup_application.rs").read_text()
        startup_keys = set(re.findall(r'DesktopError\("([a-z_]+)"\)', startup))
        factories.update(startup_keys)
        self.assertTrue(
            {
                "startup_conflict",
                "startup_binding",
                "startup_timeout",
                "startup_admission",
            }
            <= startup_keys
        )
        editor = (ROOT / "crates/yonik-desktop/src/editor.rs").read_text()
        editor_keys = set(re.findall(r'(?:Err\(|\|_\|\s*)"([a-z_]+)"', editor))
        self.assertEqual(editor_keys - keys, set())
        factories.update(editor_keys)
        self.assertGreaterEqual(len(factories), 30)
        self.assertEqual(factories - keys, set())
        qml = (ROOT / "crates/yonik-desktop/qml/Main.qml").read_text()
        error_text = qml.split("function errorText(key)", 1)[1].split(
            "\n    DesktopBridge", 1
        )[0]
        mapped = set(re.findall(r'^\s*"?([a-z_]+)"?:\s*\[', error_text, re.M))
        # Компактные записи domain/cidr и другие editor keys тоже обязательны.
        mapped.update(re.findall(r"\b([a-z_]+):\s*\[", error_text))
        self.assertEqual(factories - mapped, set())
        for item in catalog["messages"]:
            if item["message_key"].startswith("startup_"):
                self.assertIn(json.dumps(item["ru"], ensure_ascii=False), error_text)
                self.assertIn(json.dumps(item["en"], ensure_ascii=False), error_text)
                self.assertEqual(item["stage"], "operation")
                self.assertFalse(item["retryable"])
        entry = next(
            item for item in catalog["messages"] if item["message_key"] == "catalog"
        )
        self.assertIn(json.dumps(entry["ru"], ensure_ascii=False), error_text)
        self.assertIn(json.dumps(entry["en"], ensure_ascii=False), error_text)
        self.assertIn(
            "The error category is missing from the message catalog.", error_text
        )
