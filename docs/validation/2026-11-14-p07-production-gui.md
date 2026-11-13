# P07: production qrc GUI, strict HTTPS и отмена под нагрузкой

Продолжение PR177 от `f77013f8c095ba4a49a3034d1da9976b22e976b9`, без merge/rebase
с чужими runtime refs. Исходная component-приёмка сохранена отдельно.

## Реально проверенный путь

Accepted `tools/tests/check_scheduler_production_ui.py::INTERPOSE` сначала
загружает **исходный qrc Main.qml**, печатает SHA256 ресурса, затем создаёт
независимый observer с productionWindow через QQmlContext. SHA сверяется с
исходником Main.qml. Main, LogsView, application.cpp и shared helpers не менялись.

В `examples/p07_gui/bridge_impl.rs` находится private-копия production bridge.
Harness удаляет из неё единственный hook p07_logs и нормализует include-путь,
после чего требует побайтового совпадения с production bridge_impl.rs. Доставка
snapshots использует исходный bounded coalescing PendingSnapshot. Hook получает
существующий test_event_log того же actual DesktopHost и запускает producer;
остальные команды, синхронизация, startup/shutdown и readSnapshot неизменны.
Синтетический encrypted store использует ephemeral key, host profiles/SecretService
пользователя не читаются. Это **GUI с настоящим DesktopHost**, а не headless witness.

Observer вызывает существующие clicked-сигналы видимых enabled production кнопок,
как accepted scheduler GUI harness. Это проверка Qt handlers/bindings, не физический
mouse/input latency. Используются offscreen/software и тестовый timer 25 мс.

## Фиксированная нагрузка

На каждом из Xray 26.10.31 и sing-box 1.14.4 — три одинаковых коротких прогона.
Каждый прогон:

1. FR07 import_source с loopback HTTP: 65 профилей (LocalSocks health + 64 distinct
   Trojan credentials на один контролируемый TCP backlog). Выбран LocalSocks.
2. Production Connect/Stop проверяют три strict-оси: trusted CA + HTTPS 204;
   отдельный чужой CA → Limited/Tls без HTTP запроса; SOCKS peer refusal →
   Limited/Transport. Peer проверяет точные destination bytes 198.18.0.1 и port.
   Сервер trusted HTTPS получает запрос; untrusted HTTPS не получает HTTP.
3. Новое подключение удержано peer до SOCKS CONNECT response в Checking. Под ним
   стартуют producer журнала, batch ручных TCP-проверок и import_text. Последний
   нормативно получает busy; ни один contract не ослаблен ради нагрузки.
4. Producer делает 500 test_fill-пачек по 2 005 типизированных событий, интервал
   сна 10 мс: 1 002 500 попыток; фактическая длительность и потери записаны.
   Это синтетический EventLog producer, не native stderr/stdout.
5. Production LogsRefresh читает страницу <=256. Окно скрывается на >=600 мс.
   Observer считает реальные stateChanged/pageChanged и проверяет неизменность
   page.cursor. Ни одна строка producer не приходит отдельным событием в UI.
6. Окно показывается; production BatchCancel → cancelled && !busy. Через 200 мс
   публикуется marker drained; ещё 300 мс до Stop выделены для внешнего oracle.
   На первом наблюдении drained pending SYN-сокетов batch уже 0, active core ещё 1.
7. Production mainConnectButton отправляет Stop. После Disconnected/can_stop=false
   engine executable отсутствует; peer удерживаемой health получает EOF. Затем
   завершается producer и выполняется shutdown. Terminal owned group пуста.

CancelBatch **не создаёт и не отменяет engine runtime**: backend/batch_diagnostics.rs
вызывает check_tcp_batch с concurrency=8 и двумя каналами по 16, а tcp_probe.rs
открывает TCP sockets без payload. Поэтому отдельно доказаны CancelBatch → socket
cleanup и Stop → cleanup active LocalSession/core; временный batch core не выдуман.

## Изоляция и oracle

Private user/net/mount/PID/proc + private D-Bus, только lo; TUN и внешний трафик
отсутствуют. Сеть 198.18.0.1 — только запрошенный SOCKS destination, peer пересылает
в own loopback TLS. Runtime-16 runner и прежние 100-цикловые программы не запускались.

Disk sink нормативно отказал `/home` ancestor с unmapped root UID внутри userns.
Финальный fixture bind-mount собственного evidence root на `/mnt` только в private
mount namespace, с rprivate propagation. Все ancestors теперь own UID; исходная
проверка `event_log/disk.rs::directory` осталась строгой. Host `/mnt` не изменён.

Внешние выборки каждые 20 мс плюс стоимость /proc фиксируют PID/starttime, FD,
threads, PSS/private dirty, executable и pending SYN к выделенному batch-port.
Core исполняется из sealed `/memfd:yonik-engine (deleted)` (host/binary.rs).
Oracle не исключает zombie из terminal group. Краткоживущий процесс может не попасть
в sampling; terminal check независим. Negative control удерживает собственные
>=8 MiB и FD, обнаруживается, затем terminate/wait возвращает группу к пустой.

Наблюдалось ровно до 8 pending batch sockets во всех 6 финальных прогонах, после
отмены 0. Страницы ограничены 256, ring eviction >1 млн. Disk rotation реально
работала: максимум два файла из разрешённых пяти, каждый <=4 MiB. Размеры сохранены
в JSON. Command queue (128), disk producer queue (128), batch channels (16) и кольцо
(2 000) имеют исходные константные границы; этот workload проверяет их поведение
под log pressure, отдельная saturation всех типов команд здесь не заявляется.

## Границы GUI доказательства и координация W3

За интервал hidden в каждом прогоне: 0 log page changes, 1 stateChanged (visibility),
cursor неизменен. Это прямое наблюдение отсутствия UI обработки каждой строки.
GUI управление проходит реальные handlers во время producer; latency измерена
до наблюдения snapshot и включает 25-мс sampling driver. Продуктовый budget не задан.

**Passive GUI frame/stat-poll accounting пока не выполнен**: в accepted
application.cpp есть smokeSteps 25 мс и 45-секундный smoke deadline. Это не idle/
energy witness и не утверждение о нулевых кадрах скрытого окна. Root сообщил, что
W3 P01 владеет общими application.cpp/observer/bridge stats hooks; private API commit
ещё не передан. Общие файлы параллельно не менялись. После передачи API возможен
passive rerun; физический Wayland/input, P01 и энергия этим отчётом не закрываются.

## Проверки, воспроизведение и evidence

Rust 1.85.1, jobs=1, with-rust-slot, transient CARGO_TARGET_DIR под flock.
`cargo build -p yonik-desktop --features gui,test-support --example p07_gui -j 1`.
Затем `python3 tools/p07_gui.py ABS_BINARY ABS_CORES ABS_NEW_OUTPUT`.
Output не должен существовать. Проверка CLI без gui/test-support собирается и
возвращает явный unsupported при попытке запустить не тот example, без manifest edits.

GUI clippy all-targets -D warnings; default-feature cargo check обоих P07 examples;
rustfmt; pinned Ruff 0.17.4 check/format — PASS. C++ compiler выдаёт предупреждение
Qt/GCC16 QChar SFINAE; Rust clippy warnings отсутствуют.

Финальная приёмка `local/validation/p07-gui/attempt-6`: 6/6 GUI + negative PASS.
Попытка 1 выявила ошибку прав own data dir 0755 вместо 0700. Попытки 2–4 выявили
ошибки внешнего oracle: путь sealed memfd и повторную проверку сохранности runtime
уже во время начавшегося Stop до появления его snapshot. Финальный oracle проверяет
сохранность на первом drained marker до Stop; raw samples и failed Qt logs сохранены.
Attempt-5 дал 6 GUI PASS с отказавшим disk sink; именно attempt-6 принят с активным
sink. Ни один предыдущий runtime результат не присвоен другому бинарнику.

Сводка: p07-gui-evidence.json. Exact source archive/executable/source/Qt log bindings:
p07-gui-bindings.json. Raw samples, own cert fixtures и бинарник сохранены вне TTL в
`local/validation/p07-gui/` этого worktree. Приватные fixture keys не коммитятся.
Температура/питание не контролировались; kernel/Qt/backend записаны в JSON.

## Выборки и разброс (финальные три прогона на ядро)

| Ядро | Cancel snapshot, мс | Stop snapshot, мс | Max driver tick gap, мс |
| --- | --- | --- | --- |
| xray | 25 / 30 / 25 | 42 / 37 / 39 | 48 / 47 / 49 |
| sing-box | 37 / 37 / 28 | 55 / 36 / 50 | 49 / 57 / 48 |

Никакой p95/p99 из трёх наблюдений не вычислялся; абсолютный latency budget не вводился.
