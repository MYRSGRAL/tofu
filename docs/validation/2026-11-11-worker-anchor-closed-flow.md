# Closed flow WorkerChannelAnchor: кандидат на согласование

База: `2df0b07dd67ab152d1d4d21f295837b78403d928`.
Это спецификация proposed delta, не доказательство positive admission.

1. Helper в canonical system bus фиксирует PID1 unique owner и actual GetAll
   fixed Run service. До выдачи challenge проверяет root-owned ExecStart image,
   User/Group, MainPID, pidfd/starttime, InvocationID, ControlGroup. Проверяет
   RestrictNamespaces как uint64 allowed-mask == 0, SystemCallArchitectures
   как массив единственного native ABI, NNP, NET_ADMIN-only caps. Отдельно
   требуется поддерживаемый kernel enforcement, проверенный в exact invocation.
2. Отдельный negotiated private transport предшествует `receive_payload` в
   `worker::serve_stream`. Legacy bundle v2 не меняет значение. Check/version
   не участвуют. Bootstrap до config/native exec становится nondumpable;
   неизвестный child и replacement image не получают authority.
3. Helper создаёт один challenge: nonce32, runtime-config-digest32, session16,
   generation u64, invocation16; version/reserved однозначны. Полный frame
   NS_WITNESS v1 не более 256 байт, deadline 1 секунда от challenge, без продления
   после получения. Только один outstanding challenge на invocation.
4. Attested Rust bootstrap открывает фиксированные self net/pid/mnt namespace FD
   и отправляет ровно три SCM_RIGHTS. Helper требует SCM_CREDENTIALS каждого
   сообщения, actual fixed UID/PID, живой pidfd/starttime и canonical cgroup;
   принимает с CMSG_CLOEXEC, проверяет nsfs magic и NS_GET_NSTYPE по слотам.
   Неизвестные/дублированные ancillary данные, лишние FD и truncation отвергаются,
   все полученные FD закрываются через OwnedFd. Caller PID/path/proof bool
   и самостоятельно присланные inode numbers не являются authority.
5. До разрешения WorkerPayload/assets helper закрывает mnt FD. Bootstrap закрывает
   исходные три FD до подтверждения готовности принять payload. В broker никогда
   не передаётся mnt FD. В anchor остаётся только opaque mnt identity и доказанная
   невозможность сменить namespace; net/pid FD принадлежат ограниченному lease.
   ACK закрытия привязан к той же challenge и trusted bootstrap phase, не caller.
6. Только helper mint-ит OpaqueVerifiedWorkerUnit и WorkerChannelAnchor после
   повторной проверки owner/invocation/properties/liveness. BrokerLookupView
   независимо фиксирует собственный lookup scope и holder coverage. Anchor
   не owner witness и не предоставляет setns.
7. EOF/death/expiry/owner change/network epoch/invocation/property drift/revoke
   прекращают admission, закрывают retained FD. Неизвестная архитектура или
   недоказанный syscall filter дают UnitProofGap. Cache 20ms не расширяется.

## Предлагаемые места изменений

- `helper/systemd_worker.rs`: actual contract, opaque unit proof, challenge custody.
- `worker.rs`: optional private negotiation до `receive_payload`; trusted opener
  и закрытие всех исходных namespace FD до assets.
- `helper/service/session_host`: только передача opaque anchor и revoke lease.
- `attribution/broker/worker_anchor.rs`: production receive/promotion, net/pid
  custody, mount identity без retained FD.
- broker context: отдельный BrokerLookupView; lookup не использует worker mnt.

Root согласовал этот flow в текущей сессии: helper подтверждает закрытие всех
transient mnt FD до допуска payload. Согласование не заменяет unit/kernel proof. Public 6 RPC/caps v1, resources store, encoders, pins,
Go/backend/GUI/credentials/packaging не меняются.

## Обязательные actual gates

Production helper + controlled unit, foreign worker UID / broker user, прежние
NNP/caps. Positive verified bootstrap + Rust broker lookup и negatives forged FD,
credentials/PID/invocation/generation/property drift/revoke/cleanup. В той же
invocation нужны unshare/setns/clone3 negative probes с различимыми positive
controls: обычный capability EPERM не доказывает seccomp enforcement.
Ни unit text, ни успешный transport, ни этот документ не закрывают FR20.

## Предлагаемая раскладка NS_WITNESS v1

| Смещение | Размер | Поле |
| --- | --- | --- |
| 0 | 8 | magic/version `YNKNS001` |
| 8 | 32 | helper nonce |
| 40 | 32 | runtime config digest |
| 72 | 16 | helper session |
| 88 | 8 | generation, big endian |
| 96 | 16 | actual unit invocation |
| 112 | 16 | reserved, только нули |

Итого 128 байт. Challenge и witness различаются transport direction, не caller
PID/UID. На witness ровно три FD в net/pid/mnt порядке, credentials обязательны.
Отдельное bootstrap closed-ACK должно связывать тот же challenge. До принятия
ACK helper сам закрывает полученный mnt FD; bootstrap до отправки ACK закрывает
исходные три FD. На timeout/drop ACK payload не отправляется, cleanup следует
существующим reserve/cgroup правилам009. Отправленные SCM_RIGHTS не требуют
дополнительного удержания исходного FD после успешного sendmsg.

Legacy detection не может потреблять первые байты bundle v2. Negotiation требует
нового явного offer marker и отдельного SEQPACKET channel, переданного по existing
private stream; recv peek выбирает legacy или offer. Нельзя автоматически
fallback в legacy после начала negotiated exchange: любой отказ закрывает канал.
Эта раскладка пока не реализована и не является обещанием compatibility.
