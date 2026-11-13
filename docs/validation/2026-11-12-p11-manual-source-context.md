# P11: companion ручных source failures

База: `79f589a23d900af4ccf184413438b21985b877a0`.

Внутренний SourceFailure передаётся через результат конкретной команды/refresh.
Public DesktopError, command JSON, legacy keys, SourceChange ACK и маршруты slots
сохранены. Поля последней причины в actor нет. 24 typed callsites: 23 прежних
map_subscription в ручном source pipeline и отдельный select_profile mapper.
RoutingApply остаётся вне этого batch. Startup/scheduler вызовы общих page/reload
сохраняют прежнюю legacy-публикацию; scheduler Fetch/Import и maintenance здесь
не реализуются повторно.

Закрытый набор: 8 SourceService по SubscriptionError, 7 SourceRepository,
6 SourceFetch по существующим legacy-группам, одна SourceSelectionRepositoryFailed
и одна SourceServiceImportFailed. Import подтверждён настоящим parser failure.
SourceService допускает Error/Source/Connection; Fetch и Selection — Error/Source.
Connection сохранён для общего outcome SetSubscriptionSchedule, его service
preflight публикуется в Error/Source. Params пусты, retryable=false. Причины не
выводятся из storage/fetch строк; errno и низкоуровневые selection детали не создаются.

Новые stages требуют command_generation. Refresh completion сохраняет исходное
поколение task; после await публикация проверяет current generation. ScheduleFailure
из продления refresh interval сохраняет scheduler stage/generation. Legacy context
захватывается до reconcile/reload и ограничен собственными producer stages:
более поздний scheduler storage не становится причиной primary manual storage.
Primary manual outcome заменяет reload failure; async reconcile после completion
сохраняет прежний приоритет последней ошибки.

RED: настоящий AddSource с непригодным URL вернул invalid_input и 0 contexts вместо 2.
Первый fixture ошибочно завершался на dispatch validation; исправленный достигает
SubscriptionSource. Ещё один отвергнутый fixture закрывал только clone handle,
поэтому не создавал закрытую БД. Итоговый repository fixture переименовывает таблицу
только собственной временной БД и получает реальный RepositoryError::Storage.
Он проверяет reload/source_page/query/select/rename/HWID/delete, refresh admission
и service preflight schedule. Другие actual host cases: rename NotFound,
selection, SourceChange request ACK и manual Import с непригодным HTTP body.

Конечный enum batch проверяет 8 service + 7 repository + все 21 FetchError
(сведённые в 6 причин), selection repository group, legacy key parity, generation,
slot/request/code/stage rejection, same-key replacement и scheduler precedence.
Матрица V2 проверена production exporter и независимым Python validator.
Пять actual host metadata exports сохранены вместе с RED/GREEN логами и manifest
295 Rust/TOML/lock/matrix inputs вне TTL:
`/home/myrs/projects/yonik/local/validation/p11-manual-source/`.

Проверено Rust 1.85.1, offline/locked, jobs=1, own root flock и with-rust-slot:
98 library PASS (1 existing ignored), 57 backend PASS (2 existing ignored),
clippy --all-targets -- -D warnings PASS, fmt PASS, 17 Python PASS.
Новые зависимости, pins, unsafe и native ABI не добавлены. GUI foreach campaign,
полный workspace test suite и фактическое воспроизведение исчерпания счётчиков
не заявляются. Собственный Cargo target очищается после сохранения evidence.
