"""Снимок lifecycle сравнивает secret storage opaque, без hash ключа/keyring."""

import importlib
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[2] / "tools"
sys.path.insert(0, str(TOOLS))
snapshot = importlib.import_module("a4_nonempty_control").snapshot


class NonemptySnapshotTests(unittest.TestCase):
    def test_keyring_bytes_compared_without_public_digest(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-snapshot-") as directory:
            root = Path(directory)
            for name in ("storage", "config", "data"):
                (root / name).mkdir(mode=0o700)
            (root / "storage/subscriptions.sqlite").write_bytes(b"encrypted-fixture")
            (root / "data/login.keyring").write_bytes(b"opaque-one")
            (root / "keyring-password").write_bytes(b"private")
            before, opaque_before = snapshot(root)
            self.assertNotIn("sha256", before["data/login.keyring"])
            self.assertNotIn("keyring-password", before)
            self.assertIn("sha256", before["storage/subscriptions.sqlite"])
            (root / "data/login.keyring").write_bytes(b"opaque-two")
            after, opaque_after = snapshot(root)
            self.assertEqual(before, after)
            self.assertNotEqual(opaque_before, opaque_after)

    def test_symlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-snapshot-") as directory:
            root = Path(directory)
            for name in ("storage", "config", "data"):
                (root / name).mkdir()
            (root / "storage/unsafe").symlink_to("/etc/passwd")
            with self.assertRaises(AssertionError):
                snapshot(root)
