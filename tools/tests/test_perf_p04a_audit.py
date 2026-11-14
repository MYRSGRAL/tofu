"""Negative controls для пересчёта принятого evidence."""

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "audit", Path(__file__).parents[1] / "perf_p04a_audit.py"
)
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


class AuditTests(unittest.TestCase):
    def rows(self) -> list[dict]:
        return [
            {
                "count": 1000,
                "warmup": False,
                "cancelled": False,
                "exit": 0,
                "wall_s": n,
                "cpu_s": n,
                "peak_rss_kib": n,
            }
            for n in (1, 2, 3)
        ]

    def test_sample_variance(self) -> None:
        result = audit.summarize(self.rows())[0]["metrics"]["wall_s"]
        self.assertEqual(result["sample_variance"], 1)
        self.assertEqual(result["median"], 2)

    def test_failed_cancelled_or_missing_repetition_refused(self) -> None:
        for field, value in (("exit", 1), ("cancelled", True)):
            rows = self.rows()
            rows[0][field] = value
            with self.assertRaises(ValueError):
                audit.summarize(rows)
        with self.assertRaises(ValueError):
            audit.summarize(self.rows()[:2])

    def test_warmup_excluded(self) -> None:
        rows = self.rows()
        rows.append({**rows[0], "warmup": True, "wall_s": 1000})
        self.assertEqual(audit.summarize(rows)[0]["metrics"]["wall_s"]["max"], 3)
