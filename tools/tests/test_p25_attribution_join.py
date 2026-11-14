"""Чистый downstream join; fixtures не являются actual build evidence."""

import hashlib
import json
import sys
import unittest
from copy import deepcopy
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import _p25_attribution_join as join

BASE = Path(__file__).resolve().parents[2]
CANDIDATES = BASE / "docs/validation/p25-current-payload/qt-file-attribution-candidate"


def encode(value: dict) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()


def checksum(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


class AttributionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.candidate = json.loads((CANDIDATES / "candidate-join.json").read_bytes())
        self.header = next(
            r for r in self.candidate["headers"] if r["id"] == "QtCore-qstring.h"
        )
        self.files = [
            {
                "id": "unit",
                "root": "build",
                "kind": "generated",
                "path": "bridge.cpp",
                "sha256": checksum(b"generated"),
            },
            {
                "id": "h",
                "root": "sdk",
                "kind": "header",
                "path": "staged-h",
                "sha256": self.header["sha256"],
            },
            {
                "id": "unused",
                "root": "sdk",
                "kind": "header",
                "path": "not-used",
                "sha256": self.header["sha256"],
            },
            {
                "id": "compiler",
                "root": "sdk",
                "kind": "tool",
                "path": "compiler",
                "sha256": "d" * 64,
            },
            {
                "id": "dep",
                "root": "build",
                "kind": "depfile",
                "path": "dep.json",
                "sha256": "e" * 64,
            },
            {
                "id": "elf",
                "root": "build",
                "kind": "elf",
                "path": "elf",
                "sha256": "f" * 64,
            },
        ]
        self.unit = {
            "file": "unit",
            "depfile": "dep",
            "argv": ["fixed-cxx", "-c", "/build/bridge.cpp"],
        }
        self.normalized = {"schema": 1, "unit": "unit", "dependencies": ["unit", "h"]}
        self.source = {
            "commit": "a" * 40,
            "tree": "b" * 40,
            "archive_sha256": "c" * 64,
            "tracked_inputs": [],
        }
        self.profile = {
            "target": "target",
            "commands": [["cargo", "build"]],
            "features": ["gui"],
        }
        self.record = {
            "schema": 1,
            "source": self.source,
            "profile": self.profile,
            "files": self.files,
            "elfs": ["elf"],
            "primary_refs": [],
            "units": [self.unit],
            "generated": [],
            "qt_version": None,
        }
        self.report = {
            "schema": 1,
            "status": "MISSING",
            "source": self.source,
            "files": [dict(r, verified=True) for r in self.files],
            "roots": {"source": "/source", "build": "/build", "sdk": "/sdk"},
            "compile_units": [{**self.unit, "normalized_depfile": self.normalized}],
            "elf_profile_binding": {
                "profile": self.profile,
                "elfs": [dict(self.files[-1], verified=True)],
            },
            "legal_status": "NOTPROVEN",
            "final_linux_acceptance": False,
            "production_admission_gate": False,
        }
        self.observed = {
            "schema": 1,
            "records": [
                {
                    "argv": self.unit["argv"],
                    "exit": 0,
                    "compiler_id": "compiler",
                    "compiler": {"path": "/usr/bin/fixed-cxx", "sha256": "d" * 64},
                    "normalized_depfile": self.normalized,
                }
            ],
            "input_origins": {
                self.header["origin_path"]: "staged-h",
                "/usr/not-actually-used.h": "not-used",
            },
            "config": {
                "schema": 1,
                "source": "/source",
                "build": "/build",
                "sdk": "/usr",
            },
        }
        self.fragments = {
            self.header["source_fragment"]["retained_copy"]: (
                CANDIDATES / self.header["source_fragment"]["retained_copy"]
            ).read_bytes()
        }
        self.primary = {
            r["primary_sha256"]: (
                BASE / "packaging/licenses/composition/texts" / r["primary_sha256"]
            ).read_bytes()
            for r in self.header["gpl3_compatible_candidates"]
        }

    def run_join(self, **overrides: Any) -> dict:
        record = encode(self.record)
        report = encode({**self.report, "record_sha256": checksum(record)})
        observed = encode(self.observed)
        manifest = encode(
            {
                "schema": 1,
                "source": self.source,
                "record_sha256": checksum(record),
                "report_sha256": checksum(report),
                "positive_metadata": {"observed_sha256": checksum(observed)},
                "elfs": [self.files[-1]],
                "legal_status": "NOTPROVEN",
                "final_linux_acceptance": False,
            }
        )
        candidate = encode(self.candidate)
        arguments = dict(
            manifest=manifest,
            manifest_sha256=checksum(manifest),
            record=record,
            report=report,
            observed=observed,
            candidates=candidate,
            candidates_sha256=checksum(candidate),
            source_fragments=self.fragments,
            primary_texts=self.primary,
        )
        arguments.update(overrides)
        return join.attribute_capture(**arguments)

    def test_exact_join_uses_dependencies_only(self) -> None:
        before = deepcopy(self.candidate)
        result = self.run_join()
        rows = {r["file_id"]: r for r in result["inputs"]}
        self.assertEqual(set(rows), {"unit", "h"})
        self.assertEqual(rows["h"]["classification"], "MATCHED")
        self.assertTrue(rows["h"]["source_fragment"]["bytes_verified"])
        self.assertEqual(
            {r["identifier"] for r in rows["h"]["candidate_options"]},
            {"GPL-3.0-only", "LGPL-3.0-only"},
        )
        self.assertTrue(
            all(r["bytes_verified"] for r in rows["h"]["candidate_options"])
        )
        self.assertEqual(rows["unit"]["classification"], "UNKNOWN")
        self.assertEqual(result["source_capsule"]["classification"], "MISSING")
        self.assertEqual(result["legal_status"], "NOTPROVEN")
        self.assertFalse(result["final_linux_acceptance"])
        self.assertEqual(self.candidate, before)

    def test_observed_records_require_closed_nested_objects(self) -> None:
        for case in (
            "record-scalar",
            "compiler-list",
            "record-extra",
            "compiler-extra",
        ):
            with self.subTest(case=case):
                self.setUp()
                row = self.observed["records"][0]
                if case == "record-scalar":
                    self.observed["records"].append("invalid")
                elif case == "compiler-list":
                    row["compiler"] = []
                elif case == "record-extra":
                    row["unexpected"] = True
                else:
                    row["compiler"]["unexpected"] = True
                with self.assertRaises(join.CaptureError):
                    self.run_join()

    def test_classification_cases(self) -> None:
        cases = {
            "missing-origin": "MISSING",
            "unknown-origin": "UNKNOWN",
            "ambiguous-origin": "CONFLICTING",
            "hash-conflict": "CONFLICTING",
            "duplicate-candidate": "CONFLICTING",
            "missing-fragment": "MISSING",
            "bad-fragment": "CONFLICTING",
            "missing-primary": "MISSING",
            "bad-primary": "CONFLICTING",
            "unverified-file": "MISSING",
            "spdx-conflict": "CONFLICTING",
        }
        for case, expected in cases.items():
            with self.subTest(case=case):
                self.setUp()
                origin = self.header["origin_path"]
                if case == "missing-origin":
                    self.observed["input_origins"].pop(origin)
                if case == "unknown-origin":
                    self.observed["input_origins"]["/usr/unknown.h"] = self.observed[
                        "input_origins"
                    ].pop(origin)
                if case == "ambiguous-origin":
                    self.observed["input_origins"]["/usr/alias.h"] = "staged-h"
                if case == "hash-conflict":
                    self.files[1]["sha256"] = "0" * 64
                    self.report["files"][1]["sha256"] = "0" * 64
                if case == "duplicate-candidate":
                    self.candidate["headers"].append(deepcopy(self.header))
                if case == "missing-fragment":
                    self.fragments.clear()
                if case == "bad-fragment":
                    self.fragments = {key: b"changed" for key in self.fragments}
                if case == "missing-primary":
                    self.primary.clear()
                if case == "bad-primary":
                    self.primary = {key: b"changed" for key in self.primary}
                if case == "unverified-file":
                    self.report["files"][1]["verified"] = False
                if case == "spdx-conflict":
                    self.header["spdx_expressions"] = ["invented"]
                result = self.run_join()
                header = next(row for row in result["inputs"] if row["file_id"] == "h")
                self.assertEqual(header["classification"], expected)
                self.assertTrue(header["issues"])
                self.assertFalse(result["final_linux_acceptance"])

    def test_absent_or_conflicting_compile_evidence_never_promotes_inventory(
        self,
    ) -> None:
        for conflict in (False, True):
            with self.subTest(conflict=conflict):
                self.setUp()
                if conflict:
                    self.observed["records"][0]["normalized_depfile"] = {
                        **self.normalized,
                        "dependencies": ["unit", "h", "unused"],
                    }
                else:
                    self.observed["records"].clear()
                result = self.run_join()
                self.assertEqual(result["inputs"], [])
                self.assertEqual(
                    result["units"][0]["classification"],
                    "CONFLICTING" if conflict else "MISSING",
                )

    def test_configuration_source_match_does_not_inherit_license(self) -> None:
        config = next(
            r for r in self.candidate["headers"] if r["id"] == "QtCore-qconfig.h"
        )
        self.observed["input_origins"] = {config["origin_path"]: "staged-h"}
        self.files[1]["sha256"] = config["sha256"]
        self.report["files"][1]["sha256"] = config["sha256"]
        name = config["source_fragment"]["retained_copy"]
        self.fragments = {name: (CANDIDATES / name).read_bytes()}
        result = self.run_join()
        row = next(r for r in result["inputs"] if r["file_id"] == "h")
        self.assertTrue(row["source_fragment"]["bytes_verified"])
        self.assertEqual(row["candidate_options"], [])
        self.assertEqual(row["classification"], "MISSING")
        self.assertIn("missing:file_level_license_declaration", row["issues"])

    def test_bad_schema_and_scope_are_rejected(self) -> None:
        for case in ("schema", "unknown-field", "omitted-unit", "tool-as-dependency"):
            with self.subTest(case=case):
                self.setUp()
                if case == "schema":
                    self.report["schema"] = 2
                if case == "unknown-field":
                    self.candidate["unexpected"] = 1
                if case == "omitted-unit":
                    self.report["compile_units"].clear()
                if case == "tool-as-dependency":
                    self.normalized["dependencies"].append("compiler")
                with self.assertRaises(join.CaptureError):
                    self.run_join()

    def test_real_capture_api_shape(self) -> None:
        from tempfile import TemporaryDirectory

        import p25_capture_build_closure as capture

        with TemporaryDirectory() as temporary:
            roots = {
                name: Path(temporary) / name for name in ("source", "build", "sdk")
            }
            for root in roots.values():
                root.mkdir()
            bodies = {
                "unit": b"generated",
                "h": next(iter(self.fragments.values())),
                "unused": next(iter(self.fragments.values())),
                "compiler": b"fixture compiler",
                "dep": encode(self.normalized),
                "elf": b"fixture ELF",
            }
            for row in self.files:
                body = bodies[row["id"]]
                (roots[row["root"]] / row["path"]).write_bytes(body)
                row["sha256"] = checksum(body)
            self.observed["records"][0]["compiler"]["sha256"] = self.files[3]["sha256"]
            raw = encode(self.record)
            (roots["build"] / "record.json").write_bytes(raw)
            self.report = capture.capture(roots, "record.json", checksum(raw))
            self.observed["config"].update(
                source=str(roots["source"]), build=str(roots["build"])
            )
            result = self.run_join()
            self.assertEqual(
                {row["file_id"] for row in result["inputs"]}, {"unit", "h"}
            )
            header = next(row for row in result["inputs"] if row["file_id"] == "h")
            self.assertEqual(header["classification"], "MATCHED")
            self.assertEqual(self.report["status"], "MISSING")

    def test_pins_duplicate_keys_and_byte_caps_rejected(self) -> None:
        duplicate = b'{"schema":1,"schema":1}'
        cases = [
            {"manifest_sha256": "0" * 64},
            {"candidates_sha256": "0" * 64},
            {"record": b"{}"},
            {"report": b"{}"},
            {"observed": b"{}"},
            {"candidates": duplicate, "candidates_sha256": checksum(duplicate)},
            {"source_fragments": {"x": b"x" * (join.MAX_METADATA + 1)}},
            {"primary_texts": {"x": b"x" * (join.MAX_METADATA + 1)}},
            {"report": b"x" * (32 * join.MAX_METADATA + 1)},
        ]
        for index, overrides in enumerate(cases):
            with self.subTest(case=index):
                with self.assertRaises(join.CaptureError):
                    self.run_join(**overrides)

    def test_bad_paths_and_identity_rejected(self) -> None:
        for case in (
            "origin",
            "fragment",
            "primary-member",
            "source",
            "profile",
            "elf",
            "duplicate-file",
        ):
            with self.subTest(case=case):
                self.setUp()
                if case == "origin":
                    self.header["origin_path"] = "/usr/../private.h"
                if case == "fragment":
                    self.header["source_fragment"]["retained_copy"] = "../private"
                if case == "primary-member":
                    self.candidate["selected_primary_text_matches"][0][
                        "retained_offer_matches"
                    ][0]["source_offer_member"] = "../private"
                if case == "source":
                    self.report["source"] = {**self.source, "commit": "0" * 40}
                if case == "profile":
                    self.report["elf_profile_binding"]["profile"] = {
                        "unexpected": "profile"
                    }
                if case == "elf":
                    self.report["files"][-1]["verified"] = False
                if case == "duplicate-file":
                    self.files.append({**self.files[1], "id": "duplicate"})
                with self.assertRaises(join.CaptureError):
                    self.run_join()

    def test_absent_primary_ref_attestation_is_missing(self) -> None:
        for entry in self.candidate["selected_primary_text_matches"]:
            for ref in entry["retained_offer_matches"]:
                ref["verified"] = False
        result = self.run_join()
        header = next(row for row in result["inputs"] if row["file_id"] == "h")
        self.assertEqual(header["classification"], "MISSING")
        self.assertTrue(
            all(row["bytes_verified"] for row in header["candidate_options"])
        )
        self.assertFalse(
            any(
                row["candidate_offer_refs_attested"]
                for row in header["candidate_options"]
            )
        )

    def test_missing_primary_refs_report_missing_without_discarding_source_match(
        self,
    ) -> None:
        self.candidate["selected_primary_text_matches"].clear()
        result = self.run_join()
        header = next(row for row in result["inputs"] if row["file_id"] == "h")
        self.assertEqual(header["classification"], "MISSING")
        self.assertTrue(header["source_fragment"]["bytes_verified"])
        self.assertTrue(
            any(
                issue.startswith("missing:primary_offer_ref:")
                for issue in header["issues"]
            )
        )

    def test_primary_reference_conflicts_do_not_claim_match(self) -> None:
        self.candidate["selected_primary_text_matches"][0]["retained_offer_matches"][0][
            "sha256"
        ] = "0" * 64
        result = self.run_join()
        header = next(row for row in result["inputs"] if row["file_id"] == "h")
        self.assertEqual(header["classification"], "CONFLICTING")

    def test_same_hash_other_origin_never_falls_back_to_filename_or_version(
        self,
    ) -> None:
        self.observed["input_origins"] = {"/opt/other-sdk/qstring.h": "staged-h"}
        self.observed["qt"] = {"version": "6.11.2"}
        result = self.run_join()
        header = next(row for row in result["inputs"] if row["file_id"] == "h")
        self.assertEqual(header["classification"], "UNKNOWN")
        self.assertEqual(header["candidate_options"], [])

    def test_unverified_depfile_never_generates_used_rows(self) -> None:
        self.report["files"][-2]["verified"] = False
        result = self.run_join()
        self.assertEqual(result["inputs"], [])
        self.assertEqual(result["units"][0]["classification"], "MISSING")

    def test_missing_or_conflicting_compiler_identity_does_not_claim_used_inputs(
        self,
    ) -> None:
        for conflict in (False, True):
            with self.subTest(conflict=conflict):
                self.setUp()
                if conflict:
                    self.observed["records"][0]["compiler"]["sha256"] = "0" * 64
                else:
                    self.observed["records"][0].pop("compiler_id")
                result = self.run_join()
                self.assertEqual(result["inputs"], [])
                self.assertEqual(
                    result["units"][0]["classification"],
                    "CONFLICTING" if conflict else "MISSING",
                )

    def test_reference_fanout_capped_before_large_report_serialization(self) -> None:
        from unittest.mock import patch

        self.header["gpl3_compatible_candidates"] *= 256
        self.candidate["selected_primary_text_matches"][0][
            "retained_offer_matches"
        ] *= 256
        original = json.dumps

        def guarded(value: Any, *args: Any, **kwargs: Any) -> str:
            if isinstance(value, dict) and value.get("status") == "ATTRIBUTION_REPORT":
                self.assertLessEqual(
                    max(
                        (len(row["candidate_options"]) for row in value["inputs"]),
                        default=0,
                    ),
                    50,
                )
            return original(value, *args, **kwargs)

        with patch.object(join.json, "dumps", side_effect=guarded):
            with self.assertRaisesRegex(join.CaptureError, "output cap"):
                self.run_join()
