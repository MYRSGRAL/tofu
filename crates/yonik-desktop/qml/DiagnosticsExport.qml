import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: root
    required property var backend
    property bool english: false
    function tr(ru, en) { return root.english ? en : ru }
    Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: tr("Скопируйте безопасные метаданные диагностики для поддержки.", "Copy safe diagnostic metadata for support.")
    }
    Button {
        objectName: "copyDiagnosticsButton"
        text: tr("Копировать диагностику", "Copy diagnostics")
        onClicked: result.text = root.backend.copy_diagnostics()
            ? (tr("Скопировано", "Copied"))
            : (tr("Экспорт недоступен: неизвестная ошибка.", "Export unavailable: unknown error."))
    }
    Label { id: result; objectName: "diagnosticsExportResult"; Layout.fillWidth: true; wrapMode: Text.Wrap }
}
