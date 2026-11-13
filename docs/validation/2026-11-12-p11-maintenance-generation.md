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

## Продолжение: строгий observer до доставки fixed binary

Observer теперь сохраняет BEFORE COPY и COPY RESULT штатной DiagnosticsExport,
проверяет positive strict V2 A против actual error_contexts, source/error slots,
stage/reason и scheduler generation. Для двух matching complete ACK обязательны
ready, рост generation и неизменность snapshot во время copy. После replacement:
STALE_REFUSED требует отказа кнопки и сохранения clipboard; INVALIDATED требует
пустых actual contexts, отказа и сохранения clipboard. Новый context B с текущей
generation классифицируется B_OVERWROTE_A_UNPROVEN: старый fence не доказан.
Экспорт B при успехе проходит strict V2 и сравнение с actual contexts.

Шесть локальных unittest controls проверяют классификацию и отклонение stale
success, неверного request_id и изменения clipboard при отказе. Это синтетические
controls Python assertions, они не инжектируются в GUI и не являются actual
acceptance. Pinned Ruff 0.17.4 check / format --check и py_compile прошли.
Rust/backend не изменены; Cargo не запускался, transient root не создавался.

Actual прогон ожидает verified path/hash нового root binary после terminal
сборки. Старые failed backup не повторялись, healthy control не повторялся.
При B_OVERWROTE_A_UNPROVEN root нужен закрытый one-shot input/timing control;
barrier, Cause, catalog и HostConfig этим изменением не добавлены.

## Fixed binary: actual RU stale fence подтверждён

Root передал binary из PR143, source fix 21cc500 на b79:
`/home/myrs/projects/yonik/local/evidence/scheduler-interval-transaction/p11schedulerIntervalE2E`,
SHA256 `d03f6ed9f6b8ee654a3e74df0373b37b58e4bba8b0e74df908e447d912204a73`.
Hash проверен локально. Manifest связывает binary и production Main SHA256
`277a8f30075168f0d4be76ae89ff3d75ac8234a057920aec12bf79976c6c19c5`.
MSRV1.85.1 build/clippy all-targets -D warnings/fmt terminal PASS переданы root;
локально прочитаны build.log, clippy.log, fmt.log в той же evidence-папке.
verify.sh только прочитан, не запущен повторно.

Выполнен один новый actual GUI lifecycle через offscreen/private D-Bus, неизменённый
qrc Main, native clipboard и настоящую MaintenanceView.submit. A=1 имеет actual
source/error scheduler_validation scheduler_interval_invalid и положительный
strict V2 export. Backup request_id=1 complete, ready=true, owner=3 сохраняет
contexts=1; штатная copy возвращает отказ и сохраняет clipboard A. Restore
request_id=2 complete, ready=true, owner=5 сохраняет contexts=3; copy снова
отказывает, clipboard неизменён. Snapshot до/после copy совпадает. Результат
`STALE_REFUSED,STALE_REFUSED`, closed=true. Исходный terminal exit code
не сохранён; последующий узкий повтор ниже подтверждает exit=0. Повторного
fault, успевшего заменить stale context до copy, не наблюдалось; barrier не нужен.

Первичная post-processing проверка остановилась на несовпадении Main старой
ветки с новой базой binary. Сам GUI завершился успешно; лог сохранён. Observer
получил явный --binary-manifest, проверяющий hash binary и embedded Main против
manifest, и --assess-existing для оценки сохранённого лога без GUI-повтора.
Restore assertion уточнён для промежуточного stale owner=3, с отдельным control.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/p11-maintenance-generation/fixed-transaction-ru/`
содержит qt.log, observations.json, observer.cpp/.so, driver.qml, host,
backup.sqlite и restored.sqlite. Обе SQLite-копии 40960 байт, read-only
integrity_check=ok; matching successful restore дополнительно подтверждает чтение
backup с исходным синтетическим key. Snapshot/context/state не присваивались.
Старые failed и healthy evidence сохранены, не повторялись.

7 observer controls, pinned Ruff0.17.4 check/format --check и py_compile PASS.
Rust/backend не менялись, Cargo не запускался, transient root отсутствует.
Это actual RU proof, не SecretService/TUN/физический или Android proof; EN
lifecycle данным запуском не проверялся. Base PR141 переводит root после
координации зависимостей PR143; merge не выполнялся.

## Fresh EN acceptance на том же PR143 binary

Отдельный seed-процесс того же verified binary через Settings → Appearance →
languageChoice сохранил EN preferences, затем дал closed и exit=0. Его retained
contexts после смены языка НЕ использованы как EN proof. Seed evidence:
`/home/myrs/projects/yonik/local/validation/p11-maintenance-generation/fixed-en-preferences-seed/`
(driver.qml, qt.log, process.json, host/data/preferences.json).

Следующий отдельный процесс создал новый encrypted repository и scheduler A=1,
загрузив копию этих preferences до старта backend. Все actual snapshots A,
replacement, before/current copy и closed имеют language=en. Preferences warning
отсутствует. Production button имеет текст Copy diagnostics, positive result
Copied, strict V2 содержит actual source/error scheduler_validation A=1.
Backup request_id=1 complete, owner=3/contexts=1; restore request_id=2 complete,
owner=5/contexts=3. В обоих случаях штатная кнопка возвращает точный английский
отказ Export unavailable, native clipboard остаётся равен начальному export A.
Snapshots до/после copy совпадают. Результат STALE_REFUSED,STALE_REFUSED,
closed=true, сохранённый process.json exit_code=0. Barrier не добавлялся.

EN evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/p11-maintenance-generation/fixed-transaction-en-accepted/`
содержит observations.json, qt.log, process.json, driver.qml, observer.cpp/.so,
input-preferences.json (0600), host, backup.sqlite/restored.sqlite и атомарно
опубликованный acceptance-manifest.json с hashes файлов и integrity results.
Обе SQLite-копии по 40960 байт, read-only integrity_check=ok. Binary SHA256
`d03f6ed9f6b8ee654a3e74df0373b37b58e4bba8b0e74df908e447d912204a73`,
embedded Main SHA256 `277a8f30075168f0d4be76ae89ff3d75ac8234a057920aec12bf79976c6c19c5`
проверены против root manifest PR143. Финальные locale assertions дополнительно
проверены на сохранённом логе без нового GUI запуска.

Промежуточные EN FAIL сохранены в fixed-transaction-en/, fixed-transaction-en-ready/
и fixed-transaction-en-fresh/. Причина локализована: observer создавал input copy
preferences с 0644, production Store::read требует 0600 и корректно отклонял файл
с preferences_warning и RU fallback. Это не несовместимость формата. Исправлены
права промежуточной копии; добавлена проверка отсутствия warning и locale, а также
сохранение process exit code. Старые FAIL и healthy evidence не изменены.

7 controls, pinned Ruff0.17.4 check/format --check и py_compile PASS. Rust/backend
не менялись, binary не пересобирался, Cargo/transient root отсутствуют. Actual
maintenance clipboard lifecycle теперь проверен отдельно RU и fresh EN.
SecretService, Android и физические gates остаются открытыми. Root переводит
PR141 на main после merge PR143; feature-parent merge не выполнялся.


## RU terminal evidence: узкое исправление finding review

Первый RU каталог не содержит process.json: closed в Qt-логе и assess-existing
не удостоверяют exit code того процесса. Старый результат не выдаётся за exit=0.
Root повторил только RU scenario на том же PR143 binary и frozen driver4dc9,
через новый private bus без service activation, private HOME/XDG и offscreen.
Новый прямой запуск, не assess-existing, сохранил process.json exit_code=0,
closed и два STALE_REFUSED после matching complete backup/restore ACK.
Clipboard остался равен export A; обе SQLite read-only integrity_check=ok.

Evidence: /home/myrs/projects/yonik/local/validation/p11-maintenance-generation/fixed-transaction-ru-terminal-root/acceptance-manifest.json.
Manifest связывает binary/Main, frozen harness и terminal/artifact hashes.
Первая попытка root не дошла до GUI из-за отсутствия PYTHONPATH=tools;
после явного задания import path выполнен один успешный GUI запуск.
EN campaign и Rust-сборка не повторялись; старый RU каталог сохранён.
