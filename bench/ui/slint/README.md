# Slint benchmark

Самостоятельный участник workspace `bench/ui`, общий actor — `ui-bench-model`.
Запуск: `yonik-bench-slint --rows 1000` или `--rows 10000`.
Команды JSONL и поля отчёта описаны в `../PROTOCOL.md`.

Slint **1.18.1**, backend **winit**, renderer **femtovg / OpenGL** явно выбраны
через `BackendSelector`, включены Wayland и X11. Qt backend и Skia renderer
не включены. Стиль виджетов фиксирован в `build.rs`: Fluent, тёмная палитра.
Окно 800×640, Noto Sans 14 px, фон #191919, акцент строк #c3a6ef,
высота строки 66 px. Общая геометрия: margin 20 px, gap 12 px, header 38 px,
строки поиска и избранного по 40 px, viewport списка 320 px, footer 40 px. `ListView` виртуализирует страницу общего actor до 200 строк.

Поиск, сортировка, избранное, выбор и страницы отправляются actor; оболочка
только отображает snapshot. Единственный `yonik-bench-bridge` блокируется на
`recv`, доставляет события через `Weak::upgrade_in_event_loop`, без polling.
Snapshot и scroll обрабатываются последовательно по кадрам. Отчёт `frame`
выдаётся из публичного `RenderingState::AfterRendering`: отрисовка уже выполнена,
но swap/presentation ещё может не произойти. Это не измерение compositor latency.
`shown` также ждёт отрисовку; `hidden` выдаётся после hide. Event loop живёт до
явного Quit, включая время, когда окно скрыто. Quit немедленно отменяет очередь
и ожидающий кадр, включая системно свёрнутое окно; ожидания AfterRendering нет.

`created_total` считает вызовы `init` реальных делегатов `ListView`, включая
созданные заново после смены модели. Это не размер `VecModel` и не число серверов.
Публичного надёжного lifecycle/intersection API для live/visible делегатов здесь
не используется: `alive_rows` и `visible_rows` равны `null`. `model_rows` пишет
общий report по snapshot. Scroll использует публичный `ListView.content-y`
(актуальное имя Slint 1.18, `viewport-y` deprecated), с ограничением по высоте
контента и viewport; `scroll_index` сохраняет запрошенный индекс внутри страницы.

Проверка компиляции не запускает GUI. Проверка реальной виртуализации, кадров,
цветов и прокрутки требует отдельного запуска и screenshot QA.
