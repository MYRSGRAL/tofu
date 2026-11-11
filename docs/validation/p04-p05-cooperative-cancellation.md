# Кооперативная отмена import → snapshot COMMIT

База реализации: `d0a281b4c23b45f65db1ddb74c6b14378b399794`.
Отдельная реализация cancellation из proposal PR92 `bfd6149`;
performance source/tests, 48/20 snapshot campaign и external rules не переносились.

## Контракт

Старые `import_bytes`, `Subscription::stage`, `commit_snapshot`, `offline`
сохраняют API и работают без cancellation. Новые cancellable entrypoints
разделяют Cancelled и ошибку входа/хранилища, не публикуют частичный отчёт.
Pure parser принимает callback без зависимости от Linux/Tokio.
Linux передаёт существующий `Cancellation` на основе watch.

Отмена — явная передача `true` в watch, а не abort future или kill процесса.
Закрытие источника token также означает отмену. Request token действует
на staging и commit; внешний fetch token действует на fetch/staging/304.
После выдачи Pending управление отменой review принадлежит `service.cancel(id)`.

Перед COMMIT проверяются watch-источники под read guards: успешная проверка —
точка допуска COMMIT. send_replace до допуска приводит к rollback, после допуска
worker возвращает фактический результат COMMIT, включая успешную revision.
Проверок cancellation после допуска нет. Cancel request не обещает Cancelled,
если COMMIT уже допущен. SQLite failure остаётся ошибкой хранилища.

При ожидании repository/IMPORT gates token проверяется через select.
BEGIN IMMEDIATE использует retry с checkpoint каждые 10 мс, общим busy budget 3 с;
на этом пути стандартный 3-секундный busy handler временно отключён.
После worker восстанавливается прежний timeout. Parse URI/JSON records,
Base64 chunks по 4096 байт, подготовка identities, чтение/reconcile profiles
содержат checkpoints. Ограничения размеров и legacy encryption/AAD не меняются.

Service requests mutex не удерживается через repository await. Сериализация
изменений source metadata сохраняется отдельным mutex, который не блокирует
cancel. ACK-permit принадлежит worker, освобождается после repository/CPU gates;
`service.cancel` ждёт permit без requests mutex. Await cancellable repository
также дожидается worker/rollback. Сам по себе abort ожидающего future по-прежнему
не запрашивает cancellation.

## Регрессии и evidence

Внешние логи: `/home/myrs/projects/yonik/local/validation/actual-cancel/`.

- `red-write-lock.log`: production cancellable repository adapter без checkpoints
  не подтверждает отмену за 1 с, после release возвращает `Ok(2)`.
- `red-parse.log`: при отключённом URI record checkpoint 10 000 записей
  публикуют отчёт вместо Cancelled.
- `red-boundary.log`: при отключённом admission check отмена до COMMIT
  возвращает `Ok(2)` вместо Cancelled.
- Новые repository tests проверяют ACK до release SQLite write-lock, неизменные
  revision/body/metadata и освобождение gate. Два детерминированных barrier
  проверяют отмену непосредственно до/после admission на actual COMMIT path.
- Новые service tests вызывают actual `commit` + `cancel`, отдельно external
  cancellation на 304 path, без сети. После rollback проверяется старый snapshot
  и возможность следующей записи.
- Pure parser/stage: 10 000 URI, JSON validation, Base64 decode checkpoints,
  отсутствие публикации при отмене и сохранение старой revision.

Это проверка cancellation, не полный PASS P04/P05. Не повторялись performance
campaign, installed helper/native engine runtime, DNS lookup, GUI/keyring/Android проверки. Deadline 1 с проверяется
для actual SQLite write-lock cancellation; это не универсальный real-time SLA
для любых нагрузок и системного I/O. Криптографический вызов ограничен прежним
BODY_LIMIT и сохраняет прежний формат; он не прерывается внутри AEAD.

Rust 1.85.1, scoped verification: parser cancellation 4/4, staging 1/1,
Linux `--lib cancellation_` 22/22, существующие HWID regressions 9/9.
`clippy -p yonik-import -p yonik-subscription -p yonik-linux --all-targets -- -D warnings`
и `fmt --all` прошли. Использовались offline/locked, jobs=1, global Rust slot
и shared flock собственного transient root. Первый широкий прогон выявил
лишнее удержание внешнего fetch token на review; это исправлено, HWID 9/9
перепроверены на исправленном коде (`compat-hwid.log`).

Финальная совместная проверка с scoped DNS-label fix: Rust 1.85.1,
`cargo test --workspace --locked --offline` — 776 passed, 0 failed, 39 ignored
(сумма результатов 70 harness/doc-test suites; ignored не считаются PASS).
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
и `cargo fmt --all -- --check` прошли. Логи `workspace-tests.log`,
`workspace-clippy.log`, `fmt.log` сохранены во внешнем evidence-каталоге.
