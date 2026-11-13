# P11: конечный integrated-candidate gate

Exact source #172: `6f45329b2e0a9b1659232fd4558d75308a6bc6c7`.
Исторический closure163 не изменён. Новый ledger —
[p11-integrated-candidate.json](p11-integrated-candidate.json).

Критерий каждой строки: final reason → code/key/stage, допустимые slots,
обязательность generation, реальная publication chain и неизменность исходников.
Все 14 строк сверяются с оригинальными 12 boundaries + Import + Startup,
без подсчёта enum как границ. Дополнительно проверяются все Cause-ссылки четырёх
producers; существующий factory gate проверяет legacy keys и QML RU/EN,
strict V2 gate — все reason × slot × generation и отклонение неверной identity.

| Граница | Final cause / stage | Slots и generation |
| --- | --- | --- |
| scheduler_fetch | SchedulerServiceFetchFailed и HWID/UnsupportedProxy / scheduler_service | Error, Source; scheduler generation |
| manual_subscription | SourceService*, SourceRepository*, SourceFetch* / source_service, source_fetch | Error + Source/Connection согласно V2; command generation |
| probe_results | ProbeHttpStatus/Timeout/Tls/Transport / probe | Error, Connection; session generation |
| probe_host_error | ProbeCheckFailed; Cancelled без ошибки / probe | Error, Connection; session generation |
| maintenance_shutdown | MaintenanceSchedulerDrainFailed / maintenance_drain | MaintenanceAck; envelope generation + request_id |
| maintenance_admission_report | MaintenanceSchedulerAdmissionFailed / maintenance_drain | тот же ACK |
| maintenance_service_close | MaintenanceServiceDrainFailed / maintenance_drain | тот же ACK |
| maintenance_repository_close | MaintenanceRepositoryDrainFailed / maintenance_drain | тот же ACK |
| maintenance_reload | MaintenanceScheduleReloadFailed / maintenance_reopen | тот же ACK |
| maintenance_replacement | MaintenanceScheduleReplacementFailed / maintenance_reopen | тот же ACK |
| maintenance_rollback | primary copy/replacement имеет precedence; close failure запрещает drained=true | тот же ACK |
| maintenance_task_join | MaintenanceTaskFailed / maintenance_drain; drained=false | тот же ACK |
| scheduler_import | SchedulerServiceImportFailed / scheduler_service | Error, Source; scheduler generation |
| thread_start_before_actor | BackendThreadStartFailed / startup | Error, Connection; generation отсутствует |

Producer evidence повторно используется с `538ff996507ccc2566d9330e5148e7d4d1767587`:
проверяется равенство desktop blobs этому exact source и source172, SHA-256 полного
локального desktop-tests.log и наличие конкретных PASS producer/fence/V1 тестов.
Полный лог и binary/source binding остаются в
`local/validation/desktop-reviewed-integration/` данного worktree вне TTL.
Новая runtime campaign не заявляется. В shallow CI file hashes проверяются,
историческая ancestry/evidence проверка явно пропускается; полный локальный gate
обязателен для acceptance. Исходники producers совпадают; Rust rerun не требуется.

Конкретного нового потерянного typed producer в этих 14 границах не обнаружено.
Это статический integrated acceptance с прежними scoped producer proofs, а не
доказательство исполнения каждого error arm в GUI. Storage/Transport не получают
выдуманный errno. Unknown fallback не считается потерей известной причины.

Локально: 25 Python tests PASS, RU/EN 374 пары / 160 ключей / 17 QML,
Ruff 0.17.4 format/check PASS. Rust, GUI и 16 новых probes не запускались.
Тяжёлых временных сборок не создавалось.

P11 в main **не closed**. #172 OPEN на момент проверки, observed main
`b199563a32420d309e63bf6fb137dbf8aab08641`. Следующий конкретный gap — принятие
reviewed source172 с успешной CI и подтверждение его ancestry в accepted main.
Изменение любого связанного blob требует обновления evidence и конечного gate.
