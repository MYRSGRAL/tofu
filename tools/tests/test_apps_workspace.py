"""Регрессия очистки файлов G-APP после прерывания внешнего процесса."""

from __future__ import annotations

import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class WorkspaceTests(unittest.TestCase):
    def test_workspace_removed_after_interrupt(self) -> None:
        parent = """
import sys
sys.path.insert(0, 'tools')
import engine_apps as apps
original = apps.run_sandbox
def fake_worker(command, timeout):
    work = command[command.index('--work-dir') + 1]
    script = "from pathlib import Path; import sys,time; p=Path(sys.argv[1])/'clients-proof'; p.mkdir(); (p/'config.json').write_text('{}'); time.sleep(60)"
    original([sys.executable, '-c', script, work], timeout=timeout)
apps.run_sandbox = fake_worker
apps.install_signal_handlers()
sys.argv = ['engine_apps', 'xray', sys.executable, '--work-dir', sys.argv[1]]
apps.main()
"""
        for event in (signal.SIGINT, signal.SIGTERM):
            with (
                self.subTest(event=event),
                tempfile.TemporaryDirectory(prefix="yonik-apps-test-") as directory,
            ):
                root = Path(directory)
                process = subprocess.Popen(
                    [sys.executable, "-c", parent, directory],
                    cwd=ROOT,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                try:
                    deadline = time.monotonic() + 5
                    while (
                        not list(root.rglob("config.json"))
                        and process.poll() is None
                        and time.monotonic() < deadline
                    ):
                        time.sleep(0.02)
                    self.assertTrue(
                        list(root.rglob("config.json")),
                        "Worker не создал тестовый каталог",
                    )
                    process.send_signal(event)
                    process.wait(timeout=5)
                    self.assertEqual(
                        list(root.iterdir()),
                        [],
                        "После прерывания остались файлы worker",
                    )
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
