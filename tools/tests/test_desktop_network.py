"""Изоляция, лимит trace и сохранение ядра сетевого GUI smoke."""

from __future__ import annotations

import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import desktop_smoke as smoke  # noqa: E402


class DesktopNetworkTests(unittest.TestCase):
    def test_guard_rejects_host_and_nonempty_namespace_before_mutation(self) -> None:
        for expected, actual, interfaces in (
            (None, 1, {"lo"}),
            (1, 1, {"lo"}),
            (1, 2, {"lo", "eth0"}),
        ):
            with (
                self.subTest(expected=expected, actual=actual, interfaces=interfaces),
                patch.object(smoke, "NETWORK_HOST_NAMESPACE", expected),
                patch.object(
                    smoke.os, "stat", return_value=SimpleNamespace(st_ino=actual)
                ),
                patch.object(smoke, "links", return_value=interfaces),
                patch.object(smoke.subprocess, "run") as command,
                self.assertRaises(RuntimeError),
            ):
                smoke.network_worker(
                    Path("unused"), Path("unused"), Path("unused"), expected
                )
            command.assert_not_called()

    def test_trace_is_bounded_and_ignores_incomplete_last_record(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace"
            path.write_bytes(b'{"closed":false}\n{"closed":')
            self.assertEqual(smoke.read_trace(path), [{"closed": False}])
            path.write_bytes(b"x" * (smoke.TRACE_LIMIT + 1))
            with self.assertRaises(RuntimeError):
                smoke.read_trace(path)

    def test_route_event_keeps_same_process_and_cleans_route(self) -> None:
        connected = [
            {
                "status": "Connected",
                "probe_present": True,
                "tcp_status": "reachable",
                "tcp_busy": False,
            }
        ]
        invalidated = [{"status": "Limited", "error": "network_changed"}]
        for after, success in (
            ({42: "start"}, True),
            ({}, False),
            ({42: "new"}, False),
        ):
            proof: list[bool] = []
            failures: list[Exception] = []
            with (
                self.subTest(after=after),
                patch.object(smoke, "guard_network"),
                patch.object(smoke, "read_trace", side_effect=[connected, invalidated]),
                patch.object(
                    smoke, "core_processes", side_effect=[{42: "start"}, after]
                ),
                patch.object(smoke.subprocess, "run") as command,
            ):
                smoke.invalidate_network(
                    Path("trace"),
                    Path("engines"),
                    {},
                    threading.Event(),
                    proof,
                    failures,
                )
            self.assertEqual(bool(proof), success)
            self.assertEqual(bool(failures), not success)
            self.assertEqual(
                [call.args[0][2] for call in command.call_args_list], ["add", "del"]
            )

    def test_https_handler_holds_real_response_until_release(self) -> None:
        gate = smoke.ProbeGate()
        handler = smoke.HeldHttpsHandler.__new__(smoke.HeldHttpsHandler)
        handler.gate = gate
        responded = threading.Event()
        with patch.object(smoke.HttpsHandler, "do_GET", side_effect=responded.set):
            thread = threading.Thread(target=handler.do_GET)
            thread.start()
            try:
                self.assertTrue(gate.received.wait(timeout=1))
                self.assertFalse(responded.is_set())
                gate.release.set()
                self.assertTrue(gate.completed.wait(timeout=1))
                self.assertTrue(responded.is_set())
                self.assertFalse(gate.timed_out.is_set())
            finally:
                gate.release.set()
                thread.join(timeout=2)
        self.assertFalse(thread.is_alive())

    def test_checking_trigger_releases_response_only_after_invalidation(self) -> None:
        gate = smoke.ProbeGate()
        gate.received.set()
        gate.completed.set()
        proof: list[bool] = []
        failures: list[Exception] = []
        events = [
            [{"status": "Checking", "probe_present": False}],
            [{"status": "Limited", "error": "network_changed"}],
        ]

        def route(*args: object, **kwargs: object) -> None:
            if args[0][2] == "add":
                self.assertFalse(gate.release.is_set())

        with (
            patch.object(smoke, "guard_network"),
            patch.object(smoke, "read_trace", side_effect=events),
            patch.object(smoke, "core_processes", return_value={42: "start"}),
            patch.object(smoke.subprocess, "run", side_effect=route),
        ):
            smoke.invalidate_network(
                Path("trace"),
                Path("engines"),
                {},
                threading.Event(),
                proof,
                failures,
                gate,
            )
        self.assertEqual(proof, [True])
        self.assertEqual(failures, [])
        self.assertTrue(gate.release.is_set())


if __name__ == "__main__":
    unittest.main()
