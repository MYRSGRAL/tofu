# Actual maintenance generation: незакрытый blocker

База PR136: 45dc06d23fb4e85392b9fe3908ff0b2e7790a47a. Только Linux.
Это диагностический результат, а не доказательство stale fence.

Observer tools/tests/check_maintenance_generation_ui.py загружает неизменённый
qrc Main, проверяет его SHA256, читает actual backend snapshots и native clipboard.
Настоящий interval validator получает ZERO из retained example PR136; A=1,
оба scheduler_validation contexts имеют generation="1". Штатная кнопка copy
успешно экспортирует strict V2. Затем Settings → MaintenanceView.submit отправляет
настоящий backup с новым destination (пути формы задаёт observer, FileDialog не
автоматизируется). Snapshot/context/generation не присваиваются.

Два запуска дали request_id=1 draining → failed/storage, ready=false,
maintenance_available=false, generation остаётся 1. Второй запуск корректно
вызывает shutdown и требует closed snapshot и terminal Qt exit. Первоначально публикация backup не проверялась; healthy verified backup, restore, replacement B и отказ stale copy
не доказаны. Первый запуск сохранил deadline failure; повтор не менял backend,
только обработку failed ACK для штатного shutdown. Успех процесса observer
означает сохранённое наблюдение, а не успешный backup.

Evidence вне TTL:
/home/myrs/projects/yonik/local/validation/p11-maintenance-generation/terminal-ru/
(observations.json, qt.log, observer.cpp/.so, driver.qml, encrypted fixture).
Первый failed прогон: соседний actual-ru/. Синтетический ephemeral key в памяти,
SecretService/TUN/physical proof не заявляются.
Retained binary:
/home/myrs/projects/yonik/local/validation/p11-interval-132/p11schedulerIntervalE2E
SHA256 e32c4cd2554f3e0f4b68bfd6379f5082e5ae62a08199835477c45853fc5c26ce.
Rust inputs в этой ветке не менялись; Cargo/build/новый transient root не создавались.
MSRV1.85.1 all-targets clippy -D warnings и fmt для этого binary сохранены в
parent evidence p11-interval-132/{clippy,fmt}.log; повторные проверки Rust не выполнялись.
Pinned Ruff 0.17.4 check и format --check observer: PASS.

## Предложение root до дальнейших Rust изменений

1. Разрешить отдельный закрытый example mode без нового HostConfig/enum/ABI:
одноразовый ZERO на startup, затем поддерживаемый interval после настоящего
maintenance drain. Требуется приватный test-support timing barrier в supervisor:
после accept_maintenance_owner и emit complete ACK, до следующего
runtime.set_interval. Управление — только example-owned synchronisation, без
Bridge/env/preferences входа; callback не присваивает diagnostics.
2. Сначала локализовать failed/storage при настоящем backup с existing fixture;
не обходить его поддельным ACK или replacement. Нужен matching successful
backup и проверенная копия перед restore.
3. Пока barrier удерживает итерацию B, Main copy должен отказать на сохранённом
A и оставить clipboard без изменения. После release и supported interval
проверить отсутствие экспорта A. Без barrier ZERO повторно применяется в начале
каждой итерации backend.rs:3502–3505, поэтому fault B может сразу заменить A;
это само по себе старый fence не доказывает.

Новый hook требует согласования root. Pins/capabilities/ABI/dependencies не менялись.
RU/EN два успешных lifecycle cases, Android и физические gates остаются открытыми.

## Локализация failed/storage: различающий контроль

Продолжение от 8d87f465fbd8c236a7536d1bd8df4fc80b4cc99f. Третий ZERO
прогон не выполнялся. Новый supported-control использует тот же retained binary
(SHA256 выше), свежий encrypted repository с одной валидной source и тот же UI
путь; единственное изменение input — существующий аргумент `supported` (86400).
Никаких Cause/state/generation injection, timing barrier или обхода drain нет.

Результат: request_id=1 backup complete, ready=true, generation=3; request_id=2
restore complete, ready=true, generation=5. В обоих случаях error_contexts пуст.
Shutdown дал closed и terminal Qt exit. Evidence:
`/home/myrs/projects/yonik/local/validation/p11-maintenance-generation/supported-control/`
(observations.json, qt.log, driver.qml, backup.sqlite, restored.sqlite).
SQLite integrity_check всех трёх файлов (включая старый terminal-ru/backup.sqlite)
вернул ok. Production copy перед публикацией валидирует candidate с исходным
AEAD key (repository/backup.rs); успешный restore дополнительно читает новую
backup-копию с тем же key. Это healthy positive control, без Corrupt.

Точная причина исходного отказа: backend.rs перед set_interval записывает ZERO
в DesktopSchedule.interval. Runtime отклоняет input InvalidLimits, но поле
остаётся ZERO, хотя runtime сохраняет прежний валидный интервал. Далее
maintenance_job::run_with_leaf создаёт replacement ДО leaf; replacement копирует
ZERO и повторно получает InvalidLimits. Этот Result хранится до завершения
copy_closed/reopen. В ветке Err(replacement) вновь открытые repository/service
закрываются, handles=None, cause=None, result=storage. Поэтому UI показывает
failed/storage и generation=1 после реально опубликованного backup. Старое
утверждение «Backup не опубликован» неверно: terminal-ru/backup.sqlite существует,
40960 байт, integrity_check=ok. Retained stderr содержит только scheduler_validation,
без maintenance copy/reopen/access cause — соответствует именно этой ветке.

Drain failures не объясняют наблюдение: их ветка возвращает cleanup_failed до
leaf; AdmissionFailed не скрывается. Key клонируется из открытого repository в
MaintenanceAccess, supervisor удерживает instance lock; destination новый.
Positive control проходит тот же key/lock/path путь и обе операции.

## Минимальный proposal root (не реализован)

В supervisor вычислять requested_interval локально; присваивать scheduled.interval
только после успешного runtime.set_interval. InvalidLimits продолжает публиковать
реальный scheduler_validation A, а replacement наследует последний валидный
interval. Добавить regression: valid interval → actual rejected ZERO → maintenance
backup/reopen → complete + новый owner с прежним interval. Это затрагивает
production supervisor и требует root approval. Публичные preferences уже
валидируются; воспроизведение доказано для test-support override, внешнюю
достижимость ZERO в production не заявляем. Данный результат закрывает причину
backup failure; stale A/B fence и timing proposal остаются отдельными вопросами.

Harness добавляет только --supported-control и проверяет отсутствие BLOCKED,
два matching complete ACK и пустые error_contexts. Исходный ZERO case сохранён.
Rust не менялся; Cargo и transient root не создавались. MSRV/clippy/fmt evidence
retained binary остаётся parent p11-interval-132; новых Rust PASS не заявляется.
