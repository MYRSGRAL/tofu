# Production acceptance PR90: CONFIG 0600 и lifecycle matrix

Результат: **11 PASS / 1 FAIL / 0 NOT_RUN**, без повторов cases. Это **не 12/12 acceptance**. Единственный FAIL сохранён: ожидание контроллера не учитывало retained PublicBlock после реального helper crash с kill-switch. Production contracts не изменялись ради зелёного результата.

## Exact refs и source provenance

- Общая база задачи: `95e88474c0b4a18797dc28fa873a4b6b3adf2fa4`.
- Принятый предыдущий scoped gate: `a7187f64891288d5e44e9666e72ce4fbd9777ff0`, actual 4/4 с adapter. Он не подменяет эту матрицу.
- Hardening: `feb5599d9a60607cc29dcf256b8b6ed42190e0db`.
- Cases 1–5: exact committed source `1ccfdad78eec5f836aab1703e4fab40ac3c7da45`.
- Cases 6–12: exact committed source `f29d76032653705d65b0be87946fbbcd12eff7dc`; относительно предыдущего head изменён только `tests/isolatedfixture/production_adapter_case.py`. Все Rust/Cargo source hashes сверены с manifest до сборки; binaries переиспользованы без повторной сборки. `code-equivalence.json` подтверждает 196 одинаковых crate/Cargo blobs обоих heads.
- Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/production-matrix-a718`.
- `red-before-build.json` и `green-before-build.json` записаны до соответствующих сборок. `manifest-before.json` и `manifest-continuation-before.json` — до cases: все source/unit/binary/artifact/controller hashes, upstream/четыре patch SHA и проверенный image. `source.tar` и `continuation.tar` сохраняют source и delta.

## Hardening и локальные проверки

`worker.rs::config_file` делает fchmod0600 сразу после memfd_create, до первой записи и seals. Ошибка chmod закрывает FD и возвращает Io. Executable snapshots не менялись. Это confidentiality hardening, **не причина старого stock parse failure**; его различающий факт остаётся stock binary против adapter.

RED: новый тест получил 511 (0777) вместо 384 (0600), `red.log`, exit101. GREEN: mode до записи и после seal, UID, CLOEXEC, offset0, точные bytes/digest и запрет записи после seals. Linux library: **367 passed / 24 ignored**. Helper/worker build, pinned Clippy driver1.85.1 `--all-targets` с `-D warnings`, fmt и Ruff — PASS. Новые линты не подавлялись, зависимости не добавлены. `rust-results.json`, `green.log`, `build.log`, `clippy.log`, `local-checks.json`.

## Actual matrix

Одна собственная QEMU VM, официальный Arch image SHA `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`, проверенная подпись primary `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`. Настоящие installed helper/system bus/polkit/systemd worker units; caller UID968, worker UID969, CAP_NET_ADMIN (`CapEff=1000`), NoNewPrivs=1, существующий procview. Никаких новых caps/прав/ослабления unit.

Каждый case: non-app policy Prepare/Apply через RPCv1, native process с pinned SHA, четыре TUN-routed TCP/UDP v4/v6 echo потока. `stop` — RPC Stop; `restart` — SIGKILL MainPID **helper.service**, смена helper PID и рост NRestarts; `worker_exit` — SIGKILL MainPID **worker.service**, helper PID/NRestarts неизменны. Это реальные executable/systemd процессы, не libtest и не sleep.

| Core | kill_switch | Ending | Raw result |
|---|---|---|---|
| xray | false | stop | PASS |
| xray | false | restart | PASS |
| xray | false | worker_exit | PASS |
| xray | true | stop | PASS |
| xray | true | restart | FAIL |
| xray | true | worker_exit | PASS |
| sing-box | false | stop | PASS |
| sing-box | false | restart | PASS |
| sing-box | false | worker_exit | PASS |
| sing-box | true | stop | PASS |
| sing-box | true | restart | PASS |
| sing-box | true | worker_exit | PASS |

Все 12 выполнили 4/4 traffic, всего **48 потоков**; это не превращает незавершённый FAIL case в PASS. У всех 12 native stdin mode0600/seals15; Xray сохраняет inherited root-owned stdin UID0, SingBox использует worker-owned sealed copy UID969. Source offset/bytes/seals/budget checks покрыты Rust tests; actual SingBox обеих kill-switch веток выполняет положительный Prepare→Apply/traffic.

11 PASS включают State/Stop либо automatic recovery, kill-switch endpoint disposition и восстановление endpoint после единственного Unlock. Все 12 foreign nft/link/address/RPDB sentinels остались равны before/after. Sampler и проверка fixture наблюдали уменьшения network records при пустых обоих cgroups; это дискретные наблюдения, не атомарный kernel trace. Raw transitions сохранены.

## Сохранённый FAIL и точная причина

`attempt1/results/xray-true-restart-result.json`: `controller_or_inspect / TimeoutError / Automatic recovery deadline; no Stop/Unlock recovery retry`. Apply и 4 traffic уже прошли. После SIGKILL helper реально перезапустился: unit journal и диагностика показывают новый PID1656/NRestarts2; оба worker unit MainPID0. Journal имел 1 Confirmed record, public_policy=true, session_recovery=false. Контроллер ждал **ноль всех records до Unlock** и потому не разрешил клиенту перейти к State/Unlock.

Конкретный source branch: `helper/session_host.rs::finish_session → keep_after_stop → recover_ordered`. При kill_switch=true выполняются worker-first stop/restrict/session cleanup/clear_intent, а `host.unlock()` вызывается только при `!keep_block`. Один PublicBlock сохраняется намеренно. `cleanup_records` удаляет только Scope::Session. Следовательно нулевой journal до Unlock — неверный fixture contract, а не доказательство production recovery failure. Новая проверка требует ровно один Confirmed Scope::PublicBlock + public_policy для true, ноль records/public_policy для false, всегда пустые cgroups и session_recovery=None.

Независимый State/Unlock probe после FAIL прерывался на serial transport; его stdout не используется как PASS evidence. Cleanup затем подтверждён пустым journal перед дальнейшими fresh cases. Исходные case logs/results/diagnostics не переименованы в PASS. Этот case **не повторялся**. После узкой fixture correction выполнены только семь ранее NOT_RUN cases; `attempt1/case-results.json` сохранён отдельно от `continuation/continuation-case-results.json`. `checked-matrix.json` объединяет факты, сохраняя FAIL. Для полного 12/12 остаётся отдельный future fresh Xray/true/helper-crash case с исправленным контроллером; в этом scope retry budget=0.

## Финальные ресурсы и ограничения

Final journal records0/public_policy=false/session_recovery=false; обоих workers MainPID0. Worker unit остаётся failed после намеренного SIGKILL — статус не скрывался reset-failed. Helper active, NRestarts4. Managed TUN/link/rules/block отсутствуют; до poweroff оставались только peer fixture и foreign sentinels. Собственная VM завершилась exit0. Cargo clean удалил 4143 files / 2.6GiB; active references проверены, собственный transient root удалён. `cleanup.json`, `vm-shutdown.json`, `continuation/results/continuation-final-resources.json`.

Это положительные **non-app** production cases. FR20 executable attribution не объявляется выполненным: существующий Unsupported и предыдущий multiUID negative остаются отдельными фактами. Старые raw FAIL не удалялись. Name-based adapter4c31 остаётся прежним и не выдаётся за ownedFD/no-effects ABI. Go edits/build, новые hooks/caps, host/sandbox changes отсутствуют. Независимый code/security review340 был clean; новые hardening/fixture/report commits требуют своего review. Полный GPOWER/10x performance gate этой матрицей не заявляется.

## Pinned hashes

- `yonik-helper`: `ad9990d98c08322f894996d194af90a0219f3b3b2855912bf99af1901e27aae7`.
- `yonik-worker`: `7123e55a4097b6b5ca31539c525ddc9ea8693f665d268e0dfb33b794e45c3c9a`.
- `xray`: `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.
- `sing-box`: `4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.
- `source/packaging/systemd/yonik-helper.service`: `4e3378edc331943fa961e9442b4f766052202a3802cf99e6c7b25c891e3d5922`.
- `source/packaging/systemd/yonik-worker.service`: `0440da796528dd522b3563fbdac983c6410c83eddafd16b6e2668396facefe70`.
- `source/packaging/systemd/yonik-worker-check.service`: `a0084f73f6b1936a7a54fab291343397b2516c305e6006419cd14cb286de8011`.
- `manifest-before.json` SHA256: `3a139ee750f805b99c4fca0f15ffc10705781c72b96995ee8e3ce476d6798fec`.
- `manifest-continuation-before.json` SHA256: `4e84672a6a432c40b15afd928ae7c39a7b7764c99a5501c6c7a5984df9a552ff`.
- `checked-matrix.json` SHA256: `ff27943bcaddb3ae5feb4a036c7f1f04feaacea7b73067f216b0c1d5acb68bf4`.
