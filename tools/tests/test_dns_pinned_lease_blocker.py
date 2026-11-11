"""Регрессия: фоновая ошибка marker не становится успешным wire evidence."""

from __future__ import annotations

import os
import sys
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import dns_pinned_lease_blocker as fixture
import fakeip_epoch_gap


class MarkerErrors(unittest.TestCase):
    def test_thread_failure_is_fatal(self) -> None:
        listener = MagicMock()
        listener.accept.side_effect = OSError("синтетический отказ accept")
        markers = (
            fakeip_epoch_gap.markers
            if os.environ.get("YONIK_OLD_MARKER_ORACLE")
            else fixture.checked_markers
        )
        with patch("socket.socket", return_value=listener):
            with self.assertRaises(RuntimeError):
                with markers():
                    pass


if __name__ == "__main__":
    unittest.main()
