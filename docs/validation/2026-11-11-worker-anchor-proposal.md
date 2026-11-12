# Root proposal: foreign worker namespace anchor (после frozen 62aaef9)

Статус: shared helper/worker/private ABI edits требуют решения Root и владельца
FDwriter109. Этот delta не меняет их, units или admission. Frozen
`62aaef9c0f629c5576821b1bf103344c42dfb7db` остаётся отдельной review base.

## Наблюдённый baseline и proof gaps

`packaging/systemd/yonik-worker.service` задаёт RestrictNamespaces=yes,
NoNewPrivileges=yes, NET_ADMIN-only bounding/ambient caps, ProtectControlGroups,
PrivateTmp=yes. Но `helper/systemd_worker.rs::validate_service` НЕ проверяет
RestrictNamespaces actual property. На host LoadState=not-found; defaults
NoNewPrivileges=no/RestrictNamespaces=no не являются properties действующего
контролируемого unit. Нет actual closed-unit/namespace-filter attestation.

Run `worker.rs::serve_stream` подтверждает worker PID, затем делает
`configured(...).exec()`: native core заменяет image ТОГО ЖЕ PID, pidfd/starttime
сохраняются. Check/version действительно создают children. Нельзя выдавать child
PID на основании имени unit/MainPID; будущий spawn требует отдельного child pidfd,
credentials, starttime и cgroup proof до его exec. Текущий worker не является
supervisor отдельного Run core child.

PrivateTmp=yes создаёт отдельный mount scope worker. Нельзя механически подставить
worker net/pid/mnt triple в существующий kernel lookup: app broker/owner mount view
не обязаны совпадать с worker. WorkerChannelAnchor и BrokerLookupView должны быть
разными authority objects. Anchor не меняет holder/FD scan/exe checks.

## Конкретное минимальное предложение API/protocol для согласования

1. Helper получает opaque VerifiedWorkerUnit из существующего closed unit validator,
   расширенного FDwriter109 после approval. Read-only canonical system bus + fixed
   PID1 owner; проверяются unit identity/ExecStart/User/Group/InvocationID/ControlGroup,
   MainPID/starttime/pidfd, фактические RestrictNamespaces (полный запрет),
   SystemCallArchitectures/native + поддерживаемая filter architecture, NNP,
   NET_ADMIN-only caps и отсутствие namespace-capable escape paths. Наличие строки
   в unit file и сообщение worker не заменяют actual attestation. Missing/wrong
   type/stale/invocation change/property drift -> UnitProofGap/revoke.
2. До Run exec helper сам открывает nsfs FDs из fixed verified worker PID, если
   доступ разрешён. Иначе fixed worker по отдельному private SEQPACKET exchange
   передаёт ровно net/pid/mnt FDs, SCM_CREDENTIALS каждого сообщения. Helper сверяет
   PID/UID с opaque WorkerTicket + живым pidfd/starttime, FDs с trusted proc capture
   и nsfs magic/dev/ino, namespace slot/type; никакого PID/path в caller request.
   Для helper UID0 с NET_ADMIN-only proc capture тоже может быть запрещён. Тогда
   допустимый предлагаемый источник — только root-verified protected worker opener
   ДО native exec, который открывает фиксированные /proc/self/ns/net,pid,mnt;
   NS_GET_NSTYPE каждого FD подтверждает slot/type. Нужны nondumpable до authority
   exchange, root-owned exact worker image/ExecStart и закрытая unit invocation,
   исключающая code injection/namespace change. Одних credentials и nsfs magic
   недостаточно. Если trusted opener phase/provenance не доказаны — reject.
   Этот вариант требует согласования FDwriter109; текущий test-only validator
   реализует только независимое host capture сравнение там, где оно доступно.
3. Предлагаемый (НЕ implemented shared ABI) NS_WITNESS v1: ≤256 bytes,
   helper-issued nonce32/config_digest32, generation u64, invocation16, reserved=0;
   ровно 3 SCM_RIGHTS, CMSG_CLOEXEC, one outstanding, deadline 1s. PID/UID/starttime/
   cgroup берутся из host objects, не из frame. Root-approved helper передаёт broker
   отдельный anchored context/FD custody через trusted inherited private channel.
   Caller flow RPC остаётся canonical tuple/protocol/cookie/epoch/nonce/digest.
4. WorkerChannelAnchor::promote(pending, VerifiedWorkerUnit, SessionLease) ->
   AnchoredWorkerContext. Ни один параметр не Deserialize/Clone/Debug; нет public
   конструктора из trusted=true, PID/UID/namespace inode numbers. Lease связывает
   nonce/digest/generation/invocation и cgroup identity, имеет expiry и revoke.
   Promotion требует live worker pidfd/starttime/UID/cgroup и повторный unit proof.
   Anchor даёт retained namespace identity, не право setns и не owner snapshot.
5. После drop UID proc worker namespace view не переоткрывается: retained FDs
   сравниваются с helper-issued identities только при доказанной immutability.
   EOF, death, generation/nonce/digest mismatch, unit replacement/drift, expiry,
   cgroup migration, unsupported filter/clone3 path -> revoke и Unavailable.
   Неизвестный child не получает supervisor anchor. Broker отдельно подтверждает
   собственный net/pid lookup scope и host-approved holder coverage.

## Независимый validator этого delta

Pending witness проверяет sender credentials, pidfd/starttime/UID, cgroup bytes,
nsfs FD identity и закрытые session binding поля. Он не является
AnchoredWorkerContext и не имеет allow/activation выхода: при текущем baseline
promotion завершает UnitProofGap. Production VerifiedWorkerUnit minting и
использование anchor в TrustedContext НЕ добавляются до approval.
Legacy direct-proc constructor остаётся WorkerViewUnavailable при foreign UID.
Retained FD foreign-UID fixture доказывает передачу/доступность FD после drop,
но не фальсифицирует systemd enforcement или positive admission.

Primary references:
- [systemd.exec: RestrictNamespaces](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html): restricts unshare/clone/setns, architecture constraints; не считать property единственным runtime seccomp proof.
- [canonical systemd D-Bus API](https://raw.githubusercontent.com/systemd/systemd/main/man/org.freedesktop.systemd1.xml): actual typed properties/InvocationID.
- [Linux namespaces](https://man7.org/linux/man-pages/man7/namespaces.7.html): retained ns FD удерживает namespace, dev/ino сравнивают identity; FD не доказывает неизменность membership процесса.

Evidence: `/home/myrs/projects/yonik/local/evidence/owner-anchor-62aaef9/`.

Read-only сверка PR109 head `a7c83ec5b06aef141f9bc69091fb15e1826465fc`:
тот же gap RestrictNamespaces в validate_service и тот же Run exec PID.
D-Bus RestrictNamespaces имеет signature `t` (uint64 flags), не boolean. Значение
полного запрета следует проверять по pinned systemd implementation/architecture,
а не разбирать human-readable yes/no. Также проверить DelegateNamespaces/clone3
и невозможность загрузки другой ABI; неизвестные semantics отвергаются.

## Проверки подготовленного delta

Test-only opaque validator (не production export/backend) в worker_anchor.rs:
3 targeted tests и весь attribution::broker test subset; Rust 1.85.1 --locked,
clippy --all-targets -D warnings, fmt. Проверены malicious non-nsfs FD и slot swap,
реальный wrong sender PID, pidfd substitution, starttime/cgroup replacement model,
revoke, expiry и session generation mismatch. Retained observation без unit proof
завершается UnitProofGap; нет constant-allow или альтернативного Known backend.

Actual owner_anchor_foreign.py: root-marked subordinate user namespace,
SCM_CREDENTIALS в обе стороны, fixed foreign child UID1001, ns FDs передаются
SCM_RIGHTS/CMSG_CLOEXEC. Helper закрывает исходные capture FDs и drops UID до1000
с нулевыми Inh/Prm/Eff/Amb caps. Только полученные retained FDs проходят fstat и
NS_GET_NSTYPE; direct proc namespace read отклонён. Starttime/cgroup и live pidfd
проверяются после drop; exit/death наблюдается pidfd. Fixture выводит proof gap,
а не positive admission. Python fixture является transport experiment, не
производственным сериализатором opaque Rust context. AST PASS, Ruff недоступен.

Дальнейший обязательный шаг после решения Root: FDwriter109 реализует actual
VerifiedWorkerUnit minting/attestation и pre-exec exchange/custody; лишь затем
подключается separate anchored worker channel context к broker admission.
Текущие данные не разрешают перевести immutable/available/capability flag в true.

## Уточнения Root: согласовано условно

Root согласовал разделение WorkerChannelAnchor/BrokerLookupView и предложенные
bounds NS_WITNESS (≤256, 3 FD, nonce32/runtimeConfigDigest32/generation/invocation,
1s/one outstanding, per-message credentials, pidfd/starttime/canonical cgroup).
Безусловный promotion НЕ согласован. Этот transport пока не shared ABI.

Проверенный canonical API: `org.freedesktop.systemd1.Service.RestrictNamespaces`
имеет `t`, getter читает ExecContext.restrict_namespaces как ulong. Это allowed
mask: для полного запрета нужен uint64==0, а не boolean true и не маска только
из известных нам bits. Проверить на pinned runtime systemd/architecture.
[Getter](https://raw.githubusercontent.com/systemd/systemd/main/src/core/dbus-execute.c),
[seccomp implementation](https://raw.githubusercontent.com/systemd/systemd/main/src/shared/seccomp-util.c)
показывают retain mask, полный setns запрет при 0 и clone3 ENOSYS. Некоторые
неподдерживаемые filter/architecture failures могут пропускаться — property сам
по себе не доказывает enforcement.

FDwriter109 после своего frozen VM gate и отдельного handoff Root должен получить
actual VM negative unshare/setns/clone3 controls в exact controlled invocation,
NNP/native architecture/caps и supported kernel enforcement. Нужны различимые
baseline controls, чтобы обычный capability EPERM/отсутствие syscall не выдать
за seccomp enforcement. Пока таких actual данных нет — UnitProofGap.

Если helper proc capture запрещён, переданные worker FDs не называются independent
proc proof. Допускается только отдельно attested protected pre-exec Rust bootstrap
ДО native exec/получения untrusted config, привязанный к trusted PID1 WorkerTicket,
ExecStart/invocation/image identity и immutability enforcement. Нет доказанного
trust chain — reject. Check children не получают Run anchor автоматически;
mount identity worker не переносится в lookup scope broker/app.

Ownership shared helper/worker/v2 остаётся у FDwriter109. Root передаёт ему brief;
этот delta не mint-ит production VerifiedWorkerUnit и не активирует admission.
