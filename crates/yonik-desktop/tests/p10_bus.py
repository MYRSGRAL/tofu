"""Изолированные NM/UPower/PPD; stdin управляет только fixture, stdout — ack."""
import os
import sys
import threading
from gi.repository import Gio, GLib

bus = Gio.DBusConnection.new_for_address_sync(
    os.environ["DBUS_SYSTEM_BUS_ADDRESS"],
    Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
    None, None,
)
names = ["org.freedesktop.NetworkManager", "org.freedesktop.UPower", "org.freedesktop.UPower.PowerProfiles"]
paths = ["/org/freedesktop/NetworkManager", "/org/freedesktop/UPower", "/org/freedesktop/UPower/PowerProfiles"]
values = [{"State": GLib.Variant("u", 70), "Metered": GLib.Variant("u", 0)}, {"OnBattery": GLib.Variant("b", False)}, {"ActiveProfile": GLib.Variant("s", "balanced")}]

def call(method: str, name: str) -> GLib.Variant:
    return bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", method, GLib.Variant("(su)", (name, 0)) if method == "RequestName" else GLib.Variant("(s)", (name,)), None, Gio.DBusCallFlags.NONE, 2000, None)

for index, (name, path) in enumerate(zip(names, paths)):
    props = "".join(f'<property name="{key}" type="{value.get_type_string()}" access="read"/>' for key, value in values[index].items())
    info = Gio.DBusNodeInfo.new_for_xml(f'<node><interface name="{name}">{props}</interface></node>')
    bus.register_object(path, info.interfaces[0], None, lambda c, s, p, i, prop, idx=index: values[idx][prop], None)
    call("RequestName", name)
print("ready", flush=True)

def command(line: str) -> bool:
    cmd = line.strip()
    if cmd == "clients":
        result = bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames", None, None, Gio.DBusCallFlags.NONE, 2000, None)
        print(sum(name.startswith(":") for name in result.unpack()[0]), flush=True)
        return False
    if cmd in ("0", "1", "2"):
        values[0]["Metered"] = GLib.Variant("u", int(cmd))
    elif cmd in ("balanced", "power-saver"):
        values[2]["ActiveProfile"] = GLib.Variant("s", cmd)
    elif cmd in ("loss", "acquire"):
        call("ReleaseName" if cmd == "loss" else "RequestName", names[0])
    for name, path in zip(names, paths):
        bus.emit_signal(None, path, "org.freedesktop.DBus.Properties", "PropertiesChanged", GLib.Variant("(sa{sv}as)", (name, {}, list(values[names.index(name)]))))
    bus.flush_sync(None)
    print("ack", flush=True)
    return False

def reader() -> None:
    for line in sys.stdin:
        GLib.idle_add(command, line)
    os._exit(0)

threading.Thread(target=reader, daemon=True).start()
GLib.MainLoop().run()
