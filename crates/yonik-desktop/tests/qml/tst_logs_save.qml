import QtQuick
import QtTest
import "../../qml"

TestCase {
    name: "LogsSave"
    width: 700
    height: 850
    visible: true
    when: windowShown
    QtObject {
        id: fake
        property var last: null
        function dispatch(raw) { last = JSON.parse(raw) }
    }
    LogsView { id: logs; width: 680; backend: fake }
    TextEdit { id: clipboardProbe; visible: false }

    function test_save_refusal_preserves_page_clipboard_and_fences_old_ack() {
        logs.read(false)
        var readId = fake.last.request_id
        logs.response = {request_id: readId, status: "ready", cursor: "1", evicted: "3", rejected: "2", missed: "3", events: [], export: "safe page"}
        findChild(logs, "logsCopy").clicked()
        clipboardProbe.paste()
        compare(clipboardProbe.text, "safe page")
        findChild(logs, "logsDestination").text = "/fixture/existing.txt"
        findChild(logs, "logsSave").clicked()
        compare(fake.last.command, "logs_save")
        compare(fake.last.page_request_id, readId)
        verify(fake.last.export === undefined)
        var saveId = fake.last.request_id
        verify(saveId !== readId)
        logs.response = {request_id: readId, status: "saved"}
        compare(logs.pending, saveId)
        logs.response = {request_id: saveId, status: "refused", export: null, events: []}
        compare(logs.pending, "")
        compare(logs.saveStatus, "refused")
        compare(logs.page.export, "safe page")
        clipboardProbe.clear()
        clipboardProbe.paste()
        compare(clipboardProbe.text, "safe page")
    }
}
