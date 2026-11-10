# Лицензионные метаданные Cargo — 10 ноября 2026

База: `e33fa1908dfb3deb5434506341674de0b3a827b4`.

## Изменение

Семь собственных crates наследуют `license-file` из workspace и ссылаются
на существующую корневую `LICENSE`. Текст лицензии не менялся. SPDX-выражение
не назначалось предположением: этот срез исправляет отсутствующую ссылку на
лицензионный документ, а не выбирает дополнительные условия лицензирования.
Зависимости, версии и Cargo.lock не изменены.

Наследование пути соответствует [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-package-table);
`license-file` — поддерживаемое поле [Cargo manifest](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields).

## Проверки

- `cargo +1.85.1 metadata --locked --all-features --offline --format-version 1`
  до изменения: 274 packages, включая 7 собственных и 267 внешних. У всех
  внешних packages заявлено поле license; у собственных не было ни license,
  ни license_file. Это инвентаризация metadata, не независимая проверка текстов.
- `cargo +1.85.1 metadata --locked --no-deps --offline --format-version 1`
  после изменения: у всех семи crates license_file разрешается относительно
  каталога соответствующего manifest в существующую workspace LICENSE.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: успех.
- Сборка не запускалась: изменена только metadata, Cargo target не создавался.

## Остаток P25

Граф metadata с all-features включает зависимости разных targets и dev/build
контекстов. Он не доказывает точный состав распространяемого Linux-бинарника.
Заявленный SPDX не подтверждает, что пакет содержит все требуемые notices или
что выбрана допустимая ветвь сложного выражения AND/OR/WITH.

Перед поставкой остаются инвентаризация конкретных артефактов и системных Qt
библиотек, включение необходимых лицензий/уведомлений и source obligations
закреплённых Xray/sing-box и собственных patches. P25 остаётся частичным;
публикация или релиз этим изменением не выполняются.
