# Linux notices: проверяемый частичный inventory

Команды из корня checkout (Rust 1.85.1, локальный Cargo cache, сеть Cargo запрещена):

```sh
python3 tools/check_linux_notices.py --output packaging/licenses/linux-inventory.json
python3 tools/check_linux_notices.py --check packaging/licenses/linux-inventory.json
python3 -m unittest discover -s tests/tools -p test_check_linux_notices.py
```

Report генерируется из `cargo metadata --locked --offline --filter-platform
x86_64-unknown-linux-gnu --features yonik-desktop/gui`. Включены normal/build
рёбра от packages трёх бинарников PKGBUILD; dev-only рёбра исключены.
Features в metadata объединены Cargo, включая влияние dev-зависимостей;
это консервативный source inventory, **не точный состав runtime ELF**.
Build/proc-macro зависимости здесь также не объявляются runtime-кодом.
PKGBUILD закрепляет более раннюю revision: report хеширует рецепт, но не
утверждает, что пакет этой revision уже собран из текущего checkout.

`spdx_declared` сохраняет выражение Cargo без выбора ветви OR/AND/WITH.
`license_files` и `notice_files` перечисляют найденные первичные файлы
LICENSE/LICENCE/COPYING и NOTICE/AUTHORS/COPYRIGHT, включая подкаталоги;
их SHA-256 связывает source с копией в `texts/`. Одинаковые тексты дедуплицированы.
Имена файлов не доказывают полноту уведомлений внутри исходного кода.
Пути manifest относительно checkout у registry packages указывают в локальный
Cargo cache; на другой машине нужно заново сгенерировать report.
Validator сверяет closure, features, recipe/source hashes и тексты с локальными
исходниками; подмена provenance или исчезновение NOTICE приводит к ошибке.
Наличие SPDX без текста не считается достаточным. Восемь отсутствующих текстов
подключены через `vendor/sources.json`: package/version/source, checksum из
Cargo.lock и commit из shipped `.cargo_vcs_info.json` проверяются до чтения
текста. Разрешены только имена файлов внутри vendor, FD-relative NOFOLLOW,
regular file до 4 MiB, непустой текст и точный SHA-256. Оригинальные байты
сохранены; URL закреплены commit, ветвь latest не используется.
Реальный gate: 225 packages, 0 ошибок (см. evidence за 2026-11-11).

Слои поставки:

- Собственные desktop/helper/worker: GPL v3 в корневой LICENSE; Cargo намеренно
  ссылается на license-file без придуманного SPDX. PKGBUILD декларирует GPL-3.0-only.
- Rust: 225 packages source closure; тексты и заявленные SPDX в report, связь
  с конкретными байтами готовых бинарников unknown.
- Xray 26.10.31: первичный MPL-2.0 текст сохранён в upstream/ с URL и хешем.
- sing-box v1.14.4, commit c005f35300ce76427dc285481ec5695f5b179cd2:
  первичный LICENSE содержит GPL v3-or-later и дополнительное условие об имени.
  Полный оригинал сохранён; это условие не нормализовано в выдуманный SPDX WITH.
  Патчи wrapper/adapter находятся в adapters/sing-box/v1.14.4; требуются исходники
  точной сборки и Go module inventory. Их состав и исполнение source obligations
  unknown. Корневой GPL текст доступен через собственные crates в texts/.
- Qt, SQLite и системные библиотеки: зависимости рецепта не заменяют staged ELF
  и package database среды. Точные версии, плагины, статическое включение и
  notices unknown; системные библиотеки не объявлены включёнными в Rust closure.

Это заготовка notices, не юридическое заключение, полный P25 PASS или релиз.
PKGBUILD пока не устанавливает этот каталог; изменение упаковки вне ownership.
