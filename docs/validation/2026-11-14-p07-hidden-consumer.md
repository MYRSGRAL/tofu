# P07: focused passive hidden consumer gate

Продолжение PR177 после root fixes `1d0b5a9ce316f4488de327ee7818b7fb280e73ad`.
Root разрешил test-private observer без ожидания W3 API. Общие application.cpp,
bridge, QML и observer hooks W3 не менялись. Прежние шесть GUI/TLS/CancelBatch
прогонов не повторялись и не присвоены новому binary.

## Область результата

Закрыт согласованный **bounded hidden-UI gate P07**: при интенсивном producer
нет per-row delivery в перечисленные реальные UI consumer callsites и нет
автоматических logs_read. Это не утверждение о нулевой внутренней работе всего
приложения и не энергетическая/физическая GUI приёмка P01.

Остальная P07-проверка остаётся привязанной к отдельному GUI campaign:
[production GUI/TLS/CancelBatch](2026-11-14-p07-production-gui.md), исходные
p07-gui-bindings.json и component schema2 p07-concurrent-binding.json не изменены.
Новый focused example не запускает cores; оси обоих pinned cores уже проверены
неизменным прежним workload. Не выполнялись runtime-16 или прежние 100 циклов.

## Фактические callsites и source trace

`tools/p07_hidden_observer.cpp` перехватывает только исходную загрузку qrc Main,
проверяет её SHA256 и после загрузки устанавливает fixed C++ signal connections.
Ни подмены QML, ни пользовательского QML exec, ни изменения product flow нет.
В production Main используются реальные кнопки Settings/Diagnostics/LogsRefresh.

Наблюдаются независимо:

- **Admission команды**: private bridge печатает READ_ADMITTED только после
  успешного actual DesktopHost.dispatch для JSON command=logs_read. Это не serial
  и не общий snapshot counter. Ошибка admission не увеличивает этот trace.
- **Вызов чтения**: настоящий LogsView.serialChanged плюс значение serial.
- **Применение/consumer callback**: LogsView.pageChanged и responseChanged.
- **Модель**: notify настоящих logsRows.model/count, фактический count.
- **Delegates**: создание конкретных объектов с modelData, их modelDataChanged
  и textChanged. Начальное modelData может установиться до connection; поэтому
  ноль modelDataChanged не объявляется отсутствием всей инициализации. Canary
  положительно подтверждает создание и реальные bound text updates.
- **Кадры**: QQuickWindow.frameSwapped через direct connection с atomic counter.
- snapshot_jsonChanged считается отдельно, **не используется как row counter**.

Static trace закреплён SHA256 Main.qml и LogsView.qml: LogsView.read увеличивает
serial, ставит pending и отправляет logs_read; два callsite — явные кнопки Refresh/
Next. В LogsView нет Timer. onResponseChanged применяет page только при
visible && pending != empty && response.request_id == pending. Модель logsRows
зависит от visible/stale/page. Main.readSnapshot делает JSON.parse и fresh state,
поэтому response notify отдельно учитывается даже без page apply.

Private focused bridge отличается от production **только телом dispatch**:
producer hook и trace успешных logs_read. Runner заменяет этот метод исходным,
нормализует relative include и требует полного побайтового совпадения остального
модуля. Исходный coalescing PendingSnapshot и real DesktopHost сохранены.

## Passive phase и canaries

Новый p07_hidden binary вызывает production run_application с smoke_test=false:
нет 25-мс smoke driver и 45-секундного smoke timeout. В test-private C++ только
singleShot на границах фаз и общий failure deadline; periodic timer отсутствует.
В измеряемом hidden interval нет screenshot, update/requestUpdate, UI actions,
property polling или QML script. Qt notifications лишь увеличивают простые counters.

На прогон producer делает 500×2 005 typed EventLog attempts с паузой 10 мс между
пачками: 1 002 500 попыток. Наблюдатель progress пишет witness после каждых 50
завершённых пачек. Внутри hidden interval положительно наблюдалась разница
>=100 250 attempts между двумя progress witnesses. Это нижняя граница фактической
нагрузки внутри измеренного интервала, не заявленная без измерения скорость.

Порядок проверки:

1. Visible Refresh должен реально увеличить admission, serial/page/model и
   create/data/text counters. Проверяется наполненная настоящая ListView (256).
2. Отдельный frame canary вызывает один window.update **до hidden phase** и
   требует новый frameSwapped. Исходные startup frames не засчитываются.
3. Quiet visible negative interval 150 мс без действий: consumer counters не растут,
   несмотря на продолжающийся producer. Никакого bump counters в fixture нет.
4. hide; 200 мс transition settle, затем пассивные >=1200 мс. Снимается delta
   реальных callsites: reads/serial/page/model/count/delegate notifications=0;
   frameSwapped=0. Snapshot/response учитываются отдельно с fixture bound <=4,
   не привязанным к числу строк и не являющимся продуктовым latency budget.
5. show и новый visible Refresh снова обязаны увеличить admission/serial/page:
   observer и consumer остаются живы после hidden phase.

Два fault controls тем же focused binary: отключённые notify connections обязаны
провалить visible canary при реально принятом logs_read; отключённая frame connection
обязана провалить frame canary при живых notify counters. Нули сломанного observer
не могут пройти hidden gate. Это отключение connections, не fakecounter bump.

## Изоляция и сохранность

Три одинаковых positive прогона и два отдельных negative controls. Private
user/net/mount/PID/proc/D-Bus; lo only. Own synthetic encrypted store/ephemeral key,
пустой engine-dir; host profiles, TUN, чужие UID/сокеты и пользовательский keyring
не используются. Disk sink работает через private bind alias собственного output
на /mnt; ancestor policy не ослаблена. После каждого процесса terminal owned group
пуста, включая negative controls. Внешний namespace oracle не исключает zombie.

Raw Qt logs, observer .so/moc/source, binary и exact source archive сохранены вне TTL
в local/validation/p07-hidden/. Новый p07-hidden-bindings.json разделяет focused
campaign и неизменные прежние campaigns. Старые raw bindings не редактируются.

## Фактические выборки

| Прогон | Hidden duration, мс | Reads / page / model / delegate / frames | Frame canary | Producer duration, мс |
| --- | --- | --- | --- | --- |
| positive-0 | 1201 | 0 / 0 / 0 / 0 / 0 | 1 | 5463.356 |
| positive-1 | 1201 | 0 / 0 / 0 / 0 / 0 | 1 | 5522.870 |
| positive-2 | 1201 | 0 / 0 / 0 / 0 / 0 | 1 | 5486.230 |

Финальные три positive PASS; оба disconnected-observer controls дали ожидаемый FAIL
в соответствующем canary, до hidden PASS. Terminal group пуста во всех пяти случаях.
Первый focused campaign также сохранён: перед final observer уточнён так, чтобы
однократная startup sample не увеличивала snapshot signal counter. Rust binary
не пересобирался; повторены только focused controls для новой observer .so.

Rust 1.85.1 build и clippy gui,test-support --all-targets -D warnings — PASS;
default-feature cargo check focused example — PASS. Rustfmt и pinned Ruff 0.17.4
check/format — PASS. C++ observer собран Qt6 moc + clang++; первоначальный compile
показал, что системный /usr/bin/moc относится к Qt5, поэтому runner использует
точный /usr/lib/qt6/moc. Exploratory compile logs сохранены вне campaign.

Воспроизведение: собрать `cargo build -p yonik-desktop --features gui,test-support
--example p07_hidden -j 1` с внешним CARGO_TARGET_DIR под flock/with-rust-slot;
затем `python3 tools/p07_hidden.py ABS_FOCUSED_BINARY ABS_NEW_OUTPUT`.
Output должен отсутствовать. Новая observer .so компилируется runner из own C++,
production/shared файлы не патчатся. Полный agreed P07 подтверждается совокупностью
неизменного прежнего GUI campaign и этого focused bounded consumer proof;
общий zero-internal-work, P01, энергия и physical Wayland/input не заявляются.

После сохранения exact source/binary/observer/log bindings проверены /proc exe/cwd/fd
references: активных ссылок на build root нет. Cargo clean удалил 8 485 файлов,
3.6 GiB; собственный /var/tmp/codex-myrs/transient/p07-hidden удалён. Cleanup log
сохранён вне TTL. Cargo.lock возвращён к accepted байтам; зависимости не менялись.
Чужие untracked cross-UID файлы сохранены и не включены.
