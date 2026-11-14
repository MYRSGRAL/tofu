# N5: exact signed CachyOS libseccomp — отдельный delta

Продолжение `f6fb2ea1115e175a3cec0f62a14279ee7c4e891f`, та же task branch.
[Предыдущий bounded proof](2026-11-14-n5-supported-implementation-bytes.md) и его
manifest/static records заморожены без изменений. Этот delta закрывает отсутствие
signed libseccomp archive и exact installed bytes mapping; полное supported
implementation proof пока **не закрыто**, source/admission gates закрыты.

## Provenance → installed bytes

[Delta manifest](../evidence/n5-supported-implementation/libseccomp-delta-manifest.json)
и [командные записи](../evidence/n5-supported-implementation/libseccomp-delta-records.json)
содержат metadata, подпись, package build statements и выбранный disassembly.

Сначала read-only `pacman -Si cachyos-core-v3/libseccomp` и exact entry
`libseccomp-2.6.0-1.1/desc` существующей sync DB дали version, architecture,
filename, размер 103177 и SHA256. `/etc/pacman.conf` включает
`/etc/pacman.d/cachyos-v3-mirrorlist`; его действующая Server строка
`https://us.cachyos.org/repo/$arch_v3/$repo` определила источник. Sync refresh
не выполнялся. DB metadata — locator и дополнительная hash cross-check;
основная authenticity authority — отдельная package signature существующим keyring.

Один bounded download-пакет, без альтернативных версий/общего поиска:

- [Exact package](https://us.cachyos.org/repo/x86_64_v3/cachyos-core-v3/libseccomp-2.6.0-1.1-x86_64_v3.pkg.tar.zst): HTTP 200, 103177 bytes.
- [Detached signature](https://us.cachyos.org/repo/x86_64_v3/cachyos-core-v3/libseccomp-2.6.0-1.1-x86_64_v3.pkg.tar.zst.sig): HTTP 200, 438 bytes.

Архив SHA256 `68a684f61f9d3a3fbe1ae9392eebf55fbb71cf0799dd6b9d620070360a89c4f7`
совпал с exact DB entry. GPG exit 0, GOODSIG/VALIDSIG и TRUST_FULLY для
`882DCFE48E2051D48E2562ABF3B607488DB35A47`, CachyOS admin@cachyos.org.
Использован `/etc/pacman.d/gnupg`, без импорта/TOFU/смены trust.
Warnings unsafe ownership/nonwritable trustdb сохранены. Trust bootstrap keyring
отдельно не аттестуется. Detached signature сохранена также base64 в manifest.

Signed member `usr/lib/libseccomp.so.2.6.0` потоково прочитан через bsdtar exit 0;
SHA256 `5cd82027e6ab3dba31ee44200bb07f364a9f8e881d073fefbdf15a65cee8005c`
равен installed `/usr/lib/libseccomp.so.2.6.0` и frozen f6fb input. Таким образом,
точные байты ранее проанализированной библиотеки теперь имеют signed CachyOS
package provenance. `.PKGINFO` и `.BUILDINFO` из того же authenticated archive
сохранены: pkgbuild_sha256sum `19d33fec402794ae80c49dafbea44506addc6429c12eb4aec136cd837502b910`.
Сам hash PKGBUILD не доказывает exact source/build reproduction.

Минимальные inputs вне TTL, абсолютный local prefix:
`/home/myrs/.t3/worktrees/yonik/t3-4a4c738e-40d1ff75c85cc1eb/local/evidence/n5-libseccomp-download/`.
Там package, detached signature, download.json; это небольшие сохраняемые inputs,
не extracted/build tree. ELF читался в память, на диск не извлекался; transient
каталог и активные build/target процессы не создавались. В Git — только новые
текстовые evidence/manifest/docs, без архивов.

## Bounded translation edges: observation и предел вывода

Exact signed ELF имеет local symbols, поэтому дополнительные edges получены
через существующий objdump без запуска библиотеки. Linked VA не runtime address.

- `abi_syscall_rewrite` `0x13080–0x130c1`: два диапазона отрицательных pseudo
  selectors переводит через поля arch definition; неотрицательный selector
  возвращает успех без изменения (`0x130b8→0x13093`). Это ограниченный rewrite
  edge, а не доказательство всех ветвей rule insertion.
- `x32_syscall_resolve_name_munge` `0xea90–0xeab1`: resolver callback + OR
  `0x40000000` для nonnegative результата, сохранение отрицательной ошибки.
  `x32_syscall_resolve_num_munge` `0xeac0–0xead6`: снимает тот же bit перед
  reverse resolver, отрицательный input сохраняет. Native token из frozen
  `seccomp_arch_native` — `0xc000003e`; x32 — отдельная обработка syscall bit,
  поэтому одной package architecture `x86_64_v3` недостаточно для ABI exclusion.
- Frozen `db_col_rule_add` `0xd711–0xd74a`: comparator op7 идёт через
  `0xd840→0xd746`, сохраняя оба исходных 64-bit datum; обычные op1..6 перед этим
  заменяют mask word на all-ones. Это байтовое подтверждение сохранения
  mask/value для MASKED_EQ input. Новый `db_rule_add` содержит отдельную ветвь
  op7 и преобразование 64-bit comparison nodes; полный optimizer/emitter proof
  здесь не заявляется.
- Frozen `gen_bpf_generate` содержит native/x32 dispatch (`0x7a35` и далее),
  включая конструирование сравнения с `0x40000000` в native branch
  `0x7c7e–0x7ca5`. Наличие этой ветви **не доказывает** её достижимость и полный
  terminal action graph для canonical native-only filter. Это оставшийся
  конкретный static edge: architecture set/default badarch + syscall-bit branch
  → final reachable BPF actions, вместе с mask comparator → AND/EQ emitted code.

## Оставшиеся UNKNOWN и authenticated invocation proof

Закрыт edge signed libseccomp package → installed ELF; cache-only blocker снят.
Не закрыты exact downstream source/build reproduction, полный MASKED_EQ backend
и native-only i386/x32 exclusion. Следующая статическая проверка должна проследить
конкретные comparator/architecture nodes через emitter и final jump resolution;
повтор package lookup и kernel RE для этого не нужны. Записанный disassembly сам
по себе не равен доказанной end-to-end filter semantics.

Loaded PID1/executor/libseccomp bytes и dynamic resolution отдельно UNKNOWN.
Даже после закрытия static edges требуется authenticated proof успешного полного
filter load именно pending invocation с actual properties до/после и общим
nonce/digest/session/generation/invocation/retained net/pid custody по root
minimal contract. Old3/New16 — только в пределах reviewed evidence, errno не
самостоятельная authority; setns argument-independent rule и mask semantics не
требуют универсальной per-type matrix. NEW16 runtime/callers NOT AUTHORIZED.

Проверки: parse обоих новых JSON; download status/hash/signature; package member
hash = installed hash = frozen hash; records hash; docs links; frozen files
побайтно равны f6fb2ea; staged diff --check и scope только новых файлов.
Host install, tool/dependency changes, target/filter execution, builds, units,
VM, crossUID, capabilities и ptrace не выполнялись.
