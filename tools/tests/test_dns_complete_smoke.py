"""Проверки oracle и обязательного изолированного launcher."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import dns_complete_smoke as probe  # noqa: E402


class CompleteDnsTests(unittest.TestCase):
    def test_partial_failure_is_rejected_only_in_strict_mode(self) -> None:
        for case in probe.CASES:
            strict = probe.expected_marker(case, True)
            default = probe.expected_marker(case, False)
            if case.name in {"a_servfail", "aaaa_servfail", "aaaa_nxdomain"}:
                self.assertFalse(strict)
                self.assertTrue(default)
            elif case.name in {"a_nodata", "aaaa_nodata", "ipv4_only", "ipv6_only"}:
                self.assertTrue(strict)
                self.assertTrue(default)

    def test_strict_flag_is_absent_from_upstream_control(self) -> None:
        case = probe.CASES[0]
        for explicit in (False, True):
            baseline = probe.configuration(case, None, explicit, True)
            strict = probe.configuration(case, True, explicit, True)
            self.assertNotIn("strict_complete", baseline["route"]["rules"][0])
            self.assertTrue(strict["route"]["rules"][0]["strict_complete"])
            self.assertEqual(explicit, "server" in strict["route"]["rules"][0])

    def test_manifest_is_bound_to_actual_binary_and_patch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "engine"
            binary.write_bytes(b"synthetic-binary")
            manifest = Path(directory) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "engine": "sing-box",
                        "source_commit": probe.SOURCE_COMMIT,
                        "binary_sha256": probe.digest(binary),
                        "patch_sha256": probe.digest(probe.PATCH),
                        "adapter_features": {"strict_complete_dns": True},
                        "strict_dns_gate": False,
                    }
                )
            )
            self.assertEqual(probe.validate_artifact(binary, manifest), binary)
            binary.write_bytes(b"changed-binary")
            with self.assertRaisesRegex(ValueError, "бинарник"):
                probe.validate_artifact(binary, manifest)
            manifest.write_text("[]")
            with self.assertRaisesRegex(ValueError, "manifest"):
                probe.validate_artifact(binary, manifest)

    def test_launcher_always_creates_namespaces(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "engine"
            binary.touch()
            with (
                patch.object(probe, "validate_artifact", return_value=binary),
                patch.object(probe, "validated_binary", return_value=binary),
                patch.object(probe, "run_sandbox") as launch,
                patch.object(
                    sys,
                    "argv",
                    [
                        "probe",
                        str(binary),
                        str(binary),
                        "--manifest",
                        str(root / "manifest"),
                        "--output",
                        str(root / "evidence"),
                    ],
                ),
            ):
                self.assertEqual(probe.main(), 0)
            command = launch.call_args.args[0]
            for flag in (
                "--net",
                "--user",
                "--map-root-user",
                "--keep-caps",
                "--pid",
                "--fork",
                "--mount",
                "--mount-proc",
            ):
                self.assertIn(flag, command)
            self.assertNotIn("--worker-namespace", command)

    def test_public_worker_bypass_is_rejected(self) -> None:
        with (
            patch.object(
                sys,
                "argv",
                [
                    "probe",
                    "unused",
                    "unused",
                    "--manifest",
                    "unused",
                    "--worker-namespace",
                    "1",
                ],
            ),
            patch.object(probe, "run_sandbox") as launch,
        ):
            with self.assertRaises(SystemExit):
                probe.main()
            launch.assert_not_called()
