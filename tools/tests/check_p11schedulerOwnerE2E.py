#!/usr/bin/env python3
"""Реальный owner exhaustion: startup RU/EN, strict export и positive control."""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
from pathlib import Path

from check_diagnostic_actual_copy import INTERPOSE, ROOT
from check_diagnostic_export_v2 import export_metadata


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("task", type=Path)
    parser.add_argument("mode", choices=("positive", "exhausted"))
    args = parser.parse_args()
    if os.environ.get("QT_QPA_PLATFORM") != "offscreen" or not os.environ.get(
        "DBUS_SESSION_BUS_ADDRESS"
    ):
        parser.error("нужны offscreen и private D-Bus")
    args.task.mkdir(mode=0o700, parents=True, exist_ok=True)
    source = args.task / "interpose.cpp"
    source.write_text(INTERPOSE)
    library = args.task / "interpose.so"
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
    mapping = (
        (ROOT / "crates/yonik-desktop/qml/Main.qml")
        .read_text()
        .split("    function errorText(key) {", 1)[1]
        .split("\n    DesktopBridge", 1)[0]
    )
    catalog = json.loads(
        (ROOT / "docs/specs/diagnostic-message-catalog.json").read_text()
    )
    key = "startup"
    message = next(m for m in catalog["messages"] if m["message_key"] == key)
    qml = (
        """import QtQuick
import QtQuick.Controls
import cloud.highpriority.yonik
ApplicationWindow {
 id: window; visible: true; width: 500; height: 300
 property string language: "ru"
 function tr(ru,en) { return language === "ru" ? ru : en }
 DesktopBridge { id: backend; onSnapshot_jsonChanged: {
  const latest=JSON.parse(snapshot_json)
  if (latest.closed) {
   if (MODE === "positive") window.require(latest.error_contexts.length === 0 && !latest.error)
   console.log("E2E closed=true")
  }
  window.prove()
 } }
 TextEdit { id: probe }
 function errorText(key) {MAPPING
 function require(value) { if (!value) { Qt.exit(3); throw new Error("scheduler proof failed") } }
 function clipboard() { probe.clear(); probe.paste(); return probe.text }
 property bool tested: false
 Text { id: visibleError; text: window.errorText("startup") }
 Component.onCompleted: backend.initialize()
 function prove() {
  const state=JSON.parse(backend.snapshot_json)
  if (tested) return
  if (MODE === "positive") {
   if (!state.ready) return
   tested=true
   require(!state.error_contexts || state.error_contexts.length === 0)
   console.log("E2E SNAPSHOT " + backend.snapshot_json)
   backend.shutdown()
   return
  }
  if (!state.error_contexts || state.error_contexts.length === 0) return
  tested=true
  require(state.closed && !state.ready)
  require(state.scheduler_generation === null || state.scheduler_generation === undefined)
  require(state.error_contexts.length === 2)
  require(state.error_contexts.some(c => c.slot === "error"))
  require(state.error_contexts.some(c => c.slot === "source"))
  require(state.error_contexts.every(c => c.reason==="scheduler_initialize_exhausted" && c.stage==="scheduler_initialize"))
  console.log("E2E SNAPSHOT " + backend.snapshot_json)
  probe.text="owner-exhaustion-sentinel"; probe.selectAll(); probe.copy()
  require(clipboard()==="owner-exhaustion-sentinel")
  for (let locale of ["ru", "en"]) {
   language=locale
   require(visibleError.text===MESSAGES[locale])
   require(backend.copy_diagnostics_v2())
   console.log("SCHEDULER EXPORT " + locale + " " + clipboard())
   console.log("E2E UI " + locale + " " + visibleError.text)
  }
  console.log("E2E owner exhaustion PASS")
  backend.shutdown()
  Qt.quit()
 }
 Timer { interval: 100; running: true; repeat: true; onTriggered: window.prove() }

}
""".replace("MODE", json.dumps(args.mode))
        .replace("MAPPING", mapping)
        .replace("KEY", json.dumps(key))
        .replace(
            "MESSAGES",
            json.dumps({k: message[k] for k in ("ru", "en")}, ensure_ascii=False),
        )
    )
    fixture = args.task / "scheduler.qml"
    fixture.write_text(qml)
    result = subprocess.run(
        [
            str(args.binary.resolve()),
            str(args.task / "host"),
            args.mode,
        ],
        env=dict(
            os.environ,
            LD_PRELOAD=str(library),
            YONIK_TEST_QML=str(fixture),
            QT_FORCE_STDERR_LOGGING="1",
            QT_LOGGING_RULES="qml.debug=true;qml.warning=true",
        ),
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    output = result.stdout + result.stderr
    (args.task / "qt.log").write_text(output)
    result.check_returncode()
    prefix = "E2E SNAPSHOT "
    captured = next(
        line.split(prefix, 1)[1] for line in output.splitlines() if prefix in line
    )
    (args.task / "actual-snapshot.json").write_text(captured)
    if "E2E closed=true" not in output:
        raise RuntimeError(output)
    if args.mode == "exhausted" and "E2E owner exhaustion PASS" not in output:
        raise RuntimeError(output)
    if args.mode == "exhausted":
        for locale in ("ru", "en"):
            prefix = f"SCHEDULER EXPORT {locale} "
            exports = [
                line.split(prefix, 1)[1]
                for line in output.splitlines()
                if prefix in line
            ]
            if len(exports) != 1:
                raise RuntimeError("Нет единственного strict export")
            export_metadata(exports[0].encode())
            (args.task / f"export-{locale}.json").write_text(exports[0])
    print(output)


if __name__ == "__main__":
    main()
