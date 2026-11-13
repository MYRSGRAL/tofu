# P07: конкурентная нагрузка DesktopHost (компонентная приёмка)

База origin/main `d74e61884385cd20f659112182142b05cacb958e`.
Production-код и shared contracts не менялись. Это **частичная компонентная
приёмка**, полный P07 не закрыт.

## Нагрузка и границы

`p07_concurrent.rs` использует настоящий DesktopHost с ephemeral ключом и отдельной
синтетической БД. Контролируемый LocalSocks peer удерживает greeting health-запроса
в состоянии Checking. Одновременно запускается ручная TCP-проверка выбранного
сервера (`tcp_busy=true` при Checking), producer настоящего EventLog host и попытка
FR07 import_text. Последняя нормативно возвращает busy: контракт backend.rs
запрещает импорт при can_stop. Контракт не ослаблен ради нагрузки.

Producer вызывает существующий test_fill: 100 пачек по 2 005 типизированных
событий, интервал сна 10 мс между пачками, всего 200 500 попыток. Номинальная
интенсивность 200 500 событий/с; фактическая длительность измеряется, сон и
время записи входят в неё. Это синтетические события, не native stdout/stderr.
Перед Stop проверяется ненулевой eviction: нагрузка журнала действительно
достигла host до отмены. Producer продолжает работу во время Stop и завершает
все пачки до shutdown.

После 100 мс нагрузки отправляется Stop; измеряется время до snapshot
Disconnected. Это не request_id ACK и не Qt latency. EOF на удерживаемом peer
подтверждает закрытие сокета; shutdown/wait_closed и внешний oracle проверяют
terminal cleanup. Страница журнала ограничена 256 событиями, eviction кольца
наблюдается после нагрузки. Верхняя граница command queue, долгосрочная
стабилизация кэшей и отдельная CancelBatch здесь не доказаны.

По три коротких прогона на Xray 26.10.31 и sing-box 1.14.4. Ядра проверяет штатный
pinned loader host; hashes сохранены в raw manifest. Python harness создаёт private
user/net/PID/proc namespace и private D-Bus, поднимает только lo. Нет TUN, host
profiles, чужих UID или внешней сети. 16 fixed probes, прежние 2×100/100 switch
циклы не запускаются.

## Доказательства

Машиночитаемые результаты: `p07-concurrent-evidence.json`; привязка
source archive / executable / harness: `p07-concurrent-binding.json`.
Полные snapshots и выборки FD/threads/PSS/private dirty с PID/starttime сохранены
в `local/validation/p07-concurrent/run-final/` указанного worktree, вне TTL.
Выборка ресурсов каждые 20 мс плюс стоимость чтения /proc: короткоживущий процесс
может не попасть в sampling; пустая terminal group проверяется независимо.

Oracle snapshot/remaining совпадает с `88c398a:tools/native_cancel_matrix.py`.
Negative control удерживает собственный процесс с >=8 MiB private dirty и FD;
после terminate/wait остатки должны исчезнуть. Это не повторное доказательство
100 resource cycles и не перенос результата прежнего бинарника на новый.

## Оставшийся P07 gap

Нет prodMain.qml/actualDesktopHost GUI witness: отзывчивость кнопок, скрытый UI,
сигналы/кадры и отсутствие обработки каждой строки не измерялись. Headless
DesktopHost не заменяет GUI. Ручная TCP-проверка проходит loopback без задержки;
долгая временная batch probe-runtime отмена отдельно не доказана.

Health-проверка намеренно удержана до SOCKS negotiation, поэтому этот workload
не даёт нового strict HTTPS positive/negative oracle. Прежняя HTTPS-приёмка
не присваивается этому executable. Для полного P07 нужен отдельный GUI workload
с strict HTTPS peer, наблюдаемой CancelBatch и скрытием окна.

Абсолютный продуктовый budget не вводится. Это короткий функциональный стенд,
не энергетическая или физическая GUI-приёмка; температура и питание не контролировались.

## Воспроизведение

Rust 1.85.1, jobs=1, with-rust-slot и CARGO_TARGET_DIR в собственном transient root
под flock. Собрать `cargo build -p yonik-desktop --features test-support --example
p07_concurrent -j 1`, затем `python3 tools/p07_concurrent.py ABS_BINARY ABS_CORES
ABS_NEW_OUTPUT`. Свой каталог output должен отсутствовать. Сохранить бинарник,
исходники и логи вне transient до штатного cargo clean.

## Результат финального кандидата

6/6 компонентных прогонов PASS; terminal group пуста после каждого, negative oracle PASS.

| Ядро | Stop snapshot, мс (3 выборки) | Import busy, мс | Длительность нагрузки, мс |
| --- | --- | --- | --- |
| xray | 8.329 / 11.158 / 13.907 | 0.532 / 0.991 / 0.505 | 1063.425 / 1085.933 / 1085.807 |
| sing-box | 17.025 / 12.521 / 7.339 | 0.767 / 0.872 / 0.833 | 1049.517 / 1047.967 / 1034.023 |

Диапазоны и исходные выборки сохранены без округления в JSON. Все страницы: 256 событий; eviction до Stop ненулевой.

Проверки: Rust 1.85.1 build, clippy `-p yonik-desktop --features test-support
--all-targets -j 1 -- -D warnings`, rustfmt целевого example, Ruff 0.17.4 check
и format --check целевого Python — PASS. Начальная сборка harness отклонила
Option через `?`; исправлено явным ok_or. Первый запуск clippy обнаружил неверный
runtime library path обёртки; финальный запуск использовал явный toolchain/lib.
Оба первичных лога сохранены, production-код не менялся.

Source archive, executable и build/check логи сохранены вне TTL в
`local/validation/p07-concurrent/`. После проверки отсутствия /proc exe/cwd/fd
ссылок на transient выполнен cargo clean; собственный transient root удалён.
Cargo.lock вернулся к accepted байтам: Cargo только переместил workspace записи,
зависимости не менялись. Чужие untracked cross-UID файлы сохранены и не включены.

## Уточнение source binding после review

`p07-concurrent-binding.json` schema 2 явно разделяет archived campaign source и
проверенный файл Git `b01a3a1`. Runtime executable относится к source member
в сохранённом archive (SHA `844b864…`), а текущий файл имеет SHA `e942b655…`.
Изменения ограничены cfg-гейтами и unsupported main; workload не менялся.
Это не exact-current-head build proof. Хеши source archive, executable и raw
логов сохранены; результаты старого binary не присвоены новому source.
