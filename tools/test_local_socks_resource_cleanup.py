"""Аварийный timeout должен закрывать core с отдельной process group."""

from __future__ import annotations

import importlib.util
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "resource_fixture",
    Path(__file__).resolve().parents[1]
    / "crates/yonik-desktop/tests/local_socks_resource.py",
)
assert SPEC is not None and SPEC.loader is not None
FIXTURE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXTURE)
CHILD = """
import json, os, socket, sys, time
from pathlib import Path
with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    Path(sys.argv[1]).write_text(json.dumps([os.getpid(), listener.getsockname()[1]]))
    time.sleep(120)
"""
PARENT = """
import subprocess, sys, time
subprocess.Popen([sys.executable, "-c", sys.argv[1], sys.argv[2]], process_group=0)
time.sleep(120)
"""


class CleanupTests(unittest.TestCase):
    def test_timeout_closes_listener_in_separate_core_group(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yonik-cleanup-") as temporary:
            marker = Path(temporary) / "child.json"
            with self.assertRaises(subprocess.TimeoutExpired):
                FIXTURE.run_probe(
                    [sys.executable, "-c", PARENT, CHILD, str(marker)],
                    os.environ.copy(),
                    1,
                )
            pid, port = json.loads(marker.read_text())
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", port))
            stat = Path(f"/proc/{pid}/stat")
            if stat.exists():
                self.assertEqual(stat.read_text().rsplit(") ", 1)[1].split()[0], "Z")

    def test_old_parent_only_cleanup_is_detected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yonik-cleanup-control-") as temporary:
            marker = Path(temporary) / "child.json"
            try:
                with self.assertRaises(subprocess.TimeoutExpired):
                    subprocess.run(
                        [sys.executable, "-c", PARENT, CHILD, str(marker)],
                        timeout=1,
                        start_new_session=True,
                        stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL,
                    )
                pid, port = json.loads(marker.read_text())
                handle = os.pidfd_open(pid)
                try:
                    with socket.socket() as listener, self.assertRaises(OSError):
                        listener.bind(("127.0.0.1", port))
                finally:
                    signal.pidfd_send_signal(handle, signal.SIGKILL)
                    os.close(handle)
            finally:
                if marker.exists():
                    pid, _ = json.loads(marker.read_text())
                    try:
                        handle = os.pidfd_open(pid)
                    except ProcessLookupError:
                        pass
                    else:
                        try:
                            signal.pidfd_send_signal(handle, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        finally:
                            os.close(handle)


if __name__ == "__main__":
    unittest.main()
