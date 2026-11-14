# Текущая готовность и оставшаяся очередь — accepted cfcb457

Это компактный source-bound реестр 26 пунктов, не новый test framework и не
замена исторических 256 строк. Исторический счёт **3/21/2**, знаменатель **26**
и два отложенных Android-пункта сохраняются. Принятый счёт (решение о повышении P05 ещё не принято) —
**8/16/2**. Предложение к пакетной интеграции root: исходный P05 software Ready,
предлагаемый итог **9 готовы / 15 частично / 2 отложены**, остаток 17 пунктов.
Текущий принятый остаток — **18: 16 Linux + 2 Android**; после явного принятия
предложения P05 — **17: 15 Linux + 2 Android**. Таблица ниже отражает accepted
8/16/2; software-qualified не равен автоматически accepted.
Linux done, final A5 и release PASS не заявлены.

## Принятая база и источник утверждений

Авторитетный MAIN по handoff root — `cfcb4576dd0ead9cbf65a5bed95ff4f008813ba9`
(PR191), предыдущий accepted217 — `a3f609e66e5a95b978c0bf3da34ade4f1445e219`.
Read-only GitHub API подтвердил PR191 MERGED, exact head
`b8c7380586472d396b5a60b49bd349fc5c96d80f`, merge SHA выше и parents:
`a3f609e66e5a95b978c0bf3da34ade4f1445e219` и exact head PR191.
Локального объекта merge191 нет; источник этой проверки — API, не локальная ancestry.
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
новый MAIN не расширяет его source bounds и не повышает статус P05/P21/P23/P25.
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
в указанных выше границах. N5 software `4ae21`: pure PASS по trusted handoff;
[PR216](https://github.com/MYRSGRAL/yonik/pull/216) остаётся DRAFT:
known-positive-path repair, не accepted и не native runtime PASS. Runtime/profile
закрыты для этой задачи; новые исполнения и профильные проверки не разрешены.

## P05: основание единственного предложенного повышения

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

Вывод: исходный P05 предлагается Ready в software scope; P10 energy, native
System authority, N5 и final A5 этим не закрыты. Полного rerun 124 cases,
новой сборки, Host/SS/native runtime в этой задаче не было.

## Все 26 строк

Принятые Ready: P01/P02/P04/P06/P08/P11/P18/P22 (8). Linux Partial:
P05/P07/P09/P10/P12/P13/P14/P15/P16/P17/P19/P20/P21/P23/P24/P25 (16).
Android deferred: P03/P26 (2). Предложение меняет только P05, оставляя
остальные 15 Linux Partial и оба Android deferred.

| ID | Статус | Текущее основание или реальный остаток |
| --- | --- | --- |
| P01 | Готов | Требования и архитектурные контракты; implementation gates отдельно. |
| P02 | Готов | Исторический Android compile-check; final6pairs [PR187](https://github.com/MYRSGRAL/yonik/pull/187) pending, не новый final build/VPN runtime PASS. |
| P03 | Отложен | Android runtime-host, оба engine/FFI/TUN/protect/revoke. |
| P04 | Готов | Исходный импорт; исторические source bounds сохранены. |
| P05 | Частично | Software qualified Ready по trusted R2; повышение предложено, решение root ожидается. |
| P06 | Готов | Storage/restore; physical login/PAM и final A5 отдельно. |
| P07 | Частично | #210 ports/protocol и format5 приняты через #213; format4 как текущий остаток устарел. Полная integrated/editor/enforcement матрица OPEN. |
| P08 | Готов | Исходный compiler scope; native activation/DNS/handshake отдельно. |
| P09 | Частично | Common LocalSocks qualified Ready, R1 91cbaf41; #212 Validate/Apply cancellation принят. Full native/System/helper lifecycle OPEN. |
| P10 | Частично | Software Ready; physical G-POWER отложена, НЕ PASS. |
| P11 | Готов | Принятый #213 replacement/source freshness; #211 worker source integration принят без расширения runtime scope. |
| P12 | Частично | Production helper ownership, live topology и app/domain Apply OPEN. |
| P13 | Частично | Full production TUN обоих cores, authority и cleanup OPEN. |
| P14 | Частично | Полный DNS/Fake DNS D-01…D-07 обоих cores OPEN. |
| P15 | Частично | Полная native attribution A-01…A-08 и стоимость OPEN. |
| P16 | Частично | Private VPN/LAN, split DNS, сеть/sleep/recovery OPEN. |
| P17 | Частично | Integrated native policy priorities и Linux acceptance OPEN. |
| P18 | Готов | Исходные UI/HWID; product/physical gates отдельно. |
| P19 | Частично | Production GUI/TUN, recovery и полный пользовательский путь OPEN. |
| P20 | Частично | Actual atomic Apply и полная семантика двух редакторов OPEN. |
| P21 | Частично | Две отдельные обязательные startup-настройки с default OFF software accepted в #214/8223. #215 capture + #217 join accepted отдельно; physical PAM/reboot/lock OPEN, NOT PASS. Полный критерий OPEN. |
| P22 | Готов | Исходные batch checks/diagnostics с сохранёнными source bounds. |
| P23 | Частично | Toolkit191 software-only accepted; physical G-UI/G-POWER отложены без waiver/PASS. |
| P24 | Частично | Конечный source freeze/package/helper/cores и final A5 OPEN. |
| P25 | Частично | P25 capture215 + join217 source/tool integration accepted; actual final build и итоговая release/FR/licensing acceptance OPEN. |
| P26 | Отложен | Android приложение/поставка после P03 и Linux v1. |

P21: [requirements:45](../specs/requirements.md) называет две настройки,
[routing-and-tray:25](../specs/routing-and-tray.md) закрепляет отдельный startup
в трей и autoconnect, первоначально выключенные. В текущей очереди это
обязательное требование по явному решению root, несмотря на исторический заголовок
раздела «рекомендации» в requirements. Настоящий документ не редактирует контракт.

## Порядок оставшейся работы

1. Root интегрирует coherent docs patch; final6pairs PR187 pending. Startup214,
   capture215 + join217 и toolkit191 уже accepted в указанных software/source bounds.
2. Native N5 (software 4ae21 pure PASS; PR216 DRAFT, known-positive-path repair не accepted) и P12–P17: authority/installed lifecycle, DNS, attribution,
   private-network и integrated policy proofs. LocalSocks не заменяет System/TUN.
3. P19–P21 и full FR16 runtime: product paths, physical startup/PAM/reboot/lock и worker
   availability; принятый #211 source integration и capture не закрывают эти строки.
4. P23 physical gates остаются deferred без PASS; P24/P25 и final A5 требуют
   конечного source freeze и своей приёмки. A4 требует production/package/
   userdata custody, A5 — итоговую интегрированную приёмку; прежние functional
   tray/LocalSocks proofs их не заменяют. Required autostart/autoconnect
   software принят отдельно в #214 (оба default OFF); physical power и
   PAM/reboot/lock остаются отдельными gates без PASS. Android P03/P26 остаются отложенными.

Проверка этой правки — только 26 уникальных IDs, accepted 8/16/2 и proposed 9/15/2, локальные Git ancestry до доступной базы и API merge191,
локальные ссылки и diff scope. Исторические 256 строк и их artifacts не менялись.
