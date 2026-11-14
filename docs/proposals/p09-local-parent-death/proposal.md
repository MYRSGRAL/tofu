# P09: неприменённый fixed parent-death adapter

Статус: **proposal only, approval required; не fixed PASS**. Никакой patch не
применён, Rust-код не компилировался, host/core не запускались в этой задаче.
Новые файлы находятся только в `docs/proposals/p09-local-parent-death/`.
[Frozen finding](../../validation/2026-11-14-p09-host-sigkill.md) и его runtime
harness на `59ece58809b749804f1e4fc41eb83bb8be55b6e1` остаются неизменными.

[adapter.patch](adapter.patch) рассчитан на exact base
`59ece58809b749804f1e4fc41eb83bb8be55b6e1`, production inputs которого совпадают с
accepted `8ea826a04a745c8916e14a4329588c75f66b683e`.
SHA256 patch:
`b7b9c2d0f7bb08db43864fb9f206ae75ba33df5e1662316cb921b7020b13c8a2`.
[Binding](binding.json) содержит before/after SHA256 каждого файла и exact base.
Не применять поверх shared W2/W3 изменений без новой source/review сверки.

## Выбор минимального исправления

| Вариант | Что добавляется | Гарантия и стоимость |
| --- | --- | --- |
| Отдельный persistent supervisor на safe Rust API | Ещё один executable/process, control/ownership protocol, EOF detection, lifecycle supervisor, FD custody sealed image, resource/queue bounds, upgrade/crash rules | Может пережить host и выполнить terminate/wait/ACK; сам требует watchdog/recovery при своей смерти. Дополнительные процессы, RSS, FD, IPC и failure modes; значительно шире finding |
| Fixed child pre_exec + rustix PDEATHSIG | Один unsafe installer, fixed SIGKILL, getppid до/после; нет нового процесса, PID registry, IPC или конфигурации | Kernel убивает непосредственный core при смерти породившего потока. Не выполняет application cleanup/ACK и не гарантирует cleanup произвольных grandchildren. Нужны отдельное unsafe approval и actual обоих cores |

Предлагается второй вариант как минимальное исправление именно наблюдавшегося
LocalSocks core/listener survival. Supervisor не нужен для обхода approval.
Он остаётся возможной будущей архитектурой, если потребуется surviving owner с
подтверждением cleanup при смерти самого host, чего один PDEATHSIG не даёт.

Safe rustix `set_parent_process_death_signal` вызывается в **текущем** процессе.
Вызвать его в host до spawn недостаточно: Linux сбрасывает настройку ребёнку при
fork, а host получил бы чужую lifecycle policy. В имеющихся std/Tokio Command
нет safe setter child PDEATHSIG. Регистрация child-only callback требует unsafe
`CommandExt::pre_exec`; устаревший before_exec не является допустимым обходом
этого требования. Safe `process_group`/`kill_on_drop` не исполняют cleanup после
SIGKILL host. Конкретного safe API без нового unsafe, сохраняющего текущий
sealed execution path, в просмотренных pinned версиях не найдено.

## Точная поверхность patch

1. В `yonik-nsfs/Cargo.toml` **предлагается**, но не внесено включение `process`
   у существующего `rustix = =1.1.6`, default-features=false, fs/std сохраняются.
   Нового crate/version нет. Сегодня workspace yonik-linux уже объединяет
   process feature, но standalone nsfs обязан декларировать его сам.
2. Новый Linux-only `yonik-nsfs::local_parent_death` экспортирует только safe
   `install_kill_on_parent_death(&mut std::process::Command)`. Ни signal, PID,
   flags, pointer, callback, generic syscall, ни выбор exec caller не передаёт.
   Единственный новый unsafe block — registration pre_exec в этой функции;
   `#[allow(unsafe_code)]` ограничен функцией, общий deny остаётся.
3. Одна строка вызова в private `BinarySnapshot::command`, через Tokio
   `as_std_mut()`. Этот общий **локальный host** command builder покрывает
   version/config-check/run. `.command()` callers на базе находятся только в
   `host/{process,tests}.rs`; helper registry может создавать BinarySnapshot,
   но не вызывает этот private builder. Worker/systemd/N5 пути patch не меняет.

`/proc/self/fd/<sealed memfd>` остаётся executable, env_clear/LANG/SSL allowlist,
process_group(0), kill_on_drop и FD lifetime не меняются. Не добавлены pathname
копия, shell/setpriv/controller, снятие CLOEXEC/seals, caps, UID/GID изменения.
Новый hook не читает/не закрывает FD. `pre_exec` может выключить posix_spawn
fast path и потребовать fork; стоимость spawning подлежит проверке после grant.

## Parent process и parent thread

Проверенный call path: `DesktopHost::start_inner` создаёт OS thread
`yonik-backend` → `Builder::new_current_thread().block_on(run)` →
`tokio::spawn(session(...))` → `CheckedConfig::validate` / `LocalSession::start`
→ `BinarySnapshot::command` → Tokio `Command::spawn` → синхронный
`self.std.spawn()`. Только snapshot load отправлен в spawn_blocking; fork/exec
выполняется на runtime thread, не на retiring blocking-pool worker. CLI также
использует current-thread runtime. Перед fork installer внутри захватывает
`getpid()` (TGID), не caller-supplied PID и не PID внешнего UI.

Linux PDEATHSIG относится к **создавшему ребёнка потоку**. После arming смерть
`yonik-backend` убьёт core даже если Qt/main thread процесса жив. Это желаемая
fail-closed потеря runtime-владельца, но не равнозначно только process death.
Нормальное ожидание/Stop не завершает backend thread; thread не должен быть
перенесён в short-lived pool. Будущая смена runtime/spawn path требует re-review.

`getppid()` возвращает TGID, не thread identity. Если один поток умер **до**
arming, а ребёнок переподчинён другому потоку той же thread group, двойная
проверка не различит это событие. Нельзя обещать закрытие произвольной thread-only
pre-arm гонки. В текущем штатном path породивший поток синхронно ждёт завершения
std spawn, не исполняет пользовательскую async отмену между fork и arming;
асинхронное pthread cancellation не является контрактом приложения. При
SIGKILL **всего host** умирают все его потоки: либо последующая смерть текущего
parent доставляет уже armed сигнал, либо reparent за пределы TGID выявляется
проверкой. Не заявлять, что проверяется TID или что SIGKILL == Rust Drop.

Основание: [Linux PR_SET_PDEATHSIG](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html)
и [kernel v6.12 exit.c](https://github.com/torvalds/linux/blob/v6.12/kernel/exit.c),
`find_new_reaper` / `forget_original_parent`: возможен reparent внутри thread
group; настройка сигнала обрабатывается при exit/reparent. Это статический разбор
Linux semantics, не новый actual trace установленного kernel.

## Safety proof и race/error поведение

Новая unsafe necessity — только регистрация AS-safe callback между fork и exec.
[std CommandExt contract](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.pre_exec)
требует отдельно доказать безопасность операций в child; safe Rust типизация
сама по себе async-signal-safety не доказывает.

На локальных pinned rustix 1.1.6 просмотрены `process/id.rs`,
`process/prctl.rs`, `prctl.rs`, `backend/linux_raw/{process,prctl}/syscalls.rs`,
`io/errno.rs`. getpid/getppid/prctl идут через raw Linux syscall wrappers;
Errno→io::Error использует from_raw_os_error. Closure хранит только Copy PID;
getppid сравнивается с Some(expected_parent); обе mismatch ветки возвращают
фиксированный ESRCH. Ошибка prctl сохраняет числовой errno, без string allocation.
Нет heap allocation/deallocation, formatting/logging, env, locks, async/await,
Tokio runtime calls, panic/unwrap/assert, unwind, mutable global state или FD
mutation внутри closure. Подготовка Command и регистрация closure допускают
allocation **в parent до fork**, не в child. При изменении rustix backend или
версии wrappers этот proof нужно перепроверить; libc backend автоматически
этим разбором не сертифицирован.

| Момент / результат | Предлагаемое поведение |
| --- | --- |
| Parent уже переподчинён до первой проверки | ESRCH → std spawn error, child не exec core |
| Parent погиб между первой проверкой и arming | Вторая проверка выявит смену TGID; если уже armed и parent ещё умирает — kernel SIGKILL |
| Parent погиб между arming и второй проверкой / после неё | Armed SIGKILL либо mismatch запрещает exec; нет окна unarmed successful exec |
| prctl вернул ошибку, включая EINTR | Spawn fail-closed с raw errno. Нет бесконечного retry или продолжения без hook |
| Core завершился самостоятельно | Обычный wait/try_wait и существующий error path, не новый cleanup ACK |
| Explicit Stop / уже exited / repeated Stop | Существующие terminate/wait/reap правила; hook не заменяет и не перехватывает их |
| Drop host при живом runtime | Существующий shutdown/kill_on_drop остаётся; отдельное подтверждение cleanup требует await |
| Host SIGKILL | Нет Drop/ACK; ожидается kernel exit непосредственного core, reap выполняет surviving ancestor/init |

SIGKILL нельзя catch/block/ignore. Pdeath настройка сохраняется при обычном exec,
но сбрасывается при privileged exec/смене ряда credentials и не наследуется
новым fork-child. Текущий sealed memfd без setuid/file caps и неизменяемый local
exec path нужны для гарантии; behaviour core, меняющего credentials, rearming
PDEATHSIG или daemonizing, не покрыт blanket proof. Это не общий descendant
fence и не замена worker/cgroup architecture.

## План проверки только после отдельного grant

1. Root сверяет актуальные shared refs и patch scope, получает user unsafe grant,
   затем отдельный исполнитель применяет reviewed delta; эту задачу не продолжать
   применением. Standalone nsfs process-feature check и MSRV1.85/all-targets
   fmt/clippy/tests обязательны; workspace feature unification не заменяет первый.
2. Узкий process fixture: parent жив → child canary реально исполняется; normal
   Stop/already-exited/repeated Stop; thread-owner exit при живом main thread;
   whole-host SIGKILL после exec. PID namespace supervisor переживает parent и
   проверяет identity/listener до namespace exit, затем kill/reap только своих.
3. Детерминированная race матрица до первой проверки, между первой и arming,
   между arming и второй, после второй: controlled test-only fixture, без sleeps
   как доказательства границы и без runtime injection production API. Для
   thread-only pre-arm reparent фиксировать ограничение, а не выдавать false PASS.
   Дополнительные hooks/shared edits требуют отдельного root ownership contract.
4. Ошибки/EINTR: static fail-closed branch + отдельная test-only pure error
   propagation fixture; не добавлять caller-controlled syscall/signal в production
   ради injection. Если actual EINTR недостижим безопасно, явно отметить это;
   не использовать broad ptrace/anti-engine monitor. Проверить std spawn error
   без exec canary и обязательный reap child. Убитый parent не может получить
   результат spawn — supervisor проверяет отсутствие живого executable core.
5. Actual normal DesktopHost на **обоих pinned cores**: positive Connected/Stop →
   exit/reap/rebind, live-child/occupied-port negative controls, SIGKILL host →
   pidfd-ready/identity/listener disappearance до namespace teardown; supervisor
   reap и final no children. Вариант unpatched retained ELF должен воспроизводить
   известный survives, если потребуется отдельный regression comparison; не
   повторять широкие 100-cycle/P03 кампании.
6. Seals/hash/version/config stdin/FD/CLOEXEC regression; worker/N5 не затронуты
   по callgraph/diff. Одна focused frozen build matrix, shared≤2 slots/jobs1/flock,
   empty wrappers, source/ELF bindings и retained artifacts перед cargo clean.
   Ни один результат не закрывает full native host-loss/Validate/Apply barriers.

## Предлагаемый текст запроса user grant (не запрашивается здесь)

> Разрешаете отдельное unsafe-исключение только для одного
> CommandExt::pre_exec installer в yonik-nsfs по proposal p09-local-parent-death,
> patch SHA256 b7b9c2d0f7bb08db43864fb9f206ae75ba33df5e1662316cb921b7020b13c8a2
> на базе 59ece58809b749804f1e4fc41eb83bb8be55b6e1: fixed SIGKILL через
> существующие safe rustix 1.1.6 wrappers, внутренний parent PID и getppid
> до/после arming, process feature в существующей nsfs dependency и единственный
> local sealed-command call site? После разрешения — отдельная реализация и
> перечисленные проверки обоих pinned cores. Новых syscalls с raw pointers,
> caller-selected signals/PIDs, caps, supervisor process, worker/N5 изменений
> или ослабления sealed/CLOEXEC пути scope не содержит.

Ранее согласованные nsfs/TUN/Runtime16 unsafe исключения этот installer **не
покрывают**. Root сначала review; только затем отдельное решение пользователя.
