# N5c: частичная реализация private producer

База: `36cc6a9737e009652e6ca9af51c7f37ad22757c9`.
Contract: frozen `7faab39f829ac6e10f10c0630fc1b4bbaf59d036`.

Реализованы helper-owned session16 с fail-closed entropy, checked reservation,
retained optional artifact stamp (legacy None), consume/replay tracking,
private move-only verified network record, synchronous commit, revoke перед
invalidation/recovery, permanent native close при overflow, удержание revoked
lease до confirmed worker recovery. Сетевые проверки выделены из legacy
verify_session без изменения worker checks. UID accessor вызывает verify_identity.

Фабрика, compile caller, start_authorized, permit/report16 и wire не подключены.
Private dead_code на producer boundary необходим из-за запрета callers; он
не подавляет проверки тела функций. Generic параметры private retention позволяют
unit-заменители ресурсов без FD constructor; production Backend использует
конкретные Arc<SessionArtifact>, Arc<BoundTun>, AuthorizedRun.

## Проверки и пределы

Локальные полные логи: `local/evidence/n5c-private-owner/` (вне TTL).
Toolchain 1.85.1, jobs=1, общий with-rust-slot. Owner tests проверяют generation burn,
replay, foreign session, zero/overflow и revoked retention/confirmed release/drop.
Mutation удаляет revoke: unknown-stop test падает на ещё живом token.
Session-host regressions и strict yonik-linux all-targets clippy выполняются отдельно.
Это unit evidence, не full N5/runtime enforcement.

## Не закрыто — не считать готовым producer leaf

- Pending→Active transfer отсутствует. AuthorizedRun::ticket проверяет собственный
  binding, но не предоставляет сверку с retained pending owner authority. Нельзя
  считать корректным transfer произвольного AuthorizedRun в слот только по ticket.
  Нужен согласованный способ связывания exact run с owner до реализации transfer;
  systemd_worker вне назначенного ownership.
- Нет positive network-record unit fixture и обязательных exact artifact A→B,
  wrong canonical queue, network-change и finalbinding mutation RED.
- Private retention unit использует заменители ресурсов, а не реальный N5a run.
  Поэтому не доказывает active ownership transfer или kernel queue lifetime.
- Нет отдельной native compilation: reservation и optional stamp готовы, но
  stamping bytes и compile integration не активированы согласно запрету callers.

Full N5 OPEN. Этот checkpoint не удовлетворяет всему acceptance N5c и требует
доработки перед merge.
