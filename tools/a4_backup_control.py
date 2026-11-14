"""Только normal UI actions и readonly D2 export текущего own diagnosis run."""

from __future__ import annotations

import json
import os
import shutil
import sys
from pathlib import Path

from a4_userdata_control import rpc

OUT = (
    Path(__file__).resolve().parents[1]
    / "docs/evidence/a4-backup-cause-142"
    / os.environ.get("A4_DIAGNOSTIC_RUN", "run2")
)


def main() -> None:
    request = json.loads(sys.argv[1])
    active = json.loads((OUT / "active.json").read_text())
    root = Path(active["root"])
    assert (
        str(root).startswith("/var/tmp/codex-myrs/transient/a4-backup-cause-")
        and root.stat().st_uid == os.getuid()
    )
    response = rpc(root, request)
    if request["action"] == "capture":
        shutil.copyfile(
            root / "home/fixture/evidence" / response["file"], OUT / response["file"]
        )
    with (OUT / "controls.jsonl").open("a") as stream:
        stream.write(json.dumps({"request": request, "response": response}) + "\n")
    os.write(1, (json.dumps(response) + "\n").encode())


if __name__ == "__main__":
    main()
