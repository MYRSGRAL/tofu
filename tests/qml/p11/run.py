"""Изолированный Qt6 тест реальных scoped QML bindings без native backend."""

import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
source = (ROOT / "crates/yonik-desktop/qml/Main.qml").read_text()
routing = source[
    source.index(
        '                SmallLabel { objectName: "routingDiagnosticsScope"'
    ) : source.index(
        "                RowLayout {", source.index('objectName: "routingExplanation"')
    )
]
schedule = source[
    source.index(
        "        ColumnLayout {", source.index("        id: schedulePanel")
    ) : source.index("    Component {\n        id: appearancePanel")
].rstrip()
schedule = schedule[: schedule.rfind("    }")]
row = source[
    source.index(
        '                    SettingsRow { objectName: "settingsScheduleButton"'
    ) : source.index(
        "\n",
        source.index(
            '                    SettingsRow { objectName: "settingsScheduleButton"'
        ),
    )
]
header = """import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtTest
TestCase {
 id: window
 name: "P11Visible"
 width: 900; height: 1200; visible: true
 when: windowShown
 property string language: "ru"
 property var state: ({routing_revision: 7, routing_diagnostics: {incomplete: true, stale: false, warnings: [{rule_id: 2, earlier: 1, kind: "shadowed"}, {rule_id: 3, earlier: 1, kind: "partial"}]}, routing_explanation: null})
 property int routingRequestId: 2
 property bool routingPending: false
 property string settingsSection: ""
 property string connectionError: ""
 function tr(ru, en) { return language === "ru" ? ru : en }
 function send(command) {}
 function sendRouting(command) {}
 function errorText(code) { return tr("Не удалось сохранить настройки", "Could not save preferences") }
 component PlainLabel: Label {}
 component SmallLabel: Label {}
 component ActionButton: Button {}
 component SettingsRow: Button { property string description; property string symbol }
 ColumnLayout {
 enabled: !window.routingPending && !window.state.closed
"""
tests = """
 }
 function texts(item) {
   let result = []
   if (item.text !== undefined) result.push(item.text)
   if (item.description !== undefined) result.push(item.description)
   if (item.placeholderText !== undefined) result.push(item.placeholderText)
   for (let child of item.children) result = result.concat(texts(child))
   return result
 }
 function test_locales() {
   for (let lang of ["en", "ru", "en", "ru"]) {
     language = lang
     for (let stale of [false, true]) {
       routingPending = stale
       state = Object.assign({}, state, {closed: stale, busy: stale})
       for (let reason of ["application_witness_missing", "dns_witness_missing", "dns_missing", "evaluation_rejected", "secret-native-reason"]) {
         state = Object.assign({}, state, {routing_diagnostics: {incomplete: true, stale: stale, warnings: [{rule_id: 2, earlier: 1, kind: "shadowed"}, {rule_id: 3, earlier: 1, kind: "partial"}]}, routing_explanation: {revision: 7, request_id: 2, result: {status: "unknown", reason: reason}}})
         let all = texts(window).join("\\n")
         verify(all.indexOf("secret-native-reason") < 0)
         if (lang === "en") verify(!/[А-Яа-яЁё]/.test(all), all)
         else verify(all.indexOf("Обновления подписок") >= 0)
       }
     }
     let explanation = findChild(window, "routingExplanation")
     state = Object.assign({}, state, {routing_explanation: {revision: 7, request_id: 2, result: {status: "fixture", action: "proxy", origin: "rule"}}})
     verify(explanation.visible)
     verify(explanation.text.indexOf(lang === "en" ? "Preliminary" : "Предварительный") >= 0)
     state = Object.assign({}, state, {routing_explanation: {revision: 6, request_id: 2, result: {status: "fixture"}}})
     verify(!explanation.visible)
     state = Object.assign({}, state, {routing_explanation: {revision: 7, request_id: 1, result: {status: "fixture"}}})
     verify(!explanation.visible)
     state = Object.assign({}, state, {routing_explanation: null})
     compare(explanation.text, "")
   }
 }
}
"""
with tempfile.TemporaryDirectory(prefix="yonik-p11-") as directory:
    path = Path(directory) / "tst_visible.qml"
    path.write_text(header + row + routing + schedule + tests)
    result = subprocess.run(
        [
            "dbus-run-session",
            "--",
            "/usr/lib/qt6/bin/qmltestrunner",
            "-input",
            str(path),
        ],
        env=dict(os.environ, QT_QPA_PLATFORM="offscreen"),
        check=False,
    )
    raise SystemExit(result.returncode)
