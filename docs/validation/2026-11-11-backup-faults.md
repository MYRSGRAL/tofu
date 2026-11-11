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

## Исправление P1: доказательство владения staging

Предыдущий протокол удаления `.yonik-backup-<32hex>` по имени/mode/UID
**небезопасен и заменён**. Он мог удалить обычную исходную БД, существующее
назначение или ранее опубликованный backup с допустимым именем. База P1:
`80610e7c4ec1b9968230eb85bb7b38e9c56f1fa2`. Три regression-теста на этом
production-коде дали RED: 0 passed, 3 failed. Никакие новые production-защиты
во время RED не были отключены. В случае текущего destination прежний код
удалял и создавал файл заново, поэтому проверяется также обязательный Conflict.

Теперь каждая операция создаёт отдельный случайный каталог 0700
`.yonik-maintenance-<nonce>` с приватным `candidate`. `owner.v1` содержит
16 байт dev/ino кандидата и 32 байта HMAC-SHA256; отдельный домен подписи
связывает эти данные с dev/ino родителя, dev/ino каталога и его именем.
Проверяется неизменность inode кандидата, marker открывается O_NOFOLLOW и
NONBLOCK, ограничен 48 байтами и обязан иметь nlink=1; directory/file type,
UID и права проверяются дополнительно. Marker синхронизируется до VACUUM,
затем синхронизируются namespace и родитель. Ключ не экспортируется;
существующие AEAD-домены и публичный API не изменены, зависимостей нет.

Recovery работает под прежним parent flock **после проверки source** и
удаляет только содержимое аутентифицированного каталога: исходный inode
candidate и SQLite sidecars; неизвестные entries, symlink, несовпадающий
inode и недостоверный marker дают отказ до удаления. Публикация по-прежнему
через hard_link без замены. API отказывает при попытке чтения/публикации
внутри доказанно принадлежащего maintenance namespace. Публичные имена
`.yonik-backup-<32hex>` разрешены; legacy-файлы в родителе не удаляются.
Похожее имя каталога без marker также не доказывает владение.

Порядок cleanup: удалить candidate/sidecars, checkpoint, fsync namespace,
удалить marker, checkpoint, rmdir namespace. При crash после удаления
candidate оставшаяся подпись ещё позволяет убрать каталог. При crash до
полной ownership-записи либо между unlink marker и rmdir доказательства
недостаточно: каталог сохраняется на месте для ручного разбора, автоматическое
удаление не обещается. При частичном/недостоверном marker операция отказывает
с UnsafePath; при отсутствующем marker каталог игнорируется. Это сознательная
граница безопасности, а не подтверждение полной автоматической очистки.
Перед VACUUM такие кандидаты пустые; в последнем окне namespace уже пуст.

GREEN harness: 15 passed. Три P1-сценария; 7 вариантов marker/candidate
forgery/link подмен; wrong-key/missing-source до recovery; legacy public
file и unproven namespace; запрет public API внутри owned namespace.
Реальный SIGKILL: прежняя матрица расширена ownership-synced и
cleanup-stage-unlinked (8 точек), дополнительно namespace-created,
candidate-created, marker-prefix и cleanup-marker-unlinked (4 точки).
Во всех них source неизменен; опубликованный target валиден, nlink=1 после
recovery. Недоказанные артефакты в специальных quarantine-тестах сохранены.

Прежние пределы остаются: нет защиты от враждебного same-UID writer с доступом
к ключу, аппаратного power loss и частичной записи SQLite VFS. Scan линейный,
marker читается с постоянным пределом, candidate по-прежнему ограничен 256 MiB.
GUI, optional cancellation и opaque closed-maintenance key lease здесь не
добавлены; публичного getter ключа нет. Полная P06 не заявляется.

Evidence P1 вне TTL:
`/home/myrs/projects/yonik/local/validation/backup-p1-62195/`:
`red.log`, `green.log`, `red-source-hashes.txt`, `green-source-hashes.txt`.

Финальная проверка P1 (Rust 1.85.1, jobs=1, own task-root flock + global
with-rust-slot): unit harness 15 passed; storage_backup 4 passed;
clippy --all-targets -- -D warnings, fmt --all --check и diff --check успешно.
Cargo.lock побайтово восстановлен: Cargo менял порядок записей, состав/версии
зависимостей проверены и не изменились.
