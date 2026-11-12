# Worker anchor: production transport и независимые VM gates

База этого продолжения: `88dafca9d9fc7e371a4947d36a46f8de2c2aad1c`.
Root согласовал optional Run-only exchange до payload/assets. Ни transport,
ни перечисленные probes пока не разрешают positive foreign-UID admission.

## Custody и compatibility

`worker::anchor_exchange` использует отдельный offer `YNKANC01` с одним private
SEQPACKET FD. Legacy headers/preflight читаются прежним decoder: bootstrap
сначала делает peek и не потребляет legacy bytes. Check отвергает anchor offer.
После начала negotiation fallback отсутствует. NS_WITNESS содержит 128 байт,
helper nonce/config digest/session/generation/invocation; reserved нулевые.
Каждое SEQPACKET сообщение требует kernel credentials, CMSG_CLOEXEC, exact frame,
FD count, отсутствие truncation. Challenge одноразовый, общий deadline 1 секунда.

Worker открывает только фиксированные self net/pid/mnt и закрывает все sender
copies после sendmsg, до closed-ACK. Helper закрывает полученный mnt до чтения
closed-ACK; подтверждает ACK до возврата exchange caller. NamespaceObservation
хранит ровно net/pid FD и identities, не mount FD. Observation не anchor,
не owner witness и не даёт namespace-changing API.

BootstrapUnitObservation создаётся только canonical systemd supervisor для
fresh pending Run до payload: PID1 unique owner/UID/PID, installed root-owned
fixed image/unit, actual GetAll contract, invocation/MainPID/pidfd/starttime,
fixed UID и canonical cgroup. Yama policy 1/2/3 обязательна для loader phase до
nondumpable. Этот объект ещё не OpaqueVerifiedWorkerUnit: kernel proof отсутствует.

BrokerLookupView отдельно захватывает собственный scope процесса broker.
Kernel owner/holder/executable scan и cache revalidation используют этот scope.
Legacy Broker::new сохраняет проверку равенства прежнего direct-proc worker
scope; изменение не открывает legacy foreign UID admission.

## Actual signed disposable VM

Evidence: `/home/myrs/projects/yonik/local/evidence/worker-anchor-production-4a4/`.
Base cache `/home/myrs/projects/yonik/local/validation/vm-base` открыт read-only
под shared flock корня. Подпись проверена отдельным GNUPGHOME; primary fingerprint
`1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`, SHA256 image
`794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`.
Source/probe binary/unit manifest сохранён до VM изменений.

Actual runtime: systemd 262.1-1-arch (+SECCOMP), kernel 7.2.11-arch1-1 x86_64,
Yama ptrace_scope=1. Два отдельных controlled unit запускают один static probe
с User/Group=nobody, NoNewPrivileges=yes, NET_ADMIN-only bounding/ambient caps,
SystemCallArchitectures=native. Отличие — RestrictNamespaces=yes у второго.

| Probe | Control | Filtered |
| --- | --- | --- |
| unshare каждого из 8 namespace bits + заведомо invalid flag | EINVAL (22), 8/8 | EPERM (1), 8/8 |
| setns(-1,0) | EBADF (9) | EPERM (1) |
| clone3(NULL,0) | EINVAL (22) | ENOSYS (38) |

Invalid arguments намеренны: probes не создают namespace/process и отличают
filter interception от обычного capability EPERM. Отдельный actual property
probe подтвердил D-Bus `RestrictNamespaces: t 0`, `SystemCallArchitectures:
as 1 "native"`, NNP и NET_ADMIN-only caps. Units завершены/остановлены.
Эти probes не исполнялись в exact production worker invocation и не заменяют
её kernel attestation. Actual production-helper admission gate остаётся открытым.

Первый ошибочный install из-за source path substitution завершился до изменения
файла; после проверки исходного/целевого пути выполнен исправленный install.
Следующий command остановился на отсутствующем `/usr/bin/systemd`, после install;
install не повторялся. Для read-only версии использован `/usr/lib/systemd/systemd`.
Все historical failure logs сохранены.

## Actual foreign-UID transport

`actual-exchange-vm-v2.log`: helper UID0, child worker UID65534, production
bootstrap transport, explicit closed-ACK до первого payload byte — PASS.
У child проверено отсутствие открытых mnt namespace FD после bootstrap; у helper
ровно два retained namespace FD. Это fixture transport gate: process child ещё
не fixed systemd worker, helper test process ещё не production service с прежними
ограничениями. Он не доказывает UnitProof или positive Broker admission.
Первый actual run упал на слишком строгой проверке `flags.is_empty()` при recv
SCM_RIGHTS. Исправлено: принимается kernel CMSG_CLOEXEC flag, отвергаются
TRUNC/CTRUNC. Historical failure log `actual-exchange-vm.log` сохранён.

## Fixed production unit bootstrap

`actual-production-unit-gate.log`: установлены manifest-checked production
helper/worker binaries и неизменённые units. Разрешённые signed VM packages:
polkit128 и duktape2.7; python уже установлен. Production helper active,
actual CapInh/CapAmb=0, CapPrm/CapEff/CapBnd=NET_ADMIN (0x1000), NNP=1,
Seccomp=2. `production-after.log` сохраняет actual status.

Из отдельного controlled test unit с UID0/NNP/NET_ADMIN-only bounding/effective
caps вызван production SystemWorkerSupervisor API. PID1 запустил fixed
`yonik-worker.service`, User/Group=yonik-worker UID/GID969. Canonical observation
и pre-payload exchange — PASS; retained net/pid=2, explicit revoke=0, cleanup
worker/sockets/cgroup — PASS. Worker намеренно не получает payload: EOF даёт
его exit1/failed, MainPID=0, ControlGroup пустой. Это не failed admission gate.
Issuer этого observation — тестовый процесс с production supervisor API;
actual helper service работает рядом. Gate пока не проверяет issuance внутри
production helper session flow, opaque verified unit promotion или positive
Rust broker lookup. Root-issued authority не заявлена.

`actual-exchange-vm-v4.log`: 6 PASS, включая 8 forgery cases и FD count после
каждого отказа, positive custody и revoke/expiry. Дополнительный host test
проверяет сохранение legacy SCM_RIGHTS после MSG_PEEK; legacy v2 не потребляется.

VM завершена штатным poweroff из finally по bounded 1800s deadline;
`vm-shutdown.json` подтверждает QEMU exit0. Runner вернул TimeoutError ожидания
finish; последующая cleanup-команда SSH/finish не исполнилась, так как VM уже
завершилась. Ошибка сохранена, мутации не повторялись. Бинарники и manifests
сохранены вне transient (`binaries/`, `preserved-binaries.json`).

## Незакрытая граница NS_GET_NSTYPE

Locked rustix 1.1.6 не предоставляет safe NS_GET_NSTYPE wrapper;
`rustix::ioctl::ioctl` требует unsafe. Workspace `unsafe_code=forbid` сохраняется,
Cargo manifests/lock не изменены. Внешний готовый safe API не подтверждён.
Конкретный proposed local yonik-nsfs 0.1.0 adapter (MSRV1.85, GPL проекта,
existing rustix1.1.6, 0 новых registry transitives) находится в evidence
`nsfs-dependency-proposal.patch`; его isolated unsafe exception ожидает явного
решения пользователя. Предложение не применено. Symlink names и nsfs magic
не используются вместо kernel type proof.

До разрешённого adapter и exact-invocation enforcement отсутствует production
OpaqueVerifiedWorkerUnit/WorkerChannelAnchor promotion. Capability остаётся false;
public RPC/caps v1, Go, packaging, credentials, resources store и encoders не менялись.

## Проверки перед публикацией draft

Rust1.85.1 `--locked`: полный lib run 426 PASS/31 ignored (последовательный,
чтобы существующий global FD-count test не гонялся с соседними tests).
После opaque identity API проверен affected exchange subset: 7 PASS;
clippy --all-targets -D warnings и fmt — PASS. VM-only guards при host run
не считаются actual VM evidence: actual результаты перечислены отдельно выше.
Ни Cargo manifests/lock, ни unsafe policy не изменены.

PR109 во время работы advanced до `ecabf78e262fc020eb1db83adc430b17aa9b9b1e`:
main integration, 71 files, owned helper/worker/broker files не менялись.
Текущий source сохранён на required base2df0b07 с отдельным88dafca; интеграция
нового base и production promotion не объявлены проверенными.
