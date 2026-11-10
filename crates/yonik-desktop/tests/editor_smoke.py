"""Настоящая Qt-форма с синтетическим draft и ephemeral DB."""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from desktop_ui_smoke import execute, ui  # noqa: E402


def main() -> None:
    binary = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    expected = {
        "version": 1,
        "default_action": "proxy",
        "rules": [
            {
                "id": 1,
                "enabled": True,
                "executables": [],
                "domains": ["example.org"],
                "destinations": [],
                "action": "proxy",
            }
        ],
    }
    with tempfile.TemporaryDirectory(prefix="yonik-editor-smoke-") as temporary:
        execute(
            binary,
            Path(temporary),
            [
                ui("click", "settingsButton", when={"ready": True}),
                ui("click", "settingsRoutingButton"),
                ui("click", "routingModeButton"),
                ui("text", "routingDomain", value="example.org"),
                ui("click", "routingSaveRule"),
                ui("click", "routingModeButton", capture="simple.png"),
                ui("click", "routingModeButton", capture="advanced.png"),
                {
                    "when": {"routing": expected},
                    "command": {"command": "shutdown"},
                },
            ],
            output,
            "1",
        )


if __name__ == "__main__":
    main()
