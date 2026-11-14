# Linux acceptance: delta принятого main на 14 ноября 2026

База reviewed PR144 — `c2e406f4065a050f54c2aeab148e42dffab56f95`;
принятый main — `d74e61884385cd20f659112182142b05cacb958e`.
Исходный inventory CLEAN не переаудирован. Статусы плана сохраняются:
**3 ready / 21 partial / 2 deferred**, знаменатель 26. Merge доказывает включение
source, а runtime вывод ограничен exact source и scope исходного witness.

## Новые принятые срезы

| Срез | Подтверждённая delta и предел |
| --- | --- |
| #126/#156/#158, #167 | [Native integration](2026-11-13-pr126-integration.md), [full pinned builders](2026-11-13-full-pinned-builders-158.md): Plain TUN validators/constructors обоих cores, TCP/UDP и исправление DNS socket lifetime; production ELF обоих cores собраны. Vnet не заявлен; native_protocol=null/runtime_hooks_connected=false не превращаются в production admission. |
| #147 | [Format4 GUI с реальным Secret Service](2026-11-12-format4-secret-service-gui.md). Это новый custody witness, не полная приёмка P06/P07 и mixed policy. |
| #148/#149/#150/#154 | Типизированные Import, scheduler Fetch/SourceFetch, ручные SourceFailure и Startup backend-thread причины приняты в source. [Fetch](2026-11-12-p11-scheduler-fetch-causes.md), [manual source](2026-11-12-p11-manual-source-context.md); полный P11 ещё открыт. |
| #145/#152/#160/#163 через #172 | [Reviewed desktop integration](2026-11-14-desktop-reviewed-integration.md): maintenance/probe cause mapping, bounded disk journal/drain guard, closure ledger/contracts. Старый ledger frozen, его incoming не переписан задним числом. Disk I/O с неопределённым временем остаётся pending. |
| #151/#161/#162 | Приняты port-filter/single-RPC и механические Clippy guards. Они не подтверждают full kernel/native/runtime acceptance. |
| #146/#170 | Kernel observation source и [v2 compile-only](kernel-probe-v2-compile-only.md) приняты. 16 fixed probes не запускались; runtime не разрешён, mint и system_tun закрыты. |

## Git authority каждого нового claim

Снимок refs на 2026-11-14. Для каждого head ниже выполнены
`git merge-base --is-ancestor HEAD BASE` и та же проверка с accepted main:
все accepted строки отсутствуют в baseline и входят в d74; incoming не входят в d74.
Статус PR OPEN сам по себе не опровергает ancestry (#158 включён через #126).

| PR | Exact head | В d74 | В baseline |
| --- | --- | --- | --- |
| [#126](https://github.com/MYRSGRAL/yonik/pull/126) | `aa442ab92492856152eec987b6314fb8cf0953ee` | accepted | нет |
| [#145](https://github.com/MYRSGRAL/yonik/pull/145) | `6e3d0490706c223b90fe6ab773446ec66ffb001f` | accepted | нет |
| [#146](https://github.com/MYRSGRAL/yonik/pull/146) | `8fbcb5a5e24f27ce462b8cd768c09ab45ad96be7` | accepted | нет |
| [#147](https://github.com/MYRSGRAL/yonik/pull/147) | `dd78f81c5c95930fad18b91f73f3a03d438e97a9` | accepted | нет |
| [#148](https://github.com/MYRSGRAL/yonik/pull/148) | `85f163542cbc346917c20e5c0357904f1f48f88e` | accepted | нет |
| [#149](https://github.com/MYRSGRAL/yonik/pull/149) | `0d4c8028e43713e74fb4b0bbe23894ef271a6ac1` | accepted | нет |
| [#150](https://github.com/MYRSGRAL/yonik/pull/150) | `9f4625314b47a9133b2fec22d4d4da2b232b9f8e` | accepted | нет |
| [#151](https://github.com/MYRSGRAL/yonik/pull/151) | `7d78767fdd586fc216861b0b2523a07d5fca3cdb` | accepted | нет |
| [#152](https://github.com/MYRSGRAL/yonik/pull/152) | `edc301b85afa34fbfb5049276070c10b27bbf125` | accepted | нет |
| [#154](https://github.com/MYRSGRAL/yonik/pull/154) | `3342dd45e39e5de7f7a37b9ef53c2f3c81eedabd` | accepted | нет |
| [#156](https://github.com/MYRSGRAL/yonik/pull/156) | `2c9dc795608081d6c8e30427c425b64a0e3fc5a3` | accepted | нет |
| [#158](https://github.com/MYRSGRAL/yonik/pull/158) | `8ca7f3f83097b040b1566aa2c6a918b78653b1eb` | accepted | нет |
| [#160](https://github.com/MYRSGRAL/yonik/pull/160) | `8fff27dbe58892a6831e9171530f6dad8c9da900` | accepted | нет |
| [#161](https://github.com/MYRSGRAL/yonik/pull/161) | `7934f1fb639945d9b14646873c52bfe06a94f0ed` | accepted | нет |
| [#162](https://github.com/MYRSGRAL/yonik/pull/162) | `8b29a1162d355505eed6fd957c82b9e6c1382f01` | accepted | нет |
| [#163](https://github.com/MYRSGRAL/yonik/pull/163) | `999662c53cbdb92c009340831aaf69e868c7294e` | accepted | нет |
| [#167](https://github.com/MYRSGRAL/yonik/pull/167) | `ce356b452675036fab3388ef64ea03f8f00910ce` | accepted | нет |
| [#170](https://github.com/MYRSGRAL/yonik/pull/170) | `16aefcfd4cdc3031bdabc362b0429b76489ed1c5` | accepted | нет |
| [#172](https://github.com/MYRSGRAL/yonik/pull/172) | `6f45329b2e0a9b1659232fd4558d75308a6bc6c7` | accepted | нет |
| [#155](https://github.com/MYRSGRAL/yonik/pull/155) | `1cf6aba2280a2ec8e45d7f5d0655f939e85fc9e6` | incoming | нет |
| [#166](https://github.com/MYRSGRAL/yonik/pull/166) | `87a51f9dd8d24035260d6df1811762a3dc998935` | incoming | нет |
| [#168](https://github.com/MYRSGRAL/yonik/pull/168) | `5a5e3beea896ee31b5043c1fe194adb4c7e74a49` | incoming | нет |
| [#169](https://github.com/MYRSGRAL/yonik/pull/169) | `88c398a1a3973f24c65eb7e1b369711daf3547a5` | incoming | нет |
| [#171](https://github.com/MYRSGRAL/yonik/pull/171) | `ca801bc1b6916522954b6e97a081b305f7b0bfe7` | incoming | нет |
| [#173](https://github.com/MYRSGRAL/yonik/pull/173) | `66df61753d37eaa5ff26d83ba1a024d86c56855e` | incoming | нет |
| [#174](https://github.com/MYRSGRAL/yonik/pull/174) | `c51bcf327c0ca76688e4080cbb418774b12eaf99` | incoming | нет |
| [#175](https://github.com/MYRSGRAL/yonik/pull/175) | `174e2e2ead7398b620cf7ead944d04553475ff59` | incoming | нет |

## Оставшийся нормативный critical path

Это компактные группы остатка, а не замена полного requirement inventory.
[Requirements](../specs/requirements.md), [validation](../specs/validation-and-roadmap.md),
[performance](../specs/performance.md) и [исходный индекс](2026-11-11-linux-acceptance-index.md)
сохраняют весь FR/gates/D/A/T/P scope, storage/recovery, GUI и privacy boundaries.

| Remaining gap | Внешнее доказательство / зависимость | Владелец |
| --- | --- | --- |
| Issuer / trusted bootstrap | #171 helper incoming; immutable snapshot, canonical PID/unit/invocation, trusted launcher и actual health по существующим contracts; полный mint остаётся закрыт. Не добавляется универсальный exec-monitor или повторный /proc/exe gate. | Root + helper/runtime owner (N2) |
| Full TUN | Reviewed helper custody/binding #168/#171 и actual consumer bridge #175 ещё incoming. Нужны production issuer/BoundTun, kernel admission, foreign-UID и actual attach→DNS→route→dial с отказами/cleanup; component FD proof не заменяет цепь. | Helper/session owner + adapters (N3/N4) |
| Native lifecycle и P05 | #169/#173 LocalSocks resource/alternating-core proofs имеют pending cleanup; не full P05 и не полный native lifecycle. Нужен принятый cleanup witness, затем требуемые system lifecycle/cancellation/resource cases на bound integrated source; subscription response/credentials/proxy-fetch scope остаётся. | Resource harness owner (W2), root acceptance |
| Mixed routing / DNS / policy | Actual Apply с app/process provenance, mixed domain/CIDR, private-network enforcement и нужная Linux native matrix; опирается на issuer/full TUN. Не требуется выдуманный whole cross-product всех transport/UDP для FR07. | Policy/native owner + root acceptance |
| UI/storage/diagnostics | #155 integrated manual witness incoming (synthetic key scope сохранён); #174 `c51bcf3` — incoming gate 14 boundaries на candidate #172, требует review/CI и ancestry closure. #172 не означает final P11 main closure; recovery и прочие непокрытые исходные критерии остаются. | Desktop/acceptance owner + root review |
| Performance | P-01…P-08, actual Qt/GTK4/Slint сравнения и engine-only baseline; allocator/pool balance и длительный plateau не следуют из component terminal cleanup. #169/#173 не дают полный seal. | Performance/acceptance owner |
| Final package | #166 incoming trusted source→build→ELF/compliance producer Ready не final build. Нужны итоговый reviewed freeze, реальные helper/cores/notices/package, isolated install/upgrade/rollback/uninstall и конечная requirement→evidence сверка. #167 только pinned core build scope. | Packaging owner (W1), root final acceptance |
| Deferred | Physical G-UI/G-POWER и Android P03/P26/G-ANDROID явно deferred, никогда PASS; отдельное решение пользователя перед возобновлением. | Пользователь + root |

Kernel runtime preconditions receipt сохранён отдельно в existing worktree:
`docs/evidence/kernel-runtime-preconditions-20261114/README.md` (не включён в этот PR).
Boot GNU build-id соответствует reviewed ELF только при доверии kernel/sysfs/root;
taint=64, ftrace_enabled=1, конкретный tracing/BPF UNKNOWN. Это conditional
preconditions, не filter attestation и не permission. Runtime16 остаётся запрещён
до отдельного явного разрешения. Kernel provenance refs/evidence, #172 и
#174 `c51bcf3` сохранены; повторный kernel/code audit не выполнялся.

NEXT: root review этого delta; отдельно review/CI #174 и helper/bridge leaves,
затем разрешённые scope-bound acceptance и финальная поставка. Общая Linux цель
остаётся незавершённой; количество merged PR и CI green не подменяют scope proof.
