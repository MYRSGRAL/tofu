# A4: preflight package provenance

[Design contract root](../design/package-trust-contract.md) определяет TCB,
canonical Git anchor, fixed registry, source selection и trusted native job.
Initial 128 — не final Linux freeze.

## Controller operations

После review tooling и выбора final source root/доверенный CI запускает отдельно:

~~~sh
python tools/release_controller.py freeze --release-id root-selected-id \
  --commit FULL_REVIEWED_SOURCE_SHA
python tools/release_controller.py ingest-cores --release-id root-selected-id
python tools/release_controller.py collect --release-id root-selected-id
~~~

Идентификатор использует строчные латинские буквы/цифры, "_" и "-".
Ingest опционален: без него collect запускает frozen native builder для обоих
cores. Он читает только закреплённый trusted PR167 job path; caller не передаёт
manifest/record paths. Ingest сверяет accepted binary hashes, builder из reviewed
source158, frozen patches и source locks. Collect восстанавливает pinned sources,
применяет patches, сверяет actual source package и GoBuildInfo.

Freeze сам получает source commit/tree/archive из canonical common Git project.
Он создаёт новый release directory и FROZEN request без перезаписи предыдущего.
Collect проверяет content hashes tooling, материализует точный Git snapshot,
проверяет tracked release-recipe.json против fixed controller recipe и frozen
PKGBUILD seal. Затем выполняет фиксированные cargo commands под общим Rust slot,
jobs=1 с target вне checkout; запускает native builders либо использует явно
controller-ingested accepted cores. Никаких makepkg/install/hooks не исполняется.

Producer собирает package.tar из actual outputs и frozen package generator:
три Rust ELF, оба core ELF, compiled notices/licenses, metadata, upstream/patched
core archives, manifests и source offers. Go dependencies сопоставляются с frozen
composition, их source zips проверяются по pinned SHA. Core ELF headers проверяют
ELF64 little-endian x86_64; версии, patches и ABI spec commit берутся из frozen
adapters metadata и trusted native builder observations. Candidate ELF не
исполняется. Статические проверки не заявляют bit reproducibility.

COLLECTED record публикуется O_EXCL только после успешной проверки полного
package payload и cargo clean. Он связывает source/archive, fixed recipe,
toolchains, команды с exit codes, фактические ELF/core observations и каждый
payload file. FROZEN request остаётся неизменным. Private registry records читаются
через NOFOLLOW, с own UID/mode/nlink проверками под shared root/release locks;
writer удерживает exclusive release lock. Same-UID attacker вне модели угроз.

Тяжёлый task root в /var/tmp/codex-myrs/transient удерживается flock от создания
до завершения команд; outputs и command/native-builder logs сохраняются в registry
вне TTL. При успехе own target очищается cargo clean, собственный task root удаляется.
При failure COLLECTED не появляется; оставшийся task требует диагностики.

## Candidate verification

~~~sh
python tools/check_package_provenance.py --release-id root-selected-id \
  --package /path/to/package.tar --inputs /path/to/release-inputs
~~~

Inputs должны содержать exact source.tar, соответствующий release. Их содержимое
не выбирает registry/source/recipe. Старые candidate/expected/repo/build-record
аргументы отвергаются. Verifier берёт только controller-collected context из
фиксированного registry, материализует frozen snapshot, сверяет actual package hash,
metadata/compliance с frozen generators и каждый payload digest, включая оба core
ELF/source packages. PASS относится только к local TCB provenance preflight,
final_linux_acceptance всегда false. Без actual collected job — JSON BLOCKED, exit 2.

Внутренний _validate_materials остаётся диагностикой старого untrusted schema 1,
результат MATERIALS_CHECKED не создаёт authority и не используется production CLI.

## Проверки R2

tests/tools/test_package_provenance.py и test_release_controller.py запускаются
через unittest discovery с явным -s tests/tools. FakeTrustedRunner проходит всю
цепочку freeze/collect/package/verify в отдельном TemporaryDirectory; это synthetic
evidence, не production build. Negative controls: fabricated/self-consistent
records, отсутствующий registry/collection, redirect inputs, wrong core/version/ELF,
extraneous compliance, изменённый checkout, symlink/traversal/mixed prefix.
Проверки архивов/путей выполняются до source extraction/hash; ссылки и специальные
файлы не входят в поддерживаемый source closure.

Текущая production readiness остаётся BLOCKED до actual final freeze/build.
candidate-readiness.json не изменён; новых GUI/VM/host install artifacts нет.
