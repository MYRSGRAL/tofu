"""Нативный GUI smoke: только вымышленные URI, свой HTTP fixture и PID namespace."""

from __future__ import annotations

import argparse
import json
import logging
import os
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from unittest.mock import patch

from engine_compile import UUID
from engine_host import HttpsHandler, https_server, peer
from engine_smoke import links, run_sandbox

LOG = logging.getLogger("desktop-smoke")


class Fixture(BaseHTTPRequestHandler):
    body: bytes = b""
    refresh_gate: ProbeGate | None = None

    def do_GET(self) -> None:
        if self.refresh_gate is not None and self.refresh_gate.received.is_set():
            self.refresh_gate.refresh_requested.set()
            if (
                not self.refresh_gate.completed.wait(timeout=5)
                or self.refresh_gate.terminal not in ("sent", "closed")
                or self.refresh_gate.timed_out.is_set()
            ):
                self.send_error(503)
                return
            self.refresh_gate.refresh_passed.set()
        self.send_response(200)
        self.send_header("Content-Length", str(len(self.body)))
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.end_headers()
        self.wfile.write(self.body)

    def log_message(self, _format: str, *args: object) -> None:
        # Данные запроса не входят в диагностический вывод.
        return


class ProbeGate:
    def __init__(self) -> None:
        self.received = threading.Event()
        self.release = threading.Event()
        self.completed = threading.Event()
        self.timed_out = threading.Event()
        self.refresh_requested = threading.Event()
        self.refresh_passed = threading.Event()
        self.terminal: str | None = None


class HeldHttpsHandler(HttpsHandler):
    gate: ProbeGate

    def do_GET(self) -> None:
        self.server.requested.set()
        self.gate.received.set()
        try:
            if not self.gate.release.wait(timeout=35):
                self.gate.timed_out.set()
                return
            # Дольше старого screenshot dwell: Stop обязан ждать handshake,
            # а не случайно выигрывать гонку за счёт таймера 500 мс.
            time.sleep(0.8)
            self.send_response(204)
            self.send_header("Content-Length", "0")
            try:
                self.end_headers()
                self.wfile.flush()
                self.gate.terminal = "sent"
            except (BrokenPipeError, ConnectionResetError, ssl.SSLEOFError):
                self.gate.terminal = "closed"
        finally:
            self.gate.completed.set()


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def step(
    when: dict[str, object], command: str, **arguments: object
) -> dict[str, object]:
    if command in ("connect", "stop") and not arguments:
        expected: dict[str, object] = {
            "window": {"page": 0},
            "mainConnectButton": {"enabled": True},
        }
        statuses = {
            "Connected": "Подключено",
            "Limited": "Соединение ограничено",
            "Checking": "Проверка доступа…",
        }
        if label := statuses.get(str(when.get("status"))):
            expected["mainStatusLabel"] = {"text": label}
        return {
            "when": when,
            "ui_when": expected,
            "ui": {"action": "click", "object": "mainConnectButton"},
        }
    return {"when": when, "command": {"command": command, **arguments}}


TRACE_LIMIT = 500_000
NETWORK_HOST_NAMESPACE: int | None = None


def guard_network() -> None:
    if (
        NETWORK_HOST_NAMESPACE is None
        or os.stat("/proc/self/ns/net").st_ino == NETWORK_HOST_NAMESPACE
        or links() != {"lo"}
    ):
        raise RuntimeError("Нужен новый пустой network namespace")


def read_trace(path: Path) -> list[dict[str, Any]]:
    """Читаем только ограниченный metadata trace, включая ещё не завершённый файл."""
    try:
        with path.open("rb") as stream:
            payload = stream.read(TRACE_LIMIT + 1)
    except FileNotFoundError:
        return []
    if len(payload) > TRACE_LIMIT:
        raise RuntimeError("Trace превысил лимит")
    return [json.loads(line) for line in payload.split(b"\n")[:-1] if line]


def core_processes(engines: Path) -> dict[int, str]:
    """PID и starttime различают тот же процесс и повторное использование PID."""
    processes = {}
    for directory in Path("/proc").iterdir():
        if not directory.name.isdecimal():
            continue
        try:
            executable = os.readlink(directory / "exe")
            if executable != "/memfd:tofu-engine (deleted)" and not Path(
                executable
            ).is_relative_to(engines):
                continue
            fields = (directory / "stat").read_text().rsplit(")", 1)[1].split()
            if fields[0] != "Z":
                processes[int(directory.name)] = fields[19]
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return processes


def invalidate_network(
    trace: Path,
    engines: Path,
    baseline: dict[int, str],
    done: threading.Event,
    proof: list[bool],
    failures: list[Exception],
    gate: ProbeGate | None = None,
) -> None:
    route_added = False
    current: dict[int, str] = {}
    try:
        deadline = time.monotonic() + 35
        while not done.wait(0.01):
            if time.monotonic() >= deadline:
                raise TimeoutError("Не получено доказательство network invalidation")
            events = read_trace(trace)
            if not route_added:
                if gate is None:
                    ready = any(
                        event.get("status") == "Connected"
                        and event.get("probe_present") is True
                        and event.get("tcp_status") == "reachable"
                        and not event.get("tcp_busy")
                        for event in events
                    )
                else:
                    ready = gate.received.is_set() and any(
                        event.get("status") == "Checking"
                        and event.get("probe_present") is False
                        for event in events
                    )
                    if any(event.get("status") == "Connected" for event in events):
                        raise RuntimeError(
                            "Удерживаемая HTTPS-проверка завершилась раньше события"
                        )
                if not ready:
                    continue
                current = {
                    pid: started
                    for pid, started in core_processes(engines).items()
                    if baseline.get(pid) != started
                }
                if len(current) != 1:
                    raise RuntimeError(
                        "Не найден единственный процесс клиентского ядра"
                    )
                guard_network()
                subprocess.run(
                    ["ip", "route", "add", "198.18.254.0/24", "dev", "lo"],
                    check=True,
                    capture_output=True,
                    timeout=5,
                )
                route_added = True
            if any(
                event.get("status") == "Limited"
                and event.get("error") == "network_changed"
                for event in events
            ):
                alive = core_processes(engines)
                if any(alive.get(pid) != started for pid, started in current.items()):
                    raise RuntimeError("При инвалидации завершилось клиентское ядро")
                if gate is not None:
                    gate.release.set()
                    if not gate.completed.wait(timeout=2) or gate.timed_out.is_set():
                        raise RuntimeError(
                            "Удерживаемый HTTPS handler не отправил поздний ответ"
                        )
                    if gate.terminal not in ("sent", "closed"):
                        raise RuntimeError(
                            "Нет доказанного terminal исхода поздней HTTPS-попытки"
                        )
                    alive = core_processes(engines)
                    if any(
                        alive.get(pid) != started for pid, started in current.items()
                    ):
                        raise RuntimeError(
                            "Клиентское ядро завершилось до поздней HTTPS-попытки"
                        )
                proof.append(True)
                return
    except Exception as error:
        failures.append(error)
    finally:
        if route_added:
            try:
                guard_network()
                subprocess.run(
                    ["ip", "route", "del", "198.18.254.0/24", "dev", "lo"],
                    check=True,
                    capture_output=True,
                    timeout=5,
                )
            except (OSError, RuntimeError, subprocess.SubprocessError) as error:
                failures.append(error)


def run_case(
    binary: Path,
    engines: Path | None,
    output: Path,
    name: str,
    partial: bool,
    engine: str | None,
    namespace: bool,
    *,
    profile: str | None = None,
    probe_url: str | None = None,
    expected_status: int = 204,
    expected_state: str = "Limited",
    observed_status: int | None = None,
    ca: Path | None = None,
    tcp_status: str | None = None,
    network_invalidate: bool = False,
    gate: ProbeGate | None = None,
) -> dict[str, object]:
    if network_invalidate and (namespace or engines is None):
        raise ValueError("Network fixture требует внешний изолированный worker")
    if network_invalidate:
        guard_network()
    endpoint = free_port()
    bodies = [
        f"trojan://invented-secret@127.0.0.1:{endpoint}#Москва • TLS",
        "socks5://invented:password@192.0.2.1:1080#<b>Plain text</b>",
    ]
    if profile is not None:
        bodies = [profile]
    if partial:
        bodies.append("unknown://not-supported")
    handler = type(
        "SubscriptionFixture",
        (Fixture,),
        {"body": "\n".join(bodies).encode(), "refresh_gate": gate},
    )
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    trace = output / f"{name}.jsonl"
    screenshot = output / f"{name}.png"
    try:
        with tempfile.TemporaryDirectory(prefix=f"tofu-gui-{name}-") as temporary:
            root = Path(temporary)
            steps: list[dict[str, Any]] = [
                step(
                    {"ready": True, "sources_count": 0},
                    "add_source",
                    name="Тестовая подписка",
                    url=f"http://127.0.0.1:{server.server_port}/subscription",
                    local_exception=True,
                    route="direct",
                ),
                step({"sources_count": 1, "busy": False}, "refresh"),
            ]
            if partial:
                approve = step({"pending": True, "busy": False}, "approve_refresh")
                approve["capture"] = True
                steps.append(approve)
            steps.append(
                step(
                    {
                        "profiles_count": len(bodies) - int(partial),
                        "busy": False,
                        "pending": False,
                    },
                    "select_profile",
                    id=1,
                    revision=1,
                )
            )
            if tcp_status is not None:
                steps.append(step({"selected_profile": 1}, "check_tcp"))
                finish_when = {"tcp_status": tcp_status, "tcp_busy": False}
                if tcp_status == "reachable":
                    finish_when.update(tcp_samples_count=3, tcp_samples_requested=3)
                finish = step(finish_when, "shutdown")
                finish["capture"] = True
                steps.append(finish)
            elif engine is not None:
                if engines is None:
                    raise ValueError("Для проверки ядра нужен --engine-dir")
                if engine != "xray":
                    steps.append(
                        step({"selected_profile": 1}, "set_engine", engine=engine)
                    )
                if probe_url is not None:
                    steps.append(
                        step(
                            {"engine": engine, "selected_profile": 1},
                            "set_probe_target",
                            url=probe_url,
                            expected_status=expected_status,
                        )
                    )
                connect_when: dict[str, object] = {
                    "engine": engine,
                    "selected_profile": 1,
                }
                if probe_url is not None:
                    connect_when.update(
                        probe_custom=True, probe_expected_status=expected_status
                    )
                steps.append(step(connect_when, "connect"))
                stop_when: dict[str, object] = {
                    "status": expected_state,
                    "can_stop": True,
                }
                if probe_url is not None:
                    if gate is None:
                        steps.append(step(stop_when, "check_tcp"))
                    stop_when = {
                        **stop_when,
                        "tcp_status": "reachable",
                        "tcp_busy": False,
                    }
                    if network_invalidate:
                        stop_when = {
                            "status": "Limited",
                            "error": "network_changed",
                            "can_stop": True,
                            "probe_result": None,
                            "tcp_result": None,
                            "tcp_busy": False,
                        }
                        if gate is not None:
                            checkpoint = step(stop_when, "refresh")
                            checkpoint["capture"] = True
                            steps.append(checkpoint)
                            # Этот revision приходит через backend после HTTP-
                            # барьера, освобождаемого только terminal HTTPS.
                            stop_when = {**stop_when, "revision": 2, "busy": False}
                            stop_when.pop("error")
                    if not network_invalidate:
                        if observed_status is not None:
                            steps.extend(
                                [
                                    {
                                        "when": stop_when,
                                        "ui": {
                                            "action": "click",
                                            "object": "settingsButton",
                                        },
                                    },
                                    {
                                        "when": stop_when,
                                        "ui_when": {
                                            "probeResultLabel": {
                                                "visible": True,
                                                "text": {
                                                    "contains": f"HTTP {observed_status}"
                                                },
                                            }
                                        },
                                        "ui": {
                                            "action": "click",
                                            "object": "probeResultLabel",
                                        },
                                    },
                                    {
                                        "when": stop_when,
                                        "capture_name": f"{name}-diagnostics.png",
                                        "ui": {
                                            "action": "click",
                                            "object": "homeButton",
                                        },
                                    },
                                ]
                            )
                        frozen = step(
                            stop_when,
                            "set_probe_target",
                            url="https://mutation.invalid/never-request?secret=not-sent",
                            expected_status=201,
                        )
                        frozen["capture"] = True
                        steps.append(frozen)
                        stop_when = {
                            **stop_when,
                            "error": "busy",
                            "probe_expected_status": expected_status,
                        }
                steps.extend(
                    [
                        {
                            **step(stop_when, "stop"),
                            "capture": (probe_url is None or network_invalidate)
                            and gate is None,
                        },
                        step({"status": "Disconnected", "can_stop": False}, "shutdown"),
                    ]
                )
            else:
                steps.append(
                    step({"selected_profile": 1}, "favorite", id=1, value=True)
                )
                finish = step({"selected_profile": 1, "busy": False}, "shutdown")
                finish["capture"] = not partial
                steps.append(finish)
            script = root / "script.json"
            script.write_text(json.dumps({"steps": steps}, ensure_ascii=False))
            command = (
                [
                    "unshare",
                    "--user",
                    "--map-current-user",
                    "--pid",
                    "--fork",
                    "--kill-child",
                    "--mount",
                    "--mount-proc",
                ]
                if namespace
                else []
            ) + [
                str(binary),
                "--smoke-test",
                "--data-dir",
                str(root / "data"),
                "--listen-port",
                str(free_port()),
                "--smoke-script",
                str(script),
                "--smoke-trace",
                str(trace),
                "--smoke-screenshot",
                str(screenshot),
            ]
            if engines is not None:
                command.extend(["--engine-dir", str(engines)])
            done = threading.Event()
            proof: list[bool] = []
            failures: list[Exception] = []
            watcher = None
            if network_invalidate:
                assert engines is not None
                trace.write_bytes(b"")
                watcher = threading.Thread(
                    target=invalidate_network,
                    args=(
                        trace,
                        engines,
                        core_processes(engines),
                        done,
                        proof,
                        failures,
                        gate,
                    ),
                    daemon=True,
                )
                watcher.start()
            try:
                if ca is None:
                    run_sandbox(command, timeout=45)
                else:
                    with patch.dict(os.environ, {"SSL_CERT_FILE": str(ca)}):
                        run_sandbox(command, timeout=45)
            finally:
                done.set()
                if gate is not None:
                    gate.release.set()
                if watcher is not None:
                    watcher.join(timeout=6)
                    if watcher.is_alive():
                        raise RuntimeError("Поток network fixture не завершился")
                if failures:
                    raise RuntimeError("Network fixture не пройден") from failures[0]
            if network_invalidate and not proof:
                raise RuntimeError("Нет подтверждения сохранения ядра после смены сети")
        events = read_trace(trace)
        if not events or not events[-1].get("closed"):
            raise RuntimeError("Нет подтверждения closed после cleanup")
        if not screenshot.is_file() or screenshot.stat().st_size < 1000:
            raise RuntimeError("Окно не сохранило собственный screenshot")
        text = trace.read_text()
        if "invented-secret" in text or "subscription" in text or "password" in text:
            raise RuntimeError("Чувствительные данные попали в trace")
        allowed = {
            "status",
            "ready",
            "busy",
            "error",
            "revision",
            "selected_profile",
            "closed",
            "source_count",
            "profile_count",
            "pending",
            "tcp_busy",
            "tcp_status",
            "tcp_samples_count",
            "tcp_samples_requested",
            "step",
            "can_stop",
            "probe_present",
        }
        if any(set(event) - allowed for event in events):
            raise RuntimeError("Trace содержит поля вне разрешённых metadata")
        if probe_url is not None and (
            probe_url in text or UUID in text or "https://" in text
        ):
            raise RuntimeError("Тестовый endpoint или credential попал в trace")
        if engine is not None and not any(
            event.get("status") == expected_state for event in events
        ):
            raise RuntimeError(f"Не наблюдалось состояние {expected_state} живого ядра")
        if tcp_status is not None and not any(
            event.get("tcp_status") == tcp_status for event in events
        ):
            raise RuntimeError("Ожидаемый TCP результат не получен")
        if tcp_status == "reachable" and not any(
            event.get("tcp_status") == "reachable"
            and event.get("tcp_samples_count") == 3
            and event.get("tcp_samples_requested") == 3
            for event in events
        ):
            raise RuntimeError("GUI не подтвердил три TCP-выборки")
        if engine is not None:
            observed = next(
                index
                for index, event in enumerate(events)
                if event.get("status") == expected_state
            )
            if not any(
                event.get("status") == "Disconnected"
                for event in events[observed + 1 :]
            ):
                raise RuntimeError("После Stop не наблюдалось Disconnected")
            if (
                probe_url is not None
                and not network_invalidate
                and not any(event.get("error") == "busy" for event in events)
            ):
                raise RuntimeError("Активная сессия не отклонила изменение endpoint")
        if network_invalidate and not any(
            event.get("status") == "Limited"
            and event.get("error") == "network_changed"
            and event.get("can_stop") is True
            and event.get("probe_present") is False
            and event.get("tcp_status") is None
            for event in events
        ):
            raise RuntimeError("Trace не подтвердил очистку результатов network event")
        if gate is not None and any(
            event.get("status") == "Connected" for event in events
        ):
            raise RuntimeError("Поздний HTTPS-ответ восстановил Connected")
        if gate is not None:
            if not gate.refresh_passed.is_set() or not any(
                event.get("revision") == 2
                and event.get("status") == "Limited"
                and event.get("can_stop") is True
                for event in events
            ):
                raise RuntimeError(
                    "Нет backend checkpoint после terminal HTTPS и до Stop"
                )
        return {
            **(
                {
                    "checking_cancelled": True,
                    "late_response_ignored": gate.terminal == "sent",
                    "cancelled_transport_closed": gate.terminal == "closed",
                    "terminal_before_stop": True,
                }
                if gate is not None
                else {}
            ),
            **(
                {"network_invalidated": True, "core_kept_alive": True}
                if network_invalidate
                else {}
            ),
            "case": name,
            "events": len(events),
            "closed": True,
            "screenshot": str(screenshot),
            **(
                {
                    "engine": engine,
                    "expected_status": expected_status,
                    "status": "Limited" if network_invalidate else expected_state,
                }
                if probe_url is not None
                else {}
            ),
        }
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def run_checking_case(
    binary: Path,
    engines: Path,
    output: Path,
    engine: str,
    port: int,
    https: ThreadingHTTPServer,
    workspace: Path,
) -> dict[str, object]:
    gate = ProbeGate()
    handler = https.RequestHandlerClass
    https.RequestHandlerClass = type(
        "HeldProbeFixture", (HeldHttpsHandler,), {"gate": gate}
    )
    checking_output = output / "checking"
    checking_output.mkdir(exist_ok=True)
    try:
        result = run_case(
            binary,
            engines,
            checking_output,
            f"network-{engine}-checking",
            False,
            engine,
            False,
            network_invalidate=True,
            gate=gate,
            profile=f"vless://{UUID}@127.0.0.1:{port}#Тестовый сервер",
            probe_url=f"https://127.0.0.1:{https.server_port}/",
            expected_state="Checking",
            ca=workspace / "health.ca.pem",
        )
        if (
            not gate.received.is_set()
            or not gate.completed.is_set()
            or gate.timed_out.is_set()
        ):
            raise RuntimeError("HTTPS fixture не подтвердил удержание и поздний ответ")
        LOG.info(
            "network-%s-checking: Checking → Limited → late HTTPS → Stop → closed",
            engine,
        )
        return result
    finally:
        gate.release.set()
        https.RequestHandlerClass = handler


def run_health(
    binary: Path, engines: Path, output: Path, *, network_invalidate: bool = False
) -> list[dict[str, object]]:
    results = []
    with tempfile.TemporaryDirectory(prefix="tofu-gui-health-") as temporary:
        workspace = Path(temporary)
        with https_server(workspace, "health", 0, "127.0.0.1") as https:
            for engine, directory in (
                ("xray", "xray-26.10.31"),
                ("sing-box", "sing-box-1.14.4"),
            ):
                port = free_port()
                with peer(
                    engines / directory / engine,
                    engine,
                    workspace,
                    port=port,
                    target_ports=str(https.server_port),
                ):
                    for suffix, path, status, state in (
                        ("204", "/", 204, "Connected"),
                        ("unexpected-200", "/ok200", 204, "Limited"),
                        ("custom-200", "/ok200", 200, "Connected"),
                    ):
                        if network_invalidate and suffix != "204":
                            continue
                        name = f"{'network' if network_invalidate else 'health'}-{engine}-{suffix}"
                        https.requested.clear()
                        results.append(
                            run_case(
                                binary,
                                engines,
                                output,
                                name,
                                False,
                                engine,
                                not network_invalidate,
                                network_invalidate=network_invalidate,
                                profile=f"vless://{UUID}@127.0.0.1:{port}#Тестовый сервер",
                                probe_url=f"https://127.0.0.1:{https.server_port}{path}",
                                expected_status=status,
                                expected_state=state,
                                observed_status=200 if path == "/ok200" else 204,
                                ca=workspace / "health.ca.pem",
                            )
                        )
                        if not https.requested.is_set():
                            raise RuntimeError("HTTPS fixture не получил проверку")
                        LOG.info(
                            "%s: %s → Stop → Disconnected → closed",
                            name,
                            "Connected → Limited/network_changed"
                            if network_invalidate
                            else state,
                        )
                    if network_invalidate:
                        results.append(
                            run_checking_case(
                                binary, engines, output, engine, port, https, workspace
                            )
                        )
    return results


def run_tcp(
    binary: Path, output: Path, *, native_dns: bool = True, namespace: bool = True
) -> list[dict[str, object]]:
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    port = listener.getsockname()[1]
    listener.listen(2)
    listener.settimeout(15)
    failures: list[str] = []
    closed: list[bool] = []
    cases = [("tcp-literal", "127.0.0.1", namespace, "reachable")]
    if native_dns:
        if namespace:
            cases.append(
                ("tcp-resolver-restricted", "localhost", True, "resolver_unavailable")
            )
        # Host root/service UID не отображены в user namespace. Проверяем native
        # resolver в исходном UID-пространстве; сценарий не запускает ядра.
        cases.append(("tcp-resolved", "localhost", False, "reachable"))
    expected_connections = 3 * sum(status == "reachable" for _, _, _, status in cases)

    def accept_probes() -> None:
        try:
            for _ in range(expected_connections):
                with listener.accept()[0] as client:
                    client.settimeout(5)
                    if client.recv(1) != b"":
                        raise RuntimeError("TCP probe отправил payload")
                    closed.append(True)
        except (OSError, RuntimeError) as error:
            failures.append(type(error).__name__)
        finally:
            listener.close()

    thread = threading.Thread(target=accept_probes, daemon=True)
    thread.start()
    results = []
    try:
        for name, host, scoped, status in cases:
            results.append(
                run_case(
                    binary,
                    None,
                    output,
                    name,
                    False,
                    None,
                    scoped,
                    profile=f"trojan://invented-secret@{host}:{port}#TCP fixture",
                    tcp_status=status,
                )
            )
        thread.join(timeout=15)
        if thread.is_alive() or failures or len(closed) != expected_connections:
            raise RuntimeError("TCP fixture не подтвердил EOF/cleanup")
        results.append(
            run_case(
                binary,
                None,
                output,
                "tcp-refused",
                False,
                None,
                namespace,
                profile=f"trojan://invented-secret@127.0.0.1:{port}#TCP closed fixture",
                tcp_status="connect_failed",
            )
        )
        return results
    finally:
        listener.close()
        thread.join(timeout=16)
        if thread.is_alive():
            raise RuntimeError("TCP fixture не завершился")


def network_worker(
    binary: Path, engines: Path, output: Path, host_namespace: int
) -> list[dict[str, object]]:
    global NETWORK_HOST_NAMESPACE
    NETWORK_HOST_NAMESPACE = host_namespace
    guard_network()
    subprocess.run(
        ["ip", "link", "set", "lo", "up"],
        check=True,
        capture_output=True,
        timeout=5,
    )
    with (
        tempfile.TemporaryDirectory(prefix="tofu-network-runtime-") as runtime,
        tempfile.TemporaryDirectory(prefix="tofu-r-", dir="/tmp") as alias,
    ):
        os.chmod(runtime, 0o700)
        # Bind сохраняет короткий AF_UNIX path; Qt отвергает symlink runtime.
        guard_network()
        subprocess.run(
            ["mount", "--bind", runtime, alias],
            check=True,
            capture_output=True,
            timeout=5,
        )
        try:
            with patch.dict(
                os.environ,
                {
                    "QT_QPA_PLATFORM": "offscreen",
                    "QT_QUICK_BACKEND": "software",
                    "QT_QUICK_CONTROLS_STYLE": "Basic",
                    "XDG_RUNTIME_DIR": alias,
                },
            ):
                return run_health(binary, engines, output, network_invalidate=True)
        finally:
            subprocess.run(
                ["umount", alias],
                check=True,
                capture_output=True,
                timeout=5,
            )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--worker-namespace", type=int, help=argparse.SUPPRESS)
    parser.add_argument("--engine-dir", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--no-namespace",
        action="store_true",
        help="Только data-only сценарии без запуска ядер",
    )
    parser.add_argument(
        "--case",
        choices=(
            "profiles",
            "partial",
            "xray",
            "sing-box",
            "health",
            "network",
            "tcp",
            "tcp-basic",
            "all",
        ),
        default="profiles",
    )
    arguments = parser.parse_args()
    if arguments.no_namespace and arguments.case not in (
        "profiles",
        "partial",
        "tcp-basic",
    ):
        parser.error(
            "--no-namespace разрешён только для profiles/partial/tcp-basic без ядра"
        )
    binary = arguments.binary.resolve(strict=True)
    engines = (
        arguments.engine_dir.resolve(strict=True) if arguments.engine_dir else None
    )
    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if arguments.case == "network":
        if engines is None:
            parser.error("Для network нужен --engine-dir")
        if arguments.worker_namespace is None:
            run_sandbox(
                [
                    "unshare",
                    "--user",
                    "--map-root-user",
                    "--net",
                    "--pid",
                    "--fork",
                    "--mount",
                    "--mount-proc",
                    "--keep-caps",
                    "--kill-child",
                    sys.executable,
                    str(Path(__file__).resolve()),
                    str(binary),
                    "--engine-dir",
                    str(engines),
                    "--output",
                    str(output),
                    "--case",
                    "network",
                    "--worker-namespace",
                    str(os.stat("/proc/self/ns/net").st_ino),
                ],
                timeout=120,
            )
            return 0
        results = network_worker(binary, engines, output, arguments.worker_namespace)
        (output / "results.json").write_text(
            json.dumps(results, ensure_ascii=False, indent=2) + "\n"
        )
        return 0
    if arguments.worker_namespace is not None:
        parser.error("Worker доступен только для network")
    cases = (
        ("profiles", "partial", "xray", "sing-box", "health", "tcp")
        if arguments.case == "all"
        else (arguments.case,)
    )
    results = []
    for name in cases:
        if name in ("tcp", "tcp-basic"):
            results.extend(
                run_tcp(
                    binary,
                    output,
                    native_dns=name == "tcp",
                    namespace=not arguments.no_namespace,
                )
            )
            continue
        if name == "health":
            if engines is None:
                parser.error("Для health нужен --engine-dir")
            results.extend(run_health(binary, engines, output))
            continue
        engine = name if name in ("xray", "sing-box") else None
        result = run_case(
            binary,
            engines,
            output,
            name,
            name == "partial",
            engine,
            not arguments.no_namespace,
        )
        results.append(result)
        LOG.info("%s: closed подтверждён, screenshot сохранён", name)
    (output / "results.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n"
    )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    raise SystemExit(main())
