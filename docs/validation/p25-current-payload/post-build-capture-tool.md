# Bounded post-build capture: контракт будущего вызова

Новый `tools/p25_capture_build_closure.py` читает существующие metadata и файлы.
Он не исполняет commands из records, не запускает build/compiler/version query,
не меняет builder/controller/package и не является production admission gate.
Source142, frozen c005/40c/abe778 и actual142 не повышены до P25/A5 PASS.

Использован test-driven-development: первоначальный импорт нового tool RED;
после реализации baseline GREEN. Отдельный RED на отсутствие generated SPDX
показал ошибочный CAPTURED; теперь такой случай явно MISSING. Fixtures полностью
синтетические: файл `elf` не считается настоящим ELF, tools никогда не исполняются.

## Trust contract

Caller до вызова независимо проверяет final build record/source archive/closure,
замораживает roots и устанавливает pin нормализованного record. Tool проверяет
байты против этого pin и whitelist hashes; **он не создаёт authority для record**,
не проверяет Git tree или SDK publisher, не доказывает полноту переданного списка
translation units и не выводит соответствующие исходники из ELF. Поля commit/tree/
archive_sha256 — externally verified source identity claims; tool не выдаёт их
за independently established provenance. Существующий provenance verifier нужен
по-прежнему. Подстановка нового самосогласованного record вместе с новым pin —
смена доверенного входа, не обход existing admission.

Только три явных непересекающихся absolute root: source/build/SDK. Все ancestors
от `/`, subdirectories и final files открываются через FD-relative NOFOLLOW;
regular files проверяются fstat до/после чтения. Нет glob/recursive scan, HOME,
key/config discovery, symlink traversal, source extraction или engine execution.
`/` и сам HOME не принимаются как root; private `.ssh/.gnupg/.config/.aws/.env`
path segments отвергаются. Source-root файлы должны точно совпасть с
caller-pinned tracked_inputs whitelist. SDK/build inputs — конечный whitelist
`files`; metadata refs не могут добавлять произвольные пути. Mutable hostile
same-UID writers не входят в модель: нужен завершённый frozen build без активной
записи. Файл проверяется в момент чтения; record дополнительно перечитывается
перед output. Это не filesystem snapshot/lock и не обещание отсутствия изменений
после чтения. Input roots и record не изменяются.

Лимиты в коде: 4096 entries на список, JSON record/dep/query ≤1 MiB, source/header/
primary ≤8 MiB, generated ≤16 MiB каждый **и суммарно**, ELF/tool ≤512 MiB каждый,
все чтения вместе ≤1 GiB (включая повторное чтение record). ELF/tool хешируются
потоком по 64 KiB, DWARF не разбирается. Source content не сохраняется в RAM после
hash; headers сохраняют только первые 16 KiB для SPDX/copyright. Это может дать
MISSING на позднюю атрибуцию; поиск не расширяется автоматически. Generated UTF-8
bytes сохраняются в report как JSON string (обратный UTF-8 даёт исходные bytes),
с SHA и generation inputs/generator refs. Primary bytes остаются в независимо
retained source offer; report сохраняет exact refs/hashes, а не копию всех notices.

Нарушение структуры/pin/hash/symlink/type/size/изменение входа — REJECTED, exit 2.
Отсутствующий файл/identity/query/depfile/generated record/CXX invocation/source
ref/SPDX/primary text ref — MISSING, exit 1, с точными IDs причин. CAPTURED, exit 0,
означает только согласованный capture **перечисленных** inputs. Во всех случаях
`final_linux_acceptance=false`; даже CAPTURED имеет `legal_status=NOTPROVEN` и
file-level `applicability=NOTPROVEN`. Наличие primary identifier/text не является
юридическим выбором условий OR/WITH. Проверки используют require/CaptureError,
не assert, и работают под Python -O.

## Закрытый schema 1

CLI читает `--record` относительно build root. SHA берётся из independently
verified record, не вычисляется из произвольного обнаруженного файла для admission.
Показанный вызов предназначен только для будущей уже обязательной final build:

```sh
python3 tools/p25_capture_build_closure.py \
  --source-root /verified/final/source \
  --build-root /verified/final/build \
  --sdk-root /verified/sdk \
  --record capture-record.json --record-sha256 VERIFIED_RECORD_SHA256 \
  > /retained/evidence/final-build-closure.json
```

Выход пишется caller в новый собственный retained evidence file вне TTL; перед
cargo clean caller проверяет exit/status, фиксирует output SHA и сохраняет primary
source offers. Код не устанавливает hook и не меняет cleanup/default workflow.
Пути — placeholders, никаких вызовов на actual142 не выполнено.

Record содержит ровно следующие поля (unknown/duplicate JSON keys отвергаются):

- `schema: 1`; `source: {commit, tree, archive_sha256, tracked_inputs: [file_id]}`.
- `profile: {target, commands: [[argv...]], features: [string]}` — observed final
  profile из уже проверенного build record. Commands сохраняются, не исполняются.
- `files: [{id, root, path, kind, sha256?, source_ref?}]`. Пути relative без
  traversal/control. Kinds source/primary/sdk-source относятся к source root;
  header — SDK; tool — SDK или build; generated/elf/depfile/qt-version — build.
  `sdk-source` — retained source fragment/artifact из verified source closure;
  header `source_ref` указывает его ID. Это claimed counterpart, не semantic
  equivalence proof. Отсутствующий hash даёт MISSING без чтения файла.
- `elfs: [file_id]`, `qt_version: file_id | null`, `units: [...]`,
  `generated: [...]`, `primary_refs: [{identifier, file: primary_file_id}]`.
  Последние три metadata поля qt_version/units/generated допускаются отсутствующими
  только для возврата MISSING, не для автоматического восстановления/догадки.
- `units`: `{file: source_or_generated_id, depfile: id | null, argv?: [args]}`.
  Depfile здесь **normalized closed JSON**, не произвольный Make/CXX:
  `{schema: 1, unit: same_unit_id, dependencies: [whitelisted_file_id]}`.
  В report сохраняются argv и нормализованные dependencies; каждый header должен
  встречаться в переданном dependency наборе. Это не доказательство полного
  transitive closure при неполном producer record.
- Qt query record: `{schema: 1, version: "major.minor.patch", sdk_files: [id],
  query_argv: [args], query_tool?: tool_id, exit: 0}`. `sdk_files` должен точно
  совпасть с whitelist SDK-root files; bytes/hash каждой записи проверяются.
  Нет query_tool — MISSING. Запись фиксирует ранее выполненный trusted query,
  tool его не повторяет и не подменяет host version текущим значением.
- `generated`: `{file: generated_id, inputs: [id], generator: tool_id}`. Для
  каждого generated file нужны generation record и compile unit. SPDX извлекается
  также из generated text; если header/output не содержит SPDX, tool не наследует
  его из другого файла или generator license, а возвращает MISSING. Исключение
  на output может потребовать отдельного root file-level attribution решения.

SPDX expression сохраняется целиком; выделенные identifiers сопоставляются с
primary refs как evidence candidates. Tool не является SPDX/legal parser,
не выбирает OR, не применяет WITH/Qt output exception, не объявляет отдельный
identifier обязательным. Версия historical primary inventory не назначается SDK
автоматически. Это устраняет перенос Qt6.12/package78 assumptions на actual142.

## Hook proposal и минимальный следующий шаг

Root после review отдельно решает внедрение **одного capture вызова во время
обязательной final build перед cargo clean**. Producer должен предоставить уже
имеющиеся CXX metadata/depfiles/generated paths и query record из verified build
profile, плюс whitelist/source-primary refs. Если current command profile не
даёт depfiles, producer передаёт null/отсутствующие metadata и получает MISSING;
tool не включает -MMD, wrapper, дополнительные probes или новую audit build.
Изменение compiler flags/wrapper/controller gate требует отдельного root approval.
Неполный capture не заменяет существующие final provenance/source/legal проверки.

Проверки: pure fixtures с bad/missing/malformed/pin/hash/symlink/ancestor/nonregular/
FIFO/sparse oversize/count/total/changed-stat/changed-record controls, CLI read-only
snapshot и -O rejection; полный fixture suite также запущен под -O. Root review
ожидается; live final-build capture не выполнен.
