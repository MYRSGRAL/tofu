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
    @classmethod
    def setUpClass(cls) -> None:
        result = subprocess.run(
            [
                "unshare",
                "--map-current-user",
                "--pid",
                "--fork",
                "--kill-child=KILL",
                "--mount-proc",
                "--",
                "true",
            ],
            capture_output=True,
            text=True,
            timeout=5,
        )
        if result.returncode != 0:
            raise unittest.SkipTest(
                "PID namespace fixture недоступна: " + result.stderr.strip()
            )

    def test_timeout_closes_listener_in_separate_core_group(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yonik-cleanup-") as temporary:
            marker = Path(temporary) / "child.json"
            with self.assertRaises(subprocess.TimeoutExpired):
                FIXTURE.run_probe(
                    [sys.executable, "-c", PARENT, CHILD, str(marker)],
                    os.environ.copy(),
                    1,
                )
            _, port = json.loads(marker.read_text())
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", port))

    def test_parent_exit_closes_inherited_child_pipes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yonik-exit-cleanup-") as temporary:
            marker = Path(temporary) / "child.json"
            parent = PARENT.replace(
                "time.sleep(120)",
                """
from pathlib import Path
while not Path(sys.argv[2]).exists():
    time.sleep(0.01)
""",
            )
            result = FIXTURE.run_probe(
                [sys.executable, "-c", parent, CHILD, str(marker)],
                os.environ.copy(),
                3,
            )
            self.assertEqual(result.returncode, 0)
            _, port = json.loads(marker.read_text())
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", port))

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
