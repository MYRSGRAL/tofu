pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import cloud.highpriority.tofu

ApplicationWindow {
    id: window
    width: 640
    height: 780
    minimumWidth: 520
    minimumHeight: 640
    visible: true
    title: "tofu"
    objectName: "tofuWindow"
    color: "#070707"
    palette.window: "#070707"
    palette.windowText: "#f3f4f6"
    palette.text: "#f3f4f6"
    palette.buttonText: "#f3f4f6"
    palette.highlight: "#c3a6ef"
    palette.base: "#191919"
    palette.button: "#191919"
    palette.mid: "#29262e"
    palette.placeholderText: "#9ba1ab"
    palette.highlightedText: "#070707"
    font.family: bodyFont.status === FontLoader.Ready ? bodyFont.name : "Noto Sans"
    font.pixelSize: 14
    FontLoader { id: bodyFont; source: "qrc:/fonts/Onest.ttf" }
    FontLoader { id: displayFont; source: "qrc:/fonts/Unbounded.ttf" }
    readonly property color surface: "#191919"
    readonly property color line: "#29262e"
    readonly property bool editable: state.ready && !state.busy && !state.can_stop && !state.closed
    readonly property string actionText: state.can_stop
        ? (state.status === "CleanupError" ? tr("Повторить остановку", "Retry stopping")
           : (["Preparing", "Starting", "Checking"].indexOf(state.status) >= 0 ? tr("Отменить подключение", "Cancel connection") : tr("Отключить", "Disconnect")))
        : tr("Подключить", "Connect")
    property string language: "ru"
    property int page: 0
    property string parseError: ""
    property var state: ({ready: false, busy: true, error: null, pending: null,
        sources: [], sources_total: 0, sources_offset: 0, source_id: null, profiles: [], total: 0, offset: 0,
        revision: 0, status: "Disconnected", selected_profile: null,
        probe_custom: false, probe_expected_status: 204, probe_result: null,
        tcp_busy: false, tcp_name: null, tcp_result: null,
        tcp_invalidated: false, network_observed: false,
        network_routes: false, network_resolver: false, network_manager: false,
        selected_name: "", engine: "xray", active_name: "", active_engine: "", port: 10808, can_connect: false, can_stop: false})
    readonly property bool canEditProbe: state.ready && !state.busy && !state.can_stop
    readonly property color ink: "#f3f4f6"
    readonly property color muted: "#9ba1ab"
    readonly property color accent: "#c3a6ef"

    function tr(ru, en) { return language === "ru" ? ru : en }
    function send(command) { if (!state.closed) backend.dispatch(JSON.stringify(command)) }
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
    function probeResultText(result) {
        if (!result) return ""
        const kinds = {
            http: ["Ответ HTTPS", "HTTPS response"],
            tls: ["Ошибка TLS", "TLS error"],
            timeout: ["Истекло время проверки", "Check timed out"],
            transport: ["Ошибка транспорта", "Transport error"]
        }
        const pair = kinds[result.kind]
        return (pair ? tr(pair[0], pair[1]) : tr("Результат проверки", "Check result"))
            + (result.http_status !== null && result.http_status !== undefined ? " · HTTP " + result.http_status : "")
            + (result.elapsed_ms !== null && result.elapsed_ms !== undefined ? " · " + result.elapsed_ms + tr(" мс", " ms") : "")
    }
    function tcpStatusText(status) {
        const values = {
            reachable: ["TCP доступен", "TCP reachable"],
            not_applicable: ["Неприменимо: UDP или неизвестный транспорт", "Not applicable: UDP or unknown transport"],
            resolver_unavailable: ["DNS недоступен: требуется systemd-resolved с Varlink", "DNS unavailable: systemd-resolved with Varlink is required"],
            dns_failed: ["Не удалось разрешить DNS-имя", "DNS resolution failed"],
            connect_failed: ["Не удалось установить TCP-соединение", "TCP connection failed"],
            dns_timeout: ["Истекло время DNS-запроса", "DNS timed out"],
            connect_timeout: ["Истекло время TCP-соединения", "TCP connection timed out"],
            cancelled: ["TCP-проверка отменена", "TCP check cancelled"]
        }
        const pair = values[status]
        return pair ? tr(pair[0], pair[1]) : tr("Результат недоступен", "Result unavailable")
    }
    function tcpTimingText(result) {
        if (!result) return ""
        const dns = result.dns_ms !== null && result.dns_ms !== undefined
            ? "DNS: " + result.dns_ms + tr(" мс", " ms")
            : (["reachable", "connect_failed", "connect_timeout"].indexOf(result.status) >= 0
                ? tr("DNS не потребовался", "DNS not needed") : "DNS: —")
        const tcp = result.connect_ms !== null && result.connect_ms !== undefined
            ? tr("TCP, медиана: ", "TCP median: ") + result.connect_ms + tr(" мс", " ms") : "TCP: —"
        const samples = result.samples_ms ? result.samples_ms.length : 0
        return dns + " · " + tcp + " · " + tr("выборки: ", "samples: ") + samples + "/" + result.samples_requested
    }
    function tcpEndpointText(result) {
        if (!result) return ""
        const host = result.host.indexOf(":") >= 0 ? "[" + result.host + "]" : result.host
        return result.name + " · " + host + ":" + result.port
            + (result.selected_address ? tr(" · адрес: ", " · address: ") + result.selected_address : "")
    }
    function tcpCheckedAtText(result) {
        if (!result) return ""
        if (!result.checked_at_ms) return tr("Время проверки недоступно", "Check time unavailable")
        return tr("Проверено: ", "Checked: ")
            + new Date(result.checked_at_ms).toLocaleString(Qt.locale(language === "ru" ? "ru_RU" : "en_US"))
    }
    function networkObservationText() {
        const available = tr("доступно", "available")
        const unavailable = tr("недоступно", "unavailable")
        return tr("Наблюдение за изменениями: маршруты — ", "Change monitoring: routes — ")
            + (state.network_routes ? available : unavailable)
            + "; DNS — " + (state.network_resolver ? available : unavailable)
            + "; NetworkManager — " + (state.network_manager ? available : unavailable)
    }
    function reviewText(row) {
        const reasons = {
            malformed_uri: ["Некорректная ссылка", "Malformed URI"],
            unsupported_protocol: ["Протокол не поддерживается", "Unsupported protocol"],
            invalid_port: ["Некорректный порт", "Invalid port"],
            invalid_host: ["Некорректный адрес", "Invalid host"],
            invalid_credentials: ["Некорректные учётные данные", "Invalid credentials"],
            invalid_encoding: ["Некорректная кодировка", "Invalid encoding"],
            invalid_json: ["Некорректный JSON", "Invalid JSON"],
            too_large: ["Превышен размер записи", "Record too large"],
            duplicate_parameter: ["Повторяющийся параметр", "Duplicate parameter"],
            conflicting_parameter: ["Конфликт параметров", "Conflicting parameters"],
            unknown_parameter: ["Неизвестный параметр", "Unknown parameter"],
            unsupported_transport: ["Транспорт не поддерживается", "Unsupported transport"],
            unsupported_security: ["Защита не поддерживается", "Unsupported security"],
            unsupported_cipher: ["Шифр не поддерживается", "Unsupported cipher"],
            unsupported_option: ["Опция не поддерживается", "Unsupported option"],
            unsafe_option: ["Небезопасная опция", "Unsafe option"],
            legacy_option: ["Устаревшая опция", "Legacy option"]
        }
        const fields = {
            source: ["источник", "source"], endpoint: ["адрес", "endpoint"],
            credentials: ["учётные данные", "credentials"],
            parameters: ["параметры", "parameters"], payload: ["содержимое", "payload"]
        }
        const reason = reasons[row.code]
        const field = fields[row.field]
        return (row.kind === "rejected" ? tr("Строка ", "Line ") : tr("Принятый профиль ", "Accepted profile "))
            + row.position + ": " + (reason ? tr(reason[0], reason[1]) : tr("Требует проверки", "Review required"))
            + (field ? " (" + tr(field[0], field[1]) + ")" : "")
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
            probe_target_invalid: ["Укажите корректный HTTPS URL (до 2048 символов) и ожидаемый статус 200–299.", "Enter a valid HTTPS URL (up to 2048 characters) and an expected status of 200–299."],
            check_http_status: ["Сайт проверки вернул неожиданный HTTP-статус. Редирект не считается успехом.", "The check endpoint returned an unexpected HTTP status. Redirects are not successful."],
            check_tls: ["Не удалось проверить TLS сайта проверки.", "TLS verification failed for the check endpoint."],
            check_timeout: ["Истекло время проверки доступа через прокси.", "The connectivity check through the proxy timed out."],
            check_transport: ["Не удалось выполнить запрос через прокси: ошибка транспорта.", "The request through the proxy failed with a transport error."],
            check_failed: ["Прокси работает, но доступность интернета не подтверждена.", "The proxy is running, but internet access is unconfirmed."],
            network_changed: ["Обнаружено изменение сети. Прежняя HTTPS-проверка больше не актуальна; прокси-ядро продолжает работать с ограниченным статусом. Переподключитесь для новой HTTPS-проверки.", "A network change was detected. The previous HTTPS check is no longer current; the proxy engine keeps running with limited status. Reconnect for a new HTTPS check."],
            tcp_failed: ["Не удалось выполнить TCP-проверку. Повторите попытку.", "Could not complete the TCP check. Try again."],
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
    Shortcut {
        sequence: "Escape"
        enabled: window.page !== 0 && !addDialog.visible && !reviewDialog.visible
        onActivated: window.page = 0
    }

    component PlainLabel: Label {
        textFormat: Text.PlainText
        color: window.ink
        wrapMode: Text.WordWrap
    }
    component SectionTitle: PlainLabel {
        font.pixelSize: 22
        font.family: displayFont.status === FontLoader.Ready ? displayFont.name : window.font.family
        font.weight: Font.DemiBold
    }
    component SmallLabel: PlainLabel {
        color: window.muted
        font.pixelSize: 13
    }


    component ActionButton: Button {
        id: control
        implicitHeight: 44
        leftPadding: 16
        rightPadding: 16
        Accessible.name: text
        Keys.onReturnPressed: if (enabled) clicked()
        Keys.onEnterPressed: if (enabled) clicked()
        contentItem: PlainLabel {
            text: control.text
            font: control.font
            color: !control.enabled ? window.muted : (control.highlighted ? "#070707" : window.ink)
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: 10
            color: control.highlighted && control.enabled ? window.accent : (control.hovered ? "#242127" : window.surface)
            border.width: control.activeFocus ? 2 : 1
            border.color: control.activeFocus ? window.accent : window.line
            opacity: control.enabled ? 1 : 0.55
        }
    }
    component Divider: Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: window.line }
    component DarkDialog: Dialog {
        id: dialog
        padding: 20
        background: Rectangle { color: window.surface; radius: 16; border.color: window.line }
        header: PlainLabel {
            text: dialog.title
            font.pixelSize: 19
            font.weight: Font.DemiBold
            leftPadding: 20; rightPadding: 20; topPadding: 20; bottomPadding: 8
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: window.width < 600 ? 24 : 36
        spacing: 20
        RowLayout {
            Layout.fillWidth: true
            ActionButton {
                objectName: "homeButton"
                visible: window.page !== 0
                text: "‹"
                implicitWidth: 44
                Accessible.name: window.tr("На главный экран", "Home")
                onClicked: window.page = 0
            }
            RowLayout {
                spacing: 10
                Rectangle {
                    implicitWidth: 25; implicitHeight: 25; radius: 7; rotation: -8
                    color: "transparent"; border.color: window.accent; border.width: 2
                    PlainLabel { anchors.centerIn: parent; text: "··"; color: window.accent; font.bold: true; font.pixelSize: 22 }
                }
                PlainLabel {
                    text: "tofu"; font.pixelSize: 27
                    font.family: displayFont.status === FontLoader.Ready ? displayFont.name : window.font.family
                    font.weight: Font.DemiBold
                }
            }
            Item { Layout.fillWidth: true }
            ActionButton {
                objectName: "settingsButton"
                text: "⚙"; implicitWidth: 44; font.pixelSize: 22
                Accessible.name: window.tr("Настройки", "Settings")
                onClicked: window.page = 2
            }
        }
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: window.page
            Item {
                ColumnLayout {
                    anchors.centerIn: parent
                    width: Math.min(parent.width, 360)
                    spacing: 0
                    ActionButton {
                        id: serverCard
                        objectName: "serverPickerButton"
                        Layout.fillWidth: true
                        implicitHeight: 88
                        enabled: window.state.ready && !window.state.closed
                        Accessible.name: window.tr("Выбрать сервер", "Choose server") + ": " + serverName.text
                        onClicked: window.page = 1
                        contentItem: RowLayout {
                            spacing: 16
                            PlainLabel { text: "◎"; color: window.accent; font.pixelSize: 30 }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 5
                                PlainLabel {
                                    id: serverName
                                    objectName: "mainServerName"
                                    Layout.fillWidth: true
                                    text: (window.state.can_stop ? window.state.active_name : window.state.selected_name) || window.tr("Выберите сервер", "Choose a server")
                                    maximumLineCount: 1; elide: Text.ElideRight; font.weight: Font.Medium; font.pixelSize: 16
                                }
                                SmallLabel { text: window.state.can_stop ? window.tr("Активное подключение", "Active connection") : window.tr("Ваш следующий маршрут", "Your next route") }
                            }
                            PlainLabel { text: "›"; color: window.muted; font.pixelSize: 25 }
                        }
                    }
                    Item { Layout.preferredHeight: 38 }
                    Button {
                        id: powerButton
                        objectName: "mainConnectButton"
                        Layout.alignment: Qt.AlignHCenter
                        implicitWidth: 186; implicitHeight: 186
                        enabled: window.state.ready && !window.state.closed && (window.state.can_stop || window.state.can_connect)
                        Accessible.name: window.actionText
                        text: window.actionText
                        onClicked: window.send({command: window.state.can_stop ? "stop" : "connect"})
                        Keys.onReturnPressed: if (enabled) clicked()
                        Keys.onEnterPressed: if (enabled) clicked()
                        background: Rectangle {
                            radius: width / 2; color: "transparent"
                            border.color: powerButton.activeFocus ? window.accent : window.line
                            border.width: powerButton.activeFocus ? 3 : 1
                            Rectangle {
                                anchors.fill: parent; anchors.margins: 10; radius: width / 2
                                color: window.state.can_stop ? window.surface : window.accent
                                opacity: powerButton.enabled ? (powerButton.down ? 0.7 : 1) : 0.35
                                border.color: window.state.can_stop ? window.accent : "transparent"
                            }
                        }
                        contentItem: Item {
                            Canvas {
                                anchors.centerIn: parent; width: 56; height: 56
                                property color stroke: window.state.can_stop ? window.accent : "#070707"
                                onStrokeChanged: requestPaint()
                                onPaint: {
                                    const ctx = getContext("2d"); ctx.reset(); ctx.strokeStyle = stroke; ctx.lineWidth = 3; ctx.lineCap = "round"
                                    ctx.beginPath(); ctx.arc(28, 29, 20, -Math.PI / 2 + 0.6, 3 * Math.PI / 2 - 0.6); ctx.stroke()
                                    ctx.beginPath(); ctx.moveTo(28, 5); ctx.lineTo(28, 28); ctx.stroke()
                                }
                            }
                        }
                    }
                    PlainLabel {
                        objectName: "mainStatusLabel"
                        Layout.topMargin: 25; Layout.fillWidth: true
                        horizontalAlignment: Text.AlignHCenter; font.pixelSize: 23; font.weight: Font.Medium
                        text: !window.state.ready ? window.tr("Инициализация…", "Initializing…") : window.statusText(window.state.status)
                        color: ["Limited", "CleanupError", "Error"].indexOf(window.state.status) >= 0 ? "#e6bc83" : window.ink
                        Accessible.role: Accessible.StaticText
                    }
                    SmallLabel {
                        objectName: "mainErrorLabel"
                        Layout.fillWidth: true; Layout.topMargin: 9
                        horizontalAlignment: Text.AlignHCenter
                        text: window.parseError || (window.state.error ? window.errorText(window.state.error)
                            : window.state.status === "Limited" ? window.tr("Доступ к интернету не подтверждён", "Internet access is unconfirmed")
                            : window.state.status === "CleanupError" ? window.tr("Повторите остановку перед выходом", "Retry stopping before quitting")
                            : window.state.can_stop ? window.actionText
                            : window.state.selected_name ? window.tr("Нажмите, чтобы подключиться", "Press to connect") : window.tr("Добавьте подписку и выберите сервер", "Add a subscription and choose a server"))
                    }
                }
                SmallLabel {
                    anchors.bottom: parent.bottom; anchors.horizontalCenter: parent.horizontalCenter
                    text: window.tr("Локальный SOCKS5 · только для настроенных приложений", "Local SOCKS5 · for configured applications only")
                    font.pixelSize: 11
                }
            }
            ColumnLayout {
                spacing: 14
                SectionTitle { text: window.tr("Серверы", "Servers") }
                    RowLayout {
                        PlainLabel { text: window.tr("Профили", "Profiles"); font.pixelSize: 18; font.weight: Font.DemiBold; Layout.fillWidth: true }
                        ComboBox {
                            id: overviewSources
                            objectName: "profileSourceFilter"
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
                            onActivated: window.send({command: "select_source", id: currentValue})
                            enabled: window.state.ready && !window.state.busy && count > 0
                            Accessible.name: window.tr("Подписка профилей", "Profile subscription")
                        }
                    }
                    RowLayout {
                        TextField {
                            id: search
                            objectName: "profileSearch"
                            Layout.fillWidth: true
                            placeholderText: window.tr("Имя или адрес сервера", "Server name or address")
                            maximumLength: 256
                            selectByMouse: true
                            enabled: window.state.ready && !window.state.busy
                            onAccepted: window.query(0)
                            Accessible.name: window.tr("Поиск профилей", "Search profiles")
                        }
                        ActionButton { objectName: "profileSearchButton"; text: window.tr("Найти", "Search"); enabled: window.state.ready && !window.state.busy; onClicked: window.query(0) }
                        CheckBox { id: favorites; objectName: "favoritesFilter"; text: window.tr("Избранное", "Favorites"); enabled: window.state.ready && !window.state.busy; onClicked: window.query(0) }
                    }
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: "#191919"
                        border.color: "#29262e"
                        radius: 6
                        Layout.minimumHeight: 160
                        Layout.preferredHeight: 220
                        ListView {
                            id: profileList
                            objectName: "profileList"
                            anchors.fill: parent
                            anchors.margins: 1
                            clip: true
                            model: window.state.profiles
                            boundsBehavior: Flickable.StopAtBounds
                            ScrollBar.vertical: ScrollBar {}
                            delegate: ItemDelegate {
                                id: profileRow
                                objectName: "profileSelect_" + modelData.id
                                required property var modelData
                                required property int index
                                width: ListView.view.width
                                height: 66
                                highlighted: window.state.selected_profile === modelData.id
                                background: Rectangle { color: profileRow.highlighted ? "#302638" : (profileRow.hovered ? "#242127" : "#191919"); border.color: profileRow.activeFocus ? window.accent : "transparent"; radius: 4 }
                                enabled: window.state.ready && !window.state.busy
                                Accessible.name: modelData.name + ", " + modelData.host
                                onClicked: { window.send({command: "select_profile", id: modelData.id, revision: window.state.revision}); window.page = 0 }
                                Keys.onReturnPressed: if (enabled) clicked()
                                Keys.onEnterPressed: if (enabled) clicked()
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
                                        objectName: "favorite_" + profileRow.modelData.id
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
                            ActionButton { Layout.alignment: Qt.AlignHCenter; text: window.state.sources.length === 0 ? window.tr("Добавить подписку", "Add subscription") : window.tr("Открыть подписки", "Open subscriptions"); onClicked: { window.page = 2; if (window.state.sources.length === 0) addDialog.open() } }
                        }
                    }
                    RowLayout {
                        SmallLabel { text: window.state.total > 0 ? (window.state.offset + 1) + "–" + (window.state.offset + window.state.profiles.length) + " / " + window.state.total : window.tr("Нет профилей", "No profiles"); Layout.fillWidth: true }
                        ActionButton { objectName: "profilePreviousButton"; text: window.tr("Назад", "Previous"); enabled: window.state.offset > 0 && !window.state.busy; onClicked: window.query(Math.max(0, window.state.offset - 200)) }
                        ActionButton { objectName: "profileNextButton"; text: window.tr("Далее", "Next"); enabled: window.state.offset + window.state.profiles.length < window.state.total && !window.state.busy; onClicked: window.query(window.state.offset + 200) }
                    }

            }
            ScrollView {
                id: settingsScroll
                objectName: "settingsScroll"
                clip: true
                contentWidth: availableWidth
                ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                ColumnLayout {
                    width: settingsScroll.availableWidth
                    spacing: 16
                    PlainLabel { Layout.fillWidth: true; visible: !!window.state.error || !!window.parseError; text: window.parseError || window.errorText(window.state.error); color: "#e6bc83" }
                        SectionTitle { text: window.tr("Настройки", "Settings") }

                        PlainLabel { text: window.tr("Подключение", "Connection"); font.pixelSize: 18; font.weight: Font.DemiBold }
                        SmallLabel { Layout.fillWidth: true; text: window.tr("Ядро, порт, проверка и язык действуют в текущем сеансе приложения.", "Engine, port, check and language apply to this application session.") }
                        RowLayout {
                            Layout.fillWidth: true
                            PlainLabel { text: window.tr("Прокси-ядро", "Proxy engine"); Layout.fillWidth: true }
                            ComboBox { id: engineSelect; objectName: "engineChoice"; model: ["xray", "sing-box"]; currentIndex: window.state.engine === "sing-box" ? 1 : 0; enabled: window.editable; onActivated: window.send({command: "set_engine", engine: currentText}); Accessible.name: window.tr("Прокси-ядро", "Proxy engine") }
                        }
                        PlainLabel { text: window.tr("Локальный порт", "Local port") }
                        RowLayout {
                            SpinBox { id: listenPort; objectName: "listenPort"; from: 1; to: 65535; value: window.state.port; editable: true; enabled: window.editable; Accessible.name: window.tr("Локальный порт SOCKS5", "Local SOCKS5 port") }
                            ActionButton { objectName: "applyListenPort"; text: window.tr("Применить порт", "Apply port"); enabled: window.editable && listenPort.value >= 1 && listenPort.value <= 65535; onClicked: window.send({command: "set_listen_port", port: listenPort.value}) }
                        }
                        SmallLabel { Layout.fillWidth: true; text: window.tr("Порт применяется при следующем подключении. Для изменения сначала отключите прокси.", "The port applies on the next connection. Disconnect the proxy before editing.") }
                        Divider {}
                        GridLayout {
                            columns: window.width < 580 ? 1 : 2
                            columnSpacing: 30
                            rowSpacing: 20
                            PlainLabel { text: window.tr("Язык интерфейса", "Interface language") }
                            ComboBox { objectName: "languageChoice"; model: ["Русский", "English"]; currentIndex: window.language === "ru" ? 0 : 1; onActivated: window.language = currentIndex === 0 ? "ru" : "en"; Accessible.name: window.tr("Язык интерфейса", "Interface language") }
                            PlainLabel { text: window.tr("Режим подключения", "Connection mode") }
                            PlainLabel { text: window.tr("Локальный SOCKS5", "Local SOCKS5") }
                            PlainLabel { text: window.tr("Адрес прокси", "Proxy address") }
                            PlainLabel { text: "127.0.0.1:" + window.state.port }
                        }
                        Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: "#29262e" }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    RowLayout {
                        SectionTitle { text: window.tr("Подписки", "Subscriptions"); Layout.fillWidth: true }
                        ActionButton { objectName: "addSourceButton"; text: window.tr("Добавить", "Add"); highlighted: true; enabled: window.state.ready && !window.state.busy; onClicked: addDialog.open() }
                    }
                    SmallLabel { Layout.fillWidth: true; text: window.tr("Ссылки хранятся локально и не показываются в списках. Обновление применяется после проверки импорта.", "Links are stored locally and are hidden from lists. Updates are applied after import review.") }
                    ListView {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 190
                        clip: true
                        model: window.state.sources
                        spacing: 4
                        ScrollBar.vertical: ScrollBar {}
                        delegate: ItemDelegate {
                            id: subscriptionRow
                            objectName: "sourceSelect_" + modelData.id
                            required property var modelData
                            width: ListView.view.width
                            height: 64
                            text: modelData.name
                            highlighted: window.state.source_id === modelData.id
                            background: Rectangle { color: subscriptionRow.highlighted ? "#302638" : (subscriptionRow.hovered ? "#242127" : window.surface); radius: 8; border.color: subscriptionRow.activeFocus ? window.accent : "transparent" }
                            enabled: window.state.ready && !window.state.busy
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
                        ActionButton {
                            text: window.tr("Назад", "Previous")
                            enabled: window.state.ready && !window.state.busy && window.state.sources_offset > 0
                            onClicked: window.send({command: "source_page", offset: Math.max(0, window.state.sources_offset - 64)})
                        }
                        ActionButton {
                            text: window.tr("Далее", "Next")
                            enabled: window.state.ready && !window.state.busy && window.state.sources_offset + window.state.sources.length < window.state.sources_total
                            onClicked: window.send({command: "source_page", offset: window.state.sources_offset + 64})
                        }
                    }
                    RowLayout {
                        SmallLabel { text: window.tr("Выберите подписку для обновления", "Select a subscription to refresh"); Layout.fillWidth: true }
                        ActionButton { objectName: "refreshSourceButton"; text: window.tr("Обновить", "Refresh"); enabled: window.state.ready && window.state.source_id !== null && !window.state.busy && !window.state.pending; onClicked: window.send({command: "refresh"}) }
                    }
                }
                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: tcpLayout.implicitHeight + 24
                        color: "#191919"
                        border.color: "#29262e"
                        radius: 6
                        ColumnLayout {
                            id: tcpLayout
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 5
                            RowLayout {
                                Layout.fillWidth: true
                                PlainLabel {
                                    Layout.fillWidth: true
                                    text: window.tr("Доступность сервера по TCP", "Server TCP availability")
                                    font.weight: Font.DemiBold
                                }
                                ActionButton {
                                    objectName: "tcpCheckButton"
                                    text: window.tr("Проверить TCP", "Check TCP")
                                    enabled: window.state.ready && window.state.selected_profile !== null && !window.state.busy && !window.state.tcp_busy
                                    onClicked: window.send({command: "check_tcp"})
                                }
                                ActionButton {
                                    objectName: "tcpCancelButton"
                                    visible: !!window.state.tcp_busy
                                    text: window.tr("Отменить", "Cancel")
                                    enabled: !!window.state.tcp_busy
                                    onClicked: window.send({command: "cancel_tcp"})
                                }
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                visible: !!window.state.tcp_busy
                                text: window.tr("Проверяется: ", "Checking: ") + (window.state.tcp_name || "")
                                maximumLineCount: 2
                                elide: Text.ElideRight
                            }
                            PlainLabel {
                                Layout.fillWidth: true
                                visible: !!window.state.tcp_result
                                text: window.state.tcp_result ? window.tcpStatusText(window.state.tcp_result.status) : ""
                                font.weight: Font.Medium
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                visible: !!window.state.tcp_result
                                text: window.tcpEndpointText(window.state.tcp_result)
                                maximumLineCount: 2
                                elide: Text.ElideRight
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                visible: !!window.state.tcp_result
                                text: window.tcpTimingText(window.state.tcp_result)
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                visible: !!window.state.tcp_result
                                text: window.tcpCheckedAtText(window.state.tcp_result)
                            }
                            PlainLabel {
                                Layout.fillWidth: true
                                visible: !!window.state.tcp_invalidated
                                text: window.tr("Обнаружено изменение сети: TCP-проверка отменена, прежний результат сброшен. Запустите проверку заново вручную.", "A network change was detected: the TCP check was cancelled and the previous result cleared. Run a new check manually.")
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                visible: !!window.state.tcp_result && !window.state.tcp_result.network_observed
                                text: window.tr("Во время TCP-проверки наблюдение за изменениями сети было неполным. Её результат относится к моменту проверки.", "Network change monitoring was incomplete during the TCP check. Its result applies to the time of the check.")
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                visible: window.state.ready
                                font.pixelSize: 11
                                text: window.networkObservationText()
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                visible: window.state.ready && !window.state.network_observed
                                text: window.tr("Часть изменений сети может не отслеживаться. При смене сети повторите TCP-проверку вручную; для новой HTTPS-проверки переподключитесь.", "Some network changes may go undetected. After a network change, rerun the TCP check manually; reconnect for a new HTTPS check.")
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                font.pixelSize: 11
                                text: window.tr("Напрямую через обычную сеть, без системного прокси. Серия из трёх проб; до 5 с на DNS и TCP вместе. Без внешнего DNS-резерва.", "Direct over the regular network, without the system proxy. A series of three samples; up to 5 s for DNS and TCP combined. No external DNS fallback.")
                            }
                            SmallLabel {
                                Layout.fillWidth: true
                                font.pixelSize: 11
                                text: window.tr("Не проверяет аутентификацию, HTTPS или скорость. История пока не сохраняется. Автоматических проверок и переподключений нет.", "Does not test authentication, HTTPS or throughput. History is not saved yet. No automatic checks or reconnections.")
                            }
                        }
                    }
                        PlainLabel { text: window.tr("Проверка доступа", "Connectivity check"); font.pixelSize: 18; font.weight: Font.DemiBold }
                        SmallLabel {
                            Layout.fillWidth: true
                            text: (window.state.probe_custom ? window.tr("Применена своя проверка", "Custom check applied") : window.tr("Стандартная проверка: gstatic", "Default check: gstatic"))
                                + window.tr(" · ожидается HTTP ", " · expected HTTP ") + window.state.probe_expected_status
                        }
                        SmallLabel {
                            objectName: "probeResultLabel"
                            Layout.fillWidth: true
                            visible: window.state.can_stop && !!window.state.probe_result
                            text: window.state.probe_result ? window.tr("Проверка активной сессии: ", "Active session check: ") + window.probeResultText(window.state.probe_result) : ""
                        }
                        SmallLabel {
                            objectName: "probeNetworkWarning"
                            Layout.fillWidth: true
                            visible: window.state.can_stop && !!window.state.probe_result && !window.state.probe_result.network_observed
                            text: window.tr("Во время HTTPS-проверки наблюдение за изменениями сети было неполным. Её результат относится к моменту проверки.", "Network change monitoring was incomplete during the HTTPS check. Its result applies to the time of the check.")
                        }
                        PlainLabel { text: window.tr("HTTPS URL новой проверки", "New check HTTPS URL") }
                        RowLayout {
                            Layout.fillWidth: true
                            TextField {
                                id: probeUrl
                                objectName: "probeUrlInput"
                                Layout.fillWidth: true
                                maximumLength: 2048
                                placeholderText: "https://www.gstatic.com/generate_204"
                                echoMode: revealProbe.checked ? TextInput.Normal : TextInput.Password
                                selectByMouse: true
                                enabled: window.canEditProbe
                                inputMethodHints: Qt.ImhUrlCharactersOnly | Qt.ImhNoPredictiveText | Qt.ImhSensitiveData
                                Accessible.name: window.tr("Секретный HTTPS URL проверки", "Secret check HTTPS URL")
                            }
                            CheckBox { id: revealProbe; objectName: "revealProbeUrl"; text: window.tr("Показать", "Show"); enabled: window.canEditProbe }
                        }
                        RowLayout {
                            PlainLabel { text: window.tr("Ожидаемый HTTP-статус", "Expected HTTP status") }
                            SpinBox {
                                id: probeExpectedStatus
                                objectName: "probeExpectedStatus"
                                from: 200
                                to: 299
                                value: 204
                                enabled: window.canEditProbe
                                Accessible.name: window.tr("Ожидаемый HTTP-статус проверки", "Expected check HTTP status")
                            }
                        }
                        RowLayout {
                            ActionButton {
                                objectName: "applyProbeTargetButton"
                                text: window.tr("Применить", "Apply")
                                enabled: window.canEditProbe && probeUrl.text.trim().length > 0
                                onClicked: {
                                    window.send({command: "set_probe_target", url: probeUrl.text.trim(), expected_status: probeExpectedStatus.value})
                                    probeUrl.clear()
                                    revealProbe.checked = false
                                }
                            }
                            ActionButton {
                                objectName: "resetProbeTargetButton"
                                text: window.tr("Сбросить проверку", "Reset check")
                                enabled: window.canEditProbe
                                onClicked: {
                                    window.send({command: "reset_probe_target"})
                                    probeUrl.clear()
                                    revealProbe.checked = false
                                    probeExpectedStatus.value = 204
                                }
                            }
                        }
                        SmallLabel {
                            Layout.fillWidth: true
                            text: window.tr("Настройка действует в этом сеансе приложения со следующего подключения. Для изменения сначала отключите прокси. Повторная проверка подключённой сессии пока недоступна. URL может содержать секрет: после отправки поле очищается.", "This setting applies for this application session from the next connection. Disconnect the proxy before editing. Rechecking a connected session is not available yet. The URL may contain a secret: the field is cleared after sending.")
                        }
                        SmallLabel {
                            Layout.fillWidth: true
                            text: window.tr("HTTPS-запрос идёт через выбранный outbound локального SOCKS5. TLS проверяется; редиректы не считаются успехом. Время включает весь запрос и не измеряет пропускную способность. Ошибка транспорта пока не разделяет DNS, соединение и туннель.", "The HTTPS request uses the selected outbound through local SOCKS5. TLS is verified; redirects are not successful. Timing covers the whole request and does not measure throughput. Transport errors do not yet distinguish DNS, connection and tunnel failures.")
                        }
                        Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: "#29262e" }
                        SmallLabel { Layout.fillWidth: true; text: window.tr("Эта версия предоставляет локальный прокси. TUN, системный VPN и редактор сетевых политик ещё не доступны. Язык выбирается на текущий сеанс.", "This version provides a local proxy. TUN, system VPN and the routing policy editor are not available yet. Language applies to this session.") }
                        SmallLabel { Layout.fillWidth: true; text: window.tr("Если трей доступен, закрытие окна скрывает приложение в нём. Для выхода используйте меню трея. Без трея закрытие отключает прокси и завершает приложение.", "When a tray is available, closing hides the window there. Use the tray menu to quit. Without a tray, closing disconnects the proxy and exits.") }
                        ActionButton { text: window.tr("Перечитать состояние", "Reload state"); enabled: window.state.ready && !window.state.busy; onClicked: window.send({command: "reload"}) }
                        SmallLabel { text: "tofu · " + window.tr("Открытый исходный код", "Open source") }

                    Item { Layout.preferredHeight: 16 }
                }
            }
        }
    }
    DarkDialog {
        id: addDialog
        objectName: "addSourceDialog"
        anchors.centerIn: parent
        width: Math.min(window.width - 64, 540)
        modal: true
        title: window.tr("Добавить подписку", "Add subscription")
        closePolicy: Popup.CloseOnEscape
        onClosed: { sourceName.clear(); sourceUrl.clear(); localException.checked = false; route.currentIndex = 0; reveal.checked = false }
        contentItem: ColumnLayout {
            spacing: 12
            PlainLabel { text: window.tr("Название", "Name") }
            TextField { id: sourceName; objectName: "sourceNameInput"; Layout.fillWidth: true; maximumLength: 128; selectByMouse: true; Accessible.name: window.tr("Название подписки", "Subscription name") }
            PlainLabel { text: window.tr("Ссылка подписки", "Subscription URL") }
            RowLayout {
                TextField { id: sourceUrl; objectName: "sourceUrlInput"; Layout.fillWidth: true; maximumLength: 8192; echoMode: reveal.checked ? TextInput.Normal : TextInput.Password; selectByMouse: true; inputMethodHints: Qt.ImhUrlCharactersOnly | Qt.ImhNoPredictiveText | Qt.ImhSensitiveData; Accessible.name: window.tr("Секретная ссылка подписки", "Secret subscription URL") }
                CheckBox { id: reveal; objectName: "revealSourceUrl"; text: window.tr("Показать", "Show") }
            }
            SmallLabel { Layout.fillWidth: true; text: window.tr("Ссылка может содержать токен доступа. Не передавайте её другим людям.", "The link may contain an access token. Keep it private.") }
            CheckBox { id: localException; objectName: "sourceLocalException"; Layout.fillWidth: true; text: window.tr("Разрешить HTTP и локальные / приватные адреса", "Allow HTTP and local / private addresses"); contentItem: PlainLabel { text: localException.text; leftPadding: localException.indicator.width + localException.spacing; verticalAlignment: Text.AlignVCenter } }
            SmallLabel { Layout.fillWidth: true; text: window.tr("Включайте исключение только для доверенного источника в своей сети.", "Enable this exception only for a trusted source in your own network.") }
            PlainLabel { text: window.tr("Маршрут загрузки", "Download route") }
            ComboBox { id: route; objectName: "sourceRouteChoice"; Layout.fillWidth: true; model: [window.tr("Напрямую", "Direct"), window.tr("Через прокси (пока недоступно)", "Proxy (not available yet)")]; Accessible.name: window.tr("Маршрут загрузки подписки", "Subscription download route") }
            SmallLabel { visible: route.currentIndex === 1; Layout.fillWidth: true; color: "#e6bc83"; text: window.tr("Загрузка через прокси пока не поддерживается. Backend отклонит этот маршрут.", "Proxy fetching is not supported yet. The backend will reject this route.") }
            RowLayout {
                Layout.topMargin: 10
                Item { Layout.fillWidth: true }
                ActionButton { objectName: "sourceAddCancelButton"; text: window.tr("Отмена", "Cancel"); onClicked: addDialog.close() }
                ActionButton {
                    objectName: "sourceAddConfirmButton"
                    text: window.tr("Добавить", "Add")
                    highlighted: true
                    enabled: window.state.ready && sourceName.text.trim().length > 0 && sourceUrl.text.trim().length > 0 && !window.state.busy
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
    DarkDialog {
        id: reviewDialog
        objectName: "refreshReviewDialog"
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
            PlainLabel { objectName: "nativeJsonWarning"; Layout.fillWidth: true; color: "#e6bc83"; visible: !!window.state.pending && !!window.state.pending.native_json; text: window.tr("Источник содержит конфигурацию ядра JSON. Её содержимое не показывается: она может содержать секреты. Подтверждайте импорт только из доверенного источника.", "The source contains native engine JSON configuration. Its contents are hidden because they may contain secrets. Approve only a trusted source.") }
            ListView {
                id: reviewRows
                Layout.fillWidth: true
                Layout.preferredHeight: visible ? 150 : 0
                visible: !!window.state.pending && window.state.pending.rows_total > 0
                clip: true
                spacing: 8
                model: window.state.pending ? window.state.pending.rows : []
                delegate: PlainLabel {
                    required property var modelData
                    width: reviewRows.width - 20
                    text: window.reviewText(modelData)
                }
                ScrollBar.vertical: ScrollBar { }
            }
            RowLayout {
                visible: !!window.state.pending && window.state.pending.rows_total > 100
                ActionButton {
                    text: window.tr("Назад", "Previous")
                    enabled: !!window.state.pending && window.state.pending.rows_offset > 0 && !window.state.busy
                    onClicked: { reviewRows.positionViewAtBeginning(); window.send({command: "review_page", offset: Math.max(0, window.state.pending.rows_offset - 100)}) }
                }
                SmallLabel { text: window.state.pending ? (window.state.pending.rows_offset + 1) + "–" + Math.min(window.state.pending.rows_offset + 100, window.state.pending.rows_total) + " / " + window.state.pending.rows_total : "" }
                ActionButton {
                    text: window.tr("Далее", "Next")
                    enabled: !!window.state.pending && window.state.pending.rows_offset + 100 < window.state.pending.rows_total && !window.state.busy
                    onClicked: { reviewRows.positionViewAtBeginning(); window.send({command: "review_page", offset: window.state.pending.rows_offset + 100}) }
                }
            }
            Flow {
                Layout.fillWidth: true
                spacing: 8
                layoutDirection: Qt.RightToLeft
                ActionButton { objectName: "approveRefreshButton"; text: window.tr("Сохранить профили", "Save profiles"); highlighted: true; enabled: !!window.state.pending && !window.state.busy; onClicked: window.send({command: "approve_refresh"}) }
                ActionButton { objectName: "reviewStopButton"; visible: window.state.can_stop; text: window.tr("Отключить", "Disconnect"); onClicked: window.send({command: "stop"}) }
                ActionButton { objectName: "discardRefreshButton"; text: window.tr("Отменить импорт", "Discard import"); enabled: !window.state.busy; onClicked: window.send({command: "discard_refresh"}) }
            }
        }
    }
}
