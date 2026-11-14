"""Чистые проверки bounded producer без compiler/Qt execution."""

import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import _p25_capture_producer as p


class ProducerTests(unittest.TestCase):
    def test_missing_is_retained(self) -> None:
        tmp_path = Path(self.enterContext(TemporaryDirectory()))
        task = tmp_path / "task"
        task.mkdir()
        source, build = (task / "source", task / "build")
        retained = tmp_path / "retained"
        for root in (source, build, retained):
            root.mkdir()
        (build / "release").mkdir()
        (build / "release/elf").write_bytes(b"fixture")
        frozen = {
            "source": {"commit": "a" * 40, "tree": "b" * 40},
            "archive_sha256": "c" * 64,
        }
        commands = [{"argv": ["slot", "cargo", "build"], "cwd": str(source), "exit": 0}]
        result = p.retain(source, build, retained, frozen, commands, ["elf"], "target")
        self.assertEqual(result["status"], "MISSING")
        self.assertTrue((retained / "build-capture-manifest.json").is_file())
        self.assertTrue(result["missing"])
        self.assertIs(result["final_linux_acceptance"], False)

    def test_failed_command_cannot_be_observed(self) -> None:
        tmp_path = Path(self.enterContext(TemporaryDirectory()))
        with self.assertRaises(ValueError):
            p.retain(tmp_path, tmp_path, tmp_path, {}, [{"exit": 1}], [], "target")

    def test_output_symlink_ancestor_rejected(self) -> None:
        tmp_path = Path(self.enterContext(TemporaryDirectory()))
        actual = tmp_path / "actual"
        actual.mkdir()
        alias = tmp_path / "alias"
        alias.symlink_to(actual, target_is_directory=True)
        with self.assertRaises(OSError):
            p._write(alias / "record.json", {"schema": 1})
        self.assertEqual(list(actual.iterdir()), [])

    def test_output_cap_before_creation(self) -> None:
        tmp_path = Path(self.enterContext(TemporaryDirectory()))
        with self.assertRaises(ValueError):
            p._write(tmp_path / "record.json", {"data": "x" * p.MAX_METADATA})
        self.assertFalse((tmp_path / "record.json").exists())
