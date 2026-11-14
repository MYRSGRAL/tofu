"""Закрытые fixtures post-build capture без компилятора и host SDK."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
import p25_capture_build_closure as capture


class CaptureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.roots = {key: self.base / key for key in ("source", "build", "sdk")}
        for root in self.roots.values():
            root.mkdir()
        self.files: list[dict] = []
        self.add("src", "source", "main.cpp", "source", b"// own source\n")
        self.add("primary", "source", "LICENSE", "primary", b"fixture primary bytes\n")
        self.add(
            "header",
            "sdk",
            "q.h",
            "header",
            b"// SPDX-License-Identifier: LGPL-3.0-only OR GPL-3.0-only\n",
        )
        self.add("tool", "sdk", "moc", "tool", b"not executable fixture")
        self.add(
            "generated",
            "build",
            "moc.cpp",
            "generated",
            b"// SPDX-License-Identifier: GPL-3.0-only\n// generated fixture\n",
        )
        self.add("elf", "build", "desktop", "elf", b"fixture ELF bytes, never executed")
        self.add(
            "deps",
            "build",
            "deps.json",
            "depfile",
            self.encode(
                {"schema": 1, "unit": "generated", "dependencies": ["src", "header"]}
            ),
        )
        self.add(
            "qt",
            "build",
            "qt.json",
            "qt-version",
            self.encode(
                {
                    "schema": 1,
                    "version": "6.11.2",
                    "sdk_files": ["header", "tool"],
                    "query_tool": "tool",
                    "query_argv": ["qmake6", "-query", "QT_VERSION"],
                    "exit": 0,
                }
            ),
        )
        self.add(
            "sdk-source",
            "source",
            "qt-source.h",
            "sdk-source",
            b"// source counterpart fixture\n",
        )
        self.files[2]["source_ref"] = "sdk-source"
        self.record = {
            "schema": 1,
            "source": {
                "commit": "a" * 40,
                "tree": "b" * 40,
                "archive_sha256": "c" * 64,
                "tracked_inputs": ["src", "primary", "sdk-source"],
            },
            "profile": {
                "target": "x86_64-unknown-linux-gnu",
                "commands": [["cargo", "rustc", "--locked", "--release"]],
                "features": ["gui"],
            },
            "files": self.files,
            "elfs": ["elf"],
            "qt_version": "qt",
            "units": [
                {
                    "file": "generated",
                    "depfile": "deps",
                    "argv": ["c++", "-c", "moc.cpp"],
                }
            ],
            "generated": [
                {"file": "generated", "inputs": ["src", "header"], "generator": "tool"}
            ],
            "primary_refs": [
                {"identifier": "LGPL-3.0-only", "file": "primary"},
                {"identifier": "GPL-3.0-only", "file": "primary"},
            ],
        }

    @staticmethod
    def encode(value: dict) -> bytes:
        return json.dumps(value).encode()

    def add(self, identity: str, root: str, path: str, kind: str, data: bytes) -> None:
        (self.roots[root] / path).write_bytes(data)
        self.files.append(
            {
                "id": identity,
                "root": root,
                "path": path,
                "kind": kind,
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )

    def run_capture(self) -> dict:
        raw = self.encode(self.record)
        (self.roots["build"] / "record.json").write_bytes(raw)
        return capture.capture(
            self.roots, "record.json", hashlib.sha256(raw).hexdigest()
        )

    def test_closed_fixture(self) -> None:
        report = self.run_capture()
        self.assertEqual(report["status"], "CAPTURED")
        self.assertFalse(report["final_linux_acceptance"])
        self.assertEqual(report["legal_status"], "NOTPROVEN")
        self.assertEqual(report["qt_version"], "6.11.2")
        self.assertEqual(
            report["compile_units"][0]["normalized_depfile"]["dependencies"],
            ["src", "header"],
        )
        self.assertEqual(report["qt_binding"]["query_tool"]["id"], "tool")
        self.assertEqual(
            report["headers"][0]["expressions"], ["LGPL-3.0-only OR GPL-3.0-only"]
        )
        self.assertEqual(
            report["generated_sources"][0]["text"],
            "// SPDX-License-Identifier: GPL-3.0-only\n// generated fixture\n",
        )
        self.assertEqual(report["elf_profile_binding"]["elfs"][0]["id"], "elf")

    def test_missing_metadata(self) -> None:
        for field in ("qt_version", "units", "generated"):
            with self.subTest(field=field):
                value = self.record.pop(field)
                report = self.run_capture()
                self.assertEqual(report["status"], "MISSING")
                self.record[field] = value

    def test_missing_file_and_identity(self) -> None:
        (self.roots["sdk"] / "q.h").unlink()
        self.assertEqual(self.run_capture()["status"], "MISSING")
        self.files[2].pop("sha256")
        self.assertEqual(self.run_capture()["status"], "MISSING")

    def test_primary_unknown(self) -> None:
        self.record["primary_refs"] = []
        report = self.run_capture()
        self.assertEqual(report["status"], "MISSING")
        self.assertTrue(any("primary" in value for value in report["missing"]))

    def test_wrong_hash(self) -> None:
        (self.roots["sdk"] / "q.h").write_text("changed")
        with self.assertRaises(capture.CaptureError):
            self.run_capture()

    def test_symlink_and_root_symlink(self) -> None:
        (self.roots["sdk"] / "q.h").unlink()
        (self.roots["sdk"] / "q.h").symlink_to(self.roots["source"] / "LICENSE")
        with self.assertRaises(capture.CaptureError):
            self.run_capture()
        alias = self.base / "alias"
        alias.symlink_to(self.roots["sdk"], target_is_directory=True)
        self.roots["sdk"] = alias
        with self.assertRaises(capture.CaptureError):
            self.run_capture()

    def test_path_and_schema_rejected(self) -> None:
        for path in ("../secret", "/etc/passwd", "a//b", "./x"):
            with self.subTest(path=path):
                self.files[2]["path"] = path
                with self.assertRaises(capture.CaptureError):
                    self.run_capture()
        self.files[2]["path"] = "q.h"
        self.record["unexpected"] = True
        with self.assertRaises(capture.CaptureError):
            self.run_capture()

    def test_not_tracked_and_duplicate(self) -> None:
        self.record["source"]["tracked_inputs"] = ["src"]
        with self.assertRaises(capture.CaptureError):
            self.run_capture()
        self.record["source"]["tracked_inputs"] = ["src", "primary", "sdk-source"]
        self.files.append(self.files[0])
        with self.assertRaises(capture.CaptureError):
            self.run_capture()

    def test_bounds(self) -> None:
        for limit in ("MAX_FILES", "MAX_TOTAL", "MAX_FILE"):
            with self.subTest(limit=limit), patch.object(capture, limit, 1):
                with self.assertRaises(capture.CaptureError):
                    self.run_capture()

    def test_malformed_json(self) -> None:
        raw = b'{"schema":1,"schema":1}'
        (self.roots["build"] / "record.json").write_bytes(raw)
        with self.assertRaises(capture.CaptureError):
            capture.capture(self.roots, "record.json", hashlib.sha256(raw).hexdigest())

    def test_changed_metadata_during_capture(self) -> None:
        original = capture.Reader.read
        fired = False

        def mutate(
            reader: capture.Reader, root: str, path: str, *args: object
        ) -> bytes:
            nonlocal fired
            result = original(reader, root, path, *args)
            if path == "q.h" and not fired:
                fired = True
                (self.roots["build"] / "record.json").write_bytes(b"{}")
            return result

        with (
            patch.object(capture.Reader, "read", mutate),
            self.assertRaises(capture.CaptureError),
        ):
            self.run_capture()

    def test_generated_without_condition_is_not_complete(self) -> None:
        raw = b"// generated with no license statement\n"
        (self.roots["build"] / "moc.cpp").write_bytes(raw)
        self.files[4]["sha256"] = hashlib.sha256(raw).hexdigest()
        report = self.run_capture()
        self.assertIn("SPDX:generated", report["missing"])

    def test_cli_readonly_and_optimized_rejection(self) -> None:
        self.run_capture()
        before = {
            p: p.read_bytes() for root in self.roots.values() for p in root.iterdir()
        }
        argv = [
            sys.executable,
            "-O",
            str(Path(capture.__file__)),
            "--record",
            "record.json",
            "--record-sha256",
            hashlib.sha256(before[self.roots["build"] / "record.json"]).hexdigest(),
        ]
        for key, root in self.roots.items():
            argv.extend(["--" + key + "-root", str(root)])
        result = subprocess.run(argv, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["status"], "CAPTURED")
        self.assertEqual(
            before,
            {p: p.read_bytes() for root in self.roots.values() for p in root.iterdir()},
        )
        argv[argv.index("--record-sha256") + 1] = "0" * 64
        result = subprocess.run(argv, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)["status"], "REJECTED")

    def test_fifo_and_sparse_oversize(self) -> None:
        path = self.roots["sdk"] / "q.h"
        path.unlink()
        os.mkfifo(path)
        with self.assertRaises(capture.CaptureError):
            self.run_capture()
        path.unlink()
        with path.open("wb") as stream:
            stream.truncate(capture.MAX_TEXT + 1)
        with self.assertRaises(capture.CaptureError):
            self.run_capture()

    def test_bad_record_pin(self) -> None:
        raw = self.encode(self.record)
        (self.roots["build"] / "record.json").write_bytes(raw)
        with self.assertRaises(capture.CaptureError):
            capture.capture(self.roots, "record.json", "0" * 64)

    def test_nonregular_and_intermediate_symlink(self) -> None:
        (self.roots["sdk"] / "q.h").unlink()
        (self.roots["sdk"] / "q.h").mkdir()
        with self.assertRaises(capture.CaptureError):
            self.run_capture()
        (self.roots["sdk"] / "q.h").rmdir()
        (self.roots["sdk"] / "alias").symlink_to(
            self.roots["source"], target_is_directory=True
        )
        self.files[2]["path"] = "alias/LICENSE"
        with self.assertRaises(capture.CaptureError):
            self.run_capture()

    def test_malformed_nested_metadata(self) -> None:
        for value in (None, 42, [], {"kind": []}):
            with self.subTest(value=value):
                original = self.record["profile"]
                self.record["profile"] = value
                with self.assertRaises(capture.CaptureError):
                    self.run_capture()
                self.record["profile"] = original
        raw = self.encode({"schema": 1, "unit": "other", "dependencies": ["header"]})
        (self.roots["build"] / "deps.json").write_bytes(raw)
        self.files[6]["sha256"] = hashlib.sha256(raw).hexdigest()
        with self.assertRaises(capture.CaptureError):
            self.run_capture()

    def test_changed_during_file_read(self) -> None:
        original = capture.os.fstat
        calls = 0

        def changed(fd: int) -> object:
            nonlocal calls
            calls += 1
            value = original(fd)
            if calls == 2:
                return type(
                    "Stat",
                    (),
                    {
                        "st_dev": value.st_dev,
                        "st_ino": value.st_ino,
                        "st_size": value.st_size,
                        "st_mtime_ns": value.st_mtime_ns + 1,
                        "st_ctime_ns": value.st_ctime_ns,
                    },
                )()
            return value

        with (
            patch.object(capture.os, "fstat", changed),
            self.assertRaises(capture.CaptureError),
        ):
            self.run_capture()

    def test_metadata_and_generated_size_bounds(self) -> None:
        for limit in ("MAX_METADATA", "MAX_GENERATED", "MAX_TEXT"):
            with self.subTest(limit=limit), patch.object(capture, limit, 1):
                with self.assertRaises(capture.CaptureError):
                    self.run_capture()

    def test_missing_cxx_and_header_source(self) -> None:
        self.record["units"][0].pop("argv")
        self.assertIn("compile_metadata:generated", self.run_capture()["missing"])
        self.record["units"][0]["argv"] = ["c++", "-c", "moc.cpp"]
        self.files[2].pop("source_ref")
        self.assertIn("header_source:header", self.run_capture()["missing"])

    def test_depfile_header_coverage(self) -> None:
        self.add(
            "other",
            "sdk",
            "other.h",
            "header",
            b"// SPDX-License-Identifier: GPL-3.0-only\n",
        )
        self.files[-1]["source_ref"] = "sdk-source"
        qt = self.roots["build"] / "qt.json"
        d = json.loads(qt.read_bytes())
        d["sdk_files"].append("other")
        qt.write_bytes(self.encode(d))
        self.files[7]["sha256"] = hashlib.sha256(qt.read_bytes()).hexdigest()
        self.assertIn("header_depfile_coverage:other", self.run_capture()["missing"])

    def test_bad_depfile_and_missing_generated_record(self) -> None:
        self.record["units"][0]["depfile"] = None
        self.assertEqual(self.run_capture()["status"], "MISSING")
        self.record["units"][0]["depfile"] = "deps"
        self.record["generated"] = []
        self.assertEqual(self.run_capture()["status"], "MISSING")


if __name__ == "__main__":
    unittest.main()
