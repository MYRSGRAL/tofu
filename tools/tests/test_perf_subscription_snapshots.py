"""Проверка строгого acceptance для resource harness snapshots."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[1]))
from perf_subscription_snapshots import run_child, validate


class AcceptanceTests(unittest.TestCase):
    def test_refusal_and_missing_profiles_fail(self) -> None:
        good = {
            "sources": 20,
            "successful_commits": 80,
            "disk_imported": 4000,
            "staged_imported": 4000,
            "compiled": [4000, 4000],
            "refusal_preserved": True,
            "cancel_probe": {"aborted_future_committed": True},
            "phases": [{"state": {"snapshots": 20, "profiles": 1000, "rss_kib": 10000}}]
            * 4,
        }
        validate(good, 1000)
        for delta in (
            {"compiled": [1, 1]},
            {"successful_commits": 79},
            {
                "phases": [
                    {"state": {"snapshots": 21, "profiles": 1000, "rss_kib": 10000}}
                ]
                * 4
            },
            {"refusal_preserved": False},
        ):
            with self.assertRaises(ValueError):
                validate(good | delta, 1000)


class ChildTests(unittest.TestCase):
    def test_output_limit_and_timeout(self) -> None:
        for script, timeout in [
            ("import sys; sys.stdout.write('x'*100000)", 2),
            ("import time; time.sleep(5)", 0.01),
        ]:
            with self.assertRaises(RuntimeError):
                run_child([sys.executable, "-c", script], b"", timeout)

    def test_child_metrics(self) -> None:
        result = run_child([sys.executable, "-c", "print('{}')"], b"", 2)
        self.assertEqual(result["result"], {})
        self.assertGreater(result["peak_rss_kib"], 0)
