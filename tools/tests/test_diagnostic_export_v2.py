"""V2: закрытые причины, точные ID и fail-whole при канарейках."""

from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_diagnostic_export import CODES, ExportError  # noqa: E402
from check_diagnostic_export_v2 import CONTEXTS, export_metadata  # noqa: E402


class V2(unittest.TestCase):
    def document(self) -> dict:
        return {
            "schema_version": 2,
            "platform": "linux",
            "errors": [
                {
                    "slot": "connection",
                    **{
                        field: CONTEXTS["data_directory_permissions"][field]
                        for field in ("code", "message_key", "stage")
                    },
                    "reason": "data_directory_permissions",
                    "request_id": None,
                    "generation": None,
                    "retryable": False,
                    "params": {},
                }
            ],
        }

    def test_backend_thread_start_failure_contract(self) -> None:
        reason = "backend_thread_start_failed"
        self.assertEqual(
            CONTEXTS[reason],
            {
                "code": "STARTUP",
                "message_key": "startup",
                "stage": "startup",
                "slots": ["error", "connection"],
                "generation_required": False,
            },
        )
        doc = self.document()
        doc["errors"] = [
            {
                **doc["errors"][0],
                "slot": slot,
                "code": "STARTUP",
                "message_key": "startup",
                "stage": "startup",
                "reason": reason,
            }
            for slot in ("error", "connection")
        ]
        self.assertEqual(json.loads(export_metadata(json.dumps(doc).encode())), doc)
        for slot in ("source", "routing_ack", "logs_read"):
            bad = copy.deepcopy(doc)
            bad["errors"][0]["slot"] = slot
            with self.assertRaises(ExportError):
                export_metadata(json.dumps(bad).encode())

    def test_all_causes(self) -> None:
        for reason, identity in CONTEXTS.items():
            doc = self.document()
            doc["errors"][0].update(
                {field: identity[field] for field in ("code", "message_key", "stage")},
                reason=reason,
            )
            slot = identity["slots"][0]
            doc["errors"][0]["slot"] = slot
            doc["errors"][0]["request_id"] = (
                "1" if slot.endswith("_ack") or slot.startswith("logs_") else None
            )
            doc["errors"][0]["generation"] = (
                "1" if identity["generation_required"] else None
            )
            self.assertEqual(json.loads(export_metadata(json.dumps(doc).encode())), doc)

    def test_whole_rejection(self) -> None:
        doc = self.document()
        for field, value in [
            ("reason", "CANARY\x1b"),
            ("params", {"url": "https://secret:CANARY@example.invalid"}),
            ("stage", "operation"),
            ("slot", "operation_ack"),
            ("extra", 1),
            ("request_id", "01"),
            ("request_id", "18446744073709551616"),
            ("generation", 1),
        ]:
            bad = copy.deepcopy(doc)
            bad["errors"][0][field] = value
            with self.assertRaises(ExportError):
                export_metadata(json.dumps(bad).encode())
        doc["errors"] *= 17
        with self.assertRaises(ExportError):
            export_metadata(json.dumps(doc).encode())

    def test_full_u64_canonical(self) -> None:
        doc = self.document()
        doc["errors"][0].update(
            slot="routing_ack",
            request_id="18446744073709551615",
            generation="7",
            reason="routing_repository_failed",
            code="ROUTING_STORAGE",
            message_key="routing_storage",
            stage="routing_save",
        )
        self.assertIn(
            b'"18446744073709551615"', export_metadata(json.dumps(doc).encode())
        )

    def test_version_unknown_fields_and_duplicate_rejection(self) -> None:
        for field, value in [
            ("schema_version", 1),
            ("schema_version", True),
            ("unknown", "CANARY"),
        ]:
            document = self.document()
            document[field] = value
            with self.assertRaises(ExportError):
                export_metadata(json.dumps(document).encode())
        with self.assertRaises(ExportError):
            export_metadata(
                b'{"schema_version":2,"schema_version":2,"platform":"linux","errors":[]}'
            )

    def test_closed_enum_and_catalog_coverage(self) -> None:
        import re

        root = Path(__file__).resolve().parents[2]
        source = (root / "crates/yonik-desktop/src/error_context.rs").read_text()
        variants = re.findall(
            r"^\s*([A-Z][A-Za-z0-9]*),",
            source.split("pub enum Cause {", 1)[1].split("\n}", 1)[0],
            re.M,
        )
        reasons = {
            re.sub(r"(?<!^)(?=[A-Z])", "_", variant).lower() for variant in variants
        }
        self.assertEqual(reasons, set(CONTEXTS))
        for identity in CONTEXTS.values():
            self.assertEqual(CODES[identity["code"]], identity["message_key"])
