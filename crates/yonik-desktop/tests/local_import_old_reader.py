"""Actual старый Qt binary: local DB4 не открывается, closed backup3 открывается."""

from __future__ import annotations

import hashlib
import json
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path
from typing import Any
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from desktop_ui_smoke import execute  # noqa: E402


def version(path: Path) -> int:
    with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as connection:
        return int(connection.execute("PRAGMA user_version").fetchone()[0])


def main() -> None:
    binary = Path(sys.argv[1]).resolve(strict=True)
    source = Path(sys.argv[2]).resolve(strict=True)
    output = Path(sys.argv[3]).resolve()
    output.mkdir(mode=0o700, parents=True)
    live = source / "subscriptions.sqlite"
    backup = source / "copies/before-local.sqlite"
    assert version(live) == 4
    assert version(backup) == 3
    reject = output / "reject4"
    (reject / "data").mkdir(mode=0o700, parents=True)
    database = reject / "data/subscriptions.sqlite"
    shutil.copyfile(live, database)
    database.chmod(0o600)
    before = hashlib.sha256(database.read_bytes()).hexdigest()
    original = subprocess.Popen

    def launch(command: list[str], **kwargs: Any) -> subprocess.Popen[bytes]:
        return original([*command, "--editor-persistence"], **kwargs)

    with patch("desktop_ui_smoke.subprocess.Popen", launch):
        execute(
            binary,
            reject,
            [
                {
                    "when": {"busy": False, "ready": False},
                    "command": {"command": "connect"},
                },
                {
                    "when": {"error": "startup"},
                    "command": {"command": "delete_source", "id": 1},
                },
                {
                    "when": {"busy": False, "ready": False},
                    "command": {
                        "command": "routing_default",
                        "action": "direct",
                        "request_id": 7,
                        "expected_revision": 0,
                    },
                },
                {
                    "when": {
                        "routing_ack": {
                            "request_id": 7,
                            "saved_id": None,
                            "error": "startup",
                        }
                    },
                    "command": {"command": "shutdown"},
                },
            ],
            reject,
            "1",
        )
    assert hashlib.sha256(database.read_bytes()).hexdigest() == before
    control = output / "backup3"
    (control / "data").mkdir(mode=0o700, parents=True)
    shutil.copyfile(backup, control / "data/subscriptions.sqlite")
    (control / "data/subscriptions.sqlite").chmod(0o600)
    with patch("desktop_ui_smoke.subprocess.Popen", launch):
        execute(
            binary,
            control,
            [{"when": {"ready": True}, "command": {"command": "shutdown"}}],
            control,
            "1",
        )
    (output / "result.json").write_text(
        json.dumps(
            {
                "local_import_created_format4": True,
                "old_open_connect_delete_write_refused": True,
                "db4_bytes_unchanged_sha256": before,
                "verified_backup3_opened_by_old": True,
                "old_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "backup3_sha256": hashlib.sha256(backup.read_bytes()).hexdigest(),
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
