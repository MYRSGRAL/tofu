# Linux composition: статический частичный шаг, 2026-11-11

База реализации: `ca225267432efe1dc96243fcfcc2a8d914820c8e`.
HEAD перед работой совпал, дерево было чистым. Изменений существующего notices
checker, Rust/GUI/helper/PKGBUILD нет. 225-package/0-missing inventory не пересобирался.

## Входы и воспроизведение

Пакет: `/home/myrs/.t3/worktrees/yonik/t3-0e71b13a-c31bb75af8e3d15e/local/validation/arch-clean/yonik-0.1.0.r20261110-2-x86_64.pkg.tar.zst`.
Исторический clean Arch package 78, source eec7; это не latest product.
Ядра: `/home/myrs/projects/yonik/local/engines/{xray-26.10.31/xray,sing-box-1.14.4/sing-box}`.

SHA-256 пакета: `e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0`.

- `xray-26.10.31/xray`: `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.
- `sing-box-1.14.4/sing-box`: `1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a`.

[Manifest](linux-composition.json) содержит хеш source inventory, каждого ELF,
DT_NEEDED, PT_INTERP, RPATH/RUNPATH, полные PKGINFO/BUILDINFO, module versions/h1
и разрешённые build settings, URL и проверенные хеши первичных core LICENSE.
Байты LICENSE остаются в upstream без изменений. Соответствие LICENSE точному
source checksum бинарника пока unresolved. Версия в имени каталога не доказательство.

```sh
python tools/check_linux_composition.py --package /путь/yonik.pkg.tar.zst \
  --core /путь/xray-26.10.31/xray --core /путь/sing-box-1.14.4/sing-box \
  --check packaging/licenses/linux-composition.json --output /путь/verification.json
python -m unittest discover -s tools/tests -p test_linux_composition.py -v
ruff format --check tools/check_linux_composition.py tools/tests/test_linux_composition.py
ruff check tools/check_linux_composition.py tools/tests/test_linux_composition.py
```

На реальных входах scan и независимая повторная сверка `--check`: exit 0.
10 unit tests: PASS. Ruff format/check: PASS (существующий локальный Ruff,
без установки). Negative tests: traversal/absolute paths, symlink/hardlink/FIFO,
дубликаты, лимиты размера/числа, symlink-компоненты входа и подмена manifest.
Временные ELF читались readelf и `go version -m`, не исполнялись; ldd не применялся.
Cargo, GUI, сеть продукта, helper, keyring, host install и Docker не запускались.
Временные копии удалены автоматически; тяжёлых сборок/TTL-каталогов нет.

## Что доказано и что осталось

Три ELF пакета; desktop имеет семь прямых Qt6 DT_NEEDED, helper/worker — SQLite
и системные библиотеки. В самом пакете plugins не найдены. PT_INTERP и search
paths зафиксированы; это не resolver и не полный транзитивный runtime closure.
BUILDINFO: qt6-base/declarative 6.12.0-4, qt6-svg 6.12.0-1,
sqlite 3.54.1-1, glibc 2.44+r50+g1848099f063e-1. Это версии build environment,
не доказанная принадлежность загруженных файлов или комплект notices Arch.
Xray: 45 module rows, main `(devel)`, revision отсутствует. sing-box: 141 module
rows, main `(devel)`, revision c005f35300ce76427dc285481ec5695f5b179cd2, modified=false.
Оба CGO_ENABLED=0; это не доказательство отсутствия всех native/static компонентов.
Module h1 — checksum module content, не checksum исходного архива главного проекта.

Остаток до точного состава:

- Получить file ownership/package metadata целевой Arch-среды, package/source
  checksums и первичные license texts точных Qt/SQLite/системных пакетов.
- В private GUI/VM подтвердить Qt plugin/QML/dlopen и loader resolution,
  хеши фактических файлов; host session не использовать как замену.
- Получить pinned module source trees/первичные notices всех Go dependencies,
  exact main sources/patches/build provenance обоих ядер.
- Согласовать с root новую Rust-сборку с provenance/link map, если сохранённых
  данных недостаточно. Связать inventory с exact-build lock/features/target;
  отделить build/proc-macro и dev от runtime, учитывать static inclusion/dead code.
  Cargo metadata closure сам по себе этого не доказывает.

Статус partial намеренный. Artifact scan не юридическое заключение; P25 не PASS.
