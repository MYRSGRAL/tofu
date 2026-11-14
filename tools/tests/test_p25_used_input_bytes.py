"""Только synthetic filesystem/JSON fixtures; без SDK и subprocess."""

import hashlib
import json
import os
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import _p25_used_input_bytes as p


def encoded(value: dict) -> bytes:
    return json.dumps(value).encode()


def digest(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


class UsedBytesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = self.enterContext(TemporaryDirectory())
        self.root = Path(self.temp)
        self.sdk = self.root / "sdk"
        self.sdk.mkdir()
        self.enterContext(patch.object(p, "SDK_ROOTS", (self.sdk,)))
        self.raw = b"full header\n" * 10000
        for name in ("a.h", "b.h"):
            (self.sdk / name).write_bytes(self.raw)
        h = digest(self.raw)
        self.record = {
            "schema": 1,
            "source": {
                "commit": "a" * 40,
                "tree": "b" * 40,
                "archive_sha256": h,
                "tracked_inputs": ["u"],
            },
            "profile": {"target": "fixture", "commands": [["fixture"]], "features": []},
            "files": [
                {
                    "id": "u",
                    "root": "source",
                    "path": "u.cpp",
                    "kind": "source",
                    "sha256": h,
                },
                *[
                    {"id": n, "root": "sdk", "path": n, "kind": "header", "sha256": h}
                    for n in ("a", "b")
                ],
                {
                    "id": "d",
                    "root": "build",
                    "path": "d.json",
                    "kind": "depfile",
                    "sha256": h,
                },
            ],
            "elfs": [],
            "qt_version": None,
            "units": [{"file": "u", "depfile": "d", "argv": ["fixture"]}],
            "generated": [],
            "primary_refs": [],
        }
        tool = {"path": "/usr/bin/fixture", "sha256": h}
        self.observed = {
            "schema": 1,
            "config": {
                "schema": 1,
                "source": "/fixture/source",
                "build": "/fixture/build",
                "sdk": "/usr",
            },
            "config_sha256": h,
            "input_origins": {str(self.sdk / "a.h"): "a", str(self.sdk / "b.h"): "b"},
            "qt": {
                "schema": 1,
                "query_argv": ["fixture"],
                "version": "1.0.0",
                "exit": 0,
                "tool": tool,
                "stdout": "1.0.0",
                "stderr_sha256": h,
            },
            "records": [
                {
                    "schema": 1,
                    "config_sha256": h,
                    "argv": ["fixture"],
                    "original_argv": ["fixture"],
                    "cwd": "/fixture/source",
                    "compiler": tool,
                    "compiler_id": "compiler",
                    "exit": 0,
                    "unit": "/fixture/source/u.cpp",
                    "depfile": "d",
                    "stdout_sha256": h,
                    "stderr_sha256": h,
                    "raw_depfile_sha256": h,
                    "normalized_depfile": {
                        "schema": 1,
                        "unit": "u",
                        "dependencies": ["u", "a", "b"],
                    },
                }
            ],
        }

    def run_capture(self) -> dict:
        a, b = encoded(self.record), encoded(self.observed)
        return p.retain_used_input_bytes(a, digest(a), b, digest(b), self.root / "out")

    def test_fullbytes_dedup_origins(self) -> None:
        result = self.run_capture()
        self.assertEqual(result["status"], "RETAINED")
        self.assertEqual(len(result["inputs"]), 2)
        self.assertEqual((self.root / "out" / digest(self.raw)).read_bytes(), self.raw)
        self.assertEqual(len(list((self.root / "out").iterdir())), 2)
        self.assertEqual(result["legal_status"], "NOTPROVEN")
        self.assertEqual(result["full_source_closure"], "OPEN")

    def test_inventory_not_used(self) -> None:
        self.observed["records"][0]["normalized_depfile"]["dependencies"] = ["u"]
        self.assertEqual(self.run_capture()["status"], "MISSING")

    def test_rejects_inputs(self) -> None:
        for case in (
            "forbidden",
            "symlink",
            "component",
            "directory",
            "hash",
            "missing",
            "cap",
            "total",
            "count",
            "nested",
            "schema",
            "exclusive",
            "fsync",
        ):
            with self.subTest(case=case):
                # Новый fixture для каждого отказа.
                t = UsedBytesTests()
                t.setUp()
                try:
                    if case == "forbidden":
                        t.observed["input_origins"] = {"/home/private/key": "a"}
                    elif case in ("symlink", "directory", "missing"):
                        (t.sdk / "a.h").unlink()
                        if case == "symlink":
                            (t.sdk / "a.h").symlink_to(t.sdk / "b.h")
                        elif case == "directory":
                            (t.sdk / "a.h").mkdir()
                    elif case == "component":
                        (t.sdk / "link").symlink_to(t.sdk, target_is_directory=True)
                        t.observed["input_origins"] = {str(t.sdk / "link/a.h"): "a"}
                    elif case == "hash":
                        (t.sdk / "a.h").write_bytes(b"changed")
                    elif case == "nested":
                        t.observed["qt"]["tool"]["unknown"] = True
                    elif case == "schema":
                        t.observed["config"]["schema"] = True
                    elif case == "exclusive":
                        (t.root / "out").mkdir()
                    attr = {
                        "cap": "MAX_INPUT",
                        "total": "MAX_BYTES",
                        "count": "MAX_FILES",
                    }.get(case)
                    with self.assertRaises(p.CaptureError):
                        if attr:
                            with patch.object(p, attr, 1):
                                t.run_capture()
                        elif case == "fsync":
                            with patch.object(
                                p.os, "fsync", side_effect=OSError("fixture")
                            ):
                                t.run_capture()
                        else:
                            t.run_capture()
                    self.assertFalse((t.root / "out/manifest.json").exists())
                finally:
                    t.doCleanups()

    def test_mutation_and_path_replacement(self) -> None:
        original = os.read
        changed = False

        def read(fd: int, size: int) -> bytes:
            nonlocal changed
            data = original(fd, size)
            if data and not changed:
                changed = True
                (self.sdk / "a.h").unlink()
                (self.sdk / "a.h").write_bytes(self.raw)
            return data

        with (
            patch.object(p.os, "read", side_effect=read),
            self.assertRaises(p.CaptureError),
        ):
            self.run_capture()

    def test_late_fsync_failure_removes_manifest(self) -> None:
        original = os.fsync

        def sync(fd: int) -> None:
            if (self.root / "out/manifest.json").exists():
                raise OSError("late fixture fsync")
            original(fd)

        with (
            patch.object(p.os, "fsync", side_effect=sync),
            self.assertRaises(p.CaptureError),
        ):
            self.run_capture()
        self.assertFalse((self.root / "out/manifest.json").exists())

    def test_inplace_mutation(self) -> None:
        original = os.read
        changed = False

        def read(fd: int, size: int) -> bytes:
            nonlocal changed
            data = original(fd, size)
            if data and not changed:
                changed = True
                with (self.sdk / "a.h").open("ab") as stream:
                    stream.write(b"mutation")
            return data

        with (
            patch.object(p.os, "read", side_effect=read),
            self.assertRaises(p.CaptureError),
        ):
            self.run_capture()

    def test_unknown_nested_fields(self) -> None:
        targets = [
            self.record["source"],
            self.record["profile"],
            self.record["files"][0],
            self.record["units"][0],
            self.observed["config"],
            self.observed["qt"],
            self.observed["qt"]["tool"],
            self.observed["records"][0],
            self.observed["records"][0]["compiler"],
            self.observed["records"][0]["normalized_depfile"],
        ]
        for target in targets:
            with self.subTest(target=tuple(target)):
                target["unknown"] = True
                try:
                    with self.assertRaises(p.CaptureError):
                        self.run_capture()
                finally:
                    del target["unknown"]

    def test_generated_separate_missing(self) -> None:
        self.record["files"].append(
            {
                "id": "g",
                "root": "build",
                "path": "generated.cpp",
                "kind": "generated",
                "sha256": digest(b"generated"),
            }
        )
        self.observed["records"][0]["normalized_depfile"]["dependencies"].append("g")
        result = self.run_capture()
        self.assertEqual(result["status"], "MISSING")
        self.assertIn("generated-retained-bytes:g", result["missing"])

    def test_wrong_pin(self) -> None:
        with self.assertRaises(p.CaptureError):
            p.retain_used_input_bytes(
                encoded(self.record),
                "0" * 64,
                encoded(self.observed),
                "0" * 64,
                self.root / "out",
            )


if __name__ == "__main__":
    unittest.main()
