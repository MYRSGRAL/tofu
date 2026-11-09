# Первый нативный desktop-host: проверка и границы

Рабочая реализация: Qt Quick/CXX-Qt 0.10.0, Rust 1.85.1, Qt 6.11.2, CachyOS,
KDE Plasma/Wayland. Это первая оболочка существующих адаптеров локального SOCKS;
G-UI, T-01…T-04, G-POWER и production network gates не объявляются закрытыми.

## Реализованный путь

QML → allowlisted JSON command → bounded Rust actor → существующие
SubscriptionService/Repository либо compiler/LocalSession. Обратно передаётся DTO
состояния; Qt получает последнее coalesced событие, без фонового polling.
Список источников читается страницами по 64 без дешифровки snapshot bodies,
профили — по 200 со stable IDs/revision. Имена/адреса выводятся plain text.

Фоновый host владеет Tokio runtime, БД и задачей сессии. Stop/Shutdown обходят
очередь 128 команд и инвалидируют старые queued requests. Подписка с ошибками
показывает точный отчёт до approval; source URL не входит в DTO/argv/trace.
Активный server/engine отделён от желаемого выбора. Existing private data directory
не chmod-ится; небезопасные права вызывают отказ. Ключ — Secret Service, тестовый
ephemeral key включается только явно для собственного отдельного каталога.

LocalSession запускает тот же sealed executable snapshot закреплённого ядра,
проверяет конфигурацию и HTTPS через выбранный outbound. `Limited` сохраняет ядро;
Stop/выход ждут stop/reap. Обработчики SIGINT/SIGTERM устанавливаются до children;
при ошибке регистрации запуск не разрешается. Qt Quit/close не подтверждают выход
раньше `closed`. CleanupError сохраняет доступ к управлению.

QApplication/QSystemTrayIcon имеют restore/connect/disconnect/quit, статические
различимые формы состояний, переводы и экранированную подсказку. Закрытие скрывает
окно только при доступном трее; без него запрашивает завершение. Отсутствие
StatusNotifierWatcher восстанавливает окно по D-Bus событию; polling нет.
QLocalServer/QLockFile в приватном runtime-каталоге не допускают второго GUI на тот
же canonical data directory; единственная IPC-команда — ограниченный `show`.

Qt GUI feature optional: общие Rust/Android проверки не требуют Qt. Backend lib
и handwritten bridge impl запрещают unsafe. Allow нужен только declarative
generated CXX bridge. C++ слой владеет Qt-объектами в GUI-потоке; worker использует
CxxQtThread.queue. CXX/cxx-gen закреплены на согласованном ABI 1.0.176/0.7.176:
первый link обнаружил несовместимость с автоматически выбранным генератором198.

## Проверки

- MSRV Rust 1.85.1: workspace — 146 Rust-тестов и 4 doctest; два inherited
  environment-dependent теста остаются ignored в hermetic прогоне.
- Семь actor-тестов: clean/partial/native import, approval, revision/favorite,
  bounded strict commands, priority Stop/Shutdown, exclusive flock/reopen,
  отмена зависшей HTTP-загрузки, сохранение чужих permissions, явный отказ Proxy
  и настраиваемый порт. Все URL/credentials вымышленные.
- Отдельный source-page тест: 65 источников, граница64, повреждённое тело snapshot
  не читается при получении имён; invalid offset отклоняется.
- Workspace clippy/fmt, GUI feature clippy, release GUI и Android default-feature
  compile-check. В GCC16/Qt6.11 есть vendor header warning `QChar` SFINAE;
  Rust `-D warnings` проходит. Это не заявление о чистом C++ warning baseline.
- `tools/desktop_smoke.py` прошёл четыре release-сценария с настоящими Qt/Rust
  bridge и синтетическим loopback HTTP: profiles, partial approval, Xray, sing-box.
  У обоих ядер наблюдались `Preparing/Checking/Limited`, затем Stop,
  `Disconnected` и `closed`; unreachable fixture намеренно не доказывает здоровый
  внешний proxy. Собственные PNG просмотрены: кириллица, plain-text HTML-like name,
  выбор/избранное, отчёт partial import и Limited. Реальная пользовательская
  подписка этим этапом не загружалась и в GUI trace не сохранялась.
- Engine-сценарии выполняются в собственном PID/mount/user namespace с тем же UID;
  завершение namespace страхует cleanup всех собственных процессов. Сеть хоста
  не меняется; порт выбирается свободный, занятый пользовательский10808 не тронут.
  Data-only CI-вариант `--no-namespace` не запускает ядра.
- Native Wayland startup, второй запуск того же data-dir (exit0/show) и shutdown
  через SIGTERM (closed ack/exit0) проверены отдельно. Кадры Wayland не получены:
  `loginctl` подтвердил `LockedHint=yes`. Сессия пользователя не разблокировалась.
  PNG и четыре GUI-flow проверены через Qt offscreen/software. Это не подмена
  приёмки живого Wayland, clipboard/keyboard и fractional scaling.

Синтетические trace содержат только whitelist metadata, без URL, host, имён и
команд. Screenshot снимает только собственный QQuickWindow, не рабочий стол.
Smoke script/trace/screenshot включаются только с `--smoke-test` и явным data-dir.
Capture не считается успешным без сохранённого PNG; таймаут завершает тест ошибкой.

## Что остаётся

На разблокированном KDE Wayland нужны визуальная/keyboard/clipboard/HiDPI проверки,
реальные tray-click/close/restore и restart watcher; сами системные службы не
перезапускались. Полный T-01…T-04 включает ещё меню избранного/недавних в трее.
PSS/CPU/energy UI-группы в фоне не измерены. Окончательный дизайн с пользователем
не утверждался: Qt — рабочий выбор для технической проверки, не закрытие ADR-005.

Persistent key creation через реальный Secret Service в этом GUI-прогоне не
выполнялся; ephemeral mode не является автоматическим fallback. GUI preferences
пока на сеанс, health URL фиксирован на generate_204, recovery supervisor,
TUN/system proxy/DNS, редактор политики и управление ядрами/установка ещё впереди.
Native JSON сохраняется opaque после review и не активируется произвольно.
