# PR90: свежая полная production acceptance frozen CONFIG0600

Критерий выполнен: **12/12 lifecycle PASS, 48/48 TCP/UDP v4/v6 witnesses, 0 FAIL, 0 NOT_RUN, 0 повторов cases/Apply/Stop/traffic** в новой чистой VM. Отдельный финальный State→Stop→Unlock→State также PASS. Положительный FR20 executable attribution этим результатом не заявляется.

## Exact source и frozen binaries

- Общая база PR: `95e88474c0b4a18797dc28fa873a4b6b3adf2fa4`.
- Production CONFIG0600 frozen head: `8ae373f5bbfba408b9fbc7785f1352ea51b4c7ac`.
- Actual exact committed source/fixture: `8610f59b58d68ab090653fb6e3e39977c2bc90bb`.
- Diff frozen→actual: только `tests/isolatedfixture/production_adapter_case.py` и новый `test_production_adapter_case.py`. Ни worker.rs, ни иной production Rust/Go/unit/rights code не изменён.
- `binary-reuse-before.json`: 196 одинаковых crate/Cargo git blobs actual/frozen; все crate/Cargo SHA256 сверены также с предыдущим `green-before-build.json`. Helper/worker из `/home/myrs/projects/yonik/local/validation/production-matrix-a718/artifacts`, их hashes сверены перед staging. **0 Rust builds**, guest builds отсутствуют; Cargo target не создавался.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/production-acceptance-8ae373f`. `manifest-before.json` записан **до actual**: source/unit/binary/controller/artifact hashes, adapter upstream/четыре patch SHA, image provenance, RED/GREEN hashes и порядок всех 12 cases. `source.tar` сохраняет exact source, `guest-results.tar` — raw results. Offline checker — `analyze.py`, результат — `checked-matrix.json`.

## Oracle: RED→GREEN по production contract

После helper crash с kill_switch=true production `finish_session → keep_after_stop → recover_ordered` сохраняет restrictive PublicBlock до явного Unlock; session recovery очищается после worker-first session cleanup. Ожидать ноль всех records до Unlock неверно. Блокировку ради теста не очищали.

Извлечённый старый guard проверял только присутствие public_policy. RED (`oracle-red.log`) показал принятие исключений и Removing: 6 failures. GREEN (`oracle-green.log`): 6 tests прошли. Guard теперь требует empty workers, session_recovery=None, один Confirmed Scope::PublicBlock с соответствующим id, фазу Stable, restrictive policy без TUN/private/LAN/transport/bootstrap-DNS exceptions. Для kill_switch=false ожидаются пустые records/public_policy. Тест фиксирует, что пустой journal при true **до Unlock** не считается восстановлением.

Fresh State допускает только bounded typed Busy (до 20 чтений с 250ms между Busy), затем точный ожидаемый state. Unknown, другие ошибки и преждевременный NoManagedPolicy при true отвергаются сразу; Busy deadline проверен. В actual Busy не наблюдался — эта ветка подтверждена unit tests. Общий case deadline90s остаётся; write/traffic retry отсутствует.

## Actual matrix — все четыре строки, двенадцать cases

| Pinned core | kill_switch | Stop | Helper crash/restart | Worker exit |
|---|---|---|---|---|
| Xray 26.10.31 | false | PASS | PASS | PASS |
| Xray 26.10.31 | true | PASS | PASS | PASS |
| Sing-box adapter 1.14.4 | false | PASS | PASS | PASS |
| Sing-box adapter 1.14.4 | true | PASS | PASS | PASS |

Каждая ячейка — отдельный fresh Prepare/Apply non-app policy, 4 native TUN-routed echo flows: TCPv4, UDPv4, TCPv6, UDPv6. Это **12 lifecycle PASS**, а не переименование 12 traffic groups. Matrix RPC: Prepare12/Apply12/Stop4/Unlock12; остальные окончания имеют automatic worker cleanup до Unlock и не получают повторный Stop. Дополнительный финальный cleanup отдельно: State/Stop/Unlock/State по одному.

Использованы реальные installed helper executable, system bus, canonical peer credentials, polkit и production systemd worker units. Caller UID968, worker UID969; actual native status CapEff=0000000000001000 (CAP_NET_ADMIN), NoNewPrivs=1, штатные procview/mounts, без расширения прав. VM — QEMU 2CPU/2GiB с отдельным overlay/bootid; не libtest/sleep. Peer — изолированный VM-local netns, traffic проверяет настоящие pinned cores.

Четыре helper crashes: SIGKILL именно helper MainPID, новый PID, NRestarts увеличен, exe SHA совпадает frozen helper. Четыре worker exits: SIGKILL именно worker unit, helper PID/NRestarts не меняются. Helper final NRestarts4. Для true до Unlock проверены restrictive retained block, RecoveryRequired и недоступность direct endpoint; после явного Unlock — NoManagedPolicy и доступность endpoint. Для false — NoManagedPolicy и доступность endpoint после ending.

У всех 12 native stdin mode0600/seals15; Xray inherited root-owned FD UID0, SingBox worker-owned sealed config copy UID969. Sing-box — именно existing adapter4c31, stock1264 не использован. Adapter manifest/upstream `c005f35300ce76427dc285481ec5695f5b179cd2` и четыре patch hashes сверены с staged source; старый name-based adapter не выдаётся за ownedFD/no-effects ABI.

## Worker-first, sentinels и cleanup

Все observed уменьшения network records происходили при пустых обоих worker cgroups before/after sample; raw `*-transitions.jsonl` и `*-cleanup.json` сохранены. Это дискретные observations, не атомарный kernel trace. Все 12 before/after foreign nft/link/address/RPDB sentinels равны. После каждого case journal records0/public_policy=false/session_recovery=false, workers empty.

Финальный дополнительный Stop/Unlock выполнен UID968 через настоящий bus/polkit, с отдельным controller hash до запуска (`final-cleanup-before.json`, `final-stop-unlock.log`). После него повторно проверены empty journal/workers, отсутствие managed links/tables и неизменность rules (`final-after-cleanup.log`). Worker unit остаётся failed после намеренного SIGKILL, MainPID0 — статус не скрывался reset-failed. Check worker inactive/MainPID0. До poweroff оставались только peer fixture и foreign sentinels.

VM shutdown exit0, активные ссылки на собственный transient root проверены, root удалён; отдельный временный GPG agent завершён. Host настройки не менялись. Target/Cargo clean неприменимы: сборок/target не было. `cleanup.json`, `vm-shutdown.json`.

## Старые результаты и предел утверждения

Старый raw controller FAIL и прежний 11/12 результат остаются в `/home/myrs/projects/yonik/local/validation/production-matrix-a718`; ничего не переписано и не переименовано в PASS. `previous-evidence-preserved.json` фиксирует hashes; после run они повторно сверены. Новый отдельный clean-VM run закрывает lifecycle gap прошлого отчёта `2026-11-11-production-config-private-matrix.md`.

Это acceptance положительного **non-app policy** production path. Existing executable-policy Unsupported/foreign-UID ограничения не объявляются решёнными; полного FR20/GPOWER/10x performance proof здесь нет. Go/native hooks/caps/ProtectProc/sandbox и production worker не менялись. Независимый review frozen CONFIG0600 остаётся за root/reviewer; новый commit содержит только fixture/tests, затем этот report.

## Pinned provenance

Официальный image SHA256 `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`, подпись primary `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`; полная проверка в image/signature manifests.

- `yonik-helper` SHA256: `ad9990d98c08322f894996d194af90a0219f3b3b2855912bf99af1901e27aae7`.
- `yonik-worker` SHA256: `7123e55a4097b6b5ca31539c525ddc9ea8693f665d268e0dfb33b794e45c3c9a`.
- `xray` SHA256: `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.
- `sing-box` SHA256: `4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.
- `source/packaging/systemd/yonik-helper.service` SHA256: `4e3378edc331943fa961e9442b4f766052202a3802cf99e6c7b25c891e3d5922`.
- `source/packaging/systemd/yonik-worker.service` SHA256: `0440da796528dd522b3563fbdac983c6410c83eddafd16b6e2668396facefe70`.
- `source/packaging/systemd/yonik-worker-check.service` SHA256: `a0084f73f6b1936a7a54fab291343397b2516c305e6006419cd14cb286de8011`.
- `manifest-before.json` SHA256: `f8ae41267e26319aa8462c918cb4a8a0da5853993b9f4d02b49f13d4ad5f2671`.
- `binary-reuse-before.json` SHA256: `02cb181ca4998461871828faa32e93f2bdb77a397d6f881272e1b2b974b3b83d`.
- `checked-matrix.json` SHA256: `e7e417c41cc7f3cc23b8bd51d2c5175cbc2a927b1a272db2170e5a94307731a4`.
