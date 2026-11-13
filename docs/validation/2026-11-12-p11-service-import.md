# P11: общий типизированный отказ импорта scheduler

Независимая база: `b79f902def680fdf9e60df01cf08fd771ddcd653`.
`SubscriptionError::Import(_)` получает единственную общую причину
`scheduler_service_import_failed`, stage `scheduler_service`, code `IMPORT`,
legacy/message key `import`, slots `error/source`, generation required, params `{}`.
Подварианты Import не различаются; Fetch сохраняет прежнее поведение.
Существующее сообщение IMPORT используется без изменения переводов.

Integration harness `subscription_import_failure` подаёт контролируемые HTTP
ответы через настоящий loopback FetchHost и production parser/service/scheduler.
Валидный body проходит Review/commit; malformed body возвращает настоящий
Failed Import. Профиль остаётся выбираемым, revision/raw original/metadata
совпадают. Оба scheduler завершаются, server joined, service closed.

GUI example `p11schedulerImportE2E` использует существующий PolicyBus на private
D-Bus без activation и system provider с `with_system_subscription_policy(true)`.
Это контролируемый platform input, не проверка NetworkManager хоста.
Транспорт — настоящий native loopback HTTP; body контролирует стенд.
В production DesktopHost загружается persisted due source; ошибки/Cause/state
не внедряются. После закрытия GUI example повторно открывает encrypted repository,
сверяет original/revision и закрывает service. Положительный `valid` case дошёл до pending Review без diagnostic failure
и завершил lifecycle. Native clipboard RU/EN и independent strict V2 прошли
на malformed case; оба GUI case подтвердили closed=true и сохранность original.

QML harness использует production Bridge, exporter и настоящий Qt clipboard
RU/EN; функцию errorText извлекает из production Main.qml. Проверка strict V2
независимым Python validator обязательна. QML harness не является проверкой
неизменённой production страницы настроек.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/p11-service-import-cause/`.
Desktop library: 90 PASS, 1 ignored (существующий gate с pinned engines).
Integration parser/service/scheduler PASS; Python V1/V2 15 PASS; fmt и Ruff PASS.
Clippy MSRV85 обоих пакетов `--all-targets -- -D warnings` PASS.
Полный P11, неизменённая production страница настроек и реальный NM хоста
этим slice не заявляются.

GUI proof использует сохранённый `p11schedulerImportE2E` с build ID
`0.1.0+b79f902def68-dirty` и source snapshot `source-verified.tar.gz`.
После успешных кампаний только стендам добавлены явные трёхсекундные deadlines
server join/service close. Финальные исходники находятся в
`source-final-lifecycle.tar.gz`; финальная сборка хранится отдельно как
`p11schedulerImportE2E-final`. Кампании на этой версии не повторялись:
GUI доказательство не приписывается другому бинарнику задним числом.

Первая GUI compile ошибка была отсутствующим rt-multi-thread; исправлена
current-thread runtime в управляемом std::thread без изменения dependencies.
Первые GUI timeouts вызваны длинным XDG_RUNTIME_DIR (Unix socket path Qt);
успешные кампании используют короткий приватный runtime в /tmp.
Первая clippy попытка ошибочно выбирала Rust 1.97 driver; MSRV85 frontend
с явным cargo wrapper/clippy-driver 1.85.1 прошёл. Все исходные failed logs
сохранены отдельно. Окончательные hashes и cleanup фиксируются в evidence.
