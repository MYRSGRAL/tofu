# Sing-box: authority после DNS и cache

База delta: `8006f1c1e4f39932b86922abf3c152bb21e46a14`.
Закреплённый upstream: `c005f35300ce76427dc285481ec5695f5b179cd2`.

`DNSGuard` использует закрытый context key существующего handoff. Revoke не удаляет
marker, поэтому повторный DNS-вызов отозванного flow не становится legacy caller.
Обычный native caller, включая context с методом `FlowContext`, marker не получает.

Реальные `dns.Router.Exchange`, `ExchangeAsync`, общий `lookup` проверяют authority
перед работой и перед выдачей результата. Проверка включает свежий owner proof,
epoch, cancel и revoke через существующий `AfterDNS`. Она выполняется на каждом
cache hit и у каждого waiter coalesced exchange. Отказ очищает адреса и provenance.
Перед reverse mapping и публикацией route addresses/resolution pin есть отдельная
проверка. Existing beforeDial/TFO/socket ownership guards сохранены.

Проверки:

- Race: broker, DNS, route, dialer.
- Controlled await: A/AAAA; Known → cancel/Unknown/Unavailable/stale/revoke.
- Настоящий приватный TCP/UDP resolver на случайных loopback-портах: оба семейства,
  положительный контроль и отрицательные переходы во время await.
- Cache hit: отдельные closed contexts, отказ не выдаёт cached IP.
- Два concurrent flow одного вопроса: один transport exchange, независимые результаты
  authority; stale первого не запрещает Known второго.
- Async callbacks: A/AAAA, после отказа reply отсутствует.
- Go vet, gofmt; builder suite 18 tests + 2 subtests; Ruff check/format.
- MSRV 1.85.1 clippy all-targets с `-D warnings`, Cargo fmt и current metadata.

Evidence сохраняется в `local/evidence/pr126-sing-dns/`: source archive, hashes,
логи, toolchain и production artifact. Это локальный ignored каталог.

Ограничения: fixtures создают закрытый тестовый handoff; production issuer остаётся
неподключённым. Отдельная OS cross-UID приёмка не заявляется этими fixtures.
Проверки DNS publication и существующих route/dial guards не являются полным
production attach→DNS→route→dial campaign. ABI 260, policy serialization, pins,
capabilities и feature gates не изменены. Android и физические G-UI/G-POWER отложены.
Host DNS, routes и private keys не используются.
