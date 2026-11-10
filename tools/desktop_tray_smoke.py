"""Изолированный Qt close regression: без трея окно остаётся доступным."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path

from desktop_ui_smoke import execute


def run(binary: Path, output: Path) -> None:
    """Проверить настоящий close и явный выход в отдельной session bus."""
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="yonik-tray-") as temporary:
        root = Path(temporary)
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        os.environ["XDG_RUNTIME_DIR"] = str(runtime)
        steps = [
            {"when": {"ready": True, "closed": False},
             "ui_when": {"window": {"visible": True}},
             "ui": {"action": "close"}},
            {"when": {"ready": True, "closed": False},
             "ui_when": {"window": {"visible": True}},
             "ui": {"action": "quit"}, "capture_name": "after-close.png"},
        ]
        events = execute(binary, root, steps, output, "1")
        if any(event.get("tray_available") is not False for event in events):
            raise RuntimeError("Gate требует Qt без доступного трея")
        (output / "result.json").write_text(
            json.dumps({"steps": len(steps), "events": len(events),
                        "closed": events[-1]["closed"]}, indent=2) + "\n"
        )


def main() -> int:
    """Запустить проверку, не подключаясь к пользовательской D-Bus."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--isolated", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.isolated:
        return subprocess.run(
            ["dbus-run-session", "--", "python", str(Path(__file__).resolve()),
             str(args.binary.resolve()), "--output", str(args.output.resolve()),
             "--isolated"], check=False,
        ).returncode
    run(args.binary.resolve(strict=True), args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
