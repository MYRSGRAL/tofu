pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import cloud.highpriority.tofu

ApplicationWindow {
    id: window
    width: 1100
    height: 760
    minimumWidth: 820
    minimumHeight: 620
    visible: true
    title: "tofu"
    objectName: "tofuWindow"
    color: "#f5f4ef"
    palette.window: "#f5f4ef"
    palette.windowText: "#252b28"
    palette.text: "#252b28"
    palette.buttonText: "#252b28"
    palette.highlight: "#376b51"
    palette.highlightedText: "#ffffff"
    property string language: "ru"
    property int page: 0
    property string parseError: ""
    property var state: ({ready: false, busy: true, error: null, pending: null,
        sources: [], sources_total: 0, sources_offset: 0, source_id: null, profiles: [], total: 0, offset: 0,
        revision: 0, status: "Disconnected", selected_profile: null,
        selected_name: "", engine: "xray", active_name: "", active_engine: "", port: 10808, can_connect: false, can_stop: false})
    readonly property bool activeConnection: ["Preparing", "Starting", "Checking", "Connected", "Limited", "Stopping", "CleanupError"].indexOf(state.status) >= 0
    readonly property color ink: "#252b28"
    readonly property color muted: "#626b65"
    readonly property color accent: "#376b51"

    function tr(ru, en) { return language === "ru" ? ru : en }
    function send(command) { backend.dispatch(JSON.stringify(command)) }
    function query(offset) {
        send({command: "query", search: search.text, favorites_only: favorites.checked, offset: offset})
    }
    function readSnapshot() {
        try {
            const raw = backend.snapshot_json
            if (!raw || raw.length > 4194304) return
            const next = JSON.parse(raw)
            if (next && Object.keys(next).length === 0) return
            if (next && typeof next.error === "string" && !Array.isArray(next.sources) && !Array.isArray(next.profiles)) {
                state = Object.assign({}, state, {error: next.error})
                parseError = ""
                return
            }
            if (!next || !Array.isArray(next.sources) || !Array.isArray(next.profiles)) {
                parseError = tr("Не удалось прочитать состояние приложения.", "Could not read application state.")
                return
            }
            state = next
            parseError = ""
            if (next.pending && !reviewDialog.visible) reviewDialog.open()
            if (!next.pending && reviewDialog.visible) reviewDialog.close()
        } catch (_) {
            parseError = tr("Не удалось прочитать состояние приложения.", "Could not read application state.")
        }
    }
    function statusText(status) {
        const values = {
            Disconnected: ["Отключено", "Disconnected"], Preparing: ["Подготовка…", "Preparing…"],
            Starting: ["Запуск ядра…", "Starting engine…"], Checking: ["Проверка доступа…", "Checking connectivity…"],
            Connected: ["Подключено", "Connected"], Stopping: ["Отключение…", "Stopping…"],
            Limited: ["Соединение ограничено", "Connection limited"],
            Error: ["Ошибка подключения", "Connection failed"], CleanupError: ["Не удалось завершить отключение", "Cleanup failed"]
        }
        const pair = values[status]
        return pair ? tr(pair[0], pair[1]) : tr("Состояние недоступно", "State unavailable")
    }
    function errorText(key) {
        const messages = {
            startup: ["Не удалось запустить приложение.", "Could not start the application."],
            keyring_locked: ["Хранилище секретов заблокировано. Разблокируйте системный кошелёк и повторите запуск.", "Secret storage is locked. Unlock the system wallet and restart."],
            keyring_missing: ["Системное хранилище секретов недоступно.", "System secret storage is unavailable."],
            storage: ["Ошибка локального хранилища. Проверьте доступ к данным приложения.", "Local storage error. Check access to application data."],
            invalid_input: ["Проверьте введённые данные и адрес подписки.", "Check the input and subscription URL."],
            busy: ["Дождитесь завершения текущей операции.", "Wait for the current operation to finish."],
            source_required: ["Сначала выберите подписку.", "Select a subscription first."],
            unsupported_proxy: ["Загрузка подписки через прокси пока не поддерживается. Выберите прямой маршрут.", "Fetching subscriptions through a proxy is not supported yet. Choose Direct."],
            fetch: ["Не удалось загрузить подписку. Проверьте сеть и адрес. Сохранённые профили не изменены.", "Could not fetch the subscription. Check the network and URL. Saved profiles are unchanged."],
            "import": ["Не удалось распознать профили подписки.", "Could not parse subscription profiles."],
            review_required: ["Сначала подтвердите или отмените текущий импорт.", "Approve or discard the current import first."],
            profile_required: ["Сначала выберите профиль.", "Select a profile first."],
            profile_unavailable: ["Профиль недоступен или список изменился. Выберите профиль заново.", "The profile is unavailable or the list has changed. Select the profile again."],
            engine_missing: ["Прокси-ядро не найдено. Установите выбранное ядро и перезапустите приложение.", "Proxy engine not found. Install the selected engine and restart the application."],
            engine_config: ["Профиль несовместим с выбранным ядром. Проверьте параметры подключения.", "The profile is incompatible with the selected engine. Check connection settings."],
            engine_failed: ["Не удалось запустить прокси-ядро. Проверьте профиль и наличие ядра.", "Could not start the proxy engine. Check the profile and engine installation."],
            check_failed: ["Прокси работает, но доступность интернета не подтверждена.", "The proxy is running, but internet access is unconfirmed."],
            cancelled: ["Операция отменена.", "Operation cancelled."],
            cleanup_failed: ["Не удалось завершить отключение. Повторите остановку перед выходом.", "Cleanup failed. Retry stopping before quitting."]
        }
        const pair = messages[key]
        return pair ? tr(pair[0], pair[1]) : tr("Операция не выполнена. Проверьте настройки и повторите попытку.", "Operation failed. Check the settings and try again.")
    }

    DesktopBridge {
        id: backend
        onSnapshot_jsonChanged: window.readSnapshot()
    }
    Component.onCompleted: { backend.initialize(); readSnapshot() }

    component PlainLabel: Label {
        textFormat: Text.PlainText
        color: window.ink
        wrapMode: Text.WordWrap
    }
    component SectionTitle: PlainLabel {
        font.pixelSize: 25
        font.weight: Font.DemiBold
    }
    component SmallLabel: PlainLabel {
        color: window.muted
        font.pixelSize: 13
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0
        Rectangle {
            Layout.preferredWidth: 194
            Layout.fillHeight: true
            color: "#eaece5"
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 20
                spacing: 8
                PlainLabel { text: "tofu"; font.pixelSize: 35; font.weight: Font.DemiBold; Layout.bottomMargin: 6 }
                SmallLabel { text: window.tr("ЛИЧНЫЙ ПРОКСИ", "PERSONAL PROXY"); font.pixelSize: 10; font.letterSpacing: 1.5; Layout.bottomMargin: 25 }
                Repeater {
                    model: [window.tr("Обзор", "Overview"), window.tr("Подписки", "Subscriptions"), window.tr("Настройки", "Settings")]
                    delegate: Button {
                        id: navigationButton
                        required property int index
                        required property string modelData
                        Layout.fillWidth: true
                        text: modelData
                        checkable: true
                        checked: window.page === index
                        implicitHeight: 38
                        contentItem: PlainLabel { text: navigationButton.text; verticalAlignment: Text.AlignVCenter; font.weight: navigationButton.checked ? Font.DemiBold : Font.Normal }
                        background: Rectangle { radius: 6; color: navigationButton.checked ? "#d9e6dc" : (navigationButton.hovered ? "#e1e6dd" : "transparent"); border.color: navigationButton.activeFocus ? window.accent : "transparent" }
                        onClicked: window.page = index
                        Accessible.name: modelData
                    }
                }
                Item { Layout.fillHeight: true }
                Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: "#d3d8cf" }
                SmallLabel { text: window.tr("Локальный SOCKS5", "Local SOCKS5"); Layout.topMargin: 8 }
                PlainLabel { text: "127.0.0.1:" + window.state.port; font.pixelSize: 13 }
                SmallLabel { text: window.tr("Системная сеть не меняется", "System network is unchanged"); font.pixelSize: 11 }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 30
            spacing: 18
            Rectangle {
                visible: !!window.state.error || !!window.parseError
                Layout.fillWidth: true
                implicitHeight: errorLabel.implicitHeight + 24
                color: "#f5e8db"
                radius: 6
                PlainLabel {
                    id: errorLabel
                    anchors.fill: parent
                    anchors.margins: 12
                    text: window.parseError || window.errorText(window.state.error)
                    color: "#75431e"
                    Accessible.role: Accessible.AlertMessage
                }
            }
            StackLayout {
                currentIndex: window.page
                Layout.fillWidth: true
                Layout.fillHeight: true
                ColumnLayout {
                    spacing: 16
                    RowLayout {
                        SectionTitle { text: window.tr("Обзор", "Overview"); Layout.fillWidth: true }
                        SmallLabel { text: window.state.busy ? window.tr("Выполняется операция…", "Working…") : window.tr("Локальное подключение", "Local connection") }
                    }
                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: connectionLayout.implicitHeight + 40
                        color: "#ffffff"
                        radius: 8
                        border.color: "#dce1d8"
                        Rectangle { width: 5; anchors.top: parent.top; anchors.bottom: parent.bottom; color: window.activeConnection ? window.accent : "#afb9ad"; radius: 2 }
                        ColumnLayout {
                            id: connectionLayout
                            anchors.fill: parent
                            anchors.margins: 20
                            spacing: 9
                            RowLayout {
                                Layout.fillWidth: true
                                Rectangle { implicitWidth: 8; implicitHeight: 8; radius: 4; color: window.activeConnection ? window.accent : "#919b92" }
                                PlainLabel { text: window.statusText(window.state.status); font.weight: Font.DemiBold; color: window.accent }
                                Item { Layout.fillWidth: true }
                                SmallLabel { text: "SOCKS5"; font.letterSpacing: 1 }
                            }
                            PlainLabel {
                                text: (window.state.can_stop ? window.state.active_name : window.state.selected_name) || window.tr("Выберите профиль", "Choose a profile")
                                font.pixelSize: 23
                                font.weight: Font.DemiBold
                                Layout.fillWidth: true
                                maximumLineCount: 2
                                elide: Text.ElideRight
                            }
                            SmallLabel {
                                visible: window.state.can_stop
                                text: window.tr("Активное ядро: ", "Active engine: ") + window.state.active_engine
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                text: window.state.status === "Limited"
                                    ? window.tr("Прокси-ядро работает, но проверка доступа не пройдена. Доступность интернета не подтверждена.", "The proxy engine is running, but the connectivity check failed. Internet access is unconfirmed.")
                                    : window.tr("Настройте SOCKS5 в нужном приложении. Это подключение не является TUN или системным VPN.", "Configure SOCKS5 in your application. This connection is not TUN or a system VPN.")
                            }
                            RowLayout {
                                Layout.topMargin: 8
                                PlainLabel { text: window.tr("Выбранное ядро", "Selected engine") }
                                ComboBox {
                                    id: engineSelect
                                    model: ["xray", "sing-box"]
                                    currentIndex: window.state.engine === "sing-box" ? 1 : 0
                                    enabled: window.state.ready && !window.state.busy && !window.state.can_stop
                                    onActivated: window.send({command: "set_engine", engine: currentText})
                                    Accessible.name: window.tr("Прокси-ядро", "Proxy engine")
                                }
                                Item { Layout.fillWidth: true }
                                Button {
                                    text: window.state.can_stop ? window.tr("Отключить", "Disconnect") : window.tr("Подключить", "Connect")
                                    highlighted: !window.state.can_stop
                                    enabled: window.state.ready && (window.state.can_stop || (!window.state.busy && window.state.can_connect))
                                    onClicked: window.send({command: window.state.can_stop ? "stop" : "connect"})
                                }
                            }
                        }
                    }
                    RowLayout {
                        PlainLabel { text: window.tr("Профили", "Profiles"); font.pixelSize: 18; font.weight: Font.DemiBold; Layout.fillWidth: true }
                        ComboBox {
                            id: overviewSources
                            Layout.maximumWidth: 220
                            model: window.state.sources
                            textRole: "name"
                            valueRole: "id"
                            currentIndex: { for (let i = 0; i < window.state.sources.length; ++i) if (window.state.sources[i].id === window.state.source_id) return i; return -1 }
                            displayText: currentIndex < 0 ? window.tr("Выберите подписку", "Choose subscription") : currentText
                            // Штатный TextField показывает plain text и сохраняет API native style.
                            delegate: ItemDelegate {
                                id: sourceOption
                                required property var modelData
                                width: overviewSources.width
                                contentItem: PlainLabel {
                                    text: sourceOption.modelData.name
                                    maximumLineCount: 1
                                    elide: Text.ElideRight
                                }
                            }
                            onActivated: { search.clear(); favorites.checked = false; window.send({command: "select_source", id: currentValue}) }
                            enabled: window.state.ready && !window.state.busy && count > 0
                            Accessible.name: window.tr("Подписка профилей", "Profile subscription")
                        }
                    }
                    RowLayout {
                        TextField {
                            id: search
                            Layout.fillWidth: true
                            placeholderText: window.tr("Имя или адрес сервера", "Server name or address")
                            maximumLength: 256
                            selectByMouse: true
                            enabled: window.state.ready && !window.state.busy
                            onAccepted: window.query(0)
                            Accessible.name: window.tr("Поиск профилей", "Search profiles")
                        }
                        Button { text: window.tr("Найти", "Search"); enabled: window.state.ready && !window.state.busy; onClicked: window.query(0) }
                        CheckBox { id: favorites; text: window.tr("Избранное", "Favorites"); enabled: window.state.ready && !window.state.busy; onClicked: window.query(0) }
                    }
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: "#ffffff"
                        border.color: "#dce1d8"
                        radius: 6
                        ListView {
                            id: profileList
                            anchors.fill: parent
                            anchors.margins: 1
                            clip: true
                            model: window.state.profiles
                            boundsBehavior: Flickable.StopAtBounds
                            ScrollBar.vertical: ScrollBar {}
                            delegate: ItemDelegate {
                                id: profileRow
                                required property var modelData
                                required property int index
                                width: ListView.view.width
                                height: 66
                                highlighted: window.state.selected_profile === modelData.id
                                background: Rectangle { color: profileRow.highlighted ? "#e3eee6" : (profileRow.hovered ? "#f0f3ee" : "#ffffff"); border.color: profileRow.activeFocus ? window.accent : "transparent"; radius: 4 }
                                enabled: window.state.ready && !window.state.busy
                                Accessible.name: modelData.name + ", " + modelData.host
                                onClicked: window.send({command: "select_profile", id: modelData.id, revision: window.state.revision})
                                contentItem: RowLayout {
                                    spacing: 12
                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 3
                                        PlainLabel { text: profileRow.modelData.name; Layout.fillWidth: true; elide: Text.ElideRight; maximumLineCount: 1; font.weight: Font.Medium }
                                        SmallLabel { text: profileRow.modelData.host + ":" + profileRow.modelData.port + " · " + profileRow.modelData.protocol; Layout.fillWidth: true; elide: Text.ElideRight; maximumLineCount: 1 }
                                    }
                                    SmallLabel { visible: !profileRow.modelData.available; text: window.tr("Недоступен", "Unavailable") }
                                    ToolButton {
                                        text: profileRow.modelData.favorite ? "★" : "☆"
                                        font.pixelSize: 23
                                        Accessible.name: profileRow.modelData.favorite ? window.tr("Убрать из избранного", "Remove favorite") : window.tr("В избранное", "Add favorite")
                                        onClicked: window.send({command: "favorite", id: profileRow.modelData.id, value: !profileRow.modelData.favorite})
                                    }
                                }
                            }
                        }
                        ColumnLayout {
                            anchors.centerIn: parent
                            width: Math.min(parent.width - 40, 400)
                            visible: window.state.profiles.length === 0
                            spacing: 10
                            PlainLabel { Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; text: window.state.sources.length === 0 ? window.tr("Начните с подписки", "Start with a subscription") : window.tr("Профили не найдены", "No profiles found"); font.pixelSize: 20 }
                            SmallLabel { Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; text: window.state.sources.length === 0 ? window.tr("Добавьте ссылку, проверьте импорт и выберите сервер.", "Add a link, review the import and choose a server.") : window.tr("Обновите подписку или измените условия поиска.", "Refresh the subscription or change the search filters.") }
                            Button { Layout.alignment: Qt.AlignHCenter; text: window.state.sources.length === 0 ? window.tr("Добавить подписку", "Add subscription") : window.tr("Открыть подписки", "Open subscriptions"); onClicked: { window.page = 1; if (window.state.sources.length === 0) addDialog.open() } }
                        }
                    }
                    RowLayout {
                        SmallLabel { text: window.state.total > 0 ? (window.state.offset + 1) + "–" + (window.state.offset + window.state.profiles.length) + " / " + window.state.total : window.tr("Нет профилей", "No profiles"); Layout.fillWidth: true }
                        Button { text: window.tr("Назад", "Previous"); enabled: window.state.offset > 0 && !window.state.busy; onClicked: window.query(Math.max(0, window.state.offset - 200)) }
                        Button { text: window.tr("Далее", "Next"); enabled: window.state.offset + window.state.profiles.length < window.state.total && !window.state.busy; onClicked: window.query(window.state.offset + 200) }
                    }
                }
                ColumnLayout {
                    spacing: 18
                    RowLayout {
                        SectionTitle { text: window.tr("Подписки", "Subscriptions"); Layout.fillWidth: true }
                        Button { text: window.tr("Добавить", "Add"); highlighted: true; enabled: window.state.ready && !window.state.busy; onClicked: addDialog.open() }
                    }
                    SmallLabel { Layout.fillWidth: true; text: window.tr("Ссылки хранятся локально и не показываются в списках. Обновление применяется после проверки импорта.", "Links are stored locally and are hidden from lists. Updates are applied after import review.") }
                    ListView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: window.state.sources
                        spacing: 4
                        ScrollBar.vertical: ScrollBar {}
                        delegate: ItemDelegate {
                            id: subscriptionRow
                            required property var modelData
                            width: ListView.view.width
                            height: 64
                            text: modelData.name
                            highlighted: window.state.source_id === modelData.id
                            enabled: !window.state.busy
                            contentItem: PlainLabel { text: subscriptionRow.modelData.name; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight; maximumLineCount: 1 }
                            onClicked: window.send({command: "select_source", id: modelData.id})
                        }
                        PlainLabel { anchors.centerIn: parent; width: parent.width - 40; horizontalAlignment: Text.AlignHCenter; visible: window.state.sources.length === 0; text: window.tr("Подписок пока нет. Добавьте первую по ссылке.", "No subscriptions yet. Add your first subscription by URL.") }
                    }
                    RowLayout {
                        SmallLabel {
                            Layout.fillWidth: true
                            text: window.state.sources_total > 0
                                ? (window.state.sources_offset + 1) + "–" + (window.state.sources_offset + window.state.sources.length) + " / " + window.state.sources_total
                                : window.tr("Нет подписок", "No subscriptions")
                        }
                        Button {
                            text: window.tr("Назад", "Previous")
                            enabled: window.state.ready && !window.state.busy && window.state.sources_offset > 0
                            onClicked: window.send({command: "source_page", offset: Math.max(0, window.state.sources_offset - 64)})
                        }
                        Button {
                            text: window.tr("Далее", "Next")
                            enabled: window.state.ready && !window.state.busy && window.state.sources_offset + window.state.sources.length < window.state.sources_total
                            onClicked: window.send({command: "source_page", offset: window.state.sources_offset + 64})
                        }
                    }
                    RowLayout {
                        SmallLabel { text: window.tr("Выберите подписку для обновления", "Select a subscription to refresh"); Layout.fillWidth: true }
                        Button { text: window.tr("Обновить", "Refresh"); enabled: window.state.ready && window.state.source_id !== null && !window.state.busy && !window.state.pending; onClicked: window.send({command: "refresh"}) }
                    }
                }
                ColumnLayout {
                    spacing: 20
                    SectionTitle { text: window.tr("Настройки", "Settings") }
                    GridLayout {
                        columns: 2
                        columnSpacing: 30
                        rowSpacing: 20
                        PlainLabel { text: window.tr("Язык интерфейса", "Interface language") }
                        ComboBox { model: ["Русский", "English"]; currentIndex: window.language === "ru" ? 0 : 1; onActivated: window.language = currentIndex === 0 ? "ru" : "en"; Accessible.name: window.tr("Язык интерфейса", "Interface language") }
                        PlainLabel { text: window.tr("Режим подключения", "Connection mode") }
                        PlainLabel { text: window.tr("Локальный SOCKS5", "Local SOCKS5") }
                        PlainLabel { text: window.tr("Адрес прокси", "Proxy address") }
                        PlainLabel { text: "127.0.0.1:" + window.state.port }
                    }
                    Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: "#dce1d8" }
                    SmallLabel { Layout.fillWidth: true; text: window.tr("Эта версия предоставляет локальный прокси. TUN, системный VPN и редактор сетевых политик ещё не доступны. Язык выбирается на текущий сеанс.", "This version provides a local proxy. TUN, system VPN and the routing policy editor are not available yet. Language applies to this session.") }
                    SmallLabel { Layout.fillWidth: true; text: window.tr("Если трей доступен, закрытие окна скрывает приложение в нём. Для выхода используйте меню трея. Без трея закрытие отключает прокси и завершает приложение.", "When a tray is available, closing hides the window there. Use the tray menu to quit. Without a tray, closing disconnects the proxy and exits.") }
                    Button { text: window.tr("Перечитать состояние", "Reload state"); enabled: window.state.ready && !window.state.busy; onClicked: window.send({command: "reload"}) }
                    Item { Layout.fillHeight: true }
                    SmallLabel { text: "tofu · " + window.tr("Открытый исходный код", "Open source") }
                }
            }
        }
    }
    Dialog {
        id: addDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 64, 540)
        modal: true
        title: window.tr("Добавить подписку", "Add subscription")
        closePolicy: Popup.CloseOnEscape
        onClosed: { sourceName.clear(); sourceUrl.clear(); localException.checked = false; route.currentIndex = 0; reveal.checked = false }
        contentItem: ColumnLayout {
            spacing: 12
            PlainLabel { text: window.tr("Название", "Name") }
            TextField { id: sourceName; Layout.fillWidth: true; maximumLength: 128; selectByMouse: true; Accessible.name: window.tr("Название подписки", "Subscription name") }
            PlainLabel { text: window.tr("Ссылка подписки", "Subscription URL") }
            RowLayout {
                TextField { id: sourceUrl; Layout.fillWidth: true; maximumLength: 8192; echoMode: reveal.checked ? TextInput.Normal : TextInput.Password; selectByMouse: true; inputMethodHints: Qt.ImhUrlCharactersOnly | Qt.ImhNoPredictiveText | Qt.ImhSensitiveData; Accessible.name: window.tr("Секретная ссылка подписки", "Secret subscription URL") }
                CheckBox { id: reveal; text: window.tr("Показать", "Show") }
            }
            SmallLabel { Layout.fillWidth: true; text: window.tr("Ссылка может содержать токен доступа. Не передавайте её другим людям.", "The link may contain an access token. Keep it private.") }
            CheckBox { id: localException; Layout.fillWidth: true; text: window.tr("Разрешить HTTP и локальные / приватные адреса", "Allow HTTP and local / private addresses"); contentItem: PlainLabel { text: localException.text; leftPadding: localException.indicator.width + localException.spacing; verticalAlignment: Text.AlignVCenter } }
            SmallLabel { Layout.fillWidth: true; text: window.tr("Включайте исключение только для доверенного источника в своей сети.", "Enable this exception only for a trusted source in your own network.") }
            PlainLabel { text: window.tr("Маршрут загрузки", "Download route") }
            ComboBox { id: route; Layout.fillWidth: true; model: [window.tr("Напрямую", "Direct"), window.tr("Через прокси (пока недоступно)", "Proxy (not available yet)")]; Accessible.name: window.tr("Маршрут загрузки подписки", "Subscription download route") }
            SmallLabel { visible: route.currentIndex === 1; Layout.fillWidth: true; color: "#75431e"; text: window.tr("Загрузка через прокси пока не поддерживается. Backend отклонит этот маршрут.", "Proxy fetching is not supported yet. The backend will reject this route.") }
            RowLayout {
                Layout.topMargin: 10
                Item { Layout.fillWidth: true }
                Button { text: window.tr("Отмена", "Cancel"); onClicked: addDialog.close() }
                Button {
                    text: window.tr("Добавить", "Add")
                    highlighted: true
                    enabled: sourceName.text.trim().length > 0 && sourceUrl.text.trim().length > 0 && !window.state.busy
                    onClicked: {
                        window.send({command: "add_source", name: sourceName.text.trim(), url: sourceUrl.text.trim(), local_exception: localException.checked, route: route.currentIndex === 0 ? "direct" : "proxy"})
                        sourceUrl.clear()
                        addDialog.close()
                    }
                }
            }
        }
        onOpened: sourceName.forceActiveFocus()
    }
    Dialog {
        id: reviewDialog
        anchors.centerIn: parent
        width: Math.min(window.width - 64, 520)
        title: window.tr("Проверка импорта", "Review import")
        modal: true
        closePolicy: Popup.NoAutoClose
        contentItem: ColumnLayout {
            spacing: 16
            PlainLabel { Layout.fillWidth: true; text: window.tr("Проверьте результат перед заменой сохранённых профилей.", "Review the result before replacing saved profiles.") }
            PlainLabel { text: window.tr("Принято: ", "Accepted: ") + (window.state.pending ? window.state.pending.accepted : 0) }
            PlainLabel { text: window.tr("Отклонено: ", "Rejected: ") + (window.state.pending ? window.state.pending.rejected : 0) }
            PlainLabel { text: window.tr("Требуют внимания: ", "Flagged: ") + (window.state.pending ? window.state.pending.flagged : 0) }
            PlainLabel { Layout.fillWidth: true; color: "#75431e"; visible: !!window.state.pending && !!window.state.pending.native_json; text: window.tr("Источник содержит конфигурацию ядра JSON. Её содержимое не показывается: она может содержать секреты. Подтверждайте импорт только из доверенного источника.", "The source contains native engine JSON configuration. Its contents are hidden because they may contain secrets. Approve only a trusted source.") }
            RowLayout {
                Item { Layout.fillWidth: true }
                Button { text: window.tr("Отменить импорт", "Discard import"); enabled: !window.state.busy; onClicked: window.send({command: "discard_refresh"}) }
                Button { visible: window.state.can_stop; text: window.tr("Отключить", "Disconnect"); onClicked: window.send({command: "stop"}) }
                Button { text: window.tr("Сохранить профили", "Save profiles"); highlighted: true; enabled: !!window.state.pending && !window.state.busy; onClicked: window.send({command: "approve_refresh"}) }
            }
        }
    }
}
