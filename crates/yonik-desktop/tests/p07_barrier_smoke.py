"""Actual старый Qt binary: version3 control и version4 open/Connect/write refusal."""

import hashlib
import json
import sqlite3
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from desktop_ui_smoke import execute

binary = Path(sys.argv[1]).resolve(strict=True)
root = Path(sys.argv[2]).resolve()
root.mkdir(parents=True, exist_ok=True)
subprocess.run(
    [str(binary), "--data-dir", str(root / "data"), "--editor-seed-v2"], check=True
)
db = root / "data/subscriptions.sqlite"
original = subprocess.Popen


def launch(cmd: list[str], **kw: object) -> subprocess.Popen:
    return original([*cmd, "--editor-persistence"], **kw)


with patch("desktop_ui_smoke.subprocess.Popen", launch):
    out = root / "control3"
    out.mkdir(exist_ok=True)
    execute(
        binary,
        root,
        [{"when": {"ready": True}, "command": {"command": "shutdown"}}],
        out,
        "1",
    )
with sqlite3.connect(db) as c:
    assert c.execute("PRAGMA user_version").fetchone()[0] == 3
    c.execute("PRAGMA user_version=4")


def state() -> tuple[int, str]:
    with sqlite3.connect(db.as_uri() + "?mode=ro", uri=True) as c:
        return c.execute("SELECT revision,hex(payload) FROM routing_draft").fetchone()


before = state()
digest = hashlib.sha256(db.read_bytes()).hexdigest()
with patch("desktop_ui_smoke.subprocess.Popen", launch):
    out = root / "reject4"
    out.mkdir(exist_ok=True)
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
                    "expected_revision": 1,
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
assert state() == before
assert hashlib.sha256(db.read_bytes()).hexdigest() == digest
(root / "result.json").write_text(
    json.dumps(
        {
            "old_binary": str(binary),
            "v3_open": True,
            "v4_open_connect_write_rejected": True,
            "revision_ciphertext_unchanged": True,
            "database_sha256": digest,
        }
    )
    + "\n"
)
