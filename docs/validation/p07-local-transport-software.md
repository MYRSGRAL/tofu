# P07: LocalSocks software lowering

Изменение относительно accepted main993943a8343dbc0fd7ef027b8ab4876348cf8ad5.
Основание: законченный source trace `local/orchestration/evidence/p07-native-lowering-contract/contract.md/json`, норматив ports — `2026-11-14-p07-port-protocol-implementation.md`.

Public `compile_policy` и LocalSocks desktop Connect допускают destination transport:
Xray получает network `tcp`/`udp`/`tcp,udp`, port `n,a-b`;
sing-box получает network list, port u16 list, port_range `a:b`.
Границы включительные; gaps не расширяются. Transport AND всей destination-группы;
resource OR остаётся внутри неё. Action/outbound переносятся на внешний sing-box
logical AND, inbound остаётся в destination conjunct. Policy не изменяется,
disabled правила пропускаются, порядок user/preset/default сохранён.
Новых DNS/resolve путей нет. Прежние Names/IP-only/private/platform отказы сохранены.
Дополнительная сумма диапазонов ограничена 4096, lowering одного правила ограничен
типизированными constructors (2 protocols, 16 ranges); существующие checkpoints
до каждого enabled rule и после emission сохранены.

System/TUN, source_selectors DATA validation, SystemPolicy wire и desktop inline
conversion по-прежнему отвергают enabled transport. Capability/ABI/owner/namespace
не изменены. LocalSocks executable selectors по-прежнему отвергаются, включая
их conjunct с ports. UI eligibility и ранний Connect gate различают Local/System.

Добавлены pure source tests: exact JSON keys, все protocol sets, ports 1/65535,
singletons/range/gaps, destination conjunct, logical resource OR, action/inbound,
user/preset/default order, disabled/IDs, mode/process gates, malformed transport,
native wire rejection. Это не runtime enforcement.

Проверки этого leaf: standalone rustfmt 1.85.1 (edition 2024, skip_children),
`git diff --check`. Cargo/test/clippy/ELF не запускались: root capture03 удерживает
build slot. Системный stable rustfmt не работает из-за отсутствующей
librustc_driver; использован уже установленный 1.85.1, без downloads.

Требуется root integration: существующий engine `tests/routing.rs`, тест
`transport_native_adapters_refuse_enabled_preserve_disabled`, ожидает отказ public
LocalSocks compiler. Он вне ownership leaf и оставлен неизменным. Его enabled
assertion следует перевести на System compiler (Local success покрыт новым тестом).
До этой правки полный engine suite заведомо содержит устаревшее ожидание.
Compile, новые tests и pure ELF проверки остаются pending до одного root refresh.
Cores/Host/worker/proc/network/services/VM не запускались. **Full P07/P15 NOT PASS.**
