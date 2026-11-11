"""Отказ VM acceptance при недоказанной смене Secret Service owner."""

import copy
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "credential_vm_fixture", ROOT / "tools/credential_vm_fixture.py"
)
assert SPEC is not None and SPEC.loader is not None
fixture = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fixture)


class CredentialResultsTests(unittest.TestCase):
    def test_stale_owner_requires_identity_and_denial(self) -> None:
        accepted = {
            "owner_changed": True,
            "stale_session_open": {"state": "session_open", "pid": 42},
            "stale_after_owner_change": {
                "pid": 42,
                "get_secret_readable": 0,
                "get_secret_refused": 1,
            },
        }
        fixture.verify_results(accepted)
        for field, value in [
            ("owner_changed", False),
            ("stale_session_open", {"state": "session_open", "pid": True}),
            (
                "stale_after_owner_change",
                {"pid": 42, "get_secret_readable": 1, "get_secret_refused": 0},
            ),
            (
                "stale_after_owner_change",
                {"pid": 43, "get_secret_readable": 0, "get_secret_refused": 1},
            ),
            ("stale_after_owner_change", None),
        ]:
            changed = copy.deepcopy(accepted)
            changed[field] = value
            with (
                self.subTest(field=field, value=value),
                self.assertRaises(RuntimeError),
            ):
                fixture.verify_results(changed)
        with self.assertRaises(RuntimeError):
            fixture.verify_results({})
