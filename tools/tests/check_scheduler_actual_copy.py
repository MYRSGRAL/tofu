#!/usr/bin/env python3
"""Actual scheduler snapshot из Rust producer → настоящий Qt clipboard RU/EN."""

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
    parser.add_argument("snapshot", type=Path)
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
    message = next(m for m in catalog["messages"] if m["message_key"] == "storage")
    qml = (
        """import QtQuick
import QtQuick.Controls
import cloud.highpriority.yonik
ApplicationWindow {
 id: window; visible: true; width: 500; height: 300
 property string language: "ru"
 function tr(ru,en) { return language === "ru" ? ru : en }
 DesktopBridge { id: backend }
 TextEdit { id: probe }
 function errorText(key) {MAPPING
 function require(value) { if (!value) { Qt.exit(3); throw new Error("scheduler proof failed") } }
 function clipboard() { probe.clear(); probe.paste(); return probe.text }
 Timer { interval: 100; running: true; onTriggered: {
  const state=SNAPSHOT
  for (let locale of ["ru", "en"]) {
   language=locale
   require(window.errorText("storage")===MESSAGES[locale])
   backend.snapshot_json=JSON.stringify(state)
   require(backend.copy_diagnostics_v2())
   console.log("SCHEDULER EXPORT " + locale + " " + clipboard())
  }
  const previous=clipboard()
  for (let field of ["scheduler_generation", "channel", "replacement"]) {
   const bad=JSON.parse(JSON.stringify(state))
   if (field==="scheduler_generation") bad.scheduler_generation=78
   else if (field==="channel") bad.error_contexts[0].slot="routing_ack"
   else delete bad.error_contexts
   backend.snapshot_json=JSON.stringify(bad)
   require(!backend.copy_diagnostics_v2() && clipboard()===previous)
  }
  console.log("SCHEDULER clipboard refusal PASS")
  Qt.quit()
 } }
}
""".replace("MAPPING", mapping)
        .replace("SNAPSHOT", args.snapshot.read_text())
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
            "--smoke-test",
            "--data-dir",
            str(args.task / "data"),
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
        check=True,
    )
    output = result.stdout + result.stderr
    for locale in ("ru", "en"):
        prefix = f"SCHEDULER EXPORT {locale} "
        line = next(
            line.split(prefix, 1)[1] for line in output.splitlines() if prefix in line
        )
        export_metadata(line.encode())
    if "SCHEDULER clipboard refusal PASS" not in output:
        raise RuntimeError(output)
    print(output)


if __name__ == "__main__":
    main()
