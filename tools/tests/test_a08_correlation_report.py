"""Negative controls проверки сохранённых actual GUI records, не mock GUI."""

from __future__ import annotations

import copy
import json
import unittest
from pathlib import Path

from tools.a08_correlation_report import correlation

RECORDS = (
    Path(__file__).resolve().parents[2]
    / "docs/evidence/a08-correlation/gui-contract-records.json"
)


class CorrelationReportTests(unittest.TestCase):
    def test_actual_records_and_rejected_mutations(self) -> None:
        original = json.loads(RECORDS.read_text())
        correlation(original)
        for phase, key, damaged in (
            ("stale_rejected", "rows", 1),
            ("desktop_after_reopen", "expected", 3),
            ("query_invalidated", "expected", 99),
            ("mode_invalidated", "rows", 1),
            ("older_reply_rejected", "current_expected", 1),
            ("completed", "last_safe_id", 9_007_199_254_740_992),
        ):
            records = copy.deepcopy(original)
            next(r for r in records if r["phase"] == phase)[key] = damaged
            with self.subTest(phase=phase), self.assertRaises(ValueError):
                correlation(records)
        records = copy.deepcopy(original)
        next(r for r in records if r["phase"] == "legacy_shape")["reply"][
            "request_id"
        ] = 0
        with self.assertRaisesRegex(ValueError, "Legacy shape"):
            correlation(records)
        records = [
            r
            for r in original
            if not (r["phase"] == "error_preserved" and r["request_id"] == 7777)
        ]
        with self.assertRaisesRegex(ValueError, "error paths"):
            correlation(records)
