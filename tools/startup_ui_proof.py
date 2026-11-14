"""Собственные offscreen Qt/private SNI fixtures: UI RU/EN, hidden, loss, duplicate.

Interface witness не является physical Plasma/login proof. XDG пишет только в
собственный fixture config; ephemeral GUI не доказывает normal autoconnect.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
import threading
import time
from pathlib import Path
from typing import Any


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--case",
        action="append",
        choices=[
            "settings",
            "hint-no-tray",
            "hint-duplicate",
            "hint-tray-loss",
            "hint-blocked",
        ],
    )
    args = parser.parse_args()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    import dbus
    import dbus.service
    from dbus.mainloop.glib import DBusGMainLoop
    from gi.repository import GLib

    DBusGMainLoop(set_as_default=True)
    with tempfile.TemporaryDirectory(
        prefix="yonik-startup-ui-", dir="/tmp"
    ) as temporary:
        runtime = Path(temporary)
        conf = runtime / "bus.conf"
        conf.write_text(
            f'<busconfig><type>session</type><listen>unix:path={runtime}/bus</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
        )
        bus_process = subprocess.Popen(
            [
                "dbus-daemon",
                "--nofork",
                "--nopidfile",
                "--print-address=1",
                "--config-file",
                str(conf),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert bus_process.stdout
        address = bus_process.stdout.readline().strip()
        if not address.startswith(f"unix:path={runtime}/bus"):
            bus_process.terminate()
            bus_process.wait(timeout=3)
            raise RuntimeError("Private bus не стартовала; fallback запрещён")
        bus = dbus.bus.BusConnection(address)
        loop = GLib.MainLoop()
        thread = threading.Thread(target=loop.run, daemon=True)
        thread.start()
        registered: list[str] = []

        class Watcher(dbus.service.Object):
            @dbus.service.method(
                "org.kde.StatusNotifierWatcher",
                in_signature="s",
                sender_keyword="sender",
            )
            def RegisterStatusNotifierItem(
                self, service: str, sender: str = ""
            ) -> None:
                with (args.output / "registration.jsonl").open("a") as log:
                    log.write(
                        json.dumps({"service": str(service), "sender": str(sender)})
                        + "\n"
                    )
                registered.append(
                    sender + service
                    if service.startswith("/")
                    else service + "/StatusNotifierItem"
                )

            @dbus.service.method(
                "org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v"
            )
            def Get(self, interface: str, prop: str) -> Any:
                with (args.output / "properties.jsonl").open("a") as log:
                    log.write(
                        json.dumps({"property": str(prop), "items": registered}) + "\n"
                    )
                return {
                    "IsStatusNotifierHostRegistered": dbus.Boolean(True),
                    "ProtocolVersion": dbus.Int32(0),
                    "RegisteredStatusNotifierItems": dbus.Array(
                        registered, signature="s"
                    ),
                }[prop]

        results = []
        name = None
        watcher = None
        try:
            for case in args.case or [
                "settings",
                "hint-no-tray",
                "hint-duplicate",
                "hint-tray-loss",
                "hint-blocked",
            ]:
                root = args.output / case
                root.mkdir(mode=0o700)
                (root / "config").mkdir(mode=0o700)
                registered.clear()
                name = None
                watcher = None
                if case != "hint-no-tray":
                    name = dbus.service.BusName("org.kde.StatusNotifierWatcher", bus)
                    watcher = Watcher(bus, "/StatusNotifierWatcher")
                env = dict(
                    os.environ,
                    HOME=str(root),
                    XDG_RUNTIME_DIR=str(runtime),
                    XDG_DATA_HOME=str(root / "share"),
                    XDG_CONFIG_HOME=str(root / "config"),
                    DBUS_SESSION_BUS_ADDRESS=address,
                    DBUS_SYSTEM_BUS_ADDRESS=f"unix:path={runtime}/no-system",
                    QT_QPA_PLATFORM="offscreen",
                    QT_QPA_PLATFORMTHEME="kde",
                    QT_QUICK_BACKEND="software",
                )
                env.pop("DISPLAY", None)
                env.pop("WAYLAND_DISPLAY", None)
                trace = root / "trace.jsonl"
                script = root / "script.json"
                if case == "settings":
                    steps: list[dict[str, Any]] = [
                        {
                            "when": {"ready": True},
                            "ui": {"action": "resize", "width": 380, "height": 600},
                        },
                        {"ui": {"action": "click", "object": "settingsButton"}},
                        {"ui": {"action": "click", "object": "settingsScheduleButton"}},
                        {
                            "ui_when": {
                                "window": {"startupPending": ""},
                                "autostartIntoTray": {"checked": False},
                                "startupAutoconnect": {"checked": False},
                                "confirmStartupTarget": {"enabled": False},
                            },
                            "ui": {
                                "action": "key",
                                "object": "homeButton",
                                "key": "Tab",
                            },
                            "capture_name": "startup-ru.png",
                        },
                    ]
                    for target in [
                        "startupAutoconnect",
                        "startupAutoconnect",
                        "autostartIntoTray",
                        "autostartIntoTray",
                    ]:
                        steps.append(
                            {
                                "ui_when": {"window": {"startupPending": ""}},
                                "ui": {"action": "click", "object": target},
                            }
                        )
                    steps.extend(
                        [
                            {
                                "ui_when": {"window": {"startupPending": ""}},
                                "command": {
                                    "command": "set_language",
                                    "language": "en",
                                },
                            },
                            {
                                "ui_when": {
                                    "window": {"language": "en", "startupPending": ""}
                                },
                                "ui": {
                                    "action": "key",
                                    "object": "homeButton",
                                    "key": "Tab",
                                },
                                "capture_name": "startup-en.png",
                            },
                            {"ui": {"action": "quit"}},
                        ]
                    )
                elif case == "hint-blocked":
                    data = root / "data"
                    data.mkdir(mode=0o700)
                    preferences = data / "preferences.json"
                    preferences.write_text(
                        json.dumps(
                            {
                                "version": 3,
                                "autoconnect": True,
                                "engine": "xray",
                                "port": None,
                                "probe_url": "https://example.invalid/",
                                "probe_status": 204,
                                "language": "ru",
                            }
                        )
                    )
                    preferences.chmod(0o600)
                    steps = [
                        {
                            "when": {
                                "ready": True,
                                "startup": {
                                    "start_hidden": True,
                                    "revision": "0",
                                    "autostart_into_tray": False,
                                    "autoconnect": True,
                                    "target": None,
                                    "request_id": None,
                                    "error": "startup_binding",
                                    "save": "idle",
                                    "attempt": "blocked",
                                    "xdg": "unchanged",
                                },
                            },
                            "ui_when": {"window": {"visible": True}},
                            "ui": {"action": "quit"},
                        }
                    ]
                elif case == "hint-no-tray":
                    steps = [
                        {
                            "when": {"ready": True, "tray_available": False},
                            "ui_when": {"window": {"visible": True}},
                            "ui": {"action": "quit"},
                        }
                    ]
                else:
                    steps = [
                        {
                            "when": {"ready": True},
                            "ui_when": {"window": {"visible": False}},
                            "ui": {"action": "resize", "width": 380, "height": 600},
                        },
                        {
                            "ui_when": {"window": {"visible": True}},
                            "ui": {"action": "quit"},
                        },
                    ]
                script.write_text(json.dumps({"steps": steps}))
                command = [
                    str(args.binary),
                    "--smoke-test",
                    "--data-dir",
                    str(root / "data"),
                    "--smoke-script",
                    str(script),
                    "--smoke-trace",
                    str(trace),
                ]
                if case == "settings":
                    command += ["--smoke-screenshot", str(root / "ui.png")]
                else:
                    command += ["--start-hidden"]
                with (root / "stderr.log").open("w") as stderr:
                    process = subprocess.Popen(
                        command, env=env, stdout=subprocess.DEVNULL, stderr=stderr
                    )
                    try:
                        deadline = time.monotonic() + 25
                        intervened = False
                        frames: list[dict[str, Any]] = []
                        while process.poll() is None and time.monotonic() < deadline:
                            frames = (
                                [
                                    json.loads(line)
                                    for line in trace.read_text().splitlines()
                                    if line.endswith("}")
                                ]
                                if trace.exists()
                                else []
                            )
                            if (
                                case in ["hint-duplicate", "hint-tray-loss"]
                                and not intervened
                                and any(
                                    frame["window_visible"] is False for frame in frames
                                )
                            ):
                                if case == "hint-duplicate":
                                    duplicate = subprocess.run(
                                        [
                                            str(args.binary),
                                            "--data-dir",
                                            str(root / "data"),
                                            "--start-hidden",
                                        ],
                                        env=env,
                                        stdout=subprocess.DEVNULL,
                                        stderr=stderr,
                                        timeout=5,
                                    )
                                    assert duplicate.returncode == 0
                                else:
                                    bus.release_name("org.kde.StatusNotifierWatcher")
                                intervened = True
                            time.sleep(0.03)
                        if process.poll() is None:
                            raise TimeoutError(f"Qt fixture deadline: {case}")
                        assert process.returncode == 0, case
                        if case in ["hint-duplicate", "hint-tray-loss"]:
                            assert intervened, case
                        results.append(
                            {
                                "case": case,
                                "passed": True,
                                "registered_items": len(registered),
                            }
                        )
                    finally:
                        if process.poll() is None:
                            process.terminate()
                            try:
                                process.wait(timeout=5)
                            except subprocess.TimeoutExpired:
                                process.kill()
                                process.wait()
                if watcher:
                    watcher.remove_from_connection()
                    watcher = None
                if name:
                    bus.release_name("org.kde.StatusNotifierWatcher")
                    name = None
        finally:
            if watcher:
                watcher.remove_from_connection()
            if name:
                bus.release_name("org.kde.StatusNotifierWatcher")
                name = None
            loop.quit()
            thread.join(timeout=3)
            bus.close()
            bus_process.terminate()
            bus_process.wait(timeout=5)
        (args.output / "result.json").write_text(json.dumps(results, indent=2))
        print(json.dumps(results))


if __name__ == "__main__":
    main()
