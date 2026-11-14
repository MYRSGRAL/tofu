# N5: bounded installed implementation / provenance

Статус: частичное static evidence; production admission закрыт. База
`0fc3442b45a2cc633f736b28d11c9863f7d35533`. Единственный fetch — origin main.
PR189/7ce и PR188/229 не изменялись. [Accepted182 input](2026-11-14-n5-supported-profile-inputs.md)
(`b0b3aaa7a24e14e5c49962c358f294b6b7dceb25`) — CLEAN INPUT ONLY;
его metadata review не повторяется. Канонический контракт:
`/home/myrs/projects/yonik/local/orchestration/proposals/n5-minimal-contract/contract.md`.

[Manifest](../evidence/n5-supported-implementation/manifest.json) связывает hashes,
package comparison, source-reference и UNKNOWN. [Статические записи](../evidence/n5-supported-implementation/static-records.json)
содержат argv, exit code и полный вывод выбранных функций; relocation inventory
ограничен явно указанными symbol references. Адреса — ELF linked VA, не live VA.

## Новые доказанные edges

1. Cached `/var/cache/pacman/pkg/systemd-262-1-x86_64.pkg.tar.zst` → подпись:
   gpg exit 0, GOODSIG/VALIDSIG signing fingerprint
   `0429897DE5F3BDAC537A30696D42BDD116E0068F`, primary
   `02FD1C7A934E614545849F19A6234074498E9CEE`, Christian Hesse / Arch Linux,
   TRUST_FULLY локального pacman keyring. Trust anchor здесь существующий keyring;
   независимая перепроверка его bootstrap не заявляется. Warning ownership и
   nonwritable trustdb сохранены. Никакие ключи не импортировались.
2. Signed package members `usr/lib/systemd/systemd` и
   `usr/lib/systemd/libsystemd-shared-262-1.so` → installed files: SHA256 равны
   побайтно, оба extraction exit 0. Извлечение потоковое в память, без checkout
   ELF на диске. `.PKGINFO`/`.BUILDINFO` сохранены как package statements,
   не как воспроизводимая source→binary сборка.
3. Installed shared ELF → bounded implementation ниже: exported symbol extents,
   instruction bytes и GOT relocation identities сохранены. Это прямая проверка
   shipped implementation, не предположение о downstream по upstream version.
4. Installed `libseccomp.so.2.6.0` → `seccomp_arch_native` в `0x9240` буквально
   возвращает `0xc000003e` (x86-64 audit architecture). Package architecture
   `x86_64_v3` из installed DB не равна syscall ABI. Exact library hash в manifest.

## Bounded byte-level semantics

`seccomp_restrict_namespaces` starts `0x214cb0`, size `0x6d4`:

- `0x214da6–0x214db4`: clone3 selector 435, action `0x50026`, arg_count=0.
- `0x214dc1–0x214def`: retain & `0x7e020080` == 0 выбирает setns selector 308,
  action `0x50001`, arg_count=0. Это argument-independent запрос правила:
  не зависит от fd, nstype и валидности аргументов. Числовой errno не authority.
- `0x214ee8–0x214ff6`: iteration namespace_info, flag с offset +16, forbidden
  mask `~retain`; для unshare 272 и clone 56 строится comparator arg0/op7 с
  одинаковым mask/value=f. Source-reference расшифровывает op7 как MASKED_EQ;
  машинные байты подтверждают construction, не полный backend compiler.
  Ветка s390 существует, но не переносится в supported x86-64 claim.
- `namespace_info` `0x596d20`, stride `0x28`, восемь flags:
  `0x02000000,0x08000000,0x40000000,0x00020000,0x20000000,0x10000000,0x04000000,0x80`,
  затем sentinel. OR = `0x7e020080`. Комбинации с известным запрещённым bit
  покрыты предполагаемой MASKED_EQ semantics; новый bit сам по себе не покрыт.
- Отрицательные rule results ведут к cleanup/следующей architecture, например
  `0x214df8→0x214dfe→0x214e43`; load/error path расположен `0x215170` и далее.
  Возврат 0 через `0x214e80→0x214d00` не удостоверяет все loads.

`seccomp_restrict_archs` `0x216930`, size `0x242`: сравнивает с native,
учитывает x86-64/x32 special case (`0xc000003e`, `0x4000003e`), исключает
невыбранные local architectures и вызывает attr_set/load (`0x216a9e–0x216ac5`).
`0x216ad7–0x216b51` содержит logging и success return для части load failures.
Property `[native]` и success return не доказывают compat exclusion.

libseccomp exact-array entry `0xdf40` проверяет arg_count<=6, context magic,
architecture count; `0xdff3` вызывает `db_col_rule_add` с exact flag=1.
`seccomp_load` `0x96d0` содержит generation (`0x998c` → gen_bpf_generate),
prctl fallback (`0x98a0–0x98af`) и error normalization. Записи этих функций
сохранены для дальнейшей проверки; наличие disassembly не означает доказанную
семантику всех внутренних compiler branches. В частности, end-to-end MASKED_EQ
и x32 syscall-bit dispatch остаются UNKNOWN.

## Конкретный остаток

- Отсутствуют cached official signed `libseccomp-2.6.0-1.1-x86_64_v3.pkg.tar.*`
  и detached `.sig`: read-only поиск `/var/cache` не нашёл их. Installed DB
  `%VALIDATION%=pgp` не заменяет доступный signed archive→ELF proof.
- Exact downstream source/build chain обоих пакетов не установлен; upstream
  seccomp-util.c snapshot (hash в manifest) только reference для интерпретации.
  Доказанные выше construction edges получены непосредственно из ELF.
- Полная libseccomp backend translation (MASKED_EQ, ABI mapping, badarch action,
  x32 bit), включая сгенерированный BPF, ещё не доказана. Требуется authenticated
  archive/source chain либо дальнейшее bounded RE внутренних generator paths;
  никаких новых dependencies для этого не устанавливалось.
- Байты реально загруженных PID1/executor/libseccomp отдельно UNKNOWN. On-disk
  package match не означает live mapping и не проверяет dynamic symbol resolution.
- После закрытия static UNKNOWN всё равно требуется authenticated observation
  exact pending invocation: успешное применение полной native-only + namespace
  политики, actual properties до/после, тот же nonce/digest/session/generation/
  invocation и retained net/pid custody по root contract. Reviewed New16/old3
  использовать только в пределах их покрытия. Errno без этого binding не permit;
  универсальная per-type setns matrix не требуется. Runtime16 NOT AUTHORIZED.

REA inspect_binary_layout вернул provider_unavailable: отсутствует настроенный
REA_PWNTOOLS_PYTHON с pwntools 4.15.0 / pyelftools 0.33 / Unicorn 2.1.2.
Fallback — существующие objdump/readelf; никакой установки. Kernel RE не повторялся.
Только новые docs/evidence; production/shared API не менялись. Ни probes, ни units,
ни builds/VM/hostinstall/caps/ptrace/crossUID не запускались. Transient directory
не создавался, единственные evidence находятся в workspace без TTL.
