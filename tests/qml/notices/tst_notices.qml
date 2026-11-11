import QtQuick
import QtTest
import "../../../crates/yonik-desktop/qml"

TestCase {
    name: "Notices"
    width: 800
    height: 700
    visible: true
    when: windowShown
    NoticesView { id: view; anchors.fill: parent }
    function test_resources() {
        compare(view.entries.length, 398)
        let list = findChild(view, "noticesList")
        let search = findChild(view, "noticesSearch")
        search.text = "ahash"
        compare(list.count, 1)
        wait(20)
        mouseClick(list.itemAtIndex(0), 20, 15)
        verify(view.selectedText.length > 100)
        search.text = "no-such-component"
        compare(list.count, 0)
        search.text = ""
        compare(list.count, 398)
        let entry = view.entries.filter(item => item.name === "ahash")[0]
        verify(view.readResource(entry.hashes[0]).length > 100)
        compare(view.readResource("../../etc/passwd"), "")
        compare(view.readResource("https://example.org"), "")
        let text = findChild(view, "noticesText")
        view.selectedText = "<b>NOTICE</b> & copyright"
        compare(text.textFormat, TextEdit.PlainText)
        compare(text.text, "<b>NOTICE</b> & copyright")
        view.english = true
        compare(search.placeholderText, "Search components")
        view.english = false
        compare(search.placeholderText, "Поиск компонента")
    }
}
