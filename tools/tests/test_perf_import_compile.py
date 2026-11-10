"""Проверки воспроизводимости и ограничений resource runner."""

import importlib.util
import sys
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "perf", Path(__file__).parents[1] / "perf_import_compile.py"
)
perf = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(perf)


class ResourceTests(unittest.TestCase):
    def test_fixture(self) -> None:
        self.assertEqual(perf.fixture(1000), perf.fixture(1000))
        self.assertEqual(len(perf.fixture(10000).splitlines()), 10000)
        with self.assertRaises(ValueError):
            perf.fixture(10001)

    def test_failure_is_not_success(self) -> None:
        with self.assertRaises(RuntimeError):
            perf.run_child([sys.executable, "-c", "raise SystemExit(2)"], b"", 2)

    def test_cancel_reaps_child(self) -> None:
        result = perf.run_child(
            [sys.executable, "-c", "import time; time.sleep(5)"], b"", 0.02
        )
        self.assertTrue(result["cancelled"])
        self.assertLess(result["wall_s"], 1)
        self.assertEqual(result["exit"], -9)
