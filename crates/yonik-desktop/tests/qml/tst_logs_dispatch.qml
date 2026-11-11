import QtQuick
import QtTest
import "../../qml"

TestCase {
    name: "LogsDispatch"
    width: 600
    height: 600
    visible: true
    when: windowShown
    QtObject {
        id: fake
        property var last: null
        function dispatch(raw) { last = JSON.parse(raw) }
    }
    LogsView { id: logs; width: 580; backend: fake }

    function test_busy_ack_releases_pending_and_old_ack_cannot_finish_retry() {
        logs.read(false)
        var rejected = fake.last.request_id
        verify(logs.pending !== "")
        logs.response = {request_id: rejected, status: "busy", cursor: "0", evicted: "0", rejected: "0", missed: "0", events: [], export: null}
        compare(logs.pending, "")
        verify(logs.stale)
        compare(findChild(logs, "logsCopy").enabled, false)
        logs.read(false)
        var retry = fake.last.request_id
        verify(retry !== rejected)
        logs.response = {request_id: rejected, status: "ready", export: "old", events: []}
        compare(logs.pending, retry)
        logs.response = {request_id: retry, status: "ready", cursor: "0", evicted: "0", rejected: "0", missed: "0", export: "safe", events: []}
        compare(logs.pending, "")
        compare(logs.stale, false)
        compare(findChild(logs, "logsCopy").enabled, true)
    }
}
