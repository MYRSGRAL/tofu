"""Нативный GUI smoke: только вымышленные URI, свой HTTP fixture и PID namespace."""

from __future__ import annotations

import argparse
import json
import logging
import os
import socket
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from unittest.mock import patch

from engine_compile import UUID
from engine_host import https_server, peer
from engine_smoke import run_sandbox

LOG = logging.getLogger("desktop-smoke")


class Fixture(BaseHTTPRequestHandler):
    body: bytes = b""

    def do_GET(self) -> None:
        self.send_response(200)
        self.send_header("Content-Length", str(len(self.body)))
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.end_headers()
        self.wfile.write(self.body)

    def log_message(self, _format: str, *args: object) -> None:
        # Данные запроса не входят в диагностический вывод.
        return


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def step(
    when: dict[str, object], command: str, **arguments: object
) -> dict[str, object]:
    return {"when": when, "command": {"command": command, **arguments}}


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
    ca: Path | None = None,
    tcp_status: str | None = None,
) -> dict[str, object]:
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
        "SubscriptionFixture", (Fixture,), {"body": "\n".join(bodies).encode()}
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
                finish = step({"tcp_status": tcp_status, "tcp_busy": False}, "shutdown")
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
                    steps.append(step(stop_when, "check_tcp"))
                    stop_when = {
                        **stop_when,
                        "tcp_status": "reachable",
                        "tcp_busy": False,
                    }
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
                            "capture": probe_url is None,
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
            if ca is None:
                run_sandbox(command, timeout=45)
            else:
                with patch.dict(os.environ, {"SSL_CERT_FILE": str(ca)}):
                    run_sandbox(command, timeout=45)
        events = [json.loads(line) for line in trace.read_text().splitlines()]
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
            "step",
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
            if probe_url is not None and not any(
                event.get("error") == "busy" for event in events
            ):
                raise RuntimeError("Активная сессия не отклонила изменение endpoint")
        return {
            "case": name,
            "events": len(events),
            "closed": True,
            "screenshot": str(screenshot),
            **(
                {
                    "engine": engine,
                    "expected_status": expected_status,
                    "status": expected_state,
                }
                if probe_url is not None
                else {}
            ),
        }
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def run_health(binary: Path, engines: Path, output: Path) -> list[dict[str, object]]:
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
                        name = f"health-{engine}-{suffix}"
                        https.requested.clear()
                        results.append(
                            run_case(
                                binary,
                                engines,
                                output,
                                name,
                                False,
                                engine,
                                True,
                                profile=f"vless://{UUID}@127.0.0.1:{port}#Тестовый сервер",
                                probe_url=f"https://127.0.0.1:{https.server_port}{path}",
                                expected_status=status,
                                expected_state=state,
                                ca=workspace / "health.ca.pem",
                            )
                        )
                        if not https.requested.is_set():
                            raise RuntimeError("HTTPS fixture не получил проверку")
                        LOG.info("%s: %s → Stop → Disconnected → closed", name, state)
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
    expected_connections = sum(status == "reachable" for _, _, _, status in cases)

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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
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
