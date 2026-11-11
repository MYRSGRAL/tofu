import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: root
    required property var backend
    property bool english: false
    property var response: null
    property var page: null
    property string pending: ""
    property bool saving: false
    property string saveStatus: ""
    property int serial: 0
    property bool stale: true
    function tr(ru, en) { return root.english ? en : ru }
    function read(next) {
        root.saving = false
        root.serial += 1
        root.pending = String(root.serial)
        root.stale = true
        root.backend.dispatch(JSON.stringify({command: "logs_read", after: next && root.page ? root.page.cursor : "0", source: source.currentIndex === 1 ? "app" : source.currentIndex === 2 ? "engine" : "all", request_id: root.pending}))
    }
    onResponseChanged: {
        if (root.visible && root.pending !== "" && root.response && root.response.request_id === root.pending) {
            if (root.saving) {
                root.saveStatus = root.response.status
                root.saving = false
                root.pending = ""
                return
            }
            root.page = root.response
            root.pending = ""
            root.stale = root.page.status !== "ready" && root.page.status !== "empty"
        }
    }
    Label { text: root.tr("Журнал событий", "Event log") }
    ComboBox {
        id: source
        objectName: "logsSource"
        model: [root.tr("Все", "All"), root.tr("Приложение", "Application"), root.tr("Движок", "Engine")]
        onActivated: { root.pending = ""; root.stale = true }
    }
    RowLayout {
        Button { objectName: "logsRefresh"; text: root.tr("Открыть / обновить", "Open / refresh"); onClicked: root.read(false) }
        Button { objectName: "logsNext"; text: root.tr("Далее", "Next"); enabled: root.pending === "" && !root.stale && !!root.page; onClicked: root.read(true) }
    }
    Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: root.pending !== "" ? root.tr("Чтение…", "Reading…") : root.stale ? root.tr("Требуется обновление", "Refresh required") : root.page && root.page.status === "empty" ? root.tr("Нет событий", "No events") : root.tr("Страница готова", "Page ready")
    }
    Label {
        visible: !!root.page && root.page.status === "cancelled"
        text: root.tr("Чтение отменено. Повторите запрос.", "Reading cancelled. Retry request.")
    }
    Label {
        visible: !!root.page && root.page.status === "busy"
        text: root.tr("Журнал занят. Повторите чтение.", "Log busy. Retry reading.")
    }
    Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: root.page ? root.tr("Курсор / вытеснено / отклонено / пропущено: ", "Cursor / evicted / rejected / missed: ") + [root.page.cursor, root.page.evicted, root.page.rejected, root.page.missed].join(" / ") : ""
    }
    ListView {
        objectName: "logsRows"
        Layout.fillWidth: true
        Layout.preferredHeight: 200
        clip: true
        model: root.visible && !root.stale && root.page ? root.page.events : []
        delegate: Label {
            required property var modelData
            width: ListView.view.width
            textFormat: Text.PlainText
            text: modelData.id + " · " + (modelData.source === "app" ? root.tr("Приложение", "Application") : root.tr("Движок", "Engine")) + " · " + modelData.code + (modelData.value === null ? "" : " · " + modelData.value)
        }
    }
    TextField {
        id: destination
        objectName: "logsDestination"
        Layout.fillWidth: true
        placeholderText: root.tr("Абсолютный путь нового файла", "Absolute path of a new file")
        maximumLength: 4096
    }
    Button {
        objectName: "logsSave"
        text: root.tr("Сохранить страницу", "Save page")
        enabled: root.pending === "" && !root.stale && !!root.page && destination.text !== ""
        onClicked: {
            if (!root.visible || root.stale || root.pending !== "" || !root.page) return
            root.serial += 1
            root.pending = String(root.serial)
            root.saving = true
            root.saveStatus = ""
            root.backend.dispatch(JSON.stringify({command: "logs_save", page_request_id: root.page.request_id, request_id: root.pending, destination: destination.text}))
        }
    }
    Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        visible: root.saveStatus !== ""
        text: root.saveStatus === "saved" ? root.tr("Страница сохранена", "Page saved") : root.saveStatus === "saved_durability_unknown" ? root.tr("Файл создан; долговечность записи не подтверждена", "File created; durability unconfirmed") : root.tr("Сохранение отклонено. Выберите новый файл и обновите страницу.", "Save refused. Choose a new file and refresh the page.")
    }
    TextEdit {
        id: exportText
        objectName: "logsExportText"
        visible: false
        textFormat: TextEdit.PlainText
        text: !root.stale && root.page && root.page.export ? root.page.export : ""
        readOnly: true
    }
    Button {
        objectName: "logsCopy"
        text: root.tr("Копировать страницу", "Copy page")
        enabled: root.pending === "" && !root.stale && exportText.text !== ""
        onClicked: {
            if (!root.visible || root.stale || root.pending !== "" || !root.page || !root.page.export) return
            exportText.selectAll()
            exportText.copy()
            exportText.deselect()
        }
    }
}
