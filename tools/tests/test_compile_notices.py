"""Мутации доверенного набора notices."""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
import compile_notices as notices


class NoticesTests(unittest.TestCase):
    def test_real_inventory(self) -> None:
        entries, texts = notices.collect(Path(__file__).parents[2])
        self.assertEqual(len(entries), 225 + 173)
        self.assertTrue(texts)

    def test_mutations(self) -> None:
        root = Path(__file__).parents[2]
        original = notices.read_bounded
        for mutation in (b"", b"wrong", b"x" * (4 * 1024 * 1024 + 1)):
            with self.subTest(size=len(mutation)):

                def read(path: Path, allowed: Path) -> bytes:
                    if path.parent.name == "texts":
                        return mutation
                    return original(path, allowed)

                with patch.object(notices, "read_bounded", read):
                    with self.assertRaises(ValueError):
                        notices.collect(root)

    def test_safe_reader(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "text").symlink_to("/etc/passwd")
            with (root / "oversize").open("wb") as stream:
                stream.truncate(4 * 1024 * 1024 + 1)
            for name in ("text", "missing", "../outside", "oversize"):
                with self.subTest(name=name), self.assertRaises((ValueError, OSError)):
                    notices.read_bounded(root / name, root)

    def test_resource_bytes(self) -> None:
        root = Path(__file__).parents[2]
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            notices.produce(root, output)
            entries = json.loads((output / "index.json").read_text())
            for entry in entries:
                for digest in entry["hashes"]:
                    self.assertEqual(
                        notices.hashlib.sha256(
                            (output / digest).read_bytes()
                        ).hexdigest(),
                        digest,
                    )
