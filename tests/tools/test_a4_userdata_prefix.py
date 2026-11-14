"""Реальные own hardlinks и отрицательные traversal/symlink cases без package effects."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
from a4_userdata_prefix import own_hardlink


class RuntimeHardlinkTests(unittest.TestCase):
    def test_real_link_preserves_bytes_and_inode(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "usr").mkdir()
            source = root / "usr/source"
            source.write_bytes(b"fixture")
            own_hardlink(root, "usr/source", root / "usr/destination")
            self.assertEqual(
                source.stat().st_ino, (root / "usr/destination").stat().st_ino
            )
            self.assertEqual(source.stat().st_nlink, 2)
            self.assertEqual((root / "usr/destination").read_bytes(), b"fixture")

    def test_traversal_and_foreign_source_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "usr").mkdir()
            (root / "usr/source").write_bytes(b"fixture")
            for source in ("../escape", "/usr/source", "usr/../source", "usr//source"):
                with self.subTest(source=source), self.assertRaises(ValueError):
                    own_hardlink(root, source, root / "usr/destination")
            with self.assertRaises(ValueError):
                own_hardlink(root, "usr/source", root / "usr/../escape")
            with self.assertRaises(ValueError):
                own_hardlink(root, "usr/source", root / "usr/source")
            (root / "usr/alias").symlink_to("source")
            with self.assertRaises(ValueError):
                own_hardlink(root, "usr/alias", root / "usr/destination")
            self.assertEqual((root / "usr/source").read_bytes(), b"fixture")
