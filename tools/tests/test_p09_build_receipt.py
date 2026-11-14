"""Pure negative controls backend gate, не authority для запуска candidate."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from p09_build_receipt import TARGET, check_raw


class BackendBoundary(unittest.TestCase):
    def test_unsupported_cfg_and_features_are_rejected(self) -> None:
        flags = ["--target", TARGET, "--cfg", "linux_raw", "--cfg", "linux_raw_dep"]
        for feature in ("fs", "std", "process"):
            flags += ["--cfg", f'feature="{feature}"']
        for cfg in (
            "libc",
            "rustix_use_libc",
            "rustix_no_linux_raw",
            "miri",
            'feature="use-libc"',
            'feature="rustc-dep-of-std"',
        ):
            with self.subTest(cfg=cfg), self.assertRaises(AssertionError):
                check_raw(flags + ["--cfg", cfg])
        with self.assertRaises(AssertionError):
            check_raw([v.replace(TARGET, "x86_64-unknown-linux-gnux32") for v in flags])
        with self.assertRaises(AssertionError):
            check_raw([v for v in flags if v != "linux_raw"])


if __name__ == "__main__":
    unittest.main()
