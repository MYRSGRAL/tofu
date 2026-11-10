"""Два запуска настоящего Qt с отдельными синтетическими настройками."""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from desktop_ui_smoke import execute, ui  # noqa: E402


def main() -> None:
    binary = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="yonik-preferences-qt-") as temporary:
        root = Path(temporary)
        first = output / "first"
        second = output / "second"
        first.mkdir(exist_ok=True)
        second.mkdir(exist_ok=True)
        execute(
            binary,
            root,
            [
                {
                    "when": {"ready": True},
                    "command": {"command": "set_engine", "engine": "sing-box"},
                },
                {
                    "when": {"engine": "sing-box"},
                    "command": {
                        "command": "set_probe_target",
                        "url": "https://example.com/check",
                        "expected_status": 200,
                    },
                },
                {
                    "when": {"probe_expected_status": 200},
                    "command": {"command": "set_language", "language": "en"},
                },
                ui("click", "settingsButton", expect={"window": {"language": "en"}}),
                {"when": {}, "command": {"command": "shutdown"}},
            ],
            first,
            "1",
        )
        # Smoke создаёт новый ephemeral key на процесс: второй запуск получает
        # только preferences, без БД с ключом первого процесса.
        reopened = root / "reopened"
        (reopened / "data").mkdir(parents=True, mode=0o700)
        shutil.copy2(
            root / "data" / "preferences.json", reopened / "data" / "preferences.json"
        )
        execute(
            binary,
            reopened,
            [
                ui(
                    "click",
                    "settingsButton",
                    when={
                        "ready": True,
                        "engine": "sing-box",
                        "probe_custom": True,
                        "probe_expected_status": 200,
                    },
                    expect={"window": {"language": "en"}},
                ),
                {"when": {}, "command": {"command": "shutdown"}},
            ],
            second,
            "1",
        )
        settings = json.loads((root / "data" / "preferences.json").read_text())
        assert settings["language"] == "en"
        assert settings["engine"] == "sing-box"
        assert settings["probe_url"] == "https://example.com/check"
        (output / "result.json").write_text(
            json.dumps(
                {
                    "launches": 2,
                    "restored": ["engine", "probe_url", "probe_status", "language"],
                }
            )
            + "\n"
        )


if __name__ == "__main__":
    main()
