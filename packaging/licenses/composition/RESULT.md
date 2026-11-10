# Composition Linux: подтверждённый результат и остаток

Дополнение PR85: [P25 scope / Xray publisher / historical Qt experiment](P25-SCOPE.md).
Состояние ниже — snapshot предыдущей волны; новые результаты и уточнённые границы
поставки приведены в дополнении.

База PR: `ca225267432efe1dc96243fcfcc2a8d914820c8e`.
Исторический package78 и source eec7 не объявляются latest продуктом.
[Integrity result](verification.json), [исходный статический manifest](../linux-composition.json).

## Входы и новые артефакты

Замороженные исходники: commit `eec7a56b6b03f2b022a87682f9097ce78b61fa0b`,
source archive SHA-256 `71e9c347ffc363da062b42b0c5ffc6ccb0a97b0988202ce21391dd1c2553a1d3`.
Package78 SHA-256 `e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0`.
Принятый PKGBUILD из commit `1a3cb4d`: SHA-256
`2cb9e296901b0f72baa230d1ec4c680e94d61ec4c453de136ddab68d6ea4be7a`,
точно совпадает с package78 BUILDINFO. Сам PKGBUILD продукта не изменён.
Lock SHA-256 `d35687ddb7cb51101abd638ef437c468936a9f404b92e46efbb3a218e1ac2f03`
совпадает до/после сборки и с принятым 225-package inventory.

Новые ELF (самостоятельные результаты, не байты package78):

- `yonik-desktop`: `1ab3b7d5a740d98feb8b15cdeb5fc08f41bf707c101dd636c4fde3ef6f0e098c`.
- `yonik-helper`: `02b72dccb0b3f111931d0d6c4f7aa0019e6ed3ca4689f1e7327f2be32a01c682`.
- `yonik-worker`: `a9b6ec2e637fa633bf1be1f0f6d794c58c1176b94904ca0d780ea9401f616887`.

Сохранённые ядра:

- Xray: `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.
- sing-box: `1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a`.

## Подтверждённые слои

- 173 уникальных dependency module/version/h1 из buildinfo двух ядер: каждый
  primary module zip проверен по алгоритму Go dirhash/h1, сохранён zip SHA-256,
  versioned URL и найденные raw LICENSE/NOTICE/AUTHORS/COPYRIGHT/PATENTS/COPYING.
  [Go sources](go-sources.json). 45/141 module rows исходного manifest включают
  main; это 44/140 dependency rows с пересечением.
- Primary sing-box source archive по buildinfo commit c005f353… и исходники Xray
  tag v26.10.31 с разрешённым commit и source archive SHA-256:
  [core sources](core-sources.json). Main sing-box — v1.14.4, Xray — (devel).
  Восстановление байтов main Xray этим не доказано.
- Источники двух наблюдённых Go toolchains go1.27.3/go1.26.10: закреплённые
  publisher commits, archive hashes, по 38 named notices, включая vendored
  stdlib sources: [Go toolchain sources](go-toolchain-sources.json).
  [Корневые тексты и vendor module list](go-toolchains.json) сохранены отдельно.
- Официальный Arch cloud image SHA-256
  `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`;
  подпись проверена primary key `1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`
  из официального Arch-boxes README. [Image provenance](image.json),
  [VALIDSIG](image-signature-official.log). На host ничего не установлено.
- Desktop именно package78 запускался в собственной VM внутри отдельного netns,
  private DBus и Xvfb/software renderer, с пустыми собственными data dirs и
  недоступным system bus. Сетевых подключений продукта/helper/keyring не было.
  Четыре /proc maps snapshots; 124 mapped и 43 дополнительно opened records.
  21 directory/nonregular observation не выдаётся за код.
  [Qt runtime](qt-runtime.json): maps отличаются от open-only plugin enumeration;
  открытие metadata плагина не объявляется его загрузкой/исполнением.
- 137 уникальных resolved runtime paths побитово совпали с payload 59 точных
  Arch package archives. Их SHA-256 совпали с sync database; pacman проверил
  signatures/integrity. [Packages, ownership, raw notices](arch/arch-packages.json).
  Common licenses атрибутированы отдельно, без выбора ветви SPDX OR.
- Для всех 56 исходных pkgbase primary PKGBUILD получен по exact version tag,
  закреплён commit, его SHA-256 совпал с BUILDINFO каждого соответствующего
  пакета. Все source/checksum arrays и patch/cherry-pick refs сохранены raw;
  PKGBUILD не исполнялись: [Arch recipes](arch-recipes.json).
- Новая сборка Rust 1.85.1 / x86_64-unknown-linux-gnu, release, jobs=1 в guest.
  Host with-rust-slot удерживался весь lifetime VM, task root под BSD flock;
  guest target также вне checkout под flock. Сначала воспроизведена ошибка
  initializer link при пропуске четырёх принятых --undefined flags; затем
  применены ровно flags принятого package78 recipe. Дополнительно только
  RUSTC_WRAPPER для invocation log и -Wl,-Map, target задан явно.
  RUSTFLAGS/CFLAGS/CXXFLAGS/CPPFLAGS/LDFLAGS unset. Ни source, ни lock/deps,
  ни PKGBUILD продукта не менялись. Точные args и map/evidence hashes:
  [Rust build](rust-build.json); [wrapper](rustc_wrapper.py).
- Actual Cargo artifacts связаны с map rlib names, версии/source/checksum и
  normal/build/proc-macro roles сопоставлены с source inventory. У всех
  выбранных identities есть совпадение с inventory. Ненулевые sections после
  Memory map (исключая discarded/нулевые) найдены в 78 Cargo archives desktop,
  44 helper и 9 worker. Native initializer archives и libc_nonshared выделены.
  Это положительное наблюдение retained sections, не исчерпывающая атрибуция
  LLVM inlining/generics/macro expansion. Metadata features остаются union;
  точные compiler --cfg лежат в сохранённом invocation log. Dev рёбра исключены.
- Лицензии/COPYRIGHT официально установленного Rust toolchain, component URLs
  и checksums из installed channel manifest: [Rust toolchain](rust-toolchain.json).

## Реальные проверки и сохранность

19 tests PASS, Ruff format/check PASS. 683 raw references проверены;
472 уникальных raw файла в staged Git blobs совпали со своими SHA-256 именами.
Первичные пробелы/CRLF и подчёркивание AUTHORS (`=======`) сохранены;
предупреждение git diff --check для этого legal text не означает merge conflict. Реальный integrity validator на package78,
двух preserved cores, всех raw текстах/recipes, cross-links runtime/payload и
новых ELF/maps PASS. Negative tests: archive path/symlink/hardlink/FIFO/size/count,
module h1/identity/raw bytes, runtime binding, Rust ELF/path tampering, отделение
build/dev/proc-macro и discarded sections. Ни ldd, ни Go build не использовались.

```sh
python tools/check_linux_composition.py --package /путь/package78.pkg.tar.zst \
  --core /путь/xray-26.10.31/xray --core /путь/sing-box-1.14.4/sing-box \
  --check packaging/licenses/linux-composition.json \
  --evidence-bundle packaging/licenses/composition \
  --rust-evidence local/validation/composition/rust-preserved/evidence \
  --output local/validation/composition/verified.json
python -m unittest discover -s tools/tests -p 'test_linux_composition*.py' -v
```

Raw maps, compiler/artifact/metadata logs, исходный lock, новые ELF и VM evidence
сохранены вне TTL в этом worktree:
`local/validation/composition/`, новые ELF — `rust-preserved/evidence/new-binaries/`.
В git ELF, полные maps и build artifacts не включены. После сохранения и проверки
отсутствия сборок выполнен cargo clean (3767 files, 960.1 MiB), guest poweroff,
QEMU exit 0; собственные transient roots удалены. [Cleanup](cleanup.json),
[Cargo clean](cargo-clean-poweroff.log), [source-cache cleanup](go-stdlib-cleanup.json).

## Точный незакрытый остаток и варианты для root

1. Xray main `(devel)` без vcs.revision: directory/tag 26.10.31 не доказывает
   соответствие main source checksum данному ELF. Dependency h1 notices закрыты,
   main binding нет. Нужен provenance исходного сборщика (source archive/patches/
   flags + связь с данным ELF SHA), либо разрешение получить официальный release
   binary asset и подписанную аттестацию/контрольные суммы для побайтового сравнения.
   Отдельный вариант — согласовать замену на заново собранное pinned ядро; Go
   rebuild/замена не входят в текущие полномочия и не выполнялись.
2. Все три текущих primary Qt v6.12.0 git archives не совпали с source checksums
   закреплённых Arch recipes: [точные expected/actual/commits](qt-sources.json).
   Raw primary notices сохранены, но их exact-source binding к Arch ELF остаётся
   unresolved. Проверен makepkg calc_checksum_git (`git -c core.abbrev=no archive
   --format tar tag`); проверка tar.umask также не дала совпадения. Причина
   расхождения не установлена; перемещение tag не утверждается. В официальном
   sources.archlinux.org/other/packages/ искомые Qt source archives не найдены
   (qt6-base directory 404, общий index без Qt). Нужны архивы/attestation сборщика
   Arch либо подтверждённый способ воспроизведения checksum. Patch refs в recipes
   закреплены; source archive hash относится к upstream до prepare patches.
3. Существующая package78 ELF composition не становится доказанной через maps
   новой сборки. Для строгого переноса нужны оригинальные maps/provenance или
   доказанная побайтовая воспроизводимость в полном историческом окружении.
   Новые ELF отдельно идентифицированы; toolchain static/vendor attribution,
   inlined/generic/macro-generated код и dead-code остаются ограничениями.
4. Runtime observation покрывает один startup XCB/software. Wayland, иные backend,
   lazy paths/страницы и плагины вне этого сценария остаются вне покрытия.
   Arch source pins/checksums и package notices не являются аудитом всех embedded
   source notices и исполнением source/redistribution obligations.

Artifact scan не юридическое заключение. Integrity PASS не повышает partial
до complete; полный P25 не заявляется.
