# Частичная приёмка P06: maintenance backup/restore

База `f8975948406c920c4d78439348001394647d17dd`, Linux, Rust 1.85.1,
jobs=1. Только синтетические SQLite fixtures и ephemeral ключ `[7; 32]`.
Никаких обращений к пользовательскому Secret Service или БД.

`backup_closed` и `restore_closed` требуют закрытия handles и эксклюзивной
SQLite-блокировки (busy → Conflict). Копирование через SQLite VACUUM INTO:
feature rusqlite backup не включён, зависимости не изменены. Формат SQLite,
user_version, sentinel/AAD/HMAC-домены остаются прежними. Ключ не экспортируется;
копия содержит ciphertext и обычные структурные метаданные SQLite, а не полное
шифрование файла. Переключение приложения на восстановленную БД отдельно.

Назначение обязано отсутствовать, как и его sidecars. Даже закрытая существующая
БД не заменяется: CAS/подтверждение замены живых данных здесь не реализованы.
Кандидат проверяется до публикации: integrity_check, версия 1..3, единственный
sentinel, AEAD sources/snapshots/profiles. Миграции выполняет обычный open позже.
Лимит 256 МиБ на main+sidecars до открытия, логические страницы после блокировки
и результат. Поля читаются построчно с существующими лимитами. Это не защита от
враждебного процесса с тем же UID, изменяющего каталог параллельно.

Публикация: приватный случайный staging 0600 в каталоге 0700 → VACUUM INTO →
fsync файла → атомарный hard_link без замены → unlink staging → fsync каталога.
До hard_link отказ не публикует назначение. После hard_link сбой удаления staging
или fsync может вернуть Storage при уже существующем назначении; rollback не
обещается, повтор требует проверки файла. При потере процесса до публикации
может остаться staging; между link/unlink могут остаться два имени одного inode,
которые repository отвергнет по nlink до ручного удаления staging. При потере
питания до fsync каталога сохранность имени не гарантируется. Реальное отключение
питания и инъекции fsync/ENOSPC не проверены.

Тест storage_backup: busy, backup/reopen/restore, snapshot, неверный ключ,
существующее назначение, corrupt ciphertext, усечение, сохранность ранее
восстановленного файла, копирование v1 без миграции и последующий open v1→v3.
Первый запуск выявил ошибку аргументов теста; behavioral RED до реализации
не получен (реализация была записана во время первой сборки). Не выдаётся за TDD.

Логи вне TTL: `/home/myrs/projects/yonik/local/validation/storage-backup/`.
Полная P06 не заявляется: L02/keyring, интеграция UI, fault injection durability
и замена существующей БД остаются отдельными задачами.

Проверено: `cargo +1.85.1 test -p yonik-linux --test storage_backup` — 1 passed;
`cargo +1.85.1 test -p yonik-linux --lib repository::tests` — 16 passed,
361 filtered out; `cargo +1.85.1 fmt --all --check`, staged diff check — чисто.
Python-код не изменялся, Ruff неприменим.
`cargo +1.85.1 clippy -p yonik-linux --all-targets -- -D warnings` — успешно.
Дополнительная регрессия проверяет отказ при существующем sidecar назначения.
