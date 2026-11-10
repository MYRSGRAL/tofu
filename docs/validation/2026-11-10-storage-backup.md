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

## Исправление review PR70 от 2026-11-11

Исходный head `f48eaf6bfbb66a1aa711e5d5abcd64b0117ec6b9`;
обычный merge `origin/main` `5157139600ff9a26fe456f19a6b9796ecbeda195`
создал `043631583b866cb6b43badece4743bcca5c61c34` без конфликтов.
Изменения ограничены repository/backup, тестами и этим evidence.

RED на merge-head без production-исправления:
`cargo +1.85.1 test -p yonik-linux --test storage_backup rejects_` —
0 passed, 2 failed: `rejects_incomplete_versioned_schema` (v3 без
source_sequence) и `rejects_authenticated_profile_replay` (аутентичные
поля generation/revision/payload профиля snapshot 1 в БД со snapshot 2).
Оба упали именно на «повреждённый кандидат опубликован», после подтверждения
побайтовой неизменности original и candidate. Лог `red.log`.

Preflight теперь проверяет обязательные поля даже пустых таблиц:
v1 repository/sources/snapshots; v2 добавляет profiles/profile_catalog и
sqlite_sequence; v3 добавляет source_sequence. Проверяются INTEGER primary
keys, unique index source_id/fingerprint, типы и диапазоны служебных значений,
ссылки на источники/snapshot, единственная строка source_sequence и last_id
не ниже max(source.id), счётчики каталога и лимит профилей на источник.
Отсутствующий lazy catalog row допустим. routing_draft остаётся необязательной
для всех legacy-версий; при наличии проверяются её поля/ключ/значения и AEAD
с существующим доменом 8. Проверка не создаёт и не исправляет таблицы.

Связь present-профиля с текущим snapshot проверяет общий private
`validate_profile_snapshot`, используемый также profiles/resolve_profile.
Недоступный избранный профиль может хранить старую generation/revision.

GREEN: 4 backup-теста. В schema-тесте 13 отказных кандидатов: отсутствующие
таблицы v2/v3, поле validators v1, поля last_id/unique_count/ordinal,
таблица sources без PK, пустой/отстающий source_sequence, неверные счётчики
каталога и corrupt routing draft. Replay проверяется отдельно. При каждом
отказе target отсутствует, байты original и candidate не изменились.
Положительная матрица: v1/v2/v3 × routing_draft present/absent (6 случаев),
backup→reopen→profiles→create_source; для v2/v3 также сохранён недоступный
избранный профиль старой версии. Ciphertext/source bytes не меняются при backup.

Логи текущего исправления вне TTL:
`/home/myrs/projects/yonik/local/validation/storage-backup-review/`.
Не проверялись аппаратные сбои fsync/питания, произвольное враждебное DDL,
интеграция UI/keyring и замена существующей БД. Предыдущие границы bounded
maintenance, no-overwrite и исход после публикации/fsync не изменены.

Финальная проверка исправления (Rust 1.85.1, jobs=1, global slot + flock):
`cargo test -p yonik-linux --test storage_backup` — 4 passed;
`cargo test -p yonik-linux --lib repository::tests` — 16 passed;
`cargo clippy -p yonik-linux --all-targets -- -D warnings` — успешно;
`cargo fmt --all --check` и `git diff --check` — успешно. Python не изменялся.
