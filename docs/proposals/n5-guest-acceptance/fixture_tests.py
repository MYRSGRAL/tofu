"""Только pure DATA и собственные временные файлы; никаких сервисов/probes."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "n5_fixture", Path(__file__).with_name("fixture.py")
)
assert SPEC and SPEC.loader
fixture = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = fixture
SPEC.loader.exec_module(fixture)


def digest(blob: bytes) -> str:
    return hashlib.sha256(blob).hexdigest()


def inputs() -> tuple[dict, dict[str, bytes]]:
    """Модель не устанавливается и не выдаёт production authority."""
    source = b"pure archive model"
    m = {
        "schema": fixture.SCHEMA,
        "source_commit": "1" * 40,
        "source_archive_sha256": digest(source),
        "guest_base_sha256": "2" * 64,
        "guest_provenance_sha256": "3" * 64,
        "kernel_build_id": "4" * 40,
        "coverage_policy_sha256": fixture.COVERAGE_POLICY_SHA256,
        "artifacts": {},
        "units": dict(fixture.UNIT_PINS),
        "profile": {},
        "environment": {
            "pid1_sha256": "6" * 64,
            "worker_uid": 973,
            "worker_gid": 973,
            "topology_sha256": digest(b"mock topology raw"),
            "uid_caps_sha256": digest(b"mock uid caps raw"),
            "cgroup_controllers_sha256": digest(b"mock cgroup raw"),
        },
    }
    files = {
        "source.tar": source,
        **{
            unit: blob.encode()
            for unit, blob in json.loads(
                Path(__file__).with_name("canonical-units.json").read_text()
            ).items()
        },
    }
    for name, (filename, installed, mode) in fixture.FILES.items():
        blob = bytearray(64)
        blob[:7] = b"\x7fELF\x02\x01\x01"
        blob[18:20] = b"\x3e\x00"
        blob[30 : 30 + len(name)] = name.encode()
        own_source = (
            ("upstream-" + name).encode() if name in ("core_a", "core_b") else source
        )
        own_commit = (
            ("a" if name == "core_a" else "b") * 40
            if name in ("core_a", "core_b")
            else m["source_commit"]
        )
        if name in ("core_a", "core_b"):
            files[filename + ".source.tar"] = own_source
        receipt = fixture.canonical(
            {
                "schema": "SOURCE_BOUND_ELF_V1",
                "source_commit": own_commit,
                "source_archive_sha256": digest(own_source),
                "elf_sha256": digest(blob),
            }
        )
        m["artifacts"][name] = {
            "sha256": digest(blob),
            "size": len(blob),
            "source_archive_sha256": digest(own_source),
            "source_commit": own_commit,
            "build_receipt_sha256": digest(receipt),
            "installed_path": installed,
            "uid": 0,
            "gid": 0,
            "mode": mode,
        }
        files[filename], files[filename + ".receipt.json"] = bytes(blob), receipt
    m["profile"] = {
        "schema": 1,
        "worker_sha256": list(bytes.fromhex(m["artifacts"]["worker"]["sha256"])),
        "source_archive_sha256": list(bytes.fromhex(m["source_archive_sha256"])),
        "policy_id": fixture.POLICY,
        "policy_sha256": list(bytes.fromhex(m["coverage_policy_sha256"])),
        "kernel_release": "6.18.0-fixture",
        "kernel_build_id": list(bytes.fromhex(m["kernel_build_id"])),
        "kernel_machine": "x86_64",
        "kernel_abi": fixture.ABI,
    }
    base = b"mock fresh guest image DATA, never executed"
    m["guest_base_sha256"] = digest(base)
    provenance = fixture.canonical(
        {
            "schema": "FRESH_GUEST_BASE_V1",
            "base_sha256": digest(base),
            "origin": "TRUSTED_FRESH_BASE",
            "lineage": "NEW_ACCEPTANCE_N5",
            "state": "NEVER_RUN",
        }
    )
    m["guest_provenance_sha256"] = digest(provenance)
    files["guest-base.img"] = base
    files["guest-base.receipt.json"] = provenance
    return m, files


def receipt(m: dict) -> dict:
    files = {}
    for name in fixture.FILES:
        a = m["artifacts"][name]
        files[name] = {
            key: a[key]
            for key in ("sha256", "size", "installed_path", "uid", "gid", "mode")
        }
        files[name].update(
            kind="regular",
            ancestors=[
                {
                    "path": str(path),
                    "uid": 0,
                    "gid": 0,
                    "mode": 0o755,
                    "kind": "directory",
                }
                for path in Path(a["installed_path"]).parents
            ],
        )
    for name, path, sha in [
        ("source", "/usr/lib/yonik/native-source-v1.tar", m["source_archive_sha256"]),
        (
            "profile",
            "/usr/lib/yonik/native-profile-v1.json",
            digest(fixture.canonical(m["profile"])),
        ),
    ]:
        files[name] = {
            "installed_path": path,
            "sha256": sha,
            "uid": 0,
            "gid": 0,
            "mode": 0o444,
            "kind": "regular",
        }
    env = m["environment"]
    return {
        "schema": fixture.RECEIPT,
        "manifest_sha256": digest(fixture.canonical(m)),
        "guest_base_sha256": m["guest_base_sha256"],
        "guest_provenance_sha256": m["guest_provenance_sha256"],
        "setup_finished_ns": 10,
        "captured_ns": 11,
        "source_archive_sha256": m["source_archive_sha256"],
        "kernel": {
            "build_id": m["kernel_build_id"],
            "release": m["profile"]["kernel_release"],
            "machine": "x86_64",
        },
        "files": files,
        "units": m["units"],
        "profile": m["profile"],
        "profile_registration": {
            "scope": "OWN_GUEST_ONLY",
            "grant_manifest_sha256": digest(fixture.canonical(m)),
            "grant_source_sha256": m["source_archive_sha256"],
        },
        "pid1": {
            "pid": 1,
            "uid": 0,
            "comm": "systemd",
            "exe_sha256": env["pid1_sha256"],
            "start": 1,
        },
        "system_bus": {
            "address": "unix:path=/run/dbus/system_bus_socket",
            "uid": 0,
            "pid": 2,
            "start": 2,
            "socket_inode": 5,
            "owner": ":1.4",
        },
        "identities": {
            "helper_uid": 0,
            "worker_uid": 973,
            "worker_gid": 973,
            "uid_caps_sha256": env["uid_caps_sha256"],
            "capture": "mock uid caps raw",
        },
        "cgroup": {
            "version": 2,
            "helper": "/system.slice/yonik-helper.service",
            "worker": "/system.slice/yonik-worker.service",
            "controllers_sha256": env["cgroup_controllers_sha256"],
            "capture": "mock cgroup raw",
        },
        "topology": {
            "sha256": env["topology_sha256"],
            "capture": "mock topology raw",
            "network_namespace": [1, 2],
            "pid_namespace": [1, 3],
            "mount_namespace": [1, 4],
        },
    }


class FixtureTests(unittest.TestCase):
    def test_complete_model_and_missing_capture(self) -> None:
        m, _ = inputs()
        self.assertEqual(fixture.validate_receipt(m, receipt(m)).status, "READY")
        self.assertEqual(fixture.validate_receipt(m, None).status, "MISSING")

    def test_null_pins_cannot_pass(self) -> None:
        m, _ = inputs()
        m["guest_base_sha256"] = None
        self.assertEqual(fixture.preflight(m).status, "MISSING")

    def test_unknown_fields_and_bool_integer_rejected(self) -> None:
        for mutation in (
            lambda m: m.update(command="true"),
            lambda m: m["artifacts"]["helper"].update(uid=False),
            lambda m: m["profile"].update(kernel_build_id=[True] * 20),
        ):
            m, _ = inputs()
            mutation(m)
            self.assertEqual(fixture.preflight(m).status, "REJECTED")

    def test_receipt_mutations_fail_closed(self) -> None:
        m, _ = inputs()
        mutations = [
            lambda r: r.update(manifest_sha256="f" * 64),
            lambda r: r.update(captured_ns=10),
            lambda r: r["pid1"].update(comm="init"),
            lambda r: r["system_bus"].update(address="session"),
            lambda r: r["kernel"].update(build_id="f" * 40),
            lambda r: r["files"]["worker"].update(mode=0o755),
            lambda r: r["files"]["worker"].update(kind="symlink"),
            lambda r: r["files"]["source"].update(mode=0o644),
            lambda r: r["identities"].update(worker_uid=0),
            lambda r: r["cgroup"].update(version=1),
            lambda r: r["topology"].update(sha256="f" * 64),
            lambda r: r["profile_registration"].update(scope="HOST"),
            lambda r: r.update(extra=1),
        ]
        for mutation in mutations:
            r = copy.deepcopy(receipt(m))
            mutation(r)
            self.assertEqual(fixture.validate_receipt(m, r).status, "REJECTED")

    def test_partial_capture_and_missing_inputs_are_missing(self) -> None:
        m, _ = inputs()
        r = receipt(m)
        del r["kernel"]
        self.assertEqual(fixture.validate_receipt(m, r).status, "MISSING")
        r = receipt(m)
        r["kernel"]["build_id"] = None
        self.assertEqual(fixture.validate_receipt(m, r).status, "MISSING")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(
                fixture.stage_inputs(m, root / "absent", root / "stage").status,
                "MISSING",
            )

    def test_numeric_type_and_untrusted_ancestor_rejected(self) -> None:
        m, _ = inputs()
        for value in [True, 1.0]:
            r = receipt(m)
            r["pid1"]["pid"] = value
            self.assertEqual(fixture.validate_receipt(m, r).status, "REJECTED")
        r = receipt(m)
        r["files"]["worker"]["ancestors"][0]["mode"] = 0o777
        self.assertEqual(fixture.validate_receipt(m, r).status, "REJECTED")

    def test_setup_plan_is_closed_data_and_missing_never_provisions(self) -> None:
        m, _ = inputs()
        steps = fixture.setup_plan(m)
        self.assertEqual(len(steps), len(fixture.FILES) + len(fixture.UNITS) + 2)
        for step in steps:
            self.assertEqual(step["argv"][0], "/usr/bin/install")
            self.assertEqual(step["timeout_seconds"], 10)
            self.assertNotIn("env", step)
        m["guest_base_sha256"] = None
        self.assertEqual(fixture.setup_plan(m), ())

    def test_duplicate_json_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "duplicate.json"
            path.write_text('{"a":1,"a":2}')
            with self.assertRaises(fixture.Invalid):
                fixture.load(path)

    def test_stage_owned_files_without_execution(self) -> None:
        m, data = inputs()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "in"
            source.mkdir()
            for name, blob in data.items():
                (source / name).write_bytes(blob)
            target = root / "sealed"
            self.assertEqual(fixture.stage_inputs(m, source, target).status, "READY")
            self.assertEqual((target / "yonik-worker").stat().st_mode & 0o777, 0o400)
            self.assertEqual(fixture.stage_inputs(m, source, target).status, "REJECTED")

    def test_missing_fresh_base_never_seals_stage(self) -> None:
        m, data = inputs()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, blob in data.items():
                if name != "guest-base.img":
                    (root / name).write_bytes(blob)
            result = fixture.stage_inputs(m, root, root / "stage")
            self.assertEqual(result.status, "MISSING")
            self.assertFalse((root / "stage" / "SEALED_GUEST_FIXTURE_V1.json").exists())

    def test_forged_source_receipt_and_old_base_are_rejected(self) -> None:
        for attack in ("source", "base"):
            m, data = inputs()
            if attack == "source":
                name = "xray.receipt.json"
                record = json.loads(data[name])
                record["source_commit"] = "f" * 40
                data[name] = fixture.canonical(record)
                m["artifacts"]["core_b"]["build_receipt_sha256"] = digest(data[name])
            else:
                name = "guest-base.receipt.json"
                record = json.loads(data[name])
                record["lineage"] = "OLD_FAILED_A4"
                data[name] = fixture.canonical(record)
                m["guest_provenance_sha256"] = digest(data[name])
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                for name, blob in data.items():
                    (root / name).write_bytes(blob)
                self.assertEqual(
                    fixture.stage_inputs(m, root, root / "stage").status, "REJECTED"
                )
                self.assertFalse(
                    (root / "stage" / "SEALED_GUEST_FIXTURE_V1.json").exists()
                )

    def test_arbitrary_units_and_policy_never_pass(self) -> None:
        for section, key in (
            ("units", "yonik-worker.service"),
            (None, "coverage_policy_sha256"),
        ):
            m, _ = inputs()
            target = m[section] if section else m
            target[key] = "f" * 64
            self.assertEqual(fixture.preflight(m).status, "REJECTED")

    def test_closure_echo_without_actual_capture_is_rejected(self) -> None:
        m, _ = inputs()
        for section in ("topology", "identities", "cgroup"):
            r = receipt(m)
            r[section]["capture"] = "expected hash echoed without raw facts"
            self.assertEqual(fixture.validate_receipt(m, r).status, "REJECTED")

    def test_stage_rejects_wrong_source_and_symlink(self) -> None:
        m, data = inputs()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, blob in data.items():
                (root / name).write_bytes(blob)
            (root / "source.tar").write_bytes(b"wrong")
            self.assertEqual(
                fixture.stage_inputs(m, root, root / "out").status, "REJECTED"
            )
            (root / "source.tar").unlink()
            (root / "source.tar").symlink_to(root / "yonik-worker")
            self.assertEqual(
                fixture.stage_inputs(m, root, root / "out").status, "REJECTED"
            )


if __name__ == "__main__":
    unittest.main()
