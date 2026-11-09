"""Изоляция, лимит trace и сохранение ядра сетевого GUI smoke."""

from __future__ import annotations

import sys
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import desktop_smoke as smoke  # noqa: E402


class DesktopNetworkTests(unittest.TestCase):
    def test_refresh_rejects_completed_handler_without_transport_outcome(self) -> None:
        for terminal in (None, "unknown"):
            with self.subTest(terminal=terminal):
                gate = smoke.ProbeGate()
                gate.received.set()
                gate.completed.set()
                gate.terminal = terminal
                handler = type(
                    "FailedBarrierFixture", (smoke.Fixture,), {"refresh_gate": gate}
                )
                server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
                serving = threading.Thread(target=server.serve_forever)
                serving.start()
                try:
                    with self.assertRaises(urllib.error.HTTPError) as failure:
                        urllib.request.urlopen(
                            f"http://127.0.0.1:{server.server_port}/", timeout=2
                        )
                    self.assertEqual(failure.exception.code, 503)
                    self.assertFalse(gate.refresh_passed.is_set())
                finally:
                    server.shutdown()
                    server.server_close()
                    serving.join(timeout=2)
                self.assertFalse(serving.is_alive())

    def test_subscription_refresh_waits_for_terminal_late_https_attempt(self) -> None:
        gate = smoke.ProbeGate()
        gate.received.set()
        handler = type(
            "BarrierFixture",
            (smoke.Fixture,),
            {"refresh_gate": gate, "body": b"profile"},
        )
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        serving = threading.Thread(target=server.serve_forever)
        serving.start()
        received = threading.Event()
        failures: list[Exception] = []

        def request() -> None:
            try:
                with urllib.request.urlopen(
                    f"http://127.0.0.1:{server.server_port}/", timeout=3
                ) as response:
                    self.assertEqual(response.read(), b"profile")
                received.set()
            except Exception as error:
                failures.append(error)

        client = threading.Thread(target=request)
        client.start()
        try:
            self.assertTrue(gate.refresh_requested.wait(timeout=1))
            self.assertFalse(
                received.wait(timeout=0.1),
                "Refresh не должен завершаться до позднего HTTPS",
            )
            gate.terminal = "sent"
            gate.completed.set()
            self.assertTrue(received.wait(timeout=2))
            self.assertTrue(gate.refresh_passed.is_set())
            self.assertFalse(failures)
        finally:
            gate.completed.set()
            client.join(timeout=4)
            server.shutdown()
            server.server_close()
            serving.join(timeout=2)
        self.assertFalse(client.is_alive())
        self.assertFalse(serving.is_alive())

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
        handler = type("HeldFixture", (smoke.HeldHttpsHandler,), {"gate": gate})
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        server.requested = threading.Event()
        serving = threading.Thread(target=server.serve_forever)
        serving.start()
        responded = threading.Event()
        failures: list[Exception] = []

        def request() -> None:
            try:
                with urllib.request.urlopen(
                    f"http://127.0.0.1:{server.server_port}/", timeout=3
                ) as response:
                    self.assertEqual(response.status, 204)
                    self.assertEqual(response.read(), b"")
                responded.set()
            except Exception as error:
                failures.append(error)

        client = threading.Thread(target=request)
        client.start()
        try:
            self.assertTrue(gate.received.wait(timeout=1))
            self.assertFalse(responded.wait(timeout=0.1))
            gate.release.set()
            self.assertTrue(gate.completed.wait(timeout=2))
            self.assertTrue(responded.wait(timeout=2))
            self.assertEqual(gate.terminal, "sent")
            self.assertFalse(gate.timed_out.is_set())
            self.assertFalse(failures)
        finally:
            gate.release.set()
            client.join(timeout=4)
            server.shutdown()
            server.server_close()
            serving.join(timeout=2)
        self.assertFalse(client.is_alive())
        self.assertFalse(serving.is_alive())

    def test_checking_trigger_releases_response_only_after_invalidation(self) -> None:
        gate = smoke.ProbeGate()
        gate.received.set()
        gate.completed.set()
        gate.terminal = "sent"
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
