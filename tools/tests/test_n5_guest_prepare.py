"""Только чистые doubles: никакого runtime I/O."""

import copy
import json
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import n5_guest_prepare as n


class FakeHost:
    def __init__(self, backend, reopened=False):
        self.backend = backend
        self.state = (
            copy.deepcopy(backend.saved)
            if reopened
            else {
                "ready": True,
                "profiles": [],
                "sources": [],
                "revision": 7,
                "routing_revision": 3,
                "routing": {"rules": []},
            }
        )

    def until(self, predicate):
        transient = dict(
            self.state,
            maintenance_ack=None,
            routing_ack=None,
            ready=False,
            pending=None,
            profiles=[],
            selected_profile=None,
        )
        predicate(transient)
        n.require(predicate(self.state))
        return copy.deepcopy(self.state)

    def command(self, command):
        self.backend.commands.append(command)
        state = self.state
        match command["command"]:
            case "import_file":
                state["pending"] = {"accepted": 1}
            case "maintenance_start":
                n.require(command["revision"] == 7)
                state["maintenance_ack"] = {
                    "request_id": 90,
                    "phase": self.backend.backup,
                }
            case "approve_refresh":
                state.update(
                    pending=None,
                    busy=False,
                    profiles=[{"id": 42, "name": "declared"}],
                    sources=[{"id": 17}],
                    source_id=17,
                )
            case "select_profile":
                state["selected_profile"] = command["id"]
            case "edit_routing":
                n.require(command["expected_revision"] == state["routing_revision"])
                state["routing_revision"] += 0 if self.backend.stale else 1
                state["routing"]["rules"].append(command["rule"])
                state["routing_ack"] = {
                    "request_id": command["request_id"],
                    "error": None,
                }
            case _:
                raise AssertionError("Запрещённая команда")


class FakeBackend:
    backup = "complete"
    stale = False
    corrupt = False
    cleanup_failure = False

    def __init__(self, data):
        self.commands = []
        self.saved = None
        self.closed = False
        self.finishes = 0

    def start(self):
        pass

    def host(self):
        host = FakeHost(self, self.saved is not None)
        if self.corrupt and self.saved is not None:
            host.state["selected_profile"] = 999
        return host

    def finish(self, host):
        self.saved = copy.deepcopy(host.state)
        self.finishes += 1

    def close(self):
        self.closed = True
        if self.cleanup_failure:
            raise n.Closed("UNKNOWN: cleanup")


class PreparationTests(unittest.TestCase):
    def setUp(self):
        paths = (
            n.HOST,
            n.DRIVER,
            n.PYTHON,
            n.BUS,
            n.SS,
            n.BUSCTL,
            str(n.CONFIG),
            str(n.ARCHIVE),
            str(n.ROOT / "n5_guest_prepare.py"),
        )
        self.data = {
            "schema": "N5_PREPARATION_V1",
            "caller_uid": 1000,
            "tool_source_commit": "a" * 40,
            "product_source_commit": "b" * 40,
            "pins": dict.fromkeys(paths, n.digest(b"pinned")),
            "profile_name": "declared",
            "rules": [
                {
                    "id": 1,
                    "enabled": True,
                    "executables": [n.DRIVER],
                    "domains": [],
                    "destinations": ["10.0.0.4/32"],
                    "transport": {
                        "transport_protocols": ["tcp"],
                        "destination_ports": [[8000, 8000]],
                    },
                    "action": "direct",
                }
            ],
            "max_seconds": 120,
        }
        self.grant = True
        self.backend = None

    def read(self, path, maximum=n.LIMIT):
        raw = json.dumps(self.data).encode()
        if path == n.INPUT:
            return raw
        if path == n.GRANT:
            return json.dumps(
                {
                    "schema": "N5_PREPARATION_GRANT_V1",
                    "permission": "USER_GRANTED_GUEST_PREPARATION"
                    if self.grant
                    else "CLOSED",
                    "input_sha256": n.digest(raw),
                    "caller_uid": 1000,
                }
            ).encode()
        return b"pinned"

    def factory(self, data):
        self.backend = FakeBackend(data)
        return self.backend

    def prepare(self):
        return n.prepare(self.read, lambda: 1000, self.factory)

    def test_full_flow_and_lease(self):
        with self.prepare() as lease:
            self.assertFalse(self.backend.closed)
            self.assertEqual(lease.receipt["profile_id"], 42)
            self.assertEqual(lease.receipt["source_id"], 17)
            self.assertEqual(lease.receipt["runtime_grant"], "CLOSED")
            self.assertEqual(self.backend.finishes, 2)
            self.assertEqual(
                [c["command"] for c in self.backend.commands],
                [
                    "import_file",
                    "maintenance_start",
                    "approve_refresh",
                    "select_profile",
                    "edit_routing",
                ],
            )
        self.assertTrue(self.backend.closed)

    def test_grant_closed_before_backend(self):
        self.grant = False
        with self.assertRaises(n.Closed):
            self.prepare()
        self.assertIsNone(self.backend)

    def test_pin_mismatch_before_backend(self):
        self.data["pins"][n.HOST] = "0" * 64
        with self.assertRaises(n.Closed):
            self.prepare()
        self.assertIsNone(self.backend)

    def test_uid_before_backend(self):
        with self.assertRaises(n.Closed):
            n.prepare(self.read, lambda: 0, self.factory)
        self.assertIsNone(self.backend)

    def test_stale_backup_reopen_cleanup(self):
        for name, value in [("stale", True), ("backup", "failed"), ("corrupt", True)]:
            with self.subTest(name=name), patch.object(FakeBackend, name, value):
                with self.assertRaises(n.Closed):
                    self.prepare()
                self.assertTrue(self.backend.closed)

    def test_cleanup_unknown(self):
        lease = self.prepare()
        with (
            patch.object(FakeBackend, "cleanup_failure", True),
            self.assertRaisesRegex(n.Closed, "UNKNOWN"),
        ):
            lease.close()
        self.assertFalse(lease.closed)

    def test_strict_json(self):
        for raw in (b'{"a":1,"a":2}', b"NaN", b"x" * (n.LIMIT + 1)):
            with self.assertRaises(n.Closed):
                n.strict_json(raw)

    def test_unknown_fields_and_bounds(self):
        for key, value in [
            ("unknown", 1),
            ("max_seconds", 3601),
            ("rules", []),
            ("profile_name", ""),
        ]:
            with self.subTest(key=key):
                old = copy.deepcopy(self.data)
                self.data[key] = value
                with self.assertRaises(n.Closed):
                    self.prepare()
                self.assertIsNone(self.backend)
                self.data = old

    def test_rule_ids_and_transport(self):
        for bad in (True, False, "1", 0, -1, 9007199254740992):
            with self.subTest(bad=bad):
                self.data["rules"][0]["id"] = bad
                with self.assertRaises(n.Closed):
                    self.prepare()
                self.assertIsNone(self.backend)
        for value in (1, 9007199254740991):
            self.data["rules"][0]["id"] = value
            self.prepare().close()

    def test_invalid_destination_transport_before_backend(self):
        original = copy.deepcopy(self.data["rules"][0])
        for key, value in (
            ("destinations", ["10.0.0.1/24"]),
            ("transport", None),
            (
                "transport",
                {"transport_protocols": ["tcp"], "destination_ports": [[True, 3]]},
            ),
            (
                "transport",
                {"transport_protocols": ["udp"], "destination_ports": [[2, 4], [4, 5]]},
            ),
        ):
            self.data["rules"][0] = dict(original, **{key: value})
            with self.subTest(key=key, value=value), self.assertRaises(n.Closed):
                self.prepare()
            self.assertIsNone(self.backend)

    def test_matrix_preserves_typed_conditions(self):
        self.data["rules"] = [
            dict(
                self.data["rules"][0],
                id=index + 1,
                executables=[executable],
                action=action,
                transport={
                    "transport_protocols": [protocol],
                    "destination_ports": [[8000 + index, 8000 + index]],
                },
            )
            for index, (executable, protocol, action) in enumerate(
                (e, p, a)
                for e in (n.DRIVER, n.PYTHON)
                for p in ("tcp", "udp")
                for a in ("direct", "proxy", "block")
            )
        ]
        with self.prepare():
            actual = [
                c["rule"]
                for c in self.backend.commands
                if c["command"] == "edit_routing"
            ]
            self.assertEqual(actual, self.data["rules"])
        self.assertEqual(n.PYTHON, "/usr/lib/yonik-acceptance/n5-python")


class CleanupTests(unittest.TestCase):
    def child(self):
        child = n.Child.__new__(n.Child)
        child.pidfd = 77
        child.process = MagicMock()
        child.process.wait.return_value = 0
        return child

    def test_each_cleanup_trigger_is_sticky_and_reaped(self):
        for trigger in ("stdin", "signal", "wait", "stdout", "fd"):
            with self.subTest(trigger=trigger):
                child = self.child()
                if trigger == "stdin":
                    child.process.stdin.close.side_effect = OSError()
                if trigger == "stdout":
                    child.process.stdout.close.side_effect = OSError()
                if trigger == "wait":
                    child.process.wait.side_effect = [OSError(), 0]
                with (
                    patch.object(
                        n.signal,
                        "pidfd_send_signal",
                        side_effect=OSError() if trigger == "signal" else None,
                    ),
                    patch.object(
                        n.os,
                        "close",
                        side_effect=OSError() if trigger == "fd" else None,
                    ),
                ):
                    with self.assertRaisesRegex(n.Closed, "UNKNOWN"):
                        child.close()
                    self.assertTrue(child.reaped)
                    with self.assertRaisesRegex(n.Closed, "UNKNOWN"):
                        child.close()

    def test_failed_reap_retains_pidfd_and_backend_ownership(self):
        child = self.child()
        child.process.wait.side_effect = OSError()
        backend = n.Backend({"caller_uid": 1000, "max_seconds": 10})
        backend.children = [child]
        lease = n.PreparedLease(backend, {})
        with (
            patch.object(n.signal, "pidfd_send_signal") as send,
            patch.object(n.os, "close") as close,
        ):
            with self.assertRaises(n.Closed):
                backend.finish(child)
            self.assertEqual(backend.children, [child])
            self.assertEqual(child.pidfd, 77)
            close.assert_not_called()
            self.assertEqual(send.call_count, 2)
            with self.assertRaises(n.Closed):
                lease.close()
            self.assertFalse(lease.closed)
            child.process.wait.side_effect = None
            child.process.wait.return_value = 0
            with self.assertRaises(n.Closed):
                lease.close()
            self.assertFalse(lease.closed)
            self.assertEqual(backend.children, [child])
            self.assertIsNone(child.pidfd)

    def test_root_parent_and_bind_gate_before_writes(self):
        backend = n.Backend({"caller_uid": 1000, "max_seconds": 10})
        with (
            patch.object(
                Path,
                "lstat",
                return_value=SimpleNamespace(st_mode=0o40700, st_uid=1000, st_gid=1000),
            ),
            patch.object(backend, "spawn") as spawn,
        ):
            with self.assertRaises(n.Closed):
                backend.start()
            spawn.assert_not_called()
        root = SimpleNamespace(st_mode=0o40755, st_uid=0, st_gid=0)
        caller = SimpleNamespace(st_mode=0o40700, st_uid=1000)
        with (
            patch.object(Path, "lstat", return_value=root),
            patch.object(n.os, "open", return_value=88) as opened,
            patch.object(n.os, "close"),
            patch.object(n.os, "fstat", return_value=caller),
            patch.object(
                Path,
                "stat",
                side_effect=[
                    SimpleNamespace(st_dev=1, st_ino=2),
                    SimpleNamespace(st_dev=1, st_ino=3),
                ],
            ),
            patch.object(backend, "spawn") as spawn,
        ):
            with self.assertRaises(n.Closed):
                backend.start()
            spawn.assert_not_called()
            self.assertFalse(
                any(call.args[1] & n.os.O_CREAT for call in opened.call_args_list)
            )

    def test_finish_failed_then_lease_close_never_success(self):
        child = self.child()
        child.process.stdin.close.side_effect = OSError()
        backend = n.Backend({"caller_uid": 1000, "max_seconds": 10})
        backend.children = [child]
        with patch.object(n.signal, "pidfd_send_signal"), patch.object(n.os, "close"):
            with self.assertRaises(n.Closed):
                backend.finish(child)
            self.assertEqual(backend.children, [child])
            child.process.stdin.close.side_effect = None
            for _ in range(2):
                with self.assertRaises(n.Closed) as raised:
                    backend.close()
                self.assertIs(raised.exception.owner, backend)
                self.assertEqual(backend.children, [child])

    def test_success_reap_is_idempotent(self):
        child = self.child()
        with (
            patch.object(n.signal, "pidfd_send_signal"),
            patch.object(n.os, "close") as close,
        ):
            child.close()
            child.close()
            close.assert_called_once_with(77)
            child.process.wait.assert_called_once()


if __name__ == "__main__":
    unittest.main()
