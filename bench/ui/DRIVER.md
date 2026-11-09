# Драйвер одного кандидата

Python 3.12+, Linux `/proc`, активная разблокированная Wayland-сессия с доступным
`loginctl`. Драйвер запускает **один заранее собранный native binary** и передаёт
ему `--rows 1000|10000`. Параллельные кандидаты и сервер не используются.

```sh
python tools/ui_benchmark.py \
  --binary /absolute/path/to/tofu-bench-gtk \
  --rows 1000 \
  --output /absolute/path/to/results/gtk-1000-run1 \
  --idle-seconds 900 \
  --timeout 30
```

Каждому запуску нужен новый каталог результатов. Кандидатов и размеры dataset
запускать последовательно в одинаковых условиях. `--timeout` задаёт предел
ожидания отдельной записи/ответа/завершения Quit; длительность двух idle-фаз
задаётся отдельно, по умолчанию **900 секунд каждая**. Никакого изменения DE,
профиля питания, настройки lock или пользовательских данных драйвер не делает.
Перед измерением следует самостоятельно обеспечить стабильные условия.

Последовательность: первый `frame`; Query страницы 0 (200 строк); точный host
`server-0.example.invalid`; Select 1; Favorite 1 true; Query favorites_only;
Query очистки поиска/фильтра; Query sort=name; Scroll 199; visible idle; Hide
с `hidden`; hidden idle; Show с `shown`; Quit с ожиданием успешного exit.
Каждая следующая команда отправляется только после проверки предыдущего ack.
Quit отменяет pending работу shell и не обещает завершающий frame.

Oracle реализован на Python независимо от Rust-модели: формирует синтетические
строки, фильтрует, сортирует и считает FNV-1a64. Для каждого ответа сверяются
revision, event, total, offset, model_rows, page_hash, first_id, last_id,
selected_id и scroll_index. Проверяются типы/наличие счётчиков, неизменность
идентификатора toolkit. Scroll/Hide/Show сохраняют ревизию модели. Нет таймера,
который выдаёт успешный результат без ack. `frame/shown` остаются заявлениями
shell о реальном callback layout/render: драйвер не доказывает видимость пикселей
и не заменяет проверку реализации toolkit и screenshot QA.

В каталоге результата:

- `evidence.json`: `valid`, причина ошибки, условия, spawn → first frame,
  latency каждого действия от записи stdin до получения и разбора ack,
  исходные reports, метрики процесса после действий и на границах idle-интервалов.
- `stderr.log`: первые 1 MiB stderr; далее поток читается и отбрасывается,
  evidence показывает фактическое число прочитанных байтов и truncation.

JSONL stdout ограничен 8192 байтами на строку, суммарно 1 MiB; входная команда
ограничена 4096 байтами. Слишком длинный/невалидный stdout, error event,
незапрошенный ack, неверное состояние, timeout или ранний exit делают запуск
невалидным. При ошибке сохраняется частичный evidence. Дочерний процесс создаёт
собственную session/process group; cleanup посылает TERM, затем KILL этой группе
и ожидает дочерний процесс. SIGTERM/SIGINT драйвера также вызывают cleanup;
в окне создания/cleanup отмена откладывается до безопасной границы. Прежние
signal handlers восстанавливаются. Чужие PID не обходятся и не завершаются.

Метрики читаются только из `/proc/<собственный-child-pid>`: `smaps_rollup` даёт
PSS и USS (Private_Clean + Private_Dirty + Private_Hugetlb); `stat` даёт сумму
utime + stime всех потоков процесса в ticks, evidence содержит `SC_CLK_TCK`;
`status` даёт voluntary и nonvoluntary context switches **главного потока**
(`main_thread_context_switches_proxy`), а не сумму по всем потокам.
**Context switches — proxy, НЕ wakeups**.
Idle CPU оценивается разностью ticks на границах фазы; это не аппаратная энергия.
`thread_names` берутся из task/*/comm, Linux может обрезать имена до 15 байт.
GPU и hardware power записываются `null`, отсутствие данных не подменяется нулём.
Учитывается только процесс кандидата, не compositor, Xwayland или весь компьютер.

Idle не отправляет GUI-команд и не опрашивает GUI: selectors ждёт stdout/stderr
до следующей выборки с интервалом максимум 60 секунд. Закрытие процесса
обнаруживается через EOF/проверку процесса; непрошенный report завершает запуск
с ошибкой. `loginctl` проверяется до старта и на границах idle-интервалов;
visible idle требует Type=wayland, Active=yes, LockedHint=no, иначе abort/invalid.
Краткую блокировку между проверками можно пропустить; hidden idle сохраняет
состояние сессии без требования видимости. В conditions записываются ОС,
выбранные переменные renderer/session, доступный ACPI platform_profile и статусы
power_supply. Эти поля описывают условия, но не гарантируют их неизменность.

Код возврата 0 означает валидный проход протокола, 1 — ошибку измерения.
Для модульной проверки, без окон:

```sh
python -m unittest discover -s tools/tests -p test_ui_benchmark.py -v
ruff check tools/ui_benchmark.py tools/tests/test_ui_benchmark.py
ruff format --check tools/ui_benchmark.py tools/tests/test_ui_benchmark.py
```

Тесты используют короткоживущие Python fake children и временные каталоги;
они не запускают native GUI, сеть, proxy core или данные профилей.
