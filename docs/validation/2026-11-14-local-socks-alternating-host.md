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

## Унаследованный P2: negative peer oracle

Обычным merge включён parent #169
`c2e9e27c9398d65470838c82e76d563512de4ebb`. Alternate driver напрямую
импортирует его `denied_probe`, без копии и без изменения production API.
Перед positive host он запускает отдельный новый denied host для каждого ядра
при SIGSTOP собственного pinned sing-box peer, с подтверждением WIFSTOPPED.
Direct TLS control обязан получить 204, после сброса fixture production Connect
не должен достичь HTTPS: только ready, Timeout и ненулевой exit.
SIGCONT находится в finally родительского oracle.

Адаптация ограничена Python driver: новая опция `--peer-gate-only` выполняет
оба denied Connect и затем два restored-positive цикла (sing-box → Xray) в одном
новом DesktopHost с existing JSON ACK, generation/profile/probe/ELF,
busy/refusal, Stop/pidfd/reap/rebind и closed проверками.
JSON ACK alternate example отличается от строкового positive ACK #169;
поэтому positive часть использует alternate `command`, а не родительский
driver. Negative oracle совместим без адаптации: до timeout оба example
публикуют только ready. Rust example не менялся.

Focused invocation:

```sh
python crates/yonik-desktop/tests/local_socks_alternating.py \
  /home/myrs/projects/yonik/local/evidence/local-socks-alternating-host/local_socks_alternating \
  /home/myrs/projects/yonik/local/engines \
  /home/myrs/projects/yonik/local/evidence/local-socks-alternating-peer-negative \
  --peer-gate-only
```

Прежние 100 циклов не повторялись. Их бинарник retained от
`527ae21a77b42f8b7c4708fb1248b4186471de25`; его SHA256 совпал с исходным
binary.sha256, Rust example побайтно идентичен. AST функций sample/oracle/
identity/children родителя также идентичен прежнему. Final Python harness
добавляет negative gate и короткий режим; сам полный final driver на 100 циклах
не запускался. Это две явно разделённые части evidence, а не новый full-stack
или финальный 100-cycle seal.

Focused evidence вне TTL:
`/home/myrs/projects/yonik/local/evidence/local-socks-alternating-peer-negative/`.
`source-binding.json` фиксирует parent, retained binary, неизменность Rust
example/resource functions и новые Python hashes. Исходный resource evidence
каталог не перезаписан. Новая Rust-сборка не нужна и не выполнялась.

Focused gate: PASS для обоих ядер. Каждый denied host: direct TLS 204,
peer_stopped=true, exit_code=1, Timeout примерно через 20 секунд,
https_reached=false. После восстановления peer positive sing-box и Xray
прошли в одном host, все Stop/reap/rebind проверки и closed ACK получены.
Короткий режим возвращает PEER_NEGATIVE_AND_RESTORED_ALTERNATING_POSITIVE,
не resource verdict по двум samples. Ruff check/format обоих Python файлов
и diff-check прошли. TemporaryDirectory/private D-Bus/peer/host закрыты;
transient build-каталог не создавался.

## P2b: окончательная cleanup граница

Промежуточный вариант `7571d92` заменён обычным merge #169
`88c398a1a3973f24c65eb7e1b369711daf3547a5`: imported denied_probe использует
собственную PID namespace и util-linux kill-child. Host с унаследованными pipes
и core с отдельной process group завершает kernel; cleanup не зависит от
нового pidfd/сканирования для kill. UID mapping сохраняет текущий UID.
Механизм и ограничения описаны в отчёте #169.

Три коротких cleanup unit — PASS, без SKIP: старый parent-only путь обнаружен
занятым listener; новый timeout и ранний выход host освобождают listener.
Ruff и diff-check PASS. Rust examples/100-cycle workload не менялись;
100 циклов и real restored-positive кампании снова не запускались.
Оба actual peer-negative/restored-positive controls именно нового helper
прошли в root #169, evidence в `local-socks-resource-peer-negative/pidns-cleanup`.
Им не приписывается новый alternating runtime: retained 100 и focused
alternating proofs остаются привязаны к прежним бинарникам и drivers.

Merge binding: `/home/myrs/projects/yonik/local/evidence/local-socks-alternating-peer-negative/pidns-merge/binding.json`.
