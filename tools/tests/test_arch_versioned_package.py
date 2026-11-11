"""Проверки границы доверия готовых архивов до любых VM effects."""

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "arch_runner", Path(__file__).parents[1] / "arch_clean_vm_build.py"
)
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class ArchiveInputsTest(unittest.TestCase):
    def test_preserves_existing_failed_run(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            marker = root / "versioned-run.json"
            marker.write_text('{"status":"FAIL"}')
            with patch.object(runner, "digest") as digest:
                with self.assertRaises(ValueError):
                    runner.archive_manifest((root / "old", root / "new"), root)
                digest.assert_not_called()
            self.assertEqual(marker.read_text(), '{"status":"FAIL"}')

    def test_renamed_new_archive_is_not_historical_old(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            old = root / "old.pkg.tar.zst"
            new = root / "new.pkg.tar.zst"
            with (
                patch.object(
                    runner,
                    "digest",
                    return_value="a10f0840737b2ababc572fbd432335abfd5c232c59ec0493aa0483d749abafcc",
                ),
                patch.object(runner.subprocess, "check_output") as extract,
            ):
                with self.assertRaises(ValueError):
                    runner.archive_manifest((old, new), root)
                extract.assert_not_called()
                self.assertFalse((root / "versioned-inputs.json").exists())

    def test_rejects_corruption_before_extraction(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            old = root / "old.pkg.tar.zst"
            old.write_bytes(b"corrupted archive")
            with patch.object(runner.subprocess, "check_output") as extract:
                with self.assertRaises(ValueError):
                    runner.archive_manifest((old, old), root)
                extract.assert_not_called()


if __name__ == "__main__":
    unittest.main()
