#!/usr/bin/env python3
"""Реальный GUI host → scheduler Import → strict V2 → clipboard RU/EN."""

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
    parser.add_argument("--valid", action="store_true")
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
    key = "import"
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
  console.log("TRACE SNAPSHOT " + snapshot_json)
  if (JSON.parse(snapshot_json).closed) console.log("E2E closed=true")
 } }
 TextEdit { id: probe }
 function errorText(key) {MAPPING
 function require(value) { if (!value) { Qt.exit(3); throw new Error("scheduler proof failed") } }
 function clipboard() { probe.clear(); probe.paste(); return probe.text }
 property bool tested: false
 Text { id: visibleError; text: window.errorText("import") }
 Component.onCompleted: { backend.initialize(); backend.dispatch(JSON.stringify({command:"subscription_visibility",visible:true})) }
 Timer { interval: 100; running: true; repeat: true; onTriggered: {
  const state=JSON.parse(backend.snapshot_json)
  if (state.closed) { console.log("E2E closed=true"); Qt.quit(); return }
  if (tested || !state.error_contexts || state.error_contexts.length===0) return
  tested=true
  require(typeof state.scheduler_generation === "number")
  require(state.error_contexts.length === 2)
  require(state.error_contexts.every(c => c.reason==="scheduler_service_import_failed" && c.stage==="scheduler_service"))
  console.log("E2E SNAPSHOT " + backend.snapshot_json)
  for (let locale of ["ru", "en"]) {
   language=locale
   require(visibleError.text===MESSAGES[locale])
   require(backend.copy_diagnostics_v2())
   console.log("SCHEDULER EXPORT " + locale + " " + clipboard())
   console.log("E2E UI " + locale + " " + visibleError.text)
  }
  console.log("SCHEDULER native clipboard PASS")
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
    if args.valid:
        start = qml.index("  if (tested || !state.error_contexts")
        end = qml.index("  backend.shutdown()", start)
        qml = (
            qml[:start]
            + """  if (tested || !state.pending) return
  tested=true
  require(!state.error_contexts || state.error_contexts.length===0)
  console.log("POSITIVE valid body review PASS")
"""
            + qml[end:]
        )
    fixture = args.task / "scheduler.qml"
    fixture.write_text(qml)
    result = subprocess.run(
        [
            str(args.binary.resolve()),
            str(args.task / "host"),
            *(["valid"] if args.valid else []),
        ],
        env=dict(
            os.environ,
            LD_PRELOAD=str(library),
            YONIK_TEST_QML=str(fixture),
            QT_FORCE_STDERR_LOGGING="1",
            QT_LOGGING_RULES="qml.debug=true;qml.warning=true",
        ),
        stdout=(args.task / "live-qt.log").open("w"),
        stderr=subprocess.STDOUT,
        text=True,
        timeout=30,
        check=False,
    )
    output = (args.task / "live-qt.log").read_text()
    (args.task / "qt.log").write_text(output)
    result.check_returncode()
    if args.valid:
        if (
            "POSITIVE valid body review PASS" not in output
            or "E2E closed=true" not in output
        ):
            raise RuntimeError(output)
        return
    prefix = "E2E SNAPSHOT "
    captured = next(
        line.split(prefix, 1)[1] for line in output.splitlines() if prefix in line
    )
    (args.task / "actual-snapshot.json").write_text(captured)
    for locale in ("ru", "en"):
        prefix = f"SCHEDULER EXPORT {locale} "
        line = next(
            line.split(prefix, 1)[1] for line in output.splitlines() if prefix in line
        )
        export_metadata(line.encode())
        (args.task / f"export-{locale}.json").write_text(line)
    if (
        "SCHEDULER native clipboard PASS" not in output
        or "E2E closed=true" not in output
    ):
        raise RuntimeError(output)
    print(output)


if __name__ == "__main__":
    main()
