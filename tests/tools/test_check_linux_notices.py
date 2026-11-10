"""Изолированные отрицательные проверки лицензионного отчёта."""

import copy
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "notices", Path(__file__).resolve().parents[2] / "tools/check_linux_notices.py"
)


class NoticesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('[package]\nname="fixture"\n')
        (self.root / "LICENSE").write_text("MIT primary license fixture")
        (self.root / "NOTICE").write_text("Required attribution fixture")
        self.metadata = {
            "packages": [
                {
                    "id": "fixture",
                    "name": "yonik-desktop",
                    "version": "1",
                    "source": None,
                    "manifest_path": str(self.root / "Cargo.toml"),
                    "license": "MIT",
                    "license_file": None,
                }
            ],
            "resolve": {"nodes": [{"id": "fixture", "features": ["gui"], "deps": []}]},
        }
        self.report = notices.inventory(self.metadata, self.root)

    def test_valid(self) -> None:
        self.assertEqual(notices.validate(self.report, self.metadata, self.root), [])

    def test_missing_license(self) -> None:
        self.report["packages"][0]["license_files"] = []
        self.assertTrue(
            any(
                "missing_license" in error
                for error in notices.validate(self.report, self.metadata, self.root)
            )
        )

    def test_missing_notice(self) -> None:
        self.report["packages"][0]["notice_files"] = []
        self.assertTrue(
            any(
                "missing_notice" in error
                for error in notices.validate(self.report, self.metadata, self.root)
            )
        )

    def test_wrong_artifact_provenance(self) -> None:
        self.report["artifacts"][0]["features"] = ["test-support"]
        self.assertTrue(
            any(
                "artifact_provenance" in error
                for error in notices.validate(self.report, self.metadata, self.root)
            )
        )

    def test_changed_license(self) -> None:
        (self.root / "LICENSE").write_text("different text")
        self.assertTrue(notices.validate(self.report, self.metadata, self.root))

    def test_missing_package(self) -> None:
        self.report["packages"] = []
        self.assertTrue(notices.validate(self.report, self.metadata, self.root))

    def test_license_paths_cannot_escape_or_follow_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as outside:
            external = Path(outside) / "canary"
            external.write_text("OWNED CANARY")
            link = self.root / "linked-license"
            link.symlink_to(external)
            for name in [
                str(external),
                os.path.relpath(external, self.root),
                str(link.name),
            ]:
                with self.subTest(path=name):
                    metadata = copy.deepcopy(self.metadata)
                    metadata["packages"][0]["license_file"] = name
                    with self.assertRaises(ValueError):
                        notices.inventory(metadata, self.root)

    def test_whitespace_license_and_notice_are_not_evidence(self) -> None:
        for name in ["LICENSE", "NOTICE"]:
            with self.subTest(name=name):
                path = self.root / name
                original = path.read_bytes()
                path.write_bytes(b" \t\r\n")
                try:
                    with self.assertRaises(ValueError):
                        notices.inventory(self.metadata, self.root)
                finally:
                    path.write_bytes(original)

    def test_report_digest_cannot_trigger_external_read(self) -> None:
        with tempfile.TemporaryDirectory() as outside:
            external = Path(outside) / "owned-canary"
            external.write_text("OWNED CANARY")
            report = copy.deepcopy(self.report)
            report["packages"][0]["license_files"][0]["sha256"] = str(external)
            report_path = self.root / "report.json"
            report_path.write_text(json.dumps(report))
            reads = []
            original_digest = notices.digest

            def record(path: Path) -> str:
                reads.append(path.resolve())
                return original_digest(path)

            with (
                mock.patch.object(
                    notices, "__file__", str(self.root / "tools/check_linux_notices.py")
                ),
                mock.patch.object(
                    notices.subprocess,
                    "check_output",
                    return_value=json.dumps(self.metadata),
                ),
                mock.patch("sys.argv", ["notices", "--check", str(report_path)]),
                mock.patch.object(notices, "digest", side_effect=record),
            ):
                self.assertTrue(notices.main())
            self.assertNotIn(external.resolve(), reads)

    def test_output_bundle_directory_cannot_be_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as outside:
            external = Path(outside)
            (self.root / "texts").symlink_to(external, target_is_directory=True)
            with (
                mock.patch.object(
                    notices, "__file__", str(self.root / "tools/check_linux_notices.py")
                ),
                mock.patch.object(
                    notices.subprocess,
                    "check_output",
                    return_value=json.dumps(self.metadata),
                ),
                mock.patch(
                    "sys.argv", ["notices", "--output", str(self.root / "report.json")]
                ),
            ):
                result = notices.main()
            self.assertTrue(result)
            self.assertEqual(list(external.iterdir()), [])

    def test_workspace_root_license_is_supported(self) -> None:
        crate = self.root / "crate"
        crate.mkdir()
        (crate / "Cargo.toml").write_text("[package]\n")
        metadata = copy.deepcopy(self.metadata)
        metadata["packages"][0]["manifest_path"] = str(crate / "Cargo.toml")
        metadata["packages"][0]["license_file"] = "../LICENSE"
        report = notices.inventory(metadata, self.root)
        self.assertEqual(
            report["packages"][0]["license_files"][0]["path"], "../LICENSE"
        )
        self.assertEqual(notices.validate(report, metadata, self.root), [])

    def test_dev_only_excluded(self) -> None:
        metadata = copy.deepcopy(self.metadata)
        metadata["resolve"]["nodes"][0]["deps"] = [
            {"pkg": "dev", "dep_kinds": [{"kind": "dev"}]}
        ]
        self.assertEqual(len(notices.inventory(metadata, self.root)["packages"]), 1)


if SPEC and SPEC.loader:
    notices = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(notices)

if __name__ == "__main__":
    unittest.main()
