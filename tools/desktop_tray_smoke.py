"""Изолированные Qt tray regression: fallback и активная SOCKS-сессия."""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import tempfile
import threading
import time
from http.server import ThreadingHTTPServer
from pathlib import Path

from desktop_smoke import Fixture, free_port, step
from desktop_ui_smoke import execute
from engine_compile import UUID
from engine_host import peer


def run(binary: Path, output: Path) -> None:
    """Проверить настоящий close и явный выход в отдельной session bus."""
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="yonik-tray-") as temporary:
        root = Path(temporary)
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        os.environ["XDG_RUNTIME_DIR"] = str(runtime)
        steps = [
            {
                "when": {"ready": True, "closed": False},
                "ui_when": {"window": {"visible": True}},
                "ui": {"action": "close"},
            },
            {
                "when": {"ready": True, "closed": False},
                "ui_when": {"window": {"visible": True}},
                "ui": {"action": "quit"},
                "capture_name": "after-close.png",
            },
        ]
        events = execute(binary, root, steps, output, "1")
        if any(event.get("tray_available") is not False for event in events):
            raise RuntimeError("Gate требует Qt без доступного трея")
        (output / "result.json").write_text(
            json.dumps(
                {
                    "steps": len(steps),
                    "events": len(events),
                    "closed": events[-1]["closed"],
                },
                indent=2,
            )
            + "\n"
        )


def active(binary: Path, engines: Path, output: Path) -> None:
    """Настоящие Qt SNI, IPC relaunch и SOCKS traffic без пользовательской bus."""
    output.mkdir(parents=True, exist_ok=False)
    import dbus
    import dbus.service
    from dbus.mainloop.glib import DBusGMainLoop
    from gi.repository import GLib

    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    name = dbus.service.BusName("org.kde.StatusNotifierWatcher", bus)
    registered: list[str] = []

    class Watcher(dbus.service.Object):
        @dbus.service.method("org.kde.StatusNotifierWatcher", in_signature="s")
        def RegisterStatusNotifierItem(self, service: str) -> None:
            registered.append(str(service))

        @dbus.service.method(
            "org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v"
        )
        def Get(self, interface: str, prop: str) -> object:
            return {
                "IsStatusNotifierHostRegistered": dbus.Boolean(True),
                "ProtocolVersion": dbus.Int32(0),
                "RegisteredStatusNotifierItems": dbus.Array(registered, signature="s"),
            }[prop]

    watcher = Watcher(bus, "/StatusNotifierWatcher")
    loop = GLib.MainLoop()
    thread = threading.Thread(target=loop.run)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="yt-") as temporary:
            root = Path(temporary)
            runtime = root / "runtime"
            runtime.mkdir(mode=0o700)
            os.environ.update(
                XDG_RUNTIME_DIR=str(runtime),
                QT_QPA_PLATFORM="offscreen",
                QT_QPA_PLATFORMTHEME="kde",
                QT_QUICK_BACKEND="software",
            )
            port, socks = free_port(), free_port()
            handler = type(
                "LocalFixture",
                (Fixture,),
                {"body": f"vless://{UUID}@127.0.0.1:{port}#Локальный".encode()},
            )
            server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
            http = threading.Thread(target=server.serve_forever)
            http.start()
            steps = [
                step(
                    {"ready": True},
                    "add_source",
                    name="Локальный",
                    url=f"http://127.0.0.1:{server.server_port}/",
                    local_exception=True,
                    route="direct",
                ),
                step({"sources_count": 1, "busy": False}, "refresh"),
                step(
                    {"profiles_count": 1, "busy": False},
                    "select_profile",
                    id=1,
                    revision=1,
                ),
                step(
                    {"selected_profile": 1},
                    "set_probe_target",
                    url="https://127.0.0.1:1/",
                    expected_status=204,
                ),
                step({"selected_profile": 1, "probe_custom": True}, "connect"),
                {
                    "when": {"can_stop": True, "status": "Limited"},
                    "ui": {"action": "close"},
                },
                {
                    "when": {"can_stop": True},
                    "ui_when": {"window": {"visible": True}},
                    "ui": {"action": "click", "object": "mainConnectButton"},
                },
                step({"status": "Disconnected", "can_stop": False}, "connect"),
                {
                    "when": {"status": "Limited", "can_stop": True},
                    "ui": {"action": "quit"},
                },
            ]
            script = root / "script.json"
            script.write_text(json.dumps({"steps": steps}, ensure_ascii=False))
            trace = output / "trace.jsonl"
            command = [
                str(binary),
                "--smoke-test",
                "--data-dir",
                str(root / "data"),
                "--engine-dir",
                str(engines),
                "--listen-port",
                str(socks),
            ]
            try:
                with peer(
                    engines / "xray-26.10.31/xray",
                    "xray",
                    root,
                    port=port,
                    target_ports=str(server.server_port),
                ):
                    process = subprocess.Popen(
                        command
                        + ["--smoke-script", str(script), "--smoke-trace", str(trace)],
                        stderr=(output / "stderr.log").open("wb"),
                    )
                    try:
                        deadline = time.monotonic() + 35
                        hidden = False
                        while time.monotonic() < deadline and process.poll() is None:
                            events = (
                                [
                                    json.loads(line)
                                    for line in trace.read_text().splitlines()
                                ]
                                if trace.exists()
                                else []
                            )
                            if any(
                                e.get("step") == 6
                                and e.get("window_visible") is False
                                and e.get("status") == "Limited"
                                for e in events
                            ):
                                hidden = True
                                children = {
                                    pid
                                    for task in Path(
                                        f"/proc/{process.pid}/task"
                                    ).iterdir()
                                    for pid in (task / "children").read_text().split()
                                }
                                if len(children) != 1:
                                    raise RuntimeError(
                                        "Ожидалось ровно одно активное ядро"
                                    )
                                subprocess.run(
                                    [
                                        "curl",
                                        "--silent",
                                        "--fail",
                                        "--max-time",
                                        "3",
                                        "--noproxy",
                                        "",
                                        "--socks5-hostname",
                                        f"127.0.0.1:{socks}",
                                        f"http://127.0.0.1:{server.server_port}/",
                                    ],
                                    check=True,
                                    stdout=subprocess.DEVNULL,
                                )
                                subprocess.run(
                                    command,
                                    check=True,
                                    timeout=5,
                                    stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL,
                                )
                                break
                            time.sleep(0.05)
                        code = process.wait(timeout=8)
                        if not hidden or len(registered) != 1 or code != 0:
                            raise RuntimeError(
                                f"Active tray: hidden={hidden}, registered={bool(registered)}, exit={code}"
                            )
                        events = [
                            json.loads(line) for line in trace.read_text().splitlines()
                        ]
                        if not events[-1].get("closed") or events[-1].get(
                            "step"
                        ) != len(steps):
                            raise RuntimeError(
                                "Quit не подтвердил завершение всех шагов"
                            )
                        if any(Path(f"/proc/{pid}").exists() for pid in children):
                            raise RuntimeError("После Quit осталось первое ядро")
                        with socket.socket() as probe:
                            if probe.connect_ex(("127.0.0.1", socks)) == 0:
                                raise RuntimeError(
                                    "После Quit SOCKS listener остался открыт"
                                )
                        (output / "result.json").write_text(
                            json.dumps(
                                {
                                    "hidden_traffic": True,
                                    "relaunch": True,
                                    "stop_quit": True,
                                    "registered": len(registered),
                                }
                            )
                        )
                    finally:
                        if process.poll() is None:
                            process.terminate()
                            try:
                                process.wait(timeout=3)
                            except subprocess.TimeoutExpired:
                                process.kill()
                                process.wait(timeout=3)
            finally:
                server.shutdown()
                server.server_close()
                http.join(timeout=3)
    finally:
        watcher.remove_from_connection()
        loop.quit()
        thread.join(timeout=3)
        del name


def main() -> int:
    """Запустить проверку, не подключаясь к пользовательской D-Bus."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--engine-dir", type=Path)
    parser.add_argument("--isolated", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.isolated:
        return subprocess.run(
            [
                "dbus-run-session",
                "--",
                "python",
                str(Path(__file__).resolve()),
                str(args.binary.resolve()),
                "--output",
                str(args.output.resolve()),
                "--isolated",
                *(
                    ["--engine-dir", str(args.engine_dir.resolve())]
                    if args.engine_dir
                    else []
                ),
            ],
            check=False,
        ).returncode
    if args.engine_dir:
        active(
            args.binary.resolve(strict=True),
            args.engine_dir.resolve(strict=True),
            args.output.resolve(),
        )
    else:
        run(args.binary.resolve(strict=True), args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
