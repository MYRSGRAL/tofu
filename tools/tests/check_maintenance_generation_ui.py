#!/usr/bin/env python3
"""Наблюдение actual maintenance replacement через production Main без записи snapshot."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import subprocess
from pathlib import Path

from check_diagnostic_export_v2 import export_metadata
from check_scheduler_production_ui import DRIVER, INTERPOSE

FLOW = r"""
 property var original: null
 property string previous: ""
 property int completed: 0
 Timer { interval: 200; running: true; repeat: true; onTriggered: {
  require(++driver.ticks<150,"deadline")
  const backend=window.backendForDiagnostics
  const state=JSON.parse(backend.snapshot_json)
  if(step===5 && state.maintenance_ack && state.maintenance_ack.phase==="failed") {
   console.log("GEN BLOCKED " + backend.snapshot_json); step=8; backend.shutdown(); return
  }
  if(!state.ready) return
  if(step===0) {
   require(SUPPORTED ? state.error_contexts.length===0 : (state.error_contexts.length===2 && state.error_contexts.every(c=>c.reason==="scheduler_interval_invalid")),"actual input contexts")
   original=state; console.log("GEN A " + backend.snapshot_json)
   click("settingsButton"); step=1; return
  }
  if(step===1) { click("settingsDiagnosticsButton"); step=2; return }
  if(step===2) {
   click("copyDiagnosticsButton"); previous=clipboard()
   require(previous.length>0,"initial clipboard"); console.log("GEN EXPORT A " + previous)
   window.back(); step=3; return
  }
  if(step===3) { click("settingsMaintenanceButton"); step=4; return }
  if(step===4) {
   const panel=find(window.contentItem,"maintenanceBackup").parent
   require(panel.available && !panel.busy,"maintenance available")
   panel.operation=completed===0?"backup":"restore"
   panel.sourcePath=BACKUP
   panel.destinationPath=completed===0?BACKUP:RESTORED
   panel.submit(); step=5; return
  }
  if(step===5) {
   const ack=state.maintenance_ack
   if(!ack || ack.phase!=="complete") return
   require(ack.request_id===completed+1 && !ack.error,"matching complete ACK")
   require(state.scheduler_generation!==original.scheduler_generation,"replacement B")
   console.log("GEN REPLACEMENT " + backend.snapshot_json)
   completed++; window.back(); step=6; return
  }
  if(step===6) { click("settingsDiagnosticsButton"); step=7; return }
  if(step===7) {
   click("copyDiagnosticsButton")
   console.log("GEN CURRENT " + backend.snapshot_json)
   console.log("GEN CLIPBOARD " + clipboard())
   if(completed===1) { window.back(); step=3; return }
   step=8; backend.shutdown()
  }
 } }
 Connections { target: window ? window.backendForDiagnostics : null
  function onSnapshot_jsonChanged() {
   const raw=window.backendForDiagnostics.snapshot_json
   const state=JSON.parse(raw)
   if(state.maintenance_ack) console.log("GEN OBSERVED " + raw)
   if(state.closed) { require(step===8,"closed after scenario"); console.log("GEN CLOSED " + raw) }
  }
 }
}
"""


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--supported-control",
        action="store_true",
        help="Исправный interval: обязательный positive backup/restore control",
    )
    args = parser.parse_args()
    if os.environ.get("QT_QPA_PLATFORM") != "offscreen" or not os.environ.get(
        "DBUS_SESSION_BUS_ADDRESS"
    ):
        parser.error("нужны offscreen и private D-Bus")
    out = args.output.resolve()
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    source = out / "observer.cpp"
    source.write_text(INTERPOSE)
    library = out / "observer.so"
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
    driver = out / "driver.qml"
    driver.write_text(
        DRIVER.split(" Timer {", 1)[0]
        + FLOW.replace("SUPPORTED", "true" if args.supported_control else "false")
        .replace("BACKUP", json.dumps(str(out / "backup.sqlite")))
        .replace("RESTORED", json.dumps(str(out / "restored.sqlite")))
    )
    result = subprocess.run(
        [str(args.binary.resolve()), str(out / "host")]
        + (["supported"] if args.supported_control else []),
        env=dict(
            os.environ,
            LD_PRELOAD=str(library),
            YONIK_UI_DRIVER=str(driver),
            QT_FORCE_STDERR_LOGGING="1",
        ),
        capture_output=True,
        text=True,
        timeout=40,
    )
    output = result.stdout + result.stderr
    (out / "qt.log").write_text(output)
    result.check_returncode()
    records: dict[str, list[dict]] = {}
    for line in output.splitlines():
        for marker in (
            "A",
            "EXPORT A",
            "REPLACEMENT",
            "CURRENT",
            "CLIPBOARD",
            "OBSERVED",
            "CLOSED",
            "BLOCKED",
        ):
            prefix = "GEN " + marker + " "
            if prefix in line:
                records.setdefault(marker, []).append(
                    json.loads(line.split(prefix, 1)[1])
                )
    if not args.supported_control:
        for value in records["EXPORT A"]:
            export_metadata(json.dumps(value).encode())
    if args.supported_control:
        assert "BLOCKED" not in records, "healthy control failed"
        assert all(not value["error_contexts"] for value in records["REPLACEMENT"])
    assert records["CLOSED"][-1]["closed"]
    if "BLOCKED" not in records:
        assert len(records["REPLACEMENT"]) == 2
    main_path = (
        Path(__file__).resolve().parents[2] / "crates/yonik-desktop/qml/Main.qml"
    )
    assert (
        "PRODUCTION MAIN SHA256 " + hashlib.sha256(main_path.read_bytes()).hexdigest()
        in output
    )
    summary = {
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "records": records,
    }
    (out / "observations.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2)
    )
    print(
        "BLOCKED"
        if "BLOCKED" in records
        else "Observed replacement (requires fence assessment)",
        out,
    )


if __name__ == "__main__":
    main()
