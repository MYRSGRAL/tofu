# Явный системный путь DesktopHost и Qt

Исходная локальная база: `b6352e988161b6f733e6d1ac99e1c70d9395610a`.
Она объединяет main `c21422f` и зависимый PR76 `d9351bc` обычным merge.
Это не свидетельство принятия PR76 в main. Известный P2 bulk preparation
исправляет отдельный writer; до интеграции его fix combined acceptance открыт.

Backend принимает `set_connection_mode` (`local`/`system`), `system_state`
и `system_unlock`. Начальный режим — Local SOCKS. Изменение режима запрещено
при активной/неопределённой сессии, RPC и сохранённой блокировке. Выбор System
не запускает LocalSession и не выдаёт ProxyLease для subscription fetch.
Существующие preferences, batch producer и network providers не меняются.

Один отменяемый Flight владеет SystemSession во время RPC; очередь команд
DesktopHost остаётся ограниченной. Prepare/Apply используют один deadline 30 с,
типизированный ServerIntent и одноразовый handle существующего клиента.
Capability gate проверяется до Prepare. Production `system_tun` не включается.
Stop/Shutdown отменяют текущий RPC через отдельный priority watch, затем
выполняют Stop; неопределённый исход остаётся Unknown. Неудачная остановка
сохраняет can_stop и препятствует подтверждённому закрытию host.

Completion сверяется с priority generation, source/profile/snapshot revision.
Повтор выбора во время Prepare также отменяет Flight. Поздний ответ RPC не
владеет состоянием GUI. Poll State выполняется отдельной задачей; потеря owner
публикует Unknown до нового запроса State. Новый клиент привязывается к текущему
owner; старые handles не переносятся.

TunnelStarting, VerifiedRules, RecoveryRequired и NoManagedPolicy — только
подтверждённые состояния helper. Они не означают transport readiness/Connected.
После Stop сохранённая публичная политика показывается явно; новый Prepare
допустим без Unlock. Отдельный Unlock снимает её по явному действию пользователя.
QML показывает snapshot backend и не добавляет собственного reducer.

## Проверка

До реализации: 22 desktop unit tests (включая batch), семь private-bus
SystemClient tests. RED: JSON-команда нового режима отклонялась invalid_input;
GREEN: regression явного переключения System/Local прошёл.

`tests/system_smoke.py` запускает настоящий Qt offscreen и DesktopHost с отдельной
синтетической D-Bus-службой через DBUS_SYSTEM_BUS_ADDRESS только дочернего процесса.
Import использует собственный loopback HTTP. Выбор режима клавишей ComboBox,
Connect/Stop, State/Unlock идут через настоящие UI actions и CXX bridge.
Сервис не является production helper и не создаёт TUN, nft/RPDB, worker или
host polkit dialogs. Проверки этого транспорта не доказывают actual TUN.

Сценарии: повтор Connect с retained policy, capability=false без Prepare/Apply,
denial, отмена долгого Prepare/Apply, stale выбор, Shutdown, потеря owner,
RecoveryRequired и deadline. Отмена проверяет Stop раньше завершения долгого RPC;
поздние ответы не приводят к Apply/Connected. Каждый GUI-процесс ограничен 60 с.
Логи и бинарник сохраняются вне TTL в
`/home/myrs/projects/yonik/local/validation/system-controller/`.

Actual production GUI VM, transport readiness и полные G-DNS/G-APP gates
остаются открыты. Этот срез не закрывает FR03/P09/P19 целиком.

Финальные проверки: desktop unit 22/22, backend integration 34 passed / 1 ignored,
SystemClient 7/7. Scoped desktop clippy (`--all-targets --features gui --no-deps
-- -D warnings`) прошёл. Полный clippy текущего stable 1.97 останавливается
на существующем `collapsible_match` в `yonik-engine/src/lib.rs:165`; engine
в этом срезе не исправлялся. Rust fmt, Python ruff format/check и diff check
включены в проверку. Smoke harness получает 45 секунд вместо 30 только в
smoke-test, чтобы deadline RPC 30 секунд успел завершиться до общей отмены.

Дополнительные regression guards: отмена read-only State не вызывает Stop при
отсутствии активной/неопределённой мутации (в том числе после capability=false).
Ответ Stop=Unknown сохраняет can_stop и незавершённый lifecycle до State/recovery;
успех RPC сам по себе не подтверждает завершение worker.

Private-Qt acceptance включает 11 сценариев. В свежем прогоне задержанный
Prepare/Apply равен 2 секундам, а RPC Stop приходит примерно через 0,20 секунды
после начала Prepare/Apply (cancel, applycancel, stale, shutdown). Для deadline
fixture задерживает Prepare на 35 секунд при controller deadline 30 секунд.
Stop=Unknown отдельно проверяет сохранение can_stop до подтверждённого State.
После Unlock финальный статус normal/recovery/cancel/applycancel/stale —
NoManagedPolicy. Ни один сценарий не публикует Connected.
