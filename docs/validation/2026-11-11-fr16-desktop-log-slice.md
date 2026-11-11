# FR16: desktop-журнал событий

База: `d0a281b4c23b45f65db1ddb74c6b14378b399794`.

Backend создаёт один EventLog на DesktopHost и передаёт каждому LocalSession
через with_event_log. Журнал сохраняется между подключениями. Core, production
pins, version/config-check, routing и сетевые настройки не менялись.

`logs_read`: строковые u64 after/request_id, source all/app/engine.
`snapshot.logs`: ready/empty/busy/cancelled, request_id, cursor,
evicted/rejected/missed, максимум 256 событий, export из LogPage.export.
Строковые числа сохраняют точность в Qt JS. Очередь остаётся ограниченной 128
запросами. Stop использует прежний priority watch. В общем generation-discard
добавлен только согласованный logs cancel-ответ; он не содержит export.

LogsView включён одной строкой в диагностическую панель и зарегистрирован
в build.rs. Чтение, смена страницы и копирование запускаются явно. При смене
фильтра страница становится stale; busy/cancelled требуют повторного запроса.
Таймеров нет. Невидимый ListView имеет пустую модель. Ответ принимается только
для активного matching request_id, поэтому чужой ACK не заменяет принятую
страницу и не снимает pending. Copy использует только export принятой страницы
как PlainText. Файловое сохранение и отдельная команда export не добавлены.

## Проверки

Локальные evidence без TTL: `local/evidence/fr16/`.

- До production-кода QML-проверка упала на отсутствии LogsView.
  Первый backend-test был написан до кода, но завершил компиляцию уже после него:
  initial backend RED не заявляется.
- cancel-red.log → cancel-green-msrv.log: queued LogsRead → Stop → matching
  cancelled с export=null → новый успешный read. Два backend-теста проходят
  с Rust 1.85.1 и private D-Bus.
- clippy-msrv.log: Rust 1.85.1, --all-targets -- -D warnings проходит.
- fmt-msrv.log, qml-test.log, translations.log, qmllint.log: формат Rust,
  QML contract checks, строгий RU/EN checker и qmllint.
- unit.log: u64 bounds и native-secret canary не попадают в DTO/export.
- core-event-log.log: четыре проверки bounds/cursor/loss/busy/stop/reap проходят;
  отдельный native exit-status test ignored без явно выбранного образа.

Initial GUI собран stable rustc/cargo 1.97.0; это честно отделено от MSRV evidence
в toolchain.json. Initial clippy на 1.97 остановился на существующем
collapsible_match в yonik-engine/src/lib.rs:166; файл не менялся. Ruff отсутствует,
установок не выполнялось.

## Actual Qt/native

`tests/logs_native.py` запускает настоящий GUI/DesktopHost с pinned Xray и
sing-box из /home/myrs/projects/yonik/local/engines, synthetic loopback VLESS/TLS,
отдельным D-Bus и ephemeral DB. SHA обоих cores совпадают с production pins.
Профиль пользователя, keyring и настройки сети хоста не используются.

Initial acceptance прошёл для обоих cores: два start → Limited → Stop → reap,
пять событий на сессию, app=4 / engine=1 (ProcessSpawned классифицирует core),
RU/EN, clipboard без изменений после invalid cursor и с CleanupConfirmed после
явного copy. При закрытой панели второе подключение не обновляет список;
открытая заново панель пуста до явного чтения, после него содержит 10 событий.
Оба GUI завершились с closed=true. Первый прогон остановился на ошибочном
ожидании app=3; исправлено только ожидание теста.

GUI binary сохранён вне TTL: local/artifacts/fr16/yonik-desktop.
SHA binary и исходников до native-теста: native-manifest.json.
Evidence: native/{xray,sing-box}/{trace.jsonl,logs.png,stderr.log}, native.log.
Финальный прогон после cancel/stale-ACK изменений также прошёл: Rust 1.85.1,
44 шага для каждого core, closed=true. Дополнительно проверены чужой request_id
(не заменяет принятую страницу), отказ на неизвестный source с сохранением
clipboard, следующая пустая страница с cursor=5. Финальные evidence:
native-msrv/{xray,sing-box}/{trace.jsonl,logs.png,stderr.log}, native-msrv.log,
native-msrv-manifest.json, gui-build-msrv.log. Unit canary на MSRV также проходит
(unit-msrv.log). Сохранённый GUI binary — финальный MSRV-образ.

## Границы

Полный FR16 acceptance не заявляется. Отдельного synthetic ELF negative
exit/cleanup сценария и test-support hook нет. Ненулевые evicted/rejected/missed
и busy проверены в core/DTO, но не воспроизведены через actual Qt/DesktopHost под
нагрузкой. Отдельная команда export/save не реализована. Публичный core API
не расширялся для тестовой подстановки, IDs или сброса журнала.

Перед завершением pre-clean.json подтвердил 0 активных task-процессов и 0 открытых
handles build-каталога. Cargo clean удалил 11.9 GiB; собственный transient root
удалён. Binary/evidence сохранены в workspace вне TTL.


## Отказ передачи запроса GUI

P2 устранён: отказ очереди `logs_read` возвращает ACK с исходным ID, статусом busy и пустыми events/export. При отсутствии host запрос также завершается ошибкой startup. Общий путь routing ACK сохранён. Невалидный ID не попадает в ответ.

Регрессия RED: вместо ID 7 оставался ID старой страницы 6; GREEN: четыре Rust-теста общего helper. Выполнены rustc/clippy-driver 1.85.1 с существующей readonly serde_json rlib, fmt и diff-check. Полный GUI для этой дельты не пересобирался. Настоящий QtTest 6.11.2 проверил busy ACK → повторный запрос → игнорирование старого ACK → готовая страница; один поведенческий тест плюс init/cleanup. Stub dispatch в этом QtTest не воспроизводит реальное переполнение очереди. CLI qmltestrunner из PATH оказался Qt5 и вернул exit1; он не считается пройденной проверкой.

Evidence: `/home/myrs/projects/yonik/local/validation/pr98-logs-ack/`. Manifest записан после проверок; прежний native evidence не изменён. Actual GUI busy/loss и прочие открытые gates остаются открытыми.

### Доставка ACK через неполный snapshot и объединение с main

Повторная проверка обнаружила потерю startup ACK в early-return ветке `Main.qml`. Добавлен отдельный RED→GREEN: два Python/Node-теста выполняют фактическую функцию `readSnapshot()` из Main.qml, без её копии. ACK теперь сохраняется в state.logs при отсутствии каталога; обычная ошибка не создаёт страницу. Это тест функции, а не production failure GUI.

При объединении с main f3c937d сохранены обе команды LogsRead/RoutingExplain, начальные diagnostics/logs, системные и журнальные regression tests. Combined Rust 1.85.1: 30 library + 36 backend + 4 dispatch tests PASS, один прежний ignored. Clippy all-targets с feature gui выполнен явным driver1.85.1; fmt, translations 230/96 и Ruff нового Python-теста прошли. GUI-типизация проверена, новый runtime GUI-бинарник не выпускался; прежние native traces не приписываются этому combined head.
