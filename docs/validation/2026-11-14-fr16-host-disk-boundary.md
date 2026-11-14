# FR16: host boundary при stalled actual disk consumer

Продолжение PR #189 после frozen `7ce512591ac154f9213b1c2ba27c0dc591403c91`.
Его source/binary/log binding в `local/evidence/fr16-consumer` сохранён отдельно.
Этот отчёт уточняет оставшуюся host-границу предыдущего отчёта; не расширяет
scope #177 до несуществующего общего native/runtime запуска.

## Реализация test-support

`DiskFixture::new()` создаёт только собственный уникальный private root под HOME,
не принимает пути, fd или I/O callbacks. Consumer использует настоящий start,
очередь 128, directory validation/lock, Sink и regular file. After-open barrier
передаётся конкретному State, без глобального registry и без эффекта на другие
consumer-ы. Он однократный, имеет 30-секундный предохранительный timeout;
`release()` идемпотентен. Fixture Drop всегда release + request_shutdown,
ждёт consumer не более 5 секунд и очищает только свой root после Done.
При непрекращающемся kernel I/O root сохраняется; timeout syscall не обещается.

Cross-crate helper и extra State field доступны исключительно через
`cfg(any(test, feature="test-support"))`. Существующий nondefault feature edge
`yonik-desktop/test-support → yonik-linux/test-support` переиспользован.
Новых dependencies/versions и изменений release/GUI defaults нет.

Desktop wrapper проверяет совпадение DataDir с fixture и принимает её EventLog
и DiskLogGuard однократно. Дальше выполняется настоящий start_inner → run →
Actor/control loop → normal drain → snapshot emit → control.closed.
HostConfig, CLI/JSON, обычный start, production limits/format/lifecycle не меняются.

Дополнительно одобренная loopback lease fixture полностью приватна и `cfg(test)`:
публикует identity `(17, own_endpoint)` в **тот же** production proxy_session
watch-channel, а observer и DesktopFetch подписываются на него. Без native Running
test-only fallback сохраняет уже опубликованный watch lease; не восстанавливает
его после отзыва. Настоящая priority-ветка Stop по-прежнему выполняет send_replace(None).
Не создаются второй watch-channel, native/system authority, core или mock worker.

## Проверяемые сценарии

1. Consumer достиг after-open на реальном файле и блокируется. Actor получает
   именно этот EventLog; test_fill заполняет настоящую очередь: enqueued=128,
   written=0. Stop и следующая routing_default mutation получают настоящий ACK
   без release диска (deadline 2 секунды). Consumer по-прежнему blocked.
2. При final shutdown тест ждёт именно stop flag DiskLogGuard, установленный
   backend после остальных cleanup, затем проверяет: consumer Done=false,
   host.wait_closed(100ms)=false и ни один actual snapshot не имеет closed=true.
   После release появляются настоящий closed snapshot и control.closed ACK;
   consumer Closed, written=enqueued, число строк в реальном файле равно written.
3. Для lease case перед Stop подтверждены Some и identity в watch и pinned
   DesktopFetch. Операция подключается к собственному loopback endpoint;
   fixture читает SOCKS greeting `[5,1,0]`, но не отвечает. После actual Stop
   тот же channel сообщает change/None, ожидающий DesktopFetch возвращает
   Cancelled за ≤2 секунды, пока disk written=0. Затем отдельно проверяется
   pending final ACK и drain по сценарию 2.
4. DataDir вне собственного fixture отвергается до take_sink. Drop fixture
   освобождает after-open без ручного release: actual guard Done/Closed и
   собственный root удалён. Это fault/cleanup control, не mock consumer.

Scope lease case: host watch-lease revocation boundary с fixture lease при
stalled actual disk. Это не доказательство полного active-core/native session
lifecycle. #177 real-core Stop сохраняет свой отдельный scope. Нет новых
native/physical/kernel/runtime16 claims и нет утверждения полного Linux acceptance.

## Evidence

Новые логи/runner/binary/source binding: `local/evidence/fr16-host/` этого worktree,
отдельно от frozen 7ce evidence. Сборки: Rust 1.85.1, wrappers пустые, jobs=1,
общий with-rust-slot (два общих слота), собственный directory flock и target
в `/var/tmp/codex-myrs/transient/fr16-host-boundary`.

Сам `DiskLogGuard::Drop` также освобождает только свой test-support after-open
barrier перед обычным request_shutdown; при обычной сборке этот блок отсутствует.
Четвёртый host test держит DiskFixture живым, сбрасывает sink guard и подтверждает
Done/Closed; затем fixture удаляет собственный root. Так проверены оба порядка
Drop, в дополнение к явному release после pending drain.

## Fault controls и relevant regressions

Новые assertion-based mutation RED (в commit не входят):

- `control.closed()` до disk drain: оба actual host сценария отвергают ранний ACK;
- `snapshot.closed=true; emit()` до disk drain: оба отвергают ранний snapshot;
- удалённый `proxy_session.send_replace(None)` в priority-ветке: lease test
  отвергает отсутствие change/None и pending DesktopFetch revoke;
- удалённый release из DiskLogGuard Drop: отдельный Drop test отвергает
  незавершённый consumer. Fixture Drop затем освобождает barrier для cleanup.

Восстановление исходников выполнялось в finally. Старые пять мутаций 7ce
не повторялись. Из-за изменения State/start выполнен relevant consumer regression на четырёх
frozen сценариях; после дополнительного изменения Drop выполнена финальная
перепроверка тех же сценариев. Старый harness/source не изменён.
Два existing integration checks `disk_log_host`/`logs_save_ack` и три
`proxy_session_tests` — GREEN. #177 GUI/core не запускался повторно.

Финальное восстановленное состояние: **4 новых host tests GREEN**;
**4 relevant consumer tests GREEN**, 0 failures. Strict all-targets clippy для
Linux+desktop с test-support и workspace fmt — GREEN. Product check без default
features — GREEN; compiler-artifact JSON подтверждает features=[] у обоих
`yonik_linux` и `yonik_desktop`, то есть без test-support.

Два новых test binaries, source archive и SHA-256 binding сохранены вне TTL
в `local/evidence/fr16-host`. Перед cleanup открытых fd на build root не найдено;
все build/verify sessions завершены. `cargo clean` удалил 3.1 GiB, собственный
transient root удалён. Frozen 7ce evidence не перезаписывался.
