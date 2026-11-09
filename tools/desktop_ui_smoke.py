"""Пользовательские действия в собственном Qt-окне с вымышленными данными."""

from __future__ import annotations

import argparse
import json
import logging
import os
import signal
import socket
import subprocess
import tempfile
import threading
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Any

from desktop_smoke import Fixture, read_trace

LOG = logging.getLogger("desktop-ui-smoke")


def ui(
    action: str,
    target: str = "",
    *,
    when: dict[str, object] | None = None,
    expect: dict[str, object] | None = None,
    capture: str | None = None,
    **values: object,
) -> dict[str, object]:
    step: dict[str, object] = {
        "when": when or {},
        "ui": {"action": action, "object": target, **values},
    }
    if expect:
        step["ui_when"] = expect
    if capture:
        step["capture_name"] = capture
    return step


def script(url: str, width: int, height: int) -> list[dict[str, object]]:
    return [
        ui(
            "resize",
            when={"ready": True},
            expect={"window": {"width": 520, "height": 800}},
            width=width,
            height=height,
        ),
        ui(
            "click",
            "settingsButton",
            expect={"mainConnectButton": {"enabled": False}},
            capture="home-empty.png",
        ),
        ui(
            "click",
            "settingsConnectionButton",
            expect={"window": {"settingsSection": ""}},
            capture="settings-index.png",
        ),
        ui(
            "click",
            "proxyCopyButton",
            expect={"proxyAddress": {"text": {"contains": "127.0.0.1:"}}},
            capture="settings-connection.png",
        ),
        ui("click", "homeButton"),
        ui("click", "settingsAppearanceButton"),
        ui(
            "key",
            "languageChoice",
            expect={"window": {"settingsSection": "appearance"}},
            key="Down",
        ),
        ui("key", "languageChoice", key="Enter"),
        ui(
            "key",
            "languageChoice",
            expect={"window": {"language": "en"}},
            key="Up",
            capture="settings-en.png",
        ),
        ui("key", "languageChoice", key="Enter"),
        ui("click", "homeButton"),
        ui(
            "click",
            "settingsSubscriptionsButton",
            expect={"window": {"language": "ru"}},
        ),
        ui("click", "addSourceButton"),
        ui(
            "text",
            "sourceUrlInput",
            expect={"addSourceDialog": {"visible": True}},
            value="https://cancel.invalid/?ui-secret=hidden",
        ),
        ui("key", "sourceAdvancedButton", key="Enter"),
        ui(
            "click", "sourceHwidChoice", expect={"sourceHwidChoice": {"checked": False}}
        ),
        ui("click", "revealSourceUrl"),
        ui("click", "sourceAddCancelButton"),
        ui("click", "addSourceButton", expect={"addSourceDialog": {"visible": False}}),
        ui(
            "key",
            "sourceAdvancedButton",
            key="Enter",
            expect={
                "sourceUrlInput": {"text": ""},
                "revealSourceUrl": {"checked": False},
                "sourceHwidChoice": {"checked": False},
            },
        ),
        ui("text", "sourceNameInput", value="Тест А"),
        ui("text", "sourceUrlInput", value=url),
        ui(
            "click",
            "sourceLocalException",
            expect={"sourceLocalException": {"checked": False}},
        ),
        ui(
            "click",
            "sourceHwidChoice",
            expect={"sourceHwidChoice": {"checked": False}},
        ),
        ui(
            "click",
            "sourceAddConfirmButton",
            expect={
                "sourceLocalException": {"checked": True},
                "sourceHwidChoice": {"checked": True},
            },
        ),
        ui(
            "click",
            "discardRefreshButton",
            when={"pending": True, "busy": False},
            expect={
                "refreshReviewDialog": {"visible": True},
                "sourceUrlInput": {"text": ""},
            },
            capture="import-review.png",
        ),
        ui(
            "click",
            "refreshSourceButton",
            when={"pending": False, "busy": False, "profiles_count": 0},
            expect={
                "refreshReviewDialog": {"visible": False},
                "subscriptionFeedback": {"visible": False},
            },
        ),
        ui("click", "approveRefreshButton", when={"pending": True, "busy": False}),
        ui(
            "click",
            "subscriptionHwidChoice",
            when={"pending": False, "busy": False, "profiles_count": 2},
            expect={"subscriptionHwidChoice": {"checked": True}},
        ),
        ui(
            "click",
            "approveRefreshButton",
            when={"pending": True, "busy": False, "source_hwid_enabled": False},
        ),
        ui(
            "click",
            "subscriptionHwidChoice",
            when={"pending": False, "busy": False, "source_hwid_enabled": False},
            expect={"subscriptionHwidChoice": {"checked": False}},
        ),
        ui(
            "click",
            "approveRefreshButton",
            when={"pending": True, "busy": False, "source_hwid_enabled": True},
        ),
        ui(
            "click",
            "subscriptionServersButton",
            when={"profiles_count": 2, "busy": False, "pending": False},
            expect={"subscriptionFeedback": {"visible": False}},
        ),
        ui(
            "click",
            "favorite_1",
            expect={"window": {"page": 1}},
            capture="picker-ru.png",
        ),
        ui(
            "text", "profileSearch", expect={"favorite_1": {"text": "★"}}, value="Plain"
        ),
        ui("key", "profileSearch", key="Enter"),
        ui(
            "text",
            "profileSearch",
            when={"profiles_count": 1},
            capture="picker-search.png",
            value="",
        ),
        ui("key", "profileSearch", key="Enter"),
        ui("click", "profileSelect_1", when={"profiles_count": 2}),
        ui("click", "homeFavoritesButton", expect={"window": {"page": 0}}),
        ui(
            "click",
            "profileSelect_1",
            when={"profiles_count": 1, "favorites_only": True},
        ),
        ui(
            "key",
            "mainConnectButton",
            when={"selected_profile": 1, "can_connect": True},
            expect={"window": {"page": 0}, "mainServerName": {"text": "Москва • TLS"}},
            capture="home-selected.png",
            key="Enter",
        ),
        ui(
            "click",
            "settingsButton",
            when={"error": "engine_missing", "can_stop": False},
            expect={
                "mainStatusLabel": {"text": "Ошибка подключения"},
                "mainErrorLabel": {"visible": True},
            },
            capture="home-error.png",
        ),
        ui("click", "settingsDiagnosticsButton"),
        ui("wheel", "diagnosticsScroll", delta=-1200),
        ui(
            "wheel",
            "diagnosticsScroll",
            expect={"diagnosticsScroll": {"atEnd": True, "moving": False}},
            delta=1200,
            capture="settings-scroll-bottom.png",
        ),
        ui(
            "click",
            "tcpCheckButton",
            expect={
                "window": {"settingsSection": "diagnostics"},
                "diagnosticsScroll": {"atStart": True, "moving": False},
            },
        ),
        ui(
            "text",
            "probeUrlInput",
            when={
                "tcp_status": "reachable",
                "tcp_samples_count": 3,
                "tcp_samples_requested": 3,
                "tcp_busy": False,
            },
            capture="settings-tcp.png",
            value="https://health.invalid/private?ui-secret=hidden",
        ),
        ui("value", "probeExpectedStatus", value=200),
        ui("click", "revealProbeUrl", expect={"revealProbeUrl": {"checked": False}}),
        ui(
            "click",
            "applyProbeTargetButton",
            expect={"revealProbeUrl": {"checked": True}},
        ),
        ui("click", "homeButton"),
        ui(
            "click",
            "settingsAppearanceButton",
            when={"probe_custom": True, "probe_expected_status": 200},
        ),
        ui("click", "homeButton"),
        ui("click", "settingsDiagnosticsButton"),
        ui(
            "key",
            "probeExpectedStatus",
            key="Tab",
            expect={"probeExpectedStatus": {"value": 200}},
        ),
        ui(
            "click",
            "resetProbeTargetButton",
            when={"probe_custom": True, "probe_expected_status": 200},
            expect={
                "probeUrlInput": {"text": ""},
                "revealProbeUrl": {"checked": False},
            },
        ),
        ui("click", "homeButton"),
        ui(
            "click",
            "settingsAppearanceButton",
            when={"probe_custom": False, "probe_expected_status": 204},
        ),
        ui("key", "languageChoice", key="Down"),
        ui("key", "languageChoice", key="Enter"),
        ui("click", "homeButton", expect={"window": {"language": "en"}}),
        ui("click", "settingsAboutButton", capture="settings-about.png"),
        ui(
            "click",
            "homeButton",
            expect={"buildIdLabel": {"visible": True, "text": {"contains": "0.1.0"}}},
            capture="about-build.png",
        ),
        ui("click", "homeButton"),
        ui(
            "click",
            "serverPickerButton",
            expect={"window": {"page": 0}},
            capture="home-en.png",
        ),
        ui("key", "profileSearch", expect={"window": {"page": 1}}, key="Escape"),
        {
            "when": {},
            "ui_when": {"window": {"page": 0}},
            "command": {"command": "shutdown"},
            "capture_name": "home-final.png",
        },
    ]


def execute(
    binary: Path,
    root: Path,
    steps: list[dict[str, object]],
    output: Path,
    scale: str,
    expected_exit: int = 0,
    engines: Path | None = None,
) -> list[dict[str, Any]]:
    source = root / "steps.json"
    source.write_text(json.dumps({"steps": steps}, ensure_ascii=False))
    trace = output / "trace.jsonl"
    env = os.environ.copy()
    env.update(
        QT_QPA_PLATFORM="offscreen",
        QT_QUICK_BACKEND="software",
        QT_QUICK_CONTROLS_STYLE="Basic",
        QT_FORCE_STDERR_LOGGING="1",
        QT_SCALE_FACTOR=scale,
    )
    command = [
        str(binary),
        "--smoke-test",
        "--data-dir",
        str(root / "data"),
        "--engine-dir",
        str(engines or root / "empty-engines"),
        "--smoke-script",
        str(source),
        "--smoke-trace",
        str(trace),
        "--smoke-screenshot",
        str(output / "default.png"),
    ]
    process = subprocess.Popen(
        command,
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    saved = bytearray()

    def drain() -> None:
        assert process.stderr is not None
        while chunk := process.stderr.read(8192):
            saved.extend(chunk[: max(0, 65536 - len(saved))])

    reader = threading.Thread(target=drain, daemon=True)
    reader.start()
    try:
        code = process.wait(timeout=40)
    finally:
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                continue
        reader.join(timeout=3)
        if process.stderr is not None:
            process.stderr.close()
        if reader.is_alive():
            raise RuntimeError("Не завершился reader собственного GUI")
        (output / "stderr.log").write_bytes(saved)
    if expected_exit == 0 and any(
        token in saved
        for token in (b"ReferenceError", b"TypeError", b"Binding loop detected")
    ):
        raise RuntimeError("QML runtime error in own window")
    events = read_trace(trace)
    if code != expected_exit or not events or not events[-1].get("closed"):
        progress = events[-1].get("step") if events else None
        raise RuntimeError(
            f"GUI exit={code}, ожидалось {expected_exit}; step={progress}\n"
            + saved[-8192:].decode(errors="replace")
        )
    if expected_exit == 0 and events[-1].get("step") != len(steps):
        raise RuntimeError("Не выполнены все UI-действия")
    if expected_exit != 0 and "Qt завершился с кодом 2".encode() not in saved:
        raise RuntimeError("Отказ не подтверждён smoke driver")
    text = trace.read_text()
    for secret in ("ui-secret", "invented-secret", "health.invalid", "subscription"):
        if secret in text:
            raise RuntimeError("Чувствительные поля попали в trace")
    if expected_exit == 0:
        for step in steps:
            if name := step.get("capture_name"):
                image = output / str(name)
                if not image.is_file() or image.stat().st_size < 1000:
                    raise RuntimeError("Нет собственного screenshot")
    return events


def run(
    binary: Path, output: Path, width: int, height: int, scale: str
) -> dict[str, object]:
    output.mkdir(parents=True, exist_ok=False)
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    port = listener.getsockname()[1]
    listener.listen(8)
    listener.settimeout(0.2)
    stop = threading.Event()
    accepted: list[bool] = []
    failures: list[str] = []

    def accept_tcp() -> None:
        try:
            while not stop.is_set():
                try:
                    stream, _ = listener.accept()
                except socket.timeout:
                    continue
                with stream:
                    stream.settimeout(3)
                    if stream.recv(1) != b"":
                        raise RuntimeError("TCP probe отправил payload")
                    accepted.append(True)
        except OSError as error:
            if not stop.is_set():
                failures.append(type(error).__name__)
        except RuntimeError as error:
            failures.append(str(error))

    tcp = threading.Thread(target=accept_tcp, daemon=True)
    tcp.start()
    body = f"trojan://invented-secret@127.0.0.1:{port}#Москва • TLS\nsocks5://invented:password@192.0.2.1:1080#<b>Plain text</b>\nunknown://not-supported".encode()
    captured_hwids: list[str | None] = []

    class UiFixture(Fixture):
        def do_GET(self) -> None:
            captured_hwids.append(self.headers.get("x-hwid"))
            super().do_GET()

    UiFixture.body = body
    handler = UiFixture
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    http = threading.Thread(target=server.serve_forever, daemon=True)
    http.start()
    try:
        with tempfile.TemporaryDirectory(prefix="tofu-ui-actions-") as temporary:
            steps = script(
                f"http://127.0.0.1:{server.server_port}/subscription", width, height
            )
            events = execute(binary, Path(temporary), steps, output, scale)
        rejected = []
        for label, bad_step in (
            ("missing-control", ui("click", "doesNotExist", when={"ready": True})),
            (
                "capture-traversal",
                {
                    **ui("resize", when={"ready": True}, width=640, height=780),
                    "capture_name": "../outside.png",
                },
            ),
        ):
            negative_output = output / label
            negative_output.mkdir()
            with tempfile.TemporaryDirectory(prefix="tofu-ui-invalid-") as temporary:
                execute(
                    binary,
                    Path(temporary),
                    [bad_step],
                    negative_output,
                    scale,
                    expected_exit=1,
                )
            if (output / "outside.png").exists():
                raise RuntimeError("Capture вышел за пределы собственного каталога")
            rejected.append(label)
        if (
            len(captured_hwids) != 4
            or not captured_hwids[0]
            or captured_hwids[2] is not None
            or captured_hwids[0] != captured_hwids[1]
            or captured_hwids[0] != captured_hwids[3]
        ):
            raise RuntimeError("HWID opt-in/toggle wire sequence не совпадает")
        saved_trace = (output / "trace.jsonl").read_text()
        if any(value and value in saved_trace for value in captured_hwids):
            raise RuntimeError("HWID попал в GUI trace")
        if failures or len(accepted) != 3:
            raise RuntimeError(f"TCP fixture: EOF={len(accepted)}, failures={failures}")
        result = {
            "steps": len(steps),
            "events": len(events),
            "tcp_eof": len(accepted),
            "hwid_requests": len(captured_hwids),
            "width": width,
            "height": height,
            "scale": scale,
            "rejected_scripts": rejected,
            "screenshots": [
                step["capture_name"] for step in steps if "capture_name" in step
            ],
        }
        (output / "result.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n"
        )
        return result
    finally:
        server.shutdown()
        server.server_close()
        http.join(timeout=3)
        stop.set()
        listener.close()
        tcp.join(timeout=4)
        if http.is_alive() or tcp.is_alive():
            raise RuntimeError("Не завершился собственный fixture")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--width", default=520, type=int, choices=range(520, 1601))
    parser.add_argument("--height", default=800, type=int, choices=range(640, 1401))
    parser.add_argument("--scale", default="1", choices=("1", "1.25", "1.5", "2"))
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    try:
        result = run(
            args.binary.resolve(strict=True),
            args.output.resolve(),
            args.width,
            args.height,
            args.scale,
        )
        LOG.info("UI actions passed: %s", result)
        return 0
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        LOG.error("%s", error)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
