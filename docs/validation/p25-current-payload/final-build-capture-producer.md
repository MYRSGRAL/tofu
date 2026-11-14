# Producer capture обязательной будущей final build

План реализации (до кода):

1. Проверить schema capture tool и фактические records `_collect`.
2. Реализовать private producer конечного whitelist ELF и source-freeze binding.
3. Вызвать capture после новых Rust builds, сохранить record/report/manifest вне TTL.
4. Чистыми fixtures проверить MISSING, observed commands и запрет cleanup до retention.
5. Проверить diff, тесты, Ruff и сделать атомарный русский commit для root review.

Scope: controller/private helper/tests/docs. Build, download, Qt query, VM,
install, package/release execution не разрешены и не выполняются.

## Реализованный hook и retained evidence

Только fresh Rust branch `_collect` после двух успешных Rust commands и проверки/
копирования трёх ELF вызывает private `retain_build_capture`. Вызов выполняется
до native/package work и до `cargo clean`. `RECIPE`, flags, wrapper, environment,
toolchain и зависимости неизменны. Rust/collected reuse не вызывают producer;
исторические evidence/bindings не переписываются.

Producer использует `p25_capture_build_closure.capture` напрямую; точный повторимый
CLI argv с абсолютными roots и record pin сохраняется в manifest. Build root —
`task/target`; source root — проверенный snapshot; SDK root — новый пустой каталог
`task/capture-empty-sdk`, а не host SDK. Это обозначает отсутствие полученного
SDK whitelist. Читаются только три явно названных ELF через существующий
FD-relative NOFOLLOW Reader с caps. Нет рекурсивного поиска generated/depfiles,
SDK/tool inventories и исполнения команд из metadata.

В controller-owned registry release-каталоге вне task TTL до clean сохраняются:

- `build-capture-record.json`: source commit/tree/archive и exact successful
  observed Rust argv, target/features текущего неизменённого profile, ELF hashes;
- `build-capture-report.json`: результат tool с `MISSING` и точными missing IDs;
- `build-capture-manifest.json`: hashes record/report, observed command records,
  ELF whitelist, конкретные producer_missing IDs и reusable CLI invocation.

Файлы создаются exclusively, fsync файлов и retained directory завершается до
возврата producer. Manifest — маркер завершённого retention. Source archive уже
сохранён freeze в том же registry; ELF copies сохранены `_collect`. Неполный
capture не блокирует существующий collect/legal workflow. Ошибка записи/проверки
capture оставляет target без clean: обычный failure path сохраняет task и сообщает
его путь/размер. Успешно сохранённый MISSING позволяет штатный cleanup. Повторный
CLI предназначен до clean; после clean используют retained record/report/manifest,
а не отсутствующие roots.

## Заблокированная часть / точный вопрос root

Текущий runner получает stdout/stderr Cargo, но не structured CXX invocations,
полный перечень generated paths, их generation inputs/tool identity, normalized
JSON depfiles, build-time Qt query/version/tool record или header-source-primary
join. Cargo stderr/include diagnostics и tool inventories не являются этими
records. У producer нет authority нормализовать несуществующий depfile или
объявить recipe command observed CXX invocation.

Конкретные отсутствующие inputs: `cxx:invocations`, `cxx:normalized-depfiles`,
`generated:paths-and-generation-records`, `qt:observed-query-and-tool`,
`qt:header-source-primary-join`. Capture tool дополнительно возвращает свои IDs
`headeridentity`, `qt_version`, `generatedrecord`, `depfiles`.

Root question перед final build: есть ли уже trusted finite producer этих inputs
в approved final profile, с точным форматом и frozen root/path/hash bindings?
Если нет, нужен отдельный approval изменения instrumentation/flags или query;
такой patch здесь не применяется и команды не исполняются. Текущая реализация
намеренно не имеет пути CAPTURED: это reusable honest MISSING capture, не
утверждение полноты closure. Полный CXX/Qt producer остаётся blocked до ответа.
Не добавлены legal gate, P25 PASS или OR/WITH decision. Нужен root review до
обязательной final build; actual build здесь не запускалась.

Проверки: pure fixtures и fakeRunner, включая retain-before-clean и сохранение
target при retention error; capture schema suite. Ни компиляторы, ни Qt tools
fixtures не исполняют.

Локальный результат проверки implementation: 83 passed, 33 subtests; под
`python -O` producer/capture suite: 26 passed, 17 subtests (ожидаемое предупреждение
pytest об assert вне tests). `git diff --check` чистый. Ruff executable и модуль
в этой среде отсутствуют; установка зависимостей не выполнялась, Ruff format/check
остаются непроверенными. Общий suite не запускался: разрешены pure tests, actual
build/download/package execution запрещены.
