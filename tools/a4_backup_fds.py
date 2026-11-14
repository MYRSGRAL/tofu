"""Readonly FD/lock metadata только потомков собственного fixture worker."""

import json
import os
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
OUT = (
    BASE
    / "docs/evidence/a4-backup-cause-142"
    / os.environ.get("A4_DIAGNOSTIC_RUN", "run2")
)


def main() -> None:
    state = json.loads((OUT / "active.json").read_text())
    pending = [state["pid"]]
    rows = []
    while pending:
        pid = pending.pop()
        process = Path("/proc") / str(pid)
        try:
            assert process.stat().st_uid == os.getuid()
            pending.extend(
                map(int, (process / "task" / str(pid) / "children").read_text().split())
            )
            descriptors = []
            for fd in (process / "fd").iterdir():
                try:
                    target = os.readlink(fd)
                    if "subscriptions.sqlite" in target or "/copies/" in target:
                        descriptors.append(
                            {
                                "fd": fd.name,
                                "path": target,
                                "fdinfo": (process / "fdinfo" / fd.name).read_text(),
                            }
                        )
                except OSError:
                    continue
            rows.append(
                {
                    "pid": pid,
                    "comm": (process / "comm").read_text().strip(),
                    "fds": descriptors,
                }
            )
        except FileNotFoundError:
            continue
    (OUT / "fd-after-inspect.json").write_text(json.dumps(rows, indent=2) + "\n")
    print(json.dumps(rows))


if __name__ == "__main__":
    main()
