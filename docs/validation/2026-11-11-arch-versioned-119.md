# Versioned package lifecycle и reboot #119

Checkpoint `40531b781ced995a9483036dcb9b2560ffef8b56` принят и не изменяется.
Rust, Go и native исходники не собирались. Проверка использует реальные архивы:

| Версия | Source revision | Package SHA-256 |
| --- | --- | --- |
| 0.1.0.r20261110-2 | eec7a56b6b03f2b022a87682f9097ce78b61fa0b | e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0 |
| 0.1.0.r20261111-1 | 6cf65a4cf9760a7df68b8eee10f012770b38604d | a10f0840737b2ababc572fbd432335abfd5c232c59ec0493aa0483d749abafcc |

Old artifact сохранён в worktree
`/home/myrs/.t3/worktrees/yonik/t3-0e71b13a-c31bb75af8e3d15e/local/validation/arch-clean/`;
checksum совпадает с принятым source-build evidence #78 и `.BUILDINFO`.
Новый artifact — сохранённый результат actual clean source build #119.
Текущий прогон не является новым source→ELF proof; архивы не перепаковывались,
ELF не заимствовались для создания нового пакета. `versioned-inputs.json`
фиксирует archive/member ELF hashes, `.PKGINFO`, `.BUILDINFO`, source revisions
до VM effects. SHA immutable inputs проверяются fail-closed.

Evidence: `/home/myrs/projects/yonik/local/validation/arch-versioned-119/`.
`run1` сохранён целиком как FAIL: новый GUI ждал smoke deadline 45 секунд,
который совпал с harness timeout. В fixture добавлен явный shutdown. До ошибки
реально выполнены old install/start и upgrade new; этот run не называется PASS.
`versioned-artifacts.tar` сохраняет исходные данные и traces до continuation.

Actual reboot в run1: kernel `7.2.11-arch1-1` → `7.2.15-arch1-1`, boot ID
`c28a04bf-97ea-46ee-9f61-ac7a35c9fc72` →
`589f988a-896f-4533-8b45-08ed08b8b810`. SSH и modules доступны после reboot.
Обновлялся только guest. Signed base cache использован read-only под BSD lock;
его SHA `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`
не изменился. Host settings, сеть, пакеты и keyring не менялись.

Continuation `resume.py` использует тот же overlay и готовые архивы, без
обновления пакетов и Rust slot. Сначала сохранены FAIL artifacts; затем только
собственные fixture-каталоги прошлого прогона очищены до нового полного маршрута.
Внутри маршрута old→new→old каталог данных сохраняется неизменным.

Ограничения: smoke использует ephemeral key, контрольный keyfile — fixture,
постоянный Secret Service и перенос зашифрованных пользовательских записей не
проверяются. Runtime engine binaries не включены в пакет. #107/#113 исключены
из source freeze; native Go/provenance gaps и полный P25 остаются вне этой
приёмки. Независимый review seal и final runner candidate выполняет root.

`run2` тоже сохранён как FAIL. Явный shutdown устранил первый harness timeout,
но actual new startup на БД первого процесса дал `error=storage`, `ready=false`.
Причина ограничения стенда: smoke генерирует новый ephemeral key для каждого
процесса, а repository проверяет зашифрованный sentinel. Поэтому key continuity
и разбор future preferences на этом пути недостижимы. Это не доказательство
дефекта package upgrade или несовместимости схем. Данные run2 сохранены до
следующего continuation; fresh-config обход между new и rollback не применяется.

`run3` проверяет package transaction и сохранность данных, явно ожидая
наблюдаемый storage fail-closed у следующих процессов на той же БД. Контрольная
строка SQLite, hashes/UID/mode БД, keyfile и future preferences проверяются на
каждой стадии; установленный ELF должен совпадать с соответствующим реальным
архивом. Отдельная проверка отказа записи именно из-за future preferences не
пройдена и не заявляется. Для неё потребуется отдельно согласованный стенд с
постоянным тестовым ключом; текущий запуск keyring не использует и не изменяет.

Итог run3: PASS именно versioned package transaction
`0.1.0.r20261110-2 → 0.1.0.r20261111-1 → 0.1.0.r20261110-2 → uninstall`.
Qkk: old 32 files / new 338 files, 0 altered. Installed ELF каждого этапа
совпали с archive member hashes, rollback вернул первоначальные ELF. Helper
inactive/3 проверен до и после desktop startup, намеренной активации не было.
Все шесть snapshots сохранили одинаковые hashes/UID/mode SQLite DB, keyfile,
future preferences и одинаковые nft table/interface attributes. Контрольная
строка настоящей SQLite БД сохранилась. После uninstall ELF отсутствуют.
Оба повторных GUI закрылись с наблюдаемым `storage`, `ready=false`; это
fail-closed сохранность данных, не успешное повторное открытие БД.

`summary.json` в корне evidence повторно проверяет snapshots и rollback ELF;
`run3/versioned-artifacts.tar` содержит actual guest fixture, данные и traces.
Guest fixture SHA-256:
`fabf96f56352edc5c0ead67298f9787e51a379f8f4895a7ce97a133568e48134`;
bytes совпали с финальным `tools/arch_versioned_package.py`.
VM exit0, свой overlay/cloud-init/SSH key удалены; никаких Cargo targets в
этом продолжении не создавалось. Base cache сохранён. Десять Arch tests и Ruff
проходят. Оба FAIL run оставлены как evidence, без подмены статусов успехом.
