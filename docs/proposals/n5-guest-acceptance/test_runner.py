"""Исключительно pure/mock проверки; никаких guest/product процессов."""

from __future__ import annotations

import importlib.util
import json
import unittest
from copy import deepcopy
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "n5_acceptance_runner", Path(__file__).with_name("runner.py")
)
assert SPEC is not None and SPEC.loader is not None
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


def expected() -> dict:
    return {
        "caller_uid": 1000,
        "kernel_build_id": "1" * 40,
        "artifacts": {name: "2" * 64 for name in runner.ARTIFACTS},
    }


def receipt(stage: str, binding: dict | None = None) -> dict:
    facts = dict.fromkeys(runner.COUNTERS, 0)
    facts["apply_sends"] = 1
    if stage not in {"profile_unavailable", "profile_rejected"}:
        binding = binding or {name: 1 for name in runner.BINDING}
        binding.update(
            owner_uid=1000,
            invocation="1" * 32,
            **{
                name: "1" * 64
                for name in runner.BINDING
                if name.endswith("sha256") or name == "digest"
            },
        )
    coverage = None
    if stage in runner.PREFIX[1:]:
        coverage = {
            "baseline": 4,
            "current": 5,
            "errno": [1, 1, 38] + [1] * 16,
            "receipt_sha256": "3" * 64,
            "report_sha256": "4" * 64,
        }
    if stage in runner.PREFIX[2:]:
        facts["admitted"] = 1
    if stage in runner.PREFIX[3:]:
        facts["role_sends"] = 1
    if stage in runner.PREFIX[4:]:
        facts["ack"] = 1
    if stage == "healthy":
        facts["health"] = 1
    if stage in {"revoked", "cancelled", "host_lost", "unknown"}:
        facts.update(revoked=1, retained=1)
    if stage == "closed":
        facts.update(empty=1, reaped=1, closed=1)
    return {
        "schema": 1,
        "seq": 1,
        "core": "xray",
        "case": "positive",
        "stage": stage,
        "facts": facts,
        "binding": binding,
        "coverage": coverage,
        "artifacts": expected()["artifacts"],
        "kernel_build_id": expected()["kernel_build_id"],
    }


class MockDriver:
    def __init__(self, stages: list[str]) -> None:
        self.stages = stages
        self.actions: list[str] = []
        self.binding = receipt("profile")["binding"]

    def exchange(self, core: str, case: str, action: str, deadline: float) -> dict:
        self.actions.append(action)
        return receipt(self.stages.pop(0), self.binding)


class PureRunnerTests(unittest.TestCase):
    def test_default_closed_before_guest_file_or_process_access(self) -> None:
        with (
            patch.object(runner, "read_json", side_effect=AssertionError("guest read")),
            patch.object(
                runner.subprocess, "Popen", side_effect=AssertionError("runtime")
            ),
        ):
            with self.assertRaisesRegex(runner.Refused, "EXECUTION_CLOSED"):
                runner.execute({"execution": "CLOSED"})

    def test_authorization_missing_source_blocker_and_unsealed_inputs(self) -> None:
        authorization = {
            "schema": "N5_AUTHORIZATION_V1",
            "execution": "AUTHORIZED",
            "source_blockers": ["real gap"],
        }
        with self.assertRaisesRegex(runner.Refused, "SOURCE_BLOCKED"):
            runner.authorize(authorization, {}, {})
        authorization["source_blockers"] = []
        with self.assertRaisesRegex(runner.Refused, "MISSING_PINS"):
            runner.authorize(authorization, {}, {})

    def test_strict_fixture_rejects_old_runner_extension(self) -> None:
        authorization = {
            "schema": "N5_AUTHORIZATION_V1",
            "execution": "AUTHORIZED",
            "source_blockers": [],
            **{
                key: "1" * 64
                for key in ("runner_sha256", "fixture_sha256", "grant_sha256")
            },
        }
        fixture = {
            "schema": "SEALED_GUEST_FIXTURE_V1",
            "source_commit": runner.SOURCE,
            "status": "SEALED",
            "n5_acceptance": expected(),
        }
        with self.assertRaisesRegex(runner.Refused, "MISSING_FIXTURE"):
            runner.authorize(authorization, fixture, {})

    def test_ready_data_and_grant_cannot_substitute_actual_driver(self) -> None:
        authorization = {
            "schema": "N5_AUTHORIZATION_V1",
            "execution": "AUTHORIZED",
            "source_blockers": [],
            "source_commit": runner.SOURCE,
            **{
                key: "1" * 64
                for key in ("runner_sha256", "fixture_sha256", "grant_sha256")
            },
        }
        grant = {
            "schema": "N5_USER_GRANT_V1",
            "explicit_user_runtime_grant": True,
            "source_commit": runner.SOURCE,
            "fixture_sha256": "1" * 64,
            "runner_sha256": "1" * 64,
            "cases": [
                f"{core}:{case}" for core in runner.CORES for case in runner.CASES
            ],
        }
        # Мок только результата DATA validator; никаких actual receipts/процессов.
        with patch.object(runner, "preflight") as preflight:
            preflight.return_value.status = "READY"
            with self.assertRaisesRegex(
                runner.Refused, "MISSING_ACTUAL_DRIVER_AND_FIXTURE"
            ):
                runner.authorize(authorization, {"source_commit": runner.SOURCE}, grant)

    def test_duplicate_bytes_and_json_nonobject_rejected(self) -> None:
        for data in (b'{"x":1,"x":2}', b"[]", b"x" * (runner.MAX_FRAME + 1)):
            with self.assertRaises(runner.Refused):
                runner.decode(data, runner.MAX_FRAME)

    def test_positive_chain_requires_actual_coverage_ack_health_and_closed(
        self,
    ) -> None:
        driver = MockDriver([*runner.PREFIX, "stopping", "closed"])
        result = runner.run_case(
            driver, "xray", "positive", expected(), runner.time.monotonic() + 100
        )
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(driver.actions.count("start"), 1)
        self.assertEqual(driver.actions.count("stop"), 1)

    def test_coverage_overflow_wrong_vector_and_absent_receipt_fail(self) -> None:
        for change in (
            {"baseline": 2**32 - 1, "current": 0},
            {"errno": [1] * 19},
            {"receipt_sha256": "0" * 64},
        ):
            data = receipt("data4")
            data["coverage"].update(change)
            with self.assertRaisesRegex(runner.Refused, "COVERAGE_MISMATCH"):
                runner.check_receipt(data, "data4", None, expected())

    def test_readiness_without_ack_or_after_revoke_is_rejected(self) -> None:
        for change in ({"ack": 0}, {"revoked": 1}):
            data = receipt("healthy")
            data["facts"].update(change)
            with self.assertRaisesRegex(runner.Refused, "FALSE_READINESS"):
                runner.check_receipt(data, "healthy", None, expected())

    def test_changed_canonical_pid_or_generation_cannot_rebind(self) -> None:
        previous = receipt("profile")["binding"]
        for key in ("worker_pid", "worker_start", "generation", "epoch", "invocation"):
            data = receipt("data4")
            data["binding"][key] = "2" * 32 if key == "invocation" else 2
            with self.assertRaisesRegex(runner.Refused, "CANONICAL_CHANGED"):
                runner.check_receipt(data, "data4", previous, expected())

    def test_missing_tampered_profile_or_report_never_admits(self) -> None:
        for stage in ("profile_unavailable", "profile_rejected", "report_rejected"):
            data = receipt(stage)
            data["facts"]["admitted"] = 1
            if data["binding"] is None:
                data["binding"] = receipt("profile")["binding"]
            with self.assertRaisesRegex(runner.Refused, "DENIAL_BYPASS"):
                runner.check_receipt(data, stage, None, expected())

    def test_cancel_revoke_hostloss_unknown_require_retained_custody(self) -> None:
        for stage in ("cancelled", "revoked", "host_lost", "unknown"):
            data = receipt(stage)
            data["facts"]["retained"] = 0
            with self.assertRaisesRegex(runner.Refused, "EARLY_RELEASE"):
                runner.check_receipt(data, stage, None, expected())

    def test_unknown_stop_without_closed_cleanup_quarantines_no_resend(self) -> None:
        driver = MockDriver([*runner.PREFIX, "unknown", "unknown"])
        result = runner.run_case(
            driver, "xray", "unknown-stop", expected(), runner.time.monotonic() + 100
        )
        self.assertEqual(result["status"], "QUARANTINED")
        self.assertEqual(driver.actions.count("stop"), 1)
        self.assertEqual(driver.actions.count("start"), 1)

    def test_raw_fields_bool_counters_and_extra_keys_are_rejected(self) -> None:
        for key, value in (("raw_log", "secret"), ("stdout", "raw")):
            data = receipt("healthy")
            data[key] = value
            with self.assertRaisesRegex(runner.Refused, "STAGE_PROTOCOL"):
                runner.check_receipt(data, "healthy", None, expected())
        data = receipt("healthy")
        data["facts"]["health"] = True
        with self.assertRaisesRegex(runner.Refused, "COUNTER_PROTOCOL"):
            runner.check_receipt(data, "healthy", None, expected())

    def test_environment_is_fixed_no_bus_or_authority_override(self) -> None:
        env = runner.environment(1000)
        self.assertNotIn("DBUS_SYSTEM_BUS_ADDRESS", env)
        self.assertFalse(any(key.startswith("YONIK_") for key in env))
        self.assertEqual(
            runner.DRIVER_ARGV,
            ("/usr/lib/yonik-acceptance/n5-guest-driver", "--receipt-protocol=n5-v1"),
        )

    def test_receipt_binding_and_artifact_change_are_rejected(self) -> None:
        data = receipt("profile")
        data["artifacts"] = deepcopy(data["artifacts"])
        data["artifacts"]["worker"] = "5" * 64
        with self.assertRaisesRegex(runner.Refused, "ACTUAL_ARTIFACT_BINDING"):
            runner.check_receipt(data, "profile", None, expected())

    def test_all_fixed_cases_have_bounded_single_start(self) -> None:
        self.assertEqual(len(runner.CASES) * len(runner.CORES), 18)
        for steps in runner.CASES.values():
            self.assertEqual(sum(action == "start" for action, _ in steps), 1)
            self.assertLessEqual(len(steps), 7)
        self.assertEqual(
            json.loads((Path(__file__).parent / "authorization.json").read_text())[
                "execution"
            ],
            "CLOSED",
        )


if __name__ == "__main__":
    unittest.main()
