# CI Harness и граница closed → SQLite cleanup

База отдельной task-ветки: main `f3c937d717a97345dee3bc5bb79127fe4c96ae1e`
с уже слитым PR95. Cancellation API/source и три scheduler-контракта не меняются.
Незакоммиченный Harness diff перенесён из прежней ветки с сохранением upstream
системного теста. Исходный diff и RED evidence сохранены вне transient build:
`/home/myrs/projects/yonik/local/validation/ci-harness/`.

## Доказанная причина и точные границы вывода

`open()` создаёт два handle одной БД: service и routing_repository.
Shutdown ожидал только service.close(), снимал directory lock и публиковал closed;
routing_repository оставался жив до выхода из backend run, включая callback.
Это доказанное нарушение порядка завершения ресурса, создающее окно early reopen.
Оно согласуется с сообщёнными root CI UnsafePath на early reopen, но не доказывает,
что все Harness deadlines и все UnsafePath в PR91/92/93/96 имеют одну причину.
Counter+PID считаются уникальными; collision не объявляется установленным.

Содержательный RED `closed_snapshot_has_released_last_database_handle` удерживает
callback closed. В этот момент уже опубликован closed=true, но WAL ещё существует.
На актуальном main: тест упал за 0,02 с, WAL inode=49041624, nlink=1,
mode=33152 (0100600), `red-main-close-boundary.log`.
Это не проверка «runner не успел за timeout» и не принудительный kill.

Согласованная shared boundary: сначала независимо await service.close() и
routing_repository.close(), затем вычисление closed и снятие directory lock.
Ошибка любого close входит в closed=false/cleanup_failed. Проверки UnsafePath,
retry и deadline не меняются. Другие production backend paths не затронуты.

## Безопасная диагностика Harness

При recv timeout/disconnected Harness сообщает caller file/line/column, elapsed,
PID/instance ID, количество полученных frames, последнюю команду (только имя из
allowlist) и последнюю безопасную проекцию snapshot. Unknown enum/code редактируется.
Body, URL, headers/HWID, имена источников/профилей, search и refresh token не выводятся.
Closed ACK также фиксируется безопасной проекцией: при следующем прямом repository
open failure запись останется в captured output того же теста.
Privacy controls используют CANARY_PRIVATE и проверяют его отсутствие в диагностике.
10-секундный deadline Harness сохранён.

## Bounded verification/provenance

Повторение запускает последовательно backend и duplicate batch_diagnostics,
по пять полных запусков каждого, test-threads=4. Budget каждого target 30 с,
общий budget 120 с. На первом сбое повторение останавливается, process group
ограниченно завершается/reap, а log и код выхода сохраняются. Внутренний Harness
timeout не изменяется. До запусков записываются source/binary/runner SHA256,
база и argv/budgets (`repeat-before.json`); результаты — `repeat-results.json`.
Runner и логи находятся во внешнем evidence-каталоге, generated files в git не входят.

## Результат

Rust 1.85.1: GREEN того же probe — 0,03 с, WAL к closed callback отсутствует.
Scoped integration: backend 37 passed/1 ignored; batch_diagnostics
39 passed/1 ignored. Bounded repeats: все 10 запусков за 8,77 с,
суммарно 380 passed, 0 failed, 10 ignored. Desktop library tests: 29/29.
`clippy -p yonik-desktop --all-targets -- -D warnings` и fmt check прошли.
Default features; GUI/native engine/host install не запускались.
Внутренний Harness deadline остаётся 10 с. Raw last DTO/command не печатаются.

Доказано устранение outstanding DB handle после closed на указанной границе.
UnsafePath policy и cancellation/scheduler sources не изменялись. Наличие других
причин CI deadlines/UnsafePath этим bounded-срезом не исключено.
