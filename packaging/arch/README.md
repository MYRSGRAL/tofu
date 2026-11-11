# Arch/CachyOS x86_64

Рецепт фиксирует исходники 6cf65a4cf9760a7df68b8eee10f012770b38604d.
SHA-256 получен скачиванием GitHub source archive, не вычислен из локального
`git archive`. Для обновления менять revision, checksum и pkgver вместе; .SRCINFO генерировать при подготовке к публикации.
Повторяемость здесь означает закреплённый вход и Cargo.lock; побитовая
воспроизводимость двух независимых сборок ещё не доказана.

Сборка без установки на host (из каталога с PKGBUILD):

```sh
mkdir -p /var/tmp/codex-myrs/transient/yonik-arch-build
flock -s /var/tmp/codex-myrs/transient/yonik-arch-build env \
  CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-arch-build/target \
  CARGO_BUILD_JOBS=1 makepkg --cleanbuild
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

Actual clean source build, install и Qt smoke описаны в
`docs/validation/2026-11-11-arch-source-6cf65a4.md`; versioned package
old→new→old→uninstall и actual guest reboot — в
`docs/validation/2026-11-11-arch-versioned-119.md`. Эти результаты не закрывают
полную Linux acceptance, trusted engine registry и миграцию зашифрованных данных.

Notices компилируются из закреплённого source при сборке; Python объявлен в
makedepends и устанавливается только в VM. Рецепт поставляет три Rust ELF;
внешние core binaries остаются отдельной зависимостью и в архив не включены.
`payload.list` генератора задаёт явный список license payload. Валидатор сверяет
inventory и SHA-256 текстов с исходниками. Generated Qt resources находятся в OUT_DIR.

Source freeze: main 6cf65a4. Незамёрженные #107 (maintenance) и #113 (Policy)
исключены из этой source phase. Проверка пакета не означает полный P25 или
финальную Linux acceptance; unresolved hashes трёх Qt archives не пересчитываются.

Runner `tools/arch_clean_vm_build.py` использует только подписанный cache
`local/validation/vm-base` под shared BSD lock; overlay и cloud-init принадлежат
текущей задаче. Общий Rust slot занят только тяжёлой guest-сборкой.

Проверка versioned lifecycle без повторной Rust-сборки:

```sh
python tools/arch_clean_vm_build.py \
  --task-root /var/tmp/codex-myrs/transient/yonik-versioned-check \
  --evidence /path/outside/ttl/versioned-check \
  --arch-public-key /home/myrs/projects/yonik/local/validation/vm-base/arch-ci-public-key.asc \
  --old-package /path/to/yonik-0.1.0.r20261110-2-x86_64.pkg.tar.zst \
  --new-package /path/to/yonik-0.1.0.r20261111-1-x86_64.pkg.tar.zst
```

Этот режим принимает только два проверенных archive SHA-256 из #78/#119;
переименование нового пакета в старый отклоняется до VM effects. Manifest
сохраняет source revisions, package metadata, BUILDINFO и hashes ELF каждого
архива. Rust toolchain не устанавливается, Cargo/makepkg не вызываются.
После гостевого `-Syu` runner проверяет смену boot ID, kernel и наличие modules,
затем устанавливает old→new→old и удаляет пакет. Реальный desktop запускается
под builder на одном каталоге данных; smoke использует ephemeral key, без keyring.
Fixture проверяет контрольную строку настоящей SQLite БД, hashes и права keyfile/
preferences, installed ELF, inactive/3, Qkk и чужие nft/interface resources.
Preferences нового формата сохраняются побайтово без замены каталога данных.
После первого процесса следующий smoke ephemeral key отличается: на сохранённой
зашифрованной БД проверяется фактический fail-closed `storage`, `ready=false`.
Разбор future preferences и отказ записи именно по их схеме в этом маршруте
не достигнуты и не заявляются. Этот сценарий не доказывает миграцию зашифрованных
пользовательских записей с постоянным ключом. FAIL run и его artifacts сохраняются;
чужие cache/overlay не удаляются.

Для userdata acceptance добавьте к той же команде `--production-custody`.
Этот режим использует установленный desktop без `--smoke-test`, собственные
Xvfb/tray и private D-Bus/gnome-keyring внутри VM. Старый GUI создаёт encrypted
profile и preferences; новый читает их с тем же production Secret Service key,
создаёт preferences нового формата; rollback старого GUI читает профиль и
явно отказывает записи неподдерживаемых preferences без их перезаписи.
Проверяется persistence Secret Service при restart provider и сохранность после
uninstall. Raw key, keyring password и содержимое keyring не экспортируются;
evidence содержит metadata, hashes, encrypted SQLite snapshots и screenshots.
VM-only пакеты стенда не добавляются в зависимости дистрибутивного пакета.
Подробности: `docs/validation/2026-11-11-arch-production-custody.md`.
