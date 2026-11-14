"""Pure coordinator tests; worker и системные эффекты замещены."""

import concurrent.futures
import unittest
from unittest.mock import Mock, patch

import coordinator as c
import runner


class Coordinator(unittest.TestCase):
    def tearDown(self) -> None:
        c._RETAINED.clear()

    def test_gate_denial_zero_effects(self) -> None:
        for reason in ("MISSING_USER_GRANT", "ROOT_CAPTURE_AUTH"):
            with (
                patch.object(c, "_gate", side_effect=runner.Refused(reason)),
                patch.object(c, "_Owner") as owner,
                patch.object(c, "_root_directory") as parent,
                patch.object(runner, "ProcessDriver") as driver,
            ):
                with self.assertRaises(runner.Refused):
                    c.run_owned_guest("xray", "positive")
                owner.assert_not_called()
                parent.assert_not_called()
                driver.assert_not_called()

    def test_late_completion_retains_owner(self) -> None:
        future = Mock()
        future.result.side_effect = concurrent.futures.TimeoutError
        expected = {"controls": {"capture_nonce": "a" * 64}}
        with (
            patch.object(c.concurrent.futures, "ThreadPoolExecutor") as executor,
            patch.object(c, "_gate", return_value=(expected, 1000, 100)),
            patch.object(c.time, "monotonic", side_effect=range(100)),
        ):
            executor.return_value.submit.return_value = future
            owner = c._Owner("xray", expected)
            with self.assertRaises(c.Quarantined) as error:
                owner.publish("baseline", 10)
            future.result.side_effect = None
            self.assertIs(error.exception.owner, owner)
            self.assertIn(owner, c._RETAINED)
            with self.assertRaises(runner.Refused):
                owner.publish("connected", 10)
            executor.return_value.submit.assert_called_once()

    def test_strict_json(self) -> None:
        for raw in (b'{"x":1,"x":2}', b'{"x":NaN}'):
            with self.assertRaises(runner.Refused):
                c._json(raw)

    def test_negative_does_not_read_gate(self) -> None:
        with patch.object(c, "_gate") as gate:
            for core in runner.CORES:
                for case in runner.CASES:
                    if case != "positive":
                        self.assertEqual(
                            c.run_owned_guest(core, case)["status"], "UNAVAILABLE"
                        )
            gate.assert_not_called()


if __name__ == "__main__":
    unittest.main()


class SourceOrder(unittest.TestCase):
    def test_all_publications_and_launch(self) -> None:
        from test_phase_exchange import Fake

        events = []
        driver = Fake()
        expected = {"controls": {"capture_nonce": "a" * 64}}

        def submit(call, *args):
            events.append(args[0] if len(args) == 3 else "unit")
            future = concurrent.futures.Future()
            future.set_result(None)
            return future

        def initialize(obj, uid, gid):
            events.append("spawn")
            obj.process = Mock()
            obj.process.poll.return_value = None
            obj.exchange = driver.exchange
            obj.close = Mock()

        with (
            patch.object(c, "_gate", return_value=(expected, 1000, 100)),
            patch.object(c.os, "geteuid", return_value=0),
            patch.object(c, "_root_directory", return_value=9),
            patch.object(c, "_absent"),
            patch.object(c.fcntl, "flock"),
            patch.object(c.os, "open", return_value=10),
            patch.object(c.os, "write", return_value=1),
            patch.object(c.os, "fsync"),
            patch.object(c.os, "close"),
            patch.object(c.concurrent.futures, "ThreadPoolExecutor") as executor,
            patch.object(runner.ProcessDriver, "__init__", initialize),
            patch.object(c, "_validate_child"),
            patch.object(c.select, "select", return_value=([], [], [])),
            patch.object(runner, "check_receipt"),
        ):
            executor.return_value.submit.side_effect = submit
            self.assertEqual(c.run_owned_guest("xray", "positive")["status"], "PASS")
        self.assertEqual(events, ["unit", "spawn", "baseline", "connected", "stopped"])
        self.assertEqual(
            driver.events, ["start", "await", "stop", "close_desktop_owner", "cleanup"]
        )


class SealedReads(unittest.TestCase):
    def test_writable_ancestor_and_nonregular_are_rejected(self) -> None:
        import stat
        from types import SimpleNamespace

        good = SimpleNamespace(st_uid=0, st_mode=stat.S_IFDIR | 0o755)
        bad = SimpleNamespace(st_uid=0, st_mode=stat.S_IFDIR | 0o777)
        fifo = SimpleNamespace(st_uid=0, st_mode=stat.S_IFIFO | 0o444)
        for metadata in ((bad,), (good, fifo)):
            with (
                patch.object(c.os, "open", return_value=10),
                patch.object(c.os, "close"),
                patch.object(c.os, "fstat", side_effect=metadata),
                patch.object(c.os, "read") as read,
            ):
                with self.assertRaises(runner.Refused):
                    c._read("/sealed", 0o444)
                read.assert_not_called()

    def test_duplicate_json_never_becomes_authority(self) -> None:
        with (
            patch.object(
                c,
                "_read",
                return_value=b'{"execution":"AUTHORIZED","execution":"CLOSED"}',
            ),
            patch.object(c, "_root_directory") as root,
            patch.object(runner, "ProcessDriver") as driver,
        ):
            with self.assertRaises(runner.Refused):
                c.run_owned_guest("xray", "positive")
            root.assert_not_called()
            driver.assert_not_called()
