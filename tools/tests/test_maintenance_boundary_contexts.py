"""Закрытый контракт причин maintenance boundaries."""

import json
import unittest
from pathlib import Path


class MaintenanceBoundaryContexts(unittest.TestCase):
    def test_closed_matrix(self) -> None:
        matrix = json.loads(
            (
                Path(__file__).resolve().parents[2]
                / "docs/specs/diagnostic-context-v2.json"
            ).read_text()
        )
        for owner in (
            "scheduler_drain",
            "scheduler_admission",
            "service_drain",
            "repository_drain",
            "schedule_reload",
            "schedule_replacement",
            "task",
        ):
            reopen = owner in ("schedule_reload", "schedule_replacement")
            with self.subTest(owner=owner):
                self.assertEqual(
                    matrix[f"maintenance_{owner}_failed"],
                    {
                        "code": "STORAGE" if reopen else "CLEANUP_FAILED",
                        "message_key": "storage" if reopen else "cleanup_failed",
                        "stage": "maintenance_reopen"
                        if reopen
                        else "maintenance_drain",
                        "slots": ["maintenance_ack"],
                        "generation_required": True,
                    },
                )
