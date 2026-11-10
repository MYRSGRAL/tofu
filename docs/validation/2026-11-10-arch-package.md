# Проверка Arch-пакета — 2026-11-10

Base: `e33fa1908dfb3deb5434506341674de0b3a827b4` (сверен до изменений).
Head реализации: `a6ddac26c8c84cf1b9df61ab544d0fb7fb6e64ba`.
Финальный documentation commit добавляет этот отчёт; его SHA передан в handoff/PR.
Worktree: `/home/myrs/.t3/worktrees/yonik/t3-5ce5cb91`, ветка `t3/arch-package`.
Относящиеся CONTRIBUTING/AGENTS в tracked-файлах не найдены; применены инструкции
назначения, Task4 completion-waves и docs/specs/platforms.md.

## Изменения и границы

Новые `packaging/arch/PKGBUILD`, `.gitignore`, `README.md`;
`tools/check_arch_package.py`, `tools/tests/test_arch_package.py` и этот отчёт.
Existing units/sysusers/polkit/dbus, Cargo/dependencies, GUI/helper и PLAN не менялись.
Пакет содержит Qt desktop и helper/worker, existing интеграцию по FHS и desktop/icon.
Нет install hooks, enable/start, загрузки engine и очистки пользовательских данных.
Recipe seal фиксирует весь разрешённый shell; любое изменение требует review.
Валидатор не является sandbox Bash и не доказывает подлинность ELF.

## Проверки

- TDD RED: два ожидаемых assertion failure до появления validator.
- `python -m unittest discover -s tools/tests -p test_arch_package.py`: 4 tests OK;
  исполнение настоящего `package()` с тестовыми ELF и последующая проверка tar;
  отказы на missing binary, writable/setuid mode, неверный ELF/UID, desktop Exec,
  traversal, hook и ссылку; мутации shell-рецепта отклонены seal.
- Ruff установлен только в собственный transient venv: `ruff format` и
  `ruff check` двух новых Python-файлов — проходят.
- `bash -n packaging/arch/PKGBUILD`, `python tools/check_arch_package.py`,
  `git diff --check` — проходят.
- `makepkg --printsrcinfo` из packaging/arch — проходит; generated .SRCINFO
  не коммитится согласно общим инструкциям.
- `makepkg --nobuild --nodeps` в transient package — checksum Passed, source extracted.
  SHA-256 скачанного GitHub archive:
  `f02df043825282f51ef8ad132d6eb7895532416112964b5d388b325e44c13b34`.
- `cargo +1.85.1 fmt --all -- --check` — проходит.
- `cargo +1.85.1 clippy --locked --workspace --all-targets -- -D warnings` — проходит.
- `cargo +1.85.1 test --locked --workspace` — 525 passed, 24 ignored, 49 suites,
  без failures; GUI feature этим запуском не покрывается.
- Полная Python suite `python -m unittest discover -s tools/tests`: 122 tests,
  1 failure `test_ui_benchmark.ChildTests.test_long_line_and_duplicate_ack_fail`
  (`BenchmarkError not raised`). Чужой код не исправлялся; suite не объявляется зелёной.

Cargo: jobs=2; `/var/tmp/codex-myrs/transient/yonik-arch-package/target`, flock самого
корня task с первого использования. Первый clippy без --locked нормализовал
Cargo.lock; это собственное изменение было отменено, проверки повторены с --locked.
В итоговом diff Cargo.lock отсутствует.

## Оставшаяся приёмка

Повторное review обнаружило различие границ строк с libalpm: Unicode
`splitlines()` мог выделить зависимость внутри LF-комментария. Новая регрессия
воспроизвела шесть обходов (CR/VT/FF/NEL/LS/PS). Парсер теперь разделяет
записи только по LF; все шесть unittest и проверка реального архива проходят.

Независимое review обнаружило P2: отсутствие или подмена `.PKGINFO` не
отклонялись валидатором. Регрессии сначала подтвердили пять ложных успехов.
Теперь `.PKGINFO` обязателен, ограничен 64 KiB и mode 0644; имя, версия,
архитектура и полный список runtime-зависимостей сверяются с sealed PKGBUILD
без исполнения shell. Проверяются также каждый подменённый scalar отдельно,
отсутствующая зависимость и дубли. Все пять целевых unittest, Ruff и реальный
сохранённый архив проходят проверку после исправления; архив не пересобирался.

P24 остаётся частичным. Нужна disposable Arch VM: clean makepkg build,
install, sysusers/D-Bus/polkit и отсутствие автозапуска, Qt на чистом HOME,
проверенный root-owned registry и engine versions/hashes, update/rollback,
uninstall с контрольными пользовательскими БД/ключами и чужой сетью.
Engine не поставляется: xray 26.10.31 / sing-box 1.14.4 принимаются только через
существующий helper registry, обычная установка бинарника в PATH недостаточна.
SPDX variant, dependency notices и побитовая воспроизводимость остаются gate P25.
На host пакет/helper не устанавливались; systemd, сеть, keyring не менялись.
Независимый review и merge выполняет root; исполнитель не запускал вложенных агентов.

## Реальный архив и завершение

- Все declares depends/makedepends проверены `pacman -T`: exit 0, список missing пуст.
- `cargo +1.85.1 build --locked --release -p yonik-desktop --features gui
  --bin yonik-desktop -p yonik-linux --bin yonik-helper --bin yonik-worker`:
  exit 0, 8m36s. Есть C++ GCC/Qt SFINAE warnings; они не скрывались.
- Реальный `makepkg --config ../makepkg.conf --noextract --nodeps`:
  exit 0; release build закреплённых распакованных source 4m25s, fakeroot/strip/
  package compression выполнены. `--nodeps` не устанавливает зависимости;
  их наличие отдельно подтверждено `pacman -T` выше.
- Host makepkg config включает native flags/LTO; этот пробный запуск остановлен.
  Для принятого архива использован отдельный transient config:

```bash
source /etc/makepkg.conf
unset RUSTFLAGS CFLAGS CXXFLAGS CPPFLAGS LDFLAGS
OPTIONS=(strip docs !libtool !staticlibs emptydirs zipman purge !debug !lto !autodeps)
```

`RUSTUP_TOOLCHAIN=1.85.1`, `CARGO_BUILD_JOBS=2`, тот же external target и flock.
Пользовательский config не менялся. Это local archive validation; clean VM gate
и две независимые сборки всё ещё обязательны для release/reproducibility.

- `python tools/check_arch_package.py packaging/arch/evidence/yonik-0.1.0.r20261110-1-x86_64.pkg.tar.zst`:
  exit 0, реальный архив прошёл allowlist/owner/mode/metadata/desktop/ELF проверки.
- Из проверенного архива извлечён только desktop ELF в свой transient unpack;
  `python tools/desktop_ui_smoke.py <unpack>/usr/bin/yonik-desktop
  --output packaging/arch/evidence/smoke`: exit 0; 142 steps, 42 events,
  3 TCP EOF, 7 HWID fixture requests; offscreen, software Qt, собственные data-dir
  и HTTP/TCP fixtures. Пользовательские подписки/данные не использовались.
- Повторные targeted tests (4), Ruff format --check/check, bash -n и archive
  validator перед завершением — проходят.
- Перед очисткой проверено отсутствие активных cargo/rustc/C++/zstd процессов
  этой задачи и symlink workspace в transient. `cargo +1.85.1 clean --target-dir
  /var/tmp/codex-myrs/transient/yonik-arch-package/target` выполнен; собственный
  transient root затем удалён полностью.

Evidence вне TTL (gitignored, бинарные/генерированные артефакты не коммитятся):
`/home/myrs/.t3/worktrees/yonik/t3-5ce5cb91/packaging/arch/evidence/`.
Там архив, source/build/makepkg/clippy/test/cleanup logs, принятый config,
сгенерированная SRCINFO и smoke result/trace/screenshots.
SHA-256 архива:
`2b8110af8d8eb335ff2897edde87f84474dd65ac87bfe9522b0d91f93dd5bb7e`.
