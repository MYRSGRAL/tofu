"""Только синтетические fixture/mocks, никаких обращений к хосту."""

import copy
import hashlib
import json
import unittest
from unittest.mock import Mock, patch

import collect_observations as c

NONCE = "a" * 64
CASE = "case-1"


class Fixture(c.SystemBackend):
    def __init__(self) -> None:
        self.calls = []
        inputs = {
            "schema": "ROOT_CAPTURE_INPUT_V1",
            "source": "b" * 40,
            "manifest_sha256": "c" * 64,
        }
        raw = json.dumps(inputs).encode()
        self.files = {c.ROOT + "/input.json": raw}
        auth = {
            "schema": "ROOT_CAPTURE_AUTH_V1",
            "execution": "USER_GRANTED_GUEST_CAPTURE",
            "nonce": NONCE,
            "case": CASE,
            "stages": list(c.STAGES),
            "input_sha256": hashlib.sha256(raw).hexdigest(),
        }
        self.files[c.ROOT + "/authorization.json"] = json.dumps(auth).encode()
        self.files["/proc/sys/kernel/random/boot_id"] = (
            b"12345678-1234-1234-1234-123456789abc"
        )
        self.files["/usr/lib/yonik-acceptance/collect_observations.py"] = b"mock source"
        for path in c.ARTIFACTS:
            self.files[path] = b"\x7fELFmock"

    def uid(self) -> int:
        return 0

    def pid(self) -> int:
        return 7

    def read(self, path: str, *, sealed: bool = False, cap: int = c.LIMIT) -> bytes:
        self.calls.append(path)
        if path in self.files:
            return self.files[path]
        if path.endswith("/stat"):
            return b"7 (mock) S " + b"0 " * 18 + b"123 0"
        if path.endswith("/status"):
            return b"Uid:\t0 0 0 0\n"
        if path.endswith("cgroup.events"):
            return b"populated 0\n"
        if path.endswith("cgroup.procs"):
            return b""
        raise FileNotFoundError(path)

    def command(self, name: str) -> bytes:
        self.calls.append(name)
        if name in c.UNITS:
            return f"MainPID=0\nInvocationID=\nControlGroup=/system.slice/{name}\nActiveState=inactive\n".encode()
        return b'{"nftables": []}' if name == "nft" else b"[]"


class CaptureTests(unittest.TestCase):
    def test_valid_grant_reaches_measurements(self) -> None:
        fixture = Fixture()
        data = c.collect("baseline", NONCE, CASE, backend=fixture)
        self.assertEqual(data["status"], "ACTUAL")
        self.assertIn("links", fixture.calls)
        self.assertEqual(data["observations"]["collector"]["start_ticks"], 123)

    def test_gate(self) -> None:
        for mutation in ("root", "nonce", "stage", "closed", "malformed", "missing"):
            with self.subTest(mutation=mutation):
                fixture = Fixture()
                stage, nonce = "baseline", NONCE
                if mutation == "root":
                    fixture.uid = lambda: 1000
                elif mutation == "nonce":
                    nonce = "d" * 64
                elif mutation == "stage":
                    stage = "arbitrary"
                elif mutation == "missing":
                    fixture.files.clear()
                else:
                    fixture.files[c.ROOT + "/authorization.json"] = (
                        b"{}"
                        if mutation == "malformed"
                        else fixture.files[c.ROOT + "/authorization.json"].replace(
                            b"USER_GRANTED_GUEST_CAPTURE", b"CLOSED"
                        )
                    )
                self.assertEqual(
                    c.collect(stage, nonce, CASE, backend=fixture)["status"],
                    "UNAVAILABLE",
                )
                self.assertFalse(
                    any(
                        path.startswith("/proc") or path == "links"
                        for path in fixture.calls
                    )
                )

    def test_missing_capture_and_denied_measurement(self) -> None:
        fixture = Fixture()
        self.assertEqual(
            c.read_capture("connected", NONCE, CASE, backend=fixture)["status"],
            "UNAVAILABLE",
        )
        fixture.command = Mock(side_effect=PermissionError)
        self.assertEqual(
            c.collect("baseline", NONCE, CASE, backend=fixture)["status"], "UNAVAILABLE"
        )

    def test_capture_schema_and_provenance(self) -> None:
        fixture = Fixture()
        capture = c.collect("baseline", NONCE, CASE, backend=fixture)
        path = c.ROOT + "/capture-baseline.json"
        fixture.files[path] = json.dumps(capture).encode()
        self.assertEqual(
            c.read_capture("baseline", NONCE, CASE, backend=fixture)["status"], "ACTUAL"
        )
        for field in ("nonce", "stage", "case", "source", "collector_pid"):
            broken = copy.deepcopy(capture)
            broken["provenance"][field] = False
            fixture.files[path] = json.dumps(broken).encode()
            self.assertEqual(
                c.read_capture("baseline", NONCE, CASE, backend=fixture)["status"],
                "UNAVAILABLE",
            )
        broken = copy.deepcopy(capture)
        broken["observations"]["units"] = []
        fixture.files[path] = json.dumps(broken).encode()
        self.assertEqual(
            c.read_capture("baseline", NONCE, CASE, backend=fixture)["status"],
            "UNAVAILABLE",
        )

    def test_secure_file_checks_are_mocked(self) -> None:
        info = Mock(st_uid=1000, st_mode=0o100444)
        with (
            patch.object(c.os, "open", return_value=4),
            patch.object(c.os, "close"),
            patch.object(c.os, "fstat", return_value=info),
        ):
            with self.assertRaises(c.Unavailable):
                c.SystemBackend().read(c.ROOT + "/input.json", sealed=True)
        with patch.object(c.os, "open", side_effect=OSError("nofollow")):
            with self.assertRaises(OSError):
                c.SystemBackend().read(c.ROOT + "/input.json", sealed=True)

    def test_decode_bounds(self) -> None:
        for raw in (b'{"a":1,"a":2}', b"NaN", b"x" * (c.LIMIT + 1)):
            with self.assertRaises(c.Unavailable):
                c.decode(raw)

    def test_cleanup_foreign_comparison(self) -> None:
        base = c.collect("baseline", NONCE, CASE, backend=Fixture())
        live, stopped = copy.deepcopy(base), copy.deepcopy(base)
        live["provenance"]["stage"] = "connected"
        stopped["provenance"]["stage"] = "stopped"
        live["observations"]["network"]["links"] = ["d" * 64]
        live["observations"]["units"][c.UNITS[0]]["members"] = [
            {"pid": 8, "start_ticks": 4, "uids": [0] * 4}
        ]
        stopped["observations"]["tracked_processes"] = [
            {"pid": 8, "start_ticks": 4, "absent": True}
        ]
        self.assertEqual(c.compare_cleanup(base, live, stopped)["status"], "ACTUAL")
        stopped["observations"]["network"]["links"] = ["e" * 64]
        self.assertFalse(c.compare_cleanup(base, live, stopped)["network_restored"])
        stopped["observations"]["network"] = base["observations"]["network"]
        stopped["observations"]["tracked_processes"][0]["absent"] = False
        self.assertFalse(
            c.compare_cleanup(base, live, stopped)["tracked_processes_absent"]
        )

    def test_command_overflow_deadline_and_reap_errors(self) -> None:
        for failure in ("overflow", "deadline", "reap"):
            with self.subTest(failure=failure):
                child = Mock()
                child.stdout.fileno.return_value = 5
                child.poll.return_value = None
                child.wait.return_value = 0
                selector = Mock()
                selector.select.return_value = [] if failure == "deadline" else [1]
                selector.__enter__ = Mock(return_value=selector)
                selector.__exit__ = Mock(return_value=False)
                if failure == "reap":
                    child.wait.side_effect = c.subprocess.TimeoutExpired("mock", 5)
                with (
                    patch.object(c.subprocess, "Popen", return_value=child),
                    patch.object(c.selectors, "DefaultSelector", return_value=selector),
                    patch.object(
                        c.os,
                        "read",
                        return_value=b"x" * (c.LIMIT + 1)
                        if failure == "overflow"
                        else b"",
                    ),
                ):
                    with self.assertRaises(
                        (c.Unavailable, c.subprocess.TimeoutExpired)
                    ):
                        c.SystemBackend().command("links")
                child.kill.assert_called_once()
                child.stdout.close.assert_called_once()

    def test_ipv6_only_changes_are_observed(self) -> None:
        for key in ("routes6", "rules6"):
            with self.subTest(key=key):
                fixture = Fixture()
                base = c.collect("baseline", NONCE, CASE, backend=fixture)
                self.assertIn(key, fixture.calls)
                self.assertIn("-6", c.COMMANDS[key])
                live, stopped = copy.deepcopy(base), copy.deepcopy(base)
                live["provenance"]["stage"] = "connected"
                stopped["provenance"]["stage"] = "stopped"
                live["observations"]["units"][c.UNITS[0]]["members"] = [
                    {"pid": 8, "start_ticks": 4, "uids": [0] * 4}
                ]
                stopped["observations"]["tracked_processes"] = [
                    {"pid": 8, "start_ticks": 4, "absent": True}
                ]
                stopped["observations"]["network"][key] = ["e" * 64]
                self.assertFalse(
                    c.compare_cleanup(base, live, stopped)["network_restored"]
                )
                base["observations"]["network"][key] = ["d" * 64]
                self.assertEqual(
                    c.compare_cleanup(base, live, stopped)["foreign_missing"][key],
                    ["d" * 64],
                )
                original = fixture.command
                fixture.command = lambda name: (
                    (_ for _ in ()).throw(PermissionError())
                    if name == key
                    else original(name)
                )
                self.assertEqual(
                    c.collect("baseline", NONCE, CASE, backend=fixture)["status"],
                    "UNAVAILABLE",
                )

    def test_arbitrary_command_never_spawns(self) -> None:
        with patch.object(c.subprocess, "Popen") as spawn:
            with self.assertRaises(c.Unavailable):
                c.SystemBackend().command("stop")
            spawn.assert_not_called()


if __name__ == "__main__":
    unittest.main()
