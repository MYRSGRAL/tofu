"""Синтетические offline capture; никаких compiler/ELF execution."""

import hashlib
import json
import os
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
import n5_capture_receipts as seam
from _p25_capture_producer import retain


class CaptureReceiptTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.retained = self.root / "retained"
        self.retained.mkdir()
        source = self.root / "source"
        source.mkdir()
        build = self.root / "task" / "target"
        (build / "release").mkdir(parents=True)
        self.commit = "a" * 40
        archive = b"synthetic frozen archive bytes"
        (self.retained / "source.tar").write_bytes(archive)
        frozen = {
            "schema": 2,
            "release_id": "synthetic",
            "state": "FROZEN",
            "source": {"commit": self.commit, "tree": "b" * 40},
            "archive_sha256": hashlib.sha256(archive).hexdigest(),
            "recipe": seam.RECIPE,
            "tooling_sha256": {"controller": "c" * 64},
        }
        self.write("frozen.json", frozen)
        elf = struct.pack(
            "<16sHHIQQQIHHHHHH",
            b"\x7fELF\x02\x01\x01",
            3,
            62,
            1,
            0,
            64,
            0,
            0,
            64,
            56,
            1,
            0,
            0,
            0,
        ) + bytes(56)
        for name in seam.RUST:
            (build / "release" / name).write_bytes(elf)
            (self.retained / name).write_bytes(elf)
        commands = [
            {
                "argv": [seam.SLOT, *cmd],
                "cwd": str(source),
                "exit": 0,
                "controlled_env": {},
            }
            for cmd in seam.RECIPE["rust_commands"]
        ]
        manifest = retain(
            source,
            build,
            self.retained,
            frozen,
            commands,
            list(seam.RUST),
            seam.RECIPE["target"],
        )
        marker = {
            **frozen,
            "state": "RUST_CAPTURED_ONLY",
            "producer": "local-release-controller-v1",
            "toolchains": {"rustc": "synthetic", "cargo": "synthetic"},
            "controlled_env": {},
            "commands": commands,
            "elf": {name: seam.elf_header(elf) for name in seam.RUST},
            "build_capture": manifest,
            "rust_lineage": None,
            "final_linux_acceptance": False,
        }
        self.write("rust-captured-only.json", marker)
        self.pins = {name: self.digest(name) for name in (*seam.METADATA, "source.tar")}

    def write(self, name: str, value: dict) -> None:
        (self.retained / name).write_text(json.dumps(value))

    def digest(self, name: str) -> str:
        return hashlib.sha256((self.retained / name).read_bytes()).hexdigest()

    def produce(self) -> tuple[dict, dict]:
        return seam.produce(self.retained, self.pins, self.commit)

    def test_positive_existing_formats(self) -> None:
        receipts, report = self.produce()
        self.assertEqual(report["driver"], "MISSING")
        self.assertEqual(report["status"], "UNREGISTERED")
        self.assertEqual(
            set(receipts["worker-build-receipt.json"]),
            {"schema", "worker_sha256", "source_archive_sha256"},
        )
        for role in seam.ROLES:
            receipt = receipts[role + ".receipt.json"]
            self.assertEqual(
                set(receipt),
                {"schema", "source_commit", "source_archive_sha256", "elf_sha256"},
            )
            self.assertEqual(receipt["elf_sha256"], self.digest(seam.ROLES[role]))
            self.assertEqual(
                report["receipt_sha256"][role + ".receipt.json"],
                hashlib.sha256(seam.encode(receipt)).hexdigest(),
            )

    def test_actualbytes_mismatch(self) -> None:
        (self.retained / "yonik-worker").write_bytes(b"\x7fELF" + bytes(120))
        with self.assertRaises(ValueError):
            self.produce()

    def test_missingclosure(self) -> None:
        marker = json.loads((self.retained / "rust-captured-only.json").read_bytes())
        marker["build_capture"] = None
        self.write("rust-captured-only.json", marker)
        self.pins["rust-captured-only.json"] = self.digest("rust-captured-only.json")
        with self.assertRaises(ValueError):
            self.produce()

    def test_unpinned(self) -> None:
        self.pins.pop("rust-captured-only.json")
        with self.assertRaises(ValueError):
            self.produce()

    def test_stale(self) -> None:
        self.commit = "d" * 40
        with self.assertRaises(ValueError):
            self.produce()

    def test_dup_unknown_schema(self) -> None:
        path = self.retained / "frozen.json"
        original = path.read_text()
        for raw in (
            '{"schema":2,' + original[1:],
            original[:-1] + ',"unknown":0}',
            original.replace('"schema": 2', '"schema": true'),
        ):
            path.write_text(raw)
            self.pins["frozen.json"] = self.digest("frozen.json")
            with self.assertRaises(ValueError):
                self.produce()

    def test_bounds(self) -> None:
        with patch.object(seam, "MAX_METADATA", 8), self.assertRaises(ValueError):
            self.produce()

    def test_nofollow_file_and_ancestor(self) -> None:
        path = self.retained / "source.tar"
        path.rename(self.root / "archive")
        path.symlink_to(self.root / "archive")
        with self.assertRaises(OSError):
            self.produce()
        alias = self.root / "alias"
        alias.symlink_to(self.retained, target_is_directory=True)
        with self.assertRaises(OSError):
            seam.produce(alias, self.pins, self.commit)

    def test_mutation(self) -> None:
        original = seam.read_artifact
        calls = 0

        def changing(*args: object, **kwargs: object) -> tuple[str, bytes]:
            nonlocal calls
            result = original(*args, **kwargs)
            calls += 1
            if calls == 7:
                (self.retained / "frozen.json").write_bytes(b"{}")
            return result

        with (
            patch.object(seam, "read_artifact", side_effect=changing),
            self.assertRaises(ValueError),
        ):
            self.produce()

    def test_pathidentity_same_bytes_replacement(self) -> None:
        original = seam.read_artifact
        calls = 0

        def replacing(*args: object, **kwargs: object) -> tuple[str, bytes]:
            nonlocal calls
            result = original(*args, **kwargs)
            calls += 1
            if calls == 7:
                target = self.retained / "frozen.json"
                replacement = self.retained / "replacement"
                replacement.write_bytes(target.read_bytes())
                replacement.replace(target)
            return result

        with (
            patch.object(seam, "read_artifact", side_effect=replacing),
            self.assertRaises(ValueError),
        ):
            self.produce()

    def test_outputoverwrite(self) -> None:
        receipts, report = self.produce()
        old = Path.cwd()
        try:
            os.chdir(self.root)
            output = self.root / "output"
            seam.emit(output, receipts, report)
            with self.assertRaises(FileExistsError):
                seam.emit(output, receipts, report)
            self.assertEqual(
                (output / "worker-build-receipt.json").read_bytes(),
                seam.encode(receipts["worker-build-receipt.json"]),
            )
            with self.assertRaises(ValueError):
                seam.emit(Path("/etc/forbidden"), receipts, report)
        finally:
            os.chdir(old)


if __name__ == "__main__":
    unittest.main()
