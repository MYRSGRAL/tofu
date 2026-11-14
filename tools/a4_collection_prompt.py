"""Настоящий CreateCollection/Prompt с одной живой private D-Bus connection."""

from __future__ import annotations

import json
from pathlib import Path

import dbus
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib


def main() -> None:
    out = Path(__file__).resolve().parents[1] / "docs/evidence/a4-mtree4/gui-run"
    state = json.loads((out / "active.json").read_text())
    DBusGMainLoop(set_as_default=True)
    connection = dbus.bus.BusConnection(
        "unix:path=" + state["root"] + "/home/fixture/runtime/bus"
    )
    service = dbus.Interface(
        connection.get_object("org.freedesktop.secrets", "/org/freedesktop/secrets"),
        "org.freedesktop.Secret.Service",
    )
    collection, prompt = service.CreateCollection(
        dbus.Dictionary(
            {"org.freedesktop.Secret.Collection.Label": dbus.String("A4 disposable")},
            signature="sv",
        ),
        "default",
    )
    loop = GLib.MainLoop()

    def completed(dismissed: bool, result: object) -> None:
        (out / "collection-prompt-result.json").write_text(
            json.dumps({"dismissed": bool(dismissed), "result": str(result)}) + "\n"
        )
        loop.quit()

    obj = connection.get_object("org.freedesktop.secrets", str(prompt))
    obj.connect_to_signal(
        "Completed", completed, dbus_interface="org.freedesktop.Secret.Prompt"
    )
    dbus.Interface(obj, "org.freedesktop.Secret.Prompt").Prompt("")
    (out / "collection-prompt-start.json").write_text(
        json.dumps({"collection": str(collection), "prompt": str(prompt)}) + "\n"
    )
    loop.run()


if __name__ == "__main__":
    main()
