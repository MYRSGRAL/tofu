# PR90: bounded provenance audit и предложение native bridge

Статус: предложение для решения root; реализации Go, новых capabilities и новых
VM runs в этом audit нет. Code head при audit: `340ccf40b9fe3a64c8eed690699256b89defe81e`.
Evidence: `/home/myrs/projects/yonik/local/validation/attribution-minimal-95e8847/audit-c758-340ccf4/`.

## 1. Противоречие #49 закрыто различием binary

#49 exact head `c758bd243faba7909ea8f06d626dcf97508e8166`, libtest SHA
`5e603603004fa67fd5c8056aba9b6bf9a2e16849be394ca38487afbe7fc40064`.
Его actual 12/12 использует **adapter** sing-box SHA
`4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.
PR90 устанавливал **stock** SHA
`1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a`.
Версия обоих в первой строке — `sing-box version 1.14.4`; одной строки version
недостаточно для выбора capabilities.

Существующий manifest `local/artifacts/sing-box-external-tun-versioned-v1.14.4/manifest.json`
привязывает adapter к upstream `c005f35300ce76427dc285481ec5695f5b179cd2`, четырём
patch hashes, Go build settings и binary SHA. Последний patch:
`external-tun-configuration.patch`, SHA
`9d27b8c00af801e1be107ccc6b4271aa445ee62b173d5319783064b1c376c9f5`.
Patch из старого source.tar и текущего repo побайтово совпадает. Он добавляет
JSON `external_configuration` и передаёт его в sing-tun EXP_ExternalConfiguration.

Один bounded offline контроль: сохранённый PR90 config SHA
`e140c43301d16e8a1bc1b25cd3cf8708e929c58fd1efba3419e3717583a42d9f`
проверен обоими сохранёнными executable в отдельных пустых user/net namespaces.
Adapter: check exit=0. Stock: exit=1, unknown field external_configuration.
`same-config-binary-controls.json` содержит exact args/hash/outputs. Новый config
не генерировался, поле не удалялось, интерфейсы не создавались.

TUN attach JSON старого и нового compiler одинаков в существенной части:
external_configuration=true, auto_route/auto_redirect=false, dns_mode=disabled.
Diff system_tun.rs меняет policy compiler entrypoint для executable selectors,
а не external_configuration. Это не причина decode error. Старый gate использовал
libtest + private bus/polkit fixture и sleep helper lifecycle unit; новый —
production helper executable/system bus/polkitd. Worker binaries тоже различаются
(e44b0d9d… в #49; 2ad61c76… final PR90). Эти отличия ограничивают перенос runtime
acceptance, но не объясняют parser error: его независимо воспроизводит binary
контроль на одном config. Старый effective libtest umask/config FD mode не
записан; не приписываем ему выдуманное значение и не переносим stdin proof.

Предыдущая фраза PR90 «нужен новый TUN adapter из-за unknown field» исправлена:
для старого name-based contract нужен **уже существующий правильный adapter**.
Его production acceptance с новыми helper/worker в этом audit не повторялась.

## 2. Обязательная truthfulness поправка app admission

`actual-final-app-negative.log` — FAIL initial State=unknown, до Prepare/denial.
Exact final helper SHA 39164032f409cd185d4eaaea49daaa166aa2f6b8ce1cc0adbda4e0881343e907,
worker 2ad61c765606402216eeb59646e2ed0e98cb3de990ec0fc1014cffb2c9d610cc,
gate 62d9e8231c7088405e3cc563afdca50f7a45495908d5b706d66db9806f213e83.
Поэтому final app admission head 340ccf4 имеет **только static/code evidence**,
а не actual denial PASS. Это не вывод об ошибке самого preflight: он не достигнут.
Равные app-before/app-after snapshots здесь не доказывают denial/no transient effects.

`actual-app-negative.log` — earlier PASS обоих ядер. Его manifest фиксирует base
95e88474c0b4a18797dc28fa873a4b6b3adf2fa4 **плюс незакоммиченный snapshot**:
helper dbdd4aee5dbe64c87d8d4859d2c3c2db7c79a08b969fc333dd6ddcb5f1c7d22a,
worker a678a76e4590b8eb10142d9f07218a412c7ac24943fab976b6719ba623d140a4,
gate fd64965642e293ead7642cbbf19cef18b641b73a469b605461f8d0da8fefbfd2.
Ни base 95e8847 без diff, ни clean head 585f53d/340ccf4 не были тем earlier PASS
source. Честного exact committed head для этого PASS нет; exact identity — набор
source hashes в native-gate-manifest-before.json. Отличающиеся Rust paths против
340ccf4: worker.rs, systemd_worker.rs, их tests и installed_helper_session.rs.
Raw logs не менялись; сохранены hashes и копия прежнего shipping-results перед
исправлением. Xray final non-app и sing-box stdin control сохраняют свои actual PASS.

## 3. Существующий hook и точная граница no-effects

Primary sing-box commit выше: `experimental/libbox/platform.go` содержит OpenTun
и FindConnectionOwner; `experimental/libbox/service.go:57` реализует OpenInterface:
получает FD через OpenTun, делает dup, задаёт tun.Options.FileDescriptor и вызывает
sing-tun.New. UsePlatformInterface/UsePlatformConnectionOwnerFinder возвращают true.
Это in-process Go interfaces, **не** готовый C ABI/Rust SDK/standalone CLI RPC.

Pinned dependency `sing-tun@040db71fcd33`:

- New при FileDescriptor>0 не вызывает open/configure; GSO=true всё ещё вызывает enableGSO.
- Start при таком FD пропускает native start/link/address/route setup.
- unsetAddresses/unsetRoute/unsetRules имеют FD guards; FlushDNSCache тоже.
- EXP_ExternalConfiguration даёт close-only ветку Close.

Существующий name-based adapter не передаёт owned FD. Он оставляет MTU/up/offload
операции, что уже прямо оговорено helper-tun-configuration.md. Нельзя объявить
ему более сильную capability owned_fd_no_effects.

Root-facing предложение: сохранить out-of-process model и добавить **private**
Rust↔native bridge ABI v1 по inherited AF_UNIX SOCK_SEQPACKET, не gomobile/CGO.
Публичный Prepare/RPCv1 и policy/evaluator signatures не меняются в этом плане.

1. HELLO(nonce32, abi=1, required_features, config_digest32, session_epoch).
   Ответ FEATURES с тем же nonce/epoch и schema version. Frame ≤4096 bytes,
   deadline 1s, ≤1 outstanding control exchange. Unknown/reserved fields отказ.
   Runtime features сравниваются с root-controlled exact binary/patch manifest;
   self-report и строка 1.14.4 не security authority. Stock и old adapter не
   заявляют новые capabilities. Существующий registry пока хранит version/hash,
   не это feature attestation — требуется узкое расширение trusted registry.
2. ATTACH(config_digest, expected netns dev/ino, ifindex, TUN flags, mtu, epoch)
   передаёт ровно один dup **уже открытого helper-owned TUN FD** через SCM_RIGHTS.
   FD≥3, не число из UI/JSON, без открытия /dev/net/tun/native-by-name fallback.
   Rust держит original lease. Shim проверяет TUNGETIFF/name/flags, ifindex и netns;
   MTU/addresses читаются для сверки, не записываются. Вызов OpenTun возвращает
   только принятый FD, а options фиксируют FileDescriptor>0, GSO=false,
   EXP_ExternalConfiguration=true, AutoRoute/AutoRedirect/StrictRoute=false,
   DNSMode=disabled и отсутствие native DNS addresses/platform mutation options.
3. READY(nonce, epoch, config_digest, tun_identity, no_effects=1) только после
   проверки FD/options и готовности owner broker. Пока READY нет, dataplane не
   публикуется; helper сохраняет текущий guarded apply/rollback lifecycle.
   Отмена/ошибка: Stop, FD close + process/cgroup empty proof до освобождения lease.
   Native Close закрывает dup, никогда не удаляет helper resources. EOF/loss epoch
   инвалидирует READY. Control/buffer/FD custody включаются в existing budgets.

Go bridge должен согласовать existing external_configuration validator с этим
verified platform-owned mode: текущий patch отвергает usePlatformInterface=true.
Это явное typed сочетание с доказанным FD, не снятие проверки/удаление JSON поля.
No-effects acceptance отдельно проверяет setup, update и Close; поддержка одного
только FileDescriptor без GSO/DNS/Close guards недостаточна.

## 4. Один trusted attribution вариант для решения root

Выбран root-managed **Rust broker per current holder UID** с protected executable,
а не доверие к UI/user daemon. Root helper — authority допуска policy/coverage и
lease epoch; root-verified broker — делегированный authority только kernel owner
snapshot. PID1 запускает фиксированный root-owned broker unit для UID из проверенной
kernel/session identity. Helper не получает SETUID/SETGID; broker capabilities пусты,
NoNewPrivileges, dumpable=0 до открытия IPC, очищенная environment/groups, root-owned
код/конфигурация, отсутствие произвольных exec/path/read команд.

Peer identity: broker-created AF_UNIX endpoint; root helper сверяет SO_PEERCRED с
PID1 MainPID/InvocationID, UID и живым pidfd, затем session nonce/epoch. Один UID
не является достаточным доказательством. Same-UID подмена socket даёт отказ по PID,
не доверенный ответ; dumpable=0 защищает broker от same-UID ptrace/FD theft. Native
получает только helper-approved connected channel; UID/process из payload не authority.

Lookup(protocol, local_addr/port, remote_addr/port, netns_identity, epoch, request_id)
возвращает Known/UnknownFlow/MechanismUnavailable. Не принимает произвольный PID,
proc path или executable assertion. Broker сверяет live socket diag tuple+cookie+
inode и kernel UID, затем same-UID proc FD ownership и открытый exe FD dev/ino/path,
pidfd/starttime и namespace identity до/после capture. Socket UID — creator UID,
а holder UID может измениться при privilege drop/FD passing: нельзя выбирать
единственный broker только по socket UID. Root-managed bounded broker roster
агрегирует holder proofs; конфликтующие holders/executable identities дают UnknownFlow.
PID reuse, exec/image change, неоднозначный UDP reuseport или исчезнувший socket
не должны превращаться в Known. Cache TTL сам по себе не proof.

Гарантия — проверенный owner/executable **snapshot интервала lookup**, не недоказанная
identity при socket creation и не обещание атомарности относительно будущего exec.
Known содержит observation interval, cookie, starttime, executable identity, epoch;
новая flow/session требует нового proof. Сохранённый pidfd/FD не доказывает отсутствие
последующего exec. Если root требует birth-time identity/линеаризацию с exec для
всех процессов, этот вариант не удовлетворяет контракту: его нельзя молча объявить
FR20. Это явная граница предлагаемого решения, а не скрытый TTL fallback.

Proposed global bounds: 16 UID brokers, 8 parallel lookups/256 queued total,
8MiB index/65536 FD entries total, ≤256 PIDs и ≤256 FDs на PID за один bounded
refresh (с общим deadline 20ms), positive observation age ≤20ms. Переполнение/потеря
broker/view — MechanismUnavailable, не UnknownFlow; stale Known запрещён. Эти числа
подлежат проверке positive 10× gate, сейчас не performance claim. Nondumpable/недоступный
отдельный process — UnknownFlow; общий запрет proc access/нет покрывающего UID broker —
MechanismUnavailable. Coverage roster/epoch проверяются перед Prepare/Apply, новые
непокрытые UID не получают ложный native default под объявленной поддержкой.
Обычный UnknownFlow сохраняет existing evaluator semantics; потеря механизма
обрабатывается native hook как fail-closed error, без process-rule fallthrough.

До положительного acceptance требуется проверить обычные dumpable приложения
реального чужого worker UID обоих ядер; per-UID broker не обещает доступ к любым
nondumpable/privileged processes. Текущий NET_ADMIN witness не доказывает, что
SYS_PTRACE неизбежен для всех возможных архитектур, и не доказывает работоспособность
этого ещё не реализованного broker.

## 5. Что действительно менять и что одобряет root

- Для объяснённого external_configuration rejection **ноль новых Go changes**:
  выбрать существующий adapter 4c31… с проверенным patch/build manifest.
- Для stronger owned-FD ABI нужен Linux Go bridge/entrypoint к existing
  PlatformInterface OpenInterface/OpenTun, узкое согласование validator и
  nonce/feature/FD/READY protocol. Standalone stock CLI этого ABI не предоставляет.
- Для positive FR20 нужен native integration обоих ядер: sing-box existing
  FindConnectionOwner hook через bridge; Xray FindProcess/routing integration
  обращается к trusted broker. В pinned Xray app/router/condition.go
  ProcessNameMatcher.Apply вызывает common/net.FindProcess и при любой ошибке
  возвращает false: одной замены FindProcess недостаточно. Нужен native flow-entry
  guard/propagation до любого default dispatch, иначе MechanismUnavailable станет
  rule miss. Аналогично sing-box bridge не должен превращать owner-finder error в
  пустой ProcessPath. Классы Known/UnknownFlow/MechanismUnavailable и failure path не сводятся к пустому process path/default. Existing selector
  signatures/sourcePolicy/evaluator не ослабляются.
- Rust changes следующей задачи: broker, fixed PID1 unit factory/peer verification,
  kernel proof/quotas, private protocol custody, trusted manifest features и
  helper admission lease. Current system_client/GUI/P04/composition/DNS не трогать.
- Новые capabilities для предложенного normal-dumpable snapshot варианта **не
  требуются**: worker сохраняет NET_ADMIN для existing network duties; brokers
  работают с пустыми caps. No SYS_PTRACE/BPF/PERFMON/SETUID/SETGID grants и no proc
  weakening. Нельзя переносить это утверждение на более сильный birth-time/all-
  process contract; такой контракт потребует отдельного доказанного дизайна.

Approval-ready решение: подтвердить snapshot semantics/coverage и описанный
private FD+owner bridge ABI; только затем реализация native bridge/broker.
Acceptance — реальные Known-selected proxy/block outcomes TCP/UDP v4/v6 обоих ядер
при 1×/10×, отдельный UnknownFlow, broker/namespace/exec/PID-reuse loss, отсутствие
link/address/route/DNS mutations native на start/update/close, fail-closed до Apply
при недоступном механизме. Старый 12/12 и текущий conservative Unsupported этого
positive acceptance не заменяют. Ни один такой новый VM gate сейчас не выполнялся.

Pinned primary anchors (полные bytes/SHA сохранены в audit evidence):
[libbox wrapper](https://github.com/SagerNet/sing-box/blob/c005f35300ce76427dc285481ec5695f5b179cd2/experimental/libbox/service.go),
[platform interface](https://github.com/SagerNet/sing-box/blob/c005f35300ce76427dc285481ec5695f5b179cd2/adapter/platform.go),
[TUN inbound](https://github.com/SagerNet/sing-box/blob/c005f35300ce76427dc285481ec5695f5b179cd2/protocol/tun/inbound.go),
[sing-tun Linux](https://github.com/SagerNet/sing-tun/blob/040db71fcd33/tun_linux.go).
Owner Lookup reply предлагается ограничить 8192 bytes и ровно двумя FD при Known
(pidfd + opened exe identity); MSG_CMSG_CLOEXEC и FD-count validation обязательны.
Нет чтения содержимого exe через RPC и нет reopen унаследованного FD через proc path.
