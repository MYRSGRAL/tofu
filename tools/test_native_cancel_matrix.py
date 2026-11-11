"""Проверки отрицательного oracle без native cores."""

import subprocess
import sys
import time
import unittest

from native_cancel_matrix import remaining, snapshot


class OracleTest(unittest.TestCase):
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
