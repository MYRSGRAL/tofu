# Готовность сравнительного стенда Linux UI

База: `3968bdb7b57a2aed1e8281e54698fe3ec985f6f2`, branch
`t3/ui-bench-comparison`. Scope: существующие Qt/GTK4/Slint harness,
measurement/protocol, без изменения production backend, contracts и MSRV 1.85.
Android отложен. Бинарники от e2ef351 не переиспользованы: после них изменились
имена пакетов, QML module, заголовки и имена потоков. Это конкретный gap provenance,
а не повтор неизменённого сравнения производительности.

## Артефакты и воспроизведение

Постоянный каталог: `/home/myrs/projects/yonik/local/artifacts/ui-readiness-20261111/`.
`manifest.json` содержит source HEAD, tree bench/ui, SHA-256 исходников и executable;
он сохраняется до первого запуска. Сборка Rust 1.97.0 — release, thin LTO,
codegen-units=1, strip=symbols, locked, jobs=1 через общий `with-rust-slot`.
Transient root `/var/tmp/codex-myrs/transient/yonik-ui-readiness-20261111`
удерживается root-directory flock на протяжении каждой операции сборки/очистки.
Новые зависимости и системные пакеты не устанавливались.

Offscreen-команда (новый output обязателен):

```sh
python bench/ui/check-offscreen.py \
  --artifacts /home/myrs/projects/yonik/local/artifacts/ui-readiness-20261111 \
  --output /home/myrs/projects/yonik/local/artifacts/ui-readiness-20261111/offscreen
```

Она исключает DISPLAY/WAYLAND_DISPLAY и не соединяется с desktop display.
Проверяются одинаковые search/select/favorite/sort/scroll/hide/show/quit и
независимый хеш страницы для 1000/10000 строк. Она не измеряет ресурсы, idle,
presentation latency, физическую энергию или реальную геометрию окна.

## Readiness live run

Хост KDE Wayland: session 3, Active=yes, LockedHint=yes;
`org.freedesktop.ScreenSaver.GetActive` возвращает `(true,)`.
Live-команда проверена только на отказ: GUI не запускался, output не создавался.
Разблокировка, настройки питания, suspend и DE не изменялись.

**Единственное действие пользователя для будущего live run: разблокировать текущую
KDE-сессию.** Сейчас ответа и ожидания idle не требуется. После этого исполнитель
может выполнить из этого checkout:

```sh
XDG_SESSION_ID=3 bash bench/ui/run-comparison.sh \
  /home/myrs/projects/yonik/local/artifacts/ui-readiness-20261111 \
  /home/myrs/projects/yonik/local/validation/ui-live-20261111
```

18 последовательных прогонов (3 повтора × 1000/10000 × Qt/GTK4/Slint),
по 900 секунд visible и hidden idle; timeout 2100 секунд на прогон и 10 секунд
TERM→KILL. Первая ошибка останавливает серию. Существующие результаты не
перезаписываются. Автоблокировка может прервать серию: скрипт её не отключает.
Эта команда — сбор process-level UI данных, а не автоматическое закрытие gates.

## Счётчики и ограничения

`counters.json` сохраняет read-only probe без sudo. perf/pidstat/turbostat,
Xvfb/Weston отсутствуют. `perf_event_paranoid=2`; чтение sched_wakeup tracepoint
в tracefs/debugfs запрещено. Доступных powercap energy_uj и DRM hwmon energy
не найдено. `/proc/self/schedstat` читается, но содержит CPU runtime, runqueue
wait и число timeslices: это не actual wakeups. Context switches главного потока
драйвер уже маркирует proxy. **Actual wakeups и hardware energy остаются null**,
не нулём и не оценкой через CPU/PSS. PSS/USS и CPU ticks доступны через /proc
собственного кандидата; compositor/controller/tray/core в этот учёт не входят.
BAT0 energy_now/power_now читаются; одиночный snapshot сохранён отдельно как
`battery_instantaneous` и не интерпретируется как энергия сценария: он относится
ко всему устройству, baseline и временной ряд не снимались.

G-UI/G-POWER открыты: нет live window QA, physical power/wakeups, connected-idle
engine baseline, screenshot/HiDPI/accessibility/clipboard/tray и матрицы DE.
Победитель toolkit не выбран. Процессный startup не гарантирует cold cache.

## Результат проверки

- Release build всех трёх executable прошёл; `ldd` разрешил все библиотеки.
  Системный GCC 16 выдал SFINAE warnings в Qt headers; build exit=0.
- Qt software/offscreen: 1000 и 10000 строк, по 11 oracle-validated ack,
  search/select/favorite/sort/scroll/hide/show и Quit прошли.
- GTK4: оба размера завершились до первого ack с `No such backend: headless`.
  Slint/winit: оба размера завершились до первого ack с
  `Could not find wayland compositor`. Их offscreen correctness **не подтверждена**.
  Добавлять другой renderer, устанавливать display server или использовать
  locked desktop ради обхода отказа не стали. Общий smoke exit=1 ожидаемо
  отражает эти blockers, сырые stderr и reports сохранены в `offscreen/`.
- 17 тестов существующего Python-драйвера прошли. Ruff check/format новых
  Python-команд и драйвера прошли; shell `bash -n` прошёл.
- 8 release-тестов общей Rust-модели прошли на Rust 1.97.0.
- Bounded-команда на fake tools выдала ровно 18 уникальных вызовов с
  TERM/10s/2100s; на реальном ScreenSaver `(true,)` отказала до GUI и output.

Это готовые executable и воспроизводимые команды с явно сохранёнными gaps,
а не завершённый live comparison. Raw evidence хранится вне Git и вне TTL.


## Отмена offscreen runner

Исправлен P2: SIGTERM родительской проверки больше не оставляет candidate в отдельной сессии. Используется существующая deferred Cancellation: spawn и cleanup защищены от прерывания, после spawn отмена доставляется, finally закрывает child; обработчики восстанавливаются. Отмена возвращает exit130.

Регрессия запускает настоящий CLI с fake binaries, ждёт их PID и отправляет SIGTERM runner во время ожидания ACK. Baseline RED оставлял процесс живым; GREEN завершает его. Fake child игнорирует TERM, поэтому проверяется и KILL. Всего 18 protocol/lifecycle-тестов PASS; Ruff format/check PASS. Rust и реальные GUI повторно не запускались. Evidence `/home/myrs/projects/yonik/local/validation/pr99-cancel/`; manifest записан после проверок.
