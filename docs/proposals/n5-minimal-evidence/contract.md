# N5: минимальный evidence contract (предложение, НЕ ПРИМЕНЕНО)

Статус: DESIGN ONLY; kernel permit и factory CLOSED. Зависимая база PR190:
`36ced8e1d4db1218a7547b4ae4d141b112708a15`. Actual bootstrap читается отдельно
через frozen Git `302f8bf`, а не считается реализацией этой ветки.
PR192 `acc01ade70b301cf5d1f132318b3d20131fde8e7` сохранён в прежней ветке.
Существующие refs не переписываются. Переданные root сокращения cfa/f6 и
unit proof PR188 `47997a` здесь не разрешены в полные SHA: это ссылки handoff,
а не независимо перепроверенный runtime результат. PR188 CLEAN unit proof
не закрывает actual canonical Runtime/full monitor coverage: они OPEN.

Решение R2 37d55d49 из root
`local/orchestration/proposals/n5-minimal-contract/supported-profile-sufficiency-decision.json`
снимает standalone full prune/link proof как admission gate в принятом
trusted installed systemd/libseccomp TCB. Не требуется новый whole-library RE.
Сохраняются exact tuple, provenance, actual-used bytes, known8 и native ABI deny.
Никакое поле ниже само по себе не является authority.

## Выбор минимального discriminant

Рассматриваются две альтернативы: совокупность trusted exact profile,
actual old3/New16, адресного unfiltered control при конкретной неоднозначности
и обоснованного count inventory; либо trusted completion receipt от точки
установки. Ни одна альтернатива пока не подтверждена actual evidence. Receipt
не является обязательным расширением архитектуры. До доказательства полноты
выбранной совокупности verifier возвращает EvidenceUnavailable, без opaque proof.

| Наблюдение | Что различает при доказанных предпосылках | Чего не доказывает |
| --- | --- | --- |
| old3 v3 до payload | unshare mask, setns(-1,0), clone3(NULL,0) в пределах принятого tuple/custody | Не покрывает legacy clone/native ABI; числовой EPERM/ENOSYS возможен из другого ограничения |
| fixed16 source170 | 8 unshare и 8 raw legacy clone selectors, только native x86_64 | Сейчас compiled only, не actual; clone(time) не создание time namespace; нет i386/x32 вызовов |
| Seccomp=2 | task находится под seccomp filter | Нет идентичности/числа/полноты нужных политик |
| Seccomp_filters=C | При известном baseline B и точном максимуме M можно обнаружить missing load | Не видит содержимое правила, источник фильтра, masked semantics либо ABI deny |
| trusted completion receipt | Все необходимые rule/arch operations и оба требуемых load завершились в этом executor | Требует actual-used emitter, защищённую доставку и точный invocation; подпись декларации не заменяет эти условия |

Для count-доказательства нужно отдельно вывести inventory всех путей загрузки
точного unit/profile: inherited baseline B для того же task до установки;
максимум K_i для каждого источника, один проверяемый источник на слот,
отсутствие дополнительных загрузчиков и межпоточной подмены. Тогда
C=B+sum(K_i), при каждом actual_i<=K_i, влечёт actual_i=K_i.
Это доказывает количество загрузок, не полную политику. Если известен только
интервал Bmin..Bmax, то C=Bmax+sum(K_i) имеет тот же вывод лишь при доказанном
верхнем пределе Bmax и отсутствии посторонних loaders; прочие C неоднозначны.
Число K не равно автоматически двум: namespace loop проходит local architectures,
плюс unit имеет другие seccomp настройки. Здесь ни B, ни K не измерялись и
конкретное число не выдумывается. Недоступный baseline => count gate unavailable.

Безопасный будущий reader читает bounded `/proc/<verified-tid>/status` (<=64 KiB),
ровно по одному Seccomp и Seccomp_filters, unsigned decimal без overflow;
pidfd/starttime/task identity проверяются до/после. Само чтение не исполняет probe.
Helper после exec не может восстановить pre-install baseline из текущего count.
Read error/duplicate/missing field => unavailable. Не читать произвольный caller PID.

Конкретный контрпример достаточности count: в systemd namespace path ошибка
clone3 rule_add логируется, затем r перезаписывается успешным setns rule_add;
полный mask loop и load могут пройти. Count не меняется, clone3 deny отсутствует.
Ошибки setns и unshare/clone loop обычно пропускают load этой architecture;
при строгом максимуме count их выявит, без него — нет. Arch path также может
вернуть 0 при nonfatal load error или blocked_new=false; локальное состояние
blocked architectures не является свидетельством реально установленного фильтра.
Основание: source snapshot seccomp-util.c 1378–1515, 2016–2088 и bounded shipped
bytes PR190. Ни success-return, ни отсутствие warning в журнале не receipt.
Этот пример опровергает только count alone. Он не исключает совокупность
trusted profile + actual old3/New16 + адресный control + count inventory.
Чтобы отвергнуть такую совокупность, нужно предъявить два конкретных сценария
exact tuple с одинаковыми ВСЕМИ допустимыми наблюдениями, но различной полнотой
политики. Общая ссылка на внешний filter/capability/argument denial такой парой
не является. Требование universal paired matrix не вводится.

## Альтернативный completion receipt и решение по exact tuple

Первый путь требует анализа различимости каждой существенной ветви именно для
поддержанного tuple: profile semantics/provenance и actual-used bytes, результаты
наблюдений с единым pending binding, обоснованный count inventory и только
необходимый адресный control. Новые controls и Runtime16 не разрешены этим
документом. Ни count, ни errno, ни подпись profile отдельно proof не создают.
Конкретный sufficient predicate выбирается отдельным root review; сейчас он не
реализован и полнота наблюдений не объявлена доказанной.

Если для этого пути установлена конкретная оставшаяся неразличимость, можно
отдельно согласовать receipt. Тогда нужен определённый producer/callgraph
actual-used installation path и защищённая передача record с invocation/task
binding. Ссылка на «будущий trusted executor» или его digest этого не заменяет.
Record должен подтверждать каждую необходимую rule/arch operation, включая
clone3, unconditional setns, known8 mask rules и оба successful load; recorded
skip/error не принимается. Нужны конкретный reviewed diff и user grant до
instrumentation installed executor. Подпись worker bool/property не receipt.
Сейчас такой producer отсутствует; это не причина автоматически отвергать
первую альтернативу или синтезировать receipt из запуска/journal silence.

Known8: `0x02000000,0x08000000,0x40000000,0x00020000,0x20000000,0x10000000,0x04000000,0x80`
(OR `0x7e020080`). Native-only exclusion i386/x32 и полная namespace semantics
остаются обязательными. Ни одна альтернатива не ослабляет этот результат.

## Bounded report и carrier

[report-layout.json](report-layout.json) — точный proposed binary layout 512 bytes,
все integers big-endian, без varlen и optional tails. Reserved строго zero.
Это layout specification, НЕ wire patch. В frozen 302f8bf anchor_exchange FRAME=128,
v3 old3; YNKROLE1 FRAME=192. Они не принимают новый frame автоматически.
Будущая отдельная version negotiation меняет ровно existing report slot на v4,
не добавляет request/ACK/FD/roundtrip. Challenge и role record остаются прежними.
Legacy/v3 в состоянии expected-v4 => reject, fallback отсутствует.

Report несёт полную challenge binding, epoch и verified task identity; digest
profile привязывает immutable allowlist manifest exact kernel build/ABI/systemd/
shared ELF/libseccomp/worker/unit hashes, provenance и actual-used binding.
Slot receipt_digest остаётся optional reference в proposed layout: все нули
при отсутствии receipt. Ненулевые bytes допустимы только как lookup key для
удержанного authenticated record того же invocation/task; helper сверяет
сам record и источник, не верит worker pointer/hash. Пустой slot сам по себе
не является ошибкой при достаточном reviewed совокупном evidence первого пути.
Count fields диагностические, не admission shortcut.
Old3+fixed16 vector имеет фиксированный source-defined порядок; namespace order
ровно из массива выше. Старые 3: unshare,setns,clone3. Следующие 16: interleaved
unshare,clone-selector для каждой namespace. Errno bounded 1..4095, ожидаемые
значения задаёт reviewed tuple, не сообщение. Успех syscall (0) => reject.
Сейчас actual vector19 недоступен без Runtime16; существующий v3 не повышается.

Аутентификация — existing private seqpacket + per-frame SCM credentials,
canonical pending unit PID/UID, live pidfd/starttime и retained namespace FDs;
MAC/signature не добавлены. Без этой custody даже perfectly formatted frame
не trusted. Channel не доступен payload до завершения bootstrap; никаких новых
caller raw-FD/env constructors. Поля task pid/start и server pid/start различны.

## Consuming verifier (сигнатура только как proposal)

`verify_pending_report(pending, owner_lease, report_frame, retained_evidence_inputs,
verified_transport, retained_net_pid, exact_bound_tun) -> Result<VerifiedTransaction, Error>`.
retained_evidence_inputs удерживает exact profile и полученные по trusted
каналам observations/control/count inputs; receipt может отсутствовать. Это не
готовый permit, bool или caller-supplied predicate: verifier обязан проверить
reviewed sufficient predicate для exact tuple либо вернуть EvidenceUnavailable.
Все arguments кроме frame — закрытые owning types. Verifier потребляет pending
один раз; bytes не создают VerifiedTransaction. Для helper — полный canonical
unit/property/hash observation до/после await; issuer сопоставляет nonce/digest/
session/generation/invocation из challenge с YNKROLE1 и owner epoch. Nonce один,
из trusted authorization/challenge RNG; не второе случайное значение.

Порядок отказов при одновременных проблемах (локальная диагностика, без утечки
содержимого неаутентифицированному peer):

1. Cancelled/Revoked и уже известный stale owner/network/invocation canonical
   pending context проверяются до framing, как в существующем bootstrap; затем
   Expired (Instant >= existing challenge deadline). Wire-malformed не маскирует
   уже известный отказ trusted context. Новые UI/error codes не вводятся.
2. Transport: EOF, TRUNC/CTRUNC, размер, SCM/peer/FD custody; закрыть полученные FD.
3. Protocol: version/reserved/closed integer bounds/vector length/duplicate frame.
4. Binding: task/server PID/starttime, nonce/digest/session/generation/epoch/invocation.
5. Profile: tuple/provenance/actual-used/property observations/known8/native semantics.
6. EvidenceUnavailable при недостаточном полном evidence выбранного пути;
   IncompleteApplication при достоверно recorded skip. Отсутствие optional receipt
   не является самостоятельным отказом, если первый путь доказан достаточным.
7. ObservationMismatch; затем Consumed при попытке повторного consume.

Перед каждым await, после него и атомарно при consume/выдаче двух scoped ролей
повторить revoke/expiry/live identity. Deadline — исходный helper monotonic
challenge Instant (сейчас 1s); wire не задаёт и не продлевает deadline. Если
future receipt не успевает, нужен отдельный review бюджета, не автоматический retry.
Shared revocation lease охватывает report, Session-role и Run-role. Epoch change,
worker/helper exit, cancellation, expiry, health failure или drop pending отзывают
все неиспользованные роли; blocking late result уничтожается. Один concurrent
consume победитель, без Clone, serialization и bool permit. Непосредственно перед
exec/ATTACH снова live/revoke check. Existing role ACK подтверждает custody,
не kernel policy. После stop timeout/nonempty cgroup attribution revoked сразу,
queue/journal/custody удержать до confirmed stop; не запускать замену автоматически.

## Negative matrix и rollout

[negative-matrix.json](negative-matrix.json) задаёт reviewable parser/verifier
fixtures; это test specification, не выполненные Rust tests. Ни одна fixture
не является runtime evidence. Для каждого reject: zero permit, revoke pending,
закрыть received FD, не отправить payload; retained owner resources по stop rule.

Стадии: (1) root review design и exact-tuple различимости совокупного evidence;
(2) назначенные private parser/consuming-lifetime unit fixtures без production
producer/callers; (3) отдельно разрешённая actual campaign, только необходимые
controls и Runtime16 при отдельном user grant; receipt instrumentation только
если её необходимость обоснована и producer/diff отдельно согласованы;
(4) review actual evidence/lifetime/cancellation; (5) отдельный kernel issuer,
затем Backend factory/callers; (6) actual canonical Runtime/full monitor acceptance.
Compile/unit proof не перескакивает runtime gate.

Root может назначить private parser/lifetime API и unit fixtures после design
review. Compiled16/profile bool/синтетические inputs не становятся actual evidence.
Только user может расширить текущий grant на конкретную Runtime16/installed
campaign и новые controls; instrumentation executor требует собственного
обоснованного diff/grant и не является обязательной частью первого пути.
Callers/factory также остаются отдельным закрытым scope. Нужны заранее reviewed
unit/task/UID inventory, stop/cleanup plan и outputs. Existing apply/compile grant
не разрешает syscall execution, unit activation, cross-UID, VM или замену installed
executor. Pending user question не считается отвеченным.
