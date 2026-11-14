# Actual142 Qt attribution: граница сохранённых metadata

Дополнение после frozen statement fix `40c754e12d1bf5e3d15d7d5f79c38675f43e11dc`.
Ни old ledger/statement bindings, ни actual package142 bytes не изменены.
Машиночитаемые выдержки: [actual142-build-metadata.json](actual142-build-metadata.json).
Это исследование конкретного артефакта; reconciler с assert (в том числе его
ограничение под Python -O) не production universal validator и не final sole gate.

## Что установлено конкретными source-bound records

Frozen ref `2599fe70ca93ebd14baaa8932e9de69fc5d66a94` содержит
`docs/evidence/linux-controller-rehearsal-14229843/registry-inventory.json` (13),
`linux-controller-rehearsal-14229843-msrv85-2/registry-inventory.json` (46) и
`linux-controller-rehearsal-14229843-msrv85-3/registry-inventory.json` (770).
Числа обозначают retained FILE records registry root: paths/size/SHA, не SDK
header inputs. Hashes самих inventories сохранены в новом ledger. В этих списках
не найдено отдельных depfiles/generated source/header identity records.

Три actual package142 Rust ELF побайтно совпадают по SHA с frozen 46-file
msrv85-2 registry inventory. Это конкретная привязка producer profile к payload,
а не перенос composition maps package78/eec7. Сохранённые command records:

- `controller-logs/2.command.json`: успешный `cargo rustc --locked --release
  -p yonik-desktop --features gui --bin yonik-desktop` с четырьмя initializer
  `--undefined` flags; jobs=1, Rust 1.85.1, build ID с source142.
- `3.command.json`: успешный release/locked build helper/worker.
- `9.command.json`: успешный cargo clean того target; retained target отсутствует.
- `2.stderr`: всего 10 469 bytes; сохранены только 15 строк include chain.
  Они называют QtCore `qstring.h`, `qhashfunctions.h`, `qlist.h`, `qstringlist.h`,
  `QStringList`, `qchar.h`, `qcomparehelpers.h`, `qcompare.h`, `qnamespace.h`,
  `qbytearray.h`, `QByteArray`, CXX-Qt `core/qstring.h`, `qstring.h` и
  `out/cxxqtgen/src/bridge.cxx.cpp`. Это positive observed paths, не exhaustive
  depfile и не доказательство retained code каждого header.

Actual `.PKGINFO` задаёт qt6-base/qt6-declarative/qt6-svg **без версий**.
Сохранённые successful Rust command records и bounded stderr не задают Qt SDK
version, generator identity или header hashes. Наличие target triple в recipe
не превращает host SDK в закреплённый build input.

## Join и точные MISSING

| Звено | Сохранено | Чего не хватает для применимого join |
| --- | --- | --- |
| Package ELF → actual source/profile | Три ELF hashes, source142/tree/archive, exact commands, Rust version, features/flags | Здесь связь подтверждена; это не build reproducibility |
| Qt header path → identity/version | Positive include-chain paths | **MISSING:** build-time content SHA каждого фактического header, qconfig/Qt version, SDK package/source identity |
| CXX-Qt/Qt tool → generated unit | Имя bridge.cxx.cpp в compile diagnostic; build.rs в release-source | **MISSING:** retained generated source bytes/hash, generator executable/version/hash, invocation и header depfile для этого unit; аналогично moc/rcc/QML output при фактическом использовании |
| Actual header/generated file → primary expression/text | Historical `qt-sources.json` версии **6.12.0**, 195 text refs внутри actual source archive | **MISSING:** file-level copyright/SPDX/exception и точный matching Qt source identity для actual142. Историческая 6.12.0 не установлена как версия actual142 SDK |
| Source offer → соответствующий SDK/generated source | Actual outer entries: core upstream/patched archives, Go ZIP, release-source.tar; внутри последнего собственный build.rs/QML/bridge и historical primary texts | **MISSING:** доказанная corresponding-source closure применимых embedded SDK/generated fragments. Сохранённые primary texts не равны полным SDK sources |

Текущее чтение host qconfig показало 6.11.2, в отличие от historical 6.12.0.
Это **не build-time observation** и не доказательство версии actual142: нет
сохранённого profile hash, с которым можно связать текущий header. Текущие host
SPDX также не повышены до применимых условий actual142. Новая версия/pin не
выдумана; отдельный host audit не проводится.

Следовательно, 195 historical primary refs доказанно доступны как bytes в source
offer, но **их применимость к actual142 Qt не доказана**. Ни 169 отсутствующих
standalone index hashes, ни весь Qt SDK inventory не объявляются обязательными.
Конкретного missing mandatory license text или необходимого OR/WITH решения
из этих данных установить нельзя. Проверенный statement defect исправлен ранее;
оставшееся — attribution/source-closure metadata gap.

## ONE minimum final-freeze action

Во время **уже обязательной final build**, до штатного cargo clean, сохранить
ограниченный build-input/generated-source capture и выполнить его file→primary
condition/text/source join с actual final ELF. Отдельная audit rebuild не нужна
и не запрошена. Предлагаемый capture contract (пока только proposal):

1. Связать release/source/tree/archive, recipe, features/target/flags и ELF hashes
   с фактическими C/C++ invocations и Qt SDK/tool identity: resolved paths,
   versions и SHA qconfig, используемых pkg-config/CMake/qmake metadata и
   фактически вызванных moc/rcc/QML generators. Собирать только используемые
   инструменты; не запускать дополнительные Qt probes ради аудита.
2. Из уже выполняемой компиляции сохранить depfiles и generated translation
   units/headers (CXX-Qt bridge, moc/rcc/QML, если созданы), их SHA и точные
   generation inputs/commands. Не читать весь DWARF и не заменять depfiles
   историческим map. Для реально перечисленных SDK headers сохранить SHA,
   file-level copyright/SPDX/exception excerpts и пакетную/source identity.
3. Закрепить первичные license/notice bytes и точные соответствующие source
   artifacts/patches/build instructions для установленного применимого набора.
   Сопоставить source offers и final package notice entries по SHA. OR/WITH
   не выбирать автоматически: если без root legal decision join не закрывается,
   вернуть конкретное file/expression/условие до изменения лицензирования.
4. Capture пишет в retained evidence вне TTL до cleanup; предлагаемые пределы:
   10 000 dependency rows, 64 MiB generated-source bytes, 8 MiB metadata/excerpts.
   Превышение фиксируется как INCOMPLETE с точным missing list и требует bounded
   решения root, а не молчаливого пропуска. Не сохранять весь target/logs/cache,
   DB, secrets или unrelated OS payload. Исходные refs/hashes неизменны.

Это добавление evidence capture к final build, **не новый реализованный gate**,
не ослабление текущего provenance и не разрешение на build сейчас. Если root
потребует автоматический production gate, его контракт/реализация требуют
отдельного решения; исследовательские assert не используются как замена.

Проверки delta: SHA join трёх actual ELF к frozen registry; counts 13/46/770;
все использованные metadata blobs закреплены hash; старый reconciler replay
byte-identical; 18 существующих pure tests и Ruff format/check. Нового product
кода, network/download/build/VM/keyring операций нет.
