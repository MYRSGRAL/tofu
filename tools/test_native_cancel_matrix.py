"""Проверки отрицательного oracle без native cores."""

import subprocess
import sys
import time
import unittest
from pathlib import Path

from native_cancel_matrix import remaining, snapshot


class OracleTest(unittest.TestCase):
    def test_unreaped_zombie_remains_until_wait(self) -> None:
        child = subprocess.Popen([sys.executable, "-c", "pass"])
        try:
            deadline = time.monotonic() + 5
            while True:
                stat = (
                    Path(f"/proc/{child.pid}/stat")
                    .read_text()
                    .rsplit(") ", 1)[1]
                    .split()
                )
                if stat[0] == "Z":
                    break
                if time.monotonic() >= deadline:
                    self.fail("Собственный child не завершился до deadline")
                time.sleep(0.01)
            identity = f"{child.pid}:{stat[19]}"
            self.assertIn(identity, snapshot(child.pid))
        finally:
            child.kill()
            child.wait()
        self.assertEqual(snapshot(child.pid), {})

    def test_starttime_reuse_is_new_identity(self) -> None:
        self.assertEqual(
            remaining({"10:200": {"fd": 1}}, {"10:100": {}}), {"10:200": {"fd": 1}}
        )

    def test_real_owned_fixture_retained_then_reaped(self) -> None:
        baseline = {}
        child = subprocess.Popen(
            [
                sys.executable,
                "-c",
                "import time; a=bytearray(8*1024*1024); time.sleep(10)",
            ]
        )
        try:
            time.sleep(0.1)
            retained = remaining(snapshot(child.pid), baseline)
            self.assertTrue(any(key.startswith(f"{child.pid}:") for key in retained))
            self.assertTrue(
                any(
                    row["private_dirty"] >= 8 * 1024 * 1024 for row in retained.values()
                )
            )
        finally:
            child.terminate()
            child.wait()
        self.assertFalse(
            any(
                key.startswith(f"{child.pid}:")
                for key in remaining(snapshot(child.pid), baseline)
            )
        )


if __name__ == "__main__":
    unittest.main()
