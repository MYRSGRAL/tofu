# Rust capture → N5 receipts

`n5_capture_receipts.py` — offline проекция **внешне доверенного controller build evidence**,
не независимая compile attestation. Ничего не компилирует, не запускает ELF,
не регистрирует профиль, не меняет admission/legal status.

`produce(retained: Path, pins: dict[str, str], source_commit: str)` возвращает
`(receipts, report)`. `emit(output, receipts, report)` создаёт новый каталог внутри
текущего workspace, без перезаписи. Родитель должен существовать и принадлежать
текущему uid. Все компоненты открываются NOFOLLOW; файлы эксклюзивные, mode 0600,
каталог 0700, данные и каталоги fsync. Частичный каталог после ошибки не является
готовым результатом; повторное использование имени отклоняется.

Обязательные независимые pins — lowercase SHA256 **точных байтов**:

- `rust-captured-only.json` — доверенный actual controller marker;
- `build-capture-manifest.json`, `build-capture-record.json`, `build-capture-report.json`;
- `frozen.json`, `source.tar`;
- для `final_profile` обязательно также `build-cxx-observed.json` (отдельный pin).

Не вычисляйте эти pins из полученного недоверенного bundle для создания доверия.
Получите их по отдельному проверенному каналу controller evidence; source commit
тоже передаётся независимо. Самостоятельное совпадение хешей не доказывает сборку.
Политика свежести здесь — точное соответствие независимо выбранным pins/commit,
не возраст файла и не автоматический выбор «последнего» build.

Retained каталог должен содержать эти шесть файлов (семь для final-profile) и `yonik-desktop`, `yonik-helper`,
`yonik-worker`. Поддерживаются исходный Rust-only profile и целевой `freeze --final-profile
p25-final-cxx-v1` → `collect --rust-capture-only`: frozen schema 2,
marker `RUST_CAPTURED_ONLY`, отсутствие lineage/reuse, точный текущий `RECIPE`,
успешные observed commands, source/archive binding, точный whitelist и реальные ELF
хеши. Идентичность путей и файлов проверяется до/после чтения и повторно после join.
Хеширование потоковое с cap; metadata до 1 MiB, report до 32 MiB, ELF до 128 MiB,
archive до 512 MiB, общий бюджет чтения до 2 GiB. Архив не распаковывается:
его соответствие commit утверждается доверенным frozen/controller evidence.

Closure проверяется существующим `_capture` API через узкий retained reader.
Его исходный `MISSING` по CXX/Qt/depfiles сохраняется, а не превращается в CAPTURED.
Отсутствующий или конфликтующий closure evidence отклоняется. Иные producers и non-Rust capture отклоняются.
Старые absolute roots в capture report используются только для проверки согласованности
record/invocation/cwd, никогда не читаются.

Для enriched capture проверяются точный `_p25_final_profile.PROFILE`, digest и
закрытый набор tooling pins в frozen profile, config hash, observed argv через
`compile_arguments`, compiler/query identity, unit/depfile bindings, normalized
dependencies, source tracked whitelist и staged input origins. Список units, Qt
metadata, producer_missing, positive counters и captured report должны совпадать.
Unknown/duplicate fields отвергаются и внутри observed/config/Qt/generated metadata.
Generated text из report и normalized dep/Qt records восстанавливаются в точные
producer JSON bytes и сверяются с record SHA256, затем проходят `_capture`.

Actual producer не сохраняет raw depfiles и полные SDK/source/tool bytes в retained
каталоге. Утилита не имитирует их содержимое и не читает старые build roots. Их
identity/observations остаются утверждением независимо pinned controller evidence;
raw depfile hash сверяется как поле observed evidence, но сами raw bytes недоступны.
Повторный capture честно возвращает `file:<id>` для отсутствующих bytes и
`retained:raw-depfiles-unavailable`. Выходной report содержит `unavailable_input_bytes`,
`capture_byte_replay` и `capture_profile`. Исходные legal missing conditions
проверяются по сохранённым conditions и whitelist; full legal PASS для receipt не
требуется. Generated provenance и Qt header/source legal join остаются MISSING.
Это позволяет выдать receipt actual worker из enriched capture без повышения
статуса source/legal evidence. Duplicate/unknown JSON fields и нарушение
whitelist отклоняются; входные pin всегда обязательны.

Фиксированные выходы:

- `desktop.receipt.json`, `helper.receipt.json`, `worker.receipt.json`: строго
  `schema: "SOURCE_BOUND_ELF_V1"`, `source_commit`, `source_archive_sha256`, `elf_sha256`.
  Это формат существующего N5 fixture; имя можно сопоставить с именем ELF receipt
  при отдельном формировании fixture, сам seam fixture не создаёт.
- `worker-build-receipt.json`: строго `schema: 1`, `worker_sha256`,
  `source_archive_sha256`, совместимо с `tools/n5_build_manifest.py --receipt`.
- `report.json`: независимые входные pins/source binding, SHA256 производных receipts,
  `UNREGISTERED`, `NOT_ADMISSION`, closure и `driver: MISSING`.

Driver receipt не создаётся. Driver требует отдельной actual build со связью с тем же
frozen source archive; receipts трёх Rust ELF не заменяют это доказательство.

CLI: `python tools/n5_capture_receipts.py --help`. Обязательны `--retained`,
`--source-commit`, `--output` и по одному `--<имя-файла-с-точками-заменёнными-на-дефисы>-sha256`
для каждого из шести базовых pin (например `--rust-captured-only-json-sha256`,
`--source-tar-sha256`). Для enriched capture также обязателен
`--build-cxx-observed-json-sha256`; в исходном Rust-only profile он не допускается. Ошибка возвращает код 2; успешная проекция — 0.

Проверка: `python -m unittest discover -s tools/tests -p test_n5_capture_receipts.py`
и та же команда с `python -O`. Fixtures синтетические; real Rust build и доступ
к реальному controller registry не требуются и не выполняются.
