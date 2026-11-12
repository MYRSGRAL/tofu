"""Отрицательные gates: не принимать замену item, процесса или оригинала."""

import sys
import unittest
from copy import deepcopy
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from format4_gui_verify import verify  # noqa: E402


def fixture() -> list[dict[str, object]]:
    state = {
        "count": 1,
        "owner_pid_matches": True,
        "items": [["/opaque/item1"], []],
        "subscriptions.sqlite": {"version": 4, "integrity": True},
        "backup3.sqlite": {"version": 3, "integrity": True},
    }
    stages = {
        "before_gui": {"count": 0},
        "db3": deepcopy(state),
        "first_launch": {"pid": 10},
        "second_launch": {"pid": 20},
        "first_closed": {"closed": True},
        "second_closed": {"closed": True},
        "first_witness": {
            "original_matches": True,
            "revision_matches": True,
            "count": 2,
        },
        "second_witness": {
            "original_matches": True,
            "revision_matches": True,
            "count": 2,
        },
        "finish": {"name_disappeared": True},
    }
    stages["db3"]["subscriptions.sqlite"]["version"] = 3
    for stage in ["backup3", "db4", "after_close", "after_reopen_close"]:
        stages[stage] = deepcopy(state)
    stages["backup3"]["subscriptions.sqlite"]["version"] = 3
    return [{"stage": stage, "result": result} for stage, result in stages.items()]


class Gates(unittest.TestCase):
    def test_public_evidence(self) -> None:
        verify(fixture())

    def test_reject_false_witnesses(self) -> None:
        for stage, field, value in [
            ("db4", "items", [["/opaque/replaced"], []]),
            ("after_close", "count", 2),
            ("after_reopen_close", "owner_pid_matches", False),
            ("second_launch", "pid", 10),
            ("first_witness", "original_matches", False),
            ("second_witness", "revision_matches", False),
            ("first_closed", "closed", False),
            ("finish", "name_disappeared", False),
        ]:
            with self.subTest(stage=stage, field=field):
                rows = fixture()
                next(row["result"] for row in rows if row["stage"] == stage)[field] = (
                    value
                )
                with self.assertRaises(AssertionError):
                    verify(rows)

    def test_missing_stage(self) -> None:
        with self.assertRaises(KeyError):
            verify(fixture()[:-1])


if __name__ == "__main__":
    unittest.main()
