# N5: входной manifest supported-profile для отдельного completeness review

Статус: INPUT ONLY. Это не permit, attestation установленного фильтра или
final readiness. Нормативная цель — actual supported enforcement всех namespace
transitions в явно ограниченном profile. Универсальная per-type setns/paired
matrix не требуется. Production/shared API не меняются.

База: `9f02440f1f4c6c05d585a6106c6dd149f8debc48`.
PR180 остаётся `b3a2838243cfefd3b5989b449956ad309ed270a1` на
`fix/n5a-authorized-lifetime`; этот manifest не включает его diff.
Канонический root contract:
`/home/myrs/projects/yonik/local/orchestration/proposals/n5-minimal-contract/contract.md`.
Хеш контракта на момент сбора и точные outputs команд находятся в
[local-inputs.json](../evidence/n5-supported-profile-inputs-20261114/local-inputs.json).
Локальные источники вне TTL; абсолютные пути ниже — входы на данной машине,
не обещание их доступности в checkout reviewer.

## Идентичность кандидата и provenance

| Поле | Доказанное локальным чтением | Ограничение / оставшийся proof |
| --- | --- | --- |
| systemd | pacman `262-1`, arch `x86_64`, Installed From `core`, packager Christian Hesse, Validated By `Signature`; `systemd --version` записан с feature flags | Это metadata локальной базы, не повторная cryptographic verification пакета и не версия живого PID1 |
| libseccomp | pacman `2.6.0-1.1`, arch `x86_64_v3`, CachyOS packager, Installed From `None`, Validated By `Signature` | Точный repository происхождения UNKNOWN; не подменять `None` догадкой. Нет доказанного mapping downstream patches → upstream/source |
| Артефакты | SHA256 `/usr/lib/systemd/systemd` и resolved `/usr/lib/libseccomp.so.2`, ownership через `pacman -Qo`, SHA256 package desc в JSON | Не доказывают, что PID1 загрузил именно эти bytes/lib, либо что bytes совпадают с подписанным archive |
| Kernel/ABI | Кандидат ограничен native Linux x86_64 LP64, согласно уже reviewed New16v2 | Exact installed kernel provenance/validation не пересобирались. Host package tuple не равен historical VM tuple; поддержка exact deployment UNKNOWN |

Assumptions: локальная pacman DB и чтение файлов не скомпрометированы; trusted
installed launcher и immutable engine входят в принятый root trust model.
Ни root adversary attestation, ни exec monitor, ни proc-engine-exe gate сюда
не добавляются. Различать package architecture `x86_64_v3` и syscall ABI.

## Protected unit/template/launcher contract

Нормативный вход — четыре точных файла на базе manifest:
[Run service](../../packaging/systemd/yonik-worker.service),
[Run socket](../../packaging/systemd/yonik-worker.socket),
[Check service](../../packaging/systemd/yonik-worker-check.service),
[Check socket](../../packaging/systemd/yonik-worker-check.socket).
Это fixed units, не произвольный instance template. Их SHA256 записаны в JSON.

[Canonical verifier](../../crates/yonik-linux/src/helper/systemd_worker.rs)
(`verify_installed`, `trusted`, actual property validation) задаёт:

- Компоненты trusted пути root-owned, без group/other write, no-follow;
  четыре installed файла `/usr/lib/systemd/system/` равны embedded bytes.
  Launcher `/usr/lib/yonik/yonik-worker` — trusted regular executable.
  Эта проверка прав сама по себе не является digest pin launcher.
- PID1 identity, canonical FragmentPath, отсутствие DropInPaths, exact
  ExecStart `/usr/lib/yonik/yonik-worker --run` (Check: `--check`), fresh
  invocation/MainPID/starttime/pidfd, fixed UID и cgroup проверяются supervisor.
- Run: User/Group `yonik-worker`, Type=exec, NNP=true, NET_ADMIN-only
  bounding/ambient, PrivateNetwork/PrivateDevices=false, `/dev/net/tun rw`;
  Check имеет отдельные ограничения и не заменяет Run evidence.
- `RestrictNamespaces=yes` в unit соответствует actual D-Bus allowed-mask
  `t 0`; architectures ровно `[native]`. Защита файлов/устройств, kill/cgroup
  contract и limits остаются полным контрактом файлов/verifier, а не только
  перечисленными здесь свойствами.

UNKNOWN: installed совпадение этих units/launcher, exact pinned immutable engine,
actual properties до/после await и authenticated report именно pending Run.
Здесь unit не активировался, properties живого worker не снимались. Required proof:
привязать exact artifact hashes и actual observation к одному nonce, digest,
session/generation/network epoch, invocation и retained net/pid custody по root
contract; metadata этого документа такого binding не создаёт.

## Уже проверенная filter semantics и пределы

Сохранённый review:
`/home/myrs/projects/yonik/local/orchestration/proposals/kernel-enforcement-probe-v2/README.md`,
раздел «systemd 262 и граница authority». Primary snapshot:
`.../kernel-enforcement-probe-v2/sources/seccomp-util.c`, SHA256
`0ef3cb61a294b7a9d468892a260403b1e9a93726a2cab87351b617f403966ae8`;
[upstream v262](https://github.com/systemd/systemd/blob/v262/src/shared/seccomp-util.c),
`seccomp_restrict_namespaces` (1378–1515), `seccomp_restrict_archs` (2016–2088).
Это сохранённый source input; новый upstream поиск не проводился.

При retain=0 source задаёт argument-independent EPERM для setns, MASKED_EQ
для каждого известного namespace flag в unshare/legacy clone и ENOSYS для
clone3. Поэтому отдельные setns runtime вызовы по типам не нужны для доказательства
этой source semantics; combinations с известным запрещённым bit покрывает mask.
Неизвестный будущий namespace bit без известного bit нельзя автоматически считать
покрытым loop по namespace_info: новый тип требует review, иначе unsupported.
Legacy clone(time) — raw selector, не тест создания time namespace.

Native-only filter source предназначен исключить non-native ABI. Но source
имеет skipped rule/load errors; возврат функции и property не гарантируют
успешную установку. UNKNOWN: downstream systemd/libseccomp implementation mapping,
libseccomp rule translation/backend и фактическая загрузка native-only + namespace
filters в exact invocation. Нельзя объявлять i386/x32 закрытыми только по строке
`native`. aarch64/s390/compat ABI и другие версии не входят в кандидат.

## Existing observations и точный остаток

[Историческая VM campaign](2026-11-11-worker-anchor-production-gates.md)
ссылается на сохранённые logs в
`/home/myrs/projects/yonik/local/evidence/worker-anchor-production-4a4/`:
systemd **262.1-1-arch**, kernel **7.2.11-arch1-1**, control/filtered 8-bit unshare,
setns(-1,0), clone3(NULL,0). Это другой tuple, не local 262-1 certification.
Old3 ограничен тремя errno и custody; New16 source/compile evidence не является
actual authenticated production report. Ни один probe в этой задаче не запускался.

Оставшийся proof для completeness review (пока UNKNOWN):

1. Связать package/binary hashes с exact supported deployed implementation,
   включая downstream patches libseccomp и systemd; обосновать native-only ABI
   exclusion и полный namespace flag universe для поддержанного kernel.
   Не требуется повторный kernel/image RE или поиск exact PKGBUILD в этой задаче.
2. Показать успешное применение полной политики в exact authenticated pending
   invocation совместно с уже reviewed New16/old3 evidence в их пределах.
   Числа EPERM/ENOSYS отдельно не отличают outer sandbox, capability denial,
   invalid args или unsupported syscall. Дополнительный control допустим только
   для конкретной оставшейся неразличимости после отдельного согласования scope,
   а не автоматически для каждой строки/типа.
3. Отдельно закрыть owner→issuer→factory→owned ATTACH→actual health,
   receiver verifier и revoke/cleanup по root contract. Это следующие scopes,
   не результат данного manifest. UNKNOWN остаётся fail-closed, без fallback.

## Проверка и границы сбора

Только новые Markdown/JSON; `git diff --check`, JSON parse, сверка base/ref,
SHA256 входов и exit codes metadata-команд. Dependencies/install, build,
syscall probes, новые callers, VM/cross-UID, activation и caps/unsafe не выполнялись.
Нет build-каталога или transient артефактов. Review должен оценить достаточность
входов и перечисленные UNKNOWN; принятие документа не открывает production gates.
