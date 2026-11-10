# P06: проверки отказов encrypted subscription storage

Дата: 2026-11-10. База: `21aba59f7e161a67d98ebe7415315e3fb8afcf9a`.
Изолированный worktree, только synthetic SQLite fixtures, ключ `[7; 32]`.
Пользовательские БД и Secret Service не открывались.

Добавлены четыре repository unit-теста:

- `failed_reconcile_rolls_back_snapshot_after_reopen`: SQLite trigger вызывает
  ABORT в конце reconcile, после изменения snapshot и профилей. После close/open
  сохранены прежние body, metadata, validators, generation, revision и каталог.
  Замена использует другой endpoint, credentials и метаданные.
- `interrupted_transaction_recovers_after_reopen`: отдельный test-процесс открывает
  БД, удаляет snapshot/каталог внутри незавершённой транзакции и вызывает exit(73)
  без Drop/close. Родитель проверяет код завершения и прежние данные после reopen.
- `snapshot_corruption_fails_closed_after_reopen`: порча body, metadata,
  validators, revision и source_generation по отдельности даёт Corrupt при load.
- `wrong_key_preserves_legacy_domain_and_data_after_reopen`: fixture схемы v1,
  неверный (нулевой) ключ даёт Corrupt, user_version остаётся 1, id/sentinel не
  изменены. Правильный ключ мигрирует v1→v3, lazy catalog восстанавливается;
  source/body/metadata/validators сохраняются также после второго reopen.

Подтверждённых дефектов реализации в этих сценариях не выявлено: production-код
не изменялся. Существующие AAD, sentinel и HMAC-домены сохранены.

Проверки на rustc/cargo 1.97.0, CARGO_BUILD_JOBS=2, target вне checkout:

- `cargo test -p yonik-linux --lib repository::tests`: 15 passed, 0 failed
  (289 filtered out), включая четыре новых теста.
- `cargo fmt --all --check` и `git diff --check`: успешно.
- `cargo clippy -p yonik-linux --all-targets -- -D warnings`: остановлен
  существующим clippy::collapsible_match в yonik-engine/src/lib.rs:163.
- `cargo clippy -p yonik-linux --all-targets --no-deps -- -D warnings`:
  успешно (все targets назначенного crate).

Ограничения: exit(73) проверяет потерю процесса без деструкторов, но не отключение
питания, fsync/I/O faults или все границы commit. Полный backup/recovery и формат
backup не добавлялись. Отсутствующий ключ не представим API repository (требуется
32-байтный ключ); нулевой ключ проверен как неверный, а не как модель Missing.
Missing/Locked/unavailable Secret Service и desktop allow_create wiring здесь
не проверены: нельзя безопасно блокировать реальную пользовательскую коллекцию,
изолированный Secret Service fixture не подготовлен. P06 остаётся частичным.
Независимое review точного head и CI требуются до merge.

После независимого review исправлен P2 в child-fixture: переменная окружения
теперь выбирает только версию протокола. Путь и случайный nonce передаются
через приватный stdin pipe родителя. До открытия repository проверяются
маркер с nonce в родительском fixture-каталоге и наличие уже созданной БД.
Регрессия запускает child с унаследованным путём к постороннему пустому файлу:
child должен отказать до SQLite, содержимое файла остаётся неизменным.
После исправления: 16 repository-тестов, fmt и clippy всех targets с
`--no-deps -- -D warnings` прошли. Логи сохранены вне временной сборки:
`local/validation/review46-root/{tests,clippy}.log` в основном workspace.
