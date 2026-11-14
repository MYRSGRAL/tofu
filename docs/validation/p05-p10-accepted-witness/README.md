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

## Выполненный actual witness, 2026-11-14

Предложение выше историческое: root одобрил конкретный transport-only builder,
реализация — `6e73f01958d677cb4fec4257ccf9878723bcda4f`.
`HostConfig::with_test_subscription_fetch(Arc<FetchHost>)` существует только под
nondefault `test-support`. Только subscription factories получают concrete host;
preset fetch и обычный default/CLI/JSON остаются на прежнем пути. Ответ не
инжектируется: production FetchHost выполняет настоящие SOCKS/TLS/route/
local_exception/validators/HWID/cancellation; Service/review/commit/persist и typed
context publication прежние. Transport input не создаёт native lease/Running.
Stop продолжает отзывать watch, включая fixture None→None.

Новый [actual-binding.json](actual-binding.json) отделён от исторического
[binding.json](binding.json). Старые 124 cases и window/full/fetch-causes reports
не запускались повторно и не присвоены новому backend. `p11-integrated-candidate`
получил только запись source update/history с `review_pending=true`.

| Критерий | Новый actual scope и наблюдение |
|---|---|
| NormalHost/key custody/persist | Обычный Host без ephemeral key, свой bus + Secret Service, encrypted repository открывается тем же actual SS key; закрытие/reopen сохраняет revision=1/рабочий профиль. Не доказательство host keyring пользователя. |
| Scheduled failure/backoff | Через существующий metadata API выставлен overdue до старта; через существующий policy API заданы online/visible. Production scheduler → HTTP503 через SOCKS → typed `scheduler_service_fetch_failed`, scheduler generation >0; после close metadata failures>0 и due в будущем. Не clock/hardware injection и не повторное window proof. |
| Manual priority | При persisted future due явный refresh получает actual HTTPS partial report за <900ms; clean baseline auto-commit, partial accepted=1/rejected=1 остаётся review, revision=1. После discard persisted failures=0 по существующему contract. |
| Cancellation/revoke | В двух отдельных held HTTPS body операциях cancel с actual refresh token и Stop закрывают socket до fixture deadline, busy=false/pending=null; профили не заменяются. |
| Старый report/source CAS | После partial source_change(expected_generation=1) подтверждает generation=2/error=null; попытка approve старого review отклонена, pending=null/revision=1. Это блокировка старого report после изменения source; исторический Service CAS matrix не переприсвоен. |
| No hidden DIRECT | После reopen выбран persisted source route=Proxy; SOCKS listener закрыт, отдельный normal-TLS health control доказывает доступный origin; refresh FAIL, subscription origin count не растёт. |
| Cleanup | Final result: 6 origin subscription requests / 6 SOCKS requests; обе held операции observed closed. Три own session child (test/private SS/bus) завершены/reaped. Дополнительный реальный child игнорирует TERM: Guard KILL→reap, exit=-9, <6s. |

`crates/yonik-desktop/tests/p05_p10_actual_priority.rs` — один ignored integration
scenario; Python runner поднимает только owned loopback HTTPS/SOCKS/control,
свой HOME/runtime/bus/SS. TLS: отдельный synthetic CA с keyCertSign, serverAuth
leaf + IP SAN, доверие только fixture CA через SSL_CERT_FILE; TLS проверки не
отключаются. Private address — явный source local_exception=true. Runtime взят
read-only из прежнего A4 cache с проверкой архивных SHA, минимальная closure
скопирована в собственный evidence prefix; нет установки или загрузки пакетов.
Пароли/ключи/URLs не сохраняются в evidence, temporary synthetic keyring удалён.

Внешний постоянный evidence root:
`/home/myrs/projects/yonik/local/evidence/p05-p10-actual-20261114-v1`.
`execution-binding.json` SHA256:
`543e3f9b81f8194aca3f85776fa05d5b3a72454ea45b4a61c71c79c7d0ce917c`.
Final ELF `p05_p10_actual_priority-v5` SHA256:
`9e2899d5c00d928e00d3b883e161e1e48226630c437188555573a3b02c8e1319`.
391 source files проверены по frozen manifest и содержимому source archive;
это Cargo/crates inputs и два новых harness files, не полный root tree.
`source-before-build-v5` фиксирован до compile; final archive отличается только
Python formatting cleanup receipt, native inputs byte-exact. Команды/логические
ограничения — внешний `commands.json`, хеши logs/runtime/artifacts в binding.

Финальная фактически выполненная команда (для повтора нужен новый output path):

```sh
python -B crates/yonik-desktop/tests/p05_p10_actual_priority.py --binary /home/myrs/projects/yonik/local/evidence/p05-p10-actual-20261114-v1/p05_p10_actual_priority-v5 --runtime /home/myrs/projects/yonik/local/evidence/p05-p10-actual-20261114-v1/runtime --output /home/myrs/projects/yonik/local/evidence/p05-p10-actual-20261114-v1/run6
```

Финальные проверки Rust 1.85.1: actual integration 1 PASS; lib unit 113 PASS,
1 ignored, 0 FAIL; clippy test-support all-targets `-D warnings` PASS;
no-default-features lib check PASS; rustfmt и ruff check/format PASS.
Один own Cargo target и dependency graph, jobs1/global slot/root-directory flock,
empty wrappers; incremental пересборки уточняли тестовый сценарий и TLS fixture.
Первые попытки не PASS: clean auto-commit ошибочно ожидался как pending,
backoff ошибочно проверялся после успешного discard, Python strict TLS control
обнаружил missing CA keyUsage. Все failures сохранены отдельно; assertions и TLS
не ослаблены. Final runner выполнен заново, не объявлен PASS по старому run.

Final ELF/source/commands сохранены до `cargo clean` (4722 files/2.3GiB),
собственный transient root удалён после завершения build sessions и owned child.
Host load не измерялся; фоновые P01/P02/прочие workers не опрашивались и не
менялись, elapsed не performance/energy acceptance. Нет native engine/TUN/N5,
GUI/hardware power PASS, полного P05/P10/Linux acceptance или P04 promotion.
Результат готов для независимого narrow review, не заменяет решение root.


Дополнительный P11 static gate: 2 source freshness/history tests PASS. Полный
исторический модуль: 5 PASS / 1 ERROR, поскольку отсутствует старый
`local/validation/desktop-reviewed-integration/source-binding.json` (также нет
в canonical `/home/myrs/projects/yonik/local/validation/desktop-reviewed-integration/source-binding.json`).
Это отдельный missing historical artifact, не FAIL нового actual witness;
чужие доказательства/paths не подменены. Логи `p11-freshness-check.log` и
`p11-ledger-check.log` сохранены в новом evidence root.

### Historical missing: original evidence найден, 2026-11-14

Предыдущий ERROR описывает отсутствие в task worktree, а не утрату original.
Bounded reconciliation по canonical paths #172/#174 нашёл исходные файлы в
`/home/myrs/.t3/worktrees/yonik/t3-8fef14a4-df61cef1986fac0c/local/validation/desktop-reviewed-integration/`;
копии по #174 path `t3-4a4c738e-40d1ff75c85cc1eb/local/validation/desktop-reviewed-integration/`
byte-identical. Root prefix `/home/myrs/projects/yonik/local/validation/desktop-reviewed-integration/`
отсутствует; широкого поиска не было.

Проверены original source `538ff996507ccc2566d9330e5148e7d4d1767587`, 7 binding
blobs, 10 historical desktop ledger blobs, ancestry через source172 до accepted
base и task head. Все 7 обязательных PASS names присутствуют ровно по одному;
original log: 176 PASS / 0 FAIL / 4 ignored. SHA сверены с manifest #174:

- `source-binding.json`: `58ce5e2f28dc21ae6ba2facb1313202aee5dcbae9c6f5d4984f8325b69eff227`;
- `desktop-tests.log`: `84ba02851f1662b663b14ea756a0a52efa6c96ea3fc509fcbc83527e8e52aab2`.

Только эти два public sanitized original файла скопированы byte-exact в own
`local/validation/desktop-reviewed-integration/`, без нового JSON вместо original
и без изменения logs. Это ignored local evidence, не новые tracked generated
artifacts. Полный существующий pure-data P11 gate: **6 PASS / 0 FAIL / 0 SKIP**,
включая missing/stale negative controls. Проверка не ослаблялась.

[Reconciliation receipt](p11-historical-reconciliation.json) содержит exact
canonical/own paths, hashes и gate log. Historical source_blobs, producer_execution,
current P05 actual-binding/source freeze не менялись. Новых Rust/SS/native/GUI
runs/builds нет. Для другого full-history checkout нужно материализовать эти же
два original файла с проверкой SHA в его local fixture path; новых producer replay
или claims об исполнении текущего whole backend не требуется.
