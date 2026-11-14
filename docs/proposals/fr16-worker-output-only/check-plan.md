# План проверок после отдельного разрешения root

Сейчас выполнено только чтение Git objects и статическая проверка артефакта.
**NOT COMPILED / NOT EXECUTED** относится и к этому patch, и к сквозному path.
Ни одна описанная ниже проверка не запускалась.

1. JSON/D-Bus negatives: extra/duplicate fields, wrong version, handle 31/33,
   не-object JSON; signatures s/ss/ah вместо sah; 0/1/3 descriptors; header
   declared count против array; повтор индекса и разные индексы одного inode.
   Проверить закрытие всех message/request copies по каждому отказу.
2. FD negatives на app/helper/worker: regular file, socket, eventfd, named FIFO,
   O_RDONLY/O_RDWR, NONBLOCK, отсутствие CLOEXEC, одинаковый inode; wrong role
   count. CLOEXEC при recv обязательно, не post-validation «исправление».
   Для двух одинаково валидных pipes перестановка неразличима по kernel metadata:
   stdout/stderr задаёт только фиксированный порядок, не выводить ложное
   утверждение о детектировании намеренного swap валидных writers.
3. V3: R=0/1/16; R=17/255; все truncation/trailing byte; mode Check;
   reserved nonzero; role count/id/order mutants; FD count 4+R±1;
   R0 ненулевой scope; R>0 нулевой resource scope; дубли manifest,
   engine mismatch, resource size/count/budget/hash/seals mismatch.
   Проверка binary/config из existing receive обязательна отдельно от header.
4. Claim/auth: чужой unique owner того же UID, другой UID, смена helper owner,
   чужой peer, истёкший/replayed handle, context/topology/stamp stale;
   отказ polkit. Ни один отказ не вызывает job/mutation/send.
5. Custody: отмена до send; partial send/unknown send outcome; late ACK,
   invocation/PID/cgroup mismatch; future drop во время blocking transfer.
   Ровно одна пересылка, не retry; sender copies закрыты даже если job ещё живёт;
   receiver error закрывает все SCM_RIGHTS. Failed cleanup сохраняет uncertainty.
6. Consumer: late G1 bytes после G2 остаются G1; fragments stdout/stderr не
   объединяются; oversize/flood/unknown version/unterminated EOF; bounded scratch,
   disk backpressure, stop task без ожидания disk/EOF; EOF не меняет state на stopped.
7. Compatibility: byte-identical V1/V2/YNKROLE1 golden и ABI260 из frozen #204;
   legacy API paths не изменились; V3 rejected legacy/check; отсутствие downgrade.
   ProductionPermit rejection по-прежнему закрывает передачу и Run.
8. Root review отдельного полного diff, затем только разрешённые unit/static
   проверки и fmt/clippy. Installed-runtime, user grants, Runtime16 и host
   acceptance оформляются отдельно. Не запускать probe/install/SystemDBus
   из этого proposal.
