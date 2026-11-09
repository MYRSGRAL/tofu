"""Нативный GUI smoke: только вымышленные URI, свой HTTP fixture и PID namespace."""

from __future__ import annotations

import argparse
import json
import logging
import socket
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

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
) -> dict[str, object]:
    endpoint = free_port()
    bodies = [
        f"trojan://invented-secret@127.0.0.1:{endpoint}#Москва • TLS",
        "socks5://invented:password@192.0.2.1:1080#<b>Plain text</b>",
    ]
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
                    {"profiles_count": 2, "busy": False, "pending": False},
                    "select_profile",
                    id=1,
                    revision=1,
                )
            )
            if engine is not None:
                if engines is None:
                    raise ValueError("Для проверки ядра нужен --engine-dir")
                if engine != "xray":
                    steps.append(
                        step({"selected_profile": 1}, "set_engine", engine=engine)
                    )
                steps.extend(
                    [
                        step({"engine": engine, "selected_profile": 1}, "connect"),
                        {
                            **step({"status": "Limited", "can_stop": True}, "stop"),
                            "capture": True,
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
            run_sandbox(command, timeout=45)
        events = [json.loads(line) for line in trace.read_text().splitlines()]
        if not events or not events[-1].get("closed"):
            raise RuntimeError("Нет подтверждения closed после cleanup")
        if not screenshot.is_file() or screenshot.stat().st_size < 1000:
            raise RuntimeError("Окно не сохранило собственный screenshot")
        text = trace.read_text()
        if "invented-secret" in text or "subscription" in text or "password" in text:
            raise RuntimeError("Чувствительные данные попали в trace")
        if engine is not None and not any(
            event.get("status") == "Limited" for event in events
        ):
            raise RuntimeError("Не наблюдалось состояние Limited живого ядра")
        return {
            "case": name,
            "events": len(events),
            "closed": True,
            "screenshot": str(screenshot),
        }
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


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
        choices=("profiles", "partial", "xray", "sing-box", "all"),
        default="profiles",
    )
    arguments = parser.parse_args()
    if arguments.no_namespace and arguments.case not in ("profiles", "partial"):
        parser.error("--no-namespace разрешён только для profiles/partial без ядра")
    binary = arguments.binary.resolve(strict=True)
    engines = (
        arguments.engine_dir.resolve(strict=True) if arguments.engine_dir else None
    )
    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    cases = (
        ("profiles", "partial", "xray", "sing-box")
        if arguments.case == "all"
        else (arguments.case,)
    )
    results = []
    for name in cases:
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
