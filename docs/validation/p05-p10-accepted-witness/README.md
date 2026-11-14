# P05/P10: accepted proof map и минимальный transport contract

Accepted source `8ea826a04a745c8916e14a4329588c75f66b683e`.
Outcome: критерии сопоставлены с прежними actual producers; найден один узкий
API gap для нового **NormalHost + owned SOCKS** witness. Production/hooks не
менялись. Новый integrated test не объявляется выполненным. Rust build не
запускался: сначала требуется решение root по контракту ниже.

## Что уже доказано и не должно повторяться

| Критерий | Существующий witness | Source/граница |
| --- | --- | --- |
| P05 proxy-only fetch, TLS, отсутствие hidden DIRECT/DNS fallback | `2026-11-11-p05-refresh-acceptance.md`, p05-native results: 2 cores × (30 Host + 32 Service/scheduler) = 124 cases; DNS и DIRECT positive controls=1, после reset hidden_direct/system_lookups=0 | Бинарники retained, SHA проверены; native matrix не повторяется. Полный accepted backend отличается от исходного manifest, новые исполнения ему не приписываются |
| P05 source/server credential rotation, empty/partial/corrupt, review/CAS/cancel | Тот же actual native witness + p05_acceptance.rs/p05_service.rs, subscription_refresh_contract.rs | Сохранение source ID/profile identity/working snapshot до approve; public Service API и реальные sockets. Это не новый key-custody proof |
| P10 actual host policy, shared 60s window, close/cancel ACK, restart interval | `2026-11-11-scheduler-windows.md`, final-privatebus.log и p10-gate | Реальные private D-Bus provider/HTTP и elapsed 60 секунд; не физические NM/UPower/energy. Proxy-lease replacement в конце отчёта отдельно назван unit/direct-call regression, не actual traffic |
| P10 scheduler backoff, bounded queue, review/stale/cancellation/persisted retry | subscription_scheduler.rs tests, subscription_scheduler_tests.rs; desktop_schedule.rs tests | Runtime + repository contracts существуют. Часть transport cases использует Fake/Counter, не actual SOCKS + NormalHost chain; чистый clock/policy input не аппаратная приёмка |
| Actual Scheduler Full → production GUI/V2 | `2026-11-12-p11-scheduler-full.md`, p11-full-132 retained ELF | Production capacity не менялась; dirty source overlay/уменьшенная test capacity отражены в отчёте. Это admission/export gate, не retry/manual-priority transport gate |
| Actual FetchError → typed scheduler cause | `2026-11-12-p11-scheduler-fetch-causes.md`, p11-fetch red-targeted/green | Один реальный SubscriptionService HTTP503 producer; 21 mapping/strict V2 exports отдельно. Не 21 actual network/TLS producers и не full P10 |

`binding.json`: canonical paths, hashes metadata/logs, focused old→accepted
source SHA comparisons и 5 retained binary hashes (все совпали). Focused source:
P05 4/7 exact, windows 13/20 exact, Full 24/28 exact, Fetch 0/1 exact.
Это число **выбранных релевантных entries**, не whole-tree/build equivalence.
У p11-fetch source hashes также есть error-context/catalog/Cargo файлы вне этого
focused счётчика. Dirty source/patch Full не выдаётся за чистый commit.
Матрицы старого source подтверждены своими reports; для changed paths не заявлено
новое accepted execution. Эти данные не переключают PLAN/acceptance автоматически.

## Один недостающий integrated witness

`NormalHost/private Secret Service + actual policy barrier → scheduled Proxy
HTTP503 → наблюдённый backoff → явный manual refresh при queued/delayed auto
work → HTTPS через owned SOCKS → partial review/cancel/source-generation stale
→ working snapshot сохранён → shutdown/closed/reopen`.

Это один focused сценарий crossing host dispatch ↔ real transport ↔ repository,
а не повтор 124-case native matrix, cross-product всех HTTP ошибок и enum causes.
Нужны positive transport control и negative control: при закрытом SOCKS endpoint
origin остаётся доступным, но origin request count не увеличивается. DIRECT не
подставляется вместо Proxy. Credentials/body — только synthetic fixtures;
evidence хранит counts/digests, а не URL/body/keys.

NormalHost создаётся `HostConfig::new`, без `with_ephemeral_key`. Собственные
private D-Bus/Secret Service/runtime запускаются только bounded owned guard;
existing runtime можно читать/копировать минимально, без cleanup чужих roots.
Source exception для loopback/private address задаётся явно существующим
SubscriptionSource API. HTTPS проверяет свою CA и IP SAN/SNI обычным TLS;
нет `accept_invalid`, trust fallback, host keyring или host network изменений.
Не вводятся clock/network/power hooks: существующие policy API/observer и
bounded snapshot barriers; logical policy simulation не выдаётся за hardware PASS.

## Почему это нельзя честно добавить одним новым external test сейчас

1. `FetchHost::with_socks_endpoint` публичен в yonik-linux/src/fetch.rs:215;
   SubscriptionScheduler::dispatch принимает actual SubscriptionFetch.
   Поэтому нижний Service/runtime transport fixture возможен уже сейчас, но
   сам по себе **не** является NormalHost dispatch/manual-priority witness.
2. В backend.rs:3340–3400 private DesktopFetch создаётся из private ProxyLease.
   При Proxy без lease возвращает UnsupportedPeerVerification; endpoint нельзя
   задать через public HostConfig или JSON. Host создаёт manual/scheduled fetch
   через DesktopFetch::new; публичные with_schedule_observer и
   set_subscription_policy не меняют transport endpoint.
3. Есть уже одобренный **private cfg(test)** ProxyLeaseFixture из FR16 disk gate:
   backend.rs:568,4014,4063 публикует `(17, own_endpoint)` в watch и привязан к
   private DiskTestInput. Он не external test-support API и не NormalHost API.
   `test_start_with_disk` наружу принимает только event-log fixture, proxy=None.
   Переиспользовать этот путь здесь значило бы выйти в запрещённый event-log
   ownership и выдать synthetic lease за новую transport authority. Это не делается.
4. Existing backend Harness/Directory используют ephemeral key [77;32]. Они
   пригодны для старых component tests, но не заменяют новый NormalHost/private
   SS persistence/custody witness. Native Running ради получения lease также
   не является обязательным prerequisite для fetch/scheduler и здесь не запускается.

Таким образом, проблема — доступ к **реальному fetch transport** из NormalHost
fixture, не отсутствие scheduler implementation и не необходимость TUN/N5.

## Минимальное предложение root — НЕ реализовано

Один nondefault `cfg(feature="test-support")` HostConfig builder для **конкретного**
`Arc<yonik_linux::FetchHost>` (условное имя `with_test_subscription_fetch`).

- Только concrete production FetchHost с bounded timeout и уже проверенным
  loopback SOCKS endpoint; не `dyn SubscriptionFetch`, callback/future готового
  response, snapshot/state/cause injection или arbitrary I/O hook.
- Используется только для subscription fetch в закрытом NormalHost fixture.
  Обычный start/CLI/JSON/default build не получают нового входа. Никакого native
  Connected, Running, ProxyLease `(17,...)`, helper/system authority не публикуется.
- FetchRoute, local_exception, validators/HWID, normal TLS и cancellation receiver
  проходят обычный FetchHost/SubscriptionService. Возврат только actual socket
  result. CAS/review/commit/persist/reconcile/manual dispatch остаются прежними.
- Не добавлять второй scheduler, clock framework, новый policy enum или трогать
  event-log/credentials/worker/compiler/P06. Если route authority требует другого
  typed решения, root уточняет **этот** контракт до production изменений.

После одобрения: один новый focused Rust 1.85.1 test/harness на принятых API;
source freeze + manifest до compile, own transient-root flock, global slot,
empty wrappers, jobs=1, без edits во время build. Один retained test ELF;
фактические socket/ACK/reopen/cancel controls, normal private SS, затем retain
source/binary/evidence и cargo clean только собственного target.

До решения root новый Rust build/fixture runtime не имеет смысла: он либо
повторит уже доказанный direct/Service-only slice, либо потребует запрещённого
нового production hook/ложного lease. Точный контракт передан вместо обхода.

## Проверки этого handoff

Только git accepted blobs, чтение четырёх обязательных reports/production/tests,
metadata/log hashes и 5 retained ELF SHA. Из 5 binary bindings mismatch=0.
Никакие retained ELF не исполнялись; private SS/HTTP/TLS/SOCKS/native/GUI не
поднимались. Нет нового runtime PASS, hardware/energy PASS или P04 promotion.
P04 source ledger и его missing observed trace boundaries остаются неизменными.
Toolkit campaign не опрашивалась, её process/session/immutable snapshot не трогались.
