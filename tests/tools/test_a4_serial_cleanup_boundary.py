"""Регрессии cleanup: только собственный Python child и mock collector, без VM."""

from __future__ import annotations

import importlib
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
driver = importlib.import_module("a4_serial_driver")
collector = importlib.import_module("a4_serial_collector")


class CleanupBoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        collector._alarm_expired = False

    def tearDown(self) -> None:
        collector._alarm_expired = False

    def test_alarm_during_pipe_setup_still_reaps_child(self) -> None:
        process = Mock()
        process.poll.return_value = None

        def alarm_at_fileno() -> int:
            collector._deadline_expired(signal.SIGALRM, None)
            return 42

        process.stdout.fileno.side_effect = alarm_at_fileno
        with (
            patch.object(collector, "_alarm_expired", False, create=True),
            patch.object(collector.subprocess, "Popen", return_value=process),
            patch.object(collector.os, "set_blocking"),
        ):
            try:
                collector._command("jobs", time.monotonic() + 5)
            except TimeoutError:
                pass
        process.kill.assert_called_once_with()
        process.wait.assert_called_once_with(timeout=1)
        process.stdout.close.assert_called_once_with()

    def test_alarm_during_cleanup_does_not_interrupt_reap(self) -> None:
        process = Mock()
        process.stdout.fileno.return_value = 42

        def alarm_at_poll() -> None:
            collector._deadline_expired(signal.SIGALRM, None)
            return None

        process.poll.side_effect = alarm_at_poll
        with (
            patch.object(collector, "_alarm_expired", False, create=True),
            patch.object(collector.subprocess, "Popen", return_value=process),
            patch.object(collector.os, "set_blocking"),
            patch.object(
                collector.select, "select", return_value=([process.stdout], [], [])
            ),
            patch.object(collector.os, "read", return_value=b"x" * 32769),
        ):
            try:
                collector._command("jobs", time.monotonic() + 5)
            except TimeoutError:
                pass
        process.kill.assert_called_once_with()
        process.wait.assert_called_once_with(timeout=1)
        process.stdout.close.assert_called_once_with()

    def test_open_pipe_cannot_hold_capture_after_leader_exit(self) -> None:
        process = subprocess.Popen(
            [sys.executable, "-c", "pass"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        assert process.stdout is not None
        process.stdout.close()
        reader, writer = os.pipe()
        process.stdout = os.fdopen(reader, "rb", buffering=0)
        began = time.monotonic()
        try:
            _stdout, _stderr, failure = driver._capture_host(process, 0.05)
            self.assertEqual(failure, "timeout")
            self.assertIsNotNone(process.returncode)
            self.assertLess(time.monotonic() - began, 2)
        finally:
            os.close(writer)
            if process.poll() is None:
                process.kill()
            process.wait(timeout=2)

    def test_reap_timeout_is_reported_and_both_waits_bounded(self) -> None:
        process = Mock()
        readers = []
        for _ in range(2):
            reader, writer = os.pipe()
            os.close(writer)
            readers.append(os.fdopen(reader, "rb", buffering=0))
        process.stdout, process.stderr = readers
        process.returncode = None
        process.wait.side_effect = subprocess.TimeoutExpired("own fixture", 0.1)
        with patch.object(driver.os, "killpg") as kill_group:
            _stdout, _stderr, failure = driver._capture_host(process, 0.05)
        self.assertEqual(failure, "reap_timeout")
        self.assertEqual(process.wait.call_count, 2)
        self.assertTrue(
            all(call.kwargs["timeout"] > 0 for call in process.wait.call_args_list)
        )
        self.assertEqual(process.wait.call_args.kwargs, {"timeout": 1})
        kill_group.assert_called_once_with(process.pid, signal.SIGKILL)
        self.assertTrue(all(reader.closed for reader in readers))

    def test_host_output_overflow_rejects_and_caps_retained_bytes(self) -> None:
        instance = driver.SerialDriver()
        instance.started = time.monotonic()
        instance._record = Mock()
        original = subprocess.Popen
        children: list[subprocess.Popen[bytes]] = []

        def spawn(_argv: list[str], **kwargs: object) -> subprocess.Popen[bytes]:
            process = original(
                [sys.executable, "-c", "import sys; sys.stdout.write('x' * 70000)"],
                **kwargs,
            )
            children.append(process)
            return process

        try:
            with patch.object(driver.subprocess, "Popen", side_effect=spawn):
                with self.assertRaises(ValueError):
                    instance._host("gpg-import")
            record = instance._record.call_args.args[1]
            self.assertLessEqual(len(record["stdout"]) + len(record["stderr"]), 65536)
            self.assertEqual(record["capture_failure"], "overflow")
            self.assertIsNotNone(children[0].returncode)
        finally:
            for process in children:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=2)

    def test_alarm_marks_deadline_without_interrupting_ownership(self) -> None:
        with (
            tempfile.TemporaryFile() as serial,
            patch.object(collector, "_identity", return_value="own-test-boot"),
            patch.object(collector, "_contract", return_value={}),
            patch.object(collector, "Emitter"),
            patch.object(collector, "_snapshot", side_effect=ValueError("fixture")),
            patch.object(collector.os, "open", return_value=os.dup(serial.fileno())),
            patch.object(collector.os, "_exit") as immediate_exit,
            patch.object(collector.signal, "alarm"),
            patch.object(collector.signal, "signal") as install,
        ):
            self.assertEqual(collector.main(), 1)
            handler = install.call_args.args[1]
            handler(signal.SIGALRM, None)
            self.assertTrue(collector._alarm_expired)
            immediate_exit.assert_not_called()

    def test_alarm_during_command_read_kills_and_reaps(self) -> None:
        process = Mock()
        process.stdout.fileno.return_value = 42
        process.poll.return_value = None
        with (
            patch.object(collector.subprocess, "Popen", return_value=process),
            patch.object(collector.os, "set_blocking"),
            patch.object(collector.select, "select", side_effect=TimeoutError("alarm")),
        ):
            with self.assertRaises(TimeoutError):
                collector._command("jobs", time.monotonic() + 5)
        process.kill.assert_called_once_with()
        process.wait.assert_called_once_with(timeout=1)
        process.stdout.close.assert_called_once_with()

    def test_alarm_during_command_wait_kills_and_reaps(self) -> None:
        process = Mock()
        process.stdout.fileno.return_value = 42
        process.poll.return_value = None
        process.wait.side_effect = [TimeoutError("alarm"), 0]
        with (
            patch.object(collector.subprocess, "Popen", return_value=process),
            patch.object(collector.os, "set_blocking"),
            patch.object(collector.os, "read", return_value=b""),
            patch.object(
                collector.select, "select", return_value=([process.stdout], [], [])
            ),
        ):
            with self.assertRaises(TimeoutError):
                collector._command("jobs", time.monotonic() + 5)
        process.kill.assert_called_once_with()
        self.assertEqual(process.wait.call_count, 2)
        self.assertEqual(process.wait.call_args.kwargs, {"timeout": 1})


if __name__ == "__main__":
    unittest.main()
