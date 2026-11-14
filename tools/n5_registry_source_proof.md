# Retained Cargo registry source proof

`n5_registry_source_proof.produce(retained, pins, commit)` вызывается отдельно
после capture. `pins` содержит независимо полученные SHA-256 frozen.json,
source.tar, build-capture-record.json, build-capture-report.json,
build-cxx-observed.json. Возвращает `(sidecar, sidecar_sha256)`; запись SHA сама
по себе не создаёт доверия. Передача pin consumer должна идти по доверенному
каналу controller evidence.

Producer читает Cargo.lock только из pinned raw git source.tar. Этот узкий reader
не распаковывает архив: проверяет заголовки/размеры/EOA, принимает ровно один
regular `src/Cargo.lock`, проверяет нормализованные имена и отсутствие повторов.
PAX обрабатывается общим bounded parser до чтения members. Для каждого observed registry cwd выбирает exact
crates.io index/name/version из lock. Читает только соответствующий cached
`.crate` по фиксированному PROFILE registry root, не extracted registry sources,
не сканирует cache. Проверяет archive checksum и member bytes против staged
SDK-unit и header record/report/origin/cwd. Сохраняет одну копию на SHA в
`registry-archives/<sha>.crate` и `registry-proof.json`, без перезаписи, с fsync.
Частичный output после ошибки не считается готовым proof.

Schema `P25_CARGO_REGISTRY_PROOF_V1`:

- source_commit, source_archive_sha256, cargo_lock_sha256;
- input_sha256: frozen/record/report/observed independent pins;
- packages: exact cwd/name/version/checksum, archive_path/size/sha256;
- units и headers: file_id, member_path, size, sha256; units содержит только
  observed compiled SDK units, headers — остальные captured registry source inputs.

Consumer `validate(reader, proof_path, proof_pin, pins, commit)` проверяет schema,
unknown/duplicate fields, exact package selection, source/input bindings, archive
identity и SHA всех captured registry members. Возвращает разрешённые exact cwd и проверенные bytes
для retained capture reader. Proof обязателен только при registry cwd. Пустая
подстановка или blanket registry-prefix allow запрещены.

Ни producer, ни consumer не запускают компилятор. Consumer не обращается к host
registry/cache. NOFOLLOW и проверки identity/mutation применяются ко всем читаемым
retained/cache файлам; архивы проходят общий bounded parser профиля. Evidence не
доказывает вызов компилятора независимо от доверенного controller и не повышает
legal/source/admission status.

CLI producer принимает `--retained`, `--source-commit` и
`--<имя-файла-с-точками-заменёнными-на-дефисы>-sha256` для пяти перечисленных
входов. Пишет только фиксированные новые имена внутри retained; повторный запуск
в тот же output отклоняется. В рамках разработки использованы только synthetic
fixtures, без реального registry/cache, сборок и запросов Qt.

Пределы: до 128 selected packages, до 64 MiB compressed/decoded на `.crate`,
до 256 MiB unique retained archives суммарно; source.tar до прежних 512 MiB.
Probe-only package имеет context proof и пустой units. Только units с exact
observed compile/origin/record/report/member joins объявляются compiled source;
headers проходят такой же byte join без утверждения об их компиляции. Probe не
объявляется компиляцией source unit.
