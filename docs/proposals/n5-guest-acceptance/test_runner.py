"""Pure v2 contract tests; никакого guest/helper/worker запуска."""
from __future__ import annotations
import importlib.util
import unittest
from copy import deepcopy
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("n5_runner", Path(__file__).with_name("runner.py"))
assert SPEC and SPEC.loader
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


def witness():
    capture, source = "a" * 64, "b" * 64
    observations = {name: i + 1 for i, name in enumerate(sorted(runner.ACTUAL))}
    claims = {name: {"class": "ACTUAL", "capture_sha256": capture, "observation_id": i}
              for name, i in observations.items()}
    claims.update({name: {"class": "SOURCE_INVARIANT", "source_sha256": source,
                          "anchor_sha256": capture} for name in runner.INFERRED})
    claims.update({name: {"class": "UNAVAILABLE", "reason": "SOURCE_ONLY"}
                   for name in runner.UNAVAILABLE})
    return ({"schema": 2, "seq": 4, "core": "xray", "case": "positive",
             "stage": "witness", "claims": claims},
            {"trusted_capture_sha256": capture, "executing_source_sha256": source,
             "verified_observations": observations})


class Contract(unittest.TestCase):
    def test_tagged_witness(self):
        value, expected = witness()
        runner.check_receipt(value, expected)

    def test_actual_cannot_be_inferred(self):
        value, expected = witness()
        for name in runner.ACTUAL:
            changed = deepcopy(value)
            changed["claims"][name] = {"class": "SOURCE_INVARIANT",
                "source_sha256": "b" * 64, "anchor_sha256": "a" * 64}
            with self.assertRaises(runner.Refused):
                runner.check_receipt(changed, expected)

    def test_untrusted_echo_is_not_capture(self):
        value, expected = witness()
        expected.pop("verified_observations")
        with self.assertRaises(runner.Refused):
            runner.check_receipt(value, expected)

    def test_legacy_counter_protocol_rejected(self):
        value, expected = witness()
        value["schema"] = 1
        value["facts"] = {"admitted": 1}
        with self.assertRaises(runner.Refused):
            runner.check_receipt(value, expected)

    def test_unknown_fields_and_boolean_ids_rejected(self):
        value, expected = witness()
        value["claims"]["worker_reaped"]["observation_id"] = True
        with self.assertRaises(runner.Refused):
            runner.check_receipt(value, expected)

    def test_unavailable_cases_do_not_exchange(self):
        class NoDriver:
            def exchange(self, *args):
                raise AssertionError("Нельзя запускать source-only случай")
        for case in runner.CASES:
            if case != "positive":
                result = runner.run_case(NoDriver(), "xray", case, {}, 0)
                self.assertEqual(result["status"], "UNAVAILABLE")

    def test_closed_execution_has_no_file_reads_or_launches(self):
        with patch.object(runner, "read_json", side_effect=AssertionError), \
             patch.object(runner.subprocess, "Popen", side_effect=AssertionError):
            with self.assertRaisesRegex(runner.Refused, "EXECUTION_CLOSED"):
                runner.execute({"execution": "CLOSED"})

    def test_nonstring_case_is_protocol_error(self):
        value, expected = witness()
        value["case"] = []
        with self.assertRaises(runner.Refused):
            runner.check_receipt(value, expected)

    def test_duplicate_keys_and_bounds(self):
        for raw in (b'{"a":1,"a":2}', b"[]", b"x" * 65537):
            with self.assertRaises(runner.Refused):
                runner.decode(raw)


if __name__ == "__main__":
    unittest.main()
