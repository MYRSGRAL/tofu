# N5: bounded emitter / compat delta после cfa8bc

Статус: доказаны локальные construction/emission edges; конечное
profile-specialized linking condition ниже **ещё не доказано**. Это результат
ограниченного статического RE, не successful filter load и не admission.
Frozen [f6fb proof](2026-11-14-n5-supported-implementation-bytes.md) и
[cfa8bc signed delta](2026-11-14-n5-libseccomp-signed-delta.md) не изменены.

[Manifest](../evidence/n5-supported-implementation/emitter-delta-manifest.json)
связывает exact signed installed ELF, upstream source references и покрытые edges.
[Командные записи](../evidence/n5-supported-implementation/emitter-delta-records.json)
содержат полные выбранные функции, raw data и декодированные op7 jump tables.
Все VA ниже — linked ELF addresses `/usr/lib/libseccomp.so.2.6.0`, SHA256
`5cd82027e6ab3dba31ee44200bb07f364a9f8e881d073fefbdf15a65cee8005c`.

Read-only получены primary upstream v2.6.0 `src/{api,arch,db,gen_bpf}.c`,
`arch-{x86_64,x32}.c`, `arch.h`, `db.h` с raw.githubusercontent.com/seccomp/libseccomp.
URL/hash каждого входа и exit codes в manifest. Снимки вне TTL:
`/home/myrs/.t3/worktrees/yonik/t3-4a4c738e-40d1ff75c85cc1eb/local/evidence/n5-emitter-source/`.
Это source-guided RE: tag/source не объявлены точным downstream build source.
Доказательства shipped behavior опираются на сохранённые instruction bytes,
а source помогает объяснить поля/контрольный граф. REA provider_unavailable,
зафиксированный в f6fb, не повторялся; setup/install не выполнялись.

## Точный universe и ожидаемый граф

Только native x86-64 little endian, retain=0, известные восемь namespace bits
из frozen signed systemd table, architectures ровно `[native]`. Другие ABI
рассматриваются только как исключаемые пути. Не требуется произвольная маска
будущих namespace типов или полная корректность всей libseccomp.

Условный ожидаемый граф, **если успешно построен, связан и загружен целиком**:

- Architecture filter без syscall rules: arch!=AUDIT_ARCH_X86_64 → badarch;
  native nr>=0x40000000 и nr!=-1 → badarch; остальные native → ALLOW.
- Namespace filter: setns(308), clone3(435) имеют безусловный action, arg_count=0;
  unshare(272)/clone(56) имеют OR восьми `(args[0] & f)==f`, action из systemd.
  Нет требования перебрать setns по типам. Аргумент с любым известным bit
  совпадает с одним правилом; произвольные остальные bits не снимают совпадение.
- Namespace default ALLOW — нормально для такого deny filter. Badarch ALLOW
  namespace contexts не заменяет отдельный architecture restriction filter:
  ABI exclusion обосновывается именно успешно загруженным native-only filter.

## Mask/value → native64 nodes → emission

1. cfa8bc уже сохранил exact-array → db_col_rule_add: op7 сохраняет оба datum
   words. В `db_rule_add` `0xc234–0xc245` arch.size=64 выбирает `0xc630`.
   `0xc69d–0xc6a5→0xc912` пропускает op7 лишь при **всей** нулевой 64-bit mask;
   известные flags ненулевые. Это не ошибочное усечение к low mask.
2. `0xc6ed–0xc6f8` читает 64-bit mask/value; `0xc757–0xc7c3` разделяет их на
   high/low, маскирует datum через AND и выставляет op7 обоим nodes.
   True link high→low записывается в `0xc7ae`. Для flags f<2^32 high mask/value
   равны 0, значит high comparison — tautology, low остаётся `(A & f)==f`.
   Ненулевые верхние 32 бита syscall argument не создают ложного bypass:
   они маскируются нулём. Это требуемое поведение MASKED_EQ, не требование
   `args[0] == f` на всех 64 битах.
3. `.data.rel.ro 0x31c40` буквально содержит token/token_bpf `0xc000003e`,
   size=64, endian=1; pointer fields rule_add/syscall_rewrite NULL.
   `0xc736–0xc73e→0xca94` для endian=1 вычисляет high=8*arg+20,
   low=8*arg+16. Для systemd native clone/unshare arg0 это offsets 20 и 16,
   соответствующие high/low первого uint64 args после nr/arch/IP.
   Здесь нет s390 swap и нет преобразования arg0 в pointer-width compat arg.
4. Emitter `_gen_bpf_chain.isra.0` инлайнит source `_gen_bpf_node`:
   `0x556c–0x5595` сверяет offset и `(~old_mask & new_mask)==0`.
   Несовместимый offset/mask → reload BPF code 0x20 (LD W ABS), K=arg_offset;
   совместимый accumulator переиспользуется лишь когда не потерял нужные bits.
   `0x5632–0x56af`: при отличающейся mask emits code 0x54 (AND K), K=node.mask.
   Для op7 `0x56d0→0x5b10→0x5b19` emits code 0x15 (JEQ K), K=node.datum.
   На little endian bswap ветви не выбираются. Из этих инструкций следует
   сохранение локального predicate: `(loaded_word & mask)==(datum & mask)`.
5. Node true/false targets переносятся в typed symbolic instructions
   `0x572c–0x57b1`; terminal action blocks создаются с code 0x06 (RET K),
   например `0x5847–0x58ad`, K=act_t. Поэтому action не выводится из errno,
   а следует из systemd action input, node edge и RET construction.

## Relevant optimizer: что проверено и где предел

Primary `db.c:461–657,681–818` описывает prune/add; native ELF функции
`_db_tree_prune.isra.0 0x3280`, `_db_tree_add 0x38c0` сохранены полностью.
`0x3520–0x3534` проверяет и datum, и mask после совпадения op; разные низкие
one-bit predicates не считаются одинаковыми только по op или arg index.
Op7 dispatch tables декодированы из raw `.rodata` (VA=file offset подтверждён
readelf): `0x18120[7]→0x36f0`, `0x18140[7]→0x3750`,
`0x181a0[7]→0x3970`, `0x181c0[7]→0x3b40`.
Общие high nodes (mask=datum=0) допускают общий true subtree, а low nodes с
разными f упорядочиваются по datum и сохраняют отдельные предикаты.
Это source-supported объяснение выбранных binary comparisons, не доказательство
всех recursive mutation states prune/add.

Accumulator optimization тоже имеет явно проверенную локальную предпосылку:
смена low bit f→g не может использовать A&f для g, потому что
`(~f & g)!=0` при разных one-bit masks; emitter обязан reload. Повторный
high mask=0 может переиспользоваться, поскольку результат всё равно 0.
Это закрывает конкретный возможный loss-of-mask edge на соседних known bits.

## Native-only: default, allowlist, i386 и x32

Signed systemd `seccomp_restrict_archs 0x216930` (f6fb records) начинает
`seccomp_init(ALLOW)`; native уже включён libseccomp. В native-only input
не добавляет i386/x32 через arch_add; вместо этого помечает их blocked в
local_archs. Совпадение native пропускается, x32-native exception неприменим
к установленному `seccomp_arch_native()==0xc000003e`.

`db_col_reset.part.0` `0x3f7b–0x3f92`, `0x4067–0x409c` загружает defaults
из `0x18960`, подставляет только act_default, копирует attrs с context+4;
act_badarch (context+8) остаётся 0 = KILL_THREAD, а default — ALLOW.
`0x40c0` выбирает arch_def_x86_64. Source `db.c:1067–1085` согласуется с
этой инициализацией. Нет source assertion, что все другие filter contexts
должны иметь тот же badarch: systemd namespace init специально меняет его.

В `gen_bpf_generate` source `_gen_bpf_arch` заинлайнен:

- `0x76ec–0x77ae` constructs JEQ K=arch.token_bpf, true=architecture head,
  false=TGT_NXT. В единственном native context i386 token `0x40000003`
  не совпадает; source top-level добавляет badarch tail (`gen_bpf.c:2076–2080`).
- При отсутствии secondary x32 context `0x7a35→0x7c7e` строит native guard.
  `0x7c9b–0x7ca0`: BPF JGE K=0x40000000, false→syscall/default head.
  `0x7d3b–0x7d99`: BPF JEQ K=0xffffffff, true→same head,
  false→bad_arch_hsh. -1 — explicit tracer-skip exception, не namespace syscall;
  остальные номера с x32 bit идут к badarch symbolic target.
- Пустой syscall list architecture filter не отменяет guard: head=NULL
  подставляет default hash в `0x7db8→0x7cf8` и `0x7dab→0x7ad8`.
  Поэтому ALLOW default не является source-level разрешением x32/i386.

Эти branches подтверждают **construction условного graph**, но не финальные
serialized jump destinations. Последний предел не замещается property `[native]`.

## Остаточное статическое условие, а не новый общий UNKNOWN

Артефакты для дальнейшего RE доступны; missing archive/dependency здесь нет.
Один оставшийся составной invariant для именно этого профиля:

> После prune/add восьми one-bit rules и symbolic linking все low-predicate
> true edges всё ещё достигают выбранного RET, false edges продолжают OR;
> i386 mismatch и x32 guard false достигают badarch RET. Accumulator-elision
> при переносе/shared blocks не удаляет нужный reload/AND.

Точные границы: `_gen_bpf_chain_lvl_res 0x5d90` и inlined block assembly /
accumulator elimination `gen_bpf_generate 0x7dc5–0x8176`, совместно с recursive
prune/add states `0x3280/0x38c0`. Проверены локальные emitter/comparison edges,
но полного profile-specialized доказательства этого invariant пока нет.
Минимально неразличимое условие — symbolic target может быть верным до assembly,
а результат sharing/elision/jump rewrite ещё не удостоверен до final instruction.
Ни найденный opcode, ни совпадение upstream tag не устраняют эту разницу.

Проверенная нижняя граница final packing: `0x8307–0x8344` копирует opcode,
immediate jt/jf и K в sock_filter; `0x836f` отвергает неразрешённый target type,
обнуляет/освобождает output. `_gen_bpf_build_jmp 0x5040` ищет target hash,
рассчитывает расстояния, при дальних переходах вставляет JA; весь код сохранён.
Это исключает утверждение «любой symbolic target молча serializes», но не
заменяет недостающий invariant. Для закрытия нужен статический разбор именно
этой специализации до terminal edges; export/выполнение библиотеки не требуются
и здесь не выполнялись. Универсальное доказательство всех ABI/операторов не нужно.

## Ошибки и независимые runtime gates

libseccomp `seccomp_load 0x96d0`: generation call `0x998c`, отрицательный result
уходит в error paths, не означает установку. Ошибка final packing `0x836f`
доходит до caller как negative code; normalize ветви не превращают её в load.
В frozen systemd namespace rule error выбрасывает architecture context;
часть load errors логируется и функция возвращает 0. Architecture function
также имеет nonfatal skip path. Следовательно даже завершённый static proof
даёт semantics только **успешно применённой полной** политики, не свидетельство,
что конкретный caller её получил.

Loaded PID1/executor/libseccomp bytes, dynamic resolution, actual successful
filter load и authenticated pending invocation report с общим nonce/digest/
session/generation/invocation/custody остаются отдельными runtime gates root
minimal contract. New16/callers NOT AUTHORIZED; errno не authority. Никаких
runtime/target/probes/seccomp_load/export/ptrace, новых tools/deps, builds,
unit activation, kernel RE или host trust изменений не выполнялось.

Проверка delta: exact installed SHA256, новые JSON parse/records hash/source
hashes, relative docs links, frozen files byte-equal cfa8bc/f6fb, staged diff check.
Только новые docs/evidence; source inputs 204 КБ вне TTL, без transient
ELF/build tree и без исполнявшихся target процессов.
