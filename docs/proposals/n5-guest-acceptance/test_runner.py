"""Pure v2 contract tests; никакого guest/helper/worker запуска."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import unittest
from copy import deepcopy
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "n5_runner", Path(__file__).with_name("runner.py")
)
assert SPEC and SPEC.loader
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


def witness() -> tuple[dict, dict]:
    # Только mocked vectors для parser/consumer; не runtime evidence.
    capture, source = "a" * 64, "b" * 64
    controls = {
        "caller_uid": 1000,
        "profile_id": 1,
        "source_id": 1,
        "capture_nonce": "c" * 64,
        "capture_authorization_sha256": "d" * 64,
        "collector_sha256": "e" * 64,
        "kernel_boot_id": "12345678-1234-1234-1234-123456789abc",
        "flows": [],
    }
    expected = {
        "trusted_capture_sha256": capture,
        "executing_source_sha256": source,
        "source_commit": "f" * 40,
        "manifest_sha256": "9" * 64,
        "core": "xray",
        "artifacts": {"/mock/elf": {"sha256": "8" * 64, "size": 128}},
        "controls": controls,
    }
    helper = {
        "main": {"pid": 2, "start_ticks": 1, "uids": [0] * 4},
        "members": [],
        "populated": 1,
    }
    member = {"pid": 3, "start_ticks": 2, "uids": [997] * 4}
    captures = []
    for stage in ("baseline", "connected", "stopped"):
        observations = {
            "artifacts": expected["artifacts"],
            "network": {
                key: (["7" * 64] if stage == "connected" and key == "links" else [])
                for key in (
                    "links",
                    "addresses",
                    "routes",
                    "rules",
                    "routes6",
                    "rules6",
                    "nft",
                )
            },
            "units": {
                "yonik-helper.service": helper,
                "yonik-worker.service": {
                    "main": member if stage == "connected" else None,
                    "members": [member] if stage == "connected" else [],
                    "populated": int(stage == "connected"),
                },
            },
        }
        if stage == "stopped":
            observations["tracked_processes"] = [
                {"pid": 3, "start_ticks": 2, "absent": True}
            ]
        captures.append(
            {
                "schema": 1,
                "status": "ACTUAL",
                "provenance": {
                    "source": expected["source_commit"],
                    "manifest_sha256": expected["manifest_sha256"],
                    "case": "xray_positive",
                    "stage": stage,
                    "nonce": controls["capture_nonce"],
                    "authorization_sha256": controls["capture_authorization_sha256"],
                    "collector_sha256": controls["collector_sha256"],
                    "kernel_boot_id": controls["kernel_boot_id"],
                },
                "observations": observations,
            }
        )
    flow_observations = []
    for transport in ("tcp", "udp"):
        for app in ("rust", "python"):
            for action in ("proxy", "direct", "block"):
                flow = {
                    "transport": transport,
                    "application": app,
                    "action": action,
                    "endpoint": "10.0.0.4:100",
                    "token": f"{transport}_{app}_{action}",
                    "response": "server",
                    "expected_peer_ip": "10.0.0.3",
                }
                controls["flows"].append(flow)
                token = flow["token"] + ":" + "1" * 32
                flow_observations.append(
                    {
                        "class": "ACTUAL",
                        "flow": flow,
                        "child_reaped": True,
                        "pid": 8,
                        "start": 2,
                        "uid": 1000,
                        "client": {
                            "remote": flow["endpoint"],
                            "transport": transport,
                            "received": action != "block",
                            "reply": token + ":server" if action != "block" else "",
                        },
                        "endpoint_capture": {
                            "token": token,
                            "transport": transport,
                            "endpoint": flow["endpoint"],
                            "seen": int(action != "block"),
                            "server": "server",
                            "peer_ip": "10.0.0.3",
                        },
                    }
                )
    observations = {"flows": flow_observations, "root_captures": captures}
    commitment = {
        "witness_sha256": hashlib.sha256(
            json.dumps(
                observations, sort_keys=True, separators=(",", ":"), ensure_ascii=False
            ).encode()
        ).hexdigest()
    }
    actual = {
        "desktop_system_connected": {
            "status": "Connected",
            "connection_mode": "system",
            "policy_requires_attribution": True,
            "selected_profile": 1,
            "source_id": 1,
            "engine": "xray",
        },
        "trusted_execution_tuple": {
            "source_sha256": source,
            "launch_sha256": capture,
            "live_verified": True,
        },
        "tcp_udp_policy": commitment,
        "app_owner_policy": commitment,
    }
    actual.update({name: commitment for name in runner.ACTUAL if name not in actual})
    claims = {
        name: {
            "class": "ACTUAL",
            "capture_sha256": capture,
            "observation": actual[name],
        }
        for name in runner.ACTUAL
    }
    claims.update(
        {
            name: {
                "class": "SOURCE_INVARIANT",
                "source_sha256": source,
                "anchor_sha256": capture,
            }
            for name in runner.INFERRED
        }
    )
    claims.update(
        {
            name: {"class": "UNAVAILABLE", "reason": "SOURCE_ONLY"}
            for name in runner.UNAVAILABLE
        }
    )
    return (
        {
            "schema": 2,
            "seq": 4,
            "core": "xray",
            "case": "positive",
            "stage": "witness",
            "claims": claims,
            "observations": observations,
        },
        expected,
    )


class Contract(unittest.TestCase):
    def test_tagged_witness(self) -> None:
        value, expected = witness()
        runner.check_receipt(value, expected)

    def test_actual_cannot_be_inferred(self) -> None:
        value, expected = witness()
        for name in runner.ACTUAL:
            changed = deepcopy(value)
            changed["claims"][name] = {
                "class": "SOURCE_INVARIANT",
                "source_sha256": "b" * 64,
                "anchor_sha256": "a" * 64,
            }
            with self.assertRaises(runner.Refused):
                runner.check_receipt(changed, expected)

    def test_untrusted_echo_is_not_capture(self) -> None:
        value, expected = witness()
        expected.pop("trusted_capture_sha256")
        with self.assertRaises(runner.Refused):
            runner.check_receipt(value, expected)

    def test_legacy_counter_protocol_rejected(self) -> None:
        value, expected = witness()
        value["schema"] = 1
        value["facts"] = {"admitted": 1}
        with self.assertRaises(runner.Refused):
            runner.check_receipt(value, expected)

    def test_unknown_fields_and_boolean_ids_rejected(self) -> None:
        value, expected = witness()
        value["observations"]["root_captures"][2]["observations"]["units"][
            "yonik-worker.service"
        ]["populated"] = False
        with self.assertRaises(runner.Refused):
            runner.check_receipt(value, expected)

    def test_unavailable_cases_do_not_exchange(self) -> None:
        class NoDriver:
            def exchange(self, *args: object) -> dict:
                raise AssertionError("Нельзя запускать source-only случай")

        for case in runner.CASES:
            if case != "positive":
                result = runner.run_case(NoDriver(), "xray", case, {}, 0)
                self.assertEqual(result["status"], "UNAVAILABLE")

    def test_closed_execution_has_no_file_reads_or_launches(self) -> None:
        with (
            patch.object(runner, "read_json", side_effect=AssertionError),
            patch.object(runner.subprocess, "Popen", side_effect=AssertionError),
        ):
            with self.assertRaisesRegex(runner.Refused, "EXECUTION_CLOSED"):
                runner.execute({"execution": "CLOSED"})

    def test_nonstring_case_is_protocol_error(self) -> None:
        value, expected = witness()
        value["case"] = []
        with self.assertRaises(runner.Refused):
            runner.check_receipt(value, expected)

    def test_missing_ipv6_is_not_empty_ipv6_capture(self) -> None:
        value, expected = witness()
        observed = value["observations"]["root_captures"]
        del observed[2]["observations"]["network"]["routes6"]
        with self.assertRaises(runner.Refused):
            runner.check_cleanup(
                {"class": "ACTUAL", "root_captures": observed}, expected
            )

    def test_duplicate_keys_and_bounds(self) -> None:
        for raw in (b'{"a":1,"a":2}', b"[]", b"x" * 65537):
            with self.assertRaises(runner.Refused):
                runner.decode(raw)


if __name__ == "__main__":
    unittest.main()
