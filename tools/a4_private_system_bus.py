"""Пустой настоящий private system bus: own UID, без service dirs/activation/manager."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


def main() -> None:
    out = Path(__file__).resolve().parents[1] / "docs/evidence/a4-userdata-142/session7"
    state = json.loads((out / "active.json").read_text())
    root = Path(state["root"])
    config = root / "home/fixture/private-system.conf"
    config.write_text(
        '<busconfig><type>system</type><listen>unix:path=/home/fixture/runtime/no-system-bus</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
    )
    args = state["argv"][:-2] + [
        "/usr/bin/dbus-daemon",
        "--nofork",
        "--config-file=/home/fixture/private-system.conf",
    ]
    with (out / "private-system-bus.log").open("w") as log:
        process = subprocess.Popen(args, stdout=log, stderr=log)
        (out / "private-system-bus-process.json").write_text(
            json.dumps(
                {
                    "argv": args,
                    "pid": process.pid,
                    "purpose": "real empty bus, no activation directories, no systemd/helper/worker",
                },
                indent=2,
            )
            + "\n"
        )
        code = process.wait()
        (out / "private-system-bus-exit.json").write_text(
            json.dumps({"pid": process.pid, "exit": code}) + "\n"
        )


if __name__ == "__main__":
    main()
