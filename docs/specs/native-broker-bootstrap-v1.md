# Production NativeBroker v1 — closed proposal для Root/Go writer

Статус: FROZEN v1, согласовано Root с обязательными поправками ingress/cookies,
features и trusted server credentials до wire. Прежнее
предложение YOB1/136 было основано на fixture, НЕ становится shared ABI.
Новый magic/type/length protocol ниже отличен от attribution broker fixtures.
Никаких изменений Go FD roles в Rust до Root approval.

## Transport, issuer и roles

UNIX SOCK_SEQPACKET, SO_PASSCRED до передачи endpoints, MSG_CMSG_CLOEXEC при
каждом recvmsg. Native endpoint только inherited `YONIK_NATIVE_BROKER_FD` из
закрытого Run worker bootstrap; произвольный env/openFromEnv не authority.
Server endpoint принадлежит helper session issuer/relay. Native->server каждое
сообщение имеет actual fixed worker MainPID/UID SCM_CREDENTIALS; Run exec меняет
image ТОГО ЖЕ PID. Check/version children этот endpoint/anchor не получают.
Native проверяет server PID/UID/pidfd/starttime, полученные из trusted closed
worker bootstrap, на каждом сообщении; HELLO не является источником server
identity. Epoch/digest также проверяются на каждом сообщении. Присланные
PID/UID поля не заменяют проверку credentials/liveness.

Helper mint-ит VerifiedWorkerUnit только из canonical PID1 owner/GetAll fixed
unit identity/ExecStart protected image/UserGroup/MainPID/pidfd/starttime/
invocation/cgroup, typed RestrictNamespaces uint64==0/native architectures,
NNP/NET_ADMIN-only caps и actual supported kernel enforcement. NS witness —
trusted protected Rust bootstrap ДО payload/native exec, не caller FD.
mnt FD initial type/identity only, затем CLOSE всех sender/receiver copies ДО
payload/assets. В broker и Go mount namespace FD не передаётся.

Разделение ролей:
- namespace proof: private NS exchange, ровно net/pid/mnt; позднее retained
  net/pid в scoped lease, mnt только opaque identity. Не TUN и не lookup view.
- app-UID Rust lookup broker: helper-controlled private channel и отдельный
  BrokerLookupView собственного app scope; caps0, PID1-controlled launch без
  новых caps у helper. Не получает TUN FD. Возвращает kernel owner observation.
- native channel server/relay: helper issuer, а не app caller. Только он передаёт
  ATTACH с одним helper-owned already-bound TUN queue duplicate. Helper удерживает
  original до session cleanup; Go владеет received duplicate. FD не выбирается
  native JSON/env/caller request. Unbound /dev/net/tun не ATTACH authority.

Root должен согласовать private worker role handoff: один authenticated
SEQPACKET native endpoint, без namespace/TUN FD в Go environment. ATTACH ниже
передаётся уже по этому endpoint. Worker наследует только channel FD при Run
exec; TUN arrives через ATTACH, отдельного YONIK_OWNED_TUN_FD env API НЕТ.
Existing native NewTun arbitrary env openFromEnv не участвует в trusted path.

## Closed worker handoff и trusted server identity

Producer-side private extension предложен отдельно от native wire: negotiated
`YNKANC02` (старый diagnostic YNKANC01 и bundle v2 без изменений). NS witness
v1 остаётся 128B/exact3FD; optional protected kernel probe report отдельный,
не bool authority. После validated NS proof и closed-mnt ACK helper передаёт
worker `YNKROLE1`,192B/exact2SCM_RIGHTS: slot0=NATIVE_CHANNEL, slot1=SERVER_PIDFD.
Это НЕ ATTACH и не TUN role; TUN передаётся позднее только native ATTACH.
Role record: magic0..8; nonce8..40; digest40..72; epoch72..80;
session80..96; generation96..104; invocation104..120; server PID120..124;
server UID124..128=0; server starttime128..136; reserved136..144;
role_count u16 at144=2; role0 u16 at146=1; role1 u16 at148=2;
reserved150..192=0. Record проверяется по credentials actual helper peer PID/UID,
canonical PID1 helper MainPID/invocation, live pidfd/starttime. Role ACK без FD
повторяет binding с magic `YNKRACK1`; one outstanding/deadline1s, mismatch closes.
Producer implementation этого extension требует review Root; перечисление
не означает, что Go должен принимать namespace/control records.

Перед Run exec worker инжектирует в очищенный Command environment ТОЛЬКО свои
verified values: `YONIK_NATIVE_BROKER_FD=<channel fd>`,
`YONIK_NATIVE_BROKER_SERVER_PIDFD=<pidfd fd>`,
`YONIK_NATIVE_BROKER_BINDING=<hex exact192-byte role record>`.
CLOEXEC снимается только с двух этих descriptors перед same-PID Run exec.
Ни config JSON, ни caller environment не могут задавать эти entries. Check/version
не наследуют descriptors. Go хранит expected server PID/UID/starttime/epoch/
digest/nonce из этого bootstrap, а не HELLO. FDINFO pidfd/liveness и per-message
SCM_CREDENTIALS сверяются на каждом message; server UID0 обязателен. Номера FD
и env сами по себе не создают authority: only helper-controlled worker issuance
с fixed verified sender/recipient допускает этот runtime path. Arbitrary upstream
NewTun openFromEnv не используется для TUN: accepted TUN только ATTACH после
successful authenticated HELLO/FEATURES по этому каналу.

## Common header — 64 bytes

Все integers unsigned big endian. Magic bytes0..8=`YNKBROK1`.
version u16 bytes8..10=1; type u16 10..12; total_bytes u32 12..16;
request_id u64 16..24; network_epoch u64 24..32; RUNTIMEconfigdigest32 32..64.
Max total frame=768 bytes. Exact length/type, flags/reserved/unknown roles reject.
Zero/extra/duplicate/truncated FD/control messages reject and close all FDs.
Никакого native JSON/caller PID/UID/path в request.

Types: HELLO=1, FEATURES=2, ATTACH=3, ATTACH_ACK=4, LOOKUP=16,
LOOKUP_REPLY=17, REVOKE=32. Type determines exact FD count: ATTACH=1,
все остальные=0. Bootstrap request_id=0; LOOKUP id1..u64MAX strictly increasing,
без wrap/reuse. Responses echo exact id/epoch/digest; late reply не переиспользуется.

## Ordered bootstrap (общий deadline1s от Run Accepted/pre-exec-ready)

1. Server HELLO (176B): header64; nonce32 at64; session16 at96;
   generation u64 at112; invocation16 at120; max_frame u32 at136=768;
   max_inflight u16 at140=1; reserved2 at142; owner_max_TTL_ns u64 at144=20M;
   bootstrap_deadline_ns u64 at152=1G; supported_features u64 at160=7;
   immutable policy_flags u64 at168 (bit0 app-required, остальные0).
2. Native FEATURES (112B): header64; nonce32 at64; supported_features u64 at96;
   selected_features u64 at104. Bits: OWNED_TUN=1, OWNER_LOOKUP=2,
   STRICT_APP_REQUIRED=4. Unknown bits reject; selected согласуется с HELLO: OWNED_TUN обязателен всегда; app-required
   требует OWNER_LOOKUP+STRICT_APP_REQUIRED. selected0 и понижение требований
   отвергаются. Echo не proof: credentials/anchor обязательны.
3. Server ATTACH (128B, ровно1 SCM_RIGHTS): header64; nonce32 at64;
   role u32 at96=1(OWNED_TUN); actual ifindex u32 at100; semantic TUN flags u32 at104=0x1001 (IFF_TUN|IFF_NO_PI, single queue);
   MTU u32 at108; IFNAMSIZ16 NUL-padded ASCII name at112. FD duplicate already
   bound к exact helper-owned journal/TUN identity; native не делает TUNSETIFF
   создания/переименования по config. Root retains original, Go owns duplicate.
4. Native ATTACH_ACK (104B,0FD): header64; nonce32 at64; role u32 at96=1;
   reserved u32 at100=0. До ACK нет LOOKUP/forwarding.

NS negotiation и mount-close ACK завершаются раньше payload/assets. Native
bootstrap timer начинается после Run Accepted/pre-exec-ready, чтобы asset
materialization не съела handshake deadline. EOF/timeout после начала
negotiation закрывает endpoint/TUN/all flows; silent legacy fallback запрещён.

## LOOKUP — 144B,0FD

header64; nonce32 at64; family u8 at96=4/6; protocol u8 at97=6/17;
flags u16 at98=0; local_port u16 at100;
remote_port u16 at102; local_addr16 at104; remote_addr16 at120;
cookie u64 at136=0. TUN ingress v1 не имеет kernel app-socket cookie witness.
Любые nonzero flags/has-cookie/cookie -> Malformed до отдельного verified source
contract. Reply cookie только информационный; нельзя echo его в запрос или
фильтровать им reuseport/другие tuple matches. IP network order, IPv4 первые4 bytes,
остальные12 нули. Ports/tuple строго bounded/canonical. SourceUID/caller PID,
request-supplied process/executable/path отсутствуют.

После DNS и непосредственно перед dial native повторно revalidate через LOOKUP
исходного canonical app tuple (не proxy-upstream tuple). Просроченный/отозванный
owner не используется.

Root relay сначала проверяет fixed worker credentials+live anchor, затем
передаёт canonical tuple app-UID broker по отдельному trusted channel. App
broker выполняет SOCK_DIAG/holder/FD/exe proof в своём lookup scope, не worker mnt.

## LOOKUP_REPLY — 160B + path_len, max672B,0FD

header64; nonce32 at64; status u8 at96: Known=1/Unknown=2/Unavailable=3;
reason u8 at97: none0/proofgap1/revoked2/expired3/coverage4/limit5/malformed6/
ambiguous7; reserved2 at98; path_len u16 at100 (0..512); reserved2 at102;
remaining_owner_TTL_ns u64 at104 (<=20M); actual socket cookie u64 at112;
observed owner PID u32 at120; reserved4 at124; observed starttime u64 at128;
executable dev u64 at136; executable inode u64 at144; reserved8 at152;
actual kernel-observed executable path UTF8 at160 (exact path_len, no NUL).

Known projection взят только из retained executable FD + validated pidfd/holder
snapshot, не native proc lookup. Path — observed kernel data, не caller hint;
берётся по retained exe FD, затем повторная owner/exe revalidation до ответа.
Deleted/nonUTF8/too-long path -> Unavailable(Coverage), не guessed identity.
Unknown/Unavailable: path_len0, TTL/cookie/owner fields0. Known не policy allow:
Go сопоставляет observation с compiled app policy, связанной digest.

TTL консервативно уменьшается на полный request round-trip; остаток<=0 ->
Unavailable. Snapshot interval/cache20ms не продлевается wire TTL. Native не
имеет proc/cache fallback. Unknown/Unavailable при app-required -> deny/drop,
при прочих policy только явно предусмотренное поведение, без owner fabrication.

## Revoke, budgets, closure

REVOKE (104B,0FD): header64/id0; nonce32 at64; reason u32 at96;
reserved4 at100. Server sends best-effort then closes endpoint. Go закрывает
TUN duplicate и все flow; helper original освобождается штатным worker/session
cleanup. EOF/death равнозначны revoke. Network epoch/generation/invocation,
PID1 owner/property drift, unsupported proof/kernel, expiry -> revoke/failclosed.

Anchor TTL<=1s от helper proof, renew только helper после actual revalidation;
native packets/ACK не renew. One outstanding lookup, Go waiting callers<=64,
общий lookup deadline20ms; overflow -> Unavailable(Limit). No retries after
lookup timeout: endpoint retire, чтобы stale reply не стал новым ответом.
Rust owner cache64 и cold throttle не расширяются.

FD budgets: native channel1 + received TUN1; initial ATTACH exactly1FD.
Helper keeps original TUN1; app broker never receives TUN/mnt. Net/pid retained
только scoped anchor lease; copies не продлевают deadline. NS received mnt
закрывается до payload/assets, credits не освобождаются только по cgroup-empty
при наличии любых mount retainers. Preferred flow не удерживает mount вообще.

Public six RPC/caps v1, existing bundle v2, resource encoder ABI не меняются.
Global capability=false до обоих native core end-to-end actual root VM gates.
