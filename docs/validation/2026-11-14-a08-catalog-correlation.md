# A-08: catalog correlation RED → GREEN

Продолжение `03bdf96`, тот же worktree/task branch, без nested agents.
Product source: `a73e12ed21c5acd7828426cf22d17d3d85565681`.
**Все согласованные controlled A-08 controls прошли.** Physical Linux остаётся
DEFERRED/NOT PASS; полный G-APP/FR-20, N5 и native admission не подразумеваются.
Предыдущее finding/evidence сохраняется исторически, не переписывается в PASS.

## Минимальный product-контракт

Main владеет одним `catalogRequestSequence`. До инкремента проверяются safe
integer, nonnegative и граница Number.MAX_SAFE_INTEGER; при исчерпании нет
wrap/dispatch, текущий display очищен и показано объяснение перезапуска.
Другого session counter/epoch нет. Проверен actual путь application.cpp:
единственный engine.load(Main.qml), bridge — ребёнок Main и владелец Host.
Hide/show не пересоздаёт Main. Пути recreate Main с прежним живым Host не найдено.

Каждый routing Loader создаёт локальный `catalogExpectedId=0`. Query/mode change
обнуляет его, pagination/request выделяет новый глобальный ID. Rows, incomplete
warning и pagination работают только с result, у которого expected>0 и
reply.request_id===expected. Уничтожение старого Loader ничего не сбрасывает в
новом Loader и не меняет sequence Main.

RoutingCatalog получил optional request_id с serde default=0. ID захватывается
в blocking closure до producer и добавляется только к успешному positive-ID
result. Для legacy 0 поле **отсутствует**, старые четыре поля result сохранены.
Остальные inputs, limits, error mapping и producer не изменены. ID — только
correlation: control.generation, N5/native generations и observer token не
используются и не меняются. Закрытие не отправляет cancellation или дополнительный
roundtrip, не отменяет уже начатое IO и не запускает periodic scans/timers.

## Реальная причинная проверка

Original production qrc Main, bridge, backend и Catalog; независимый C++ observer
вызывает существующие clicked handlers enabled/visible controls и меняет обычные
input properties. Snapshot/model не подменяются. Физические mouse/keyboard и
физическая сессия не измерялись. Same host UID в private user/PID/mount/net
namespace, private Xvfb/software; system applications roots закрыты своими bind
mounts. Использованы прежние hash-bound Xvfb/helper, без installs/download/deps.
Filename tracer **не менялся и не расширялся**: только прежнее owned child tree,
никаких чужих UID, argv contents или credentials. Канарейка synthetic env отдельно
видит cmdline/environ; actual GUI trace этих чтений не содержит.

RED — сохранённый старый `local/validation/a08-runtime/a08_gui` и его original
Main SHA. Driver удерживает process result, закрывает Loader, открывает новый
Loader в desktop mode и только затем выпускает result. Проверяется настоящий
filtered reply и видимая строка helper. `red-bound/gui.log` заканчивается
`stale process reply displayed after reopen`, exit nonzero. Исходник именно этого
RED driver и его ELF сохранены в `red-source/`; он собран из явно сохранённого
test source и исполнен заново на неизменённом старом GUI ELF. Первоначальная
промежуточная RED library не присваивается другому source. Старый GUI ELF и весь
старый retained directory не изменены.

GREEN — новый единственный GUI ELF в `local/validation/a08-correlation/a08_gui`.
Финальная campaign4 (limited/complete/drop/hidden-scan):

- Actual process reply ID=3 после reopen действительно публикуется в backend
  snapshot. Новый Loader expected=0, rows=0; close/reopen даёт 0 новых admissions
  и 0 новых IO reads. Затем explicit desktop ID=4 принят и показан.
- Query и mode change сразу дают expected=0/rows=0, без автоматического scan.
  Новые явные requests показывают соответствующие результаты.
- 40 минимальных дополнительных own .desktop обеспечивают настоящую pagination:
  Next даёт новый ID=8, offset=32 и 10 видимых rows.
- A request ID=9 удержан. После выдачи B ID=10 выпускается A; при реальном reply=9
  и expected=10 rows остаются 0. Затем B принимается. Это actual старый reply
  после более нового **request**, а не mock out-of-order snapshot. Actor ждёт
  blocking producer последовательно, поэтому B-publication-before-A в текущем
  пути недостижима; новый механизм concurrent publication не добавлен.
- Legacy no-ID command возвращает ровно items/total/offset/incomplete и не
  расходует GUI counter; новый GUI не показывает этот uncorrelated result.
- Legacy и positive-ID oversized-query errors оба остаются `limit`, не входят
  в producer и оставляют прежние catalog data/shape, policy и revision. Между
  ними normal explicit GUI request очищает error; generation не используется
  как per-command counter или authority oracle.
- Boundary injection меняет только sequence на MAX_SAFE_INTEGER−1, пропуская
  нереальное число ручных действий. Настоящий Load отправляет последний safe ID,
  backend echo и GUI acceptance работают точно. Следующие два Load не создают
  producer requests, sequence остаётся MAX, rows=0 и limit label виден.
- Known Android unsupported mapping #8 и отдельные unknown-kind parser refusals
  исполняются прежним fixture; Edit сохраняет selector и example.com. После всех
  controls policy неизменна и revision=2. Oversized vs complete catalog показывает
  причинное incomplete объяснение. Own Exec marker не появился при наличии
  marker-control; выбран только own helper, PID не сохранён как условие.
- Все 13 producer tokens монотонны, имеют reads и ровно один release. Diagnostic
  token=13 обслуживает GUI request ID=9007199254740991: это разные пространства.
  Фактические RAII error/panic проверки fixture сохранены.
- Negative drop даёт deadline, injected real hidden command даёт
  `close/reopen no NEW admission`; оба nonzero. IO oracle sensitivity сохранена.

Компактные результаты: `docs/evidence/a08-correlation/{limited,complete}.json`.
Actual phase records для checker negative mutation tests сохранены отдельно;
они не выдаются за новые GUI executions. Raw evidence и source/ELF bindings —
в `local/validation/a08-correlation/` и `runtime-bindings.json`.

## Проверки, история и ограничения

Одна fresh controlled Cargo tree, Rust 1.85.1, offline/locked, wrappers пустые,
jobs1, build slot и BSD directory flock. Product ELF после первой успешной
сборки не пересобирался и product source не менялся; refinements затрагивали
только C++ driver/error-path oracle. Его промежуточные ошибки (неверное ожидание
generation advance и dangling QJsonValueRef) исправлены до final campaign;
неуспешные прогоны не входят в PASS evidence. Первая попытка Clippy выбрала
stable helper из harness; повтор с явным RUSTUP_TOOLCHAIN=1.85.1 прошёл.

Проверены normal --lib --no-default-features build и два Rust correlation tests
в normal и test-support режимах; GUI/test-support all-targets Clippy двух packages
-D warnings; три Python A08 tests с negative mutations; RU/EN translations,
Rustfmt, Ruff и diff-check. P11 freshness: шесть tests. Source updates backend
и Main добавлены только через reviewed_source_updates; source_blobs, старые
producer execution bindings и предыдущие review entries сохранены исторически.
Это scoped self-review; независимый root review ещё не выполнен (nested запрещён).

Старая runtime directory/ELF и source142/P01/P02/193 references не изменялись.
Helper, kernel_evidence, bridge, Cargo dependencies и прочие writer scopes не
затронуты. Новых Rust unsafe, tracing syscalls или probes нет.

ELF/source/raw logs сохранены вне TTL до cargo clean; точные SHA, check outcomes
и cleanup записаны в runtime-bindings. Новая единственная build tree очищена.
Оставшийся scope: physical acceptance и остальные G-APP/native требования;
подтверждённый A-08 stale correlation finding закрыт в controlled scope.
