"""Actual старый reader против DB после новой миграции и её backup v3."""

import hashlib
import json
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from desktop_ui_smoke import execute

binary = Path(sys.argv[1]).resolve(strict=True)
source = Path(sys.argv[2]).resolve(strict=True)
root = Path(sys.argv[3]).resolve()
root.mkdir(mode=0o700)
shutil.copytree(source / "migrated-data", root / "data")
db = root / "data/subscriptions.sqlite"
with sqlite3.connect(db.as_uri() + "?mode=ro", uri=True) as c:
    assert c.execute("PRAGMA user_version").fetchone()[0] == 4
before = hashlib.sha256(db.read_bytes()).hexdigest()
original = subprocess.Popen


def launch(cmd: list[str], **kw: object) -> subprocess.Popen:
    return original([*cmd, "--editor-persistence"], **kw)


with patch("desktop_ui_smoke.subprocess.Popen", launch):
    out = root / "reject4"
    out.mkdir()
    execute(
        binary,
        root,
        [
            {
                "when": {"busy": False, "ready": False},
                "command": {"command": "connect"},
            },
            {
                "when": {"busy": False, "ready": False},
                "command": {
                    "command": "routing_default",
                    "action": "direct",
                    "request_id": 7,
                    "expected_revision": 2,
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
        out,
        "1",
    )
assert hashlib.sha256(db.read_bytes()).hexdigest() == before
control = root / "rollback3"
(control / "data").mkdir(parents=True, mode=0o700)
shutil.copyfile(source / "backup/v3.sqlite", control / "data/subscriptions.sqlite")
(control / "data/subscriptions.sqlite").chmod(0o600)
with patch("desktop_ui_smoke.subprocess.Popen", launch):
    out = control / "accept3"
    out.mkdir()
    execute(
        binary,
        control,
        [{"when": {"ready": True}, "command": {"command": "shutdown"}}],
        out,
        "1",
    )
(root / "result.json").write_text(
    json.dumps(
        {
            "new_transaction_created_format4": True,
            "old_open_connect_write_rejected": True,
            "sha256_unchanged": before,
            "rollback_backup3_opened_by_old": True,
        }
    )
    + "\n"
)
