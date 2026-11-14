"""Отказные gates P01 без GUI, VM и длительных прогонов."""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import p01_campaign_host as host
from p01_observer_measure import verify_frame_canary, verify_phase


class PreflightTests(unittest.TestCase):
    def test_broken_frame_oracle_is_rejected(self) -> None:
        before = {"visible": True, "frames": 2}
        with self.assertRaises(RuntimeError):
            verify_frame_canary(before, before)
        verify_frame_canary(before, {"visible": True, "frames": 3})

    def test_visibility_transitions_are_rejected_in_both_phases(self) -> None:
        for phase, visible in (("hidden", False), ("static-open", True)):
            before = {"visible": visible, "frames": 0, "visibility_events": 2}
            verify_phase(phase, before, before)
            for after in (
                dict(before, visible=not visible),
                dict(before, visibility_events=4),
            ):
                with (
                    self.subTest(phase=phase, after=after),
                    self.assertRaises(RuntimeError),
                ):
                    verify_phase(phase, before, after)

    def test_hidden_frames_are_rejected(self) -> None:
        before = {"visible": False, "frames": 0, "visibility_events": 2}
        with self.assertRaises(RuntimeError):
            verify_phase("hidden", before, dict(before, frames=1))

    def test_connection_schema_rejects_malformed_values(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "connection.json"
            for value in (
                {"ssh": "invalid", "vm_pid": 123},
                {"ssh": ["ssh"], "vm_pid": True},
                {"ssh": ["ssh", 1], "vm_pid": 123},
                {"ssh": [], "vm_pid": 123},
                {"ssh": ["ssh"], "vm_pid": -1},
            ):
                path.write_text(json.dumps(value))
                with self.subTest(value=value), self.assertRaises(ValueError):
                    host.read_connection(path)
            value = {"ssh": ["ssh", "fixture"], "vm_pid": 123}
            path.write_text(json.dumps(value))
            self.assertEqual(host.read_connection(path), value)

    def test_setup_failure_still_finishes_and_waits_vm(self) -> None:
        for malformed, slow_stop in (
            (False, False),
            (True, False),
            (False, True),
            ("schema", False),
            ("harvest", False),
        ):
            with (
                self.subTest(malformed=malformed),
                tempfile.TemporaryDirectory() as tmp,
            ):
                root = Path(tmp) / "vm"
                root.mkdir()
                output = Path(tmp) / "evidence"
                output.mkdir()
                if malformed:
                    (root / "connection.json").write_text(
                        json.dumps({"ssh": ["ssh"], "vm_pid": 99999999})
                        if malformed == "harvest"
                        else json.dumps({"ssh": "invalid", "vm_pid": 123})
                        if malformed == "schema"
                        else "broken"
                    )
                vm = Mock()
                vm.poll.return_value = None
                if slow_stop:
                    vm.wait.side_effect = [host.subprocess.TimeoutExpired("vm", 60), 0]
                else:
                    vm.wait.return_value = 0
                with (
                    patch.object(host, "FIRST", root),
                    patch.object(host, "EVIDENCE", output),
                    patch.object(host.subprocess, "Popen", return_value=vm),
                    patch.object(host.time, "monotonic", side_effect=[0, 601]),
                    patch.object(host, "call", side_effect=TypeError("harvest error")),
                    self.assertRaises((TimeoutError, ValueError, TypeError)),
                ):
                    host.main()
                self.assertTrue((root / "finish").exists())
                if slow_stop:
                    self.assertEqual(vm.wait.call_count, 2)
                    vm.wait.assert_called_with(timeout=360)
                    vm.send_signal.assert_called_once_with(host.signal.SIGINT)
                else:
                    vm.wait.assert_called_once_with(timeout=60)
                state = json.loads((output / "campaign-terminal.json").read_text())
                self.assertEqual(state["state"], "FAILED")
                self.assertEqual(state["preserved_root"], str(root))


if __name__ == "__main__":
    unittest.main()
