"""Границы явного обновления журнала GUI."""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class LogsViewTests(unittest.TestCase):
    def test_clipboard_smoke_observes_real_clipboard(self) -> None:
        source = (ROOT / "crates/yonik-desktop/src/ui_smoke.h").read_text()
        self.assertIn('object.key() == QStringLiteral("clipboard")', source)

    def test_explicit_bounded_view(self) -> None:
        source = (ROOT / "crates/yonik-desktop/qml/LogsView.qml").read_text()
        self.assertNotIn("Timer", source)
        self.assertNotIn("onVisibleChanged", source)
        self.assertIn("logs_read", source)
        self.assertIn("TextEdit.PlainText", source)
        self.assertIn("page.export", source)
        self.assertIn("onResponseChanged", source)
        self.assertIn('root.pending !== ""', source)
        self.assertIn("root.response.request_id === root.pending", source)
        self.assertIn('objectName: "logsRows"', source)
        self.assertIn('objectName: "logsExportText"', source)


if __name__ == "__main__":
    unittest.main()
