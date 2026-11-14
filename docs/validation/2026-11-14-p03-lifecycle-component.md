# P-03: actual GUI lifecycle component, 3×100

Новая task-ветка от fresh origin/main на момент старта:
`7e909c754ea2798a2c5615702224391b19e82ee1`. Замороженные 183/177 refs и
чужие untracked cross-UID файлы не изменялись и не исполнялись. Общие
application.cpp/build.rs/bridge/QML и W3 observers не редактировались.

## Результат и границы

**Component PASS; полный P-03 остаётся открытым.** Три отдельных процесса по
100 настоящих циклов: Settings → Routing → Advanced → Simple → Back (выгрузка
Loader) → Back → QWindow.close (production CloseHandler) → production local
socket `show\n` с ответом `ok\n`. Никаких Qt property writes вместо controls.
Первые 10 циклов каждого процесса классифицированы warmup, остальные 90 measured.
Исходные P07 GUI/TLS 6+3 campaigns не повторялись; runtime-16/core/connect/TUN
не запускались. Один дополнительный exploratory 100-cycle driver smoke на старом
retained ELF не включён в результат fresh-base campaign.

Новый example включает исходные production bridge.rs/bridge_impl.rs/dispatch_error.rs,
реальный DesktopHost и encrypted storage с отдельным ephemeral key. C++ interposer
по образцу 177 перехватывает исходную qrc загрузку, фиксирует SHA256 Main и
наблюдает реальные объекты после загрузки. QML не заменяется. Изоляция:
user/net/mount/PID/proc/D-Bus namespace, только lo, собственный /mnt bind alias,
никаких пользовательских профилей, SecretService или engine binaries.

Offscreen сам по себе не предоставляет tray: настоящий CloseHandler оставлял
окно видимым. Отдельная проверка installed KDE platform theme плюс private D-Bus
StatusNotifierWatcher дала tray_available=true. Этот fixture только объявляет
tray host; он не меняет CloseHandler, bridge или host. Открытие проходит через
production QLocalServer/QAction обработчик. Это **не physical Plasma/G-UI/GPOWER**.
Новые зависимости и настройки хоста не устанавливались.

## Oracle и отрицательные контроли

Каждый редактор содержит 376 фактически наблюдаемых QObject descendants, из них
один класс Connections. После Back все сохранённые QPointer обнуляются, включая
editor. Количество стабильно на 300 циклах. Advanced/Simple вызываются через
реальный `routingModeButton.clicked`; после возврата в event loop проверяются
advanced=true/false и ровно один actual notify на действие. Политика routing,
routing_revision и revision сравниваются до/после каждого цикла.

Три controls тем же ELF/observer: отключить notify connection; добавить второй
notify connection; пропустить Back и оставить editor. Первые два проваливают
advanced canary, третий — освобождение descendants. Нулевые/дублированные counters
и реально оставленное дерево не проходят. Это oracle конкретного дерева, а не
глобальный подсчёт всех subscriptions или внутренних handlers приложения.

## Измерения

[Сырые per-cycle показатели и summary](p03-lifecycle-evidence.json) содержат PSS,
private dirty, FD, threads, phase, cycle и warmup classification. В local raw logs
дополнительно сохранён исходный smaps_rollup каждого наблюдения. Вычитаний памяти,
коррекций baseline и суммирования RSS нет. После прогрева FD=30; threads переходят
11→9, поздние окна имеют 9. Поздние диапазоны PSS/private dirty перекрываются;
непрерывного накопления editor objects нет. Это ограниченная стабилизация на
100 циклах, не доказательство отсутствия всех долгосрочных утечек.

p95 определяется nearest-rank на 90 measured samples отдельно для каждого процесса.
Таймер начинается **до dispatch** и заканчивается после event-loop observation
ожидаемого состояния; reopen включает socket dispatch и ответ actual server.
Это время observable state, не длительность вызова кнопки. Полные исходные
значения сохранены; fixed 50 ms close observation **не объявляется close latency**.

Остаются непроверенными для полного P-03:

- p95 открытия правил и rendered-frame completion; physical/GPU latency;
- непустая составная политика и семантика её редактирования в двух режимах
  (в этом fixture неизменность проверена на пустой политике, revision=0);
- фактические admission/release backend subscriptions и накопление handlers
  за пределами наблюдаемого QObject дерева. Main.onVisibleChanged отправляет
  subscription_visibility, но внутренний subscription counter здесь не наблюдается.

Для продолжения shared API сейчас не требуется: actual production show command
установлен. Следующий bounded шаг — private observer/bridge admission trace по
образцу 177, наполненная policy fixture и action→frame/Loader latency. Общие
файлы W3 для этого не следует угадывать или править.

## Проверки и воспроизведение

Rust 1.85.1, jobs=1, общий with-rust-slot (максимум два слота), собственный transient
CARGO_TARGET_DIR под shared flock. Qt6 moc + clang++ observer. Ruff 0.17.4
check/format и rustfmt для новых файлов — PASS. Focused Clippy
`--example p03_lifecycle --features gui,test-support -- -D warnings` — PASS;
all-targets не запускался. Предупреждения GCC16 в Qt headers сохранены в build log.
Cargo.lock менял только порядок package/dependency entries; семантическое
сравнение подтвердило отсутствие dependency drift, accepted bytes восстановлены. Команда campaign:

`python3 tools/p03_preflight.py ABS_P03_ELF ABS_NEW_OUTPUT`

Сначала собрать example p03_lifecycle с gui,test-support. Output должен отсутствовать.
ELF, exact source archive, observer .so/moc, full logs и results.json сохранены вне
TTL в `local/validation/p03-lifecycle/`. Checked-in evidence связывает исходники
и ELF SHA256; local source archive и raw hashes записаны в bindings.
