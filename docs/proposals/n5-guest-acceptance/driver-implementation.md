# N5 actual driver — SOURCE, runtime CLOSED

Entry: `/usr/lib/yonik-acceptance/n5-guest-driver --receipt-protocol=n5-v1`.
Это normal DesktopHost без backend/key/SystemSession override и без прямого
SystemClient Apply. Нет новых features/dependencies/unsafe или helper RPC.
Текущие repository authorization/manifest не изменены и ничего не разрешают.

## Sealed verifier

`sealed.rs` — приватная граница. До VerifiedFixture читаются только fixed
root-owned authorization/fixture/grant/launch JSON в `/usr/lib/yonik-acceptance`:
`n5-authorization.json`, `sealed-guest-fixture-v1.json`, `n5-user-grant.json`,
`n5-trusted-launch.json`. Любой missing/invalid grant закрывает вход до proc,
DBus, сети, subprocess и Host. JSON bounded, integer-only, duplicate/unknown
поля запрещены. Каждый компонент пути — openat/NOFOLLOW; все ancestors root:root,
без group/other write; sealed JSON regular root:root 0444, один hardlink.
Проверяются size/dev/inode/mtime/ctime/uid/gid/mode до/после чтения и SHA256.
SHA256 реализован локально без shell/dependency; есть known-vector pure tests.

Authorization сохраняет N5_AUTHORIZATION_V1, требует AUTHORIZED, пустые
source_blockers/missing и pins runner/fixture/grant, плюс `launch_sha256`.
Grant N5_USER_GRANT_V1 требует explicit_user_runtime_grant=true, source_commit,
runner/fixture/launch hashes и исходные 18 IDs. Это только формат будущего
root-sealed USER grant; source-тесты не создают authorized файлов.

Launch — exact `{schema:"N5_TRUSTED_LAUNCH_V1",manifest_sha256,after_setup,controls}`.
`after_setup` соответствует SEALED_GUEST_RECEIPT_V1 из fixture.py, включая
immutable ELF/source/profile/kernel/guest/build/ancestor/PID1/bus/namespace tuple.
Mandatory driver добавлен CORE commit 64a8c705; его fixture-группа интегрируется
root отдельно, драйвер её файлы не переписывает.

Controls exact: caller_uid, source_id, profile_id, private_bus, flows,
flow_observer, worker_cgroup, python_sha256, python_client_sha256,
kernel_boot_id, capture_nonce, capture_authorization_sha256, collector_sha256.
Private bus фиксирован `/run/user/<uid>/yonik-n5-private-bus`, runtime dir
`/run/user/<uid>`, data `/run/yonik-acceptance/data`, engines `/usr/lib/yonik/engines`.
Outer fixture заранее готовит настоящую зашифрованную БД и private Secret Service;
драйвер выбирает существующие source/profile IDs и ничего не импортирует.

После token измеряется hash действительного /proc/self/exe, затем сверяются live UID/boot ID/kernel release+GNU build ID/PID1 start,
namespace tuple, installed ELF/source/profile hashes, runner/collector/Python pins.
Build receipts exact SOURCE_BOUND_ELF_V1 читаются из fixed
`/usr/lib/yonik-acceptance/<artifact-key>.build.json`, root:root 0444;
их SHA256/source/ELF сверяются с sealed manifest. Fixed Python ELF — regular
`/usr/lib/yonik-acceptance/n5-python`, script — `n5-flow-client.py` рядом.
Это будущие evidence inputs outer fixture, не команды install текущей задачи.
Python flow-client/responder также закрыты без fixed root-sealed grant.
Caller JSON/env/argv сами по себе не создают VerifiedFixture или native proof.

## Протокол и реальные команды

Bounded stdin: frame 4096, суммарно 1 MiB, ожидание 60 s, очередь 1 frame.
Request schema=2, последовательный u64 seq, закрытые core/case/action enum.
Positive: start → await → stop → cleanup. Start запускает normal Host, выбирает
engine/source/profile с revision, включает System mode, требует существующую
валидную enabled executable policy — тот же predicate, что requires_attribution.
Await вызывает normal Connect и требует System Connected, затем реальные flows.
Legacy capability `system_tun_available` остаётся информационным наблюдением:
W2 исправляет production frontdoor, драйвер не обходит Unsupported/default false.

Snapshots bounded 64 × 64 KiB, overflow/timeout отказывает. Stop одноразовый;
Unknown/CleanupError означает RecoveryRequired, не разрешает новый Apply.
Closed Host не заменяет worker/cgroup/network cleanup. EOF/error вызывает только
normal Host shutdown; чужие PID не сигнализируются.

## Packets и app ownership

Flow exact: endpoint, source_ip, expected_peer_ip, transport, application,
action, token, response, control. Только TCP/UDP, rust/python, proxy/direct/block;
12 обязательных комбинаций, до 32 flows. Для каждого block требуется allow control
к тому же endpoint/protocol из другого executable. DIRECT требует peer source IP,
proxy — отдельный pinned egress IP. Это finite matrix, не вся Linux acceptance.

Rust child — этот же sealed ELF с внутренним `--flow-child=n5-v1`; Python child —
фиксированный ELF/script с -I. Нет shell/произвольных argv/env/исполняемых путей.
Parent удерживает pidfd, проверяет PID/start/UID/executable identity, фиксирует
local/remote tuple, nonce/transport и reap своего child. Timeout не равен block:
нужен отдельный реальный fixture-responder ledger (seen=0) и allow control.
Успех требует ответа с fresh nonce, expected server и actual observed peer IP.
`n5-flow-responder.py` — непривилегированный тестовый endpoint/ledger SOURCE;
его topology/изоляцию обеспечивает trusted outer fixture. Он не заменяет helper.

## Root captures и claims

Consumer `capture.rs` читает только root-owned readonly fixed captures из
`/run/yonik-acceptance/capture-{baseline,connected,stopped}.json`, после проверки
отдельных authorization/input файлов W1. API/schema — CORE commit 86e7663,
ROOT_CAPTURE.md с последующей root IPv6 correction. Network имеет строго семь
ключей links/addresses/routes/rules/routes6/rules6/nft; missing IPv6 не равен пустому
наблюдению. Сверяются source/manifest/case/stage/nonce/collector SHA/boot ID,
root PID/UID, artifact tuple; connected/stopped файлы должны быть записаны после
соответствующей границы драйвера. Отсутствие/устаревший capture → UNAVAILABLE.
Драйвер не запускает collector, не создаёт root captures и не читает nft как appUID.
Outer fixture координирует три stage capture в пределах bounded ожиданий.

Дополнительно удерживаются pidfds фактических canonical worker cgroup members;
Stop требует kernel empty и исчезновения этих tasks. Root tracked-process records
подтверждают отсутствие PID/starttime, не родительский waitpid. Network baseline
должен восстановиться полностью; persistent helper обязан сохраниться.
Убежавшие/ненаблюдавшиеся потомки остаются UNAVAILABLE. Root comparison — actual
observation, не общий N5 PASS и не доказательство ownership каждого network delta.

Receipt v2 хранит raw flows/root captures один раз в observations; claims ссылаются
на SHA256 этого блока (commitment, не подпись). ACTUAL обязательны для packets,
app ownership, trusted execution tuple и canonical cleanup. Внутренние full-v4
report/custody/roles/ACK — только SOURCE_INVARIANT (inference), привязанные к
actual Connected и reviewed executing closure. Нет придуманных counters/B/C/hash
внутреннего report. Unsupported internal injections — UNAVAILABLE/SOURCE_ONLY.
Original 18 IDs сохранены как история proposal, не обязательный runtime gate.

Runner future execute может достичь driver только после sealed pins/grant;
legacy receipt/counter protocol отвергается. Максимальный output 4 MiB,
case 180 s, cleanup 45 s. Missing/malformed observation даёт quarantine, не PASS.
Полная Linux/P12–P17 матрица, VM, registration, runtime и services остаются открытыми.
