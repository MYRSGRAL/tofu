"""Ограничение offline capture на собственных Python children, без decoder/VM."""

from __future__ import annotations

import fcntl
import importlib
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
offline = importlib.import_module("a4_offline_bootstrap_extract")
capture = importlib.import_module("a4_offline_capture")


class OfflineCaptureBoundaryTests(unittest.TestCase):
    def child(
        self, source: str, pass_fds: tuple[int, ...] = ()
    ) -> subprocess.Popen[bytes]:
        return subprocess.Popen(
            [sys.executable, "-c", source],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            pass_fds=pass_fds,
        )

    def test_normal_capture_reaps_and_closes_pipes(self) -> None:
        process = self.child("import os; os.write(1, b'out'); os.write(2, b'err')")
        stdout, stderr, failure = capture.capture(process, 2)
        self.assertEqual((stdout, stderr, failure), ("out", "err", None))
        self.assertEqual(process.returncode, 0)
        self.assertTrue(process.stdout.closed)
        self.assertTrue(process.stderr.closed)
        with self.assertRaises(ChildProcessError):
            os.waitpid(process.pid, os.WNOHANG)

    def test_timeout_reaps_before_return(self) -> None:
        process = self.child("import time; time.sleep(60)")
        began = time.monotonic()
        _, _, failure = capture.capture(process, 0.05)
        self.assertEqual(failure, "timeout")
        self.assertEqual(process.returncode, -signal.SIGKILL)
        self.assertLess(time.monotonic() - began, 2)

    def test_combined_stderr_stdout_cap(self) -> None:
        process = self.child(
            "import os; os.write(2, b'e' * 900000); os.write(1, b'o' * 900000)"
        )
        stdout, stderr, failure = capture.capture(process, 2)
        self.assertEqual(failure, "overflow")
        self.assertLessEqual(len(stdout) + len(stderr), capture.OUTPUT_LIMIT)
        self.assertIsNotNone(process.returncode)

    def test_parent_held_writer_does_not_block_cleanup(self) -> None:
        process = self.child("import time; time.sleep(60)")
        writer = os.open(f"/proc/{process.pid}/fd/1", os.O_WRONLY)
        try:
            _, _, failure = capture.capture(process, 0.05)
            self.assertEqual(failure, "timeout")
            self.assertIsNotNone(process.returncode)
        finally:
            os.close(writer)

    def test_unreaped_child_retains_inherited_directory_lock(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            lock = os.open(tmp, os.O_RDONLY | os.O_DIRECTORY)
            fcntl.flock(lock, fcntl.LOCK_SH)
            process = self.child("import time; time.sleep(60)", (lock,))
            other = os.open(tmp, os.O_RDONLY | os.O_DIRECTORY)
            try:
                with (
                    patch.object(capture.os, "killpg"),
                    patch.object(
                        process, "wait", side_effect=subprocess.TimeoutExpired("own", 1)
                    ),
                ):
                    _, _, failure = capture.capture(process, 0.05)
                self.assertEqual(failure, "reap_timeout")
                os.close(lock)
                lock = -1
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                process.kill()
                process.wait(timeout=2)
                if lock >= 0:
                    os.close(lock)
            fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
            os.close(other)

    def test_existing_run_rejects_output_over_cap(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            reader = offline.Extraction.__new__(offline.Extraction)
            reader.started = time.monotonic()
            reader.commands = 0
            reader.out = Path(tmp)
            reader.nonce = "0" * 32
            reader.capture_locks = ()
            process = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import os; os.write(1, b'x' * (2 * 1024 * 1024))",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
                text=True,
            )
            try:
                with patch.object(offline.subprocess, "Popen", return_value=process):
                    with self.assertRaises((ValueError, TimeoutError)):
                        reader.run(["/usr/bin/btrfs", "restore"], timeout=3)
                self.assertIsNotNone(process.returncode)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=2)
                process.stdout.close()
                process.stderr.close()


if __name__ == "__main__":
    unittest.main()
