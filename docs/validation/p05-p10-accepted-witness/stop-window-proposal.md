# Bounded legacy witness: обнаруженный Stop/window contract gap

Exact inspected head: `dbbd66814daede7659bd1b3b8cfbc735870eacab`.
Новый trace/harness не реализован, builds/runtime не запускались. Это static
source finding и минимальный proposal; не measured failure и не новый PASS.

## Достижимость и различие producers

`DesktopHost::dispatch(Command::Stop)` вызывает `control.interrupt(false)`;
поэтому actual Stop идёт в priority branch `run`, не в lease-stale replacement
ветку. Priority branch сначала отзывает proxy_session, сигнализирует cancellation,
затем await `scheduled.shutdown(origin.elapsed())`. После этого, если !quitting,
создаётся **DesktopSchedule::new()**, копируются только policy и visible,
выполняется load persisted schedule. Старые window_until/next_window не копируются.

`DesktopSchedule::with_limits` задаёт window_until=None, next_window=ZERO.
Следующая run iteration восстанавливает interval/background из preferences,
но не прежние window_until/next_window. `load` восстанавливает source due, а не
общий controller window barrier. Поэтому persisted reservation одного held source
не доказывает сохранение общего окна для остальных due sources.

Иная ветка `schedule_host.stale()` вызывает `scheduled.replacement()`. Уже
существующий replacement копирует interval/background/window_until/next_window,
затем load восстанавливает due. Именно эту ветку покрывает исторический
unit/direct-call window-replacement regression, а не priority Stop.

Контрпример по правилам кода, без исполнения: до Stop открыто окно [a,a+60] и
next_window=a+interval; в другом source есть overdue работа. После Stop new
controller имеет None/ZERO; такой source может открыть окно [b,b+60], b>a,
хотя старый общий barrier запрещал новое окно до a+interval. Это не утверждение,
что конкретная runtime попытка уже наблюдала расширение; actual сценарий ещё
нужен. Наблюдатель не должен заполнять потерянные значения из своей памяти и
выдавать их за actual scheduler state.

## Минимальное решение root до implementation

Предлагается разрешить ровно одну production замену в priority Stop branch:
после matching shutdown использовать `scheduled.replacement()` вместо ручного
`DesktopSchedule::new()` + policy/visible copy. Сохранить текущие signal-before-
await, shutdown/load error publication, scheduler_generation и command admission.
Новый scheduler owner по-прежнему создаётся existing producer; никаких lease,
ACK/result, NativeRunning/permit injection. Shutdown(!quitting=false) ветка не
меняется. Это **изменение Stop window policy**, не часть одобренного read-only
trace; поэтому сейчас не выполнено.

Если семантика Stop намеренно закрывает окно, альтернативный contract должен
явно определить сохранение next_window floor при закрытии и expected actual
trace; простое None/ZERO нельзя назвать сохранённым barrier. Такой вариант
потребует отдельного минимального production API/метода, а не observer setter.
Рекомендуемый первый вариант переиспользует уже существующий replacement без
нового scheduler API.

После решения: реализовать разрешённый bounded trace actual transitions с
observable overflow, сначала focused RED на текущем Stop (loss None/ZERO), затем
GREEN после разрешённой production правки и actual normal Host/private SS/default
DesktopFetch/pinned LocalSocks witness. Положительный second-due-source control
не позволит принять «нет запросов из-за persisted due» за закрытое окно. Cancel
socket EOF, matching successful shutdown phase/owner ID, reconnect нового actual
lease, отсутствие stale publication и reopen due остаются обязательными.
Новый trace не меняет политику и не синтезирует исходные timestamps.

Scope остаётся legacy LocalSocks, без N5/NativeRunning. Никакая authority/hook,
FR16 eventlog/worker/credentials не менялись; current frozen execution bindings и
foreign untracked сохранены. Root должен одобрить production policy delta либо
уточнить Stop contract; runtime запуск до этого не нужен.
