# P25: фактическая поставка и текущие доказательства

Snapshot PR85: historical package78/source eec7. Это не latest main и не финальный
release candidate. Root продолжает Linux features; остатки provenance этого
payload не блокируют выдачу других задач.

## Требования и границы

Основание: [P25](../../../docs/PLAN.md), [FR-22](../../../docs/specs/requirements.md),
[Linux/platforms](../../../docs/specs/platforms.md),
[этап 5 и приёмка](../../../docs/specs/validation-and-roadmap.md),
[ADR-010](../../../docs/specs/decisions.md). Их hashes и фактические пути payload
сохранены в [redistribution-scope.json](redistribution-scope.json).

Спецификация требует выбранную открытую лицензию, доступные собственные исходники,
инструкции/тесты, необходимые notices и сохранение условий зависимостей, evidence
обязательных FR и перечисленные ограничения. P25 также включает пользовательские
инструкции, зелёные проверки и release-приёмку. Эти требования не заменяются
artifact scan и не понижаются здесь. Выбор SPDX/ветви OR/WITH не выполнялся;
различия ADR и package declaration остаются решением для финальной поставки.

Криптографическая аттестация каждого внешнего OS file и побитовая
воспроизводимость каждого такого файла не вводятся как новые release criteria.
Подписи/аттестации являются источниками provenance, а отсутствие SLSA само по себе
не объявляет все Linux features или внешнюю библиотеку недопустимыми.
Оценка выполнения лицензионных условий остаётся отдельной задачей.

| Слой | Факт для package78 / кандидатов | Что входит в notices/source assessment |
| --- | --- | --- |
| Payload YonIk | 17 regular archive entries, включая metadata; три ELF и units/sysusers/DBus/polkit/desktop/icon | Собственный код и включённые в ELF компоненты; фактически поставляемые notices/исходники/инструкции |
| Rust/static/native | Runtime sections наблюдались только у отдельно хешированных новых eec7 ELF. Есть Rust std, ring/native assembly, CXX-Qt/initializers и libc_nonshared | Первичные package/toolchain лицензии и notices; source/patch/build данные фактически встроенного кода. Build-only/dev не объявляются runtime через metadata union |
| Qt headers/generated code | Qt shared libraries внешние, но SDK headers, templates, QML cache и generated C++ могут попадать в собственные ELF | Этот embedded/header/generated слой не исключается из оценки лишь потому, что libQt6 находится в OS. Нужны первичные лицензии/атрибуция реально включённого кода; конечные условия не определяются этим scan |
| Qt DSO/plugins/QML и OS | Не копируются в package78. Их предоставляют пакеты OS; 59-package dataset — наблюдённая VM closure | Метаданные, версии, notices и условия выбранной интеграции сохраняются. Не утверждается, что YonIk redistributes каждый наблюдённый OS file |
| Engines/registry | В package78 нет xray/sing-box/registry.json и нет downloader/install hook для них. Отдельный системный поставщик устанавливает trusted registry | Если engines включат в конкретную поставку/provider payload, их собственные notices, исходники/patches и условия должны оцениваться по его фактическим bytes. Preserved SDK cores не объявлены финальным patched adapter registry |
| Official Xray ZIP data | Использован только для сравнения, не исполнялся/не устанавливался. geoip/geosite и README из ZIP не становятся payload YonIk | Если такие данные позднее начнут поставляться, потребуется их отдельный inventory/оценка условий |
| Build tools / VM / cache | Git/makepkg и исходные object packs использованы только в одноразовом guest/data experiment | Не объявляются runtime-поставкой YonIk; code generation/включение tool output рассматриваются отдельно от самого build tool |

Фактический package78 **не устанавливает notices-каталог**. Это конкретный остаток
упаковки финального release payload, а не доказательство, что raw notices отсутствуют
в репозитории. PKGBUILD, README и продукт в этой работе не менялись.

## Xray: exact publisher asset найден

[Publisher evidence](xray-publisher.json) связывает preserved ELF
`cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`
с официальным [XTLS/Xray-core v26.10.31](https://github.com/XTLS/Xray-core/releases/tag/v26.10.31).
Repo id `311315731`, owner XTLS id `71564206`; asset id `695486844`.

ZIP SHA-256:
`e4bd9119f50ae7c41658b1abd0679ae06f98bd864a2907ff33f2f2cf7081b214`.
Он совпал с API digest и SHA2-256/SHA2-512 из
[первичного .dgst](Xray-linux-64.zip.dgst). Все ZIP entries проверялись с лимитами,
без path traversal/symlink/special entries, без извлечения executable и без запуска.
Release LICENSE совпал с уже сохранённым первичным MPL-текстом:
`1f256ecad192880510e84ad60474eab7589218784b9a50bc7ceee34c2b91f1d5`.

Это **publisher release/tag association**, не reproducible-source proof.
Release API отмечает immutable=false. `.dgst` — неподписанные checksums;
detached signature/key assets не найдены в списке release assets. Поэтому нет
подтверждённого signing key/fingerprint, и они не выдумываются. Идентичность
publisher установлена через запрошенный официальный HTTPS/GitHub namespace,
а не через offline signing identity.

Public GitHub attestations API для ZIP и ELF вернул HTTP 404:
[ZIP response](xray-zip-attestations.json), [ELF response](xray-elf-attestations.json).
Это означает, что attestation не получена; не утверждается невозможность её
существования в другом источнике. SLSA/signature verification не заявляется.
[Tag API](xray-tag.json) указывает commit
`a4a3dff5b20c2faae567b742c172c7b95e75c083`, совпадающий с сохранённым primary
source ref. Source commit **из attestation** не проверен, потому что её нет.
main buildinfo `(devel)` не переписывается. Криптографическая связь source→ELF
и воспроизводимость остаются недоказанными; source-obligations assessment
фактической поставки остаётся отдельно.

## Qt: один новый historical guest experiment

[Полные command inputs/results](qt-historical-experiment.json),
[точные пакеты и hashes](qt-historical-packages.json),
[проверенные signatures/fingerprints](qt-historical-signatures.log),
[pinned object identities](qt-pinned-objects.json),
[обвязка](qt_guest_experiment.py).

В собственной официальной Arch VM использованы Git **2.56.0-1** и checksum
функции makepkg из pacman **7.1.0.r9.g54d9411-4 / -3 / -2** соответственно
qtbase/qtdeclarative/qtsvg, как в их исторических BUILDINFO. Historical archives
скачаны из Arch Archive как data; detached signatures проверены guest keyring.
Установленный Git ELF побайтно совпал с payload исторического package archive.
Makepkg packages распакованы отдельно: исполнялась их исходная
`calc_checksum_git`, а не PKGBUILD или полный build pipeline.

Pinned objects восстановлены из официального Qt mirror с depth=1/filter,
без checkout/hooks/submodules. Tags разрешились в те же сохранённые commits;
перед checksum подтверждено отсутствие missing objects. Команды выполнялись
в guest netns, с `env -i`, пустыми system/global config, external attributes,
HOME/XDG config и hooks; source-controlled .gitattributes оставались частью tree.
Входы checksum-функции заданы тремя детерминированными helper functions;
Git archive command внутри historical makepkg не модифицировался.

Устранены только ошибки подготовки до archive: ownership tar-копии и `errexit`
на `shopt -p` при выключенном pipefail. После подготовки выполнен один набор
из трёх archive calculations. Повторные tar.umask/индекс/host archive пробы не делались.

Результат для всех трёх: guest checksum **совпал с прежним host checksum** и
**не совпал с Arch recipe checksum**. Expected/actual hashes сохранены полностью.
Host Git ELF SHA-256 `dd95de949aae5dccd0c0a25f93e1fb9a6c23b4ab5bb0100d969ba8b2719d2bef`;
guest historical Git ELF `2912a2a32f408c9b8122e476251b32f4b35f4c3f0c15961686ae87ca456d384c`.
Различие binaries/runtime libraries зафиксировано, но не объяснило mismatch.
Причина не установлена; AppImage не объявляется причиной.

Дальнейшие эксперименты в этом scope прекращены. Внешне от Arch builder нужен
конкретный source cache/object pack либо `git archive`, который даёт заявленный
checksum, плюс provenance export/attributes/config и prepare patches. Это
недостающий artifact, а не просьба начать поиск по чужим каталогам или контактировать
с людьми. Gap относится к exact-source attribution соответствующих Qt SDK/header/
embedded компонентов. Проверенное ownership внешних OS DSO и другие Linux-задачи
этим не аннулируются.

## Следующая фактическая поставка

Нужен позднейший замороженный **latest main clean build** со своим lock/features/
target/flags/provenance, inventory встроенного кода, выбранным notices payload и
проверкой итоговых условий/source obligations. Его здесь не строили. Новые eec7 ELF
с maps остаются отдельными верифицированными кандидатами; их maps не переносятся
на старый package78 и не доказывают latest product.

23 targeted tests и Ruff выполняются для дополненного integrity gate. Новый
Xray ZIP проверяется реально как data; Rust/eec7 builds и link-map replay в этом
продолжении отсутствуют. Все raw licenses сохранены. Artifact scan не является
юридическим заключением или полным P25 PASS.
