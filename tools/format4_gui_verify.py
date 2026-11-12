"""Offline gates публичного acceptance evidence; payload/key не требуются."""

import json
import sys
from pathlib import Path


def verify(rows: list[dict[str, object]]) -> None:
    stages = {row["stage"]: row["result"] for row in rows if row.get("stage")}
    assert stages["before_gui"]["count"] == 0
    baseline = stages["db3"]
    assert baseline["subscriptions.sqlite"] == {"version": 3, "integrity": True}
    assert baseline["count"] == 1 and baseline["owner_pid_matches"] is True
    for name in ["backup3", "db4", "after_close", "after_reopen_close"]:
        state = stages[name]
        assert state["items"] == baseline["items"] and state["count"] == 1
        assert state["owner_pid_matches"] is True
        if name == "backup3":
            assert state["subscriptions.sqlite"]["version"] == 3
        else:
            assert state["subscriptions.sqlite"]["version"] == 4
        assert state["backup3.sqlite"] == {"version": 3, "integrity": True}
        assert state["subscriptions.sqlite"]["integrity"] is True
    assert stages["first_launch"]["pid"] != stages["second_launch"]["pid"]
    for name in ["first_closed", "second_closed"]:
        assert stages[name]["closed"] is True
    for name in ["first_witness", "second_witness"]:
        assert stages[name] == {
            "original_matches": True,
            "revision_matches": True,
            "count": 2,
        }
    assert stages["finish"]["name_disappeared"] is True


if __name__ == "__main__":
    verify([json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()])
    print("Acceptance gates PASS (GUI screenshots требуют отдельного visual review)")
