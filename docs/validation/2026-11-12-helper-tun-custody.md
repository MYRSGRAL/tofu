# Fixed TUN wrapper и helper custody

Статус: draft для независимого review Journal/cancellation/ownership. Не production
admission, не завершённый FD handoff и не разрешение merge.
База: accepted origin/main `1db9ecf6a7daac523182a31ddd991e766ef2541e`.
PR147 и его HEAD `dd78f81c5c95930fad18b91f73f3a03d438e97a9` сохранены отдельно.

Approved external proposal: `/home/myrs/projects/yonik/local/orchestration/proposals/helper-owned-tun-custody/adapter.patch`,
SHA256 `17685396541af5f7bf7145dd9618c3f8f0a73c76e17619ca6abaae89795370f6`.
Перенесён только fixed FFI boundary, без prototype Journal/fixture issuer/parser.
libc остаётся существующей locked версии 0.2.191; pins/capabilities/native ABI не менялись.

## Реализованный scope

- `yonik-nsfs::tun::Queue` самостоятельно открывает `/dev/net/tun` с
  CLOEXEC/NONBLOCK/NOFOLLOW. Нет внешнего FD, AsFd/AsRawFd, Clone, generic ioctl
  или pointer API. Только private TUNSETIFF/TUNGETIFF/TUNGETVNETHDRSZ.
- Allowlist Linux x86_64 GNU; ABI assertions: ifreq size40/align8/data offset16,
  i32 size4. Initialized buffers; закрытый Framing Plain/Vnet10.
- `TunExecutor::bind_session_queue` удерживает существующий operation lease
  через namespace/Journal ownership/parent_index, confirmed/reconciled Session
  records, precheck metadata/framing/MTU, attach/inspect, postcheck и transfer.
  Ожидание lease отменяется существующим Cancellation. Drop future/error закрывает
  owning Queue; lease хранится внутри token до сохранения в Backend.
- `Backend::retain_canonical_queue` реально bind-ит и сохраняет private custody.
  Пока не подключён к producer entry: это явный временный dead-code allowance
  одного private endpoint до согласованного handoff/admission, не новый gate stub.
  Legacy start_profile и его by-name verification сохранены.
- При worker stop unknown custody остаётся; has_session блокирует новую сессию,
  cleanup_network отказывает. После успешного supervisor.recover original
  освобождается перед network cleanup. Session EOF не является confirmed stop.

Single-queue Linux не публикует IFLA_TUN_NUM_QUEUES: эти поля доступны только для
MULTI_QUEUE. Поле9 — disabled queues, не число attached queues. Поэтому precheck
использует carrier-off + single-queue metadata, а успешный fixed TUNSETIFF
атомарно доказывает свободную единственную очередь (занятая даёт EBUSY). Postcheck
использует живой owning Queue/TUNGETIFF, тот же name/ifindex/netns/Journal binding,
metadata/framing/MTU и carrier-on. Это не защита от внешнего CAP_NET_ADMIN между
snapshots. Основание: Linux v6.12 `drivers/net/tun.c`, tun_attach и tun_fill_info:
https://github.com/torvalds/linux/blob/v6.12/drivers/net/tun.c

Precheck выделен из общего topology verifier без ослабления существующих callers:
полнота IPv6 routes проверяется после carrier-on. После attach один повтор только
post-dump при NetworkReadError::Changed, без повторного ioctl и под общим deadline.
Повторный Changed/mismatch/cancel означает отказ.

## Закрытый consumer contract для root/W2/native owner — OPEN

1. W2 получает opaque InvocationBinding в send(Run) после trusted receipt и
   invocation/namespace checks. Его exe-receipt gap ещё не закрыт. Наш код не
   меняет SystemWorkerSupervisor или W2-owned файлы.
2. Backend удерживает original BoundTun. Предложенная следующая граница — private
   Arc<BoundTun> в Session вместо Roles.tun: OwnedFd. Session.close отзывает
   endpoint, но Backend original живёт до confirmed workerstop. Это пока proposal:
   Session ownership и её закрытый factory в этом diff не меняются.
3. Только existing full admission/ProductionPermit может создать Session; Queue,
   observation, caller bool и feature flag этого не разрешают. Metadata32 строит
   owner из связанного Journal/kernel state, а не из caller bytes.
4. W2 получает existing V2 Roles: channel + server_pidfd + exact192B Binding,
   без TUN в bootstrap/env. HELLO 176B → FEATURES 112B → ATTACH 128B/exact 1 SCM_RIGHTS
   → ACK 104B идёт по existing native Session. Единственный CLOEXEC duplicate
   создаётся внутри attach из живого Queue; generic FD factory не предлагается.
5. ATTACH metadata at96: role 1; at100: actual ifindex; at104: semantic 0x1001;
   at108: MTU; at112: NUL-padded name 16B. Kernel flags туда не копируются.
   Netns dev/ino не добавляются в ATTACH: их equality с target worker должна
   быть доказана предыдущим namespace/admission binding, до выдачи роли.
   Internal consumer framing закрыт engine identity: SingBox=Plain,
   Xray=Vnet10/header 10; PERSIST обязателен helper-owned link. Несовпадение
   отказывает до constructor. Wire ABI остаётся прежним.
6. Native owner должен отдельно исправить validator, подтвердить actual constructor
   и header handling. В `140e058` оба validateTUNFD требуют ровно 0x1001:
   Xray native-broker-codec.patch:2097, SingBox:2224. Helper persistent Queue
   даёт 0x1801/0x5801. Xray actual constructor в patch:1930 вызывает OpenTUN →
   LinuxTun{fdTun:newFdTun(fd,0)}: hardcoded header 0 несовместим с Vnet10.
   SingBox patch:2106 NewOwnedInterface → OpenTUN → options.FileDescriptor=fd,
   GSO=false → tun.New. Нужны реальные Plain/Vnet10 constructor/header controls,
   без маскировки framing. Эти Go patches не находятся в accepted базе этой ветки,
   прочитаны через git show 140e058 и не изменены.

Canonical ProductionPermit::mint и Session::verified_helper_bootstrap остаются
закрытыми. Positive ATTACH/native forwarding, crash/recovery и полный production
admission здесь не доказаны. После helper crash original FD закроется ядром,
но persistent link/journal требуют существующего recovery; crash controls не запускались.

## Проверки и evidence

Все heavy Cargo команды: CARGO_BUILD_JOBS=1, with-rust-slot, внешний
CARGO_TARGET_DIR под shared flock корня task-каталога. Никаких Docker/Go builds.
Реальные controls запускаются раздельно через unshare -Urnm с private tmpfs /tmp;
fixture отказывает до effects, если namespace совпадает с переданным host identity.

Evidence: `/home/myrs/projects/yonik/local/orchestration/evidence/helper-tun-custody-20261112/`.

- `real_journal_queue_custody_controls`: Plain/Vnet10, wrong framing/MTU,
  unreconciled Journal, namespace/name/index/metadata mismatch, lease transfer,
  cancellation while waiting lease и после attach, Drop pending bind, foreign
  alias, duplicate queue rejection, FD baseline, revoke и durable TUN cleanup.
- `real_queue_cloexec_and_close_on_error`: kernel queue/CLOEXEC, second attach
  refusal, unbound TUNGETIFF rejection, close/error FD baseline.
- `real_backend_custody_unknown_stop_revoke_and_cleanup`: actual Backend owner
  до ActiveSession, Busy cleanup при unknown, retained custody, release endpoint,
  real network cleanup через существующий isolated executor. Systemd stop не
  симулируется как доказанный: тест непосредственно вызывает private release
  endpoint, который production вызывает после supervisor.recover.
- Linux lib: 485 passed, 34 ignored. nsfs lib: 2 passed, 1 ignored.
- Strict production scope clippy `--no-deps --lib -- -D warnings`: PASS.
  Полный all-targets заблокирован существующими findings вне diff:
  yonik-engine/src/lib.rs:170 collapsible_match (без --no-deps) и
  yonik-linux/tests/resource_fd_receiver.rs:147 cloned_ref_to_slice_refs.
  Чужой код не исправлялся. Toolchain текущего workspace: Cargo/Clippy 1.97.

До merge обязателен отдельный независимый review Journal/cancellation/ownership.
