"""Отказ preflight не оставляет запущенный GLib loop."""

import importlib.util
import sys
import tempfile
import types
import unittest
from collections.abc import Callable
from pathlib import Path
from unittest import mock

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
SPEC = importlib.util.spec_from_file_location(
    "tray_cleanup_subject", TOOLS / "desktop_tray_smoke.py"
)
assert SPEC and SPEC.loader
tray = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(tray)


class TrayCleanupTests(unittest.TestCase):
    def test_existing_output_does_not_leave_started_loop(self) -> None:
        self.check_cleanup(False)

    def test_remove_failure_still_stops_loop(self) -> None:
        self.check_cleanup(True)

    def check_cleanup(self, remove_failure: bool) -> None:
        service = types.ModuleType("dbus.service")

        class Object:
            def __init__(self, *args: object) -> None:
                pass

        Object.remove_from_connection = mock.Mock(
            side_effect=RuntimeError("remove failed") if remove_failure else None
        )
        service.Object = Object
        service.BusName = mock.Mock()

        def method_decorator(*args: object, **kwargs: object) -> Callable:
            def identity(method: Callable) -> Callable:
                return method

            return identity

        service.method = method_decorator
        dbus = types.ModuleType("dbus")
        dbus.service = service
        dbus.SessionBus = mock.Mock()
        glib_mainloop = types.ModuleType("dbus.mainloop.glib")
        glib_mainloop.DBusGMainLoop = mock.Mock()
        repository = types.ModuleType("gi.repository")
        repository.GLib = mock.Mock()
        loop = repository.GLib.MainLoop.return_value
        modules = {
            "dbus": dbus,
            "dbus.service": service,
            "dbus.mainloop": types.ModuleType("dbus.mainloop"),
            "dbus.mainloop.glib": glib_mainloop,
            "gi": types.ModuleType("gi"),
            "gi.repository": repository,
        }
        with (
            tempfile.TemporaryDirectory() as directory,
            mock.patch.dict(sys.modules, modules),
            mock.patch.object(tray.threading, "Thread") as thread,
        ):
            thread.return_value.is_alive.return_value = False
            output = Path(directory) / "new" if remove_failure else Path(directory)
            with mock.patch.object(
                tray.tempfile,
                "TemporaryDirectory",
                side_effect=RuntimeError("fixture failed"),
            ):
                with self.assertRaises((FileExistsError, RuntimeError)):
                    tray.active(Path("unused-binary"), Path("unused-engines"), output)
            self.assertTrue(
                not thread.return_value.start.called
                or (loop.quit.called and thread.return_value.join.called),
                "ошибка до try/finally оставила GLib thread без cleanup",
            )


class ProcessLifetimeTests(unittest.TestCase):
    def test_exited_leader_remains_reserved_until_group_cleanup(self) -> None:
        import os
        import subprocess

        process = subprocess.Popen(
            [sys.executable, "-c", "pass"], start_new_session=True
        )
        try:
            self.assertEqual(tray.wait_exit(process, 3), 0)
            self.assertIsNone(process.returncode)
            self.assertTrue(Path(f"/proc/{process.pid}").exists())
        finally:
            tray.stop_group(process)
        self.assertEqual(process.returncode, 0)
        with self.assertRaises(ProcessLookupError):
            os.killpg(process.pid, 0)

    def test_cleanup_kills_held_listener_after_leader_exits(self) -> None:
        import socket
        import subprocess
        import time

        with tempfile.TemporaryDirectory() as directory:
            address = Path(directory) / "port"
            child = (
                "import signal,socket,time,pathlib; "
                "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                "s=socket.socket(); s.bind(('127.0.0.1',0)); s.listen(); "
                f"pathlib.Path({str(address)!r}).write_text(str(s.getsockname()[1])); "
                "time.sleep(30)"
            )
            leader = (
                "import subprocess,sys; "
                f"subprocess.Popen([sys.executable,'-c',{child!r}])"
            )
            process = subprocess.Popen(
                [sys.executable, "-c", leader], start_new_session=True
            )
            try:
                tray.wait_exit(process, 3)
                deadline = time.monotonic() + 3
                while not address.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                port = int(address.read_text())
                with socket.create_connection(("127.0.0.1", port), timeout=1):
                    pass
            finally:
                tray.stop_group(process)
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                with socket.socket() as probe:
                    if probe.connect_ex(("127.0.0.1", port)) != 0:
                        break
                time.sleep(0.02)
            else:
                self.fail("Дочерний listener пережил cleanup группы")
