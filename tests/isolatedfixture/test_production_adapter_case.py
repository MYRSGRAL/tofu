"""Контракт acceptance oracle без VM и без побочных эффектов."""

import copy
import unittest
from unittest.mock import patch

from production_adapter_case import recovery_complete, wait_state


def retained_block() -> dict:
    return {
        "records": [{"id": 1, "scope": "PublicBlock", "state": "Confirmed"}],
        "public_policy": {
            "id": 1,
            "phase": {
                "phase": "Stable",
                "policy": {
                    "loopback_ifindex": 1,
                    "tun_ifindex": None,
                    "private_routes": [],
                    "lan": [],
                    "transport": [],
                    "bootstrap_dns": [],
                    "link_control": [2],
                },
            },
        },
        "session_recovery": None,
    }


class FakeBus:
    def __init__(self, replies: list[dict]) -> None:
        self.replies = iter(replies)
        self.calls = 0

    def call(self, method: str, payload: dict) -> dict:
        assert method == "State" and payload == {"version": 1}
        self.calls += 1
        return next(self.replies)


class AcceptanceOracleTests(unittest.TestCase):
    def test_retained_restrictive_block_is_recovered_before_unlock(self) -> None:
        self.assertTrue(recovery_complete(retained_block(), {"run": "0"}, True))
        empty = {"records": [], "public_policy": None, "session_recovery": None}
        self.assertFalse(recovery_complete(empty, {"run": "0"}, True))
        self.assertTrue(recovery_complete(empty, {"run": "absent"}, False))

    def test_exceptions_or_unstable_policy_are_not_recovery(self) -> None:
        for field in [
            "tun_ifindex",
            "private_routes",
            "lan",
            "transport",
            "bootstrap_dns",
        ]:
            journal = retained_block()
            journal["public_policy"]["phase"]["policy"][field] = (
                9 if field == "tun_ifindex" else [9]
            )
            with self.subTest(field=field):
                self.assertFalse(recovery_complete(journal, {"run": "0"}, True))
        journal = retained_block()
        journal["public_policy"]["phase"]["phase"] = "Removing"
        self.assertFalse(recovery_complete(journal, {"run": "0"}, True))

    def test_live_worker_or_recovery_intent_is_not_complete(self) -> None:
        self.assertFalse(recovery_complete(retained_block(), {"run": "1"}, True))
        journal = copy.deepcopy(retained_block())
        journal["session_recovery"] = {"kill_switch": True}
        self.assertFalse(recovery_complete(journal, {"run": "0"}, True))

    @patch("production_adapter_case.time.sleep")
    def test_only_typed_busy_can_precede_expected_state(self, sleep: object) -> None:
        bus = FakeBus([{"error": "busy"}, {"state": "recovery_required"}])
        wait_state(bus, "recovery_required")
        self.assertEqual(bus.calls, 2)

    @patch("production_adapter_case.time.sleep")
    def test_unknown_and_premature_unlocked_state_fail_without_retry(
        self, sleep: object
    ) -> None:
        for state in ["unknown", "no_managed_policy"]:
            bus = FakeBus([{"state": state}])
            with self.assertRaises(AssertionError):
                wait_state(bus, "recovery_required")
            self.assertEqual(bus.calls, 1)

    @patch("production_adapter_case.time.sleep")
    def test_busy_is_bounded(self, sleep: object) -> None:
        bus = FakeBus([{"error": "busy"}] * 20)
        with self.assertRaisesRegex(AssertionError, "deadline"):
            wait_state(bus, "no_managed_policy")
        self.assertEqual(bus.calls, 20)


if __name__ == "__main__":
    unittest.main()
