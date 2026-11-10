import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: root
    required property var backend
    property bool english: false
    Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: root.english ? "Copy safe diagnostic metadata for support." : "Скопируйте безопасные метаданные диагностики для поддержки."
    }
    Button {
        objectName: "copyDiagnosticsButton"
        text: root.english ? "Copy diagnostics" : "Копировать диагностику"
        onClicked: result.text = root.backend.copy_diagnostics()
            ? (root.english ? "Copied" : "Скопировано")
            : (root.english ? "Export unavailable: unknown error." : "Экспорт недоступен: неизвестная ошибка.")
    }
    Label { id: result; objectName: "diagnosticsExportResult"; Layout.fillWidth: true; wrapMode: Text.Wrap }
}
