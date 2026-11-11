# Arch source freeze 6cf65a4

Рецепт использует GitHub codeload archive commit
`6cf65a4cf9760a7df68b8eee10f012770b38604d`, tree
`89e088736972e4b498e29fd4cfc501524f699bfc`.
SHA-256 скачанного архива:
`5a3d3d3b4a418d5208436934c3a7f1032d23fc83444d1948060d3c802087f665`.
Все обычные файлы архива сверены SHA-256 с blobs точного git tree;
полная карта сохранена в `local/validation/arch-6cf65a4/source-proof.json`
основного workspace `/home/myrs/projects/yonik`.

Python теперь явно объявлен в makedepends. Генератор notices присутствует
в закреплённом source; текущий `payload.list` ограничивает установку notices.
Linker arguments CXX-Qt сохранены. Приложение, helper, worker, native pins и Go
исходники не изменены. Пакет содержит три заново собранных Rust ELF;
внешние xray/sing-box являются отдельной runtime зависимостью, не входят в пакет.

VM base: подписанный `Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2`,
SHA-256 `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`,
primary fingerprint `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`.
Cache только читается под BSD lock. Гость: 2 CPU, 4 GiB, Rust 1.85.1,
jobs=1, собственный target, generic makepkg flags. Host install и сетевые
изменения не выполнялись.

Evidence: `local/validation/arch-6cf65a4/`. `build.log` относится к единственной
clean source build; `finalize.py` продолжает ту же VM после сборки, сохраняя
архив и binary provenance перед установкой. Исходный runner остановлен перед
install; его статус не выдаётся за полный успешный runner run.

Проверки Python: 7 Arch, 5 compile notices, 13 inventory; Ruff и bash syntax.

Ограничения: #107 maintenance и #113 Policy исключены из source freeze.
Исторический #78 source proof принят без повторения; эксперименты #85 с hashes
трёх Qt archives не повторялись. Полный P25, юридическая сертификация,
source→ELF внешних cores и финальная Linux acceptance не заявляются.
Версионный update/rollback требует предыдущего реального package artifact;
переустановка текущего архива не является таким доказательством.

Чистая сборка завершилась: desktop release 10m12s; helper/worker также собраны
из того же source. Реальный пакет `yonik-0.1.0.r20261111-1-x86_64.pkg.tar.zst`,
SHA-256 `a10f0840737b2ababc572fbd432335abfd5c232c59ec0493aa0483d749abafcc`.
PKGBUILD seal и `pkgbuild_sha256sum` в `.BUILDINFO`:
`07fb40b269f664ce2b1a73de26cf630ba1f7bcc4e78a1bffa171b4ae25516414`.
Сохранены SHA-256 трёх target ELF до установки; использованы свежие Cargo outputs.
Validator прошёл в госте и на сохранённом архиве на host. Установка: 338 files,
0 altered; sysusers создал yonik-worker, helper inactive/3; units/dbus/polkit
проверены. Qt offscreen под builder с отдельным HOME: 142 steps, 41 events.

Первый lifecycle RED: `nft add table` вернул Operation not supported после
обновления guest kernel без перезагрузки. Исходный runner исправлен: после
`-Syu` требуется новая boot ID и доступные modules текущего kernel. Это изменение
не проверено повторной source build. Для lifecycle сохранённого пакета создана
вторая disposable VM без Rust/build: kernel base image не обновлялся
(`pacman --ignore linux`), чтобы сохранить согласованные kernel/modules.

Первая VM: cargo clean удалил 4048 files / 997.4 MiB, VM остановлена,
собственный transient overlay удалён. Base cache сохранён.

Вторая VM: PASS installation/inactive=3, integration, private Qt smoke,
reinstall/Qkk и uninstall. Контрольные `state.db` и `keyfile` в
`/home/builder/.local/share/yonik` сохранили hashes; чужие nft table
`inet yonik_foreign_fixture` и dummy interface `yonik-foreign` сохранились.
Fixture удалил только свои сетевые объекты после проверки. VM exit0, свой
transient overlay удалён. Логи и screenshots находятся в
`local/validation/arch-6cf65a4/lifecycle-vm/`, включая `artifacts.tar`.
Версионный update/rollback остаётся непроверенным, поскольку предыдущего
реального package artifact в доступном workspace нет.
