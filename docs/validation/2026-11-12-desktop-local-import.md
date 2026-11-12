# FR07/P04: локальный импорт desktop

Область: Linux Qt/QML → desktop bridge → ограниченное чтение → существующий
importer → отчёт → явное подтверждение → encrypted repository. Ручная форма
протокола остаётся отдельной задачей; полный FR07 этой поставкой не закрывается.

## Контракт

- `import_text {name,text}` принимает до 8 MiB UTF-8. QML ограничивает редактор
  1 Mi символов; для большого входа предлагается выбор файла.
- `import_file {name,path}` принимает только явно выбранный локальный файл.
  Открытие через Linux `openat2(NO_SYMLINKS|NO_MAGICLINKS|NONBLOCK)` запрещает
  symlink во всех компонентах и не блокируется на FIFO. После открытия проверяется
  тип и размер; чтение блоками 16 KiB повторно ограничивает рост файла до 8 MiB.
  На ядрах без openat2 операция завершается отказом; небезопасного fallback нет.
- Чтение и parser выполняются вне Qt-потока. `cancel_local_import` отменяет только
  локальную подготовку и её отчёт. Завершение blocking worker ожидается; Stop и
  Shutdown дренируют его до завершения поколения.
- Ошибки используют существующие V1 keys. Путь, исходник и ошибки OS/parser не
  возвращаются в snapshot. Таблица V2 diagnostic causes не изменяется.
- Каждый локальный импорт требует approve существующего review UI с counts.
  Исходник сохраняется побайтно, включая отвергнутые записи и неизвестные поля.
  Профили с неизвестными параметрами сохраняются с ограничениями активации.
  JSON не передаётся напрямую в engine: используется существующий ServerIntent.

## Хранение и совместимость

Local SourceData имеет `version:2,local:true`, пустой URL, выключенные HWID,
proxy и local_exception, без schedule. Remote v1 не получает поле `local:false`;
проверяется точная строка encoding. Pins, helper ABI и capabilities не изменены. Local v2 использует общую
format4 barrier из dependency W1 с backup до первой записи нового формата;
repository.rs/backup.rs принадлежат W1. Source и snapshot создаются одной SQLite transaction;
отмена до admission откатывает обе записи и каталог профилей.

`SubscriptionSource::is_local()` и `ScheduleMetadata::is_local()` скрывают
внутреннее представление. Desktop load/reload пропускает local. Backend отклоняет
Refresh/HWID/SourceChange до scheduler.cancel/reserve и сетевых операций. Сервис
дополнительно отклоняет эти операции и создание local без snapshot.

Компактный SourceData v1 control проверяет точное remote encoding и отказ
старой схемы при local v2. Он не заменяет проверку целого старого приложения:
actual old-binary отказ открытия/Connect/write на format4 и сохранение
DB/backup входят в совместную приёмку с API W1 для обоих продуктовых путей.

## Проверки

Тесты `local_import`, `local_transaction_tests`, `source_v1_control` и
`local_v2_is_excluded_on_load_and_reload` проверяют границы файлов, original,
шифрование, rollback/cancel, старый reader и расписание. Backend `local_*`
проверяет text/file, partial approve/discard/reopen, Base64/Xray/sing-box,
неизвестные options и отсутствие stale report после отмены.

`local_repository_reopen_intent_and_both_native_config_checks` запускается явно
с `YONIK_LOCAL_CORE_ROOT` и проверяет конфигурации обоими pinned cores после
реального encrypted import/approve/reopen. `tests/local_import_smoke.py` запускает
Qt offscreen с настоящими input/review controls и FileDialog acceptance.

Постоянные логи, binary и hashes находятся в
`/home/myrs/projects/yonik/local/evidence/fr07-desktop-import`.

## Выполненная приёмка

Dependency: ordinary merge coherent API `010242a8ee06a18b4656a42c4451a802d36a85e3`
из PR131. Остальной P07 routing UI не входит в назначенную реализацию.
Repository/backup code принадлежит W1; этот PR использует его entrypoint без
изменений. `backup: Option<PathBuf>` — кандидат, который repository заново
проверяет внутри IMMEDIATE transaction; UI bool не является authority.

В review есть создание копии через существующий maintenance supervisor:
закрытие handles → copy_closed → reopen → успешный ACK → local approve.
Путь хранится только в host-памяти и не попадает в snapshot/errors. Backup нужен
в отдельном приватном каталоге: каталог live DB удерживает instance lock.
Неудачный backup/approve сохраняет отчёт для повторной попытки; существующая
копия не заменяется. Cancel копирования использует штатный maintenance_cancel.

Проверены subscriptions unit (20 passed), desktop local library (4 passed),
backend local commands, явный native test (оба pinned core config checks),
MSRV 1.85.1 clippy GUI all-targets с `-D warnings`, fmt и ruff.
Qt offscreen проверяет настоящий text input, counts, FileDialog selection,
maintenance backup/approve, file approve, read cancel на 20 000 URI, discard,
ошибку и redaction.

Отдельный `local_format4_actual_old_reader...` создаёт encrypted local DB4
через DesktopHost/maintenance/approve и запускает старый Qt binary
`editor-native-fixture` SHA256
`a9d8f65641a7803b4068bca9e267b3f9087b1c046ff10f9f16fc0233fb9b1523`.
Старый reader не открывает DB4, отказывает Connect/delete/write; SHA256 SQLite
не меняется. Та же verified backup3 успешно открывается старым binary с тем же
synthetic key. Воспроизводимый Python control — `tests/local_import_old_reader.py`;
результат и traces — `old-reader-control/` в постоянном evidence.

Ручная protocol form остаётся отдельным leaf. Android, physical G-UI/G-POWER,
TUN/helper и изменение pins/capabilities/ABI не входят в приёмку.
