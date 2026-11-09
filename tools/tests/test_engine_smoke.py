"""Регрессия: прерывание диагностического процесса не оставляет его потомков."""

from __future__ import annotations

import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def running(pid: int) -> bool:
    try:
        status = Path(f"/proc/{pid}/stat").read_text()
        return status.rsplit(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


class CleanupTests(unittest.TestCase):
    def test_interrupt_and_termination_stop_entire_process_group(self) -> None:
        for event, startup in [
            (signal.SIGINT, False),
            (signal.SIGTERM, False),
            (None, False),
            (signal.SIGINT, True),
            (signal.SIGTERM, True),
        ]:
            with (
                self.subTest(event=event, startup=startup),
                tempfile.TemporaryDirectory(prefix="yonik-signal-") as directory,
            ):
                ready = Path(directory) / "ready"
                child = (
                    "import os,subprocess,sys,time; from pathlib import Path; "
                    "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); "
                    "tmp=Path(sys.argv[1]+'.tmp'); tmp.write_text(f'{os.getpid()} {p.pid}'); "
                    "tmp.replace(sys.argv[1]); time.sleep(60)"
                )
                parent = """
import os, sys, time
from pathlib import Path
from tools import engine_smoke as smoke
if int(sys.argv[4]):
    original = smoke.subprocess.Popen
    def launch(*args, **kwargs):
        process = original(*args, **kwargs)
        deadline = time.monotonic() + 5
        while not Path(sys.argv[2]).exists():
            if time.monotonic() >= deadline:
                process.kill()
                process.wait()
                raise TimeoutError()
            time.sleep(0.01)
        os.kill(os.getpid(), int(sys.argv[4]))
        return process
    smoke.subprocess.Popen = launch
smoke.install_signal_handlers()
smoke.run_sandbox([sys.executable, '-c', sys.argv[1], sys.argv[2]], timeout=float(sys.argv[3]))
"""
                process = subprocess.Popen(
                    [
                        sys.executable,
                        "-c",
                        parent,
                        child,
                        str(ready),
                        "1" if event is None else "30",
                        str(int(event)) if startup else "0",
                    ],
                    cwd=ROOT,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                children: list[int] = []
                try:
                    deadline = time.monotonic() + 5
                    while (
                        not ready.exists()
                        and process.poll() is None
                        and time.monotonic() < deadline
                    ):
                        time.sleep(0.02)
                    self.assertTrue(
                        ready.exists(), "Диагностический процесс не запустил sandbox"
                    )
                    children = [int(pid) for pid in ready.read_text().split()]
                    self.assertEqual(len(children), 2)
                    if event is not None and not startup:
                        process.send_signal(event)
                    process.wait(timeout=5)
                    deadline = time.monotonic() + 3
                    while (
                        any(running(pid) for pid in children)
                        and time.monotonic() < deadline
                    ):
                        time.sleep(0.02)
                    self.assertFalse(
                        any(running(pid) for pid in children),
                        "Sandbox оставил активных потомков",
                    )
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
                    for pid in children:
                        if running(pid):
                            os.kill(pid, signal.SIGKILL)


if __name__ == "__main__":
    unittest.main()
