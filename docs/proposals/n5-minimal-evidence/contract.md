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

Сравнивались: fixed16+old3; те же наблюдения с Seccomp_filters; trusted
completion receipt от точки установки. Первые два дешевле, но не различают
все разрешённые source-path skips. Поэтому предлагается третий путь только
после отдельного разрешения. Сейчас ни один receipt не существует и verifier
обязан вернуть EvidenceUnavailable, а не выпустить opaque proof.

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
Даже fixed16+old3 с ожидаемыми errno может совпасть при внешнем фильтре или
capability/argument denial; universal paired matrix этого контракта не требуется.

## Требуемый источник completion receipt

Будущий trusted executor должен до exec immutable worker сохранить успешные
результаты каждой ожидаемой операции native-only и namespace policy, включая
clone3, argument-independent setns и unshare/clone MASKED_EQ known8
`0x02000000,0x08000000,0x40000000,0x00020000,0x20000000,0x10000000,0x04000000,0x80`
(OR `0x7e020080`), затем actual successful loads. Любой skip, неизвестная
architecture/bit/implementation, missing operation или nonfatal error => отказ.
Native-only исключает i386 и x32 по reviewed exact libseccomp semantics; native
fixed16 не заменяет это доказательство. Success receipt не равен count summary.

Receipt должен происходить из reviewed actual-used installation path, а не из
worker bool, property, заявленного profile или новой подписи этих значений.
Защищённая связь executor→pending invocation и custody receipt — отдельный
обязательный review implementation. Если installed systemd этого не выпускает,
поле receipt digest запрещено синтезировать из успешного запуска или journal.
Bounded instrumentation/receipt proposal в installed executor — вариант нового
user scope, не скрытая правка в этой ветке. Новые roundtrip в worker не нужны:
receipt correlation helper делает по invocation в доверенном installation path.

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
Receipt digest ссылается на удержанный trusted completion record этого же
invocation/task, включая operation manifest; helper сверяет bytes и источник,
не верит worker pointer/hash. Count fields диагностические, не admission shortcut.
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

`verify_pending_report(pending, owner_lease, report_frame, trusted_receipt,
verified_transport, retained_net_pid, exact_bound_tun) -> Result<VerifiedTransaction, Error>`.
Все arguments кроме frame — закрытые owning types. Verifier потребляет pending
один раз; bytes не создают VerifiedTransaction. Для helper — полный canonical
unit/property/hash observation до/после await; issuer сопоставляет nonce/digest/
session/generation/invocation из challenge с YNKROLE1 и owner epoch. Nonce один,
из trusted authorization/challenge RNG; не второе случайное значение.

Порядок отказов при одновременных проблемах (локальная диагностика, без утечки
содержимого неаутентифицированному peer):

1. Cancelled/Revoked, затем Expired (Instant >= existing challenge deadline).
2. Transport: EOF, TRUNC/CTRUNC, размер, SCM/peer/FD custody; закрыть полученные FD.
3. Protocol: version/reserved/closed integer bounds/vector length/duplicate frame.
4. Binding: task/server PID/starttime, nonce/digest/session/generation/epoch/invocation.
5. Profile: tuple/provenance/actual-used/property observations/known8/native semantics.
6. EvidenceUnavailable или IncompleteApplication (missing receipt / recorded skip).
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

Стадии: (1) root review этого design и receipt feasibility; (2) после назначения
scope pure parser/consuming-verifier unit fixtures и mutation check без production
caller; (3) отдельно разрешённый trusted receipt producer и actual installed
campaign fixed16+old3 с existing challenge/YNKROLE1, negative skip cases в
изолированном test-only emitter; (4) review actual evidence и lifetime/cancellation;
(5) отдельное разрешение kernel issuer, затем Backend factory/callers; (6) actual
canonical Runtime/full monitor acceptance. Каждая стадия закрыта до предыдущей;
compile/unit proof не перескакивает runtime gate.

Root может принять layout, приоритет ошибок, lifecycle, test matrix и назначить
последующий bounded implementation/review; root не может превратить compiled16
или profile bool в evidence. Только user может расширить текущий grant:
разрешить конкретную disposable installed-systemd campaign exact supported tuple
с Runtime16+old3 pre-payload, bounded trusted executor receipt implementation и
его invocation correlation; отдельно разрешить callers/factory. Нужны заранее
reviewed executor diff, task/UID/unit inventory, stop/cleanup plan и список outputs.
Existing compile grant разрешает проверку сборки source170, но не syscall execution,
unit activation, cross-UID, VM или installed executor replacement. Здесь такие
действия не выполнялись, pending user question не считается отвеченным.
