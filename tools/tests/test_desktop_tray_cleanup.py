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
        service = types.ModuleType("dbus.service")

        class Object:
            def __init__(self, *args: object) -> None:
                pass

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
            with self.assertRaises(FileExistsError):
                tray.active(
                    Path("unused-binary"), Path("unused-engines"), Path(directory)
                )
            self.assertTrue(
                not thread.return_value.start.called
                or (loop.quit.called and thread.return_value.join.called),
                "ошибка до try/finally оставила GLib thread без cleanup",
            )
