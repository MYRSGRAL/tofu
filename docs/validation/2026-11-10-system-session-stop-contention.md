# Ограниченная сериализация Stop — 10 ноября 2026

База `bb39a00b1a1ace5b777814d50c8dc08408a38c39`, PR #49.
Прочитан actual report
`local/validation/system-session-vm-gate/tun-contract-bb39a00/report.md`.
В serial retry Apply прошёл весь путь до tunnel_starting, Stop вернул Busy,
worker и 8 records остались. Отдельный stop units не очистил journal и не является
успешным SystemSessionHost Stop. Matrix не выполнялась; initial SSH hang и
historical Unavailable не объяснены. Новая VM и stubborn gate здесь не запускались.

## Доказанный локальный источник

На исходном job() первый backend.try_lock возвращает Busy при удержанном
backend guard. RED test вызвал непосредственно job(Stop) с тем же guard:
Stop завершился до освобождения mutex. Outer service mutex, D-Bus и in-flight
semaphore в этом воспроизведении не участвуют. Не было poisoned mutex или
эффектов, поэтому это конкретная WouldBlock→Busy ветка старого preflight.
Исходный job exact bb39a00 сохранён в preflight-source-bb39a00.log, RED — red.log.

GREEN regression дополнительно использует настоящий spawn_safety с callback,
удерживающим backend и общий gate по каналам. Stop остаётся pending, после
release sweep завершается с NoManagedPolicy. Это actor fixture без network
resources: тест доказывает сериализацию, а не actual очистку восьми VM records.
У старого actual run нет отдельных Busy-stage logs: окончательное утверждение
о его конкретной ветке требует следующего actual case с новой диагностикой.

## Исправление

Все effect jobs и safety sweeps делят один FIFO semaphore permit, приобретаемый
до spawn_blocking. Обычный занятой job продолжает отвечать Busy. Уже
авторизованный Stop получает один отдельный admission slot и ожидает gate
до 30 секунд; новые sweeps не обгоняют уже queued Stop. Нет sleep/retry в
acceptance и нет очереди blocked spawn_blocking: на backend допускается один
effect/sweep, плюс один асинхронно ожидающий Stop.

Допущенный Stop выполняется в отдельной owned task. Drop RPC/caller future
не освобождает его slot/permit и не отменяет cleanup. Permit и admission slot
держатся до возврата Backend.perform, включая await worker recovery/empty cgroup
и durable network cleanup; reply не выдаётся раньше. После начала перехода
действует прежний independent cleanup Cancellation. До эффекта explicit cancel
и истёкший wait bound возвращают Cancelled/Timeout, без ложного cleanup ack.
Timeout не доказывает очистку: durable recovery-required состояние сохраняется.

После polkit и проверки peer UID outer service Stop ждёт host mutex с отдельным
30s bound и cancellation. In-flight лимит4, body limits, свежая authorization,
owner UID и worker-first ordering не сняты. Второй одновременно допущенный Stop
получает Busy от single-slot capacity, пока первый продолжает cleanup.
Unlock сохраняет отдельную authorization и обычный backend admission.

## Диагностика

Только cfg(test) и YONIK_SESSION_DIAGNOSTICS=1. Отдельные Busy stages:
rpc_in_flight_capacity, prepare_outer_host, apply_outer_host, state_outer_host,
backend_context, backend_job_preflight, backend_operation_capacity,
stop_admission_capacity, backend_owner_uid, backend_prepare_active,
backend_apply_active. Payload/credentials/peer data не печатаются.

## Проверки и границы

Новые tests: настоящий held safety sweep → Stop pending → completion;
сброшенный caller сохраняет admitted slot до завершения Stop, второй Stop
ограничен capacity; timeout и cancel до эффекта не меняют epoch/state.
Ранее существующие тесты проверяют удержание backend после abort caller,
worker-first recovery ordering, durable disposition, stale/peer/authorization
и fail-closed unknown worker. Реальные systemd/network effects не подменяются
unit fixture assertions. Ни один результат этих tests не объявляет VM journal
из восьми records очищенным на новом head.

Evidence: `local/validation/stop-contention-fix/` вне TTL/git. Target только
`/var/tmp/codex-myrs/transient/yonik-stop-contention-fix/target`, jobs=2,
BSD flock самого root с первого использования. Exact lib-test сохраняется
в yonik-linux-tests с binary-provenance.json, затем cargo clean.
Root назначает следующий exact-head actual case и review; P09/P12/P13 partial,
`system_tun:false`, historical Unavailable открыт.

Фактические результаты Rust 1.85.1:

- `cargo +1.85.1 test -p yonik-linux --lib --locked authorized_stop_waits_for_exact_backend_guard_used_by_safety`: RED на исходном preflight.
- `cargo +1.85.1 test -p yonik-linux --lib --locked helper::protection_host::session_host::tests -- --test-threads=1`: GREEN, 20 passed, 1 ignored.
- `cargo +1.85.1 test --workspace --locked -- --test-threads=1`: 568 passed, 0 failed, 27 ignored.
- `cargo +1.85.1 clippy --workspace --all-targets --locked -- -D warnings`: exit 0.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: exit 0.
- Namespace cases session-network, tun-projection, tun-config, service:
  каждый exit 0. Session-attach Xray и sing-box также проходят, по одному test.

Полные логи: red.log, green.log, workspace-tests.log, clippy.log, gate-*.log,
xray-attach.log, sing-box-attach.log. Готовый lib-test соответствует новому
commit; source base и SHA-256 фиксируются в binary-provenance.json.

## Исправление RPC timeout после review

Review head 96baf2c30e880a0e93409426a0a82a8dbb393e40 обнаружил, что оба
истечения Stop wait сериализовались как unavailable. Outer host mutex теперь
возвращает PreparedError::Timeout, как внутренний operations permit;
ServiceError::code сохраняет отдельный публичный код timeout. Authorization,
30-секундные bounds, приоритет cancellation и durable cleanup не меняются.

Две регрессии на удерживаемых mutex/permit подтвердили RED unavailable вместо
timeout. GREEN: cargo +1.85.1 test -p yonik-linux --lib —
324 passed, 0 failed, 23 ignored. Outer helper дополнительно проверяет cancelled.
Сырые логи: local/validation/pr49-stop-timeout/ вне git; это unit/code-contract
проверки, не новый actual D-Bus/VM PASS. Exact lib-test сохраняется рядом
для следующего actual Apply→Stop; historical Unavailable остаётся открытым.
