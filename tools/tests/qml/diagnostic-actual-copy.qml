import QtQuick
import QtQuick.Controls
import cloud.highpriority.yonik

ApplicationWindow {
    id: window
    visible: true
    width: 600
    height: 600
    property string language: "ru"
    DesktopBridge { id: backend }
    TextEdit { id: clipboardProbe; visible: true }
    function readClipboard() {
        clipboardProbe.clear()
        clipboardProbe.paste()
        return clipboardProbe.text
    }
    function require(value, message) {
        if (!value) { console.error(message); Qt.exit(3); throw new Error(message) }
    }
    Timer {
        interval: 50
        running: true
        onTriggered: {
            // Только контролируемые snapshot; DesktopHost не инициализируется.
            const keys = ["cancelled", "cleanup_failed", "storage", "invalid_json", "check_timeout", "dns_failed", "system_timeout"]
            for (let locale of ["ru", "en"]) {
              window.language = locale
              for (let key of keys) {
                backend.snapshot_json = JSON.stringify({error: key, native: "SYNTHETIC_PASSWORD_CANARY"})
                require(backend.copy_diagnostics(), "actualCopy rejected " + key)
                const copied = readClipboard()
                const metadata = JSON.parse(copied)
                require(metadata.errors[0].message_key === key, "clipboard key mismatch")
                require(metadata.errors[0].code === key.toUpperCase(), "clipboard code mismatch")
                require(copied.indexOf("CANARY") < 0, "clipboard leaked canary")
                console.log("actualCopy PASS " + key + " " + locale)
            }
            }
            const previous = readClipboard()
            for (let bad of ["UNKNOWN_CANARY", true, {}, "cancelled\u001b", "cancelled\u0085"]) {
                backend.snapshot_json = JSON.stringify({error: bad})
                require(!backend.copy_diagnostics(), "actualCopy accepted unknown/type/control")
                require(readClipboard() === previous, "rejection changed clipboard")
            }
            console.log("actualCopy PASS negative clipboard unchanged")
            Qt.quit()
        }
    }
}
