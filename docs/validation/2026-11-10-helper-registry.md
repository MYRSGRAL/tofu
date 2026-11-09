# Доверенный реестр системных ядер

Срез `feat/helper-trusted-registry`, база `63fb1a5` (слитый PR #28 с зелёными
push/PR workflows). Новый компонент — часть P12, не завершённый network helper.

## Реализация

`TrustedRegistry::system()` открывает только `/usr/lib/yonik/engines` через
FD-relative NOFOLLOW walk. Проверяются UID0, типы и права всех компонентов.
Manifest `registry.json` ограничен 4096 байтами; strict schema, 1–2 уникальных
allowlisted engine, закреплённые версии и SHA-256 syntax. Пути/argv не принимаются.

`snapshot(engine)` лениво открывает выбранный executable, проверяет owner/mode,
nlink/type/size и передаёт тот же File в `BinarySnapshot::load_file`. Существующая
проверка ELF/хеша и sealed memfd сохраняется. Два ядра не копируются заранее.
Обычный непривилегированный `BinarySnapshot::load(path)` сохраняет прежнюю семантику.

## Проверки

- 6 групп filesystem unit-тестов: subset/оба ядра, malformed/unknown/duplicate
  manifest fields и entries, версии/хеши, owner/mode/setid/sticky, hardlink/symlink,
  FIFO/директории/oversize, lazy verification, pinned directory и sealed inode.
- 2 существующих regression-теста snapshot: неизменяемость и отказ FIFO/oversize.
- Полный workspace на Rust 1.85.1: **372 passed, 0 failed, 7 ignored**.
- Clippy workspace/all-targets с `-D warnings`, fmt и diff check: PASS.
- Независимый read-only security review: подтверждённых findings нет.

Тесты используют собственный каталог и закрытый cfg(test) constructor с UID
fixture. Системная установка не создавалась, реальные ядра не запускались,
производственный root-owned walk проверен чтением кода, не записью в `/usr`.
Manifest version не заменяет фактический version/config-check существующего host.

Evidence: `local/validation/helper-registry/`. P12 остаётся частичным:
нужны доверенный D-Bus service, Prepare/Apply, реальные сетевые эффекты и recovery.
