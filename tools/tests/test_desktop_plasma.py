"""Negative controls FR19: потеря регистрации и реальный terminal pidfd."""

import os
import subprocess
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from desktop_plasma_smoke import assert_continuity


class ContinuityTests(unittest.TestCase):
    def test_lost_registration(self) -> None:
        descriptor = os.pidfd_open(os.getpid())
        try:
            with self.assertRaisesRegex(RuntimeError, "SNI"):
                assert_continuity([":1.1/StatusNotifierItem"], [], descriptor)
        finally:
            os.close(descriptor)

    def test_terminal_session(self) -> None:
        process = subprocess.Popen([sys.executable, "-c", "pass"])
        descriptor = os.pidfd_open(process.pid)
        try:
            process.wait(timeout=3)
            with self.assertRaisesRegex(RuntimeError, "session"):
                assert_continuity(["item"], ["item"], descriptor)
        finally:
            os.close(descriptor)
            process.wait()

    def test_live_session(self) -> None:
        descriptor = os.pidfd_open(os.getpid())
        try:
            assert_continuity(["item"], ["item"], descriptor)
        finally:
            os.close(descriptor)
