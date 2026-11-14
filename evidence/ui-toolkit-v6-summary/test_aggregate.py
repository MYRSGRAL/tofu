"""Чистые контроли агрегатора без запуска runtime."""

import runpy
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

MODULE = runpy.run_path(str(Path(__file__).with_name("aggregate.py")))


class AggregationTests(unittest.TestCase):
    def test_nearest_rank(self) -> None:
        quantile = MODULE["quantile"]
        self.assertEqual(quantile([9, 1, 5], 0.5), 5)
        self.assertEqual(quantile([9, 1, 5], 0.95), 9)

    def test_invalid_input_fails_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "контроль"):
            MODULE["check"](False, "контроль")

    def test_manifest_pin_rejects_corruption(self) -> None:
        namespace = MODULE["main"].__globals__
        with patch.dict(namespace, sha=lambda path: "0" * 64):
            with self.assertRaisesRegex(ValueError, "manifest pin"):
                MODULE["main"]()

    def test_replay_matches_reviewed_outputs(self) -> None:
        directory = Path(__file__).parent
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            with patch.dict(MODULE["main"].__globals__, OUT=output):
                MODULE["main"]()
            for name in ("summary.json", "report.ru.md"):
                self.assertEqual(
                    (output / name).read_bytes(), (directory / name).read_bytes()
                )


if __name__ == "__main__":
    unittest.main()
