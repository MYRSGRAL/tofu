"""Обязательная изоляция для настоящих privileged network-тестов."""

from __future__ import annotations

import io
import sys
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from managed_network_smoke import CASES, command, main  # noqa: E402


class BoundaryTests(unittest.TestCase):
    def test_every_case_has_exact_test_inside_full_isolation(self) -> None:
        for case, test in CASES.items():
            with self.subTest(case=case):
                args = command(Path("/test-binary"), case)
                self.assertEqual(args[0], "/usr/bin/unshare")
                for required in (
                    "--user",
                    "--map-root-user",
                    "--net",
                    "--mount",
                    "--pid",
                    "--fork",
                    "--mount-proc",
                ):
                    self.assertIn(required, args)
                self.assertEqual(
                    args[-5:],
                    ["/test-binary", "--ignored", "--exact", test, "--nocapture"],
                )

    def test_cli_cannot_select_arbitrary_test_or_direct_worker(self) -> None:
        for extra in (
            ["--case", "arbitrary"],
            ["--case", "owned-tun", "--worker-namespace", "0"],
        ):
            with (
                self.subTest(extra=extra),
                patch.object(sys, "argv", ["smoke", sys.executable, *extra]),
                patch("managed_network_smoke.run_sandbox") as run,
                redirect_stderr(io.StringIO()),
            ):
                with self.assertRaises(SystemExit) as error:
                    main()
                self.assertEqual(error.exception.code, 2)
                run.assert_not_called()

    def test_valid_cli_calls_only_isolated_runner(self) -> None:
        with (
            patch.object(sys, "argv", ["smoke", sys.executable, "--case", "owned-tun"]),
            patch("managed_network_smoke.run_sandbox") as run,
        ):
            self.assertEqual(main(), 0)
            self.assertEqual(run.call_args.args[0][0], "/usr/bin/unshare")


if __name__ == "__main__":
    unittest.main()
