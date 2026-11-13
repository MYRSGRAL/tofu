# P-05: чередование ядер в одном DesktopHost

Родитель: PR #169, source commit
`37277c8dc34d53a9a1e4b0ae2dd6ec8000fd2d45`. Проверяется конкретный gap
его отчёта: смена ядер внутри одного долгоживущего host.
Это отдельный component witness; final full-stack seal не заявляется.

## Методика

Новые example `local_socks_alternating` и Python driver
`crates/yonik-desktop/tests/local_socks_alternating.py`; production API,
зависимости и родительский стенд не изменены. Один encrypted synthetic store
со случайным ключом, один source/profile, private D-Bus, отдельные HOME/XDG.
Пользовательские preferences, Plasma и системная сеть не изменяются.
TUN, cross-UID и pending probes v2 не вызываются; runtime PR #170 не включён.

Один DesktopHost выполняет 100 последовательных connect → known HTTPS →
попытка активной смены → Stop → reap/rebind → select other core циклов:
нечётные sing-box, чётные Xray, по 50. Последний Stop завершает кампанию,
поэтому между connect имеется 99 смен. Existing set_engine сохраняет
preferences; стенд проверяет файл после каждого отказа и успешного выбора.
Активная смена по existing contract возвращает busy; проверяются прежние
engine, selected_profile, owned PID/starttime. Hot switch не требуется.

Каждый connect проверяет строго возрастающий error_generation (1…100),
тот же selected_profile, Preparing с пустым probe_result и Connected с
новым probe_result. Все snapshots сохранены. Это наблюдение публичных ACK,
не инъекция искусственного stale callback: внутренняя гонка произвольного
старого completion этим workload не доказывается.

Production DesktopHost использует BinarySnapshot с собственными pinned hashes.
Driver до запуска peer проверяет оба ELF и в каждом цикле хеширует
`/proc/<owned-child>/exe`, сверяя с соответствующим pin. Доверенные пути и
хеши совпадают с таблицей в production backend session и родительской
методикой. Оба ELF также отдельно сверены с
`packaging/licenses/linux-composition.json`; результат и SHA256 metadata
сохранены в `metadata-binding.json`. Нечётный и чётный engine направлены через один pinned sing-box
VLESS loopback peer к TLS fixture; каждый цикл fixture фиксирует HTTPS.
Прямой запрос driver к HTTPS не используется как подтверждение connect.
Это локальный controlled traffic, не проверка внешнего провайдера.

Stop подтверждается pidfd readiness, отсутствием старого PID/starttime,
повторным bind того же SOCKS-порта и отсутствием owned children.
Host PID/starttime остаётся одним; ребёнок каждого цикла записан отдельно.
Только собственные Popen/Guard используются для cleanup.

## Ресурсы и пределы

Переиспользуются sample/oracle #169: baseline 0, stopped 1…100, connected
host/core 1…100; warmup 1…10. Raw RSS/PSS/private dirty/heap RSS, FD, threads,
min/max/end/delta/slope сохраняются. Heap RSS не включает все allocator arenas.
RED означает ребёнка после Stop или рост FD/threads относительно цикла 10.
Memory REVIEW: end delta >8 MiB одновременно со slope >32 KiB/cycle —
только triage, не спецификационный budget. Меньшие slope также публикуются.
Лёгкая канарейка удерживает собственные реальные FD и обязана дать RED.

После кампании читаются все доступные страницы event ring: не более 256
events/page и не более 2000 суммарно. Это bounded retained trace, evicted/missed
сохраняются; полный журнал за всё время не заявляется. Ограниченный рост кольца
и allocator retention нельзя автоматически объявлять отсутствием утечек.

## Воспроизведение

Focused example build и Clippy с `--features test-support -j1`.
`CARGO_TARGET_DIR` — собственный transient task-каталог, удерживаемый flock;
глобальный with-rust-slot ограничивает параллельные Rust-сборки двумя.
Python driver получает binary, trusted engines directory и evidence directory.
Rustfmt проверяется только для нового example, Ruff — для нового driver;
workspace-wide campaign не запускается.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/evidence/local-socks-alternating-host/`.
Там сохраняются бинарник, raw samples, states, logs, canary, SHA256 manifest и
build/check evidence. Source binding ограничен этим parent и хешами файлов
стенда/production components; изменения последующего main требуют оценки diff.
GUI, реальный desktop session, внешний network, full-stack, power и длительный
uptime вне этого witness.

## Результат

100/100 циклов, 50 на каждом ядре, 99 смен между connect, один host starttime.
Все busy/refusal, generation/profile/probe reset, ELF hash, HTTPS,
pidfd/reap/rebind проверки пройдены. Closed ACK получен; stopped samples
не содержат детей. Oracle: BOUNDED_COMPONENT_WITNESS; реальная FD canary: RED.

После прогрева FD=15 и threads=4 во всех samples. RSS delta +76 KiB,
PSS +76 KiB, private dirty +76 KiB, heap RSS +8 KiB; полные min/max/end/slopes
в alternating.json, все raw samples опубликованы отдельно. Наблюдаемый рост
памяти не интерпретируется как доказанное отсутствие утечек.
Event ring: 500 retained events, 3 страницы с последней пустой, evicted=0.
Переполнение capacity этим прогоном не достигнуто.

Focused build, Clippy -D warnings, Rustfmt и Ruff прошли. Полные логи,
source-binding.json и metadata-binding.json сохранены в evidence.
