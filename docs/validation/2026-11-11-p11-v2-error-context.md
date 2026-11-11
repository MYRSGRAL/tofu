# P11: typed context и отдельный безопасный экспорт V2

Продолжение frozen `411fb6ba3a80f148c641564ccfe21f724e1fe79e` / PR121.
Интеграционный commit `2034ce8` включает main `b9e5071` с PR107/117/118.
Исходный native base — `6cf65a4cf9760a7df68b8eee10f012770b38604d`.

## Контракт

Старый DesktopError, строковые error/ACK, actor IDs, signature copy_diagnostics и
форма schema1 сохраняются. GUI вызывает новый copy_diagnostics_v2 через тот же
clipboard FFI; пользователь не выбирает версию. При отсутствии проверенных
метаданных интерфейс сообщает об этом явно, без общего «unknown error».

Пример одного элемента реального storage export (полный trace содержит слоты
error и connection):

```json
{"schema_version":2,"platform":"linux","errors":[{"slot":"connection","code":"STORAGE","message_key":"storage","request_id":null,"generation":null,"stage":"storage_open","reason":"data_directory_permissions","retryable":false,"params":{}}]}
```

Закрытые slots: error, connection, source, routing_ack, maintenance_ack,
logs_read, logs_save. Логи имеют отдельные каналы чтения и сохранения даже при
одинаковом request ID. Закрытые Stage/Cause заданы Rust enum;
`docs/specs/diagnostic-context-v2.json` содержит 89 точных сочетаний
reason/code/key/stage, допустимые slots и требование generation.
Gate сверяет enum, матрицу и canonical message catalog (78 ключей,
catalog_version=3). Unknown cause не заменяется предположением.

request_id/generation сериализуются из u64 в canonical decimal string|null:
0 либо число без ведущих нулей, максимум 20 ASCII digits, не больше u64::MAX.
Старые logs IDs остаются строками; V2 отклоняет неканонические `01`/`+1`, а не
сливает их с sibling ID `1`. Старый logs API не меняется.

Вход snapshot ≤1 MiB, глубина ≤64, duplicate keys запрещены во всём JSON.
Contexts/errors ≤16, выход ≤65536 bytes, params строго {}. В context разрешены
только slot/code/message_key/request_id/generation/stage/reason/retryable/params.
Экспортёр сверяет весь элемент с typed factory, допустимым каналом, текущим
строковым error/ACK, ID и имеющейся generation. Отсутствие любого нужного
контекста, неизвестное поле/enum, type mismatch или canary отклоняет весь export
до clipboard FFI. Строгий versioned validator — tools/check_diagnostic_export_v2.py;
V1 validator не изменён. Обычные поля snapshot, включая native/source/URL,
не включаются в export. retryable=false не разрешает автоматический повтор.

## Producer coverage на интегрированной базе PR107

| Producer | Typed metadata / точная граница | Доказательство и ограничения |
| --- | --- | --- |
| startup actor | panic capture → runtime_panicked; signal registration → signals_unavailable; listen port → listen_port_unavailable | реальные ветви producer, без panic payload; фактический GUI proof ниже проверяет startup storage |
| initial storage | отдельные path/create/metadata/type/owner/permissions/canonicalize/open/lock guards; spawn join; каждый RepositoryError при open; каждый CredentialError | data_directory_permissions ставится только на mode guard; generic Storage/Conflict остаются repository_open_storage/conflict; host keyring не вызывается тестами |
| preferences write | закрытые std::io::ErrorKind permission_denied/already_exists/not_found/invalid_input/other | capture до DesktopError; остальные ErrorKind fail-closed |
| catalog | JoinError → catalog_task_failed; categorical editor catalog refusal → catalog_query_failed | последний обозначает отказ catalog API, а не выдуманный errno или путь |
| local core | typed start failure/early exit; HostError::EngineExited из wait_exit → engine_run/engine_exited | настоящий pinned процесс, SHA-256 проверен через /proc/exe и pidfd перед SIGKILL |
| cleanup | typed stop failure / удерживаемая failed session → cleanup/cleanup_failed | production wiring; GUI proof только controlled negative существующего ephemeral-only hook |
| helper | каждый ClientError отдельно, включая deadline vs timeout, invalid_request/handle, stale/protocol | actual отсутствующий сервис на private system bus; production helper/VM denial и owner-loss здесь не доказаны |
| routing ACK | request/revision/rule identity guards; RepositoryError::Conflict vs остальные ошибки save | exact routing_ack + ID + command generation; editor validation и отказ до admission без typed cause fail-closed |
| maintenance ACK | private Outcome хранит MaintenanceError / RepositoryError copy/access/reopen до строкового result | matching request и принятая generation; generic RepositoryError::Conflict → maintenance_copy_conflict, без выдуманного DiskFull/Busy cause |
| logs read/save | private Page/SaveJob companion хранит точный ErrorKind до status; отдельные read/save busy и page identity/unavailable | matching request/status fence сохранён; неизвестный ErrorKind не экспортируется; actual save conflict → logs_io_already_exists |

Clear/replacement удаляют companion одновременно с заменой legacy полей,
включая замену тем же ключом. Routing/maintenance ACK и logs page replacements
сбрасывают прежний context. Session update проверяет generation до изменения
строки или контекста. Helper context появляется только в прежней fenced
completion-ветке. Maintenance completion принадлежит прежнему одному join handle;
CAS/drain/controller/cleanup guards не переписаны. SaveJob companion применяется
только внутри прежнего request_id/status==saving fence.

report_dispatch_error очищает context при полной замене snapshot;
report_error очищает затронутый error slot. Regression same-key/new-failure и
unknown replacement подтверждает отсутствие повторного использования старой
причины. Частичный unknown JSON не восстанавливает удалённый context.

## Actual GUI proof

Все сценарии запускаются с собственными путями, offscreen/software Qt,
private D-Bus, user/network/PID namespaces. Loopback поднимается только внутри
нового netns. Интерпозер заменяет начальный QML; initialize/dispatch, failure
producer, raw snapshot, Rust V2 exporter и Qt clipboard остаются настоящими.
errorText извлекается из текущего Main.qml; обе локали сравниваются с catalog.
Положительный snapshot не инъецируется. После положительного copy отдельно
подставляются controlled negative context canaries.

| Сценарий | Фактический trigger | V2 proof |
| --- | --- | --- |
| startup storage | свой каталог mode=0755, отказ до keyring | storage_open/data_directory_permissions, RU+EN, оба error slots |
| core exit | свой запущенный pinned Xray, проверка pin через pidfd, SIGKILL | engine_run/engine_exited, generation="1", RU+EN |
| helper failure | настоящий SystemClient на private bus без helper owner | helper_rpc/helper_unavailable, RU+EN |
| actual SaveJob | существующий собственный файл назначения | logs_save/logs_io_already_exists, logs_save slot, ID="101", RU+EN |
| actual maintenance | настоящий copy_closed backup в существующий собственный файл | maintenance_copy/maintenance_copy_conflict, maintenance_ack, ID="201", RU+EN |
| controlled cleanup negative | существующий test-support hook + собственный ephemeral key, release через stdin | cleanup/cleanup_failed, RU+EN; не объявляется production cleanup failure |

В каждом сценарии URL/raw/control canaries, неизвестное поле и mismatch
контекста отклоняются с сохранением прежнего clipboard. Runtime ACK сценарии
дополнительно проверяют sibling ID и чужой channel. Отдельный Rust test выполняет
настоящий maintenance conflict и отвергает sibling ACK ID при неизменной V1 форме.
Простой u64::MAX ACK regression доказывает точность canonical string без Qt/JS
округления. Qt6 qmltestrunner проверяет реальный DiagnosticsExport button со stub
success/failure; этот результат отделён от actual DesktopHost scenarios.

Evidence и binaries вне TTL:
`/home/myrs/projects/yonik/local/validation/p11-v2-f1fcd/`.
Там final logs, source/binary manifests, точные использованные QML fixtures и
повторяемый verify-final.sh. Production binary собран с features=gui;
controlled cleanup example — отдельно с gui,test-support. Pins и feature gates
не менялись. Исполняемый Xray pin:
`cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.

## Оставшиеся границы полного P11

Полный P11 не заявляется. V2 сознательно fail-closed для failures, в которых
разрешённая producer-граница уже получила только legacy string: части
subscription/probe/scheduler ошибок и admission/refusal ACK без typed cause;
также для неизвестных I/O kinds и неизвестных drain/scheduler causes.
Они не получают ложный stage/reason по общему storage key.

Конкретный предел ownership для следующего полного покрытия: typed scheduler
cause теряется внутри защищённого DesktopSchedule до backend; thread-start
failure передаётся в main.initialize только как прежний DesktopError, а
разрешённые изменения initialize не включают новый contextual start API.
Их покрытие требует отдельного узкого согласования исходной producer-границы,
а не вывода причины из ключа. Maintenance controller admission остаётся
прежним; без его typed companion V2 не выдумывает причину refusal.

Live helper timeout/denied/owner-loss требуют следующего отдельного actual gate.
Узкое предложение перед VM: использовать существующую disposable helper VM,
production helper и State RPC; создать owner-loss остановкой только гостевого
helper, затем проверить unchanged recovery fences и V2 metadata. Host service,
keyring и network settings не затрагивать. Такой VM gate здесь не запускался.

## Финальные проверки и артефакты

Rust 1.85.1 `--offline --locked`: library 59 PASS; тот же уже построенный test
binary — 59 PASS в user/net/PID namespace с private session/system bus и своим
TMPDIR. Для прямого test binary рабочий каталог — crates/yonik-desktop.
Локальный wrapper cargo требует host user-scope bus, поэтому внутри namespace
выполнялся собранный artifact, без повторной компиляции и через with-rust-slot.

Clippy `--all-targets -- -D warnings` отдельно для gui и gui,test-support PASS,
без --no-deps. GCC16/Qt headers печатают C++ warnings; Rust suppressions не
добавлены. Python V1 10 PASS, V2 5 PASS, translations 18 PASS, factory gate PASS;
Ruff check/format и cargo fmt PASS. Qt6 qmltestrunner: один поведенческий тест
RU/EN success/failure плюс init/cleanup, total=3 PASS.

Actual traces: 12 V2 exports прошли независимый strict V2 validator; 2 actual
storage exports прошли старый V1 API (schema1, одна deduplicated ошибка).
Все шесть финальных GUI flows проверили сохранение clipboard на отказе.

Production GUI SHA256:
`d154530e7248fdd4f9b206c10767687a8dfd79845a98cda1d6a945c6ef3dc913`.
Controlled cleanup binary SHA256:
`eb9203842ec46aea05456362952326b3c9b6e4d129f8bdcb6bae0af55fbcaeab`.
Exact source/fixtures/binary manifests и actual-export-traces.json сохранены
в указанном каталоге evidence вне TTL. Неуспешные начальные попытки fixture
не объявляются PASS: несовместимый wrapper cargo в namespace, рабочий каталог
прямого test artifact, формат smoke script и template placeholder исправлены
в runner/командах; финальные logs имеют суффикс final.

Перед очисткой проверены /proc cwd/exe/fd: 0 активных ссылок; ldd production
binary не содержит target paths. Cargo clean выполнен успешно; собственный
transient root удалён после повторной проверки отсутствия ссылок.
Removed 11534 files, 5.3GiB total
