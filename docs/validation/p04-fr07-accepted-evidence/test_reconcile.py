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


class DeclarationIntegrity(unittest.TestCase):
    def test_missing_required_manifest_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileNotFoundError):
                module.hash_manifest_entries(Path(directory), "fr07-manual")

    def test_malformed_or_empty_declarations_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "declarations.sha256"
            for text in ("", "corrupt line\n", "0" * 64 + "  valid\ninvalid\n"):
                path.write_text(text)
                with self.subTest(text=text), self.assertRaises(ValueError):
                    module.entries(path)

    def test_truncated_required_manifest_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "binaries.sha256").write_text("0" * 64 + "  one\n")
            with self.assertRaises(ValueError):
                module.hash_manifest_entries(root, "fr07-manual")

    def test_duplicate_target_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "declarations.sha256"
            path.write_text(("0" * 64 + "  duplicate\n") * 2)
            with self.assertRaises(ValueError):
                module.entries(path)

    def test_source_header_is_exact_and_does_not_hide_bad_lines(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source-binding.txt"
            path.write_text("expected header\n" + "0" * 64 + "  source.rs\n")
            self.assertEqual(len(module.entries(path, header=("expected header",))), 1)
            with self.assertRaises(ValueError):
                module.entries(path, header=("different header",))
            path.write_text("expected header\nmalformed declaration\n")
            with self.assertRaises(ValueError):
                module.entries(path, header=("expected header",))

    def test_valid_manifest_preserves_all_declarations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lines = ["0" * 64 + f"  artifact-{i}\n" for i in range(4)]
            (root / "binaries.sha256").write_text("".join(lines))
            actual = module.hash_manifest_entries(root, "fr07-manual")
            self.assertEqual(len(actual), 4)
            self.assertEqual(actual[-1], ("binaries.sha256", "0" * 64, "artifact-3"))


if __name__ == "__main__":
    unittest.main()
