pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import cloud.highpriority.tofu

ApplicationWindow {
    id: window
    width: 520
    height: 800
    minimumWidth: 520
    minimumHeight: 640
    visible: true
    title: "tofu · " + (state.build_id || "local")
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
    property string settingsSection: ""
    property bool reducedMotion: false
    onPageChanged: {
        if (page !== 2 && addDialog.visible) addDialog.close()
    }
    property string parseError: ""
    property var state: ({ready: false, busy: true, error: null, pending: null,
        sources: [], sources_total: 0, sources_offset: 0, source_id: null, profiles: [], total: 0, offset: 0,
        revision: 0, status: "Disconnected", selected_profile: null,
        probe_custom: false, probe_expected_status: 204, probe_result: null,
        tcp_busy: false, tcp_name: null, tcp_result: null,
        tcp_invalidated: false, network_observed: false,
        network_routes: false, network_resolver: false, network_manager: false,
        selected_name: "", engine: "xray", active_name: "", active_engine: "", port: 10808, can_connect: false, can_stop: false})
    readonly property bool canEditProbe: state.ready && !state.busy && !state.can_stop && !state.closed
    readonly property color ink: "#f5f3f8"
    readonly property color muted: "#bdb4cd"
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
            if (typeof next.search === "string" && next.search !== state.search) search.text = next.search
            if (typeof next.favorites_only === "boolean" && next.favorites_only !== state.favorites_only) favorites.checked = next.favorites_only
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
            listen_port_busy: ["Порт занят. Повторите подключение.", "Port is busy. Try connecting again."],
            startup: ["Не удалось запустить приложение.", "Could not start the application."],
            keyring_locked: ["Хранилище секретов заблокировано. Разблокируйте системный кошелёк и повторите запуск.", "Secret storage is locked. Unlock the system wallet and restart."],
            keyring_missing: ["Системное хранилище секретов недоступно.", "System secret storage is unavailable."],
            storage: ["Ошибка локального хранилища. Проверьте доступ к данным приложения.", "Local storage error. Check access to application data."],
            invalid_input: ["Проверьте введённые данные и адрес подписки.", "Check the input and subscription URL."],
            busy: ["Дождитесь завершения текущей операции.", "Wait for the current operation to finish."],
            source_required: ["Сначала выберите подписку.", "Select a subscription first."],
            unsupported_proxy: ["Загрузка подписки через прокси пока не поддерживается. Выберите прямой маршрут.", "Fetching subscriptions through a proxy is not supported yet. Choose Direct."],
            hwid_required: ["Провайдер требует ID устройства. Включите его в настройках подписки.", "The provider requires a device ID. Enable it in subscription settings."],
            hwid_limit: ["Достигнут лимит устройств подписки. Проверьте устройства у провайдера.", "The subscription device limit has been reached. Review devices with your provider."],
            hwid_rejected: ["Провайдер отклонил ID устройства. Обратитесь в поддержку провайдера.", "The provider rejected the device ID. Contact provider support."],
            hwid_redirect: ["Провайдер перенаправляет запрос с ID устройства. Проверьте ссылку подписки.", "The provider redirects device ID requests. Check the subscription link."],
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
        onActivated: { if (window.page === 2 && window.settingsSection !== "") window.settingsSection = ""; else window.page = 0 }
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
        function activateFromKeyboard() {
            if (!enabled) return
            if (checkable) toggle()
            clicked()
        }
        Keys.onReturnPressed: event => { if (!event.isAutoRepeat) control.activateFromKeyboard() }
        Keys.onEnterPressed: event => { if (!event.isAutoRepeat) control.activateFromKeyboard() }
        contentItem: PlainLabel {
            text: control.text
            font: control.font
            color: !control.enabled ? window.muted : (control.highlighted ? "#070707" : window.ink)
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: 14
            property color hoverColor: control.down ? "#35283f" : control.hovered ? "#26212b" : window.surface
            color: control.highlighted && control.enabled ? window.accent : hoverColor
            border.width: control.activeFocus ? 2 : 1
            border.color: control.activeFocus ? window.accent : window.line
            opacity: control.enabled ? 1 : 0.55
            Behavior on hoverColor { ColorAnimation { duration: window.reducedMotion ? 0 : 160 } }
            Behavior on opacity { NumberAnimation { duration: window.reducedMotion ? 0 : 120 } }
        }
    }
    component SettingsRow: ActionButton {
        id: row
        property string description: ""
        property string symbol: ""
        Layout.fillWidth: true
        implicitHeight: 78
        leftPadding: 16; rightPadding: 16
        contentItem: RowLayout {
            spacing: 14
            Rectangle {
                implicitWidth: 40; implicitHeight: 40; radius: 12; color: "#2a2033"
                PlainLabel { anchors.centerIn: parent; text: row.symbol; color: window.accent; font.pixelSize: 21 }
            }
            ColumnLayout {
                Layout.fillWidth: true; spacing: 5
                PlainLabel { text: row.text; font.weight: Font.Medium; font.pixelSize: 15 }
                SmallLabel { text: row.description; Layout.fillWidth: true; font.pixelSize: 12 }
            }
            PlainLabel { text: "›"; color: window.muted; font.pixelSize: 24 }
        }
    }
    component HelpButton: ActionButton {
        id: help
        property string explanation: ""
        text: "?"
        implicitWidth: 32
        implicitHeight: 32
        leftPadding: 6
        rightPadding: 6
        Accessible.name: window.tr("Подробнее", "More information")
        Accessible.description: explanation
        ToolTip {
            parent: help
            visible: help.hovered || help.activeFocus || help.down
            delay: 200
            timeout: 15000
            width: Math.min(340, window.width - 80)
            contentItem: PlainLabel { text: help.explanation; font.pixelSize: 13 }
            background: Rectangle { color: "#302638"; border.color: "#675174"; radius: 10 }
        }
    }
    component PanelScroll: ScrollView {
        readonly property bool atStart: contentItem.atYBeginning
        readonly property bool atEnd: contentItem.atYEnd
        readonly property bool moving: contentItem.moving
        clip: true
        contentWidth: availableWidth
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
        Component.onCompleted: contentItem.boundsBehavior = Flickable.StopAtBounds
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
        anchors.margins: 28
        spacing: 24
        RowLayout {
            Layout.fillWidth: true
            ActionButton {
                objectName: "homeButton"
                visible: window.page !== 0
                text: "‹"
                implicitWidth: 44
                Accessible.name: window.tr("Назад", "Back")
                onClicked: { if (window.page === 2 && window.settingsSection !== "") window.settingsSection = ""; else window.page = 0 }
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
            Rectangle {
                visible: window.page === 0
                implicitWidth: modeLabel.implicitWidth + 20; implicitHeight: 28; radius: 14
                color: "#191919"
                SmallLabel { id: modeLabel; anchors.centerIn: parent; text: "SOCKS5"; font.pixelSize: 11; font.letterSpacing: 1 }
            }
            ActionButton {
                objectName: "settingsButton"
                text: "⚙"; implicitWidth: 44; font.pixelSize: 22
                Accessible.name: window.tr("Настройки", "Settings")
                onClicked: { window.settingsSection = ""; window.page = 2 }
            }
        }
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: window.page
            Item {
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
                    SmallLabel {
                        visible: window.height >= 720
                        text: window.tr("ПОДКЛЮЧЕНИЕ", "CONNECTION")
                        font.pixelSize: 11; font.letterSpacing: 2
                        Layout.bottomMargin: 12
                    }
                    ActionButton {
                        id: serverCard
                        objectName: "serverPickerButton"
                        Layout.fillWidth: true
                        implicitHeight: window.height < 720 ? 80 : 92
                        leftPadding: 18; rightPadding: 18
                        enabled: window.state.ready && !window.state.closed
                        Accessible.name: window.tr("Выбрать сервер", "Choose server") + ": " + serverName.text
                        onClicked: { favorites.checked = false; window.query(0); window.page = 1 }
                        background: Rectangle {
                            radius: 20
                            border.color: serverCard.activeFocus ? window.accent : serverCard.hovered ? "#755589" : "#493254"
                            border.width: serverCard.activeFocus ? 2 : 1
                            gradient: Gradient {
                                GradientStop { position: 0; color: serverCard.down ? "#352340" : "#2a1b36" }
                                GradientStop { position: 1; color: "#17101f" }
                            }
                            Behavior on border.color { ColorAnimation { duration: window.reducedMotion ? 0 : 160 } }
                        }
                        contentItem: RowLayout {
                            spacing: 14
                            Rectangle {
                                implicitWidth: 44; implicitHeight: 44; radius: 14; color: "#372544"
                                PlainLabel { anchors.centerIn: parent; text: "◎"; color: "#d9c6fc"; font.pixelSize: 28 }
                            }
                            ColumnLayout {
                                Layout.fillWidth: true; spacing: 5
                                SmallLabel { text: window.state.can_stop ? window.tr("Текущий сервер", "Current server") : window.tr("Сервер", "Server"); font.pixelSize: 11 }
                                PlainLabel {
                                    id: serverName
                                    objectName: "mainServerName"
                                    Layout.fillWidth: true
                                    text: (window.state.can_stop ? window.state.active_name : window.state.selected_name) || window.tr("Выберите сервер", "Choose a server")
                                    maximumLineCount: 1; elide: Text.ElideRight; font.weight: Font.Medium; font.pixelSize: 17
                                }
                            }
                            PlainLabel { text: "›"; color: "#d9c6fc"; font.pixelSize: 28 }
                        }
                    }
                    Item { Layout.fillHeight: true; Layout.minimumHeight: 24; Layout.maximumHeight: 64 }
                    Button {
                        id: powerButton
                        objectName: "mainConnectButton"
                        Layout.alignment: Qt.AlignHCenter
                        implicitWidth: window.height < 720 ? 184 : 224; implicitHeight: implicitWidth
                        scale: down ? 0.96 : hovered && enabled ? 1.025 : 1
                        Behavior on scale { NumberAnimation { duration: window.reducedMotion ? 0 : 160; easing.type: Easing.OutCubic } }
                        enabled: window.state.ready && !window.state.closed && (window.state.can_stop || window.state.can_connect)
                        Accessible.name: window.actionText
                        text: window.actionText
                        onClicked: window.send({command: window.state.can_stop ? "stop" : "connect"})
                        Keys.onReturnPressed: if (enabled) clicked()
                        Keys.onEnterPressed: if (enabled) clicked()
                        background: Rectangle {
                            radius: width / 2
                            color: "#17101f"
                            border.color: powerButton.activeFocus ? "#d9c6fc" : "#483253"
                            border.width: powerButton.activeFocus ? 3 : 1
                            Rectangle {
                                anchors.fill: parent; anchors.margins: -12
                                radius: width / 2; color: "transparent"; border.color: "#271c30"
                            }
                            Rectangle {
                                anchors.fill: parent; anchors.margins: 12; radius: width / 2
                                gradient: Gradient {
                                    GradientStop { position: 0; color: window.state.can_stop ? "#392746" : "#d9c6fc" }
                                    GradientStop { position: 1; color: window.state.can_stop ? "#1d1427" : "#b69ad9" }
                                }
                                border.color: window.state.can_stop ? "#c3a6ef" : "#e2d3f8"
                                opacity: powerButton.enabled ? (powerButton.down ? 0.8 : 1) : 0.65
                            }
                        }
                        contentItem: Item {
                            Canvas {
                                anchors.centerIn: parent; width: 64; height: 64
                                property color stroke: window.state.can_stop ? window.accent : "#070707"
                                onStrokeChanged: requestPaint()
                                onVisibleChanged: if (visible) requestPaint()
                                onPaint: {
                                    const ctx = getContext("2d"); ctx.reset(); ctx.strokeStyle = stroke; ctx.lineWidth = 3.5; ctx.lineCap = "round"
                                    ctx.beginPath(); ctx.arc(32, 33, 23, -Math.PI / 2 + 0.6, 3 * Math.PI / 2 - 0.6); ctx.stroke()
                                    ctx.beginPath(); ctx.moveTo(32, 5); ctx.lineTo(32, 31); ctx.stroke()
                                }
                            }
                        }
                    }
                    RowLayout {
                        Layout.alignment: Qt.AlignHCenter; Layout.topMargin: window.height < 720 ? 20 : 26; spacing: 9
                        Rectangle {
                            implicitWidth: 7; implicitHeight: 7; radius: 4
                            color: window.state.status === "Connected" ? "#a8d8bc" : ["Limited", "CleanupError", "Error"].indexOf(window.state.status) >= 0 ? "#e6bc83" : window.accent
                        }
                        PlainLabel {
                            objectName: "mainStatusLabel"
                            font.pixelSize: 22; font.weight: Font.Medium
                            text: !window.state.ready ? window.tr("Инициализация…", "Initializing…") : window.statusText(window.state.status)
                            color: ["Limited", "CleanupError", "Error"].indexOf(window.state.status) >= 0 ? "#e6bc83" : window.ink
                            Accessible.role: Accessible.StaticText
                        }
                    }
                    SmallLabel {
                        objectName: "mainErrorLabel"
                        maximumLineCount: 3; elide: Text.ElideRight
                        Layout.fillWidth: true; Layout.topMargin: window.height < 720 ? 8 : 10
                        horizontalAlignment: Text.AlignHCenter
                        text: window.parseError || (window.state.error ? window.errorText(window.state.error)
                            : window.state.status === "Limited" ? window.tr("Доступ к интернету не подтверждён", "Internet access is unconfirmed")
                            : window.state.status === "CleanupError" ? window.tr("Повторите остановку перед выходом", "Retry stopping before quitting")
                            : window.state.can_stop ? window.actionText
                            : window.state.selected_name ? window.tr("Нажмите, чтобы подключиться", "Press to connect") : window.tr("Добавьте подписку и выберите сервер", "Add a subscription and choose a server"))
                    }
                    Item { Layout.fillHeight: true; Layout.minimumHeight: window.height < 720 ? 12 : 20 }
                    ActionButton {
                        objectName: "homeFavoritesButton"
                        Layout.fillWidth: true; implicitHeight: window.height < 720 ? 56 : 64
                        enabled: window.state.ready && !window.state.busy
                        onClicked: { favorites.checked = true; window.query(0); window.page = 1 }
                        contentItem: RowLayout {
                            spacing: 12
                            PlainLabel { text: "☆"; color: window.accent; font.pixelSize: 25 }
                            PlainLabel { text: window.tr("Избранное", "Favorites"); Layout.fillWidth: true; font.weight: Font.Medium }
                            SmallLabel { text: window.tr("Серверы", "Servers") }
                            PlainLabel { text: "›"; color: window.muted; font.pixelSize: 24 }
                        }
                    }
                    SmallLabel {
                        Layout.fillWidth: true; Layout.topMargin: window.height < 720 ? 10 : 14
                        horizontalAlignment: Text.AlignHCenter
                        text: window.tr("Локальный прокси · SOCKS5", "Local proxy · SOCKS5"); font.pixelSize: 11
                    }
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
                            ActionButton { Layout.alignment: Qt.AlignHCenter; text: window.state.sources.length === 0 ? window.tr("Добавить подписку", "Add subscription") : window.tr("Открыть подписки", "Open subscriptions"); onClicked: { window.settingsSection = "subscriptions"; window.page = 2; if (window.state.sources.length === 0) addDialog.open() } }
                        }
                    }
                    RowLayout {
                        SmallLabel { text: window.state.total > 0 ? (window.state.offset + 1) + "–" + (window.state.offset + window.state.profiles.length) + " / " + window.state.total : window.tr("Нет профилей", "No profiles"); Layout.fillWidth: true }
                        ActionButton { objectName: "profilePreviousButton"; text: window.tr("Назад", "Previous"); enabled: window.state.offset > 0 && !window.state.busy; onClicked: window.query(Math.max(0, window.state.offset - 200)) }
                        ActionButton { objectName: "profileNextButton"; text: window.tr("Далее", "Next"); enabled: window.state.offset + window.state.profiles.length < window.state.total && !window.state.busy; onClicked: window.query(window.state.offset + 200) }
                    }

            }
            ColumnLayout {
                spacing: 14
                SectionTitle { text: window.tr("Настройки", "Settings") }
                ColumnLayout {
                    visible: window.settingsSection === ""
                    Layout.fillWidth: true; Layout.fillHeight: true
                    spacing: 10
                    SmallLabel { text: window.tr("ПАРАМЕТРЫ", "PREFERENCES"); font.letterSpacing: 2; font.pixelSize: 11; Layout.bottomMargin: 4 }
                    SettingsRow { objectName: "settingsConnectionButton"; text: window.tr("Подключение", "Connection"); description: window.tr("Ядро и локальный прокси", "Engine and local proxy"); symbol: "◎"; onClicked: window.settingsSection = "connection" }
                    SettingsRow { objectName: "settingsSubscriptionsButton"; text: window.tr("Подписки", "Subscriptions"); description: window.tr("Источники серверов и обновление", "Server sources and refresh"); symbol: "+"; onClicked: window.settingsSection = "subscriptions" }
                    SettingsRow { objectName: "settingsDiagnosticsButton"; text: window.tr("Диагностика", "Diagnostics"); description: window.tr("Доступность TCP и проверка HTTPS", "TCP reachability and HTTPS check"); symbol: "≋"; onClicked: window.settingsSection = "diagnostics" }
                    SettingsRow { objectName: "settingsAppearanceButton"; text: window.tr("Интерфейс", "Appearance"); description: window.tr("Язык и анимация", "Language and motion"); symbol: "Aa"; onClicked: window.settingsSection = "appearance" }
                    SettingsRow { objectName: "settingsAboutButton"; text: window.tr("О приложении", "About"); description: window.tr("Сборка и поведение окна", "Build and window behavior"); symbol: "i"; onClicked: window.settingsSection = "about" }
                    Item { Layout.fillHeight: true }
                }
                Loader {
                    id: settingsPanel
                    objectName: "settingsPanel"
                    Layout.fillWidth: true; Layout.fillHeight: true
                    visible: window.settingsSection !== ""
                    active: window.page === 2 && window.settingsSection !== ""
                    sourceComponent: window.settingsSection === "subscriptions" ? subscriptionsPanel
                        : window.settingsSection === "diagnostics" ? diagnosticsPanel
                        : window.settingsSection === "appearance" ? appearancePanel
                        : window.settingsSection === "about" ? aboutPanel : connectionPanel
                    onLoaded: if (item && !window.reducedMotion) panelEntrance.restart()
                    NumberAnimation { id: panelEntrance; target: settingsPanel; property: "opacity"; from: 0.6; to: 1; duration: 160 }
                }
            }
        }
    }
    Component {
        id: connectionPanel
        PanelScroll {
            id: connectionScroll
            ColumnLayout {
                width: connectionScroll.availableWidth; spacing: 18
                PlainLabel { text: window.tr("Подключение", "Connection"); font.pixelSize: 19; font.weight: Font.DemiBold }
                RowLayout {
                    Layout.fillWidth: true
                    PlainLabel { Layout.fillWidth: true; text: window.tr("Ядро", "Engine") }
                    ComboBox { objectName: "engineChoice"; model: ["xray", "sing-box"]; currentIndex: window.state.engine === "sing-box" ? 1 : 0; enabled: window.editable; onActivated: window.send({command: "set_engine", engine: currentText}); Accessible.name: window.tr("Прокси-ядро", "Proxy engine") }
                    HelpButton { explanation: window.tr("Обрабатывает подключение к серверу. Меняется после отключения и действует до закрытия приложения.", "Handles the server connection. Change after disconnecting; applies until the app exits.") }
                }
                Divider {}
                RowLayout {
                    Layout.fillWidth: true
                    PlainLabel { Layout.fillWidth: true; text: window.tr("Режим", "Mode") }
                    PlainLabel { text: window.tr("Локальный SOCKS5", "Local SOCKS5") }
                    HelpButton { explanation: window.tr("Настройте прокси в нужном приложении. Системный VPN, TUN, DNS/Fake-IP и kill switch пока недоступны: трафик всей системы не перенаправляется.", "Configure the proxy in your application. System VPN, TUN, DNS/Fake-IP and kill switch are unavailable: system-wide traffic is not redirected.") }
                }
                RowLayout {
                    Layout.fillWidth: true
                    PlainLabel { Layout.fillWidth: true; text: window.tr("Адрес прокси", "Proxy address") }
                    TextField { id: proxyAddress; objectName: "proxyAddress"; Layout.preferredWidth: 160; readOnly: true; selectByMouse: true; text: window.state.listen_address || "127.0.0.1:" + window.state.port; Accessible.name: window.tr("Адрес локального прокси", "Local proxy address") }
                    ActionButton { objectName: "proxyCopyButton"; text: window.tr("Копировать", "Copy"); enabled: window.state.ready; onClicked: { proxyAddress.selectAll(); proxyAddress.copy(); proxyAddress.deselect() } }
                    HelpButton { explanation: window.tr("Свободный порт выбирается автоматически при подключении. Здесь показан текущий адрес.", "An available port is chosen automatically when connecting. The current address is shown here.") }
                }
                PlainLabel { Layout.fillWidth: true; visible: !!window.state.error; color: "#e6bc83"; text: window.state.error ? window.errorText(window.state.error) : "" }
            }
        }
    }
    Component {
        id: subscriptionsPanel
        ColumnLayout {
            spacing: 12
            RowLayout {
                PlainLabel { text: window.tr("Подписки", "Subscriptions"); font.pixelSize: 19; font.weight: Font.DemiBold; Layout.fillWidth: true }
                HelpButton { explanation: window.tr("Вставьте ссылку: tofu сохранит подписку и загрузит серверы. Частичный импорт требует вашего подтверждения. Ссылки скрыты и хранятся локально.", "Paste a link: tofu saves the subscription and fetches servers. Partial imports require your approval. Links stay hidden and are stored locally.") }
                ActionButton { objectName: "addSourceButton"; text: window.tr("Добавить", "Add"); highlighted: true; enabled: window.state.ready && !window.state.busy && !window.state.pending; onClicked: addDialog.open() }
            }
            PlainLabel {
                objectName: "subscriptionFeedback"
                Layout.fillWidth: true
                visible: window.state.busy || !!window.state.error || !!window.parseError || !!window.state.pending
                text: window.parseError || (window.state.error ? window.errorText(window.state.error)
                    : window.state.busy ? window.tr("Загружаем подписку…", "Loading subscription…")
                    : window.state.pending ? window.tr("Проверьте импорт перед сохранением", "Review the import before saving") : "")
                color: window.state.error || window.parseError ? "#e6bc83" : window.accent
            }
            ListView {
                id: sourceList
                objectName: "sourceList"
                Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumHeight: 96
                clip: true; spacing: 6; model: window.state.sources
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar {}
                delegate: ItemDelegate {
                    id: subscriptionRow
                    required property var modelData
                    objectName: "sourceSelect_" + modelData.id
                    width: ListView.view.width; height: 64
                    highlighted: window.state.source_id === modelData.id
                    enabled: window.state.ready && !window.state.busy
                    Accessible.name: modelData.name
                    background: Rectangle { color: subscriptionRow.highlighted ? "#302638" : subscriptionRow.hovered ? "#242127" : window.surface; radius: 12; border.color: subscriptionRow.activeFocus ? window.accent : window.line }
                    contentItem: PlainLabel { text: subscriptionRow.modelData.name; maximumLineCount: 1; elide: Text.ElideRight; verticalAlignment: Text.AlignVCenter }
                    onClicked: { window.send({command: "select_source", id: modelData.id}) }
                    Keys.onReturnPressed: if (enabled) clicked()
                    Keys.onEnterPressed: if (enabled) clicked()
                }
                SmallLabel { anchors.centerIn: parent; width: parent.width - 32; horizontalAlignment: Text.AlignHCenter; visible: window.state.sources.length === 0; text: window.tr("Добавьте первую подписку по ссылке", "Add your first subscription by link") }
            }
            RowLayout {
                SmallLabel { Layout.fillWidth: true; text: window.state.sources_total > 0 ? (window.state.sources_offset + 1) + "–" + (window.state.sources_offset + window.state.sources.length) + " / " + window.state.sources_total : window.tr("Нет подписок", "No subscriptions") }
                ActionButton { text: "‹"; Accessible.name: window.tr("Предыдущие подписки", "Previous subscriptions"); enabled: window.state.ready && !window.state.busy && window.state.sources_offset > 0; onClicked: window.send({command: "source_page", offset: Math.max(0, window.state.sources_offset - 64)}) }
                ActionButton { text: "›"; Accessible.name: window.tr("Следующие подписки", "Next subscriptions"); enabled: window.state.ready && !window.state.busy && window.state.sources_offset + window.state.sources.length < window.state.sources_total; onClicked: window.send({command: "source_page", offset: window.state.sources_offset + 64}) }
            }
            SmallLabel { Layout.fillWidth: true; visible: window.state.source_id !== null && !window.state.busy && !window.state.pending; text: window.state.total === 0 ? window.tr("Профилей нет. Обновите подписку, чтобы повторить загрузку.", "No profiles. Refresh the subscription to retry loading.") : window.tr("Профилей: ", "Profiles: ") + window.state.total }
            RowLayout {
                Layout.fillWidth: true
                CheckBox {
                    objectName: "subscriptionHwidChoice"
                    Layout.fillWidth: true
                    text: window.tr("ID устройства", "Device ID")
                    checked: window.state.source_hwid_enabled === true
                    enabled: window.state.ready && window.state.source_id !== null && !window.state.busy && !window.state.pending
                    onToggled: window.send({command: "set_source_hwid", enabled: checked})
                }
                HelpButton { explanation: window.tr("Случайный стабильный ID отправляется провайдеру. Данные оборудования не используются. Включите, только если провайдер требует ID устройства.", "A random stable ID is sent to the provider. Hardware data is not used. Enable only if the provider requires a device ID.") }
            }
            RowLayout {
                Layout.fillWidth: true
                ActionButton { objectName: "refreshSourceButton"; text: window.tr("Обновить", "Refresh"); enabled: window.state.ready && window.state.source_id !== null && !window.state.busy && !window.state.pending; onClicked: { window.send({command: "refresh"}) } }
                ActionButton { objectName: "subscriptionServersButton"; text: window.tr("К серверам", "Choose server"); enabled: window.state.total > 0 && !window.state.busy; onClicked: window.page = 1 }
            }
        }
    }
    Component {
        id: appearancePanel
        ColumnLayout {
            spacing: 18
            PlainLabel { text: window.tr("Интерфейс", "Appearance"); font.pixelSize: 19; font.weight: Font.DemiBold }
            RowLayout {
                PlainLabel { Layout.fillWidth: true; text: window.tr("Язык", "Language") }
                ComboBox { objectName: "languageChoice"; model: ["Русский", "English"]; currentIndex: window.language === "ru" ? 0 : 1; onActivated: window.language = currentIndex === 0 ? "ru" : "en"; Accessible.name: window.tr("Язык интерфейса", "Interface language") }
                HelpButton { explanation: window.tr("Выбор действует до закрытия приложения.", "Applies until the app exits.") }
            }
            Divider {}
            CheckBox { objectName: "reducedMotionChoice"; text: window.tr("Меньше анимации", "Reduce motion"); checked: window.reducedMotion; onToggled: window.reducedMotion = checked }
            Item { Layout.fillHeight: true }
        }
    }
    Component {
        id: aboutPanel
        ColumnLayout {
            spacing: 18
            SectionTitle { text: "tofu" }
            SmallLabel { text: window.tr("Открытый исходный код", "Open source") }
            Rectangle {
                Layout.fillWidth: true; implicitHeight: buildDetails.implicitHeight + 32
                color: window.surface; radius: 16
                ColumnLayout {
                    id: buildDetails
                    anchors.fill: parent; anchors.margins: 16; spacing: 8
                    SmallLabel { text: window.tr("СБОРКА", "BUILD"); font.pixelSize: 11; font.letterSpacing: 2 }
                    PlainLabel { objectName: "buildIdLabel"; Layout.fillWidth: true; text: window.state.build_id || window.tr("Локальная сборка", "Local build"); font.pixelSize: 13; wrapMode: Text.WrapAnywhere }
                }
            }
            RowLayout {
                PlainLabel { Layout.fillWidth: true; text: window.tr("Закрытие и трей", "Window and tray") }
                HelpButton { explanation: window.tr("С доступным треем закрытие скрывает окно; выход — в меню трея. Без трея закрытие отключает прокси и завершает приложение.", "With a tray, closing hides the window; quit from the tray menu. Without a tray, closing disconnects the proxy and exits.") }
            }
            ActionButton { text: window.tr("Перечитать состояние", "Reload state"); enabled: window.state.ready && !window.state.busy; onClicked: window.send({command: "reload"}) }
            Item { Layout.fillHeight: true }
        }
    }
    Component {
        id: diagnosticsPanel
        PanelScroll {
            id: diagnosticsScroll
            objectName: "diagnosticsScroll"
            ColumnLayout {
                width: diagnosticsScroll.availableWidth; spacing: 14
                PlainLabel { Layout.fillWidth: true; visible: !!window.state.error || !!window.parseError; color: "#e6bc83"; text: window.parseError || (window.state.error ? window.errorText(window.state.error) : "") }
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
                                    text: "TCP"
                                    font.weight: Font.DemiBold
                                }
                                HelpButton { explanation: window.tr("Три прямые TCP-пробы через обычную сеть. Не проверяет HTTPS, авторизацию или скорость. DNS и TCP вместе — до 5 секунд. После смены сети повторите проверку.", "Three direct TCP samples over the regular network. Does not check HTTPS, authentication or throughput. DNS and TCP together take up to 5 seconds. Rerun after network changes.") + "\n" + window.networkObservationText() }
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

                        }
                    }
                        Divider {}
                        RowLayout {
                            PlainLabel { Layout.fillWidth: true; text: window.tr("Проверка HTTPS", "HTTPS check"); font.pixelSize: 18; font.weight: Font.DemiBold }
                            HelpButton { explanation: window.tr("Проверяет HTTPS через локальный прокси при подключении. TLS проверяется, редиректы не принимаются. Меняется после отключения и применяется со следующего подключения. URL очищается после отправки.", "Checks HTTPS through the local proxy when connecting. TLS is verified; redirects are rejected. Change after disconnecting; applies to the next connection. The URL is cleared after sending.") }
                        }
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
                                value: window.state.probe_expected_status || 204
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
                Item { Layout.preferredHeight: 12 }
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
        onClosed: { sourceName.clear(); sourceUrl.clear(); sourceHwid.checked = false; localException.checked = false; sourceAdvanced.checked = false; reveal.checked = false }
        contentItem: ColumnLayout {
            spacing: 12
            RowLayout {
                PlainLabel { Layout.fillWidth: true; text: window.tr("Ссылка подписки", "Subscription URL") }
                HelpButton { objectName: "subscriptionUrlHelpButton"; explanation: window.tr("Ссылка может содержать секретный токен. Она скрыта и хранится локально.", "The link may contain a secret token. It stays hidden and is stored locally.") }
            }
            RowLayout {
                TextField { id: sourceUrl; objectName: "sourceUrlInput"; Layout.fillWidth: true; maximumLength: 8192; echoMode: reveal.checked ? TextInput.Normal : TextInput.Password; selectByMouse: true; inputMethodHints: Qt.ImhUrlCharactersOnly | Qt.ImhNoPredictiveText | Qt.ImhSensitiveData; Accessible.name: window.tr("Секретная ссылка подписки", "Secret subscription URL") }
                CheckBox { id: reveal; objectName: "revealSourceUrl"; text: window.tr("Показать", "Show") }
            }
            ActionButton {
                id: sourceAdvanced; objectName: "sourceAdvancedButton"
                text: window.tr("Дополнительно", "Advanced"); checkable: true; highlighted: checked
                onCheckedChanged: if (!checked) { localException.checked = false; sourceHwid.checked = false }
            }
            PlainLabel { visible: sourceAdvanced.checked; text: window.tr("Название · необязательно", "Name · optional") }
            TextField { id: sourceName; objectName: "sourceNameInput"; visible: sourceAdvanced.checked; Layout.fillWidth: true; maximumLength: 128; selectByMouse: true; Accessible.name: window.tr("Название подписки", "Subscription name") }
            RowLayout {
                visible: sourceAdvanced.checked
                CheckBox { id: localException; objectName: "sourceLocalException"; Layout.fillWidth: true; text: window.tr("Локальный источник", "Local source") }
                HelpButton { explanation: window.tr("Разрешает HTTP и адреса внутри своей сети. Включайте только для доверенной ссылки.", "Allows HTTP and addresses inside your network. Enable only for a trusted link.") }
            }
            RowLayout {
                visible: sourceAdvanced.checked
                CheckBox { id: sourceHwid; objectName: "sourceHwidChoice"; Layout.fillWidth: true; text: window.tr("ID устройства", "Device ID"); checked: false }
                HelpButton { explanation: window.tr("Случайный стабильный ID отправляется провайдеру. Данные оборудования не используются. Включите, только если провайдер требует ID устройства.", "A random stable ID is sent to the provider. Hardware data is not used. Enable only if the provider requires a device ID.") }
            }
            RowLayout {
                Layout.topMargin: 10
                Item { Layout.fillWidth: true }
                ActionButton { objectName: "sourceAddCancelButton"; text: window.tr("Отмена", "Cancel"); onClicked: addDialog.close() }
                ActionButton {
                    objectName: "sourceAddConfirmButton"
                    text: window.tr("Добавить", "Add")
                    highlighted: true
                    enabled: window.state.ready && sourceUrl.text.trim().length > 0 && !window.state.busy && !window.state.pending
                    onClicked: {
                        window.settingsSection = "subscriptions"
                        window.page = 2
                        search.clear()
                        favorites.checked = false
                        window.send({command: "import_source", name: sourceName.text.trim() || window.tr("Подписка", "Subscription"), url: sourceUrl.text.trim(), local_exception: localException.checked, hwid_enabled: sourceHwid.checked, route: "direct"})
                        sourceUrl.clear()
                        addDialog.close()
                    }
                }
            }
        }
        onOpened: sourceUrl.forceActiveFocus()
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
