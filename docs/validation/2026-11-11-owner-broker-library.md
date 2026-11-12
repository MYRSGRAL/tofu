# Независимая owner-broker библиотека: промежуточный результат

База: `89011b849ddcae66d3d8ae249af93f8a7f424ebb` (выданный combined PR90 head).
Только `attribution/broker`, additive module export и собственные тесты.
Helper, worker, PrepareBundle, system_client, Go, manifest и native capability
не изменены. Общий private ABI не предлагается этим wire fixture.

Реализованы host-only trusted context, фиксация worker pidfd/starttime/UID и
net/pid/mount namespaces, revoke/epoch/nonce/digest fences, SEQPACKET receiver
с SCM_CREDENTIALS каждого сообщения (не SO_PEERCRED creator identity), строгая
фиксированная схема без PID/path authority и запретом входящих SCM_RIGHTS.
Broker требует фактический app UID != 0 и нулевые effective/permitted/inheritable/
ambient capabilities; dumpable=0 устанавливается до возврата готового объекта.
Root/service launcher и окружение остаются отдельной integration dependency.

Kernel backend использует SOCK_DIAG и полный ограниченный dump до NLMSG_DONE,
затем собственный proc snapshot UID: до 256 PID, до 256 FD/PID, 65536 diag
records, deadline 20 ms, один lookup синхронно. Переполнение, ENOBUFS, DUMP_INTR,
truncation, permission errors и невидимый namespace дают MechanismUnavailable.
Двойной diag witness связывает tuple/cookie/inode. UID holder определяется proc,
а не socket creator UID. Подтверждаются pidfd/starttime/namespaces и exe dev/ino
до/после capture; snapshot удерживает opened exe FD и pidfd. Никаких argv,
содержимого executable или secrets в records. UDP без remote tuple — UnknownFlow;
несколько socket witnesses — Unavailable, несколько holders — UnknownFlow.

Добавлен on-demand cache до 64 положительных snapshots (ключ: canonical flow,
затем cookie/starttime/epoch в сохранённом proof). Каждый hit повторно проверяет
live pidfd/starttime/namespaces, текущий exe dev/ino, socket FD/inode и SOCK_DIAG
cookie. Начало observation interval не омолаживается при hit; возраст ≤20 ms.
Повторный cold scan разрешён не чаще одного раза в 20 ms, иначе Unavailable Limit.
Lookup требует exclusive mutable borrow: параллельных lookup и внутренней очереди
нет. Background scans нет. Внутри cache до 128 FD; result передаёт два owned FD
вызывающему. Вызывающий отвечает за ограничение одновременно удерживаемых results.
Кэш — bounded index ранее проверенных snapshots, не полный proc/socket index.
Новый holder/exec после начала интервала не превращает прежний snapshot в
birth-time/current atomic guarantee. Отдельный native adapter обязан понимать
эту границу; admission без согласованной семантики запрещён.

Это **не завершённый production broker**. Нет production helper launcher, PID1 attestation, holder roster,
Prepare coverage lease и native fail-closed integration. Нужен отдельный review
семантики snapshot относительно exec/FD passing и полноты перечисления процессов.
Snapshot не доказывает identity socket creation и атомарность против arbitrary
exec. Нет заявления о поддержке AppImage/Flatpak, privileged/nondumpable apps,
positive FR20 или 1×/10× native performance acceptance. No-default/root admission не
меняется. Нельзя активировать capability на основании этой библиотеки.

Проверки и внешние raw logs фиксируются в evidence после выполнения; локальные
cargo test не заменяют isolated usernetns/VM acceptance чужого worker UID.

## Проверки и ограничения acceptance

`cargo +1.85.1 ... --locked`, `CARGO_BUILD_JOBS=1`, общий with-rust-slot,
CARGO_TARGET_DIR под собственным transient root lock. Финальные команды:
`clippy -p yonik-linux --all-targets -- -D warnings`,
`test -p yonik-linux --test broker_private -- --test-threads=1`,
`test -p yonik-linux --lib attribution::broker -- --test-threads=1`,
`fmt --all -- --check`.

Private sender — настоящий отдельный процесс, socketpair создан родителем.
Контроль принимает child credentials и отвергает parent с тем же UID.
Проверяются nonce/digest/epoch/reserved fields/truncation, revoke, timeout,
pidfd death; starttime/UID/namespace/exec identity и epoch моделируются без
реального PID churn. Kernel controls реально открывают TCP/UDP v4/v6 sockets.
Isolated fixture проверяет actual own-UID PID/exe/socket snapshot и cache hits,
ложный cookie, равенство FD count до/после. 1×/10× — число повторений, не число
одновременно активных приложений и не native routing performance acceptance.
CPU/wall/RSS измеряет runner всего fixture process tree, включая namespace setup;
искусственные 21 ms паузы соблюдают cold-scan budget. Это не latency benchmark.

Runtime имел ненулевой CapInh; исходный запуск честно отклонён. Перед fixture
CapInh/ambient очищаются. Для настройки loopback сохраняются capabilities только
в новом user namespace; до exec broker они сброшены. Host routes не меняются.

Не закрыты production launcher/registry/roster, реальный foreign worker UID и
nondumpable worker proc-view, actual exec-race controls, reuseport/coverage loss
fault injection, границы cap/memory на 10× одновременных processes и измерение
полного cancellation/teardown. Worker validation сейчас требует доступ его proc
namespace view; при отсутствии доступа возвращает Unavailable. Нельзя объявлять
этот случай поддержанным. Python runner проверен AST; Ruff в runtime отсутствует.

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/owner-broker-89011/`.
Binary hashes, source manifest, raw cargo logs и измерения сохраняются перед
штатной очисткой build tree. Draft PR сохраняет эти acceptance gaps открытыми.
