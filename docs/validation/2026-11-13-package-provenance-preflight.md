# A4: preflight package provenance

Production CLI принимает только release-id и пути проверяемых материалов:

```sh
python tools/check_package_provenance.py --release-id root-selected-id \
  --package /path/to/package.pkg.tar.zst --inputs /path/to/retained/artifacts
```

Старые аргументы candidate/expected/repo/output удалены. Произвольные JSON,
включая совпадающие самосогласованные FROZEN и build record, не являются authority.
Сейчас CLI всегда возвращает JSON BLOCKED, final_linux_acceptance=false и exit 2.
Финальный PASS manifest не создаётся. Registry record сам по себе также не
открывает положительный путь.

## Bounded contract root

Canonical Git project: /home/myrs/projects/yonik, origin MYRSGRAL/yonik.
Root registry выводится из common Git directory reviewed tooling, а не из
candidate args, cwd или GIT_* env. Первоначальный reviewed anchor:
128a070884e82bf21b5c22a74ba908dadd7daf5a. Это принятый main, НЕ final Linux freeze.
Final source после интеграции выбирает root; candidate caller не выбирает revision.

Authorized producer — локальный release-controller из reviewed tooling,
вызванный root или доверенным CI job. Он должен сам исполнить fixed build recipe
на exact frozen archive, собрать команды/toolchains/outputs и native builder
metadata. Импорт candidate build record не допускается.

Registry: /home/myrs/projects/yonik/local/release-controller/<release-id>/frozen.json.
Verifier не создаёт registry и не регистрирует release. Чтение descriptor-relative,
O_NOFOLLOW, shared BSD lock корня registry; каталоги own UID 0700, records —
own UID regular 0600 nlink=1, ограничение размера 1 MiB.
Это hygiene и защита от случайной подмены пути, а не защита от malicious same UID.
TCB включает authorized build controller, same-UID процессы, root/operator;
злоумышленник с такими правами находится вне модели угроз. Новых daemon,
capabilities, crypto или сетевой authority нет.

## Состояние реализации

**BLOCKED:** producer не реализован. Controller-facing API
FrozenBuildRequest/collect_release отделён от candidate CLI. Сейчас collect_release
выдаёт NotReady и не записывает регистрацию. Он не принимает caller attestation.
До реализации реального исполнения/сбора нет production trusted context и PASS.

Условия для последующего включения producer:
- exact frozen Git tree/archive и fixed recipe revision, выбранные root;
- commands/toolchains и SHA actual build outputs, включая оба core ELF;
- static ELF headers и trusted SHA без исполнения package ELF на host;
- core ABI 260, версии, pins/patch metadata из frozen tracked adapters locks,
  GoBuildInfo/architecture/version/source-package hashes из trusted native job;
- actual package compliance против frozen packaging/licenses/composition,
  compiled notices, pinned core source offers/archives/source hashes.

Никакой готовый Go native builder evidence не считается полной Rust/GUI сборкой.
Исторические #119/#147 здесь не пересобираются. Ни initial 5fd/128, ни retained a8,
ни ожидаемый результат интеграции не выдаются за final Linux freeze.
candidate-readiness.json не изменён.

## Проверки материалов

Внутренний _validate_materials — только структурная диагностика недоверенных
материалов, результат MATERIALS_CHECKED; не provenance и не production entrypoint.
Оpaque hashes cores/compliance здесь по-прежнему не доказывают actual build binding.

Пути артефактов проверяются до hash: traversal, ссылки во всех компонентах и
необычные файлы запрещены. Source archive предварительно проверяется целиком:
один prefix, канонические пути, отсутствие ссылок/специальных файлов, повторов
и лишних каталогов. Git symlinks не входят в поддерживаемый closure.
Пакетный checker получает snapshot проверенных frozen Git blobs, а не mutable
checkout. Код snapshot не исполняется.

Тесты tests/tools/test_package_provenance.py используют синтетические материалы.
Они не создают и не подделывают actual production build record. Проверяются
отказ fabricated FROZEN, отсутствие registry, redirect inputs, wrong core и
compliance, режимы/ссылки registry, unsafe paths/archives и mutable checkout.
Новые сборки GUI, VM lifecycle и host install не выполнялись.
