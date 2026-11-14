"""Negative controls для приёма targeted mixed evidence."""

import importlib.util
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[1]))
SPEC = importlib.util.spec_from_file_location(
    "mixed", Path(__file__).parents[1] / "perf_p04a_unadmitted.py"
)
mixed = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mixed)


class EvidenceTests(unittest.TestCase):
    def row(self, status: str = "COMPILED", size: int = 1024) -> dict:
        return {
            "cancelled": False,
            "exit": 0,
            "result": {
                "rules": 1000,
                "domains": 100000,
                "cidrs": 50000,
                "rounds": [
                    {
                        "status": status,
                        "bytes": size,
                        "native_rules": 1003,
                        "native_nodes": 5003,
                        "cancel": [{"ns": 1}] * 3,
                    }
                    for _ in range(4)
                ],
            },
        }

    def test_compiled_within_budget(self) -> None:
        mixed.validate(self.row(), 1000)

    def test_refusal_not_pass1000_and_growth_budget(self) -> None:
        for row in (self.row("REFUSED", 0), self.row(size=4194305)):
            with self.assertRaises(ValueError):
                mixed.validate(row, 1000)

    def test_missing_cancel_and_resource_load_not_accepted(self) -> None:
        row = self.row()
        row["result"]["rounds"][0]["cancel"] = []
        with self.assertRaises(ValueError):
            mixed.validate(row, 1000)
        row = self.row()
        row["result"]["cidrs"] = 1
        with self.assertRaises(ValueError):
            mixed.validate(row, 1000)
