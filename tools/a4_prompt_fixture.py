"""Реальный KDE Secret Service prompt в отдельном own namespace/private bus."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


def main() -> None:
    out = Path(__file__).resolve().parents[1] / "docs/evidence/a4-mtree4/gui-run"
    state = json.loads((out / "active.json").read_text())
    root = Path(state["root"])
    (root / "home/fixture/prompt.py").write_text("""import os, subprocess, threading
from worker import bridge
process = subprocess.Popen(['/usr/lib/ksecretprompter'])
threading.Thread(target=bridge,args=('vnc-prompt',5902),daemon=True).start()
raise SystemExit(process.wait())
""")
    args = state["argv"][:-2] + [
        "--setenv",
        "QT_QPA_PLATFORM",
        "vnc:size=1024x900:port=5902",
        "/usr/bin/python",
        "/home/fixture/prompt.py",
    ]
    with (out / "prompt.log").open("w") as log:
        process = subprocess.Popen(args, stdout=log, stderr=log)
        (out / "prompt-process.json").write_text(
            json.dumps({"pid": process.pid, "argv": args}, indent=2) + "\n"
        )
        code = process.wait()
        (out / "prompt-exit.json").write_text(
            json.dumps({"pid": process.pid, "exit": code}) + "\n"
        )


if __name__ == "__main__":
    main()
