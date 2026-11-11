"""ACK журнала должен проходить через настоящий partial-snapshot handler Main.qml."""

import json
import shutil
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(shutil.which("node"), "Требуется Node для выполнения функции QML")
class LogsPartialSnapshotTest(unittest.TestCase):
    def apply(self, partial: dict[str, object]) -> dict[str, object]:
        text = (ROOT / "crates/yonik-desktop/qml/Main.qml").read_text()
        start = text.index("    function readSnapshot() {")
        end = text.index("    function statusText(", start)
        script = """
const vm = require('node:vm');
const input = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));
const context = {backend: {snapshot_json: JSON.stringify(input.partial)},
    state: {logs: null, ready: false}, commandContext: '', parseError: '',
    completeRouting: () => {}, tr: (ru, en) => en};
vm.runInNewContext('(' + input.handler + ')()', context, {timeout: 1000});
process.stdout.write(JSON.stringify(context.state));
"""
        result = subprocess.run(
            ["node", "-e", script],
            input=json.dumps({"partial": partial, "handler": text[start:end]}),
            text=True,
            capture_output=True,
            check=True,
            timeout=5,
        )
        return json.loads(result.stdout)

    def test_startup_ack_is_delivered_without_requiring_catalog(self) -> None:
        page = {"request_id": "7", "status": "error", "events": [], "export": None}
        state = self.apply({"error": "startup", "logs": page})
        self.assertEqual(state["logs"], page)
        self.assertFalse(state["ready"])
        self.assertEqual(state["connection_error"], "startup")

    def test_unrelated_partial_error_preserves_existing_logs(self) -> None:
        state = self.apply({"error": "busy"})
        self.assertIsNone(state["logs"])
        self.assertEqual(state["error"], "busy")
