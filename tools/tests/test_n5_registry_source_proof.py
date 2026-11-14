"""Synthetic sidecar fixtures: ни host registry, ни compiler не используются."""

import hashlib
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import n5_capture_receipts as seam
import n5_registry_source_proof as proof
import test_n5_capture_receipts as receipts_tests


class RegistryProofTests(unittest.TestCase):
    def setUp(self) -> None:
        fixture = receipts_tests.CaptureReceiptTests()
        self.addCleanup(fixture.doCleanups)
        fixture.setUp()
        fixture.final_profile_fixture(registry=True)
        self.fixture = fixture

    def generate(self) -> tuple[dict, str]:
        fixture = self.fixture
        return proof.produce(fixture.retained, fixture.pins, fixture.commit)

    def test_registry_compile_and_probe_replay_offline(self) -> None:
        fixture = self.fixture
        sidecar, pin = self.generate()
        self.assertEqual(sidecar["schema"], proof.SCHEMA)
        self.assertEqual(len(sidecar["packages"][0]["headers"]), 1)
        self.assertEqual(
            len(list((fixture.retained / "registry-archives").iterdir())), 1
        )
        (fixture.root / "registry/cache").rename(fixture.root / "cache-offline")
        with patch.object(
            proof, "produce", side_effect=AssertionError("Offline producer access")
        ):
            result, report = seam.produce(
                fixture.retained,
                fixture.pins,
                fixture.commit,
                registry_proof="registry-proof.json",
                registry_proof_sha256=pin,
            )
        self.assertIn("worker.receipt.json", result)
        self.assertEqual(report["closure_status"], "MISSING")
        self.assertFalse(report["independent_compile_attestation"])
        self.assertEqual(report["registry_proof"]["sha256"], pin)

    def test_probe_only_context_does_not_claim_compiled_unit(self) -> None:
        fixture = receipts_tests.CaptureReceiptTests()
        self.addCleanup(fixture.doCleanups)
        fixture.setUp()
        self.fixture = fixture
        # Новая fixture сохраняет Source compile и только probe в registry cwd.
        fixture.final_profile_fixture(registry=True, probe_only=True)
        sidecar, pin = self.generate()
        self.assertEqual(sidecar["packages"][0]["units"], [])
        _, report = seam.produce(
            fixture.retained,
            fixture.pins,
            fixture.commit,
            registry_proof="registry-proof.json",
            registry_proof_sha256=pin,
        )
        self.assertEqual(report["closure_status"], "MISSING")

    def test_registry_headers_are_bound_and_replayed(self) -> None:
        fixture = self.fixture
        sidecar, pin = self.generate()
        header = sidecar["packages"][0]["headers"][0]
        _, report = seam.produce(
            fixture.retained,
            fixture.pins,
            fixture.commit,
            registry_proof="registry-proof.json",
            registry_proof_sha256=pin,
        )
        self.assertNotIn(header["file_id"], report["unavailable_input_bytes"])
        sidecar["packages"][0]["headers"] = []
        raw = seam.retained_json(sidecar)
        (fixture.retained / "registry-proof.json").write_bytes(raw)
        with self.assertRaises(ValueError):
            seam.produce(
                fixture.retained,
                fixture.pins,
                fixture.commit,
                registry_proof="registry-proof.json",
                registry_proof_sha256=hashlib.sha256(raw).hexdigest(),
            )

    def test_captured_header_sha_must_match_archive(self) -> None:
        fixture = self.fixture
        reader = seam.RetainedReader(fixture.retained)
        _, record, report, observed, lock = proof.inputs(
            reader, fixture.pins, fixture.commit
        )
        cwd = next(iter(proof.selected(observed, lock)))
        rows = proof.unit_bindings(cwd, record, report, observed)
        for row in rows:
            if row["kind"] == "header":
                row["sha256"] = "0" * 64
        cache = (
            fixture.root
            / "registry/cache/index.crates.io-1949cf8c6b5b557f/link-cplusplus-1.0.12.crate"
        )
        with self.assertRaisesRegex(ValueError, "member/staged SHA"):
            proof.archive_units(cache.read_bytes(), Path(cwd).name, rows, set())

    def test_missing_proof_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "MISSING.*registry proof"):
            self.fixture.produce()

    def test_proof_mismatches_rejected(self) -> None:
        value, _ = self.generate()
        fixture = self.fixture
        import copy

        for case in (
            "nonselected",
            "traversal",
            "checksum",
            "member",
            "file-id",
            "size",
            "boolean-size",
            "unknown",
            "missing",
            "input-pin",
        ):
            sidecar = copy.deepcopy(value)
            package = sidecar["packages"][0]
            if case == "nonselected":
                package["cwd"] += "-unknown"
            elif case == "traversal":
                package["cwd"] += "/../link-cplusplus-1.0.12"
            elif case == "checksum":
                package["checksum"] = "0" * 64
            elif case == "member":
                package["units"][0]["member_path"] = "src/other.cc"
            elif case == "file-id":
                package["units"][0]["file_id"] = "unknown"
            elif case == "size":
                package["archive_size"] += 1
            elif case == "boolean-size":
                package["units"][0]["size"] = True
            elif case == "unknown":
                sidecar["unknown"] = 1
            elif case == "missing":
                sidecar["packages"] = []
            elif case == "input-pin":
                sidecar["input_sha256"]["build-cxx-observed.json"] = "0" * 64
            raw = seam.retained_json(sidecar)
            (fixture.retained / "registry-proof.json").write_bytes(raw)
            with self.subTest(case=case), self.assertRaises(ValueError):
                seam.produce(
                    fixture.retained,
                    fixture.pins,
                    fixture.commit,
                    registry_proof="registry-proof.json",
                    registry_proof_sha256=hashlib.sha256(raw).hexdigest(),
                )

    def test_archive_symlink_and_mutation_rejected(self) -> None:
        value, pin = self.generate()
        fixture = self.fixture
        path = fixture.retained / value["packages"][0]["archive_path"]
        original = path.read_bytes()
        path.write_bytes(b"changed")
        with self.assertRaises(ValueError):
            seam.produce(
                fixture.retained,
                fixture.pins,
                fixture.commit,
                registry_proof="registry-proof.json",
                registry_proof_sha256=pin,
            )
        path.unlink()
        target = fixture.root / "archive-copy"
        target.write_bytes(original)
        path.symlink_to(target)
        with self.assertRaises(OSError):
            seam.produce(
                fixture.retained,
                fixture.pins,
                fixture.commit,
                registry_proof="registry-proof.json",
                registry_proof_sha256=pin,
            )

    def test_changed_proof_after_join_rejected(self) -> None:
        fixture = self.fixture
        _, pin = self.generate()
        original = seam.final_capture

        def mutate(*args, **kwargs):
            result = original(*args, **kwargs)
            (fixture.retained / "registry-proof.json").write_bytes(
                b"changed after join"
            )
            return result

        with patch.object(seam, "final_capture", side_effect=mutate):
            with self.assertRaises(ValueError):
                seam.produce(
                    fixture.retained,
                    fixture.pins,
                    fixture.commit,
                    registry_proof="registry-proof.json",
                    registry_proof_sha256=pin,
                )

    def test_archive_aggregate_cap_rejected(self) -> None:
        fixture = self.fixture
        value, pin = self.generate()
        with patch.object(
            proof, "MAX_ARCHIVES_TOTAL", value["packages"][0]["archive_size"] - 1
        ):
            with self.assertRaisesRegex(ValueError, "aggregate cap"):
                seam.produce(
                    fixture.retained,
                    fixture.pins,
                    fixture.commit,
                    registry_proof="registry-proof.json",
                    registry_proof_sha256=pin,
                )

    def test_selected_package_cap_rejected(self) -> None:
        with patch.object(proof, "MAX_PACKAGES", 0):
            with self.assertRaisesRegex(ValueError, "package cap"):
                self.generate()

    def test_cached_archive_checksum_and_symlink_rejected(self) -> None:
        fixture = self.fixture
        cache = (
            fixture.root
            / "registry/cache/index.crates.io-1949cf8c6b5b557f/link-cplusplus-1.0.12.crate"
        )
        cache.write_bytes(b"wrong checksum")
        with self.assertRaisesRegex(ValueError, "cache checksum"):
            self.generate()
        cache.unlink()
        target = fixture.root / "untrusted.crate"
        target.write_bytes(b"unknown")
        cache.symlink_to(target)
        with self.assertRaises(OSError):
            self.generate()

    def test_registry_archive_rejects_path_type_and_duplicates(self) -> None:
        import io
        import tarfile

        package = "link-cplusplus-1.0.12"
        for case in ("traversal", "prefix", "symlink", "duplicate"):
            raw = io.BytesIO()
            with tarfile.open(fileobj=raw, mode="w:gz") as tar:
                for _ in range(2 if case == "duplicate" else 1):
                    name = package + "/src/dummy.cc"
                    if case == "traversal":
                        name = package + "/../escape"
                    elif case == "prefix":
                        name = package + "-sibling/src/dummy.cc"
                    member = tarfile.TarInfo(name)
                    if case == "symlink":
                        member.type = tarfile.SYMTYPE
                        member.linkname = "/outside"
                    tar.addfile(member)
            with self.subTest(case=case), self.assertRaises(ValueError):
                proof.archive_units(raw.getvalue(), package, [], set())

    def test_probe_member_context_does_not_create_compiled_unit(self) -> None:
        import json

        fixture = receipts_tests.CaptureReceiptTests()
        self.addCleanup(fixture.doCleanups)
        fixture.setUp()
        fixture.final_profile_fixture(registry=True, probe_only=True)
        self.fixture = fixture
        observed = json.loads(
            (fixture.retained / "build-cxx-observed.json").read_bytes()
        )
        probe = observed["records"][-1]
        probe["original_argv"] = ["-E", "src/dummy.cc"]
        probe["argv"] = [probe["argv"][0], *probe["original_argv"]]
        fixture.write("build-cxx-observed.json", observed)
        fixture.pins["build-cxx-observed.json"] = fixture.digest(
            "build-cxx-observed.json"
        )
        sidecar, _ = self.generate()
        self.assertEqual(sidecar["packages"][0]["units"], [])

    def test_captured_cwd_nonselected_and_traversal_rejected(self) -> None:
        import json

        fixture = self.fixture
        observed = json.loads(
            (fixture.retained / "build-cxx-observed.json").read_bytes()
        )
        original = observed["records"][0]["cwd"]
        for suffix in ("-unknown", "/../link-cplusplus-1.0.12"):
            observed["records"][0]["cwd"] = original + suffix
            fixture.write("build-cxx-observed.json", observed)
            fixture.pins["build-cxx-observed.json"] = fixture.digest(
                "build-cxx-observed.json"
            )
            with self.subTest(suffix=suffix), self.assertRaises(ValueError):
                self.generate()

    def test_source_tar_rejects_duplicate_and_trailing(self) -> None:
        import io
        import tarfile

        fixture = self.fixture
        for duplicate in (False, True):
            output = io.BytesIO()
            with tarfile.open(fileobj=output, mode="w") as tar:
                for _ in range(2 if duplicate else 1):
                    member = tarfile.TarInfo("src/Cargo.lock")
                    member.size = 1
                    tar.addfile(member, io.BytesIO(b"x"))
            raw = output.getvalue() + (b"garbage" if not duplicate else b"")
            (fixture.retained / "source.tar").write_bytes(raw)
            with self.assertRaises(ValueError):
                proof.lock_bytes(
                    seam.RetainedReader(fixture.retained),
                    hashlib.sha256(raw).hexdigest(),
                )

    def test_lock_missing_or_nonselected_rejected(self) -> None:
        with patch.object(proof, "lock_bytes", return_value=b"version=4\npackage=[]\n"):
            with self.assertRaises(ValueError):
                self.generate()
        with patch.object(
            proof,
            "lock_bytes",
            side_effect=ValueError("MISSING: frozen source Cargo.lock"),
        ):
            with self.assertRaises(ValueError):
                self.generate()

    def test_duplicate_proof_field_rejected(self) -> None:
        _, pin = self.generate()
        fixture = self.fixture
        path = fixture.retained / "registry-proof.json"
        raw = path.read_bytes().replace(b"{", b'{"schema":"duplicate",', 1)
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            seam.produce(
                fixture.retained,
                fixture.pins,
                fixture.commit,
                registry_proof="registry-proof.json",
                registry_proof_sha256=hashlib.sha256(raw).hexdigest(),
            )


if __name__ == "__main__":
    unittest.main()
