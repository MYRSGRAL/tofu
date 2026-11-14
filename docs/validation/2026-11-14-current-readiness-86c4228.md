# Текущая готовность и оставшаяся очередь — accepted f0ea828

Это компактный source-bound реестр 26 пунктов, не новый test framework и не
замена исторических 256 строк. Исторический счёт **3/21/2**, знаменатель **26**
и два отложенных Android-пункта сохраняются. Root принял исходный P05 как
qualified software Ready по trusted R2 и ограниченному source transfer до `cfcb457`.
Счёт — **9 готовы / 15 частично / 2 отложены**, остаток **17: 15 Linux + 2 Android**.
Native issuer/N5 и full Linux из этого решения не следуют.
Linux done, final A5 и release PASS не заявлены.

## Принятая база и источник утверждений

Авторитетный MAIN — `f0ea828982a085ede4862606ec78f8c0cca7e4ea` (PR220);
исторический PR187 — `48ce74304a3fb7263ccc7e2c06d96394113a687d`;
source transfer P05 ограничен `cfcb4576dd0ead9cbf65a5bed95ff4f008813ba9`
(PR191), предыдущий accepted217 — `a3f609e66e5a95b978c0bf3da34ade4f1445e219`.
Read-only GitHub API подтвердил PR191 MERGED, exact head
`b8c7380586472d396b5a60b49bd349fc5c96d80f`, merge SHA выше и parents:
`a3f609e66e5a95b978c0bf3da34ade4f1445e219` и exact head PR191.
Исторический источник этой проверки PR191 — API; актуальная локальная ancestry
PR216/218 проверена отдельно ниже.
[PR191](https://github.com/MYRSGRAL/yonik/pull/191) принят как software-only toolkit
campaign v6; это не physical G-UI/G-POWER PASS.
Startup [PR214](https://github.com/MYRSGRAL/yonik/pull/214) принят на
`e358653dfd8e0d74e29a2a28ec73e7969626072d`.
P25 capture [PR215](https://github.com/MYRSGRAL/yonik/pull/215) принят на
`b929b22b0c65b25d28c862b9d9f455460a8083c8` (head
`04444a3fc3f444cff136d2f0a52bc1e9ef7fe061`), attribution join
[PR217](https://github.com/MYRSGRAL/yonik/pull/217) — на accepted217 выше.
Это source/tool integration, не actual final-build proof; границы:
[capture](p25-current-payload/post-build-capture-tool.md),
[join](p25-current-payload/postcapture-attribution-join.md).
P05 semantic transfer ниже по-прежнему ограничен `07d5868`;
Новый transfer до `cfcb457` описан ниже; P21/P23/P25 не повышены.
Git objects подтверждают последовательность:

- `c079e9dea02cd34376d7103dc765c0ab43c71802` →
  `bc6b55457f987ce0fe3a9a6d236908b9ac0ee3ea`: #213,
  второй parent `0bb3e86e22d7bf6709c90453e9f8619b60a59ee4`.
- `bc6b55457f987ce0fe3a9a6d236908b9ac0ee3ea` →
  `86c4228c252c9fec229b4b20ba39a2dc1b230221`: #212,
  второй parent `a3dc9dd6a1aa696f8ffb53ff0883a58488997a0a`.

- `86c4228c252c9fec229b4b20ba39a2dc1b230221` →
  `07d5868f58de8f73e9005f21a4ec3945b4b7a14d`: #211,
  второй parent `379515a2727064c205f0447c137e76731a1332c0`.

Для #213/#212 accepted heads root передал all 10 green + CLEAN. Это доверенный
acceptance handoff, не повторная проверка CI/API в настоящей задаче.
Идентификаторы review R2 `b6a55c22` и R1 `91cbaf41` также переданы root;
они не разрешаются как Git commits и не используются как source SHA.
`379515a` теперь входит в accepted #211: worker-output source integration принят.
Это не full FR16 runtime/final A5. `ac26` и capture `9f` сами по себе
не повышают readiness. Startup214 и P25 capture215 + join217 теперь accepted
в указанных выше границах. N5 [PR216](https://github.com/MYRSGRAL/yonik/pull/216)
теперь accepted software/source: exact head
`c411b32ce0303158ddb04230f8aff2488ea34bfb`, merge
`0f69d5e200f9d9434a51f9e2163cdf32c58d655b`, exact 10 CI green и trusted
scoped reviews: CORE/PIPE baseline/deltas, INSPECT/bus lifetime, main exeFD
Candidate_Verified/transport, root exact compile inputs/retained source+ELF binding.
Это заменяет прежний DRAFT/known-positive-path repair статус, но не runtime PASS.

[PR218](https://github.com/MYRSGRAL/yonik/pull/218): exact head
`088b502197f16e49ce41045ddcf4a5947b6feee9`, merge, предыдущий accepted MAIN
`b5a9af15e8a652ad4c9cde7901df776e94edcca8`, exact 10 CI green и trusted R1/R2
CLEAN (`38cf32c5` controller/default pipeline, `7991ecc2` used-input retainer).
Приняты только source-only Rust capturepartial/SDK bytes helpers: они позволяют
`RUST_CAPTURED_ONLY`, никогда не означают `COLLECTED`; full source OPEN,
legal NOTPROVEN. Actual build/capture NOT RUN.

Acceptance receipts root: локальные
`/home/myrs/projects/yonik/local/orchestration/evidence/accepted-216-merge.json` и
`/home/myrs/projects/yonik/local/orchestration/evidence/accepted-218-merge.json`.
CI/review утверждения взяты только из trusted acceptance handoff/receipts;
в этой задаче CI и review заново не исполнялись.
Локальный Git подтверждает parents merge216: `48ce743` + exact head216;
parents merge218: merge216 + exact head218. Оба exact head и merge216 — ancestors
accepted MAIN; accepted MAIN — ancestor текущего task HEAD.

[PR219](https://github.com/MYRSGRAL/yonik/pull/219) принят на `f3941a1716800e7e15bc0702301403e6b756c361`: eligibility, driver fixture и root capture source. [PR220](https://github.com/MYRSGRAL/yonik/pull/220) принят на `f0ea828982a085ede4862606ec78f8c0cca7e4ea`: capture → N5 receipts, external pins и projection. Квитанции root: `/home/myrs/projects/yonik/local/orchestration/evidence/accepted-219-merge.json` и `/home/myrs/projects/yonik/local/orchestration/evidence/accepted-220-merge.json`; локальный Git подтверждает ancestry 219 → 220 → task HEAD. Actual build/capture и runtime не выполнялись. PR221 head `2bc7939` ожидает CI/review, не принят и не является runtime evidence.

P14 требует production DNS software integration и отдельной runtime-проверки; evidence root: `/home/myrs/projects/yonik/local/orchestration/evidence/p14-production-software-boundary/boundary.md`. P20: software замены активной сессии отсутствует в текущих LocalSocks и System; evidence root: `/home/myrs/projects/yonik/local/orchestration/evidence/p20-atomic-current-boundary/boundary.md`. Actual sealed tuple, runtime grant,
profile registration, packet/DNS/app owner, cleanup, full FR16, physical PAM/power
и final A4/A5 остаются OPEN. Runtime/profile исполнения закрыты для этой задачи;
новые сборки, capture, runtime и профильные проверки не запускались.

## P05: принятое решение и границы

Trusted R2 `b6a55c22` квалифицировал исходный P05 software Ready на accepted
`c079e9d`. Основание — [124 исторических native cases](2026-11-11-p05-refresh-acceptance.md),
[criterion join](p05-p10-accepted-witness/criterion-join.md),
[NormalHost/private Secret Service witness](p05-p10-accepted-witness/README.md)
и [actual default-lease Stop/shared-window обоих cores](p05-p10-accepted-witness/legacy-stop-actual.md),
прошедшие review #199. Исторический concrete FetchHost override сам по себе
не доказывал default lease; отдельный actual legacy Stop witness закрыл именно
этот seam. Измеренный source последнего proof — `f0a1d13`, а не текущий MAIN.

Read-only semantic transfer `c079e9d..86c4228` ограничен связанными producers:
`yonik-linux/src/{fetch,subscriptions,subscription_scheduler,subscription_policy}.rs`,
`yonik-subscription/src/{lib,scheduler}.rs` не изменены. В desktop_schedule изменён
только regression test typed initialize failure. Backend меняет два replacement
callers: прежний silent отказ получает typed initialize context без выдуманного
поколения; успешный replacement/load и Stop window-preservation сохраняются.
Default DesktopFetch, Running lease/watch revocation, manual/scheduled
completion/commit/persist не переписаны. Остальные backend изменения относятся
к ports/format5/Unsupported и nondefault test-support lifecycle gate; repository
также меняет общий schema guard: допускает format5, а source create/import
вместо безусловного user_version=4 вызывает preserve_format4, чтобы не понизить
уже принятую schema5. Source generation/revision, encrypted snapshot reconcile
и CAS predicates сохранены; это совместимость schema, не новый runtime proof.
Дополнительный bounded transfer `86c4228..07d5868`: перечисленные subscription
producers и desktop_schedule byte-exact. Backend diff проводит event_log в
System Flight, сохраняет SystemSession при owner replacement и добавляет
output drain перед disk-log shutdown. LocalSocks lease/fetch/commit/replacement
ветви не изменены. Это проверка text invariants только затронутой границы,
не повтор полного принятого аудита и не System/FR16 runtime proof.
Положительный исторический proof не объявляется exact execution нового backend.

Вывод root: исходный P05 принят qualified Ready в software scope; P10 energy, native
System authority, N5 и final A5 этим не закрыты. Полного rerun 124 cases,
новой сборки, Host/SS/native runtime в этой задаче не было.

Дополнительная read-only сверка `c079e9d..cfcb457` подтверждает неизменность
DesktopFetch (102 строки), refresh producer (60), lease publication (39) и manual
result publication (16). Startup/FR16/P09 не добавили непокрытый P05 producer.
[Source binding](p05-current-source-transfer.json) сохраняет точные blobs/offsets.
Это композиция исторического proof и source-инвариантов, не выполнение нового head.

## Все 26 строк

Принятые Ready: P01/P02/P04/P05/P06/P08/P11/P18/P22 (9). Linux Partial:
P07/P09/P10/P12/P13/P14/P15/P16/P17/P19/P20/P21/P23/P24/P25 (15).
Android deferred: P03/P26 (2).

| ID | Статус | Текущее основание или реальный остаток |
| --- | --- | --- |
| P01 | Готов | Требования и архитектурные контракты; implementation gates отдельно. |
| P02 | Готов | Исторический Android compile-check; final6pairs [PR187](https://github.com/MYRSGRAL/yonik/pull/187) принят (`48ce743`), не новый final build/VPN runtime PASS. |
| P03 | Отложен | Android runtime-host, оба engine/FFI/TUN/protect/revoke. |
| P04 | Готов | Исходный импорт; исторические source bounds сохранены. |
| P05 | Готов | Исходный software-критерий принят root: trusted R2 + source transfer до cfcb457. Native/N5/full Linux отдельно. |
| P06 | Готов | Storage/restore; physical login/PAM и final A5 отдельно. |
| P07 | Частично | #210 ports/protocol и format5 приняты через #213; format4 как текущий остаток устарел. Полная integrated/editor/enforcement матрица OPEN. |
| P08 | Готов | Исходный compiler scope; native activation/DNS/handshake отдельно. |
| P09 | Частично | Common LocalSocks qualified Ready, R1 91cbaf41; #212 Validate/Apply cancellation принят. Full native/System/helper lifecycle OPEN. |
| P10 | Частично | Software Ready; physical G-POWER отложена, НЕ PASS. |
| P11 | Готов | Принятый #213 replacement/source freshness; #211 worker source integration принят без расширения runtime scope. |
| P12 | Частично | Production helper ownership, live topology и app/domain Apply OPEN. |
| P13 | Частично | Full production TUN обоих cores, authority и cleanup OPEN. |
| P14 | Частично | Production DNS software integration и runtime D-01…D-07 обоих cores OPEN. |
| P15 | Частично | Полная native attribution A-01…A-08 и стоимость OPEN. |
| P16 | Частично | Private VPN/LAN, split DNS, сеть/sleep/recovery OPEN. |
| P17 | Частично | Integrated native policy priorities и Linux acceptance OPEN. |
| P18 | Готов | Исходные UI/HWID; product/physical gates отдельно. |
| P19 | Частично | Production GUI/TUN, recovery и полный пользовательский путь OPEN. |
| P20 | Частично | Замена активной сессии в LocalSocks/System отсутствует; actual atomic Apply и полная семантика двух редакторов OPEN. |
| P21 | Частично | Две отдельные обязательные startup-настройки с default OFF software accepted в #214/8223. #215 capture + #217 join accepted отдельно; physical PAM/reboot/lock OPEN, NOT PASS. Полный критерий OPEN. |
| P22 | Готов | Исходные batch checks/diagnostics с сохранёнными source bounds. |
| P23 | Частично | Toolkit191 software-only accepted; physical G-UI/G-POWER отложены без waiver/PASS. |
| P24 | Частично | Конечный source freeze/package/helper/cores и final A5 OPEN. |
| P25 | Частично | P25 capture215 + join217 и PR218 Rust capturepartial/SDK bytes helpers source-only accepted (`RUST_CAPTURED_ONLY`, не `COLLECTED`); actual final build и итоговая release/FR/licensing acceptance OPEN. |
| P26 | Отложен | Android приложение/поставка после P03 и Linux v1. |

P21: [requirements:45](../specs/requirements.md) называет две настройки,
[routing-and-tray:25](../specs/routing-and-tray.md) закрепляет отдельный startup
в трей и autoconnect, первоначально выключенные. В текущей очереди это
обязательное требование по явному решению root, несмотря на исторический заголовок
раздела «рекомендации» в requirements. Настоящий документ не редактирует контракт.

## Порядок оставшейся работы

1. Root интегрирует coherent docs patch; final6pairs PR187 принят (`48ce743`). Startup214,
   capture215 + join217 и toolkit191 уже accepted в указанных software/source bounds.
2. Native N5 (PR216 accepted software/source; read-only eligibility `41b` LOCAL review CLEAN, не accepted main) и P12–P17: driver/fixture/root capture CODE pending; actual sealed tuple, runtime grant, profile registration, authority/installed lifecycle, packet/DNS/app owner, cleanup, attribution,
   private-network и integrated policy proofs. LocalSocks не заменяет System/TUN.
3. P19–P21 и full FR16 runtime: product paths, physical startup/PAM/reboot/lock и worker
   availability; принятый #211 source integration и capture не закрывают эти строки.
4. P23 physical gates остаются deferred без PASS; P24/P25 и final A5 требуют
   конечного source freeze и своей приёмки. A4 требует production/package/
   userdata custody, A5 — итоговую интегрированную приёмку; прежние functional
   tray/LocalSocks proofs их не заменяют. Required autostart/autoconnect
   software принят отдельно в #214 (оба default OFF); physical power и
   PAM/reboot/lock остаются отдельными gates без PASS. Android P03/P26 остаются отложенными.

Проверка этой правки — только 26 уникальных IDs, решение root 9/15/2, локальные Git ancestry PR216/218 и исторический API merge191,
локальные ссылки и diff scope. Исторические 256 строк и их artifacts не менялись.
