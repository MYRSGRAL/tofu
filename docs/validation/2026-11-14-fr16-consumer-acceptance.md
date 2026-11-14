# FR16: независимые проверки actual disk consumer

База: `0a5d8204567f75d5c2504afadbf6cb509df34bc2` (origin/main после #184).
Прежняя ветка N5c `feat/n5c-private-owner-producer` и её commits
`2297754` / `512fc18` сохранены. Frozen P01, root P02, W2 и W4 не изменялись.

## Граница доказательства

Четыре новых unit acceptance теста вызывают `EventLog::with_disk`, то есть
настоящие start/consumer thread/queue/Sink/files. Нет подмены worker, fd или I/O.
Два закрытых одноразовых checkpoint доступны исключительно при `cfg(test)`:
после открытия sink перед drain и после всей цепочки rename перед create active.
Они привязаны к абсолютному пути уникального owned fixture; registry lock
освобождается до ожидания. Ожидание ограничено 30 секундами. В product build нет
registry, test_path и вызовов checkpoint. Production APIs, lifecycle, wire,
формат и лимиты не изменены.

- Реальная запись более шести сегментов через consumer: batch ≤128 с ожиданием
  прогресса между batch, без потерь; в итоге пять файлов, каждый ≤4 MiB,
  четыре архива >4 MiB−192 байта, возрастающие ID через всю оставшуюся историю.
- Детерминированный stall после actual open: 1003 событий дают ровно 128
  enqueued / 0 written / 875 dropped_full. Shutdown и producer укладываются
  в одну секунду; новая запись после shutdown даёт dropped_closed. До release
  wait_closed(50ms)=false и статус Active. После release записаны все 128.
- Реальная ротация предварительно заполненной валидной цепочки 5×4 MiB,
  restart с partial tail: recovered_partial=1, валидные записи, хвост удалён.
- После open собственный архив заменяется каталогом. Первая попытка ротации
  завершается IoFailed / io_errors=1 / written=0. Следующая запись отклоняется;
  второй ошибки и retry нет, active остаётся прежнего размера.
- Owned child SIGKILL после marker от достигнутого checkpoint: вся цепочка
  rename уже завершена, новый active ещё не создан. После reap signal=9
  подтверждаются отсутствие active и четыре архива по 4 MiB; restart consumer
  создаёт active и записывает событие без I/O ошибок. Это **не** kill внутри
  rename syscall, не power-loss и не гарантия сохранения всей истории.
- Реальный доступный путь sample export: EventLog.read с source=Engine,
  code=ProcessExited → LogPage.export. События App исключены, типизированное
  Engine событие присутствует. Secret/control canary поступает исключительно
  через reject_native, единственный использованный API для произвольных bytes.
  Экспорт не содержит canary/ESC/NUL/CR; newline остаётся разделителем.
  Закрытые числовые поля disk/export сами по себе не доказывают redaction
  произвольного config input: API такого input не принимает.

## Оставшаяся граница host

Новая проверка доказывает pending drain DiskLogGuard, но не host closed snapshot
или control.closed ACK при детерминированно stalled disk. Старые #160/#172 host
checks и #177 GUI pressure не повторялись и не переименованы в новое доказательство.
Также нет нового host Stop/revoke под actual disk stall.

Точный proposal для отдельного согласования: в существующем test-support модуле
Linux-библиотеки добавить fixture-scoped helper создания EventLog с одноразовым
барьером after-open, доступный desktop test-support; в существующем host test
module внедрить этот EventLog через test-only constructor Actor. Проверять Stop,
revoke, отсутствие closed/ACK до release и ACK после drain. Это потребует общего
constructor/hook вне cfg(test) Linux и потому здесь не сделано. Публичный hook и
изменение production semantics не предлагаются.

Scope: только disk/runtime часть FR16 и performance disk. Native, physical,
kernel, runtime16 и весь Linux acceptance этим результатом не закрываются.

## Проверки и evidence

Rust 1.85.1, `RUSTC_WRAPPER=` и `RUSTC_WORKSPACE_WRAPPER=`, jobs=1,
общий `/home/myrs/projects/yonik/local/orchestration/with-rust-slot`.
Целевая команда: `cargo test -p yonik-linux --lib
 event_log::disk::acceptance --locked -- --test-threads=1`.
Финальный прогон: **4 passed**, 0 failed; 9.28 секунды на этой машине.

Пять мутаций production logic дали assertion-based RED, исходники восстановлены:
ёмкость 129; ранний true из wait_closed; сохранённый partial tail вместо truncate;
игнорирование I/O error в conclude; отключённый source filter. Финальный GREEN
выполнен после восстановления. Эти мутации не входят в commit.

Бинарник, source archive, SHA-256 binding, runner и полные логи сохранены вне TTL:
`local/evidence/fr16-consumer/` данного worktree. Каталог ignored, в Git входят
только tests/checkpoints и этот отчёт. Сохранённый бинарник позволяет повторить
четыре exact-сценария без повторной сборки; он предназначен для этого Linux host.

Strict `cargo clippy -p yonik-linux --all-targets --locked -- -D warnings` и
`cargo fmt --all -- --check` прошли; после перехода на явные imports четыре
acceptance теста, clippy и fmt повторно GREEN. До очистки проверено отсутствие
открытых fd на собственный build root. Бинарник/source binding сохранены;
`cargo clean` выполнен, собственный transient root удалён.
