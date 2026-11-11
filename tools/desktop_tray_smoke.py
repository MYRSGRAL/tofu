"""Изолированные Qt tray regression: fallback и активная SOCKS-сессия."""

from __future__ import annotations

import argparse
import json
import os
import signal
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


def enable_subreaper() -> None:
    """Только изолированный negative supervisor усыновляет потомков при abort Qt."""
    import ctypes

    library = ctypes.CDLL(None, use_errno=True)
    library.prctl.argtypes = [ctypes.c_int] + [ctypes.c_ulong] * 4
    # Linux PR_SET_CHILD_SUBREAPER: локальный атрибут процесса, не host-настройка.
    if library.prctl(36, 1, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "PR_SET_CHILD_SUBREAPER")


def direct_children() -> set[int]:
    children: set[int] = set()
    for task in Path("/proc/self/task").iterdir():
        try:
            children.update(int(pid) for pid in (task / "children").read_text().split())
        except FileNotFoundError:
            continue  # Поток завершился между перечислением task и чтением children.
    return children


def reap_adopted(baseline: set[int]) -> None:
    """Убрать только новых собственных детей после reap Qt; peer исключён baseline."""
    deadline = time.monotonic() + 3
    while children := direct_children() - baseline:
        for pid in children:
            descriptor = os.pidfd_open(pid)
            try:
                signal.pidfd_send_signal(descriptor, signal.SIGKILL)
            finally:
                os.close(descriptor)
            os.waitpid(pid, os.WNOHANG)
        if time.monotonic() >= deadline:
            raise RuntimeError("Усыновлённые ядра не завершились за 3 секунды")
        time.sleep(0.02)


def wait_exit(process: subprocess.Popen, timeout: float) -> int:
    """Дождаться выхода без reap: PID лидера защищает идентификатор группы."""
    deadline = time.monotonic() + timeout
    while True:
        result = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        if result is not None:
            return (
                result.si_status
                if result.si_code == os.CLD_EXITED
                else -result.si_status
            )
        if time.monotonic() >= deadline:
            raise subprocess.TimeoutExpired(process.args, timeout)
        time.sleep(0.02)


def stop_group(process: subprocess.Popen) -> None:
    """Завершить всю собственную группу до освобождения PID её лидера."""
    os.killpg(process.pid, signal.SIGTERM)
    try:
        wait_exit(process, 3)
    except subprocess.TimeoutExpired:
        pass
    finally:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=3)


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


def assert_cleanup_retained(
    snapshot: dict, *, process_alive: bool, engine_alive: bool, listener_held: bool
) -> None:
    """Принять только видимую ошибку с подтверждёнными удержанными ресурсами."""
    if not (
        snapshot.get("error") == "cleanup_failed"
        and snapshot.get("closed") is False
        and snapshot.get("window_visible") is True
        and snapshot.get("can_stop") is True
        and snapshot.get("status") == "CleanupError"
        and process_alive
        and engine_alive
        and listener_held
    ):
        raise RuntimeError(
            "cleanup_failed не подтверждает удержание ресурсов и видимость диагностики"
        )


def active(
    binary: Path, engines: Path, output: Path, cleanup_negative: str | None = None
) -> None:
    """Настоящие Qt SNI, IPC relaunch и SOCKS traffic без пользовательской bus."""
    output.mkdir(parents=True, exist_ok=False)
    import dbus
    import dbus.service
    from dbus.mainloop.glib import DBusGMainLoop
    from gi.repository import GLib

    if cleanup_negative:
        enable_subreaper()
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus(private=True)
    bus.set_exit_on_disconnect(False)
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
    thread = threading.Thread(target=loop.run, daemon=True)
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
                {
                    "when": {"selected_profile": 1, "probe_custom": True},
                    "ui": {"action": "menuConnect"},
                },
                {
                    "when": {"can_stop": True, "status": "Limited"},
                    "ui": {"action": "close"},
                },
                {
                    "when": {"can_stop": True, "tray_available": True},
                    "ui_when": {"window": {"visible": True}},
                    "ui": {"action": "close"},
                },
                {
                    "when": {"can_stop": True},
                    "ui_when": {"window": {"visible": True}},
                    "ui": {"action": "menuDisconnect"},
                },
                {
                    "when": {"status": "Disconnected", "can_stop": False},
                    "ui": {"action": "menuConnect"},
                },
                {
                    "when": {"status": "Limited", "can_stop": True},
                    "ui": {"action": "quit"},
                },
            ]
            if cleanup_negative:
                steps = steps[:6] + [
                    {
                        "when": {"can_stop": True, "status": "Limited"},
                        "ui": {
                            "action": "quit"
                            if cleanup_negative == "quit"
                            else "menuDisconnect"
                        },
                    },
                    {
                        "when": {"error": "cleanup_failed", "closed": False},
                        "ui_when": {"window": {"visible": True}},
                        "ui": {"action": "close"},
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
                    baseline_children = direct_children() if cleanup_negative else set()
                    process = subprocess.Popen(
                        command
                        + ["--smoke-script", str(script), "--smoke-trace", str(trace)],
                        stderr=(output / "stderr.log").open("wb"),
                        start_new_session=True,
                        stdin=subprocess.PIPE
                        if cleanup_negative
                        else subprocess.DEVNULL,
                    )
                    core_fds: list[int] = []
                    release_sent = False
                    try:
                        deadline = time.monotonic() + 35
                        hidden = False
                        while time.monotonic() < deadline:
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
                                core_fds = [os.pidfd_open(int(pid)) for pid in children]
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
                                if cleanup_negative:
                                    negative_deadline = time.monotonic() + 5
                                    failure = None
                                    while time.monotonic() < negative_deadline:
                                        snapshots = [
                                            json.loads(line)
                                            for line in trace.read_text().splitlines()
                                        ]
                                        failures = [
                                            event
                                            for event in snapshots
                                            if event.get("error") == "cleanup_failed"
                                            and event.get("window_visible") is True
                                            and event.get("step") == len(steps)
                                        ]
                                        if failures:
                                            failure = failures[-1]
                                            break
                                        time.sleep(0.05)
                                    # Проверяем устойчивое no-exit, затем реальный listener и PID.
                                    time.sleep(0.5)
                                    alive = (
                                        os.waitid(
                                            os.P_PID,
                                            process.pid,
                                            os.WEXITED | os.WNOHANG | os.WNOWAIT,
                                        )
                                        is None
                                    )
                                    with socket.socket() as probe:
                                        held = (
                                            probe.connect_ex(("127.0.0.1", socks)) == 0
                                        )
                                    assert_cleanup_retained(
                                        failure or {},
                                        process_alive=alive,
                                        engine_alive=all(
                                            Path(f"/proc/{pid}").exists()
                                            for pid in children
                                        ),
                                        listener_held=held,
                                    )
                                    for filename in (
                                        "backend.jsonl",
                                        "diagnostics.json",
                                    ):
                                        (output / filename).write_bytes(
                                            (root / filename).read_bytes()
                                        )
                                    backend_events = [
                                        json.loads(line)
                                        for line in (root / "backend.jsonl")
                                        .read_text()
                                        .splitlines()
                                    ]
                                    keys = ("error", "closed", "status", "can_stop")
                                    if not any(
                                        all(
                                            event.get(key) == failure.get(key)
                                            for key in keys
                                        )
                                        for event in backend_events
                                    ):
                                        raise RuntimeError(
                                            "Qt snapshot не найден в actual DesktopHost callback"
                                        )
                                    diagnostics = json.loads(
                                        (root / "diagnostics.json").read_text()
                                    )
                                    if not any(
                                        error.get("message_key") == "cleanup_failed"
                                        for error in diagnostics.get("errors", [])
                                    ):
                                        raise RuntimeError(
                                            "Нет cleanup_failed в экспорте диагностики"
                                        )
                                    (output / "held.json").write_text(
                                        json.dumps(
                                            {
                                                "mode": cleanup_negative,
                                                "snapshot": failure,
                                                "process_alive": alive,
                                                "engine_pids": sorted(children),
                                                "listener_held": held,
                                            }
                                        )
                                    )
                                    assert process.stdin is not None
                                    process.stdin.write(b"r")
                                    process.stdin.flush()
                                    release_sent = True
                                    break
                                # Исчезновение владельца и новый владелец на private bus.
                                registrations_before = len(registered)
                                owner_before = bus.get_unique_name()
                                bus.release_name("org.kde.StatusNotifierWatcher")
                                watcher.remove_from_connection()
                                del name
                                bus.close()
                                time.sleep(0.3)
                                bus = dbus.SessionBus(private=True)
                                bus.set_exit_on_disconnect(False)
                                name = dbus.service.BusName(
                                    "org.kde.StatusNotifierWatcher", bus
                                )
                                if bus.get_unique_name() == owner_before:
                                    raise RuntimeError(
                                        "Watcher не сменил D-Bus владельца"
                                    )
                                watcher = Watcher(bus, "/StatusNotifierWatcher")
                                registration_deadline = time.monotonic() + 3
                                while (
                                    len(registered) <= registrations_before
                                    and time.monotonic() < registration_deadline
                                ):
                                    time.sleep(0.05)
                                if len(registered) <= registrations_before:
                                    raise RuntimeError("Нет повторной регистрации SNI")
                                subprocess.run(
                                    command,
                                    check=True,
                                    timeout=5,
                                    stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL,
                                )
                                break
                            time.sleep(0.05)
                        code = wait_exit(process, 8)
                        if cleanup_negative:
                            (output / "backend-final.jsonl").write_bytes(
                                (root / "backend.jsonl").read_bytes()
                            )
                        if (
                            not hidden
                            or (not cleanup_negative and len(registered) < 2)
                            or code != 0
                        ):
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
                                    "relaunch": not cleanup_negative,
                                    "watcher_restart": not cleanup_negative,
                                    "cleanup_negative": cleanup_negative,
                                    "menu_connect_disconnect": not cleanup_negative
                                    or cleanup_negative == "stop",
                                    "cleanup_failed_retained": bool(cleanup_negative),
                                    "stop_quit": True,
                                    "registered": len(registered),
                                }
                            )
                        )
                    finally:
                        # PID Qt ещё не reap: его непосредственные дети принадлежат стенду.
                        if not core_fds:
                            try:
                                tasks = Path(f"/proc/{process.pid}/task")
                                pending = {
                                    pid
                                    for task in tasks.iterdir()
                                    for pid in (task / "children").read_text().split()
                                }
                                for pid in pending:
                                    try:
                                        core_fds.append(os.pidfd_open(int(pid)))
                                    except ProcessLookupError:
                                        pass
                            except FileNotFoundError:
                                pass
                        if cleanup_negative and process.stdin is not None:
                            try:
                                if not release_sent:
                                    process.stdin.write(b"r")
                                    process.stdin.flush()
                            except BrokenPipeError:
                                pass  # Fixture уже завершён; ниже выполняется reap/kill.
                            finally:
                                try:
                                    process.stdin.close()
                                except BrokenPipeError:
                                    pass  # Повторный flush после закрытия fixture pipe.

                            try:
                                wait_exit(process, 5)
                            except subprocess.TimeoutExpired:
                                pass
                        try:
                            stop_group(process)
                        finally:
                            for descriptor in core_fds:
                                try:
                                    signal.pidfd_send_signal(descriptor, signal.SIGKILL)
                                except ProcessLookupError:
                                    pass
                                finally:
                                    os.close(descriptor)
                            if cleanup_negative:
                                reap_adopted(baseline_children)
            finally:
                server.shutdown()
                server.server_close()
                http.join(timeout=3)
    finally:
        try:
            watcher.remove_from_connection()
        finally:
            try:
                loop.quit()
            finally:
                thread.join(timeout=3)
                del name
                if thread.is_alive():
                    raise RuntimeError("GLib thread не завершился за 3 секунды")


def main() -> int:
    """Запустить проверку, не подключаясь к пользовательской D-Bus."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--engine-dir", type=Path)
    parser.add_argument("--cleanup-negative", choices=("stop", "quit"))
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
                    ["--cleanup-negative", args.cleanup_negative]
                    if args.cleanup_negative
                    else []
                ),
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
            args.cleanup_negative,
        )
    else:
        run(args.binary.resolve(strict=True), args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
