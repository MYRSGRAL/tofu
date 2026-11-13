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

Локально после исправления gate: 26 Python tests PASS, RU/EN 373 пары / 104 ключа / 16 QML,
Ruff 0.17.4 format/check PASS. Rust, GUI и 16 новых probes не запускались.
Тяжёлых временных сборок не создавалось.

#172 принят в main commit `d74e61884385cd20f659112182142b05cacb958e` после
чистого review и зелёного CI. Ancestry source172 → observed main
`3e64220fc16b7cfdd2170bd0fffc7049d056c8a7` подтверждена. Merge больше не pending.

Полный P11/native issuer этим gate **не closed**: проверяются исходные 14
границ и прежние scoped proofs, а не будущие helper/native issuers или исполнение
каждого error arm. Следующий gate — отдельная проверка их actual production
publication после завершения admission/issuer. Изменение любого связанного blob
требует обновления evidence и конечного gate.

Полный локальный gate теперь обязательно читает binding и producer log.
Отсутствие любого файла, несовпадение source или SHA256 даёт FAIL; CI с shallow
history по-прежнему явно SKIP исторического evidence, а не локальный acceptance
PASS. Проверено на той же полной Git history: прежний gate без binding давал
exit 0; исправленный — exit 1; после подстановки неизменённых сохранённых
артефактов — PASS. Отдельный negative test проверяет отсутствующее evidence.

Evidence исправления: `/home/myrs/projects/yonik/local/evidence/pr174-acceptance-gate-fix/`.
