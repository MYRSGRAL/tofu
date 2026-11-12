"""FR19: настоящая private Plasma, без подмены StatusNotifierWatcher.

Запускать в own user/network namespace с поднятым lo. D-Bus activation отключена;
HOME/runtime и system bus никогда не наследуются от рабочей Plasma пользователя.
"""

from __future__ import annotations

import argparse
import json
import os
import select
import signal
import socket
import subprocess
import tempfile
import threading
import time
from collections.abc import Callable
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import TypeVar

from desktop_smoke import Fixture, free_port, step
from desktop_tray_smoke import stop_group
from engine_compile import UUID
from engine_host import peer

T = TypeVar("T")


def wait_for(predicate: Callable[[], T], message: str, timeout: float = 12) -> T:
    """Ограниченное ожидание изменяемого runtime-состояния."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.1)
    raise RuntimeError(message)


def assert_continuity(before: list[str], after: list[str], core_fd: int) -> None:
    """Отклонить исчезнувшую регистрацию и завершение того же pidfd ядра."""
    if not set(before).intersection(after):
        raise RuntimeError("SNI исходного приложения не зарегистрирован повторно")
    if select.select([core_fd], [], [], 0)[0]:
        raise RuntimeError("Исходная core session завершилась")


def run(binary: Path, engines: Path, output: Path, engine: str, language: str) -> None:
    import dbus

    if {name for _, name in socket.if_nameindex()} != {"lo"}:
        raise RuntimeError("Нужна отдельная network namespace только с lo")
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="fr19-plasma-") as temporary:
        root = Path(temporary)
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        config = root / "bus.conf"
        config.write_text(
            "<busconfig><type>session</type><listen>unix:tmpdir="
            + str(runtime)
            + '</listen><auth>EXTERNAL</auth><policy context="default">'
            '<allow send_destination="*"/><allow receive_sender="*"/>'
            '<allow own="*"/></policy></busconfig>'
        )
        environment = {
            **os.environ,
            "HOME": str(root),
            "XDG_RUNTIME_DIR": str(runtime),
            "XDG_CONFIG_HOME": str(root / "config"),
            "XDG_DATA_HOME": str(root / "data"),
            "XDG_CACHE_HOME": str(root / "cache"),
            "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent/fr19-system-bus",
            "QT_QPA_PLATFORM": "wayland",
            "QT_QPA_PLATFORMTHEME": "kde",
            "QT_QUICK_BACKEND": "software",
            "LIBGL_ALWAYS_SOFTWARE": "1",
            "KWIN_COMPOSE": "Q",
            "WAYLAND_DISPLAY": "fr19-private",
            "XDG_CURRENT_DESKTOP": "KDE",
        }
        for key in (
            "DISPLAY",
            "SESSION_MANAGER",
            "DBUS_STARTER_ADDRESS",
            "DBUS_STARTER_BUS_TYPE",
        ):
            environment.pop(key, None)
        processes: list[subprocess.Popen] = []
        logs = []

        def launch(name: str, command: list[str]) -> subprocess.Popen:
            log = (output / (name + ".log")).open("wb")
            logs.append(log)
            process = subprocess.Popen(
                command,
                env=environment,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            (output / (name + ".command.json")).write_text(json.dumps(command))
            processes.append(process)
            return process

        launch(
            "bus",
            [
                "dbus-daemon",
                "--nofork",
                "--print-address=1",
                "--config-file=" + str(config),
            ],
        )
        try:
            address = wait_for(
                lambda: (output / "bus.log").read_text().splitlines(),
                "private bus не запущена",
            )[0]
            environment["DBUS_SESSION_BUS_ADDRESS"] = address
            bus = dbus.bus.BusConnection(address)
            launch(
                "kwin",
                [
                    "kwin_wayland",
                    "--virtual",
                    "--socket",
                    "fr19-private",
                    "--no-lockscreen",
                    "--no-global-shortcuts",
                    "--no-kactivities",
                ],
            )
            wait_for(
                lambda: (runtime / "fr19-private").exists(), "KWin socket отсутствует"
            )
            watcher_name = "org.kde.StatusNotifierWatcher"

            def desktop(suffix: str) -> tuple[subprocess.Popen, subprocess.Popen]:
                kded = launch("kded" + suffix, ["kded6"])
                plasma = launch("plasma" + suffix, ["plasmashell"])
                wait_for(
                    lambda: (
                        bus.name_has_owner(watcher_name)
                        and bus.name_has_owner("org.kde.plasmashell")
                    ),
                    "private Plasma/watcher отсутствует",
                )
                return kded, plasma

            kded, plasma = desktop("-before")
            owner_before = str(bus.get_name_owner(watcher_name))
            port, socks = free_port(), free_port()
            handler = type(
                "LoopbackFixture",
                (Fixture,),
                {"body": f"vless://{UUID}@127.0.0.1:{port}#Локальный".encode()},
            )
            server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
            thread = threading.Thread(target=server.serve_forever)
            thread.start()
            try:
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
                    step({"selected_profile": 1}, "set_engine", engine=engine),
                    step(
                        {"selected_profile": 1, "engine": engine},
                        "set_probe_target",
                        url="https://127.0.0.1:1/",
                        expected_status=204,
                    ),
                    {"when": {"probe_custom": True}, "ui": {"action": "menuConnect"}},
                    {
                        "when": {
                            "status": "Limited",
                            "can_stop": True,
                            "tray_available": True,
                        },
                        "ui": {"action": "close"},
                    },
                ]
                steps[5:5] = [
                    {
                        "when": {"probe_custom": True, "selected_name": "Локальный"},
                        "ui": {"action": "click", "object": "serverPickerButton"},
                    },
                    {
                        "ui_when": {"window": {"page": 1, "selectionConfirmed": True}},
                        "ui": {"action": "click", "object": "homeButton"},
                    },
                    {
                        "ui_when": {"window": {"page": 0}},
                        "ui": {"action": "click", "object": "settingsButton"},
                    },
                    {
                        "ui_when": {"window": {"page": 2}},
                        "capture_name": "settings.png",
                        "ui": {"action": "click", "object": "homeButton"},
                    },
                ]
                if language == "en":
                    steps[9:9] = [
                        {
                            "when": {"probe_custom": True},
                            "ui": {"action": "click", "object": "settingsButton"},
                        },
                        {
                            "ui_when": {"window": {"page": 2}},
                            "ui": {
                                "action": "click",
                                "object": "settingsAppearanceButton",
                            },
                        },
                        {
                            "ui": {
                                "action": "key",
                                "object": "languageChoice",
                                "key": "Down",
                            }
                        },
                        {
                            "ui": {
                                "action": "key",
                                "object": "languageChoice",
                                "key": "Enter",
                            }
                        },
                        {
                            "ui_when": {"window": {"language": "en"}},
                            "ui": {
                                "action": "key",
                                "object": "languageChoice",
                                "key": "Escape",
                            },
                        },
                    ]
                steps[-1]["when"].update(
                    selected_name="Локальный",
                    active_name="Локальный",
                    active_engine=engine,
                )
                steps[-1]["ui_when"] = {
                    "window": {"language": language, "selectionConfirmed": True}
                }
                steps[-1]["capture_name"] = "active-before.png"
                hidden_step = len(steps)
                steps.append(
                    {
                        "when": {
                            "status": "Limited",
                            "can_stop": True,
                            "tray_available": True,
                        },
                        "ui_when": {"window": {"visible": True}},
                        "capture_name": "active-recovered.png",
                        "ui": {"action": "close"},
                    }
                )
                script = root / "script.json"
                script.write_text(json.dumps({"steps": steps}, ensure_ascii=False))
                (output / "script.json").write_bytes(script.read_bytes())
                trace = output / "trace.jsonl"
                with peer(
                    engines / "xray-26.10.31/xray",
                    "xray",
                    root,
                    port=port,
                    target_ports=str(server.server_port),
                ) as vless_peer:
                    app = launch(
                        "application",
                        [
                            str(binary),
                            "--smoke-test",
                            "--data-dir",
                            str(root / "app"),
                            "--engine-dir",
                            str(engines),
                            "--listen-port",
                            str(socks),
                            "--smoke-script",
                            str(script),
                            "--smoke-trace",
                            str(trace),
                            "--smoke-screenshot",
                            str(output / "window.png"),
                        ],
                    )

                    def snapshots() -> list[dict]:
                        return (
                            [
                                json.loads(line)
                                for line in trace.read_text().splitlines()
                            ]
                            if trace.exists()
                            else []
                        )

                    wait_for(
                        lambda: any(
                            e.get("step") == hidden_step
                            and e.get("window_visible") is False
                            and e.get("status") == "Limited"
                            for e in snapshots()
                        ),
                        "Нет hidden Limited session",
                        25,
                    )
                    children = {
                        int(pid)
                        for task in Path(f"/proc/{app.pid}/task").iterdir()
                        for pid in (task / "children").read_text().split()
                    }
                    if len(children) != 1:
                        raise RuntimeError(f"Ожидалось одно ядро: {children}")
                    core = children.pop()
                    core_fd = os.pidfd_open(core)
                    try:
                        traffic_proofs: list[dict[str, object]] = []

                        def traffic() -> None:
                            url = f"http://127.0.0.1:{server.server_port}/"
                            direct = ["curl", "--silent", "--fail", "--max-time", "3"]
                            proxied = [
                                *direct,
                                "--noproxy",
                                "",
                                "--socks5-hostname",
                                f"127.0.0.1:{socks}",
                                url,
                            ]

                            def expect_body(command: list[str]) -> None:
                                response = subprocess.run(
                                    command, check=True, capture_output=True, timeout=5
                                )
                                if response.stdout != handler.body:
                                    raise RuntimeError("HTTP fixture вернул чужое тело")

                            expect_body(proxied)
                            # Приостановлен только собственный controlled peer.
                            # Direct bypass продолжил бы работать и провалил этот gate.
                            vless_peer.send_signal(signal.SIGSTOP)
                            try:
                                wait_for(
                                    lambda: (
                                        Path(f"/proc/{vless_peer.pid}/stat")
                                        .read_text()
                                        .rsplit(")", 1)[1]
                                        .split()[0]
                                        == "T"
                                    ),
                                    "Собственный VLESS peer не приостановлен",
                                )
                                expect_body([*direct, "--noproxy", "*", url])
                                denied = subprocess.run(
                                    proxied, capture_output=True, timeout=5
                                )
                                if denied.returncode == 0:
                                    raise RuntimeError(
                                        "HTTP обошёл недоступный VLESS peer"
                                    )
                            finally:
                                vless_peer.send_signal(signal.SIGCONT)
                            expect_body(proxied)
                            traffic_proofs.append(
                                {
                                    "direct_fixture_available": True,
                                    "peer_stopped_proxy_exit": denied.returncode,
                                    "resumed_peer_exact_body": True,
                                }
                            )
                            (output / "peer-traffic.json").write_text(
                                json.dumps(traffic_proofs, indent=2)
                            )

                        def registrations() -> list[str]:
                            return list(
                                bus.get_object(
                                    watcher_name, "/StatusNotifierWatcher"
                                ).Get(
                                    watcher_name,
                                    "RegisteredStatusNotifierItems",
                                    dbus_interface="org.freedesktop.DBus.Properties",
                                )
                            )

                        before = registrations()
                        if not before:
                            raise RuntimeError(
                                "Настоящий watcher не зарегистрировал SNI"
                            )
                        traffic()
                        stop_group(plasma)
                        stop_group(kded)
                        if bus.name_has_owner(watcher_name):
                            raise RuntimeError("Watcher не исчез: negative control")
                        kded, plasma = desktop("-after")
                        owner_after = str(bus.get_name_owner(watcher_name))
                        after = wait_for(
                            registrations,
                            "Нет повторной регистрации после actual Plasma restart",
                        )
                        if owner_before == owner_after:
                            raise RuntimeError("Владелец watcher не сменился")
                        assert_continuity(before, after, core_fd)
                        wait_for(
                            lambda: any(
                                e.get("step") == len(steps)
                                and e.get("window_visible") is False
                                for e in snapshots()
                            ),
                            "После restart окно не скрылось",
                        )
                        traffic()
                        item = next(str(s) for s in after if str(s) in before)
                        service, path = item.split("/", 1)
                        obj = bus.get_object(service, "/" + path)
                        menu_path = obj.Get(
                            "org.kde.StatusNotifierItem",
                            "Menu",
                            dbus_interface="org.freedesktop.DBus.Properties",
                        )
                        menu = bus.get_object(service, str(menu_path))
                        layout = menu.GetLayout(
                            0, -1, [], dbus_interface="com.canonical.dbusmenu"
                        )
                        (output / "menu.txt").write_text(str(layout))
                        actions = {
                            str(node[1].get("label", "")): int(node[0])
                            for node in layout[1][2]
                        }
                        labels = (
                            (
                                "Соединение ограничено",
                                "Открыть yonik",
                                "Подключить",
                                "Отключить",
                                "Выйти и отключиться",
                            )
                            if language == "ru"
                            else (
                                "Connection limited",
                                "Open yonik",
                                "Connect",
                                "Disconnect",
                                "Quit and Disconnect",
                            )
                        )
                        for label in labels:
                            if label not in actions:
                                raise RuntimeError(f"Нет точной RU label: {label}")
                        menu.Event(
                            actions[labels[3]],
                            "clicked",
                            dbus.Int32(0),
                            dbus.UInt32(0),
                            dbus_interface="com.canonical.dbusmenu",
                        )
                        wait_for(
                            lambda: (
                                snapshots()
                                and snapshots()[-1].get("status") == "Disconnected"
                                and not snapshots()[-1].get("can_stop")
                            ),
                            "Stop callback не завершён",
                        )
                        if Path(f"/proc/{core}").exists():
                            raise RuntimeError("Stop не reaped исходное ядро")
                        menu.Event(
                            actions[labels[2]],
                            "clicked",
                            dbus.Int32(0),
                            dbus.UInt32(0),
                            dbus_interface="com.canonical.dbusmenu",
                        )
                        wait_for(
                            lambda: (
                                snapshots()
                                and snapshots()[-1].get("status") == "Limited"
                            ),
                            "SNI Connect не восстановил session",
                        )
                        traffic()
                        quit_children = {
                            int(pid)
                            for task in Path(f"/proc/{app.pid}/task").iterdir()
                            for pid in (task / "children").read_text().split()
                        }
                        menu.Event(
                            actions[labels[4]],
                            "clicked",
                            dbus.Int32(0),
                            dbus.UInt32(0),
                            dbus_interface="com.canonical.dbusmenu",
                        )
                        # Выход инициирован реальным SNI callback после завершения smoke steps.
                        app.wait(timeout=8)
                        if app.returncode != 0 or not snapshots()[-1].get("closed"):
                            raise RuntimeError("Quit не завершился успешно")
                        if list(runtime.glob("tofu-*")):
                            raise RuntimeError("IPC socket/lock остался после Quit")
                        with socket.socket() as probe:
                            if probe.connect_ex(("127.0.0.1", socks)) == 0:
                                raise RuntimeError("SOCKS listener остался после Quit")
                        if any(Path(f"/proc/{pid}").exists() for pid in quit_children):
                            raise RuntimeError("Quit core не reaped")
                        (output / "result.json").write_text(
                            json.dumps(
                                {
                                    "engine": engine,
                                    "core": core,
                                    "owner_before": owner_before,
                                    "owner_after": owner_after,
                                    "registrations_before": before,
                                    "registrations_after": after,
                                    "app_exit": app.returncode,
                                    "actual_plasma_restart": True,
                                },
                                indent=2,
                            )
                        )
                    finally:
                        os.close(core_fd)
            finally:
                server.shutdown()
                server.server_close()
                thread.join()
            bus.close()
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    stop_group(process)
            for log in logs:
                log.close()
            cleanup = [
                {
                    "pid": process.pid,
                    "exit": process.poll(),
                    "proc_present": Path(f"/proc/{process.pid}").exists(),
                }
                for process in processes
            ]
            (output / "cleanup.json").write_text(json.dumps(cleanup, indent=2))
            if any(item["exit"] is None or item["proc_present"] for item in cleanup):
                raise RuntimeError("Собственные Plasma/Qt/bus процессы не reaped")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--engine-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--language", choices=("ru", "en"), default="ru")
    parser.add_argument("--engine", choices=("xray", "sing-box"), required=True)
    args = parser.parse_args()
    run(
        args.binary.resolve(),
        args.engine_dir.resolve(),
        args.output.resolve(),
        args.engine,
        args.language,
    )


if __name__ == "__main__":
    main()
