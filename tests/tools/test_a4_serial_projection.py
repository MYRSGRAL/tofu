"""Design-only projection не выводит произвольные definitions/manager payload."""

import importlib
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
p = importlib.import_module("a4_serial_projection")


class SerialProjectionTests(unittest.TestCase):
    def test_definition_drops_secrets_and_unknown_dependencies(self) -> None:
        raw = b"[Unit]\nAfter=network.target secret-marker.service\nDescription=private-marker\n[Service]\nEnvironment=TOKEN=secret-marker\nExecStart=/bin/sh secret-marker\n"
        result = p.project_definition("sshd.service", raw)
        encoded = json.dumps(result)
        self.assertNotIn("secret-marker", encoded)
        self.assertNotIn("private-marker", encoded)
        self.assertEqual(result["relations"]["After"], ["network.target"])
        self.assertEqual(result["redacted_dependency_tokens"], 1)
        self.assertFalse(result["graph_complete"])
        self.assertEqual(len(result["sha256"]), 64)

    def test_invalid_truncated_and_oversize_cannot_be_definition(self) -> None:
        for raw in (b"", b"[Unit]\nAfter=x", b"x" * 16385, b"[Unit]\n\0\n", b"\xff\n"):
            with self.subTest(raw=raw[:20]):
                self.assertTrue(
                    p.project_definition("sshd.service", raw)["status"].startswith(
                        "RED"
                    )
                )

    def test_closed_unit_allowlist(self) -> None:
        for unit in ("../../etc/shadow", "yonik-helper.service", "arbitrary.service"):
            with self.assertRaises(ValueError):
                p.project_definition(unit, b"[Unit]\nAfter=network.target\n")

    def test_properties_drop_unlisted_and_malformed_fields(self) -> None:
        result = p.project_properties(
            "sshd.service",
            {
                "ActiveState": "activating",
                "Job": "42 /org/freedesktop/systemd1/job/42",
                "Environment": "secret-marker",
                "After": "network.target secret-marker",
                "SubState": "secret-marker",
            },
        )
        self.assertNotIn("secret-marker", json.dumps(result))
        self.assertEqual(result["fields"]["Job"], 42)
        self.assertGreater(result["unknown_fields"], 0)

    def test_jobs_closed_projection_no_foreign_unit_or_payload(self) -> None:
        raw = {
            "unit": "sshd.service",
            "id": 42,
            "type": "start",
            "state": "waiting",
            "payload": "secret-marker",
        }
        self.assertNotIn("secret-marker", json.dumps(p.project_job(raw)))
        for delta in (
            {"unit": "yonik-worker.service"},
            {"id": True},
            {"state": "secret-marker"},
        ):
            self.assertIsNone(p.project_job(raw | delta))

    def test_definition_reset_and_continuation(self) -> None:
        raw = b"[Unit]\nAfter=network.target\nAfter=\nAfter=time-sync.target \\\n network-online.target\n"
        result = p.project_definition("sshd.service", raw)
        self.assertEqual(
            result["relations"]["After"], ["network-online.target", "time-sync.target"]
        )

    def test_proposal_is_nonexecuting_and_keeps_existing_boot_sequence(self) -> None:
        root = Path(__file__).resolve().parents[2]
        proposal = json.loads(
            (
                root
                / "docs/design/a4-package-hook-proposal-142/serial-only-diagnostic-next.json"
            ).read_text()
        )
        self.assertFalse(proposal["boot_allowed_now"])
        self.assertEqual(proposal["execution_count"], 0)
        self.assertEqual(proposal["limits"]["ssh_attempts"], 0)
        self.assertEqual(proposal["limits"]["snapshot_count"], 2)
        diagnostic = importlib.import_module("a4_bootstrap_diagnostic")
        driver = diagnostic.DiagnosticDriver.__new__(diagnostic.DiagnosticDriver)
        driver.nonce = "a" * 32
        commands = driver.cloud_config("fixture-public")["bootcmd"]
        self.assertEqual(commands[6], ["systemctl", "enable", "--now", "sshd.service"])
        self.assertEqual(sum(argv[0] == "ln" for argv in commands[:6]), 5)
