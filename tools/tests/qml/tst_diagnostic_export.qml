import QtQuick
import QtQuick.Controls
import QtTest
import "../../../crates/yonik-desktop/qml" as UI

TestCase {
    name: "БезопаснаяДиагностика"
    visible: true
    width: 600
    height: 300
    when: windowShown
    QtObject {
        id: producer
        property int calls: 0
        property bool accepted: true
        function copy_diagnostics() { calls++; return accepted }
    }
    UI.DiagnosticsExport { id: panel; anchors.fill: parent; backend: producer }
    function test_explicit_copy_and_languages() {
        waitForRendering(panel)
        compare(producer.calls, 0)
        let button = findChild(panel, "copyDiagnosticsButton")
        let result = findChild(panel, "diagnosticsExportResult")
        compare(button.text, "Копировать диагностику")
        mouseClick(button)
        compare(producer.calls, 1)
        compare(result.text, "Скопировано")
        panel.english = true
        compare(button.text, "Copy diagnostics")
        mouseClick(button)
        compare(result.text, "Copied")
        producer.accepted = false
        mouseClick(button)
        compare(result.text, "Export unavailable: unknown error.")
        panel.english = false
        mouseClick(button)
        compare(result.text, "Экспорт недоступен: неизвестная ошибка.")
    }
}
