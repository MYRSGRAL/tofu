# Приватный P01 observer v1

Только `yonik-desktop --features gui,test-support`. Generated build guard
`p01_build_guard.h` исключает observer из default GUI на этапе C++ compilation.
Зависимости и публичный HostConfig не меняются. Normal credentials — настоящий
Secret Service. Не передавать `--smoke-test`, script или smoke trace.

Внешний controller создаёт AF_UNIX/SOCK_STREAM socketpair, передаёт один конец
через pass_fds и `YONIK_P01_CONTROL_FD=<fd >= 3>`; второй остаётся controller.
Descriptor не открывает pathname/listener и не добавляет D-Bus/public API.
Модуль `src/p01_observer.h`, класс `yonik::P01Observer(int, QWindow*, DesktopBridge*, QAction*, QAction*)`.

Команды — ASCII line с LF: `Open`, `Hide`, `Snapshot`, `Quit`,
`VisibleFrameCanary`. Open/Quit используют существующие QAction; Hide вызывает
штатный CloseHandler через close(), который может отказаться скрывать без tray.
Controller обязан проверить `visible=false` до START hidden phase. Initial
show/setup/settle не входят в измерение. Canary вызывает QQuickWindow::update()
только при видимом окне; подтверждение — последующий Snapshot с ростом frames,
сам ответ на canary не означает уже завершённый кадр.

Каждая команда возвращает одну JSON line:
`schema:1`, `command:string`, `ok:bool`, `visible:bool`, `frames:integer`,
`visibility_events:integer`, `ui_stats_delivery:null`, `statistics_poll:null`,
`stats_support:"unsupported-source-has-no-statistics-path"`.
Также возвращаются fixed backend metadata: `ready`, `status`, `can_stop`, `error`.
`frames` — накопленный frameSwapped, atomic из-за render thread; visibility —
visibleChanged. Observer подключается до application.exec, но после QML load.
Отсутствие startup frames не проверяет. Нет per-frame I/O или своего таймера.
Снимок синхронен команде, счётчик frames может расти параллельно.

Ограничение: source frozen main не имеет statistics_poll и UI statistics
consumer. Полный поиск `statistics_poll|statistics|stats` по crates на базе
не обнаруживает такого production пути; bridge_impl доставляет snapshots,
которые не являются UI statistics. Поэтому stats counter и positive canary
не реализованы: вводить новый production путь или fake increment запрещено.
Root подтвердил N/A stats-positive-canary для frozen 3e64220. Это not measured,
не измеренный ноль. Проверены Main.qml, bridge.rs, bridge_impl.rs (initialize/
set_snapshot_json), backend.rs и весь crates поиск statistics_poll/statistics/stats.
Добавление production statistics инвалидирует именно эту N/A связь.
N/A не относится к log consumer/model apply/coalescing P07.

Вход ограничен 128 байтами; неизвестные команды возвращают ok=false.
Переполнение, EOF или backpressure ответа отключают notifier и вызывают штатный
Quit. Controller должен держать bus/tray/Secret Service до исчезновения GUI,
собственный внешний timeout и cleanup process group обязательны.
Все числа будущей кампании относятся к observer ELF с overhead label,
private software/offscreen fixture не является physical Plasma acceptance.
