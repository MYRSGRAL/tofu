#!/usr/bin/env python3
"""Actual production core exit / private-bus helper unavailable → RU/EN V2 copy.

Запускать внутри unshare -Urnpf --mount-proc с поднятым loopback/private D-Bus.
Smoke-test использует только собственную ephemeral БД; cleanup injection отсутствует.
"""

from __future__ import annotations

import argparse
import hashlib
import http.server
import json
import os
import select
import shlex
import signal
import subprocess
import tempfile
import threading
import time
from pathlib import Path

from check_diagnostic_actual_copy import INTERPOSE, ROOT

PIN = "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1"


def kill_owned_core(parent: int) -> str:
    """Сигнал только прямому ребёнку GUI после проверки pinned executable через pidfd."""
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if int((proc / "stat").read_text().rsplit(")", 1)[1].split()[1]) != parent:
                continue
            fd = os.pidfd_open(int(proc.name))
            try:
                digest = hashlib.sha256((proc / "exe").read_bytes()).hexdigest()
                if digest != PIN:
                    continue
                signal.pidfd_send_signal(fd, signal.SIGKILL)
                return digest
            finally:
                os.close(fd)
        except (OSError, ValueError):
            continue
    raise RuntimeError("Не найден принадлежащий GUI pinned core")


class Subscription(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        body = b"vless://11111111-1111-4111-8111-111111111111@127.0.0.1:9#P11"
        self.send_response(200)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        pass


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("task", type=Path)
    parser.add_argument(
        "--mode",
        choices=("core", "helper", "cleanup", "logs", "maintenance"),
        required=True,
    )
    args = parser.parse_args()
    if (
        os.getuid() != 0
        or "4294967295" in Path("/proc/self/uid_map").read_text()
        or os.environ.get("QT_QPA_PLATFORM") != "offscreen"
    ):
        parser.error("нужны isolated user/net/PID namespace и offscreen")
    task = args.task.resolve()
    task.mkdir(parents=True, mode=0o700, exist_ok=True)
    mapping = (
        (ROOT / "crates/yonik-desktop/qml/Main.qml")
        .read_text()
        .split("    function errorText(key) {", 1)[1]
        .split("\n    DesktopBridge", 1)[0]
    )
    with (
        http.server.ThreadingHTTPServer(("127.0.0.1", 0), Subscription) as server,
        tempfile.TemporaryDirectory(prefix="yp11r-") as runtime,
    ):
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        url = f"http://127.0.0.1:{server.server_port}/"
        destination = task / "already-exists"
        destination.write_text("P11 existing destination")
        mode = json.dumps(args.mode)
        catalog = json.loads(
            (ROOT / "docs/specs/diagnostic-message-catalog.json").read_text()
        )
        messages = {
            entry["message_key"]: {"ru": entry["ru"], "en": entry["en"]}
            for entry in catalog["messages"]
        }
        qml = (
            """import QtQuick
import QtQuick.Controls
import cloud.highpriority.yonik
ApplicationWindow {
 id: window; visible: true; width: 640; height: 480
 property string language: "ru"
 property int phase: 0
 function tr(ru,en) { return language === "ru" ? ru : en }
 function errorText(key) {"""
            + mapping
            + """
 DesktopBridge { id: backend }
 TextEdit { id: probe }
 function require(ok, message) { if (!ok) { console.error(message); Qt.exit(3); throw Error(message) } }
 function send(command) { backend.dispatch(JSON.stringify(command)) }
 function clipboard() { probe.clear(); probe.paste(); return probe.text }
 Component.onCompleted: backend.initialize()
 Timer { interval: 100; running: true; repeat: true; property int ticks: 0
 onTriggered: {
  require(++ticks < 400, "runtime failure timeout phase " + phase)
  const state = JSON.parse(backend.snapshot_json)
  const mode = __P11_MODE__
  const ackMode = mode === "logs" || mode === "maintenance"
  const key = mode === "logs" ? "logs_save_failed" : mode === "maintenance" ? "busy" : mode === "helper" ? "system_unavailable" : mode === "cleanup" ? "cleanup_failed" : "engine_failed"
  const failed = mode === "logs" ? state.logs && state.logs.request_id === "101" && state.logs.status === "refused" : mode === "maintenance" ? state.maintenance_ack && state.maintenance_ack.request_id === 201 && state.maintenance_ack.phase === "failed" : state.error === key
  if (phase === 0 && state.ready) {
    if (mode === "logs") { send({command:"logs_read",after:"0",source:"all",request_id:"100"}); phase=20 }
    else if (mode === "maintenance") { send({command:"maintenance_start",request_id:201,operation:"backup",destination:__P11_DEST__,revision:state.revision}); phase=4 }
    else if (mode === "helper") { send({command:"set_connection_mode", mode:"system"}); phase=10 }
    else { send({command:"set_probe_target", url:"https://127.0.0.1:1/", expected_status:200}); send({command:"import_source", name:"P11", url:__P11_URL__, local_exception:true, route:"direct"}); phase=1 }
  } else if (phase === 10 && state.connection_mode === "system") { send({command:"system_state"}); phase=4 }
  else if (phase === 20 && state.logs && state.logs.request_id === "100" && (state.logs.status === "ready" || state.logs.status === "empty")) { send({command:"logs_save",page_request_id:"100",request_id:"101",destination:__P11_DEST__}); phase=4 }
  else if (phase === 1 && state.profiles && state.profiles.length) { send({command:"select_profile",id:state.profiles[0].id,revision:state.revision}); phase=2 }
  else if (phase === 2 && state.selected_profile) { send({command:"connect"}); phase=3 }
  else if (phase === 3 && (state.status === "Limited" || state.status === "Connected")) { if (mode === "cleanup") send({command:"stop"}); else console.log("P11 CORE_READY"); phase=4 }
  else if (phase === 4 && failed) {
    stop()
    const reason = mode === "logs" ? "logs_io_already_exists" : mode === "maintenance" ? "maintenance_copy_conflict" : mode === "helper" ? "helper_unavailable" : mode === "cleanup" ? "cleanup_failed" : "engine_exited"
    const stage = mode === "logs" ? "logs_save" : mode === "maintenance" ? "maintenance_copy" : mode === "helper" ? "helper_rpc" : mode === "cleanup" ? "cleanup" : "engine_run"
    for (let locale of ["ru","en"]) {
      language=locale; probe.text=window.errorText(key)
      require(probe.text === __P11_MESSAGES__[key][locale], "UI catalog mismatch")
      console.log("P11 UI " + mode + " " + locale + " " + probe.text)
      require(backend.copy_diagnostics_v2(), "V2 actual rejected")
      const exported=clipboard(); const metadata=JSON.parse(exported)
      require(metadata.schema_version===2 && metadata.errors.length===(ackMode ? 1 : 2), "V2 shape")
      for (let error of metadata.errors) require(error.reason===reason && error.stage===stage && error.retryable===false && error.code===key.toUpperCase(), "typed cause mismatch")
      console.log("P11 EXPORT " + mode + " " + locale + " " + exported)
    }
    const previous=clipboard()
    for (let [field,value] of [["reason","CANARY\\u001b"],["params",{url:"https://secret:CANARY@example.invalid"}],["extra",true],["request_id","202"],["slot","routing_ack"]]) {
      const bad=JSON.parse(JSON.stringify(state)); bad.error_contexts[0][field]=value
      backend.snapshot_json=JSON.stringify(bad)
      require(!backend.copy_diagnostics_v2() && clipboard()===previous,"negative clipboard changed")
    }
    console.log("P11 PASS " + mode + " negative clipboard unchanged")
    Qt.quit()
  }
 } }
}
""".replace("__P11_MODE__", mode)
            .replace("__P11_MESSAGES__", json.dumps(messages, ensure_ascii=False))
            .replace("__P11_URL__", json.dumps(url))
            .replace("__P11_DEST__", json.dumps(str(destination)))
        )
        fixture = task / "runtime.qml"
        fixture.write_text(qml)
        source = task / "interpose.cpp"
        source.write_text(INTERPOSE)
        library = task / "interpose.so"
        flags = shlex.split(
            subprocess.check_output(
                ["pkg-config", "--cflags", "--libs", "Qt6Qml"], text=True
            )
        )
        subprocess.run(
            [
                "clang++",
                "-shared",
                "-fPIC",
                str(source),
                "-o",
                str(library),
                *flags,
                "-ldl",
            ],
            check=True,
        )
        env = dict(
            os.environ,
            LD_PRELOAD=str(library),
            YONIK_TEST_QML=str(fixture),
            XDG_RUNTIME_DIR=runtime,
            DBUS_SYSTEM_BUS_ADDRESS=os.environ["DBUS_SESSION_BUS_ADDRESS"],
            QT_FORCE_STDERR_LOGGING="1",
            QT_LOGGING_RULES="qml.debug=true;qml.warning=true",
        )
        data = tempfile.mkdtemp(prefix="data-", dir=task)
        command = [
            str(args.binary.resolve()),
            "--data-dir",
            data,
            "--engine-dir",
            "/home/myrs/projects/yonik/local/engines",
        ]
        if args.mode == "cleanup":
            import socket

            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                port = listener.getsockname()[1]
            script = task / "steps.json"
            script.write_text('{"steps":[]}')
            command += [
                "--listen-port",
                str(port),
                "--smoke-script",
                str(script),
                "--smoke-trace",
                str(task / "trace.jsonl"),
            ]
            print(
                "P11 CONTROLLED_NEGATIVE cleanup hook; not production cleanup failure",
                flush=True,
            )
        else:
            command += ["--smoke-test"]
        process = subprocess.Popen(
            command,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        lines = []
        deadline = time.monotonic() + 50
        try:
            assert process.stdout is not None
            while time.monotonic() < deadline:
                if select.select([process.stdout], [], [], 1)[0]:
                    line = process.stdout.readline()
                    if not line:
                        break
                    print(line, end="", flush=True)
                    lines.append(line)
                    if "P11 PASS cleanup" in line:
                        assert process.stdin is not None
                        process.stdin.write("1")
                        process.stdin.flush()
                    if "P11 CORE_READY" in line:
                        print(
                            "P11 OWNED_CORE_SHA256 " + kill_owned_core(process.pid),
                            flush=True,
                        )
                elif process.poll() is not None:
                    break
            process.wait(timeout=5)
            if process.returncode != 0:
                raise RuntimeError(f"GUI exit={process.returncode}")
            output = "".join(lines)
            for marker in [
                f"P11 UI {args.mode} ru",
                f"P11 UI {args.mode} en",
                f"P11 EXPORT {args.mode} ru",
                f"P11 EXPORT {args.mode} en",
                f"P11 PASS {args.mode}",
            ]:
                if output.count(marker) != 1:
                    raise RuntimeError("Нет actual evidence: " + marker)
            print(
                "P11 BINARY_SHA256 "
                + hashlib.sha256(args.binary.read_bytes()).hexdigest()
            )
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            server.shutdown()


if __name__ == "__main__":
    main()
