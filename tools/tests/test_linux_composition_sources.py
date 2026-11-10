"""Проверки h1, raw notices и pinned module provenance."""

from __future__ import annotations

import base64
import hashlib
import importlib.util
import io
import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import ModuleType
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def load(name: str, path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


collector = load("collector", ROOT / "packaging/licenses/composition/collect_go.py")
scanner = load("scanner", ROOT / "tools/check_linux_composition.py")


class SourceTests(unittest.TestCase):
    def test_primary_zip_hash_and_raw_bytes(self) -> None:
        raw = b"Copyright\r\n\xff\n"
        name = "example.org/a@v1.0.0/LICENSE"
        line = f"{hashlib.sha256(raw).hexdigest()}  {name}\n".encode()
        expected = "h1:" + base64.b64encode(hashlib.sha256(line).digest()).decode()
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as archive:
            archive.writestr(name, raw)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "texts").mkdir()
            with patch.object(
                collector.urllib.request,
                "urlopen",
                return_value=io.BytesIO(buffer.getvalue()),
            ):
                report = collector.collect(
                    ("example.org/a", "v1.0.0", expected), root, root
                )
            self.assertEqual(report["status"], "observed")
            self.assertEqual((root / report["texts"][0]["file"]).read_bytes(), raw)
            with patch.object(
                collector.urllib.request,
                "urlopen",
                return_value=io.BytesIO(buffer.getvalue()),
            ):
                bad = collector.collect(
                    ("example.org/a", "v1.0.0", "h1:подмена"), root, root
                )
            self.assertEqual(bad["status"], "unresolved")
            self.assertIn("h1 mismatch", bad["error"])

    def test_licensewizard_is_not_notice(self) -> None:
        name, raw = "example.org/a@v1.0.0/licensewizard.png", b"illustration"
        line = f"{hashlib.sha256(raw).hexdigest()}  {name}\n".encode()
        expected = "h1:" + base64.b64encode(hashlib.sha256(line).digest()).decode()
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, "w") as archive:
            archive.writestr(name, raw)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "texts").mkdir()
            with patch.object(
                collector.urllib.request,
                "urlopen",
                return_value=io.BytesIO(stream.getvalue()),
            ):
                report = collector.collect(
                    ("example.org/a", "v1.0.0", expected), root, root
                )
            self.assertEqual(report["status"], "missing_text")
            self.assertFalse(report["texts"])

    def test_module_identity_validation(self) -> None:
        for value in ("../escape", "a?token=x", "/a/../b"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                collector.escaped(value)
        self.assertEqual(collector.escaped("example.org/Ab"), "example.org/!ab")

    def test_real_bundle_remains_partial(self) -> None:
        composition = json.loads(
            (ROOT / "packaging/licenses/linux-composition.json").read_text()
        )
        result = scanner.validate_bundle(
            ROOT / "packaging/licenses/composition", composition
        )
        self.assertEqual(result["status"], "partial")
        self.assertGreater(result["qt_source_checksum_mismatches"], 0)

    def test_bundle_rejects_raw_and_runtime_tamper(self) -> None:
        composition = json.loads(
            (ROOT / "packaging/licenses/linux-composition.json").read_text()
        )
        original = scanner.read_file

        def altered(path: Path, limit: int = scanner.MAX_FILE) -> bytes:
            if path.name == "qt-runtime.json":
                row = json.loads(original(path, limit))
                row["binary_sha256"] = "подмена"
                return json.dumps(row).encode()
            return original(path, limit)

        with (
            patch.object(scanner, "read_file", side_effect=altered),
            self.assertRaisesRegex(ValueError, "другому desktop"),
        ):
            scanner.validate_bundle(
                ROOT / "packaging/licenses/composition", composition
            )

        def corrupt(path: Path, limit: int = scanner.MAX_FILE) -> bytes:
            return b"corrupt" if path.parent.name == "texts" else original(path, limit)

        with (
            patch.object(scanner, "read_file", side_effect=corrupt),
            self.assertRaisesRegex(ValueError, "hash mismatch"),
        ):
            scanner.validate_bundle(
                ROOT / "packaging/licenses/composition", composition
            )

    def test_rust_elf_and_path_tamper(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "new-binaries").mkdir()
            rows = []
            for name in ("yonik-desktop", "yonik-helper", "yonik-worker"):
                (root / "new-binaries" / name).write_bytes(b"binary")
                raw = b"header\nMemory map\n .text 0x100 0x10 /a/libone-123.rlib(a.o)\n"
                (root / (name.replace("-", "_") + ".map")).write_bytes(raw)
                rows.append(
                    {
                        "binary": name,
                        "sha256": scanner.sha(b"binary"),
                        "map_sha256": scanner.sha(raw),
                        "archive_mentions": [],
                    }
                )
            report = {"evidence_sha256": {}, "binaries": rows}
            scanner.validate_rust_evidence(root, report)
            (root / "new-binaries/yonik-worker").write_bytes(b"modified")
            with self.assertRaisesRegex(ValueError, "ELF hash"):
                scanner.validate_rust_evidence(root, report)
            report["evidence_sha256"] = {"../escape": "x"}
            with self.assertRaisesRegex(ValueError, "небезопасный"):
                scanner.validate_rust_evidence(root, report)

    def test_retained_sections_exclude_discarded_and_zero(self) -> None:
        text = (
            "Discarded input sections\n .text 0x1 0x10 /a/libdead-123.rlib(x.o)\n"
            "Memory map\n .text 0x100 0x10 /a/liblive-123.rlib(x.o)\n"
            " .data 0x110 0x0 /a/libzero-123.rlib(x.o)\n"
            " .debug 0x0 0x10 /a/libdebug-123.rlib(x.o)\n"
        )
        self.assertEqual(set(scanner.retained_archives(text)), {"liblive-123.rlib"})
        with self.assertRaises(ValueError):
            scanner.retained_archives("unknown map format")

    def test_runtime_build_dev_roles(self) -> None:
        names = ("root", "runtime", "builder", "macro", "dev")
        metadata = {
            "packages": [
                {
                    "id": name,
                    "targets": [{"kind": ["proc-macro" if name == "macro" else "lib"]}],
                }
                for name in names
            ],
            "resolve": {"nodes": [{"id": name, "deps": []} for name in names]},
        }
        nodes = metadata["resolve"]["nodes"]
        nodes[0]["deps"] = [
            {"pkg": name, "dep_kinds": [{"kind": kind}]}
            for name, kind in (
                ("runtime", None),
                ("builder", "build"),
                ("macro", None),
                ("dev", "dev"),
            )
        ]
        nodes[2]["deps"] = [{"pkg": "runtime", "dep_kinds": [{"kind": None}]}]
        roles = scanner.dependency_roles(metadata, "root")
        self.assertEqual(roles["runtime"], ["build", "normal"])
        self.assertEqual(roles["builder"], ["build"])
        self.assertEqual(roles["macro"], ["build"])
        self.assertNotIn("dev", roles)

    def test_notice_tamper_and_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "texts").mkdir()
            digest = scanner.sha(b"original")
            (root / "texts" / digest).write_bytes(b"tampered")
            report = {
                "records": [
                    {
                        "module": "example.org/a",
                        "version": "v1.0.0",
                        "h1": "h1:pin",
                        "status": "observed",
                        "h1_verified": True,
                        "zip_url": "https://proxy.golang.org/example.org/a/@v/v1.0.0.zip",
                        "texts": [{"file": "texts/" + digest, "sha256": digest}],
                    }
                ]
            }
            manifest = root / "sources.json"
            manifest.write_text(json.dumps(report))
            composition = {
                "cores": {
                    "core": {
                        "go": {
                            "modules": [["dep", "example.org/a", "v1.0.0", "h1:pin"]]
                        }
                    }
                }
            }
            with self.assertRaisesRegex(ValueError, "подмена Go notice"):
                scanner.validate_go_sources(manifest, composition)
            report["records"][0]["version"] = "v2.0.0"
            manifest.write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, "identity"):
                scanner.validate_go_sources(manifest, composition)


if __name__ == "__main__":
    unittest.main()
