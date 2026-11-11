"""Отрицательный tray gate отвергает выдуманный cleanup успех."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from desktop_tray_smoke import assert_cleanup_retained


class RetainedCleanupTests(unittest.TestCase):
    def test_held_cleanup_error_is_accepted(self) -> None:
        assert_cleanup_retained(
            {
                "error": "cleanup_failed",
                "closed": False,
                "window_visible": True,
                "can_stop": True,
                "status": "CleanupError",
            },
            process_alive=True,
            engine_alive=True,
            listener_held=True,
        )

    def test_false_release_or_hidden_diagnostics_are_rejected(self) -> None:
        baseline = {
            "error": "cleanup_failed",
            "closed": False,
            "window_visible": True,
            "can_stop": True,
            "status": "CleanupError",
        }
        for key, value in [
            ("closed", True),
            ("window_visible", False),
            ("error", None),
            ("status", "Disconnected"),
            ("can_stop", False),
        ]:
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                assert_cleanup_retained(
                    baseline | {key: value},
                    process_alive=True,
                    engine_alive=True,
                    listener_held=True,
                )
        for resource in ["process_alive", "engine_alive", "listener_held"]:
            resources = dict(process_alive=True, engine_alive=True, listener_held=True)
            resources[resource] = False
            with self.subTest(resource=resource), self.assertRaises(RuntimeError):
                assert_cleanup_retained(baseline, **resources)


class SupervisorTests(unittest.TestCase):
    def test_aborted_parent_with_separate_group_child_is_reaped(self) -> None:
        import subprocess
        import textwrap

        code = textwrap.dedent(r"""
            import os, socket, subprocess, sys, tempfile, time
            from pathlib import Path
            from desktop_tray_smoke import enable_subreaper, direct_children, reap_adopted
            enable_subreaper()
            baseline = direct_children()
            with tempfile.TemporaryDirectory() as directory:
                address = Path(directory) / 'port'
                child = "import socket,time,pathlib; s=socket.socket(); s.bind(('127.0.0.1',0)); s.listen(); pathlib.Path(%r).write_text(str(s.getsockname()[1])); time.sleep(30)" % str(address)
                parent = "import subprocess,sys,os,time,pathlib; subprocess.Popen([sys.executable,'-c',%r],start_new_session=True); p=pathlib.Path(%r);\nwhile not p.exists(): time.sleep(0.01)\nos._exit(9)" % (child, str(address))
                process = subprocess.Popen([sys.executable, '-c', parent])
                try:
                    assert process.wait(timeout=3) == 9
                    port = int(address.read_text())
                    with socket.create_connection(('127.0.0.1', port), timeout=1): pass
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=3)
                    reap_adopted(baseline)
                with socket.socket() as probe:
                    assert probe.connect_ex(('127.0.0.1', port)) != 0
                assert direct_children() == baseline
        """)
        subprocess.run(
            [sys.executable, "-c", code],
            cwd=Path(__file__).resolve().parents[1],
            check=True,
            timeout=10,
        )
