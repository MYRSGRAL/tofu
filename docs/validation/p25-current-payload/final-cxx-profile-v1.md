# Opt-in final profile p25-final-cxx-v1

Этот профиль расширяет foundation `ba26ea6`: после новой Rust build появляются
положительные CXX metadata, а не только список отсутствующих inputs. Никакая
actual build/compiler/Qt query/download/VM/package/release операция при реализации
не выполнялась. Все executions в новых fixtures заменены fake runner/process.
Root review требуется до будущей final build.

## Включение и границы доверия

Единственный opt-in — enum при новом freeze (пример будущего вызова, не запуск):

```sh
python3 tools/release_controller.py freeze \
  --release-id final-reviewed-id --commit FULL_REVIEWED_COMMIT_SHA \
  --final-profile p25-final-cxx-v1
python3 tools/release_controller.py collect --release-id final-reviewed-id
```

`collect` читает профиль из controller-owned frozen record. Нет CLI для выбора
compiler, shell, произвольного wrapper или команды query. Исходный `RECIPE` и
старый `build_env` не переписаны. Без opt-in работает foundation; historical
Rust/collected reuse не получают final instrumentation. Для reuse ID opt-in
отвергается. Existing evidence, source offers и package bindings неизменны.

Freeze закрепляет отдельный `final_profile`: spec, SHA tooling и SHA самого
профиля. Tooling должен уже присутствовать побайтно в выбранном Git commit;
wrapper должен иметь Git mode 100755. Перед сборкой controller сверяет hashes
профиля и snapshot. Обычный frozen controller tooling hash также меняется.
Профиль не присваивается старому source tree задним числом.

Build-phase `prepare` задаёт CXX и фиксированные target-specific CXX aliases на
frozen executable `tools/p25_cxx_capture_wrapper.py`; config pin/roots передаются
через controller-owned environment. Другие flags/toolchain/dependencies не
изменяются. Inherited CXXFLAGS, если они есть, становятся actual observed argv;
конфликтующие dependency flags отвергаются. Probes определения compiler family
проходят через тот же fixed compiler и записываются отдельно от compile units.

## Fixed execution

Compiler — `/usr/bin/g++`. Query — ровно `/usr/bin/qtpaths6 --qt-version`, один
вызов перед Rust commands. Наличие этих executable на build host здесь не
проверялось. Отсутствие/неуспех означает failure с сохранённым target, а не
fallback на другой compiler/query или подставленную версию.

Только у этих двух фиксированных aliases разрешено однократное разрешение
symlink в canonical tool path внутри `/usr`. Canonical ancestors и file затем
открываются NOFOLLOW; executable хешируется потоком и запускается через уже
открытый `/proc/self/fd/N` с `pass_fds=(N,)`. Произвольные input symlinks не
разрешаются. FD закрывается после invocation; hash/size/stat проверяются.

Без shell; stdin DEVNULL; отдельная process group; deadline 300 секунд; stdout
и stderr не более 1 MiB каждый. Pipe drain не блокируется на заполненном втором
pipe. На timeout/cap/ошибке process group завершается, child reap выполняется.
Это ограничение относится к instrumentation compiler/query; общий controller
runner сохраняет свой существующий контракт.

К actual `-c` argv добавляются только `-MD -MF <private numbered depfile>`.
Нет codegen flags, `-MMD`, `-MP` или audit rebuild. Response files и существующие
`-M*` dependency flags отвергаются, чтобы capture не объявил неполную dependency
rule полной. Argv ограничен 4096 аргументами, 4096 bytes/arg и 1 MiB суммарно.
Wrapper сохраняет original/actual argv, cwd, unit, compiler canonical path/SHA,
exit, output hashes и config pin. Probe не становится compile unit. Failed
compiler не получает normalized depfile или successful unit record.

## Конечный whitelist и retained metadata

Invocation IDs выдаются private counter под bounded flock; max 4096. Producer
читает только numbered records `0..count-1`, а не glob/target scan. Отсутствующий
record или неверные argv/unit/config bindings отвергаются. Суммарные observed
records ограничены 8 MiB, dependency rows — 10000, file whitelist — 4096.

Make depfile читается как данные: одна rule, escaped spaces/#/:/backslash, `$$`,
line continuations. Make expansion, comments, дополнительные rules и malformed
escapes отвергаются. Это не shell и не Make execution. Нормализованный JSON
производится из фактического depfile; raw depfile SHA и exact dependencies
сохраняются в observed evidence. Повторная компиляция того же file допустима с
отдельным depfile: обе invocations сохраняются, exact duplicate file/depfile
pair отвергается.

Только source/build paths и dependencies под двумя fixed external roots:
`/usr`, `/home/myrs/.cargo/registry/src`. Никакого SDK/registry inventory scan.
Inputs вне этих roots — явный отказ с сохранением target. При parent traversal
NOFOLLOW проверяет также ancestors, пропускаемые lexical normalization.
Symlink/type/change/cap failure не превращается в положительный hash.

Собственные inputs связываются с frozen source snapshot. Фактически названные
external headers/tools/units потоково копируются в private `task/capture-sdk`;
origin path → staged path map сохраняется. Tool inventory не становится compiled
inputs: tools имеют kind `tool`; только actual depfile dependencies составляют
compile dependency list. SHA tool при capture должен совпасть с invocation SHA.
External readers и staging ограничены; compiler/tool ≤512 MiB, source/header/
sdk-unit ≤8 MiB, raw metadata ≤1 MiB, aggregate external inputs ≤1 GiB. Повторные
проверки/копирование добавляют ограниченный I/O, не ослабляют caps самого capture
reader. Staging копирует по 64 KiB, не держит compiler bytes в RAM.

Capture schema 1 расширена kind `sdk-unit` для реально скомпилированного внешнего
source unit. Это не claim соответствующего source offer: его join остаётся
MISSING. Generated build-root paths получают kind `generated`. Без generation
record их bytes/hash и SPDX/excerpts сохраняются в `unbound_generated_sources`,
с `generation_record=null` и `generated_record:<id>`. Компилятор не выдаётся за
CXX-Qt/moc/rcc generator; generation argv/inputs не придумываются.

Retained вне task TTL до clean:

- `build-cxx-observed.json` (≤8 MiB): actual invocation records, config/pin,
  raw depfile hashes, normalized dependency lists, input origins и observed query;
- `build-capture-record.json` (≤1 MiB): source/profile/ELF bindings и конечный
  whitelist, normalized depfiles/query refs;
- `build-capture-report.json` (≤32 MiB): capture verification, generated bytes,
  dependency hashes, header attribution candidates и конкретные missing IDs;
- `build-capture-manifest.json` (≤1 MiB): record/report/observed evidence hashes,
  final profile и reusable exact capture CLI argv.

Запись exclusive/NOFOLLOW, fsync файлов и parent directory; manifest — marker
завершённого retention. При любой ошибке до этого marker target не очищается.
Tool report `MISSING` не является legal/admission gate и позволяет штатный clean.
После clean exact CLI нельзя повторить на удалённых roots; retained report,
records и manifest служат evidence. Для source offer сохранён frozen source.tar.

## Что положительно доказано, а что остаётся открытым

Pure fixtures подтверждают CXX argv → compiler SHA → normalized depfile → source/
header/generated hashes → retained generated bytes, а также actual observed Qt
query tool/version. Нормализация не выдаётся за observed compiler command.
Query version — observation конкретного tool, не доказанная версия каждого
header/tool в whitelist; Qt query-to-header/source applicability требует review.

Header/sdk-unit corresponding-source и primary expression/text join здесь не
выдумываются. Generated provenance неизвестна, если не наблюдался generator.
Producer возвращает `source-offer-primary-join:<id>` и
`generation-provenance:<id>`; capture tool добавляет точные file-level IDs.
`legal_status=NOTPROVEN`, `final_linux_acceptance=false`; P25 PASS и OR/WITH
решения не появляются. Общий статус может оставаться MISSING при полезных
positive compile/query metadata. Нет blanket promotion historical captures.

## Проверки реализации

113 pure tests passed, 33 subtests; отдельно под `python -O`: 50 tests passed,
17 subtests. Единственное предупреждение — ожидаемое pytest о неисполняемых
assert вне test modules при `-O`. Ruff 0.17.4 check/format и `git diff --check`
чистые. Проверки охватывают positive generated/sdk-unit capture, повторные
invocations одного unit, Make escaping/parent normalization, config/tool pins,
NOFOLLOW, root/counter bounds, query/compiler failures, deadline/log caps,
закрытие FD/process cleanup и retain-before-clean. Не запускался общий suite,
который может включать запрещённые сейчас реальные build/package операции.
