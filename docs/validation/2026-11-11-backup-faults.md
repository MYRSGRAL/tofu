# Maintenance backup: fault/crash acceptance

База: `82d5fe7397dcfef01f82c48d54660efa974c9987`.
Область: repository/backup.rs и private cfg(test) harness. Формат, AEAD,
проверки схемы и replay из PR70 не изменены. Публичный API не расширен.

После VACUUM проверяется именно staging через read-only SQLite с
trusted_schema=OFF: integrity, schema и AEAD до hard_link. Неблокирующий flock
родительского каталога сериализует backup/recovery. При следующем вызове
удаляются regular files 0600 нашего UID с зарезервированным именем
`.yonik-backup-<32 hex>`. Recovery вызывается до проверки существующего target,
чтобы убрать второй hardlink после аварии между публикацией и unlink.
Это протокол для сотрудничающих новых процессов; параллельный старый бинарник,
не соблюдающий flock, не поддерживается. Враждебный same-UID writer вне модели.

Harness использует только приватные временные SQLite и синтетический ключ.
Дочерний процесс запускается в собственной process group, сообщает checkpoint
файлом и блокируется. Родитель посылает SIGKILL группе, пока Child ещё удержан
без reap, затем wait. Deadline 10 секунд; дочерний watchdog 15 секунд.
Точки: before/after VACUUM, after candidate validation, after publication,
before/after directory sync. Restart обязан оставить target отсутствующим либо
валидным, с nlink=1, доступным через Repository::open/load. Проверяются байты
source, отсутствие plaintext marker и staging.

Synthetic I/O failures: EIO/EACCES/ENOSPC на границах перед VACUUM (write),
file-sync, hard_link и directory-sync. Они проверяют cleanup/error handling,
но не эмулируют частичную запись SQLite VFS или реальное заполнение диска.
rename не используется production-путём: публикация через hard_link без замены.
Отдельный дочерний процесс портит ciphertext после VACUUM: публикация запрещена.
Проверяются dangling symlink target, symlink/hardlink source, сохранность
существующего target и файла вне зарезервированного staging-формата.

Recovery линейный по числу записей каталога, O(1) дополнительной памяти;
validation построчная, прежний предел копии 256 МиБ. Дополнительная проверка
кандидата добавляет один линейный проход. Не моделируются power loss, mid-VACUUM
SIGKILL, аппаратный fsync, hostile same-UID directory races. GUI и keyring
login-session остаются зависимостями; полная P06 не заявляется.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/backup-fault-62195/`.

Дополнительные границы: late publication race создаёт обычное назначение или
symlink прямо перед hard_link; оба остаются нетронутыми, staging удаляется.
Отдельная проверка удерживает flock каталога и требует Conflict вместо recovery.
Отрицательный контроль отключает только новые candidate validation/recovery,
сохраняя test hooks; это regression RED, а не утверждение о pre-implementation TDD.

Проверки Rust 1.85.1, jobs=1, global with-rust-slot и flock task-root:

- Negative control: 5 passed, 2 failed (corrupt candidate publication, orphan staging).
- Финальный `cargo test -p yonik-linux --lib repository::backup::fault_tests -- --test-threads=1`: 7 passed; 6 реальных child SIGKILL checkpoints, 12 synthetic I/O failure combinations, 2 late publication races.
- `cargo test -p yonik-linux --test storage_backup`: 4 passed, включая прежнюю schema/replay/legacy матрицу.
- `cargo clippy -p yonik-linux --all-targets -- -D warnings`: успешно.
- `cargo fmt --all --check`, `git diff --check`: успешно.
- Собственный transient Cargo target очищен штатным `cargo clean`: 4282 files, 2.7 GiB; task-root удалён после проверки отсутствия открытых ссылок. Evidence сохранены вне TTL. Cargo.lock восстановлен побайтово: Cargo менял лишь порядок записей, состав зависимостей неизменен.
