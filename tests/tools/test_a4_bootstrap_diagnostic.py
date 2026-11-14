"""Diagnostic-only boot не может дойти до package callback."""

import importlib
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
diag = importlib.import_module("a4_bootstrap_diagnostic")


class BootstrapDiagnosticTests(unittest.TestCase):
    def test_root_decision_exact_source_scope_and_one_boot(self) -> None:
        value = json.loads(diag.DIAGNOSTIC_DECISION.read_text())
        diag.validate_diagnostic_decision(value)
        value["limits"]["attempts"] = 2
        with self.assertRaises(ValueError):
            diag.validate_diagnostic_decision(value)

    def test_seed_masks_before_early_ssh_and_no_runcmd_duplicates(self) -> None:
        driver = diag.DiagnosticDriver.__new__(diag.DiagnosticDriver)
        driver.nonce = "a" * 32
        config = driver.cloud_config("ssh-ed25519 fixture-public")
        commands = config["bootcmd"]
        masks = [x for x in commands if x[0] == "ln"]
        self.assertEqual(len(masks), 5)
        ssh = commands.index(["systemctl", "enable", "--now", "sshd.service"])
        self.assertTrue(all(commands.index(mask) < ssh for mask in masks))
        self.assertFalse(config.get("runcmd"))
        self.assertNotIn("packages", config)
        self.assertFalse(config["package_update"])

    def test_workload_never_dispatches_package_phases(self) -> None:
        driver = diag.DiagnosticDriver.__new__(diag.DiagnosticDriver)
        driver.diagnostic_captured = True
        driver.guest = Mock(side_effect=AssertionError("package dispatch prohibited"))
        driver.workload({})
        driver.guest.assert_not_called()
        result = driver.success_result()
        self.assertFalse(result["package_ready"])
        self.assertFalse(result["live_package_hooks"])
        self.assertTrue(driver.keep_overlay())

    def test_readonly_collection_has_no_package_or_service_mutation(self) -> None:
        commands = diag.diagnostic_commands()
        joined = "\n".join(" ".join(argv) for argv in commands)
        for forbidden in [
            "pacman",
            " stop ",
            " mask ",
            "daemon-reload",
            "timedatectl",
            "adjtimex",
        ]:
            self.assertNotIn(forbidden, joined)
        self.assertIn("cloud-final.service", joined)
        self.assertIn("systemd-time-wait-sync.service", joined)
        self.assertIn("journalctl", joined)

    def test_package_command_rejected_before_any_ssh_call(self) -> None:
        driver = diag.DiagnosticDriver.__new__(diag.DiagnosticDriver)
        for command in [
            "pacman -U candidate.tar",
            "systemctl stop systemd-time-wait-sync.service",
            "systemctl daemon-reload",
        ]:
            with self.assertRaises(ValueError):
                driver.guest(command)

    def test_identity_mismatch_prevents_graph_collection(self) -> None:
        driver = diag.DiagnosticDriver.__new__(diag.DiagnosticDriver)
        driver.nonce = "a" * 32
        driver.guest = Mock(return_value="host-not-QEMU")
        with self.assertRaises(ValueError):
            driver.prepare_guest({})
        self.assertFalse(getattr(driver, "diagnostic_captured", False))
