"""Конечный acceptance gate: catalog/RU/EN и декартово покрытие V2 slots."""

from __future__ import annotations

import json
import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from check_diagnostic_export import ExportError  # noqa: E402
from check_diagnostic_export_v2 import CONTEXTS, SLOTS, export_metadata  # noqa: E402
from check_translations import literal, tokens  # noqa: E402


class AcceptanceClosure(unittest.TestCase):
    def test_catalog_identity_and_exact_locales(self) -> None:
        catalog = json.loads(
            (ROOT / "docs/specs/diagnostic-message-catalog.json").read_text()
        )["messages"]
        by_key = {row["message_key"]: row for row in catalog}
        self.assertEqual(len(by_key), len(catalog), "Повторный ключ")
        qml = (ROOT / "crates/yonik-desktop/qml/Main.qml").read_text()
        body = qml.split("function errorText(key)", 1)[1].split(
            "\n    DesktopBridge", 1
        )[0]
        ts = tokens(body)
        pairs = {}
        for index in range(len(ts) - 6):
            if ts[index + 1 : index + 3] == [":", "["]:
                key = literal(ts[index]) if ts[index].startswith('"') else ts[index]
                pairs[key] = (literal(ts[index + 3]), literal(ts[index + 5]))
        for reason, identity in CONTEXTS.items():
            with self.subTest(reason=reason):
                row = by_key[identity["message_key"]]
                self.assertEqual(row["code"], identity["code"])
                self.assertEqual(pairs[row["message_key"]], (row["ru"], row["en"]))

    def test_every_slot_and_generation_requirement(self) -> None:
        for reason, identity in CONTEXTS.items():
            for slot in SLOTS:
                item = {
                    **{key: identity[key] for key in ("code", "message_key", "stage")},
                    "slot": slot,
                    "reason": reason,
                    "request_id": "18446744073709551615"
                    if slot.endswith("_ack") or slot.startswith("logs_")
                    else None,
                    "generation": "18446744073709551615",
                    "retryable": False,
                    "params": {},
                }
                doc = {"schema_version": 2, "platform": "linux", "errors": [item]}
                for generation in ("18446744073709551615", None):
                    item["generation"] = generation
                    with self.subTest(reason=reason, slot=slot, generation=generation):
                        raw = json.dumps(doc).encode()
                        accepted = slot in identity["slots"] and (
                            generation is not None
                            or not identity["generation_required"]
                        )
                        if accepted:
                            self.assertEqual(json.loads(export_metadata(raw)), doc)
                        else:
                            with self.assertRaises(ExportError):
                                export_metadata(raw)

    def test_source_producer_causes_exist_in_contract(self) -> None:
        # Берём реальные producer ссылки, а не отдельный список enum.
        source = (ROOT / "crates/yonik-desktop/src/source_failure.rs").read_text()
        production = source.split("#[cfg(test)]", 1)[0]
        causes = set(re.findall(r"Cause::([A-Za-z0-9_]+)", production))
        context = (ROOT / "crates/yonik-desktop/src/error_context.rs").read_text()
        self.assertTrue(causes)
        for cause in causes:
            with self.subTest(cause=cause):
                self.assertRegex(context, rf"\b{cause}\b")
                # serde snake_case для существующих Source*/Scheduler* вариантов.
                reason = re.sub(r"(?<!^)(?=[A-Z])", "_", cause).lower()
                self.assertIn(reason, CONTEXTS)
