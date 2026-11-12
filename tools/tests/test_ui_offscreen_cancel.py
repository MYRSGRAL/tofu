"""Отмена offscreen runner завершает собственный зависший fake child."""

import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class OffscreenCancellationTest(unittest.TestCase):
    def test_sigterm_reaps_candidate_waiting_for_ack(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yonik-offscreen-cancel-") as raw:
            directory = Path(raw)
            artifacts = directory / "artifacts"
            artifacts.mkdir()
            pid_file = directory / "candidate.pid"
            script = (
                f"#!{sys.executable}\n"
                "import os, signal, time\n"
                "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                "with open(os.environ['YONIK_FAKE_PID'], 'w') as out: out.write(str(os.getpid()))\n"
                "while True: time.sleep(1)\n"
            ).encode()
            binaries = {}
            for toolkit in ("qt", "gtk", "slint"):
                name = f"yonik-bench-{toolkit}"
                path = artifacts / name
                path.write_bytes(script)
                path.chmod(0o700)
                binaries[name] = {"sha256": hashlib.sha256(script).hexdigest()}
            (artifacts / "manifest.json").write_text(json.dumps({"binary": binaries}))
            env = os.environ | {"YONIK_FAKE_PID": str(pid_file)}
            candidate = None
            runner = subprocess.Popen(
                [
                    sys.executable,
                    str(ROOT / "bench/ui/check-offscreen.py"),
                    "--artifacts",
                    str(artifacts),
                    "--output",
                    str(directory / "output"),
                ],
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            try:
                deadline = time.monotonic() + 4
                while (
                    not pid_file.exists()
                    and runner.poll() is None
                    and time.monotonic() < deadline
                ):
                    time.sleep(0.01)
                self.assertTrue(pid_file.exists(), "Fake child должен запуститься")
                candidate = int(pid_file.read_text())
                runner.send_signal(signal.SIGTERM)
                self.assertNotEqual(runner.wait(timeout=5), 0)
                stat = Path(f"/proc/{candidate}/stat")
                alive = (
                    stat.exists()
                    and stat.read_text().split(")", 1)[1].split()[0] != "Z"
                )
                self.assertFalse(alive, "SIGTERM runner оставил candidate живым")
            finally:
                if runner.poll() is None:
                    runner.kill()
                    runner.wait(timeout=3)
                if candidate is not None:
                    try:
                        fd = os.pidfd_open(candidate)
                    except ProcessLookupError:
                        pass
                    else:
                        try:
                            signal.pidfd_send_signal(fd, signal.SIGKILL)
                        finally:
                            os.close(fd)
