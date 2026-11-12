# P11: scheduler producer context

База: `473a53b902d3a6e3ef26f286916ed1018219bc00`.

DesktopSchedule сохраняет закрытый companion до backend: initialize exhaustion /
InvalidLimits, runtime InvalidLimits / Full / Duplicate / Exhausted / Stale.
Ошибки SubscriptionService продолжают передавать только прежний ключ с cause=None;
причина не выводится из storage string. Старые ключи startup/storage и source_error
сохранены. set_interval с прежним invalid_input пока остаётся fail-closed.

Добавлены Stage SchedulerInitialize / SchedulerAdmission и семь точных reasons
в Rust factory и versioned V2 matrix. Params остаются пустыми. Новых message keys,
изменений зависимостей, native/helper ABI, capabilities и pins нет. V1 не меняется.

SchedulerAdmission связан с identity владельца через scheduler_generation в
snapshot; exporter проверяет её для обоих slots error/source. Замена runtime
сразу меняет snapshot generation. Общие begin_command/command_error invalidations
сохранены. Это синхронная граница actor; scheduler futures остаются в прежнем select,
без новых callbacks и без ослабления cancellation / join fences. ACK slots не
разрешены для scheduler causes.

## Приёмка

`actual_restore_full_exports_only_current_generation` создаёт настоящий encrypted
repository с двумя persisted subscriptions и DesktopSchedule capacity=1. Реальный
restore второй записи возвращает Full; snapshot проходит production V2 exporter.
Тест меняет generation, подставляет routing_ack и заменяет failure тем же storage
key без cause: весь export отклоняется. Исходный RED выявил отсутствие snapshot
scheduler_generation fence, после исправления тест проходит.

`actual_initialize_invalid_limits_has_closed_context` вызывает настоящий scheduler
constructor с нулевым capacity и проверяет initialize stage, startup key, отсутствие
generation и экспорт прежних error/source slots. Capacity параметризован только
в private Rust constructor; обычный production new сохраняет лимит 20000.

`tools/tests/check_scheduler_actual_copy.py` принимает сохранённый raw snapshot
этого actual producer test. Он подменяет только QML через прежний interposer,
не запускает DesktopHost и не создаёт production exception. Production Rust V2
exporter, Qt clipboard и извлечённый Main.qml errorText остаются настоящими.
Поэтому это связка actual producer proof + replay через actual clipboard,
а не утверждение об end-to-end переполнении production лимита 20000 через GUI.
Запуск: private D-Bus, offscreen, software Qt, `PYTHONPATH=tools`.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/p11-scheduler-w4/`.

## Оставшиеся границы

Полный P11 не заявляется. Отдельный следующий шаг — typed maintenance controller
admission/refusal ACK. Также остаются subscription/probe failures, storage failures
внутри scheduler service, set_interval invalid_input, thread-start failure в
main.initialize (legacy DesktopError), неизвестные I/O kinds и drain causes.
Live helper timeout/denied/owner-loss требуют отдельного actual VM gate.
Существующие непокрытые failures продолжают fail-closed; scheduler причины им
не присваиваются. Runtime Exhausted и initialize owner exhaustion связаны с
producer, но фактическое исчерпание u64 здесь не воспроизводилось.

Clipboard replay: RU/EN PASS, оба вывода приняты независимым strict V2 validator;
три negative cases сохранили clipboard без изменений. Первый harness run завершился
timeout из-за отсутствующей QML tr; после добавления прежней функции из harness
финальный run прошёл. Неуспешные runs сохранены отдельно, PASS им не присваивается.

Финальные проверки: rustc 1.85.1 / clippy 0.1.85, `--offline --locked`;
61 desktop library tests PASS; GUI build PASS; GUI `clippy --all-targets --
-D warnings` PASS, без Rust suppressions. GCC16/Qt печатают прежние C++ warnings.
`cargo fmt --all -- --check` terminal PASS; первый fmt check до финального
форматирования сохранён как отдельный nonterminal log. Python V1/V2 15 PASS,
factory coverage PASS, RU/EN translations PASS (276 пар, 101 ключ), Ruff
check/format PASS. Final clipboard log — clipboard-final.log.
