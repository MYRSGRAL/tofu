import QtQuick
import QtQuick.Controls
import QtTest
import "../../qml"

TestCase {
    name: "ManualEntry"
    width: 640; height: 800
    when: windowShown
    ManualEntry { id: form }
    SignalSpy { id: submitted; target: form; signalName: "submitted" }
    function field(name) { return findChild(form, "manual_" + name) }
    function setup(protocol) {
        form.open()
        findChild(form, "manualProtocol").currentIndex = protocol
        findChild(form, "manualTls").checked = protocol === 0 || protocol === 2
        field("serverName").text = "Сервер"
        field("host").text = "example.com"
        field("port").text = "443"
        field("uuid").text = "11111111-2222-4333-8444-555555555555"
        field("password").text = "synthetic-secret"
    }
    function cleanup() { form.close(); form.clear(); submitted.clear() }
    function test_protocols() {
        const protocols = ["vless", "vmess", "trojan", "shadowsocks", "socks", "http"]
        for (let i = 0; i < protocols.length; ++i) {
            setup(i)
            if (i >= 4) field("username").text = "alice"
            verify(form.valid())
            const outbound = JSON.parse(form.payload()).outbounds[0]
            compare(outbound.type, protocols[i]); compare(outbound.server, "example.com"); compare(outbound.server_port, 443)
            if (i < 2) compare(outbound.uuid, "11111111-2222-4333-8444-555555555555")
            else compare(outbound.password, "synthetic-secret")
            form.clear()
        }
    }
    function test_transports() {
        for (let i = 0; i < 3; ++i) {
            setup(i)
            findChild(form, "manualTls").checked = true
            field("sni").text = "tls.example.com"
            field("alpn").text = "h2,http/1.1"
            findChild(form, "manualTransport").currentIndex = 1
            field("path").text = "/ws?token=a&b"
            field("wsHost").text = "cdn.example.com"
            let outbound = JSON.parse(form.payload()).outbounds[0]
            compare(outbound.tls.server_name, "tls.example.com")
            compare(outbound.tls.alpn, ["h2", "http/1.1"])
            compare(outbound.transport, {type: "ws", path: "/ws?token=a&b", headers: {Host: "cdn.example.com"}})
            findChild(form, "manualTransport").currentIndex = 2
            field("service").text = "service-name"
            outbound = JSON.parse(form.payload()).outbounds[0]
            compare(outbound.transport, {type: "grpc", service_name: "service-name"})
            form.clear()
        }
    }
    function test_submit() {
        setup(0)
        const expected = form.payload()
        const button = findChild(form, "manualSubmit")
        button.clicked()
        compare(submitted.count, 1)
        compare(submitted.signalArguments[0][0], "Сервер")
        compare(submitted.signalArguments[0][1], expected)
        compare(field("uuid").text, "")
        compare(field("password").text, "")
    }
    function test_protocol_keyboard() {
        form.open()
        const combo = findChild(form, "manualProtocol")
        combo.forceActiveFocus()
        keyClick(Qt.Key_Down)
        compare(form.protocol, "vmess")
        keyClick(Qt.Key_Down)
        compare(form.protocol, "trojan")
        compare(findChild(form, "manualTls").checked, true)
    }
    function test_tls_policy() {
        setup(0)
        findChild(form, "manualTls").checked = false
        verify(!form.valid())
        field("host").text = "127.0.0.1"
        verify(form.valid())
        findChild(form, "manualProtocol").currentIndex = 2
        verify(!form.valid())
    }
    function test_invalid_and_clear() {
        setup(0); field("port").text = "65536"; verify(!form.valid())
        field("port").text = "443"; field("uuid").text = "bad"; verify(!form.valid())
        form.close(); compare(field("password").text, ""); compare(field("uuid").text, "")
        form.english = true; compare(form.title, "Manual server")
        form.english = false; compare(form.title, "Сервер вручную")
    }
}
