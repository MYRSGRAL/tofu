import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml"

TestCase {
    name: "TrafficFixtures"
    width: 640; height: 800
    when: windowShown
    ManualEntry { id: form }
    SignalSpy { id: submitted; target: form; signalName: "submitted" }
    function field(name) { return findChild(form, "manual_" + name) }
    function test_emit() {
        for (let protocol = 0; protocol < 6; ++protocol) {
            for (let transport = 0; transport < (protocol < 3 ? 3 : 1); ++transport) {
                form.open()
                findChild(form, "manualProtocol").currentIndex = protocol
                field("serverName").text = "controlled-" + protocol + "-" + transport
                field("host").text = "127.0.0.1"
                field("port").text = "24443"
                field("uuid").text = "11111111-2222-4333-8444-555555555555"
                field("password").text = "synthetic-manual-secret"
                field("username").text = "alice"
                findChild(form, "manualTls").checked = protocol === 2 || transport > 0 || protocol === 5
                field("sni").text = "yonik.test"
                findChild(form, "manualTransport").currentIndex = transport
                field("path").text = "/manual-echo"
                field("wsHost").text = "yonik.test"
                field("service").text = "manual-service"
                verify(form.valid())
                findChild(form, "manualSubmit").clicked()
                compare(submitted.count, 1)
                console.log("MANUAL_FIXTURE:" + submitted.signalArguments[0][1])
                submitted.clear()
                form.close()
            }
        }
    }
}
