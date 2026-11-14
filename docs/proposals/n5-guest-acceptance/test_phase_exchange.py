"""Pure protocol/lifecycle tests: без Popen и runtime I/O."""

import unittest
from unittest.mock import patch

import runner


class Fake:
    def __init__(self) -> None:
        self.events: list[str] = []
        self.deadlines: list[float] = []
        self.fail = ""

    def before_start(self, deadline: float) -> None:
        self.events.append("baseline")

    def after_await(self, deadline: float) -> None:
        self.events.append("connected")

    def after_close_desktop_owner(self, deadline: float) -> None:
        self.events.append("stopped")

    def exchange(self, core: str, case: str, action: str, deadline: float) -> dict:
        self.events.append(action)
        self.deadlines.append(deadline)
        if action == self.fail:
            raise runner.Refused("EOF")
        seq = sum(
            x in ("start", "await", "stop", "close_desktop_owner", "cleanup")
            for x in self.events
        )
        return dict(schema=2, seq=seq, core=core, case=case, stage=action)


class Lifecycle(unittest.TestCase):
    def test_order_and_deadline(self) -> None:
        driver = Fake()
        with (
            patch.object(runner.time, "monotonic", return_value=10),
            patch.object(runner, "check_receipt") as check,
        ):
            result = runner.run_case(driver, "xray", "positive", {}, 100)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(
            driver.events,
            [
                "baseline",
                "start",
                "await",
                "connected",
                "stop",
                "close_desktop_owner",
                "stopped",
                "cleanup",
            ],
        )
        self.assertEqual(driver.deadlines, [100, 100, 100, 100, 55])
        check.assert_called_once()

    def test_lost_boundary_never_sends_recovery(self) -> None:
        for action in ("start", "await", "stop", "close_desktop_owner", "cleanup"):
            driver = Fake()
            driver.fail = action
            with patch.object(runner.time, "monotonic", return_value=10):
                result = runner.run_case(driver, "xray", "positive", {}, 100)
            self.assertEqual(result["status"], "QUARANTINED")
            self.assertEqual(driver.events[-1], action)
            self.assertLessEqual(driver.events.count("stop"), 1)

    def test_nonpositive_has_zero_effects(self) -> None:
        driver = Fake()
        for case in runner.CASES:
            if case != "positive":
                self.assertEqual(
                    runner.run_case(driver, "xray", case, {}, 100)["status"],
                    "UNAVAILABLE",
                )
        self.assertEqual(driver.events, [])


if __name__ == "__main__":
    unittest.main()


class Frames(unittest.TestCase):
    def driver(self) -> runner.ProcessDriver:
        driver = runner.ProcessDriver.__new__(runner.ProcessDriver)
        from unittest.mock import Mock

        driver.process = Mock()
        driver.process.stdin.fileno.return_value = 1
        driver.process.stdout.fileno.return_value = 2
        driver.seq = 0
        driver.total = 0
        return driver

    def test_fragmentation_and_extra_frame(self) -> None:
        import json

        frame = (
            json.dumps(
                dict(schema=2, seq=1, core="xray", case="positive", stage="start")
            ).encode()
            + b"\n"
        )
        for chunks, valid in (
            ([frame[:10], frame[10:]], True),
            ([frame + frame], False),
        ):
            driver = self.driver()
            with (
                patch.object(runner.time, "monotonic", return_value=1),
                patch.object(
                    runner.select,
                    "select",
                    side_effect=lambda r, w, e, timeout: (
                        ([], [], []) if timeout == 0 else ([2], [1], [])
                    ),
                ),
                patch.object(runner.os, "write", side_effect=lambda fd, raw: len(raw)),
                patch.object(runner.os, "read", side_effect=chunks),
            ):
                if valid:
                    self.assertEqual(
                        driver.exchange("xray", "positive", "start", 10)["seq"], 1
                    )
                else:
                    with self.assertRaises(runner.Refused):
                        driver.exchange("xray", "positive", "start", 10)


class ActualCleanup(unittest.TestCase):
    def test_closed_host_does_not_replace_empty_cgroup(self) -> None:
        from test_runner import witness

        receipt, expected = witness()
        captures = receipt["observations"]["root_captures"]
        captures[-1]["observations"]["units"]["yonik-worker.service"]["populated"] = 1
        with self.assertRaises(runner.Refused):
            runner.check_cleanup(
                {"class": "ACTUAL", "root_captures": captures}, expected
            )
