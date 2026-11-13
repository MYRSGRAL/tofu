# A4: version-aware compliance и reuse успешных Rust builds

Root выбрал новый NONFINAL ID `linux-controller-rehearsal-14229843-msrv85-3`
с product source `14229843f9a11bf98ec4f5992df00f3247102ea2`.
`final_linux_acceptance=false`; factory/N5, production admission и остальные
readiness gates остаются закрыты. Это не final Linux seal.

## Frozen contract и исправление

Frozen `packaging/licenses/composition/go-sources.json` schema 1 уже содержит
отдельные записи `(module, version, h1, zip_sha256)` для обеих версий brotli.
`release-recipe.json` задаёт Rust commands и core members, не ограничивая число
версий dependency. `package_files` уже использовал SHA-имена source zip.
Recipe/schema, Cargo inputs и frozen archive не изменялись: ошибка была в
Python-индексе только по module и потере version при сохранении dependencies.

Controller теперь индексирует offers по `(module, version)`, проверяет h1 из
фактического GoBuildInfo и сохраняет resolved tuple с archive SHA отдельно для
каждого core. Package/download closure выводится из этого отношения, а не из
множества имён module. Разные версии остаются отдельными offers/payload paths;
точные повторения frozen записи дедуплицируются, конфликт metadata/hash одного
module+version отвергается. Совпадающие архивные bytes адресуются одним SHA.
Ни одна версия не выбирается вместо другой. Notices/лицензии по-прежнему
генерируются и проверяются из exact frozen compliance inventory.

Повторная проверка package binding сверяет сохранённые dependencies с GoBuildInfo,
frozen tuple и SHA каждого необходимого zip; missing offer и wrong hash — отказ.

## Ограниченный trusted reuse

Reuse доступен только для fixed нового ID и fixed prior ID
`linux-controller-rehearsal-14229843-msrv85-2`. Никаких caller record paths,
source/profile/command overrides или candidate attestation не добавлено.
Hash anchor: согласованные partial facts из Git revision
`969f65b68f9283f81d77d9c6f1f2c00d8f26934e`,
`docs/evidence/linux-controller-rehearsal-14229843-msrv85-2/registry-inventory.json`.
Это anchor уже существовавших controller artifacts, не новый synthetic build.

Controller под locks читает fixed registry с descriptor-relative NOFOLLOW,
проверяет own UID, private directories, regular nlink=1 files, права records,
size/SHA всех 46 prior inventory files. Проверяются равенство source/tree/archive/
recipe, prior tooling hashes относительно reviewed Git revision, прежняя failure
boundary, точные успешные Rust commands/cwd/profile/jobs и toolchain version
outputs. Три ELF проходят header check и копируются без исполнения. Несовпадение
не даёт fallback на Rust build.

Новый `rust-lineage.json` содержит `build_performed=false`, prior ID, reviewed
anchor, hashes прежних logs/manifests/ELF, source и toolchain versions. Hashes
установленных qualified rustc/cargo вычисляются при reuse и явно помечены как
`toolchain_binary_sha256_observed_at_reuse`: старый job записывал версии, но не
исторические hashes toolchain binaries; задним числом такие facts не создаются.
Старые FAILED records/exit codes остаются неизменны.

## Проверки

RED для двух разных версий одного module и для отсутствующего reuse API сохранён
в [evidence](../evidence/linux-controller-rehearsal-14229843-msrv85-3/).
25 controller tests и 16 provenance tests проходят; ruff format/check чистые.
Fixture проверяет два cores с разными версиями/zip payloads и positive package
gate; negative cases покрывают wrong second archive, missing version, conflicting
same-version hash, source mismatch, exit=1, changed ELF, symlink, hardlink и mode.
Существующие проверки core/source/compliance/candidate trust boundary сохранены.

## Реальный результат

Штатные операции из назначенного worktree:

```sh
python tools/release_controller.py freeze --release-id linux-controller-rehearsal-14229843-msrv85-3 --commit 14229843f9a11bf98ec4f5992df00f3247102ea2
python tools/release_controller.py ingest-cores --release-id linux-controller-rehearsal-14229843-msrv85-3
python tools/release_controller.py collect --release-id linux-controller-rehearsal-14229843-msrv85-3
python tools/check_package_provenance.py --release-id linux-controller-rehearsal-14229843-msrv85-3 --package /home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-3/package.tar --inputs /home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-3
```

Exit всех четырёх операций — **0**. Collect создал `COLLECTED` context;
candidate gate вернул **PASS**, scope **local-TCB-provenance-preflight**, с
`final_linux_acceptance=false`. Это первый реальный положительный local
packaging/provenance rehearsal; не production/factory admission и не final seal.

Package:
`/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-3/package.tar`
— 443166720 bytes, SHA256
`229a71a54cf2d0e6e6b83396476acb3ea54138ff0b988b93110ce6b7a3f5c740`.
В fixed registry вне TTL сохранены source archive, три reused Rust ELF, оба
accepted Go ELF, manifests/prebuild/source archives, 169 source zip, compiled
notices/license payload, raw command logs и `collected.json`.

Оба core binding проверены: xray — 44 dependencies, sing-box — 136.
Фактический brotli: xray → v1.2.3 / `3017c784cffc7f0019a6573efcb6d861be4bd08e2072532a88ff39280f3ec15c`,
sing-box → v1.1.0 / `61bc9b753662e2101c63bfc9562f660e9ceca9866010948f1aaccb05cf9e263a`.
Обе версии сохранены в package как разные SHA-named zip. Core GoBuildInfo и
resolved tuples находятся в `collected.json`; core→source SHA отношения не потеряны.

Новый запуск содержит **0 Rust builds**, **0 cargo clean**. Все три reused ELF
побайтово совпадают с зафиксированными prior SHA. Target не создавался.
Штатный controller удалил собственный transient task после завершения под flock;
cleanup подтверждён отдельно. Hash inventory обоих старых registry ID проверен:
records/artifacts не изменились, прежние failures остаются failures.

`result.json` содержит компактный результат, `collected.json` — actual commands,
effective environment, toolchains, package payload и source/core bindings,
`registry-inventory.json` — пути/размеры/SHA всех сохранённых файлов, включая
raw logs. Бинарные ELF/package/source archives остаются вне Git в fixed registry;
их подмена относительно evidence проверяется по inventory SHA.

Host install/upgrade, Docker, sudo, runtime16, probes/bootstrap, cross-UID,
release/deploy не выполнялись. PR175/171/147 и FR07 evidence сохранены.
Будущие N5 product changes требуют отдельного final source/build/review.
