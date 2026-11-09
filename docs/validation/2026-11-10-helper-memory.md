# Общий бюджет buffers и executable snapshots

База: `d585b01` (PR #30 влит, push/PR workflows SUCCESS, все 5 PR jobs).
Этот компонент продолжает P12; готовность системного сервиса/TUN не заявляется.

## Изменения

- Процессные atomic budgets: config64MiB, executable256MiB; RAII leases,
  checked overflow, рост до записи и освобождение после удаления данных.
- Prepare резервирует retained8MiB до auth/factory, payload ограничен4MiB,
  затем размер уменьшается до действительной Vec.capacity. Serializer отвергает
  oversize до роста output buffer. Claim передаёт opaque owner с lease.
- Системный реестр использует per-engine weak cache и полную source identity.
  Shared sealed inode переиспользуется через dup FD, last owner освобождает pool.
  Обычный непривилегированный BinarySnapshot::load остаётся без helper pool.
- При росте источника executable budget увеличивается до записи следующего chunk.
  Неверные SHA/права/identity не становятся cache hit.

## Review и исправление

Независимый review нашёл P2: резервирование до TTL cleanup могло навсегда
заблокировать новый Prepare при заполненном pool. Исправлены own-store prune
и event-driven Weak callbacks других stores. Callback list lock отпускается до
вызова, State используется через try_lock; живые claimed owners не очищаются.
Callback registry ограничен16stores, цикла владения нет. Узкий повторный review
подтвердил закрытие P2, новых findings нет.

Один старый fixture предполагал, что peer.close уже обработан daemon. При полном
наборе выявилась гонка; тест теперь подписывается до close и ждёт NameOwnerChanged.
Проверка производственного liveness не ослаблялась, sleep/polling не добавлялись.

## Проверки Rust1.85.1

- Весь workspace: **393 passed, 0 failed, 7 ignored**.
- Helper suite после исправлений: **37 passed**.
- Registry:10 tests; binary budget:2 tests; сохранены host regression cases.
- Проверены конкурентные reservations, overflow/resize/drop, bounded Weak owners,
  общий pool нескольких stores, lifetime после claim, failure/cancel, own/other
  TTL reclaim, serializer pre-growth, same-inode reuse и invalidation.
- Workspace/all-targets Clippy с -D warnings, fmt и diff check: PASS.
- Cargo.lock не меняет состав/версии зависимостей; Cargo внёс только перестановку
  package records, сравнение parsed TOML подтвердило эквивалентность.

## Ограничения

Это учёт удерживаемых buffers/memfd и reservations, не ограничение всего RSS.
Transport allocations, временный compiler AST, активные engine mappings после
потери helper и полные G-POWER/recovery требуют отдельных проверок. Managed host
должен сохранять snapshot до reap engine. Отдельного hooked-reader mid-read grow
теста нет: проверены рост между reservation/metadata и resize до write в коде.
Системные службы, polkit пользователя, сеть и keys не изменялись.

Evidence: `local/validation/helper-memory/`.
