# IPv6 и граница native DNS

Base: `565fa26bd745565dbe64a47af8d970b6e57a3073` (PR72).

Единственный исполняемый контракт остаётся `yonik_domain::DnsPolicy`.
`upstream` принимает literal SocketAddr обоих семейств; `ipv4`/`ipv6`
выбирают ровно одно семейство. Оба true отклоняются: это запрос full-family,
для которого pinned cores не обеспечивают атомарную lease (P14, networking §5).
Оба false также отклоняются. Выбор ограничивает DNS-ответы, а не literal
SOCKS назначения или покрытие трафика ОС. IPv4-mapped IPv6, scope ID и flowinfo отклоняются,
поскольку JSON не сохраняет эту адресную семантику.

`fake_exclusions` и `require_mapping_lease` — явные обязательные требования.
Непустые exclusions и строгая lease отклоняются до генерации/probe credentials.
Это сохранение failclosed границы, а не реализация exclusions или lifetime/reset.
Исключение приватной зоны требует resolver своего VPN; публичный upstream не
может заменить его. Lease требует TTL, network/resolver/engine epochs,
блокировки старых mappings и отказа при повторной выдаче занятого адреса.
Прежние allocator/partial-family контрпримеры описаны в
`docs/validation/2026-11-09-native-dns-contracts.md`; здесь они не считаются
новыми достижениями. `require_mapping_lease=false` разрешает только прежний
локальный профиль без этих гарантий. Полный G-DNS и system_tun не включены.

Новый opt-in gate `live_ipv6_family_gates` создаёт только user/net namespace
с lo. Rust генерирует JSON; Python проверяет digest обоих pinned binaries,
делает native check и реальный AAAA→SOCKS→IPv6 TCP marker. Fake-IP проверяется
без upstream lookup. Отдельно проверяются неизвестный fake destination,
старый mapping в пустой новой сессии и отключённое семейство. Пустая новая
сессия не является доказательством reset сети, TTL или безопасного reuse пула.

Воспроизведение из корня checkout с заранее созданными TASK_ROOT и EVIDENCE:

```sh
flock -s TASK_ROOT /home/myrs/projects/yonik/local/orchestration/with-rust-slot \
  env CARGO_TARGET_DIR=TASK_ROOT/target CARGO_BUILD_JOBS=1 \
  YONIK_DNS_EVIDENCE=EVIDENCE cargo +1.85.1 test -p yonik-engine \
  --test native_dns_modes live_ipv6_family_gates -- --ignored
```

Evidence вне TTL: `local/validation/dns-family-gates/` этого worktree.
`red.log` фиксирует behavioral RED UseIPv4 вместо UseIPv6; JSON и native логи,
`ipv6-results.json`, source snapshots и `sources.json` содержат точные входы.
Хеши binaries закреплены в `tools/dns_fixture.py`; отдельный manifest содержит
хеши конфигураций и файлов evidence. Host DNS/TUN, helper и evaluator не меняются.

Native RED обнаружил пропуск неизвестного IPv6 fake destination через Xray
в upstream SOCKS. Guard CIDR активного fake pool теперь блокирует literal,
который sniff не восстановил в имя. Успешно восстановленное имя продолжает
положительный путь; guard не является mapping ledger и не защищает повторную
выдачу того же адреса другому имени. Evidence: `native-red.log` и
`native-red-results.json`, повторный результат в `live.log`.

Проверенный результат: оба pinned core прошли IPv6 Real и Fake-IP
(четыре сценария), неизвестный fake destination и старый mapping пустой
новой сессии отклонены; IPv4-ответ при IPv6-only не выдан. Наблюдавшиеся
Fake DNS TTL: Xray 1 s, sing-box 600 s. Это наблюдение DNS TTL, а не
проверка expiry mapping; TTL expiry и network reset остаются открытыми.
