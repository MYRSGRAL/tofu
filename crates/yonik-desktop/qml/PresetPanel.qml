pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: panel
    objectName: "presetPanel"
    required property var snapshot
    required property string language
    signal command(var payload)
    function tr(ru, en) { return language === "ru" ? ru : en }
    Label { text: "ru-direct"; font.bold: true }
    Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: panel.snapshot.current ? panel.tr("Текущая версия: ", "Current version: ") + panel.snapshot.current.version + " · " + panel.snapshot.current.checked_at_unix : panel.tr("Нет проверенного snapshot", "No verified snapshot")
    }
    Label {
        visible: !!panel.snapshot.previous
        text: panel.tr("Предыдущая версия: ", "Previous version: ") + (panel.snapshot.previous ? panel.snapshot.previous.version : "")
    }
    Label {
        visible: panel.snapshot.current_error || panel.snapshot.previous_error
        text: panel.tr("Ошибка целостности snapshot", "Snapshot integrity error")
    }
    RowLayout {
        Button { objectName: "presetStatusButton"; text: panel.tr("Проверить версии", "Check versions"); onClicked: panel.command({command: "preset_status"}) }
        Button { objectName: "presetRollbackButton"; text: panel.tr("Откатить", "Roll back"); enabled: !!panel.snapshot.previous; onClicked: panel.command({command: "preset_rollback"}) }
    }
    TextField {
        id: target
        Layout.fillWidth: true
        placeholderText: panel.tr("IP для проверки решения", "IP for decision check")
        maximumLength: 45
    }
    Button {
        text: panel.tr("Проверить модель Policy", "Check Policy model")
        enabled: !!panel.snapshot.current
        onClicked: panel.command({command: "preset_prepare", target: target.text})
    }
    Label {
        visible: panel.snapshot.prepared === true
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: (panel.snapshot.decision || "") + " · " + panel.tr("Решение evaluator. Проверка ядром и Host Apply не выполнена.", "Evaluator decision. Engine validation and Host Apply have not run.")
    }
    Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: panel.tr("Активация недоступна: требуется preset в Draft и проверка Apply. system_tun=false. Подготовка не меняет сеть.", "Activation unavailable: preset in Draft and Apply validation required. system_tun=false. Preparation does not change the network.")
    }
}
