# Bounded State Busy в приёмке — 10 ноября 2026

База `fd69f3286dab63371c5a1a7f1b67116ff2630199`, PR #49.
Это контрактная коррекция acceptance, не product fix.
Production source/admission/guards/Busy stages не изменены.

## Основание

Прочитан `local/validation/system-session-vm-gate/stop-fix-fd69f328/report.md`.
На exact fd69 диагностический Apply→Stop прошёл: worker/check inactive,
cgroups empty, records0, public_policy/session_recovery null. Matrix дошла
до actual Xray TCP/UDP IPv4/IPv6 PASS, затем единичный State вернул Busy
на backend_operation_capacity. Stop/restart/sing-box matrix не достигнуты.

[Контракт helper-service](../specs/helper-service.md) разрешает State Busy
при реальном занятом host. Поэтому меняется только одно ожидание State
после traffic в installed_session_tests. Apply, Stop, traffic, весь case,
assertions, поколения и read-only sampler не повторяются и не пропускаются.
Historical Unavailable и SSH hang не объяснены. Строгий actual порядок
cgroup-empty-before-network-removal этим изменением не принят.

## Политика ожидания

Общий monotonic deadline10s начинается до первого State call и не обновляется.
Каждый call обёрнут timeout_at тем же deadline; backoff также ограничен его
остатком. Перед возвращением success снова проверяются cancel и deadline.
Задержки 25→50→100→200→400ms, затем cap400ms; максимум32 вызова.
Нет busy-loop, повторов mutation RPC или cached fallback.

10s выбраны как конечный acceptance budget чуть выше существующих отдельных
bounds: RTNETLINK3s, nft5s и supervisor8s. Это не сумма всех worst-case phases
и не новая гарантия API: если contention/State execution не укладывается,
приёмка завершается ошибкой. Ограничение доступности State этим handling
не считается устранённым или опровергнутым.

Повтор разрешён только для genuine zbus MethodError с точным именем
`cloud.highpriority.yonik.Network1.Error` и actual reply body `(String,)`,
содержащим строгий JSON `{"version":1,"error":"busy"}`. Unknown/missing/duplicate
fields, неверная версия/тип, malformed JSON/body отклоняются. Description
D-Bus ошибки не используется как источник Busy. Frame ограничен64KiB.
Authorization/stale/timeout/unavailable/unknown codes, другое имя ошибки
и транспортные ошибки немедленно fail без следующего State.
JSON Busy внутри success reply также не повторяется.

Success требует нового actual reply `{"version":1,"state":"tunnel_starting"}`
со строгой схемой. Возвращается именно state из этого reply, не cached value.
Любое другое состояние/version/frame — немедленная ошибка. Каждый outcome
логирует busy count, calls, elapsed_ms и категорию результата; ни credentials,
ни raw error bodies не печатаются. Acceptance дополнительно сохраняет summary
fresh State. Helper подключён только через cfg(test) module tree.

## Тесты

RED: genuine private dbus-daemon server отвечает typed Busy, затем State success.
С исходным single-call поведением test завершается Rpc после первого ответа.
GREEN: тот же сценарий требует2 calls, busy1 и fresh tunnel_starting.

Дополнительно private-bus server проверяет:

- постоянный Busy завершается Attempts или единым Deadline;
- все non-Busy/malformed/unknown-name ответы дают ровно1 call;
- неверная success schema/state и Busy-as-success не повторяются;
- зависший ответ ограничен call deadline, cancellation прерывает ожидание;
- реальная остановка частного bus daemon даёт transport failure без retry.

Частный bus создаётся существующей безопасной test fixture и завершается Drop;
установленный helper/user bus/network/подписки/keyring не меняются.
Все новые вызовы RPC в этом helper — только State.

## Evidence и продолжение

Evidence: `local/validation/state-acceptance/` вне TTL/git: red.log, green.log,
workspace-tests.log, clippy.log. Exact lib-test — yonik-linux-tests,
head/tree/hash/размер — binary-provenance.json для VM без rebuild.
Cargo1.85.1, jobs=2; target только
`/var/tmp/codex-myrs/transient/yonik-state-acceptance/target`, под root flock.
После сохранения evidence и binary выполняется cargo clean.

Новая VM/матрица не запускались. Root назначает независимый review перед
actual rerun. Новый head не получает historical traffic/cleanup PASS старого
head автоматически; P09/P12/P13 partial и `system_tun:false` сохраняются.

Фактические результаты Rust1.85.1:

- `cargo +1.85.1 test -p yonik-linux --lib --locked real_private_bus_busy_then_fresh_state`: RED, Rpc после первого Busy.
- `cargo +1.85.1 test -p yonik-linux --lib --locked state_acceptance -- --nocapture --test-threads=1`: GREEN, 6 passed. helper-metrics.log содержит busy/calls/elapsed для каждого outcome.
- `cargo +1.85.1 test --workspace --locked -- --test-threads=1`: 575 passed, 0 failed, 27 ignored.
- `cargo +1.85.1 clippy --workspace --all-targets --locked -- -D warnings`: exit0.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: exit0.

Namespace/VM gates повторно не запускались: меняются только test acceptance
и его private-bus helper. Это не actual matrix PASS нового head.
