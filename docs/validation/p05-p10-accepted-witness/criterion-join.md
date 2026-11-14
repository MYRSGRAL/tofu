# P05/P10: criterion join accepted41 / frozen199

Только source/data review. Accepted: `41e0e8cb92b266b8053edd241f0531662bda5298`.
Frozen PR199: `7b175e4b9e207b57eec55b75a8172b586ee2c3c5` (implementation
`6e73f01958d677cb4fec4257ccf9878723bcda4f`). Этот документ не новый execution,
не общий PASS и не решение независимого review. Exact сравнение —
[criterion-join.json](criterion-join.json); старые bindings/archives не менялись.

## Исходные строки и join доказательств

Исходные строки взяты из `docs/PLAN.md` на accepted41 (P05/P10), конкретизированы
исходной таблицей `2026-11-11-p05-refresh-acceptance.md` и контрактом
`2026-11-11-scheduler-windows.md`. Accepted PLAN всё ещё говорит «Частично»;
более поздние scoped proofs не переписывают эту строку автоматически.

| Original criterion | Исторический proof / source invariant | Что добавляет frozen199; граница вывода |
|---|---|---|
| P05 proxy-загрузка без hidden DIRECT | P05 native: 2 cores × (30 Host + 32 Service/scheduler)=124 cases; DIRECT/DNS positive controls, затем zero hidden DIRECT/system lookups. Default DesktopFetch требует lease, FetchHost не переключает Proxy на Direct. | NormalHost + concrete production FetchHost, actual HTTPS через свой SOCKS; после закрытия SOCKS origin health доступен, новых subscription requests нет. Не native core/Connected/lease admission, не domain DNS-over-TCP или native fence rerun. |
| P05 credentials, source/profile identity | Историческая матрица: source-token rotation/ACK и native manual/scheduled refresh; rename сохраняет ID, новые server credentials меняют fingerprint/ID. | SourceChange generation=2 и отказ approve старого report; не новая rotation/rename/HWID matrix. Старые identity/Service producer files не изменены между accepted41/frozen199; исторический whole backend не объявляется exact. |
| P05 empty/partial/corrupt, working snapshot | Исторические native full/empty/partial/corrupt и Service CAS/admitted-commit controls: snapshot сохраняется до approve, cancel после admitted commit не откатывает результат. | Clean auto-commit baseline revision=1; partial 1 accepted/1 rejected, discard, cancel и source-change не заменяют рабочую revision; close/reopen с actual private SS. Empty/corrupt/second-Service CAS не повторены. |
| P05 отмена/typed failures | Старые Pending/Review и pre-edit socket-close/ACK barriers, same-endpoint generation revoke до network, Stop во время SOCKS handshake. | Held HTTPS body cancel с actual token и Stop закрывают socket; HTTP503 через scheduler публикует typed cause. Override Stop использует None→None notification: не доказывает Some(old-generation)→None→Some(new-generation) из Running. |
| P10 исполнение через Service, persisted due/backoff/manual | Переносимый bounded scheduler, Service/repository и DesktopHost manual dispatch/reconcile producers; unit/model время не hardware. | Overdue через существующий persisted metadata API, logical online/visible policy, actual scheduled HTTP503; после close failures>0/due future. Manual не ждёт future due; successful discard сбрасывает backoff. Не competing live-auto queue/preemption matrix. |
| P10 UI/network/power и фоновые окна | Private-bus historical gate: policy barrier, hidden fetch, held socket cancel ACK, persisted 48h/restart, actual 60s window, retained review/nonextension. Unknown/metered/power-saver запрещают фон. | Новый test отключает system policy provider и использует существующий logical policy API; не повторяет private-bus signals/window duration и не свидетельствует о физическом питании. |
| P10 bounded admission/stale/export | Full/fetch-causes reports: actual Service failure/Full producer → typed context/V2; enum export matrix отдельно. Window replacement shutdown/reload/interval barriers — исторический unit/direct-call regression, прямо не actual lease traffic. | Новый run соединяет transport failure с NormalHost/manual/review/persist, но не выдаёт authority и не закрывает native replacement/window traffic seam. |
| P10 полная энергоприёмка | В accepted PLAN остаётся отдельной; software/private bus evidence не физическое energy измерение. | Не измерялась; NULL/не заявлена. Не добавляется как prerequisite разрешённому fetch/scheduler test. |

## Точная граница override

`HostConfig::with_test_subscription_fetch(Arc<FetchHost>)` только nondefault
`test-support`. В ветке DesktopFetch он берёт endpoint из concrete FetchHost,
минуя default `self.lease` endpoint/DNS assembly. Route остаётся Proxy; actual
FetchHost по-прежнему проверяет source/local_exception, TLS, SOCKS, validators,
HWID и cancellation. Никаких готовых Responses/Success или fake NativeRunning.
Результат проходит прежние Service/typed failure/review/commit/persist producers.

Default path требует `Some((generation, endpoint))`; Running публикует lease
лишь при Connected, !cancel и незавершённой task. Изменение lease отзывает
захваченный fetch, manual pending и scheduled owner; replacement загружает
persisted schedule. Override слушает тот же watch, но positive SOCKS результат
сам по себе не доказывает происхождение endpoint из actual Running, совместимость
с native DNS path или lifecycle authority. Отсутствие override в accepted41 —
ожидаемая разница ветвей, не заявленная интеграция PR199 в accepted.

Между двумя refs byte-exact: `fetch.rs`, `subscriptions.rs`,
`subscription_scheduler.rs`, `subscription_policy.rs`, `desktop_schedule.rs`,
`yonik-subscription/src/{lib,scheduler}.rs` (7 файлов). Backend whole-file различен:
в accepted есть catalog changes, в frozen199 — transport support. Отдельно
byte-exact четыре отрезка: default fetch tail, Running→proxy_session watch,
manual completion/commit/persist и три регрессии
`domain_proxy_without_config_fails_before_socks_lookup`,
`reconnect_same_endpoint_revokes_pinned_generation_before_network`,
`stop_cancels_proxy_waiting_for_socks_handshake`.
Это source preservation, не доказательство нового исполнения accepted41.
Исторический README уже фиксирует несовпадения whole-source manifests; equality
двух текущих refs не превращает старые ELF в current execution.

## Минимальный следующий executable seam

Конкретная недостающая композиция, если root требует закрыть integrated
production-lease lifecycle: **actual Connected Running lease при scheduled held
fetch → отзыв/смена поколения (в том числе тот же endpoint) → cancellation ACK
старого socket → replacement scheduler с прежним общим window deadline и
persisted due → отсутствие старого review/commit**. Положительный control —
actual request с новым lease; negative — старое поколение не подключается и
не публикует результат. Ни fixture `(17, endpoint)`, ни новый override такого
control не заменяют. Существующий P05 matrix доказывает настоящие core/Proxy
requests, unit regressions доказывают revoke fence, private-bus gate доказывает
window; это не один joint native lifecycle witness.

Это bounded proposal для отдельного разрешения root, не найденный дефект и
не требование повторить 124 cases. Сначала выбрать уже существующий native
LocalSession harness/API и зафиксировать source/binary; без новых public
transport/authority hooks. Если существующий harness не позволяет наблюдать
matching ACK/window boundary, нужен отдельный минимальный API proposal до edits.
Текущий handoff ничего не запускает. Кроме этого seam, новый original-criterion
algorithm invariant по связанным source boundaries не обнаружен; credential,
empty/partial, identity и retry matrices остаются scoped historical proofs, а
physical energy — явно отдельный непокрытый пункт P10. Root/review решают,
достаточен ли такой join для narrow promotion, без global acceptance.
