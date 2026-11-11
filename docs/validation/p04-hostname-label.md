# Граница DNS label в managed endpoint

Источник finding: PR93 `eb6f7c`, только наблюдение 63/64/65;
production edits и performance campaign из этого PR не переносились.
Исходный evidence: `/home/myrs/projects/yonik/local/validation/p04-duration/native-boundaries/results.json`.

Действующий контракт [импорта](2026-11-09-subscription-import.md) описывает endpoint
как IDNA/IPv4/IPv6/port. Managed URI/VMess адреса проходят `uri::host`:
IpAddr либо `url::Host::parse`, возвращающий нормализованный ASCII domain.
Именно в ветке Domain проверяется длина label ≤63 октетов после IDNA.
Это проверка структуры hostname, без DNS lookup. Новых зависимостей нет.

Новые tests `hostname_label.rs`:

- ASCII label 63/64/65: 63 принимается, 64/65 дают InvalidHost/Endpoint,
  без ParsedServer.
- Punycode label `xn--9c` + 57/58/59 букв `a` имеет 63/64/65 октетов.
  Проверяются как готовый A-label, так и соответствующие Unicode `é` × 57/58/59;
  длина исходного Unicode в символах не подменяет длину ASCII label.
- IPv4, IPv6, underscore, обычный IDNA, завершающая root dot и неизвестный
  opaque native JSON сохраняют прежнее поведение. STD3-валидация и изменение
  opaque server-полей не добавлялись.

Логи RED/GREEN и финальные проверки находятся вне transient build:
`/home/myrs/projects/yonik/local/validation/actual-cancel/`.
Это отдельная семантическая регрессия; отсутствие других семантических ошибок
или полный PASS P04 этим изменением не заявляется.

RED: `red-hostname-label.log` — 2 failed (ASCII и IDNA принимают 64 октета),
контроль совместимости прошёл. GREEN: весь `yonik-import`, включая три новые
регрессии, прошёл; `hostname-clippy.log` — all-targets Clippy чистый.
Финально Rust 1.85.1 workspace tests: 776 passed, 0 failed, 39 ignored;
workspace all-targets Clippy и fmt check прошли. GUI features не включались.
