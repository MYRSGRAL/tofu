import QtQuick
import QtTest
import QtQuick.Dialogs

TestCase {
    id: test
    name: "MaintenanceFileFlow"
    width: 600
    height: 800
    when: windowShown
    property var panel
    property var commands: []
    property int serial: 0
    readonly property url candidateUrl: Qt.resolvedUrl("tst_Maintenance.qml")
    function init() { commands = []; panel.pending = 0; panel.feedback = "" }
    function initTestCase() {
        const component = Qt.createComponent(Qt.resolvedUrl("../../qml/MaintenanceView.qml"))
        compare(component.status, Component.Ready, component.errorString())
        panel = component.createObject(test, {
            width: 500, available: true,
            sendCommand: function(command) { test.commands.push(command) },
            errorText: function(key) { return key },
            nextRequestId: function() { test.serial += 1; return test.serial }
        })
        verify(panel !== null)
        findChild(panel, "maintenanceDestinationDialog").options = FileDialog.DontUseNativeDialog
        findChild(panel, "maintenanceSourceDialog").options = FileDialog.DontUseNativeDialog
    }
    function test_backup_dialog_and_matching_ack() {
        findChild(panel, "maintenanceBackup").clicked()
        const dialog = findChild(panel, "maintenanceDestinationDialog")
        tryCompare(dialog, "visible", true)
        dialog.selectedFile = "file:///tmp/yonik-qt-fixture-new.sqlite"
        dialog.accepted()
        compare(commands.length, 1)
        compare(commands[0].operation, "backup")
        verify(commands[0].source === undefined)
        compare(commands[0].destination, "/tmp/yonik-qt-fixture-new.sqlite")
        panel.response = {request_id: 999, phase: "failed", error: "storage"}
        verify(panel.busy)
        panel.response = {request_id: commands[0].request_id, phase: "failed", error: "storage"}
        verify(!panel.busy)
        compare(panel.destinationPath, "/tmp/yonik-qt-fixture-new.sqlite")
        dialog.close()
    }
    function test_cancel_matches_active_request_and_rejects_remote_url() {
        compare(panel.localPath("https://example.invalid/copy.sqlite"), "")
        panel.operation = "backup"
        panel.destinationPath = "/tmp/yonik-qt-fixture-cancel.sqlite"
        panel.submit()
        findChild(panel, "maintenanceCancel").clicked()
        compare(commands.length, 2)
        compare(commands[1].command, "maintenance_cancel")
        compare(commands[1].request_id, commands[0].request_id)
    }
    function test_restore_dialog_chain() {
        findChild(panel, "maintenanceRestore").clicked()
        const source = findChild(panel, "maintenanceSourceDialog")
        tryCompare(source, "visible", true)
        source.selectedFile = candidateUrl
        source.accepted()
        source.close()
        const destination = findChild(panel, "maintenanceDestinationDialog")
        tryCompare(destination, "visible", true)
        destination.selectedFile = "file:///tmp/yonik-qt-fixture-restored.sqlite"
        destination.accepted()
        compare(commands.length, 1)
        compare(commands[0].operation, "restore")
        compare(commands[0].source, panel.localPath(candidateUrl))
        compare(commands[0].destination, "/tmp/yonik-qt-fixture-restored.sqlite")
        source.close(); destination.close()
    }
}
