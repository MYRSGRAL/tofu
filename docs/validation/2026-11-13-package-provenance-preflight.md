# A4: preflight package provenance

`tools/check_package_provenance.py` дополняет, не заменяет
`check_arch_package.py` и исторический source→ELF closure #119.
Новые подписи не вводятся: доверенный локальный builder входит в TCB.
Build record фиксирует связь исходников и outputs; один binary hash её не заменяет.
Verifier не доказывает честность произвольного внешнего builder.

Запуск после freeze:

```sh
python tools/check_package_provenance.py --candidate candidate.json \
  --expected reviewed-frozen.json --repo /path/to/frozen/repo \
  --inputs /path/to/retained/artifacts --output /path/to/new/final-manifest.json
```

`expected` должен быть закреплён независимым review до проверки кандидата.
Передача одного самосогласованного неподтверждённого документа обоим аргументам
не даёт provenance. Финальный manifest создаётся исключительно через O_EXCL.
Его PASS означает только preflight, не финальную Linux acceptance.

Schema 1 задаётся fixture в `tests/tools/test_package_provenance.py`:
source commit/tree, arch x86_64, ABI 260, build command/toolchain/target;
artifacts — path/SHA256 для source_archive, gui/helper/worker, xray/sing_box,
обоих patched_package, notices/licenses/source_offers, build_record и package.
Source archive содержит один prefix, полный Git tree без submodules.
Build record обязан точно повторять source, archive checksum, command/toolchain,
target, ABI и семь output hashes. Это evidence сборщика, не сгенерированный
после сборки substitute. Core pins и patched package hashes, состав notices,
лицензий и source offers должны быть независимо сверены reviewer в expected;
данный checker проверяет их байтовую неизменность, не юридическую достаточность.
В package повторно проверяются рецепт, metadata и hashes трёх Rust ELF.
File capabilities пусты, setuid/autostart/runtime downloads/host install запрещены.

Точный текущий остаток находится в `packaging/arch/candidate-readiness.json`.
Ни retained a8, ни ожидаемый результат 158+156 W1 не выдаются за frozen final.
Новые сборки, VM lifecycle, Secret Service campaign и установка не выполнялись.
Тесты используют синтетические данные и mock только package boundary;
реальную проверку пакета выполняет существующий checker.

## R2: ограниченные исправления

Проверка артефактов отказывает symlink во всех компонентах пути, traversal и
необычным файлам до hashing. Source archive проходит предварительную проверку
всех заголовков: один prefix, канонические пути, отсутствие ссылок/специальных
файлов, повторов и лишних каталогов. Git symlinks в этом closure запрещены.

Пакет проверяется по временному snapshot проверенных frozen Git blobs, а не по
mutable checkout. Snapshot не исполняется. Изменение checkout после freeze
не меняет recipe, metadata и compliance, используемые пакетным checker.

**BLOCKED:** trusted producer/anchor и actual core/compliance binding ещё не
реализованы. Нужен bounded contract authorized build controller от root:
reviewed repo/revision, управляемое controller хранилище frozen record и actual
core pins/metadata. Два caller JSON не становятся trusted от равенства.
TCB должен явно включать authorized build controller и процессы того же UID:
защита от злоумышленника с тем же UID этим preflight не обеспечивается.
Текущий PASS не является закрытием этих двух P1 и не разрешает final acceptance.
