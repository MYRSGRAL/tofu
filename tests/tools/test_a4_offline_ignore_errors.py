"""Exit0 с -i не является valid definition/complete graph/input SHA proof."""

import importlib
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
reader = importlib.import_module("a4_offline_units_ignore_errors")


class IgnoreErrorsTests(unittest.TestCase):
    def test_false_exit0_failed_required_unit_is_red_and_unknown(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-per-file-") as directory:
            root = Path(directory)
            unit = root / "usr/lib/systemd/system/sshd.service"
            unit.parent.mkdir(parents=True)
            unit.write_text(
                "[Unit]\nAfter=network.target\n[Service]\nExecStart=/usr/bin/sshd\n"
            )
            issue = (
                f"ERROR: zstd frame incomplete\nERROR: copying data for {unit} failed\n"
            )
            result = reader.account_units(root, issue, 0, required={"sshd.service"})
            self.assertEqual(result["files"][0]["status"], "RED_TOOL_ERROR")
            self.assertFalse(result["graph_complete"])
            self.assertEqual(result["graph_status"], "UNKNOWN")
            self.assertNotIn("source_unchanged", result)
            self.assertEqual(result["edges"], [])

    def test_valid_other_unit_edges_preserved_missing_transitive_unknown(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-per-file-") as directory:
            root = Path(directory)
            unit = root / "usr/lib/systemd/system/sshd.service"
            unit.parent.mkdir(parents=True)
            unit.write_text(
                "[Unit]\nAfter=network.target\nWants=sshdgenkeys.service\n[Service]\nExecStart=/usr/bin/sshd\n"
            )
            result = reader.account_units(root, "", 0, required={"sshd.service"})
            self.assertEqual(result["files"][0]["status"], "VALID_FILE")
            self.assertFalse(result["graph_complete"])
            self.assertIn("network.target", result["missing_transitive"])
            self.assertTrue(result["edges"])

    def test_zero_and_truncated_required_units_are_not_definitions(self) -> None:
        for text in ("", "[Unit]\nAfter=network.tar"):
            with (
                self.subTest(text=text),
                tempfile.TemporaryDirectory(prefix="a4-per-file-") as directory,
            ):
                root = Path(directory)
                unit = root / "usr/lib/systemd/system/sshd.service"
                unit.parent.mkdir(parents=True)
                unit.write_text(text)
                result = reader.account_units(root, "", 0, required={"sshd.service"})
                self.assertNotEqual(result["files"][0]["status"], "VALID_FILE")
                self.assertFalse(result["graph_complete"])

    def test_source_immutability_checked_independently_from_tool_exit(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-input-hash-") as directory:
            source = Path(directory) / "source"
            source.write_bytes(b"original")
            expected = reader.offline.sha(source)
            reader.assert_source_hash(source, expected)
            source.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                reader.assert_source_hash(source, expected)

    def test_unattributed_error_cannot_be_complete(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-per-file-") as directory:
            result = reader.account_units(
                Path(directory), "ERROR: truncated tree", 0, required=set()
            )
            self.assertFalse(result["graph_complete"])
            self.assertTrue(result["unattributed_errors"])
