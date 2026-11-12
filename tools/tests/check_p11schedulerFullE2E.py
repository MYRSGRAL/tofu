#!/usr/bin/env python3
"""Реальный GUI host → Scheduler Full → strict V2 → clipboard RU/EN."""

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
    key = "storage"
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
  if (JSON.parse(snapshot_json).closed) { require(tested); console.log("E2E CLOSED " + snapshot_json); console.log("E2E closed=true") }
 } }
 TextEdit { id: probe }
 property bool copySucceeded: false
 Button { id: copyButton; text: window.tr("Копировать диагностику", "Copy diagnostics"); onClicked: window.copySucceeded=backend.copy_diagnostics_v2() }
 function errorText(key) {MAPPING
 function require(value) { if (!value) { Qt.exit(3); throw new Error("scheduler proof failed") } }
 function clipboard() { probe.clear(); probe.paste(); return probe.text }
 property bool tested: false
 Text { id: visibleError; text: window.errorText("storage") }
 Component.onCompleted: backend.initialize()
 Timer { interval: 100; running: true; repeat: true; onTriggered: {
  const state=JSON.parse(backend.snapshot_json)
  if (state.closed) { require(tested); console.log("E2E CLOSED " + backend.snapshot_json); console.log("E2E closed=true"); Qt.quit(); return }
  if (tested || !state.error_contexts || state.error_contexts.length===0) return
  tested=true
  require(typeof state.scheduler_generation === "number" && state.scheduler_generation > 0)
  require(state.sources_total === 2 && state.ready && !state.closed)
  require(state.error_contexts.length === 2)
  require(state.error_contexts.every(c => c.reason==="scheduler_full" && c.stage==="scheduler_admission"))
  console.log("E2E SNAPSHOT " + backend.snapshot_json)
  for (let locale of ["ru", "en"]) {
   language=locale
   require(visibleError.text===MESSAGES[locale])
   copyButton.clicked()
   require(copySucceeded)
   console.log("SCHEDULER EXPORT " + locale + " " + clipboard())
   console.log("E2E UI " + locale + " " + visibleError.text)
  }
  const previous=clipboard()
  for (let field of ["scheduler_generation", "unknown", "cause", "typed_cause"]) {
   const bad=JSON.parse(JSON.stringify(state))
   if (field==="scheduler_generation") bad.scheduler_generation=state.scheduler_generation+1
   else if (field==="unknown") bad.error_contexts[0].reason="unknown_scheduler_reason"
   else if (field==="typed_cause") delete bad.error_contexts[0].reason
   else delete bad.error_contexts
   backend.snapshot_json=JSON.stringify(bad)
   copyButton.clicked()
   require(!copySucceeded && clipboard()===previous)
   console.log("E2E refusal unchanged clipboard " + field)
  }
  console.log("SCHEDULER clipboard refusal PASS")
  backend.shutdown()
 } }

}
""".replace("MAPPING", mapping)
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
        timeout=300,
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
    prefix = "E2E CLOSED "
    closed = next(
        line.split(prefix, 1)[1] for line in output.splitlines() if prefix in line
    )
    (args.task / "closed-snapshot.json").write_text(closed)
    for locale in ("ru", "en"):
        prefix = f"SCHEDULER EXPORT {locale} "
        line = next(
            line.split(prefix, 1)[1] for line in output.splitlines() if prefix in line
        )
        export_metadata(line.encode())
        (args.task / f"export-{locale}.json").write_text(line)
    if (
        "SCHEDULER clipboard refusal PASS" not in output
        or "E2E closed=true" not in output
    ):
        raise RuntimeError(output)
    print(output)


if __name__ == "__main__":
    main()
