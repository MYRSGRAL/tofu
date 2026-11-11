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


class MainAcceptanceTests(unittest.TestCase):
    def invoke(self, responder: object) -> tuple[list[dict], int]:
        import json
        import tempfile
        from unittest.mock import patch

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "binary"
            binary.write_bytes(b"fixture")
            output = root / "evidence"
            with (
                patch.object(sys, "argv", ["perf", str(binary), str(output)]),
                patch.object(perf, "run_child", side_effect=responder) as child,
            ):
                perf.main()
            return json.loads((output / "runs.json").read_text()), child.call_count

    @staticmethod
    def valid(command: list[str], body: bytes, timeout: float) -> dict:
        if timeout < 1:
            return {"cancelled": True, "exit": -9}
        count, rules = int(command[1]), int(command[2])
        return {
            "cancelled": False,
            "exit": 0,
            "result": {
                "imported": count,
                "compiled": count,
                "app_rules": rules,
                "native_rules": rules + 3,
                "mixed_domain_refused": True,
                "import_ns": 1,
                "compile_ns": 1,
                "policy_ns": 1,
                "config_total_bytes": count * 100,
                "policy_bytes": rules * 100,
            },
        }

    def test_all_timeouts_fail_main(self) -> None:
        with self.assertRaises(RuntimeError):
            self.invoke(lambda *_: {"cancelled": True, "exit": -9})

    def test_one_failed_normal_child_fails_main(self) -> None:
        calls = 0

        def responder(command: list[str], body: bytes, timeout: float) -> dict:
            nonlocal calls
            calls += 1
            if calls == 2:
                return {"cancelled": False, "exit": 2}
            return self.valid(command, body, timeout)

        with self.assertRaises(RuntimeError):
            self.invoke(responder)

    def test_48_valid_runs_and_explicit_cancel(self) -> None:
        rows, calls = self.invoke(self.valid)
        self.assertEqual(len(rows), 48)
        self.assertEqual(calls, 49)
        self.assertTrue(all(not row["cancelled"] and row["exit"] == 0 for row in rows))
