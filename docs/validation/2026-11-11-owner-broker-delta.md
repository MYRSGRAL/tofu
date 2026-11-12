# PR112: delta после независимого review 1d4776b

Frozen candidate: `1d4776bf898b7b17651ff8ebdb2acc2910cb7c5c`.
Исходная combined база: `89011b849ddcae66d3d8ae249af93f8a7f424ebb`.
Frozen commit не изменяется, исправления отдельным commit. Go, native hooks,
helper/worker, PrepareBundle, system_client, launcher и policy admission не меняются.

## P1/P2: actual RED → GREEN

Для RED оба исходных broker файла извлечены через `git show 1d4776b:<path>`
в собственный transient root. Тот же Rust regression driver скомпилирован с
ними, pinned Rust 1.85.1 и уже locked rustix/serde_json artifacts; checkout и
новые dependencies не создавались. Runtime — отдельные user/net/pid/mount
namespaces, обычный UID, пустые capabilities, только loopback нового netns.

P1: Python thread выполняет настоящий `unshare(CLONE_FILES)`, открывает connected
UDP socket в своей отдельной FD table, передаёт только canonical port в fixture
control pipe. Frozen возвращает UnknownFlow. Delta перечисляет
`/proc/<tgid>/task/<tid>/fd` и подтверждает task starttime/UID/namespaces до/после,
а также TGID pidfd/starttime и opened exe identity. В GREEN найден actual holder
PID. Foreign leader не скрывает task с target UID: такой непокрытый authority
даёт Unavailable. Исчезновение task/permission/лимит не превращаются в полный scan.

P2: один connected UDP SO_REUSEPORT socket даёт Known, затем создаётся второй с
тем же tuple. Frozen tuple-only cache подставляет прежний cookie и даёт Known.
Delta сохраняет исходный cookie-or-unknown на hit, повторяет SOCK_DIAG uniqueness
check и возвращает MechanismUnavailable(AmbiguousSocket). Cold lookup даёт тот
же класс. observed_until обновляется после повторной проверки uniqueness;
observed_from и максимальный возраст 20 ms не омолаживаются.

## Остальные Rust gates

- SOCK_DIAG parser выделен из transport без alternate provider для Known. Pending
  match не выдаётся до NLMSG_DONE. Truncation, DUMP_INTR, ошибочный DONE, malformed
  tail, NLMSG_ERROR и OVERRUN отвергаются. ENOBUFS syscall/error mapping и kernel
  error record проверены детерминированными fault controls: ReceiveOverflow.
  Это не утверждение о принудительно вызванном kernel receive overflow.
- Полный пустой dump → UnknownFlow; несколько tuple socket witnesses →
  MechanismUnavailable(AmbiguousSocket); proc permission loss → Coverage;
  превышение FD/task/process/diag budget → Limit. Частичный match не Known.
- Actual nondumpable holder подтверждает Coverage вместо Unknown/Known.
  Actual FD cap (260 открытых descriptors) подтверждает Limit.
- Actual Python→sleep exec сохраняет PID/starttime и унаследованный socket FD.
  Старый cache executable не возвращается; opened exe FD сохраняет прежнюю
  dev/ino identity. PID reuse/starttime replacement моделируются без PID churn;
  настоящий child pidfd death и epoch/nonce/digest controls сохранены из PR112.
- Cancellation handle доступен другому потоку. Actual capture останавливается
  после обнаружения holder через test-only synchronization hook; внешний поток
  ставит cancellation. Результат Cancelled, partial proof/cache FD освобождены.
  Hook не подменяет kernel evidence и не компилируется в production.
- EOF → Disconnected, epoch revoke, cache clear. Revoke/Drop latch cancellation.
  Повторные malformed SCM_RIGHTS/truncation (256 итераций) не оставляют FD.
  FD counts до/после cache/cancel/EOF/Drop равны. Внутренней очереди задач нет.

## Actual foreign UID: receiver и admission различаются

Один root-marked fixture использует собственный subordinate-ID user namespace,
не host root и не VM FDwriter. Только в нём parent с фиксированным UID1001 child
захватывает trusted Process, принимает реальные per-message SCM_CREDENTIALS и
проверяет PID/UID. Отдельный child сохраняет PID/starttime, но после host capture
меняет UID0→1001: kernel message получает отказ Authority. Root здесь только
fixture для credentials controls; root Broker не запускается, Known не фабрикуется.

Normal app UID1000 не может читать required proc namespace view worker UID1001.
Это actual typed negative WorkerViewUnavailable из trusted constructor, а не
пустой executable path, UnknownFlow или fallback. Exact blocker — ptrace access
check при чтении `/proc/<worker>/ns/*` между этими UID. CAP_SYS_PTRACE не добавлен.
Поэтому **positive foreign-worker admission остаётся неподдержанным** в текущем
app broker. Продукционный trusted launcher/context custody — отдельная dependency;
нет сериализованной authority из RPC, environment или caller PID/UID/path payload.
Параметры UID/PID fixture задаёт только фиксированный root-marked test parent.

## Объявленные границы ресурсов

До 256 TGID и 256 task **суммарно** за scan, до 256 FD/task, до 65536 diag records;
deadline 20 ms, cold scan не чаще раза за 20 ms. 64 cache entries, до 128 cache FD;
результат передаёт два owned FD caller, его outstanding results ограничивает caller.
SEQPACKET SO_RCVBUF/SO_SNDBUF должны быть ≤256 KiB каждый, иначе Limit.
SOCK_DIAG socket запрашивает 32 KiB receive/4 KiB send buffer (Linux удваивает
учитываемые размеры). Только on-demand работа, нет idle scans или daemon threads.

Гарантия остаётся snapshot интервала lookup, не birth-time identity или атомарность
при произвольном exec/FD passing. Новые Go/native capabilities, FR20 positive,
AppImage/Flatpak и all-process admission не заявляются. ENOBUFS fault injection,
моделируемый PID reuse и actual process fixtures в evidence различаются явно.

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/owner-broker-delta-1d4776b/`.
Raw RED/GREEN, root-marked UID log, compiler output, exact source/binary hashes
и команды сохраняются перед штатной очисткой Cargo build tree.
