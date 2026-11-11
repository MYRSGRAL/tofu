# Arch/CachyOS x86_64

Рецепт фиксирует исходники e33fa1908dfb3deb5434506341674de0b3a827b4.
SHA-256 получен скачиванием GitHub source archive, не вычислен из локального
`git archive`. Для обновления менять revision, checksum и pkgver вместе; .SRCINFO генерировать при подготовке к публикации.
Повторяемость здесь означает закреплённый вход и Cargo.lock; побитовая
воспроизводимость двух независимых сборок ещё не доказана.

Сборка без установки на host (из каталога с PKGBUILD):

```sh
mkdir -p /var/tmp/codex-myrs/transient/yonik-arch-build
flock -s /var/tmp/codex-myrs/transient/yonik-arch-build env \
  CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-arch-build/target \
  CARGO_BUILD_JOBS=2 makepkg --cleanbuild
```

Для переносимого архива использовать clean VM и generic x86-64 flags:
пользовательский makepkg.conf с `-march=native` не подтверждает переносимость.
Конкретные flags/toolchain и config сохранять вместе с evidence.

Не применять `-i`/`-s`: недостающие зависимости устанавливать только в disposable
VM. Сохранить архив и evidence в workspace до `cargo clean` под тем же lock.
Сборка требует Qt6, C++ toolchain и Rust >=1.85; Qt GUI включён явно.
`makepkg --printsrcinfo` выполняется из packaging/arch.

Проверка из корня checkout:

```sh
python tools/check_arch_package.py /path/to/yonik.pkg.tar.zst
python -m unittest discover -s tools/tests -p test_arch_package.py
```

Валидатор не распаковывает архив, сверяет полный allowlist, владельцев root,
права, ELF magic и точные bytes системной интеграции/desktop/icon. Он не
доказывает подлинность ELF или работоспособность Qt. Recipe seal отклоняет
любое изменение shell-рецепта; обновлять его только после независимого review.
Это фиксированный разрешённый рецепт, а не попытка sandbox произвольного Bash.

У пакета нет install/uninstall hooks, он не включает/запускает units. D-Bus
активация после установки возможна при явном обращении клиента. Поставляются
существующие units и sysusers без расширения capabilities. Удаление пакета не
удаляет пользовательские БД, ключи, `/run` или сетевые ресурсы. Перед удалением
активную сессию следует штатно остановить: пакет не обещает автоматическую
сетевую очистку при uninstall.

Engine не входит в пакет и не скачивается при сборке/установке. Наличие
`/usr/bin/xray` или `/usr/bin/sing-box` не делает их доверенными для helper.
Отдельный системный поставщик обязан создать root-owned, недоступный для записи
пользователю `/usr/lib/yonik/engines/registry.json` schema=1 и binaries `xray`,
`sing-box` с версиями 26.10.31 / 1.14.4 и проверенными SHA-256, согласно
`helper/registry.rs`. До этого helper закономерно отказывает; копирование
непроверенного engine или добавление symlink из пользовательского cache запрещено.
Secret Service предоставляет окружение рабочего стола (например KWallet или
GNOME Keyring); пакет не изменяет keyring и настройки пользователя.

Следующий gate — disposable Arch VM: clean build, проверка реального архива,
install без автозапуска, sysusers и D-Bus/polkit, Qt smoke на чистом HOME,
проверенный engine registry, update/rollback и uninstall с контрольными БД/ключами
и чужими сетевыми ресурсами. До этой матрицы P24 остаётся частичным.

Изменение notices (planned): `package()` компилирует проверенный набор из
`packaging/licenses` и устанавливает первичные bytes и inventory в
`/usr/share/licenses/yonik`. Это изменение ещё не переводит закреплённые
`_revision`/`sha256sums` с исторического eec7 на current main. Перед финальной
сборкой нужны согласованные новый source archive, revision и checksum, независимый
review нового recipe seal и functional merge. Старые ELF не являются новой сборкой.
Генератор notices требует `python3` в build environment; согласование объявления
этого build prerequisite в makedepends оставлено root (зависимости здесь не менялись).
