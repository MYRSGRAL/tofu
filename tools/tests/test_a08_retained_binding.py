"""Отрицательные контроли допуска retained source без запуска ELF."""

import importlib.util
import io
import tarfile
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "a08", Path(__file__).resolve().parents[1] / "a08_retained_binding.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class BindingTests(unittest.TestCase):
    def test_content_missing_duplicate_and_symlink_are_distinguished(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a08-binding-") as directory:
            root = Path(directory)
            for name in ("equal", "changed", "missing", "duplicate", "link"):
                (root / name).write_bytes(b"current")
            archive = root / "source.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                for name, data in (
                    ("equal", b"current"),
                    ("changed", b"old"),
                    ("duplicate", b"current"),
                    ("duplicate", b"current"),
                ):
                    entry = tarfile.TarInfo(name)
                    entry.size = len(data)
                    output.addfile(entry, io.BytesIO(data))
                link = tarfile.TarInfo("link")
                link.type = tarfile.SYMTYPE
                link.linkname = "equal"
                output.addfile(link)
            differences = MODULE.compare(
                root, archive, ["equal", "changed", "missing", "duplicate", "link"]
            )
            self.assertEqual(
                [row["path"] for row in differences],
                ["changed", "missing", "duplicate", "link"],
            )
            self.assertEqual((root / "equal").read_bytes(), b"current")


if __name__ == "__main__":
    unittest.main()
