"""Изолированные отрицательные проверки лицензионного отчёта."""

import copy
import importlib.util
import tempfile
import unittest
from pathlib import Path

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
