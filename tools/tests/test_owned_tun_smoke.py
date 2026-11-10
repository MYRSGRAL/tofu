"""Безопасный вход и критерий успешной persistent TUN пробы."""

from __future__ import annotations

import io
import sys
import unittest
from contextlib import nullcontext, redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from owned_tun_smoke import guard_namespace, main, supported_modes, worker  # noqa: E402


class BoundaryTests(unittest.TestCase):
    def test_worker_rejects_multi_queue_only_even_when_discovery_succeeds(self) -> None:
        with (
            patch("owned_tun_smoke.guard_namespace"),
            patch("owned_tun_smoke.subprocess.run"),
            patch("owned_tun_smoke.ip"),
            patch("owned_tun_smoke.echo_servers", return_value=nullcontext()),
            patch(
                "owned_tun_smoke.one_case",
                side_effect=lambda _binary, _engine, multi, crash: {
                    "multi_queue": multi,
                    "signal": "SIGKILL" if crash else "SIGTERM",
                    "result": "PASS" if multi else "INCOMPATIBLE",
                },
            ),
            redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(worker(Path(sys.executable), "xray", (9, 2)), 1)

    def test_launcher_always_creates_full_namespace_wrapper(self) -> None:
        with (
            patch.object(sys, "argv", ["smoke", "xray", sys.executable]),
            patch("owned_tun_smoke.namespace", return_value=(9, 12345)),
            patch("owned_tun_smoke.worker") as worker,
            patch("owned_tun_smoke.run_sandbox") as sandbox,
        ):
            self.assertEqual(main(), 0)
            worker.assert_not_called()
            command = sandbox.call_args.args[0]
            self.assertEqual(command[0], "/usr/bin/unshare")
            for flag in (
                "--user",
                "--map-root-user",
                "--net",
                "--mount",
                "--pid",
                "--fork",
                "--mount-proc",
            ):
                self.assertIn(flag, command)
            self.assertNotIn("--worker-namespace", command)

    def test_public_cli_rejects_forged_namespace_worker(self) -> None:
        with (
            patch.object(
                sys,
                "argv",
                ["smoke", "xray", sys.executable, "--worker-namespace", "0"],
            ),
            patch("owned_tun_smoke.worker") as worker,
            patch("owned_tun_smoke.run_sandbox") as sandbox,
            redirect_stderr(io.StringIO()),
        ):
            with self.assertRaises(SystemExit) as error:
                main()
            self.assertEqual(error.exception.code, 2)
            worker.assert_not_called()
            sandbox.assert_not_called()

    def test_host_guard_precedes_all_network_reads(self) -> None:
        for current in ((9, 2), (8, 3)):
            with (
                self.subTest(current=current),
                patch("owned_tun_smoke.namespace", return_value=current),
                patch("owned_tun_smoke.ip") as ip,
            ):
                with self.assertRaises(RuntimeError):
                    guard_namespace((9, 2))
                ip.assert_not_called()

    def test_guard_rejects_nonempty_namespace(self) -> None:
        with (
            patch("owned_tun_smoke.namespace", return_value=(9, 3)),
            patch(
                "owned_tun_smoke.ip",
                return_value=[{"ifname": "lo"}, {"ifname": "eth0"}],
            ),
        ):
            with self.assertRaises(RuntimeError):
                guard_namespace((9, 2))

    def test_mode_requires_both_clean_and_crash_success(self) -> None:
        cases = [{"multi_queue": False, "signal": "SIGTERM", "result": "PASS"}]
        self.assertEqual(supported_modes(cases), [])
        cases.append(
            {
                "multi_queue": False,
                "signal": "SIGKILL",
                "result": "ENGINE_REMOVED_HELPER_TUN",
            }
        )
        self.assertEqual(supported_modes(cases), [])
        cases[-1]["result"] = "PASS"
        self.assertEqual(supported_modes(cases), ["single_queue"])
        cases.extend(
            {"multi_queue": True, "signal": signal_name, "result": "INCOMPATIBLE"}
            for signal_name in ("SIGTERM", "SIGKILL")
        )
        self.assertEqual(supported_modes(cases), ["single_queue"])


if __name__ == "__main__":
    unittest.main()
