# A4 source142: трактовка rollback и bounded QEMU/PID1 hook proposal

> **For agentic workers:** REQUIRED SUB-SKILL: `superpowers:executing-plans` после решения root. Выполнять самостоятельно, **no nested**. Сейчас только DESIGN/READONLY; команды ниже **не выполнялись**.

**Goal:** закрыть ровно недоказанную live ветку package daemon-reload на source142/MTREE4, сохранив раздельные статусы rollback preservation и old-reader compatibility.
**Architecture:** reuse подписанного #147 Arch base и его проверенного QEMU/UEFI/SSH bootstrap, плюс real pacman transaction pattern #119. Новый hook-only driver вместо прямого запуска старого VM runner. Никаких GUI/custody/native-TUN сценариев в VM.
**Tech Stack:** имеющиеся QEMU/KVM, qcow2 overlay, xorriso/cloud-init/SSH, guest PID1 systemd, pacman/libalpm hooks; без сборки и загрузок.
**Spec:** [original-criterion.json](original-criterion.json), [data-and-contracts.md:103](../../specs/data-and-contracts.md#L103), [f0 nonempty lifecycle](../../validation/2026-11-14-a4-nonempty-lifecycle-142.md).

## 1. Точный исходный criterion и вывод

A4 — имя root очереди финального package результата, а не отдельный раздел с требованием future-schema old-reader compatibility. В исходной P25 строке root queue (путь, SHA и точная строка сохранены в `original-criterion.json`) написано:

> Package/helper/registry/GUI/notices из итогового exact merge ref; source→ELF proof, Arch isolated install/upgrade/rollback/uninstall/data preservation, reboot где требуется.

Первоначальный контракт этой A4 задачи уточняет actual file/layout/permissions/ownership/hooks где поддержаны и **сохранение** профилей/original/revision/Secret Service item/данных. Он не содержит требования «старый GUI обязан читать все записи новой схемы».

Нормативный data contract: «Старое приложение не открывает новую схему на запись. Откат ядра и откат схемы приложения — разные операции». Это запрет повреждения, не обещание old-reader functional compatibility. `packaging/arch/README.md:106–112` описывает старый PR119 proof чтения прежнего профиля и refusal future preferences; он не добавляет обещание чтения будущей schema4.

| Proof | Actual f0 | Квалификация |
| --- | --- | --- |
| Package rollback к реальному old version | pacman downgrade; old normal ELF/title | Package rollback выполнен |
| Сохранение двух новых профилей и ключа | DB bytes/права/ownership + guarded source142 API count2/original/revision/metadata/same item | Preservation подтверждён |
| Candidate-return после old отказа | normal GUI text/file reopen + тот же API witness | Доступность новых данных current candidate подтверждена |
| Old GUI читает schema4 | generic storage refusal; old source guard `version > 3` | **Отказ чтения**, не PASS и не проверка lossless downgrade схемы |

**Вывод для root:** найденный исходный criterion не делает old-future-schema read дополнительным обязательным gate. Не нужно придумывать product fix для этого отказа или повторять GREEN data lifecycle. При этом отказ не переименовывается в успешный rollback GUI read. Для текущего NONFINAL candidate package остаётся actual live-hook proof ниже и root review его evidence. Для исходной **финальной поставки** отдельно нужен итоговый reviewed source/artifact freeze: нынешний source142 COLLECTED ID явно NONFINAL, `final_linux_acceptance=false`; данный proposal не превращает его в final release и не закрывает всю Linux acceptance.

Если root предъявляет отдельный явный criterion old GUI **читает эти два schema4 профиля**, f0 его не выполняет. Существующий `storage/copies/backup3.sqlite` создан normal GUI **до** этих двух imports, поэтому откат к нему теряет именно оба новых профиля (live schema4 при этом нужно сохранить). Нормальный current GUI `MaintenanceView` умеет Restore **в новый файл**, под тем же Secret Service key, без переключения/замены live DB. Этот путь не был выполнен в f0 и не является способом доказать сохранность двух профилей в старом GUI. Новый backup current DB сохраняет schema4, не преобразует её в schema3. Подтверждённого GUI lossless schema4→3 recovery/export пути для двух профилей нет. Нужны отдельный compatibility contract/реальный compatible artifact либо future product design; schema edit/forceversion/copy-over-live/drop/reimport вместо двух оригиналов не допускаются.

## 2. Что действительно сохранилось от #147 / #119

- #147: `tools/format4_gui_vm.py`, actual VM provenance и cleanup; UEFI QEMU/KVM с real guest PID1. Retained доказательства в `local/validation/secret-format4-gui/`; исходный source того run **cceede…**, не source142. Старый witness/его QEMU guard не меняется и в этом hook-only плане не запускается.
- Старый overlay `/var/tmp/codex-myrs/transient/secret-format4-gui` **удалён** accepted cleanup. Нельзя обещать reuse готовой runtime VM. Нужен **новый** overlay того же verified base, только после root approval.
- Existing runner нельзя вызвать unchanged: он делает `pacman -Syu`, ставит runtime и создаёт fixture user. Эти ветки **не входят** в proposal. Новый proposed entrypoint `tools/a4_package_hook_vm.py` пока отсутствует; implementation — только после review этого bounded contract. Повторно запускать `format4_gui_vm.py` / `arch_clean_vm_build.py` напрямую запрещено.
- #119: `tools/arch_versioned_package.py` и `arch_clean_vm_build.py` — reference actual package operations. Их smoke/ephemeral key/foreign nft/link/reboot/build/dependency setup **не переиспользуются**. Нет повторения rejected foreign-UID native campaign.
- Existing cached #147 image, .sig/public key и manifest SHA сверены сейчас под shared flock. `qemu-img info` readonly: no backing, dirty=false, corrupt=false. Никакой VM не запущено.

Полные пути/SHA/размеры и host tool owners: [readonly-inputs.json](readonly-inputs.json). Основные inputs:

| Input | Exact path / hash |
| --- | --- |
| Base image | `/home/myrs/projects/yonik/local/validation/vm-base/Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2`, SHA256 `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`, 576697856 bytes |
| Signature | тот же path + `.sig`, SHA256 `3dbdf7ffdc95654c47f5262df531dfa563733c72047915eff1245985f030b018` |
| Arch CI key | `…/vm-base/arch-ci-public-key.asc`, SHA256 `89c6940a0a204ce8d08d26320096a1eb9f87c416bcb3ca20b1001251340f0ff0`; primary `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F` |
| Current archive | `/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-mtree4/package.tar`, SHA256 `26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237` |
| Old archive | `/home/myrs/.t3/worktrees/yonik/t3-0e71b13a-c31bb75af8e3d15e/local/validation/arch-clean/yonik-0.1.0.r20261110-2-x86_64.pkg.tar.zst`, SHA256 `e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0` |
| Source binding | commit `14229843f9a11bf98ec4f5992df00f3247102ea2`, tree `7daa4ebb7cc25d3ceafab76809afc78753dc5ec3`; existing frozen/COLLECTED/lineage hashes в JSON |
| Firmware | `/usr/share/edk2/x64/OVMF_CODE.4m.fd` readonly; VARS копируется в own task. SHA обоих в JSON |

Runtime для **hook-only** — systemd/pacman/dbus/coreutils/sh/libalpm scripts **из самого signed base image**, не Qt/GNOME/Xvfb. Их точные guest versions/hashes deferred до разрешённого boot/preflight; наличие требуемого hook/script не объявляется по host копии. Guest preflight обязан снять `pacman -Q`, `pacman -Qo`, SHA и bytes действующих systemd/dbus hook files. Если base не содержит полного нужного пути — STOP с missing input; ни `-Syu`, ни closure download/copy host runtime, ни правка hook не разрешены. Наличие 147 GUI runtime packages не означает сохранённый runtime overlay.

## 3. Предлагаемая граница полномочий

**Root decision pending**, см. [execution-contract.json](execution-contract.json). Разрешение design не является разрешением boot.

Разрешаемый будущий delta: один новый own same-host-UID QEMU process с existing `/dev/kvm` access (сейчас metadata RW доступен; права не менять), fresh qcow2 overlay и root SSH **в disposable guest**; пять guest package transactions; live PID1/system-bus config reload **только от реальных hooks**. Стандартные guest boot/sshd/cloud-init services — bootstrap infrastructure #147. Это не обещание, что все системные процессы guest имеют UID0; тест не создаёт foreign-UID caller и не выполняет product under another UID.

Запрещены host daemon-reload/install/deps/settings/services; любые product GUI/helper/worker/check/core процессы/units; TUN/network probes, native issuer/permit/N5, caps/privilege changes, DMI/environment spoof, Rust/Go builds, downloads, mutable host mounts, Docker. Не копировать ни одну retained userdata/keyring/SSH private key старого run в VM. Source/package semantics неизменны. Не запускать или ослаблять #147 witness guard.

## 4. Exact future host bootstrap и guards

- [ ] Создать новый root mode0700 `/var/tmp/codex-myrs/transient/a4-mtree4-pid1-hooks-<nonce>`; сразу BSD directory flock EX до первого использования. Base directory shared flock на всё время QEMU references. Evidence — `local/validation/a4-package-hooks-142/<nonce>` вне TTL, mode0700. Отказ если root/evidence уже существуют, path symlink/UID mismatch.
- [ ] До эффектов сверить SHA всех JSON inputs, source/tree/frozen tooling/lineage/final=false, оба archive versions/vercmp, `.INSTALL` отсутствует, union unit paths ровно пять ниже. Existing unsigned accepted Yonik archives — тот же `LocalFileSigLevel=Optional TrustedOnly` **только для этих SHA**; это не новый signature seal. Signed image перепроверить GPG в **own** gnupg: import pinned public key, `gpg --status-fd 1 --verify image.sig image`, VALIDSIG exact primary, затем own agent reap. Host keyring не менять.
- [ ] `qemu-img create -f qcow2 -F qcow2 -b <exact base image> <task>/disk.qcow2 12G`; `qemu-img info --output=json` должен показывать exact backing. Base не открывать writable. Скопировать pinned VARS в `<task>/vars.fd`; CODE readonly. SSH key новый own0600, без вывода private bytes. `xorriso -as mkisofs -quiet -output <task>/seed.iso -volid cidata -joliet -rock <task>/seed`.
- [ ] Cloud-init: уникальный instance-id с nonce; root SSH authorized **public** key; `ssh_pwauth:false`, `disable_root:false`; **нет package_update/package_upgrade/packages/useradd/fixture**. В `bootcmd` создать `/etc/systemd/system/<каждая из пяти units> → /dev/null` (с отказом на существующую немаску); `runcmd` включает только guest sshd и создаёт `/run/a4-package-hooks-ready` с nonce. Никакого product unit enable/start/restart. Initial base не имеет Yon ik package до этих masks.
- [ ] QEMU argv — точный template в JSON: q35/kvm, CPU host, 2 vCPU/4096 MiB, nodefaults/no-user-config, display/monitor none, serial own file, no-reboot, own overlay/seed, pinned firmware. User network **restrict=on**, один `127.0.0.1:<reserved ephemeral port>→guest22`; никакого tap/bridge/9p/virtiofs/shared folders. Network outbound не нужен. SSH own key/known_hosts, ConnectTimeout3; ready deadline240s/interval5s, no endless retries. Писать progress не реже60s.

## 5. Exact guest preflight (через SSH, не на host)

До первого `pacman -U` root runner обязан связать SSH с own QEMU PID/starttime/argv/port/nonce и проверить:

```sh
cat /run/a4-package-hooks-ready             # exact nonce
cat /sys/class/dmi/id/sys_vendor           # QEMU, не подменять
readlink -f /proc/1/exe                    # /usr/lib/systemd/systemd
systemd-detect-virt --vm                   # qemu или kvm
systemd-detect-virt --chroot               # exit1; не chroot
systemd-notify --booted                    # exit0
stat -fc %T /sys/fs/cgroup                 # cgroup2fs; только read
pacman -Q systemd pacman dbus glibc
```

`systemctl show --property=Version --property=SystemState` и `/proc/1/stat`/boot_id связываются с настоящим PID1. `busctl --system` public DBus GetNameOwner/GetConnectionUnixProcessID для `org.freedesktop.systemd1` должен дать PID1. `dbus.service` loaded+active, CanReload=yes, MainPID>0; сохранить PID до/после, restart не допускается. Недоступный/nonreloadable dbus — STOP до package effects.

Пять masks (оба archive содержат этот exact список):

```text
yonik-helper.service
yonik-worker.service
yonik-worker.socket
yonik-worker-check.service
yonik-worker-check.socket
```

Для **каждой**: readlink `/etc/systemd/system/<unit>` == `/dev/null`, `systemctl show <unit> -p LoadState -p ActiveState -p SubState -p Job -p InvocationID` → masked/inactive/dead/no Job/no InvocationID; service MainPID=0. Product bus name отсутствует (`NameHasOwner`, не `StartServiceByName`/introspection). Нет product processes по `/proc/*/exe`. Маски остаются до shutdown; не удалять ради измерения. Не маскировать dbus/systemd/sshd. Архивные unit files не редактировать; masked LoadState **не** доказывает отсутствие archive payload — installed bytes сверять отдельно.

Hook preflight: `pacman -Qo` + SHA+сохранённые bytes для `/usr/share/libalpm/hooks/30-systemd-daemon-reload-system.hook`, `/usr/share/libalpm/hooks/dbus-reload.hook`, `/usr/share/libalpm/scripts/systemd-hook`. Требуются реальные trigger Install/Upgrade/Remove `usr/lib/systemd/system/*` → `systemd-hook daemon-reload-system` → `/usr/bin/systemctl --system daemon-reload`, chroot/boot guards; dbus hook → `systemd-hook reload dbus` → real try-reload-or-restart. При семантическом отличии — STOP/root review, не patch/fake/copy host script. `CanReload=yes` и неизменный dbus PID нужны, чтобы не расширить scope до restart.

## 6. Transactions и наблюдение live branch

- [ ] Скопировать через own SCP только SHA-pinned old/candidate archives в guest `/root/a4-package-hooks/input/`; пересверить SHA. Root/config/dbpath/log/cache/hookdir/gpgdir — **guest** filesystem, не host prefix/chroot. Предложенный config `/root/a4-package-hooks/pacman.conf`: `[options]`, `Architecture=x86_64`, `SigLevel=Required DatabaseOptional`, `LocalFileSigLevel=Optional TrustedOnly`. Repos не добавлять. `-dd` сознательно оставляет dependency resolution вне hook-only scope; ELF не запускаются, runtime dependency/GUI proof уже отдельный f0.
- [ ] Перед каждой фазой повторить masks/no processes/no product bus/no jobs; сохранить active units и property baselines. Запустить **readonly** real `busctl --system --json=short monitor` с `--match` для `Manager.Reload`, `Manager.Reloading`, `Manager.JobNew`/`JobRemoved` и `PropertiesChanged` dbus unit. Monitor-ready confirmation обязательна; один bounded monitor lifetime на phase, не fake manager. Экспортировать closed поля serial/sender/destination/interface/member/boolean/job unit/status; не environment/payload.
- [ ] Последовательно выполнить команды ниже. Каждая имеет phase-id/nonce, captured argv/PID/starttime/exit/stdout/stderr + monotonic time и before/after journal cursor. Нельзя вызывать ручной `systemctl daemon-reload` и выдавать его за hook.

```sh
# Все команды ТОЛЬКО через связанный guest SSH после guards.
pacman --config /root/a4-package-hooks/pacman.conf --root / --dbpath /var/lib/pacman --logfile /var/log/pacman.log --cachedir /var/cache/pacman/pkg --gpgdir /etc/pacman.d/gnupg --hookdir /etc/pacman.d/hooks --noconfirm -U -dd /root/a4-package-hooks/input/old.pkg.tar.zst
# upgrade
pacman --config /root/a4-package-hooks/pacman.conf --root / --dbpath /var/lib/pacman --logfile /var/log/pacman.log --cachedir /var/cache/pacman/pkg --gpgdir /etc/pacman.d/gnupg --hookdir /etc/pacman.d/hooks --noconfirm -U -dd /root/a4-package-hooks/input/candidate.tar
# rollback: та же первая команда с old.pkg.tar.zst
# candidate return: та же вторая команда с candidate.tar
# uninstall current candidate
pacman --config /root/a4-package-hooks/pacman.conf --root / --dbpath /var/lib/pacman --logfile /var/log/pacman.log --cachedir /var/cache/pacman/pkg --gpgdir /etc/pacman.d/gnupg --hookdir /etc/pacman.d/hooks --noconfirm -R -dd yonik
```

- [ ] Для каждой фазы нужен **совместный** proof: pacman exit0 и actual real hook line; **нет** `Skipped: Running in chroot`/`Current root is not booted`; Manager.Reload call и Reloading true→false в окне именно этой transaction, boot_id/PID1 стабильны; подходящий PID1 journal без reload error; dbus reload успешен, MainPID прежний; пять masks и zero product invocation неизменны. Других manual reload в окне нет. Если monitor miss или корреляция неоднозначна — INCONCLUSIVE, не повторять package operation слепо.
- [ ] Post-phase `pacman -Q yonik`/installed three ELF hashes/modes/root UID0 связывают old/new artifact с archive; после uninstall package отсутствует, ELF отсутствуют, masks остаются. Не повторять Qkk corruption/GUI/data proof. Проверить monitor JobNew/JobRemoved и journal: любой start job/invocation helper/worker/check/core — FAIL, остановить own VM, сохранить evidence; дальнейшие phases запрещены. Sysusers может создать только guest account metadata без запуска worker; это учитывать отдельно.

## 7. Evidence, stop и cleanup contract

Output вне TTL: `input-bindings.json`, source/COLLECTED/lineage + package metadata/ELF hashes, `qemu-command.json`, image/signature/GPG/firmware/tool bindings, guest boot/PID1/cgroup/masks/hook owners+bytes+versions, пять phase JSON/logs/closed bus traces/journal cursors, no-product-activation summary, final base hash, cleanup references. Не экспортировать SSH private key и не собирать host/user secrets. Не копировать retained data в guest: f0 continuity остаётся отдельным proof.

Любой precondition/input mismatch — **STOP до transaction** и exact gap; никакого install/download/build/другого image/root mapping/foreign-UID fallback. Любая операция с unknown exit — сначала readonly pacman/version/ELF/journal reconciliation, не повтор. Phase failure сохраняется **до** исправленного attempt; root review при изменении контракта. Общий run deadline900s; SSH methods bounded; no watcher.

Cleanup будущего own run: сохранить evidence outsideTTL до удаления; guest poweroff через own SSH, wait QEMU20s, затем только own PID с verified starttime/argv TERM15s/KILL при необходимости и reap; завершить monitor/SSH/SCP/GPG children. Проверить own cwd/exe/fd/maps references к task и base overlay, qemu image lock release; повторить base SHA. Сохранить результат/размер оставленного root при blocker. Удалять `shutil.rmtree` **только** canonical own task с marker/nonce/UID/path-prefix guard, под directory flock; никакой очистки старых caches/147/f0/private copies. Если уникальные результаты ещё внутри — не удалять. Освободить flock последним.

## 8. Review focus и текущий результат

Root review: (1) достаточно ли исходного preservation criterion без нового old-schema4-read требования; (2) fresh #147-base QEMU и guest root package hooks допустимы; (3) маски ровно всех service/socket paths и отсутствие product activation; (4) `-dd`/local accepted unsigned archives явно ограничены этим proof; (5) live Reload correlation не подменяется chroot skip/ручной командой.

Сейчас завершены только readonly hash/metadata/source/criterion проверки и этот concrete proposal. **VM не запускалась, reload/services/package operations не выполнялись, source/product/recipes не менялись.** Root должен отдельно принять execution contract/существующую авторизацию до любого bootstrap. Full A4/finalLinux остаются false, factory CLOSED. После разрешённого future hook run итогом будет ровно LIVE_HOOK_PROOF либо точный runtime input/manager blocker; не N5 admission и не final release seal.
