"""Контроли классификации evidence; не заменяют actual GUI acceptance."""

from __future__ import annotations

import copy
import unittest

from check_maintenance_generation_ui import assess_fence


def records_for(contexts: str) -> dict[str, list]:
    context = {
        "slot": "source",
        "code": "INVALID_INPUT",
        "message_key": "invalid_input",
        "request_id": None,
        "generation": "1",
        "stage": "scheduler_validation",
        "reason": "scheduler_interval_invalid",
        "retryable": False,
        "params": {},
    }
    original = [dict(context, slot="error"), context]
    export = {"schema_version": 2, "platform": "linux", "errors": original}
    records = {
        "A": [{"scheduler_generation": 1, "error_contexts": original}],
        "EXPORT A": [export],
        "CLOSED": [{"closed": True}],
        "REPLACEMENT": [],
        "BEFORE COPY": [],
        "CURRENT": [],
        "CLIPBOARD": [],
        "COPY RESULT": [],
    }
    for request, generation in ((1, 3), (2, 5)):
        errors = (
            []
            if contexts == "invalidated"
            else [dict(c, generation=str(generation)) for c in original]
            if contexts == "new"
            else original
        )
        state = {
            "scheduler_generation": generation,
            "error_contexts": errors,
            "ready": True,
            "maintenance_ack": {
                "request_id": request,
                "phase": "complete",
                "error": None,
            },
        }
        records["REPLACEMENT"].append(state)
        records["BEFORE COPY"].append(state)
        records["CURRENT"].append(state)
        records["CLIPBOARD"].append(export)
        records["COPY RESULT"].append("Export unavailable")
    return copy.deepcopy(records)


class FenceTests(unittest.TestCase):
    def test_invalidation(self) -> None:
        self.assertEqual(
            assess_fence(records_for("invalidated")), "INVALIDATED,INVALIDATED"
        )

    def test_stale_refusal(self) -> None:
        self.assertEqual(
            assess_fence(records_for("stale")), "STALE_REFUSED,STALE_REFUSED"
        )

    def test_restore_refuses_intermediate_owner(self) -> None:
        records = records_for("stale")
        for key in ("REPLACEMENT", "BEFORE COPY", "CURRENT"):
            records[key][1]["error_contexts"] = [
                dict(context, generation="3")
                for context in records[key][1]["error_contexts"]
            ]
        self.assertEqual(assess_fence(records), "STALE_REFUSED,STALE_REFUSED")

    def test_overwrite_is_not_proof(self) -> None:
        self.assertEqual(
            assess_fence(records_for("new")),
            "B_OVERWROTE_A_UNPROVEN,B_OVERWROTE_A_UNPROVEN",
        )

    def test_stale_success_rejected(self) -> None:
        records = records_for("stale")
        records["COPY RESULT"][0] = "Copied"
        with self.assertRaisesRegex(AssertionError, "stale A exported"):
            assess_fence(records)

    def test_unmatched_ack_rejected(self) -> None:
        records = records_for("invalidated")
        records["REPLACEMENT"][1]["maintenance_ack"]["request_id"] = 1
        with self.assertRaises(AssertionError):
            assess_fence(records)

    def test_changed_clipboard_on_refusal_rejected(self) -> None:
        records = records_for("invalidated")
        records["CLIPBOARD"][0] = {}
        with self.assertRaisesRegex(AssertionError, "invalidation copy"):
            assess_fence(records)


if __name__ == "__main__":
    unittest.main()
