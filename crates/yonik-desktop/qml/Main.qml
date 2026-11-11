pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import cloud.highpriority.yonik

ApplicationWindow {
    readonly property var backendForDiagnostics: backend
    id: window
    width: 420
    height: 760
    minimumWidth: 380
    minimumHeight: 600
    visible: true
    title: "yonik · " + (state.build_id || "local")
    objectName: "yonikWindow"
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
    readonly property bool editable: state.ready && !state.busy && !state.system_busy && !state.can_stop && !state.closed
    readonly property string actionText: state.can_stop
        ? (state.status === "CleanupError" ? tr("Повторить остановку", "Retry stopping")
           : (["Preparing", "Starting", "Checking"].indexOf(state.status) >= 0 ? tr("Отменить подключение", "Cancel connection") : tr("Отключить", "Disconnect")))
        : tr("Подключить", "Connect")
    property string language: "ru"
    property int page: 0
    property int settingsOrigin: 0
    property string settingsSection: ""
    property double maintenanceRequestSequence: 0
    property bool reducedMotion: false
    property var desiredProfileId: null
    property var desiredSourceId: null
    readonly property bool selectionConfirmed: (desiredProfileId === null || state.selected_profile === desiredProfileId)
        && (desiredSourceId === null || state.source_id === desiredSourceId)
    property bool sourceOptions: false
    property string commandContext: "connection"
    readonly property string connectionError: state.connection_error || (!state.ready ? state.error || "" : "")
    readonly property string sourceError: state.source_error || ""
    readonly property bool sourceLoading: state.busy && state.refresh_source_id !== undefined && state.refresh_source_id !== null
    function sourceFeedbackText() {
        if (sourceError) return errorText(sourceError)
        if (state.pending) return tr("Импорт ожидает проверки", "Import awaits review")
        if (sourceLoading) return tr("Загрузка подписки…", "Loading subscription…")
        return state.source_name ? state.source_name + tr(" · серверов: ", " · servers: ") + state.total : ""
    }
    property bool pasteOnOpen: false
    property bool searchQueued: false
    function openAdd(paste) {
        pasteOnOpen = paste
        addDialog.open()
    }
    function selectSource(id) {
        desiredSourceId = id
        desiredProfileId = null
        send({command: "select_source", id: id})
    }
    function selectProfile(id) {
        desiredProfileId = id
        send({command: "select_profile", id: id, revision: state.revision})
    }
    function openSettings() {
        settingsOrigin = page
        settingsSection = ""
        page = 2
    }
    function back() {
        if (page === 2 && settingsSection !== "") settingsSection = ""
        else if (page === 2) page = settingsOrigin
        else if (page === 1 && sourceOptions) sourceOptions = false
        else if (page === 1) page = 0
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
    readonly property bool canEditProbe: state.ready && !state.busy && !state.system_busy && !state.can_stop && !state.closed
    readonly property color ink: "#f5f3f8"
    readonly property color muted: "#bdb4cd"
    readonly property color accent: "#c3a6ef"

    function tr(ru, en) { return language === "ru" ? ru : en }
    function send(command) {
        const sourceCommands = ["import_source", "refresh", "cancel_refresh", "rename_source", "delete_source", "set_source_hwid", "select_source", "source_page", "approve_refresh", "discard_refresh", "review_page", "query", "select_profile"]
        commandContext = sourceCommands.indexOf(command.command) >= 0 ? "source" : "connection"
        if (!state.closed) backend.dispatch(JSON.stringify(command))
    }
    property bool routingPending: false
    property double routingRequestId: 0
    property double routingExpectedSavedId: 0
    property var routingFeedback: null
    function completeRouting(ack) {
        if (!routingPending || !ack || ack.request_id !== routingRequestId) return
        if (!ack.error && routingExpectedSavedId && ack.saved_id !== routingExpectedSavedId)
            ack = {request_id: ack.request_id, error: "routing_ack_identity", saved_id: null}
        routingFeedback = ack
        routingPending = false
    }
    function sendRouting(command) {
        if (routingPending || state.closed) return 0
        routingRequestId += 1
        command.request_id = routingRequestId
        if (command.expected_revision === undefined) command.expected_revision = state.routing_revision
        routingExpectedSavedId = command.command === "edit_routing" ? command.rule.id : 0
        routingFeedback = null
        routingPending = true
        commandContext = "routing"
        backend.dispatch(JSON.stringify(command))
        return routingRequestId
    }
    function query(offset) {
        if (!state.ready || state.source_id === null) { searchQueued = false; return }
        if (state.busy || state.pending) { searchQueued = true; return }
        searchQueued = false
        send({command: "query", search: search.text, favorites_only: false, offset: offset})
    }
    function readSnapshot() {
        try {
            const raw = backend.snapshot_json
            if (!raw || raw.length > 4194304) return
            const next = JSON.parse(raw)
            if (next && Object.keys(next).length === 0) return
            if (next && typeof next.error === "string" && !Array.isArray(next.sources) && !Array.isArray(next.profiles)) {
                if (commandContext === "routing") completeRouting({request_id: routingRequestId, error: next.error, saved_id: null})
                const update = commandContext === "source" ? {error: next.error, source_error: next.error} : {error: next.error, connection_error: next.error}
                if (next.maintenance_ack && typeof next.maintenance_ack === "object") update.maintenance_ack = next.maintenance_ack
                if (next.logs && typeof next.logs === "object" && !Array.isArray(next.logs) && typeof next.logs.request_id === "string") update.logs = next.logs
                state = Object.assign({}, state, update)
                parseError = ""
                return
            }
            if (!next || !Array.isArray(next.sources) || !Array.isArray(next.profiles)) {
                parseError = tr("Не удалось прочитать состояние приложения.", "Could not read application state.")
                return
            }
            if (next.source_id !== state.source_id) {
                search.text = typeof next.search === "string" ? next.search : ""
                searchQueued = false
                searchDelay.stop()
            }
            if (desiredSourceId === next.source_id) desiredSourceId = null
            if (desiredProfileId === next.selected_profile) desiredProfileId = null
            if (next.language) language = next.language
            state = next
            completeRouting(next.routing_ack)
            if (searchQueued && !next.busy && !next.pending) searchDelay.restart()
            parseError = ""
            if (next.pending && !reviewDialog.visible) reviewDialog.open()
            if (!next.pending && reviewDialog.visible) reviewDialog.close()
        } catch (_) {
            parseError = tr("Не удалось прочитать состояние приложения.", "Could not read application state.")
        }
    }
    function statusText(status) {
        const values = {
            Unknown: ["Состояние сети неизвестно", "Network state unknown"],
            NoManagedPolicy: ["Управляемых правил нет", "No managed policy"],
            TunnelStarting: ["Туннель запускается; доступ не подтверждён", "Tunnel starting; connectivity unverified"],
            VerifiedRules: ["Правила подтверждены; доступ не подтверждён", "Rules verified; connectivity unverified"],
            RecoveryRequired: ["Требуется восстановление сети", "Network recovery required"],
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
            preferences_write: ["Не удалось сохранить настройки. Изменение не применено. Проверьте preferences.json в каталоге данных.", "Could not save settings. The change was not applied. Check preferences.json in the data directory."],
            invalid_input: ["Проверьте введённые данные и адрес подписки.", "Check the input and subscription URL."],
            busy: ["Дождитесь завершения текущей операции.", "Wait for the current operation to finish."],
            source_in_use: ["Сначала отключите прокси, затем удалите подписку.", "Disconnect the proxy before deleting this subscription."],
            refresh_required: ["Загрузка уже завершилась. Проверьте список серверов.", "Loading has already finished. Check the server list."],
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
            system_unsupported: ["Системный режим недоступен: helper сообщает system_tun=false.", "System mode unavailable: helper reports system_tun=false."],
            system_denied: ["Нет разрешения на системное подключение.", "System connection authorization denied."],
            system_owner_lost: ["Helper перезапущен. Проверьте состояние сети.", "Helper restarted. Check network state."],
            system_timeout: ["Время ожидания helper истекло. Исход неизвестен.", "Helper timed out. Outcome unknown."],
            system_unavailable: ["Helper недоступен. Проверьте состояние сети.", "Helper unavailable. Check network state."],
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
        const editorMessages = {
            routing_stale: ["Черновик изменился. Ввод сохранён в форме; откройте актуальное правило заново или явно создайте новое.", "The draft changed. Input is preserved in the form; reopen the current rule or explicitly create a new one."],
            routing_ack_identity: ["Ответ сохранения не совпал с правилом. Ввод сохранён; проверьте черновик.", "Save response did not match the rule. Input is preserved; check the draft."],
            routing_storage: ["Черновик не сохранён: повреждение или конфликт БД. Исходник сохранён; перезапустите приложение.", "Draft not saved: damaged database or conflict. Original preserved; restart the application."],
            domain: ["Некорректный домен", "Invalid domain"], cidr: ["Некорректный IP/CIDR: проверьте биты сети", "Invalid IP/CIDR: check network bits"],
            executable: ["Нужен абсолютный путь без . и ..", "An absolute path without . and .. is required"],
            broad: ["Подтвердите правило для любого трафика", "Confirm matching all traffic"],
            id: ["Недопустимый или повторяющийся ID", "Invalid or duplicate ID"],
            limit: ["Лимит: 256 правил, 64 значения в группе, 4096 байт в значении", "Limit: 256 rules, 64 values per group, 4096 bytes per value"]
        }
        const pair = messages[key] || editorMessages[key]
        return pair ? tr(pair[0], pair[1]) : tr("Операция не выполнена. Проверьте настройки и повторите попытку.", "Operation failed. Check the settings and try again.")
    }

    DesktopBridge {
        id: backend
        onSnapshot_jsonChanged: window.readSnapshot()
    }
    Component.onCompleted: { backend.initialize(); readSnapshot(); backend.dispatch(JSON.stringify({command: "subscription_visibility", visible: window.visible})) }
    onVisibleChanged: backend.dispatch(JSON.stringify({command: "subscription_visibility", visible: window.visible}))
    Shortcut {
        sequence: "Escape"
        enabled: !addDialog.visible && !reviewDialog.visible && !renameDialog.visible && !deleteDialog.visible
        onActivated: window.back()
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
        implicitHeight: window.width < 460 ? 68 : 78
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
        anchors.margins: window.height < 720 ? 18 : 28
        spacing: window.height < 720 ? 12 : 20
        RowLayout {
            Layout.fillWidth: true
            ActionButton {
                objectName: "homeButton"
                visible: window.page !== 0
                text: "‹"
                implicitWidth: 44
                Accessible.name: window.tr("Назад", "Back")
                onClicked: window.back()
            }
            RowLayout {
                spacing: 10
                Image {
                    Layout.preferredWidth: 28
                    Layout.preferredHeight: 28
                    source: "qrc:/icons/yonik.svg"
                    sourceSize.width: 56
                    sourceSize.height: 56
                }
                PlainLabel {
                    text: "yonik"; font.pixelSize: 27
                    font.family: displayFont.status === FontLoader.Ready ? displayFont.name : window.font.family
                    font.weight: Font.DemiBold
                }
            }
            Item { Layout.fillWidth: true }
            ActionButton {
                objectName: "settingsButton"
                visible: window.page !== 2
                implicitWidth: 44
                leftPadding: 8; rightPadding: 8
                text: "⚙"
                font.family: "Noto Sans"
                font.pixelSize: 27
                Accessible.name: window.tr("Настройки", "Settings")
                onClicked: window.openSettings()
            }
        }
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: window.page
            PanelScroll {
                id: homeScroll
                ColumnLayout {
                    width: homeScroll.availableWidth
                    height: Math.max(implicitHeight, homeScroll.availableHeight)
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
                        onClicked: window.page = 1
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
                    Item { Layout.fillHeight: true; Layout.minimumHeight: 24 }
                    Button {
                        id: powerButton
                        objectName: "mainConnectButton"
                        Layout.alignment: Qt.AlignHCenter
                        implicitWidth: Math.min(window.width - 100, window.height < 720 ? 164 : 224); implicitHeight: implicitWidth
                        scale: down ? 0.96 : hovered && enabled ? 1.025 : 1
                        Behavior on scale { NumberAnimation { duration: window.reducedMotion ? 0 : 160; easing.type: Easing.OutCubic } }
                        enabled: window.state.ready && !window.state.closed && (window.state.can_stop || (window.state.can_connect && window.selectionConfirmed))
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
                        Layout.fillWidth: true; Layout.alignment: Qt.AlignHCenter; Layout.topMargin: window.height < 720 ? 20 : 26; spacing: 9
                        Rectangle {
                            implicitWidth: 7; implicitHeight: 7; radius: 4
                            color: window.state.status === "Connected" ? "#a8d8bc" : ["Limited", "CleanupError", "Error"].indexOf(window.state.status) >= 0 ? "#e6bc83" : window.accent
                        }
                        PlainLabel {
                            objectName: "mainStatusLabel"
                            Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter
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
                        text: window.parseError || (window.connectionError ? window.errorText(window.connectionError)
                            : window.state.status === "Limited" ? window.tr("Доступ к интернету не подтверждён", "Internet access is unconfirmed")
                            : window.state.status === "CleanupError" ? window.tr("Повторите остановку перед выходом", "Retry stopping before quitting")
                            : window.state.can_stop ? window.actionText
                            : window.state.selected_name ? window.tr("Нажмите, чтобы подключиться", "Press to connect") : window.tr("Добавьте подписку и выберите сервер", "Add a subscription and choose a server"))
                    }
                    SmallLabel {
                        objectName: "selectedNextServerLabel"
                        Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter
                        visible: window.state.can_stop && !!window.state.selected_name
                        Layout.topMargin: visible ? 12 : 0
                        text: window.tr("Выбран для следующего подключения: ", "Selected for next connection: ") + window.state.selected_name + window.tr(" · после отключения", " · after disconnecting")
                    }
                    Item { Layout.fillHeight: true; Layout.minimumHeight: 24 }
                }
            }
            PanelScroll {
                id: serversScroll
                ColumnLayout {
                width: serversScroll.availableWidth
                height: Math.max(implicitHeight, serversScroll.availableHeight)
                spacing: 8
                RowLayout {
                    SectionTitle { Layout.fillWidth: true; text: window.tr("Серверы", "Servers") }
                    ActionButton { objectName: "addSourceButton"; text: window.tr("Добавить", "Add"); leftPadding: 8; rightPadding: 8; enabled: window.state.ready && !window.state.busy && !window.state.pending; onClicked: window.openAdd(false) }
                    ActionButton { objectName: "pasteSourceButton"; text: window.tr("Вставить", "Paste"); leftPadding: 8; rightPadding: 8; enabled: window.state.ready && !window.state.busy && !window.state.pending; onClicked: window.openAdd(true) }
                }
                RowLayout {
                    ComboBox {
                        id: overviewSources
                        objectName: "profileSourceFilter"
                        Layout.fillWidth: true
                        model: window.state.sources; textRole: "name"; valueRole: "id"
                        currentIndex: { for (let i = 0; i < window.state.sources.length; ++i) if (window.state.sources[i].id === window.state.source_id) return i; return -1 }
                        displayText: currentIndex < 0 ? window.tr("Нет подписок", "No subscriptions") : currentText
                        delegate: ItemDelegate { id: sourceOption; required property var modelData; width: overviewSources.width; contentItem: PlainLabel { text: sourceOption.modelData.name; maximumLineCount: 1; elide: Text.ElideRight } }
                        onActivated: window.selectSource(currentValue)
                        enabled: window.state.ready && !window.state.busy && count > 0
                        Accessible.name: window.tr("Источник серверов", "Server source")
                    }
                    ActionButton { objectName: "refreshSourceButton"; text: window.tr("Обновить", "Refresh"); enabled: window.state.ready && window.state.source_id !== null && !window.state.busy && !window.state.pending; onClicked: window.send({command: "refresh"}) }
                    ActionButton { objectName: "sourceOptionsButton"; text: window.tr("Ещё", "More"); leftPadding: 8; rightPadding: 8; Accessible.name: window.tr("Параметры подписки", "Source options"); checkable: true; checked: window.sourceOptions; enabled: window.state.source_id !== null; onClicked: window.sourceOptions = !window.sourceOptions }
                }
                RowLayout {
                    visible: window.state.sources_total > 64
                    SmallLabel { Layout.fillWidth: true; text: window.tr("Подписки: ", "Subscriptions: ") + (window.state.sources_offset + 1) + "–" + (window.state.sources_offset + window.state.sources.length) + " / " + window.state.sources_total }
                    ActionButton { objectName: "sourcePreviousButton"; text: "‹"; enabled: !window.state.busy && window.state.sources_offset > 0; onClicked: window.send({command: "source_page", offset: Math.max(0, window.state.sources_offset - 64)}) }
                    ActionButton { objectName: "sourceNextButton"; text: "›"; enabled: !window.state.busy && window.state.sources_offset + window.state.sources.length < window.state.sources_total; onClicked: window.send({command: "source_page", offset: window.state.sources_offset + 64}) }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: window.sourceOptions && window.state.source_id !== null
                    RowLayout {
                    Layout.fillWidth: true
                    CheckBox { objectName: "subscriptionHwidChoice"; Layout.fillWidth: true; text: window.tr("ID устройства", "Device ID"); checked: window.state.source_hwid_enabled === true; enabled: window.state.ready && !window.state.busy && !window.state.pending; onToggled: window.send({command: "set_source_hwid", enabled: checked}) }
                    HelpButton { explanation: window.tr("Отправляет провайдеру случайный стабильный ID, без данных оборудования. Изменение повторно загружает подписку.", "Sends a random stable ID without hardware data. Changing it refreshes the subscription.") }
                    }
                    RowLayout {
                    Layout.fillWidth: true
                    ActionButton { Layout.fillWidth: true; objectName: "renameSourceButton"; text: window.tr("Имя", "Rename"); enabled: !window.state.busy && !window.state.pending; onClicked: { renameInput.text = window.state.source_name || overviewSources.currentText; renameDialog.sourceId = window.state.source_id; renameDialog.open() } }
                    ActionButton { Layout.fillWidth: true; objectName: "deleteSourceButton"; text: window.tr("Удалить", "Delete"); leftPadding: 8; rightPadding: 8; enabled: !window.state.busy && !window.state.pending; onClicked: { deleteDialog.sourceId = window.state.source_id; deleteDialog.sourceName = window.state.source_name || overviewSources.currentText; deleteDialog.open() } }
                    }
                }
                RowLayout {
                    visible: window.sourceFeedbackText() !== "" || !!window.parseError
                    SmallLabel { objectName: "subscriptionFeedback"; Layout.fillWidth: true; maximumLineCount: 3; elide: Text.ElideRight; color: window.sourceError ? "#e6bc83" : window.muted; text: window.parseError || window.sourceFeedbackText() }
                    ActionButton { objectName: "cancelSourceRefreshButton"; visible: window.sourceLoading; enabled: window.state.busy && window.state.refresh_source_id !== undefined && window.state.refresh_source_id !== null && typeof window.state.refresh_token === "string"; text: window.tr("Отмена", "Cancel"); onClicked: window.send({command: "cancel_refresh", id: window.state.refresh_source_id, token: window.state.refresh_token}) }
                    ActionButton { objectName: "openReviewButton"; visible: !!window.state.pending; text: window.tr("Проверить", "Review"); onClicked: reviewDialog.open() }
                }
                RowLayout {
                    TextField { id: search; objectName: "profileSearch"; Layout.fillWidth: true; placeholderText: window.tr("Поиск серверов", "Search servers"); maximumLength: 256; selectByMouse: true; enabled: window.state.ready; onTextEdited: { window.searchQueued = true; searchDelay.restart() } onAccepted: { searchDelay.stop(); window.query(0) } Accessible.name: window.tr("Поиск серверов", "Search servers") }
                    ActionButton { objectName: "profileSearchButton"; text: window.tr("Найти", "Search"); enabled: window.state.ready && !window.state.busy; onClicked: { searchDelay.stop(); window.query(0) } }
                    ActionButton { objectName: "clearSearchButton"; visible: search.text.length > 0; text: "×"; Accessible.name: window.tr("Сбросить поиск", "Clear search"); enabled: !window.state.busy; onClicked: { search.clear(); searchDelay.stop(); window.query(0) } }
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
                                onClicked: window.selectProfile(modelData.id)
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
                                    SmallLabel { visible: window.state.selected_profile === profileRow.modelData.id; text: window.tr("Выбран", "Selected"); color: window.accent }

                                }
                            }
                        }
                        ColumnLayout {
                            anchors.centerIn: parent
                            width: Math.min(parent.width - 40, 400)
                            visible: window.state.profiles.length === 0 && !window.state.busy
                            spacing: 10
                            PlainLabel { Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; text: window.state.sources.length === 0 ? window.tr("Начните с подписки", "Start with a subscription") : window.tr("Профили не найдены", "No profiles found"); font.pixelSize: 20 }
                            SmallLabel { Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; text: window.state.sources.length === 0 ? window.tr("Добавьте ссылку, проверьте импорт и выберите сервер.", "Add a link, review the import and choose a server.") : window.tr("Обновите подписку или измените условия поиска.", "Refresh the subscription or change the search filters.") }
                            ActionButton { objectName: "emptyProfilesAction"; Layout.alignment: Qt.AlignHCenter; text: window.state.sources.length === 0 ? window.tr("Добавить подписку", "Add subscription") : search.text.length > 0 ? window.tr("Сбросить поиск", "Clear search") : window.tr("Обновить", "Refresh"); enabled: !window.state.busy && !window.state.pending; onClicked: { if (window.state.sources.length === 0) window.openAdd(false); else if (search.text.length > 0) { search.clear(); searchDelay.stop(); window.query(0) } else window.send({command: "refresh"}) } }
                        }
                    }
                    RowLayout {
                        SmallLabel { text: window.state.total > 0 ? (window.state.offset + 1) + "–" + (window.state.offset + window.state.profiles.length) + " / " + window.state.total : window.tr("Нет профилей", "No profiles"); Layout.fillWidth: true }
                        ActionButton { objectName: "profilePreviousButton"; text: window.tr("Назад", "Previous"); enabled: window.state.offset > 0 && !window.state.busy; onClicked: window.query(Math.max(0, window.state.offset - 200)) }
                        ActionButton { objectName: "profileNextButton"; text: window.tr("Далее", "Next"); enabled: window.state.offset + window.state.profiles.length < window.state.total && !window.state.busy; onClicked: window.query(window.state.offset + 200) }
                    }

                RowLayout {
                    SmallLabel { Layout.fillWidth: true; maximumLineCount: 2; elide: Text.ElideRight; text: !window.selectionConfirmed ? window.tr("Выберите сервер заново, если выбор не подтвердился", "Choose the server again if selection was not confirmed") : (window.state.can_stop ? window.tr("Работает: ", "Active: ") + window.state.active_name + " · " : "") + window.tr("Выбран: ", "Selected: ") + (window.state.selected_name || "—") }
                    ActionButton { objectName: "serversConnectButton"; text: window.actionText; enabled: window.state.ready && (window.state.can_stop || (window.state.can_connect && window.selectionConfirmed)); onClicked: window.send({command: window.state.can_stop ? "stop" : "connect"}) }
                }
            }
            }
            ColumnLayout {
                spacing: 14
                SectionTitle { text: window.tr("Настройки", "Settings") }
                PlainLabel { Layout.fillWidth: true; wrapMode: Text.Wrap; visible: !!window.state.preferences_warning; text: window.tr("Файл настроек повреждён или имеет неизвестный формат. Применены значения по умолчанию; исходник preferences.json сохранён, запись заблокирована.", "Settings are damaged or unsupported. Defaults are active; the original preferences.json is preserved and writes are blocked.") }
                PlainLabel { objectName: "preferencesDurabilityWarning"; Layout.fillWidth: true; wrapMode: Text.Wrap; visible: !!window.state.preferences_durability_warning; color: "#e6bc83"; text: window.tr("Настройки применены, файл обновлён. Не удалось подтвердить синхронизацию каталога: сохранность при сбое питания неизвестна. Откат не выполнен.", "Settings were applied and the file was updated. Directory synchronization could not be confirmed: durability after power loss is unknown. No rollback was performed.") }
                ColumnLayout {
                    visible: window.settingsSection === ""
                    Layout.fillWidth: true; Layout.fillHeight: true
                    spacing: 10
                    SmallLabel { text: window.tr("ПАРАМЕТРЫ", "PREFERENCES"); font.letterSpacing: 2; font.pixelSize: 11; Layout.bottomMargin: 4 }
                    SettingsRow { objectName: "settingsRoutingButton"; text: window.tr("Маршрутизация", "Routing"); description: window.tr("Сохранённые личные правила", "Saved personal rules"); symbol: "⇄"; onClicked: window.settingsSection = "routing" }
                    SettingsRow { objectName: "settingsConnectionButton"; text: window.tr("Подключение", "Connection"); description: window.tr("Ядро и локальный прокси", "Engine and local proxy"); symbol: "◎"; onClicked: window.settingsSection = "connection" }
                    SettingsRow { objectName: "settingsDiagnosticsButton"; text: window.tr("Диагностика", "Diagnostics"); description: window.tr("Доступность TCP и проверка HTTPS", "TCP reachability and HTTPS check"); symbol: "≋"; onClicked: window.settingsSection = "diagnostics" }
                    SettingsRow { objectName: "settingsMaintenanceButton"; text: window.tr("Резервная копия", "Backup"); description: window.tr("Зашифрованная копия и восстановление в новый файл", "Encrypted backup and restore to a new file"); symbol: "⇩"; onClicked: window.settingsSection = "maintenance" }
                    SettingsRow { objectName: "settingsAppearanceButton"; text: window.tr("Интерфейс", "Appearance"); description: window.tr("Язык приложения", "Application language"); symbol: "Aa"; onClicked: window.settingsSection = "appearance" }
                    SettingsRow { objectName: "settingsAboutButton"; text: window.tr("О приложении", "About"); description: window.tr("Сборка и поведение окна", "Build and window behavior"); symbol: "i"; onClicked: window.settingsSection = "about" }
                    Item { Layout.fillHeight: true }
                }
                Loader {
                    id: settingsPanel
                    objectName: "settingsPanel"
                    Layout.fillWidth: true; Layout.fillHeight: true
                    visible: window.settingsSection !== ""
                    active: window.page === 2 && window.settingsSection !== ""
                    sourceComponent: window.settingsSection === "routing" ? routingPanel : window.settingsSection === "diagnostics" ? diagnosticsPanel
                        : window.settingsSection === "maintenance" ? maintenancePanel
                        : window.settingsSection === "appearance" ? appearancePanel
                        : window.settingsSection === "about" ? aboutPanel : connectionPanel
                    onLoaded: if (item && !window.reducedMotion) panelEntrance.restart()
                    NumberAnimation { id: panelEntrance; target: settingsPanel; property: "opacity"; from: 0.6; to: 1; duration: 160 }
                }
            }
        }
    }
    Component {
        id: maintenancePanel
        MaintenanceView {
            nextRequestId: function() { window.maintenanceRequestSequence += 1; return window.maintenanceRequestSequence }
            sendCommand: function(command) { window.send(command) }
            errorText: function(key) { return window.errorText(key) }
            english: window.language === "en"
            available: window.state.maintenance_available === true && window.editable
            revision: window.state.revision || 0
            response: window.state.maintenance_ack || null
        }
    }
    Component {
        id: routingPanel
        PanelScroll {
            id: routingScroll
            ColumnLayout {
                id: editor
                objectName: "routingEditor"
                enabled: window.state.ready && !window.state.closed && !window.routingPending
                property var formRevision: window.state.routing_revision
                property double pendingRequest: 0
                property string saveError: ""
                function settle() {
                    const ack = window.routingFeedback
                    if (!pendingRequest || !ack || ack.request_id !== pendingRequest) return
                    pendingRequest = 0
                    saveError = ack.error || ""
                    if (!saveError) {
                        selectedId = ack.saved_id
                        formRevision = window.state.routing_revision
                    }
                }
                Connections { target: window; function onRoutingFeedbackChanged() { editor.settle() } }
                width: routingScroll.availableWidth
                spacing: 12
                property bool advanced: false
                property double selectedId: 0
                property int simpleGroup: 0
                function values(text) { return text.split("\n").map(s => s.trim()).filter(s => s.length > 0) }
                function load(rule) {
                    selectedId = rule.id
                    formRevision = window.state.routing_revision
                    saveError = ""
                    executableInput.text = rule.executables.join("\n")
                    domainInput.text = rule.domains.join("\n")
                    cidrInput.text = rule.destinations.join("\n")
                    actionInput.currentIndex = ["proxy", "direct", "block"].indexOf(rule.action)
                    enabledInput.checked = rule.enabled
                    broadInput.checked = false
                }
                function order(id, delta, remove) {
                    let ids = window.state.routing.rules.map(r => r.id)
                    let index = ids.indexOf(id)
                    if (remove) {
                        window.sendRouting({command: "routing_delete", id: id})
                        return
                    }
                    if (index + delta >= 0 && index + delta < ids.length) {
                        let other = ids[index + delta]; ids[index + delta] = id; ids[index] = other
                    }
                    window.sendRouting({command: "routing_order", ids: ids})
                }
                SectionTitle { text: window.tr("Черновик маршрутизации", "Routing draft") }
                SmallLabel { Layout.fillWidth: true; text: window.tr("Черновик сохраняется в зашифрованной БД. Первое подходящее правило имеет приоритет. VPN/LAN и пресеты здесь не редактируются.", "Draft is saved in the encrypted database. First matching rule wins. VPN/LAN and presets are not editable here.") }
                ActionButton { objectName: "routingModeButton"; text: editor.advanced ? window.tr("Простой режим", "Simple mode") : window.tr("Расширенный режим", "Advanced mode"); onClicked: editor.advanced = !editor.advanced }
                SmallLabel { Layout.fillWidth: true; text: window.tr("Условия групп: И; строки внутри группы: ИЛИ. Составные правила сохраняются при смене режима.", "Groups use AND; lines within a group use OR. Switching mode preserves compound rules.") }
                SmallLabel { objectName: "routingDiagnosticsScope"; Layout.fillWidth: true; text: "Ограниченная диагностика сохранённой ревизии " + window.state.routing_revision + ". Отсутствие предупреждений не доказывает совместимость. VPN/LAN здесь не настроены." }
                SmallLabel { objectName: "routingDiagnosticsIncomplete"; visible: !!window.state.routing_diagnostics && window.state.routing_diagnostics.incomplete; text: window.state.routing_diagnostics && window.state.routing_diagnostics.stale ? "Отчёт устарел: черновик изменился. Требуется актуальный снимок." : "Анализ неполный: достигнут лимит или вход не поддерживается." }
                Repeater {
                    model: window.state.routing_diagnostics ? window.state.routing_diagnostics.warnings : []
                    delegate: SmallLabel {
                        required property var modelData
                        Layout.fillWidth: true
                        text: "#" + modelData.rule_id + " ← " + modelData.earlier + (modelData.kind === "shadowed" ? " · условия перекрыты старшим правилом" : " · возможное пересечение; существование потока не доказано")
                    }
                }
                TextField { id: explainTarget; objectName: "routingExplainTarget"; Layout.fillWidth: true; placeholderText: "Почему такой маршрут: IP или домен (синтетический ввод)"; maximumLength: 253 }
                ActionButton { objectName: "routingExplainButton"; text: "Объяснить сохранённый черновик"; onClicked: window.sendRouting({command: "routing_explain", target: explainTarget.text}) }
                PlainLabel {
                    objectName: "routingExplanation"
                    Layout.fillWidth: true
                    property var explanation: window.state.routing_explanation
                    visible: !!explanation && explanation.revision === window.state.routing_revision && explanation.request_id === window.routingRequestId
                    text: !explanation ? "" : explanation.result.status === "fixture" ? "Предварительный расчёт: " + explanation.result.action + " · " + explanation.result.origin : "Неизвестно: " + (explanation.result.reason === "application_witness_missing" ? "нет подтверждённой атрибуции приложения" : "нет подтверждённых DNS-данных") + ". Это не проверка применения."
                }
                RowLayout {
                    PlainLabel { text: window.tr("Остальной трафик", "Other traffic") }
                    ComboBox { model: ["Proxy", "Direct", "Block"]; currentIndex: window.state.routing ? ["proxy", "direct", "block"].indexOf(window.state.routing.default_action) : 0; onActivated: window.sendRouting({command: "routing_default", action: ["proxy", "direct", "block"][currentIndex]}) }
                }
                Repeater {
                    model: window.state.routing ? window.state.routing.rules : []
                    delegate: ColumnLayout {
                        id: ruleRow
                        required property var modelData
                        Layout.fillWidth: true
                        PlainLabel { Layout.fillWidth: true; text: "#" + ruleRow.modelData.id + " · " + ruleRow.modelData.action + (ruleRow.modelData.enabled ? "" : window.tr(" · выключено", " · disabled")) }
                        SmallLabel { Layout.fillWidth: true; text: ruleRow.modelData.executables.concat(ruleRow.modelData.domains, ruleRow.modelData.destinations).join(" ∧/∨ ") || window.tr("Любой трафик: перекрывает последующие правила", "Any traffic: shadows following rules") }
                        RowLayout {
                            ActionButton { objectName: "routingEdit" + ruleRow.modelData.id; text: window.tr("Изменить", "Edit"); onClicked: editor.load(ruleRow.modelData) }
                            ActionButton { objectName: "routingUp" + ruleRow.modelData.id; text: "↑"; Accessible.name: window.tr("Выше", "Move up"); onClicked: editor.order(ruleRow.modelData.id, -1, false) }
                            ActionButton { text: "↓"; Accessible.name: window.tr("Ниже", "Move down"); onClicked: editor.order(ruleRow.modelData.id, 1, false) }
                            ActionButton { text: "×"; Accessible.name: window.tr("Удалить", "Delete"); onClicked: editor.order(ruleRow.modelData.id, 0, true) }
                        }
                        ActionButton { objectName: "routingDuplicate" + ruleRow.modelData.id; text: window.tr("Дублировать", "Duplicate"); onClicked: { editor.load(ruleRow.modelData); editor.selectedId = 0; editor.formRevision = window.state.routing_revision } }
                        Divider {}
                    }
                }
                ComboBox { visible: !editor.advanced; model: [window.tr("Программа", "Executable"), window.tr("Домен", "Domain"), "IP / CIDR"]; onActivated: editor.simpleGroup = currentIndex }
                TextField { id: catalogQuery; Layout.fillWidth: true; placeholderText: window.tr("Поиск программы", "Search applications") }
                CheckBox { id: processCatalog; text: window.tr("Запущенные процессы", "Running processes") }
                ActionButton { text: window.tr("Загрузить каталог", "Load catalog"); onClicked: window.send({command: "routing_catalog", processes: processCatalog.checked, query: catalogQuery.text, offset: 0}) }
                SmallLabel { Layout.fillWidth: true; text: window.tr("Каталог читается только по запросу. Wrapper/интерпретатор может быть неподдержан. PID — диагностика; выбор сохраняет только точный executable для следующих запусков.", "Catalog is read on demand. Wrappers/interpreters may be unsupported. PID is diagnostic; selection saves only the exact executable for future launches.") }
                SmallLabel { visible: window.state.routing_catalog && window.state.routing_catalog.incomplete; text: window.tr("Каталог неполный: достигнут лимит или нет доступа", "Incomplete catalog: limit reached or access denied") }
                Repeater {
                    model: window.state.routing_catalog ? window.state.routing_catalog.items : []
                    delegate: ActionButton {
                        required property var modelData
                        Layout.fillWidth: true
                        text: modelData.name + (modelData.pid ? " · PID " + modelData.pid : "") + (modelData.executable ? " · " + modelData.executable : window.tr(" · не поддерживается: ", " · unsupported: ") + modelData.unsupported)
                        enabled: !!modelData.executable
                        onClicked: executableInput.text = modelData.executable
                    }
                }
                RowLayout {
                    ActionButton { text: window.tr("Назад", "Previous"); enabled: window.state.routing_catalog && window.state.routing_catalog.offset > 0; onClicked: window.send({command: "routing_catalog", processes: processCatalog.checked, query: catalogQuery.text, offset: Math.max(0, window.state.routing_catalog.offset - 32)}) }
                    ActionButton { text: window.tr("Далее", "Next"); enabled: window.state.routing_catalog && window.state.routing_catalog.offset + 32 < window.state.routing_catalog.total; onClicked: window.send({command: "routing_catalog", processes: processCatalog.checked, query: catalogQuery.text, offset: window.state.routing_catalog.offset + 32}) }
                }
                PlainLabel { objectName: "routingRuleIdentity"; text: editor.selectedId ? window.tr("Редактирование #", "Editing #") + editor.selectedId : window.tr("Новое правило", "New rule") }
                SmallLabel { visible: editor.advanced || editor.simpleGroup === 0; Layout.fillWidth: true; text: window.tr("Точный путь программы (для следующих запусков, без наследования дочерними процессами)", "Exact executable path (future launches, no child process inheritance)") }
                TextArea { visible: editor.advanced || editor.simpleGroup === 0; id: executableInput; objectName: "routingExecutable"; Layout.fillWidth: true; placeholderText: "/usr/bin/browser"; wrapMode: TextEdit.Wrap; selectByMouse: true }
                SmallLabel { visible: editor.advanced || editor.simpleGroup === 1; text: window.tr("Домен или *.суффикс", "Domain or *.suffix") }
                TextArea { visible: editor.advanced || editor.simpleGroup === 1; id: domainInput; objectName: "routingDomain"; Layout.fillWidth: true; placeholderText: "example.org"; wrapMode: TextEdit.Wrap; selectByMouse: true }
                SmallLabel { visible: editor.advanced || editor.simpleGroup === 2; text: window.tr("IP или канонический CIDR", "IP or canonical CIDR") }
                TextArea { visible: editor.advanced || editor.simpleGroup === 2; id: cidrInput; objectName: "routingCidr"; Layout.fillWidth: true; placeholderText: "192.0.2.0/24"; wrapMode: TextEdit.Wrap; selectByMouse: true }
                SmallLabel { visible: editor.advanced; Layout.fillWidth: true; text: window.tr("По одному значению на строку. Порты, протоколы, process name, UID, cgroup и PID не поддерживаются текущей моделью.", "One value per line. Ports, protocols, process names, UID, cgroup and PID are unsupported by the current model.") }
                ComboBox { id: actionInput; model: ["Proxy", "Direct", "Block"] }
                CheckBox { id: enabledInput; objectName: "routingEnabled"; checked: true; text: window.tr("Правило включено", "Rule enabled") }
                CheckBox { id: broadInput; text: window.tr("Подтверждаю правило для любого трафика", "Confirm matching all traffic") }
                RowLayout {
                    ActionButton {
                        objectName: "routingSaveRule"
                        text: window.tr("Подтвердить", "Confirm")
                        onClicked: {
                            const rules = window.state.routing.rules
                            const id = editor.selectedId || (rules.reduce((n, r) => Math.max(n, r.id), 0) + 1)
                            editor.saveError = ""
                            editor.pendingRequest = window.sendRouting({command: "edit_routing", expected_revision: editor.formRevision, create: editor.selectedId === 0, confirm_broad: broadInput.checked, rule: {id: id, enabled: enabledInput.checked, executables: editor.values(executableInput.text), domains: editor.values(domainInput.text), destinations: editor.values(cidrInput.text), action: ["proxy", "direct", "block"][actionInput.currentIndex]}})
                            editor.settle()
                        }
                    }
                    ActionButton { text: window.tr("Как новое", "As new"); onClicked: { editor.selectedId = 0; editor.formRevision = window.state.routing_revision; editor.saveError = "" } }
                }
                PlainLabel { objectName: "routingSaveError"; Layout.fillWidth: true; visible: !!editor.saveError || !!(window.routingFeedback && window.routingFeedback.error); text: window.errorText(editor.saveError || (window.routingFeedback ? window.routingFeedback.error : "")) }
                ActionButton { text: window.tr("Применить к сети", "Apply to network"); enabled: false }
                SmallLabel { Layout.fillWidth: true; text: window.tr("Недоступно: system_tun=false. Системный путь и атрибуция потоков ещё не подтверждены. Изменения черновика сеть не меняют.", "Unavailable: system_tun=false. System routing and flow attribution are not verified. Draft edits do not change the network.") }
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
                    ComboBox {
                        objectName: "connectionModeChoice"
                        model: [window.tr("Локальный SOCKS5", "Local SOCKS5"), window.tr("Системное подключение", "System connection")]
                        currentIndex: window.state.connection_mode === "system" ? 1 : 0
                        enabled: window.editable && !window.state.can_unlock
                        onActivated: window.send({command: "set_connection_mode", mode: currentIndex === 1 ? "system" : "local"})
                    }
                    HelpButton { explanation: window.tr("Локальный SOCKS настраивается в нужном приложении. Системный режим требует capability и разрешения helper. Подтверждение правил ещё не подтверждает доступ через туннель.", "Configure local SOCKS in your application. System mode requires helper capability and authorization. Verified rules do not confirm tunnel connectivity.") }
                }
                PlainLabel { text: window.tr("DNS для обновлений через Proxy", "DNS for Proxy updates") }
                PlainLabel { Layout.fillWidth: true; wrapMode: Text.Wrap; text: window.tr("Укажите IP:порт или [IPv6]:порт DNS-over-TCP сервера. Он используется через активное SOCKS-подключение только для обновлений подписок в режиме Proxy. Пустое значение запрещает DNS для доменных адресов. Настройки DNS ОС и ядра не меняются. Изменение доступно после отключения.", "Enter IP:port or [IPv6]:port of a DNS-over-TCP server. Subscription updates in Proxy mode use it through the active SOCKS connection. Empty disables domain DNS. OS and core DNS settings are unchanged. Disconnect before editing.") }
                RowLayout {
                    Layout.fillWidth: true
                    TextField { id: proxyFetchDns; objectName: "proxyFetchDns"; Layout.fillWidth: true; maximumLength: 128; text: window.state.proxy_fetch_dns || ""; enabled: window.editable && !window.state.can_stop; Accessible.name: window.tr("DNS обновлений Proxy", "Proxy update DNS") }
                    ActionButton { objectName: "saveProxyFetchDns"; text: window.tr("Сохранить", "Save"); enabled: window.editable && !window.state.can_stop; onClicked: window.send({command: "set_proxy_fetch_dns", upstream: proxyFetchDns.text}) }
                }
                PlainLabel { visible: window.state.connection_mode === "system"; text: window.tr("Состояние helper: ", "Helper state: ") + (window.state.system_state || "Unknown") }
                ActionButton { objectName: "systemStateButton"; visible: window.state.connection_mode === "system"; enabled: !window.state.system_busy; text: window.tr("Проверить состояние", "Check state"); onClicked: window.send({command: "system_state"}) }
                ActionButton { objectName: "systemUnlockButton"; visible: window.state.connection_mode === "system"; enabled: !!window.state.can_unlock && !window.state.system_busy && !window.state.can_stop; text: window.tr("Снять сохранённую блокировку", "Unlock retained protection"); onClicked: window.send({command: "system_unlock"}) }
                PlainLabel { text: window.tr("Адрес прокси", "Proxy address") }
                RowLayout {
                    Layout.fillWidth: true
                    TextField { id: proxyAddress; objectName: "proxyAddress"; Layout.fillWidth: true; Layout.minimumWidth: 80; readOnly: true; selectByMouse: true; text: window.state.listen_address || "127.0.0.1:" + window.state.port; Accessible.name: window.tr("Адрес локального прокси", "Local proxy address") }
                    ActionButton { objectName: "proxyCopyButton"; text: window.tr("Копировать", "Copy"); enabled: window.state.ready; onClicked: { proxyAddress.selectAll(); proxyAddress.copy(); proxyAddress.deselect() } }
                    HelpButton { explanation: window.tr("Свободный порт выбирается автоматически при подключении. Здесь показан текущий адрес.", "An available port is chosen automatically when connecting. The current address is shown here.") }
                }
                PlainLabel { Layout.fillWidth: true; visible: !!window.connectionError; color: "#e6bc83"; text: window.connectionError ? window.errorText(window.connectionError) : "" }
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
                ComboBox { objectName: "languageChoice"; model: ["Русский", "English"]; currentIndex: window.state.language === "en" ? 1 : 0; onActivated: window.send({command: "set_language", language: currentIndex === 0 ? "ru" : "en"}); Accessible.name: window.tr("Язык интерфейса", "Interface language") }
                HelpButton { explanation: window.tr("Язык сохраняется и применяется после перезапуска.", "The language is saved and restored after restarting.") }
            }
            PlainLabel { objectName: "appearanceSaveError"; Layout.fillWidth: true; wrapMode: Text.Wrap; visible: window.connectionError === "preferences_write"; color: "#e6bc83"; text: visible ? window.errorText(window.connectionError) : "" }
            Item { Layout.fillHeight: true }
        }
    }
    Component {
        id: aboutPanel
        ColumnLayout {
            spacing: 18
            SectionTitle { text: "yonik" }
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
                HelpButton { explanation: window.tr("С доступным треем закрытие скрывает окно; выход — в меню трея. Без трея окно остаётся открытым для управления.", "With a tray, closing hides the window; quit from the tray menu. Without a tray, the window stays open for control.") }
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
                LogsView { Layout.fillWidth: true; backend: window.backendForDiagnostics; english: window.state.language === "en"; response: window.state.logs || null }
                DiagnosticsExport { Layout.fillWidth: true; backend: window.backendForDiagnostics; english: window.state.language === "en" }
                PlainLabel { Layout.fillWidth: true; visible: !!window.connectionError || !!window.parseError; color: "#e6bc83"; text: window.parseError || (window.connectionError ? window.errorText(window.connectionError) : "") }
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
                            }
                            RowLayout {
                                Layout.fillWidth: true
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
                            RowLayout {
                                Layout.fillWidth: true
                                ActionButton {
                                    objectName: "batchCheckButton"
                                    text: window.tr("Проверить источник", "Check source")
                                    enabled: window.state.ready && window.state.source_id !== null && !window.state.busy && !(window.state.batch && window.state.batch.busy)
                                    onClicked: window.send({command: "check_batch"})
                                }
                                ActionButton {
                                    objectName: "batchCancelButton"
                                    text: window.tr("Отменить серию", "Cancel batch")
                                    visible: !!window.state.batch && window.state.batch.busy
                                    onClicked: window.send({command: "cancel_batch"})
                                }
                            }
                            SmallLabel {
                                objectName: "batchProgress"
                                Layout.fillWidth: true
                                text: window.state.batch ? (window.state.batch.invalidated ? window.tr("Результаты устарели", "Results invalidated") : window.state.batch.completed + " / " + window.state.batch.total + (window.state.batch.cancelled ? window.tr(" · отменено", " · cancelled") : "")) : ""
                            }
                            Repeater {
                                model: window.state.batch ? window.state.batch.rows : []
                                delegate: SmallLabel {
                                    required property var modelData
                                    Layout.fillWidth: true
                                    text: modelData.name + " · " + modelData.host + ":" + modelData.port + " · " + window.tcpStatusText(modelData.status) + (modelData.connect_ms !== null ? " · " + modelData.connect_ms + " ms" : "")
                                    wrapMode: Text.Wrap
                                }
                            }
                            RowLayout {
                                visible: !!window.state.batch && window.state.batch.completed > 32
                                ActionButton {
                                    text: "←"
                                    enabled: window.state.batch && window.state.batch.offset > 0
                                    onClicked: window.send({command: "batch_page", offset: Math.max(0, window.state.batch.offset - 32)})
                                }
                                ActionButton {
                                    text: "→"
                                    enabled: window.state.batch && window.state.batch.offset + 32 < window.state.batch.completed
                                    onClicked: window.send({command: "batch_page", offset: window.state.batch.offset + 32})
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
                                + (window.state.probe_persistent === false ? window.tr(" · URL с параметрами действует до выхода", " · URL with parameters applies until exit") : "")
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
                            PlainLabel { Layout.fillWidth: true; text: window.tr("Ожидаемый HTTP-статус", "Expected HTTP status") }
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
    Timer { id: searchDelay; interval: 200; repeat: false; onTriggered: { if (!window.state.busy) window.query(0) } }
    DarkDialog {
        id: renameDialog
        objectName: "renameSourceDialog"
        property var sourceId: null
        modal: true; anchors.centerIn: parent; width: Math.min(window.width - 32, 440)
        title: window.tr("Название подписки", "Subscription name")
        closePolicy: Popup.CloseOnEscape
        onClosed: renameInput.clear()
        contentItem: ColumnLayout {
            TextField { id: renameInput; objectName: "renameSourceInput"; Layout.fillWidth: true; maximumLength: 128; selectByMouse: true; Accessible.name: window.tr("Название подписки", "Subscription name") }
            RowLayout {
                ActionButton { objectName: "renameSourceCancelButton"; text: window.tr("Отмена", "Cancel"); onClicked: renameDialog.close() }
                ActionButton { objectName: "renameSourceConfirmButton"; text: window.tr("Сохранить", "Save"); enabled: renameInput.text.trim().length > 0 && !window.state.busy; onClicked: { window.send({command: "rename_source", id: renameDialog.sourceId, name: renameInput.text.trim()}); renameDialog.close() } }
            }
        }
    }
    DarkDialog {
        id: deleteDialog
        objectName: "deleteSourceDialog"
        property var sourceId: null
        property string sourceName: ""
        modal: true; anchors.centerIn: parent; width: Math.min(window.width - 32, 440)
        title: window.tr("Удалить подписку?", "Delete subscription?")
        closePolicy: Popup.CloseOnEscape
        contentItem: ColumnLayout {
            PlainLabel { Layout.fillWidth: true; text: deleteDialog.sourceName; font.bold: true }
            SmallLabel { Layout.fillWidth: true; text: window.tr("Подписка и её серверы будут удалены с этого устройства. Для повторного добавления понадобится ссылка.", "This subscription and its servers will be removed from this device. You will need its link to add it again.") }
            RowLayout {
                ActionButton { objectName: "deleteSourceCancelButton"; text: window.tr("Отмена", "Cancel"); onClicked: deleteDialog.close() }
                ActionButton { objectName: "deleteSourceConfirmButton"; text: window.tr("Удалить", "Delete"); enabled: !window.state.busy; onClicked: { window.desiredProfileId = null; window.desiredSourceId = null; window.send({command: "delete_source", id: deleteDialog.sourceId}); deleteDialog.close() } }
            }
        }
    }
    DarkDialog {
        id: addDialog
        objectName: "addSourceDialog"
        anchors.centerIn: parent
        width: Math.min(window.width - 32, 540)
        modal: true
        title: window.tr("Добавить подписку", "Add subscription")
        closePolicy: Popup.CloseOnEscape
        onClosed: { sourceName.clear(); sourceUrl.clear(); sourceHwid.checked = false; localException.checked = false; sourceAdvanced.checked = false; reveal.checked = false }
        contentItem: PanelScroll {
            id: addScroll
            implicitHeight: Math.min(addContent.implicitHeight, window.height - 140)
            ColumnLayout {
            id: addContent
            width: addScroll.availableWidth
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
                        window.desiredSourceId = null
                        window.desiredProfileId = null
                        searchDelay.stop()
                        window.searchQueued = false
                        window.page = 1
                        window.send({command: "import_source", name: sourceName.text.trim() || window.tr("Подписка", "Subscription"), url: sourceUrl.text.trim(), local_exception: localException.checked, hwid_enabled: sourceHwid.checked, route: "direct"})
                        sourceUrl.clear()
                        addDialog.close()
                    }
                }
            }
        }
        }
        onOpened: { sourceUrl.clear(); sourceName.clear(); sourceHwid.checked = false; if (window.pasteOnOpen) sourceUrl.paste(); window.pasteOnOpen = false; sourceUrl.forceActiveFocus() }
    }
    DarkDialog {
        id: reviewDialog
        objectName: "refreshReviewDialog"
        anchors.centerIn: parent
        width: Math.min(window.width - 32, 520)
        title: window.tr("Проверка импорта", "Review import")
        modal: true
        closePolicy: Popup.CloseOnEscape
        onRejected: window.send({command: "discard_refresh"})
        contentItem: PanelScroll {
            id: reviewScroll
            implicitHeight: Math.min(reviewContent.implicitHeight, window.height - 140)
            ColumnLayout {
            id: reviewContent
            width: reviewScroll.availableWidth
            spacing: 12
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
}
