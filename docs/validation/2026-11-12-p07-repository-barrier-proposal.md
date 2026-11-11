# P07: enforced compatibility barrier — согласованный контракт

ROOT согласовал ownership и реализацию. Repository barrier реализован в
`c4cee3e0602c53820c97e36a7f420e19dd9fb3c5`, coherent merge main —
`010242a8ee06a18b4656a42c4451a802d36a85e3`. Production V2 write проходит
через verified closed backup v3 и атомарный format4 CAS; test-only bypass отсутствует.
Одна декларация minimum-version не защищает старый установленный бинарник.

Старый V1 `Db::open` читает `PRAGMA user_version` до migration/decrypt и
возвращает UnsupportedVersion для >3. Поэтому реализован формат 4, без новых
таблиц и без игнорируемых старым reader метаданных:

1. Новый reader принимает 1..=4. Обычные V1 операции оставляют формат 3.
2. Отдельный typed repository entrypoint для V2 принимает expected revision и
   bounded payload. В той же IMMEDIATE транзакции, где CAS меняет encrypted
   routing_draft, после успешного CAS выполняется `PRAGMA user_version=4`.
   CAS failure/ошибка/падение до commit откатывают и формат, и payload/revision.
   Payload V2 нельзя писать через старый generic save без этого entrypoint.
3. После commit формат 4 не понижается при выключении/удалении selector:
   это барьер открытия всей БД, а не версия отдельного Draft.
4. Closed backup validator принимает 4 с тем же schema contract и
   прежней проверкой bounded routing_draft, когда таблица существует. Restore сохраняет 4, не downgrade.
5. До migration предоставить проверенный backup_closed формата 3 в новый путь.
   Rollback к старому приложению — только restore этой копии в новый путь после
   закрытия handles. Изменения после backup при таком rollback теряются явно.
   `PRAGMA user_version=3` на V2 DB запрещён: это удаляет барьер без миграции данных.

Ownership repository.rs/repository/backup.rs расширен ROOT в этой сессии;
production patch применён с проверкой полного backup snapshot. Обязательная приёмка перед снятием write gate:
actual старый binary + private encrypted DB version4, production open→Connect
и попытка записи, bytes/revision неизменны; negative control той же version3 DB
успешно открывается. Положительный тест нового reader: CAS/reopen/backup/restore,
формат4 и disabled selector сохраняются. Никаких пользовательских DB/ключей.

Отдельный обнаруженный baseline blocker clippy GUI all-targets: main.rs configure
возвращает Result<(), HostConfig>, clippy::result_large_err. ROOT отдельно разрешил private configure → Result<(), Box<HostConfig>>;
успешный CONFIG.set не аллоцирует. HostConfig не перестраивался.

## Выполненный actual negative control

На старом `editor-native-fixture` SHA256
`a9d8f65641a7803b4068bca9e267b3f9087b1c046ff10f9f16fc0233fb9b1523`
проверены два production DesktopHost запуска на synthetic encrypted DB:
version3 → ready=true; затем version4 → busy=false/ready=false, Connect и
routing_default отклонены startup-путём. После shutdown SHA256 всего SQLite,
revision и ciphertext неизменны. JSON и Qt/backend traces:
`/home/myrs/projects/yonik/local/evidence/p07-linux-v2/barrier-fixture/`.
Воспроизводимый сценарий: `crates/yonik-desktop/tests/p07_barrier_smoke.py OLD_BINARY OUTPUT`.
Это доказывает механизм отказа старого reader. Атомарность новой migration
проверена отдельной CAS/fault/cancellation матрицей ниже.

Исторический ограниченный diff для review: `p07-repository-barrier-review.patch`.
Он не применяется поверх итогового кода. Финальная реализация добавляет verified
backup admission, CAS/fault/cancellation tests, backend V2 entrypoint и запрет
обхода generic setters. ROOT согласовал существующий явный maintenance-flow;
проверки выполнены до снятия production write gate.

## Продуктовый путь и API

Оператор выполняет существующий maintenance backup в новый закрытый путь.
Supervisor закрывает handles, делает проверенную копию и reopen. Backend хранит
путь успешной копии; перед первым V2 commit repository повторно проверяет её
AEAD/schema, format3, repository ID и полное равенство всех таблиц snapshot.
Изменившаяся после backup БД требует новой копии. Сам live file, sidecars и
stale backup не дают допуска. После format4 путь больше не нужен, downgrade
при удалении последнего selector не выполняется.

W3 API: `create_local_snapshot(payload, original, cancel, backup: Option<PathBuf>)`.
Source/snapshot/profiles/sequence и format4 фиксируются одной IMMEDIATE транзакцией;
отмена до commit admission и fault после PRAGMA откатывают всё. Generic setters
не создают V2 в DB3. `create_source_v2` и `save_routing_draft_v2` используют тот же
проверяющий barrier; None допустим только для уже format4.

Фактическая матрица: repository tests 47 passed; storage_backup 4 passed;
format4 matrix 3 passed, включая cancellation после записи profiles/PRAGMA,
CAS conflict, fault rollback, stale/self/missing backup, reopen и backup4/restore4.
На coherent HEAD — Linux clippy all-targets -D warnings и fmt passed.
Прототип `p07-repository-barrier-review.patch` сохранён как история review;
он не является инструкцией применения и не заменяет финальную реализацию.
