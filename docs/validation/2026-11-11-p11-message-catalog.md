# P11: ограниченный каталог диагностических сообщений

База: `95e88474c0b4a18797dc28fa873a4b6b3adf2fa4`. Reference GUI88:
`9b5872c6f0b6ef8c5c03b32c6152eadd1b07b75b`; фрагмент сохранён в
`docs/evidence/p11-gui88-error-reference.txt`.

Canonical manifest: `docs/specs/diagnostic-message-catalog.json`, catalog_version=1.
В нём 74 пары code/message_key с RU/EN. Rust включает файл при компиляции;
Python читает этот же файл только в самостоятельном валидаторе. Producer не
запускает Python. Контракт экспорта остаётся schema_version=1: набор допустимых
кодов расширен монотонно; старые CANCELLED/CLEANUP_FAILED и допустимые schema1
документы сохранены. catalog_version версионирует закрытый словарь, а не форму JSON.

Producer выводит stage=operation, retryable=false, params={}. Это консервативная
семантика отсутствия автоматического повтора для всех кодов, даже когда текст
предлагает пользователю повторить Stop или проверку. System timeout/owner lost
не разрешают RetryApply: исход неизвестен, требуется сверка состояния. Валидатор
сохраняет старый контракт retryable:boolean; bool не принимается вместо int версии.

Выбираются только error, connection_error, source_error, routing_ack.error.
Наличие health/TCP/import ключа в словаре не означает сбор всей истории результатов:
отдельные probe_result/tcp_result/review rows не экспортируются автоматически.
Нейтральные http/reachable/not_applicable, имена протоколов и ввод пользователя
не считаются отсутствующими переводами ошибок.

Пределы producer: 1 MiB UTF-8 на вход, глубина 64 до разбора, повторные ключи
отклоняются во всём JSON, максимум четыре уникальные ошибки, выход ≤65536 bytes.
Валидатор: вход/выход ≤65536 bytes, depth≤4, ≤128 ошибок, точные поля и типы,
повторные ключи запрещены. Неизвестные значения и типы отклоняются до вывода.
Свободные параметры, URL, source, native config и raw cause не копируются.

| code | message_key | RU/EN и происхождение |
| --- | --- | --- |
| LISTEN_PORT_BUSY | listen_port_busy | Main.qml errorText/editorMessages; обе локали в manifest |
| STARTUP | startup | Main.qml errorText/editorMessages; обе локали в manifest |
| KEYRING_LOCKED | keyring_locked | Main.qml errorText/editorMessages; обе локали в manifest |
| KEYRING_MISSING | keyring_missing | Main.qml errorText/editorMessages; обе локали в manifest |
| STORAGE | storage | Main.qml errorText/editorMessages; обе локали в manifest |
| PREFERENCES_WRITE | preferences_write | Main.qml errorText/editorMessages; обе локали в manifest |
| INVALID_INPUT | invalid_input | Main.qml errorText/editorMessages; обе локали в manifest |
| BUSY | busy | Main.qml errorText/editorMessages; обе локали в manifest |
| SOURCE_IN_USE | source_in_use | Main.qml errorText/editorMessages; обе локали в manifest |
| REFRESH_REQUIRED | refresh_required | Main.qml errorText/editorMessages; обе локали в manifest |
| SOURCE_REQUIRED | source_required | Main.qml errorText/editorMessages; обе локали в manifest |
| UNSUPPORTED_PROXY | unsupported_proxy | Main.qml errorText/editorMessages; обе локали в manifest |
| HWID_REQUIRED | hwid_required | Main.qml errorText/editorMessages; обе локали в manifest |
| HWID_LIMIT | hwid_limit | Main.qml errorText/editorMessages; обе локали в manifest |
| HWID_REJECTED | hwid_rejected | Main.qml errorText/editorMessages; обе локали в manifest |
| HWID_REDIRECT | hwid_redirect | Main.qml errorText/editorMessages; обе локали в manifest |
| FETCH | fetch | Main.qml errorText/editorMessages; обе локали в manifest |
| IMPORT | import | Main.qml errorText/editorMessages; обе локали в manifest |
| REVIEW_REQUIRED | review_required | Main.qml errorText/editorMessages; обе локали в manifest |
| PROFILE_REQUIRED | profile_required | Main.qml errorText/editorMessages; обе локали в manifest |
| PROFILE_UNAVAILABLE | profile_unavailable | Main.qml errorText/editorMessages; обе локали в manifest |
| ENGINE_MISSING | engine_missing | Main.qml errorText/editorMessages; обе локали в manifest |
| SYSTEM_UNSUPPORTED | system_unsupported | GUI88 errorText + system_error(ClientError); обе локали в manifest |
| SYSTEM_DENIED | system_denied | GUI88 errorText + system_error(ClientError); обе локали в manifest |
| SYSTEM_OWNER_LOST | system_owner_lost | GUI88 errorText + system_error(ClientError); обе локали в manifest |
| SYSTEM_TIMEOUT | system_timeout | GUI88 errorText + system_error(ClientError); обе локали в manifest |
| SYSTEM_UNAVAILABLE | system_unavailable | GUI88 errorText + system_error(ClientError); обе локали в manifest |
| ENGINE_CONFIG | engine_config | Main.qml errorText/editorMessages; обе локали в manifest |
| ENGINE_FAILED | engine_failed | Main.qml errorText/editorMessages; обе локали в manifest |
| PROBE_TARGET_INVALID | probe_target_invalid | Main.qml errorText/editorMessages; обе локали в manifest |
| CHECK_HTTP_STATUS | check_http_status | Main.qml errorText/editorMessages; обе локали в manifest |
| CHECK_TLS | check_tls | Main.qml errorText/editorMessages; обе локали в manifest |
| CHECK_TIMEOUT | check_timeout | Main.qml errorText/editorMessages; обе локали в manifest |
| CHECK_TRANSPORT | check_transport | Main.qml errorText/editorMessages; обе локали в manifest |
| CHECK_FAILED | check_failed | Main.qml errorText/editorMessages; обе локали в manifest |
| NETWORK_CHANGED | network_changed | Main.qml errorText/editorMessages; обе локали в manifest |
| TCP_FAILED | tcp_failed | Main.qml errorText/editorMessages; обе локали в manifest |
| CANCELLED | cancelled | Main.qml errorText/editorMessages; обе локали в manifest |
| CLEANUP_FAILED | cleanup_failed | Main.qml errorText/editorMessages; обе локали в manifest |
| ROUTING_STALE | routing_stale | Main.qml errorText/editorMessages; обе локали в manifest |
| ROUTING_ACK_IDENTITY | routing_ack_identity | Main.qml errorText/editorMessages; обе локали в manifest |
| ROUTING_STORAGE | routing_storage | Main.qml errorText/editorMessages; обе локали в manifest |
| DOMAIN | domain | Main.qml errorText/editorMessages; обе локали в manifest |
| CIDR | cidr | Main.qml errorText/editorMessages; обе локали в manifest |
| EXECUTABLE | executable | Main.qml errorText/editorMessages; обе локали в manifest |
| BROAD | broad | Main.qml errorText/editorMessages; обе локали в manifest |
| ID | id | Main.qml errorText/editorMessages; обе локали в manifest |
| LIMIT | limit | Main.qml errorText/editorMessages; обе локали в manifest |
| MALFORMED_URI | malformed_uri | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| UNSUPPORTED_PROTOCOL | unsupported_protocol | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| INVALID_PORT | invalid_port | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| INVALID_HOST | invalid_host | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| INVALID_CREDENTIALS | invalid_credentials | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| INVALID_ENCODING | invalid_encoding | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| INVALID_JSON | invalid_json | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| TOO_LARGE | too_large | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| DUPLICATE_PARAMETER | duplicate_parameter | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| CONFLICTING_PARAMETER | conflicting_parameter | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| UNKNOWN_PARAMETER | unknown_parameter | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| UNSUPPORTED_TRANSPORT | unsupported_transport | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| UNSUPPORTED_SECURITY | unsupported_security | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| UNSUPPORTED_CIPHER | unsupported_cipher | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| UNSUPPORTED_OPTION | unsupported_option | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| UNSAFE_OPTION | unsafe_option | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| LEGACY_OPTION | legacy_option | reviewText / IssueCode, ReviewFlag; обе локали в manifest |
| RESOLVER_UNAVAILABLE | resolver_unavailable | tcpStatusText / TcpProbeStatus; обе локали в manifest |
| DNS_FAILED | dns_failed | tcpStatusText / TcpProbeStatus; обе локали в manifest |
| CONNECT_FAILED | connect_failed | tcpStatusText / TcpProbeStatus; обе локали в manifest |
| DNS_TIMEOUT | dns_timeout | tcpStatusText / TcpProbeStatus; обе локали в manifest |
| CONNECT_TIMEOUT | connect_timeout | tcpStatusText / TcpProbeStatus; обе локали в manifest |
| TLS | tls | probeResultText / ProbeResult; обе локали в manifest |
| TIMEOUT | timeout | probeResultText / ProbeResult; обе локали в manifest |
| TRANSPORT | transport | probeResultText / ProbeResult; обе локали в manifest |
| CATALOG | catalog | DesktopError("catalog"); отдельного Main.qml перевода нет; обе локали в manifest |

Плановые категории data-and-contracts (например DNS_POLICY_AMBIGUOUS/UNRESOLVED)
не подменяются вымышленными текущими GUI фабриками; они остаются отдельным
контрактом policy diagnostics. Сырые события ядра — FR16/PR84, вне этого среза.

Остатки P11: Main.qml использует общий fallback для ошибки catalog;
system-переводы существуют пока только в reference GUI88, который ещё не merged.
Произвольные тексты сторонних ядер не считаются переведённым UI.
Полный P11 этим PR не закрывается.

Проверки: RED известной ошибки → GREEN 74 ключей; Rust diagnostics 4 PASS,
Python export 10 PASS, translations 10 PASS. Проверка переводов охватывает
206 inline-пар, 86 ключей карт в обоих собственных QML и 9 существующих
C++ tray-пар QStringLiteral; прежние fail-closed проверки форм карт сохранены.
Ruff check/format, cargo fmt и git diff --check проходят.

QtTest: один поведенческий тест (обе локали, успешный/неуспешный backend stub)
и отдельно init/cleanup. Не интерпретировать QtTest total=3 как три сценария.
ActualCopy: production GUI binary и production Rust/CXX-Qt copy_diagnostics,
14 положительных проверок (7 ключей × RU/EN) и 5 отрицательных значений
с сохранением clipboard. Сценарий не вызывает initialize DesktopHost:
это контролируемый GUI snapshot, не настоящий DesktopHost failure.
Отдельный процесс использует offscreen и private D-Bus, host clipboard не затронут.
Тестовый LD_PRELOAD перехватывает только загрузку стартового QML и загружает
сценарий; production исходники/API не меняются. QT_FORCE_STDERR_LOGGING=1
обеспечивает проверяемые маркеры выполнения; без маркеров runner отказывает.

Логи находятся в docs/evidence/p11-*.log. Повторяемые команды:

```sh
cargo test --offline --locked -p yonik-desktop --lib diagnostics
cargo build --offline --locked -p yonik-desktop --features gui
cargo clippy --offline --locked -p yonik-desktop --features gui --all-targets --no-deps -- -D warnings
python3 -m unittest discover -s tools/tests -p test_check_diagnostic_export.py -v
python3 -m unittest discover -s tests -p test_check_translations.py -v
python3 tools/check_translations.py
QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software dbus-run-session -- python3 tools/tests/check_diagnostic_actual_copy.py BINARY TASK_DIR
```

Все Cargo-команды выполнялись через with-rust-slot с jobs=1, BSD flock корня
собственного transient task и общим CARGO_TARGET_DIR. GUI build выдаёт
предупреждения GCC16 из Qt headers; Rust lint suppressions не добавлены.
Тестовый binary сохранён вне TTL в local/validation/p11-message-catalog.
Полный clippy зависимостей не заявляется: проверка ограничена --no-deps.


GUI Clippy --all-targets --no-deps -D warnings: PASS (4m06s).
Собственный target очищен cargo clean: 8364 файла, 4.4 GiB; task-каталог
удалён после проверки отсутствия процессов/открытых ссылок. Binary SHA-256:
`410a41199ed20cfe945eb290baeb33bb395c971a59e6090571ea10c67cdcf650`.
