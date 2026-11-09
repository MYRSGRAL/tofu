"""Границы JSON и guard синтетического topology fixture."""

from __future__ import annotations

import io
import sys
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from network_topology_smoke import (  # noqa: E402
    MAX_OUTPUT,
    bounded_run,
    decode_output,
    guard_namespace,
    main,
    table_id,
)


class BoundaryTests(unittest.TestCase):
    def test_public_entrypoint_always_launches_full_namespace_wrapper(self) -> None:
        with (
            patch.object(sys, "argv", ["smoke", sys.executable]),
            patch("network_topology_smoke.namespace", return_value=(9, 12345)),
            patch("network_topology_smoke.worker") as worker,
            patch("network_topology_smoke.run_sandbox") as sandbox,
        ):
            self.assertEqual(main(), 0)
            worker.assert_not_called()
            sandbox.assert_called_once()
            command = sandbox.call_args.args[0]
            self.assertEqual(command[0], "unshare")
            for required in (
                "--user",
                "--map-root-user",
                "--net",
                "--pid",
                "--mount",
                "--mount-proc",
            ):
                self.assertIn(required, command)
            self.assertNotIn("--worker-namespace", command)

    def test_public_entrypoint_rejects_forged_worker_namespace(self) -> None:
        with (
            patch.object(
                sys, "argv", ["smoke", sys.executable, "--worker-namespace", "0", "0"]
            ),
            patch("network_topology_smoke.worker") as worker,
            patch("network_topology_smoke.run_sandbox") as sandbox,
            redirect_stderr(io.StringIO()),
        ):
            with self.assertRaises(SystemExit) as error:
                main()
            self.assertEqual(error.exception.code, 2)
            worker.assert_not_called()
            sandbox.assert_not_called()

    def test_one_bounded_object(self) -> None:
        self.assertEqual(decode_output(b'{"ok":true}\n'), {"ok": True})
        for data in (b"[]", b"{} {}", b"x" * (MAX_OUTPUT + 1), b'{"ok":NaN}'):
            with self.subTest(length=len(data)), self.assertRaises(ValueError):
                decode_output(data)

    def test_oracle_numeric_and_named_tables(self) -> None:
        for value, expected in (
            ("100", 100),
            (200, 200),
            ("main", 254),
            ("local", 255),
        ):
            with self.subTest(value=value):
                self.assertEqual(table_id(value), expected)

    def test_process_output_limit_and_wait_deadline(self) -> None:
        with patch("network_topology_smoke.MAX_OUTPUT", 128):
            with self.assertRaises(ValueError):
                bounded_run(
                    [sys.executable, "-c", "import sys; sys.stdout.write('x'*4096)"]
                )
        with self.assertRaises(TimeoutError):
            bounded_run(
                [sys.executable, "-c", "import time; time.sleep(30)"],
                b"x" * 16385,
                timeout=0.1,
            )
        self.assertEqual(
            bounded_run(
                [
                    sys.executable,
                    "-c",
                    "import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())",
                ],
                b"{}",
            ),
            (0, b"{}"),
        )

    def test_guard_rejects_host_before_reading_links(self) -> None:
        with (
            patch("network_topology_smoke.namespace", return_value=(9, 2)),
            patch("network_topology_smoke.ip_json") as oracle,
        ):
            with self.assertRaises(RuntimeError):
                guard_namespace((1, 2), {"lo"})
            oracle.assert_not_called()

    def test_guard_rejects_foreign_links_and_namespace_drift(self) -> None:
        with (
            patch("network_topology_smoke.namespace", return_value=(1, 3)),
            patch("network_topology_smoke.ip_json", return_value=[{"ifname": "eth0"}]),
        ):
            with self.assertRaises(RuntimeError):
                guard_namespace((1, 2), {"lo"})
            with self.assertRaises(RuntimeError):
                guard_namespace((1, 2), {"eth0"}, expected=(1, 4))

    def test_guard_allows_exact_owned_set(self) -> None:
        with (
            patch("network_topology_smoke.namespace", return_value=(1, 3)),
            patch("network_topology_smoke.ip_json", return_value=[{"ifname": "lo"}]),
        ):
            self.assertEqual(guard_namespace((1, 2), {"lo"}), (1, 3))


if __name__ == "__main__":
    unittest.main()
