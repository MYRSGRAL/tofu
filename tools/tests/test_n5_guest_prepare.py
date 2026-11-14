"""Только чистые doubles: никакого runtime I/O."""

import copy
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

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
                    "id": "owned-rule",
                    "enabled": True,
                    "executables": [n.DRIVER],
                    "domains": [],
                    "destinations": [],
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


if __name__ == "__main__":
    unittest.main()
