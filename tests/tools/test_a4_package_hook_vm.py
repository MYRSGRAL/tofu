"""Guards hook-only VM: авторизация, настоящие hooks и однозначный Reload."""

import contextlib
import importlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
guest = importlib.import_module("a4_package_hook_guest")
host = importlib.import_module("a4_package_hook_vm")


class HookVmTests(unittest.TestCase):
    def test_approval_rejection_cannot_be_readiness_retry(self) -> None:
        with self.assertRaises(host.ApprovalRejected) as caught:
            host.reject_approval(
                ["ssh", "guest"],
                "",
                "Automatic approval review rejected action: forbidden scope",
            )
        self.assertIn("forbidden scope", str(caught.exception))
        self.assertNotIsInstance(caught.exception, RuntimeError)
        host.reject_approval(["ssh", "guest"], "", "Connection reset by peer")

    def test_decision_requires_exact_reviewed_source(self) -> None:
        decision = json.loads(Path(host.DECISION).read_text())
        host.validate_decision(decision)
        decision["source"] = "0" * 40
        with self.assertRaises(ValueError):
            host.validate_decision(decision)

    def test_cli_failure_is_nonzero_without_restarting_vm(self) -> None:
        with patch.object(host, "Driver") as driver:
            driver.return_value.failed = True
            self.assertEqual(host.main(), 1)
            driver.return_value.execute.assert_called_once()

    def test_failed_boot_preserves_overlay_without_live_vm_or_new_attempt(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-cleanup-test-") as directory:
            root = Path(directory)
            driver = host.Driver.__new__(host.Driver)
            driver.task = root / "task"
            driver.task.mkdir()
            driver.out = root / "evidence"
            driver.out.mkdir(mode=0o700)
            driver.vm = SimpleNamespace(poll=lambda: 0, returncode=0)
            driver.failed = True
            (driver.task / "disk.qcow2").write_bytes(b"own-fixture-overlay")
            (driver.task / "serial.log").write_text("fixture boot failure")
            with (
                patch.object(host, "references", return_value=[{"kind": "unit-test"}]),
                patch.object(
                    host,
                    "sha",
                    return_value="794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003",
                ),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                driver.cleanup()
            self.assertEqual(
                (driver.out / "blocked-disk.qcow2").read_bytes(), b"own-fixture-overlay"
            )
            self.assertEqual(
                (driver.out / "serial-before-shutdown.log").read_text(),
                "fixture boot failure",
            )

    def test_hooks_reject_changed_exec(self) -> None:
        hook = "[Trigger]\nType = Path\nOperation = Install\nOperation = Upgrade\nOperation = Remove\nTarget = usr/lib/systemd/system/*\n[Action]\nDescription = Reloading system manager configuration...\nWhen = PostTransaction\nExec = /usr/share/libalpm/scripts/systemd-hook daemon-reload-system\n"
        guest.validate_reload_hook(hook)
        with self.assertRaises(ValueError):
            guest.validate_reload_hook(
                hook.replace("systemd-hook daemon", "fake daemon")
            )
        with self.assertRaises(ValueError):
            guest.validate_reload_hook(hook.replace("Operation = Remove\n", ""))

    def test_real_relevant_script_branches_and_guards(self) -> None:
        text = Path("/usr/share/libalpm/scripts/systemd-hook").read_text()
        guest.validate_script(text)
        with self.assertRaises(ValueError):
            guest.validate_script(
                text.replace(
                    "/usr/bin/systemctl --system daemon-reload", "/usr/bin/false"
                )
            )
        with self.assertRaises(ValueError):
            guest.validate_script(
                text.replace("if ! systemd-notify --booted; then", "if false; then")
            )

    def test_reload_requires_one_call_and_complete_signal_pair(self) -> None:
        messages = [
            {
                "type": "method_call",
                "interface": "org.freedesktop.systemd1.Manager",
                "member": "Reload",
                "path": "/org/freedesktop/systemd1",
                "sender": ":1.40",
            },
            {
                "type": "signal",
                "interface": "org.freedesktop.systemd1.Manager",
                "member": "Reloading",
                "payload": {"data": [True]},
            },
            {
                "type": "signal",
                "interface": "org.freedesktop.systemd1.Manager",
                "member": "Reloading",
                "payload": {"data": [False]},
            },
        ]
        self.assertEqual(guest.validate_reload_messages(messages)["reload_calls"], 1)
        with self.assertRaises(ValueError):
            guest.validate_reload_messages(messages[:-1])
        with self.assertRaises(ValueError):
            guest.validate_reload_messages(messages + [messages[0]])

    def test_any_product_job_is_failure(self) -> None:
        with self.assertRaises(ValueError):
            guest.reject_product_jobs(
                [
                    {
                        "member": "JobNew",
                        "payload": {"data": [4, "/job/4", "yonik-worker.service"]},
                    }
                ]
            )
        guest.reject_product_jobs(
            [{"member": "JobNew", "payload": {"data": [4, "/job/4", "dbus.service"]}}]
        )

    def test_bwrap_or_host_cannot_be_guest(self) -> None:
        with self.assertRaises(ValueError):
            guest.require_identity(
                "not-QEMU", "/usr/lib/systemd/systemd", "cgroup2fs", "nonce", "nonce"
            )
        with self.assertRaises(ValueError):
            guest.require_identity(
                "QEMU", "/usr/bin/python", "cgroup2fs", "nonce", "nonce"
            )
        with self.assertRaises(ValueError):
            guest.require_identity(
                "QEMU", "/usr/lib/systemd/systemd", "cgroup2fs", "wrong", "nonce"
            )
