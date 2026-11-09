# Общая политика и компиляторы маршрутизации

Основание: FR-08/11/14/15/20/21, networking.md §2/§5, D-01…D-07.

1. Добавить нормализованные IDNA domain exact/suffix, destination OR
   (domain/CIDR) и source AND. Сохранить единый порядок private → LAN → user →
   presets → default; executable проверяется до запроса DNS обычного правила.
2. Расширить portable evaluator: literal IP отдельно от имени и fake mapping;
   NeedResolution с заданным resolver path, Unresolved/Ambiguous, complete/TTL/
   generation ответов. Защищённые VPN имеют opaque ID. Смешанные private/public
   ответы, разные VPN, неизвестный/stale fake mapping не дают публичного пути.
3. Нативные компиляторы получают явный режим ingress. Names: domain/default
   без CIDR/LAN/private networks; IpOnly: CIDR/default с запретом доменных SOCKS
   запросов. Недоказанные DNS/process/private-binding комбинации дают capability
   error, а не меняют смысл правил. Неактивные правила сохраняются без эффекта.
4. Domain rules нормализуются одинаково, native ingress guard блокирует raw
   Unicode/невалидные/неполные DNS-имена, чтобы не провалиться в default.
   Xray IPOnDemand не используется как замена общего fail-closed контракта.
5. Добавить ограниченный JSON policy input и подключить к tofu-compile/tofu-local;
   HTTPS probe через выбранный outbound не должен попасть в DIRECT/BLOCK policy.
   Сначала тесты D-cases, затем код. Настоящие ядра проверят rule order,
   exact/suffix boundaries, IP4/6, protected unavailable, user/preset/default,
   отрицательные proxy/direct/block контроли и native DNS failure gap.
6. fmt/clippy/tests/MSRV/release/Android, README RU/EN, независимый review,
   исправления, атомарные коммиты и PR. Сеть хоста не менять.

Полный G-DNS не подменяется этими двумя режимами. В pinned Xray native DNS
context при ошибке продолжает младшие правила; полный mixed domain/CIDR
контракт потребует отдельного подтверждённого adapter/architecture решения.
Доступный private VPN не превращается в DIRECT без NetworkHost binding.
