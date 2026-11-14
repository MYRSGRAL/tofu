"""Negative fixtures проверяют отказ на повреждённом framing и несовпадении данных."""

from __future__ import annotations

import importlib.util
import struct
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "reconcile", Path(__file__).with_name("reconcile.py")
)
assert SPEC and SPEC.loader
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class CorpusIntegrity(unittest.TestCase):
    def test_truncated_and_oversized_records_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "synthetic.bin"
            for bad in (
                b"\x01",
                struct.pack("<I", 3) + b"ab",
                struct.pack("<I", 8 * 1024 * 1024 + 1),
            ):
                path.write_bytes(bad)
                with self.assertRaises(ValueError):
                    module.corpus_frames(path)

    def test_single_byte_corruption_cannot_reuse_original_digest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "synthetic.bin"
            path.write_bytes(struct.pack("<I", 3) + b"abc")
            original = module.corpus_frames(path)
            path.write_bytes(struct.pack("<I", 3) + b"abd")
            changed = module.corpus_frames(path)
            self.assertEqual(original["frames"], changed["frames"])
            self.assertNotEqual(original["sha256"], changed["sha256"])
            self.assertNotEqual(
                original["samples"][0]["sha256"], changed["samples"][0]["sha256"]
            )


if __name__ == "__main__":
    unittest.main()
