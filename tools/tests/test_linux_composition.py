"""Негативные границы статического composition scan."""

from __future__ import annotations

import importlib.util
import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "composition", Path(__file__).parents[1] / "check_linux_composition.py"
)
assert SPEC and SPEC.loader
composition = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(composition)


def archive(entries: list[tuple[str, bytes, bytes]]) -> bytes:
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w") as output:
        for name, data, kind in entries:
            info = tarfile.TarInfo(name)
            info.type = kind
            info.size = len(data)
            output.addfile(info, io.BytesIO(data))
    return stream.getvalue()


class CompositionTests(unittest.TestCase):
    def test_regular_member(self) -> None:
        self.assertEqual(
            composition.members(archive([("usr/bin/a", b"ELF", tarfile.REGTYPE)])),
            {"usr/bin/a": b"ELF"},
        )

    def test_unsafe_paths(self) -> None:
        for name in ("../escape", "/absolute", "a/../../escape", "a\\b"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                composition.members(archive([(name, b"", tarfile.REGTYPE)]))

    def test_symlink_hardlink_special(self) -> None:
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE):
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                composition.members(archive([("a", b"", kind)]))

    def test_duplicate(self) -> None:
        with self.assertRaises(ValueError):
            composition.members(archive([("a", b"a", tarfile.REGTYPE)] * 2))

    def test_size_limit(self) -> None:
        with patch.object(composition, "MAX_FILE", 1), self.assertRaises(ValueError):
            composition.members(archive([("a", b"ab", tarfile.REGTYPE)]))

    def test_count_limit(self) -> None:
        with patch.object(composition, "MAX_MEMBERS", 0), self.assertRaises(ValueError):
            composition.members(archive([("a", b"", tarfile.REGTYPE)]))

    def test_input_limits_and_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "input").write_bytes(b"abc")
            (root / "link").symlink_to(root / "input")
            (root / "dir").symlink_to(root, target_is_directory=True)
            self.assertEqual(composition.read_file(root / "input"), b"abc")
            with self.assertRaises(ValueError):
                composition.read_file(root / "input", 2)
            for path in (root / "link", root / "dir/input"):
                with self.assertRaises(OSError):
                    composition.read_file(path)

    def test_manifest_tamper_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            output = root / "output.json"
            manifest.write_text(json.dumps({"package_sha256": "подмена"}))
            argv = [
                "scan",
                "--package",
                "unused",
                "--check",
                str(manifest),
                "--output",
                str(output),
            ]
            with (
                patch("sys.argv", argv),
                patch.object(
                    composition, "scan", return_value={"package_sha256": "исходный"}
                ),
                self.assertRaises(ValueError),
            ):
                composition.main()
            self.assertFalse(output.exists())

    def test_parent_path_rejected(self) -> None:
        with self.assertRaises(ValueError):
            composition.read_file(Path("a/../b"))

    def test_hash_changes(self) -> None:
        self.assertNotEqual(composition.sha(b"a"), composition.sha(b"b"))


if __name__ == "__main__":
    unittest.main()
