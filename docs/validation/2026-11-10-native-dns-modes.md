# Ограниченные рабочие native DNS modes

База `dd6962e7618d688660927bc58e2694940eba5598`. Linux, без системного TUN/DNS.

Добавлен единый `yonik_domain::DnsPolicy`: Real/FakeIp, literal IPv4 UDP upstream
с прямым доступом и отдельный loopback DNS port. `compile_native_dns` принимает
его и создаёт исполняемый JSON для Xray 26.10.31 и sing-box 1.14.4.
Старый `DnsContract` сохраняет совместимость #63: он не содержит upstream/listener,
поэтому Real/FakeIp через старый API по-прежнему отвергаются. Это legacy capability
вход, а не второй источник настроек исполняемого resolver.

Поддержанный срез: только публичная default Proxy политика, literal адрес прокси,
Names, IPv4. Native DNS cache выключен, persistent Fake-IP cache не включается.
IpOnly, пользовательские/preset правила, LAN/private networks/domains, доменный
bootstrap и рекурсивный upstream отклоняются до выдачи конфигурации. Новая модель
намеренно не предоставляет настройки TTL, epochs, full-family, private exclusions:
запрос таких гарантий требует расширения контракта и доказательства возможностей.

## Native доказательство

Четыре JSON получены настоящим `compile_native_dns`, без правок fixture после
генерации. Каждый прошёл native config-check и отдельный live-запуск в unprivileged
user/network namespace с единственным `lo`. `/etc/resolv.conf`, DNS ОС, keyring,
firewall и интерфейсы хоста не менялись. Fixture использует только loopback.

| Ядро | Real A | Fake-IP A | SOCKS назначение после Fake-IP |
|---|---|---|---|
| Xray 26.10.31 | 192.0.2.20 | 198.18.0.0/15 | public.test:24081 |
| sing-box 1.14.4 | 192.0.2.20 | 198.18.0.0/15 | public.test:24081 |

В Real fixture наблюдает UDP A запрос к заданному upstream; в Fake-IP upstream
не получает запроса. Запрос TCP к выданному IP через локальный SOCKS ядра достигает
локального SOCKS proxy fixture, затем отдельного TCP endpoint; проверяется маркер
`LOCAL_ENDPOINT`. В Real SOCKS получает реальный IP, в Fake-IP восстановленное имя.
Это проверка mapping в пределах одного запуска, не смены engine-session.

Содержательный native RED: Xray принимал JSON, но DNS timeout выявил маршрутизацию
upstream через общий Proxy. GREEN после отдельного tag/direct маршрута для resolver.
Для sing-box Fake-IP требуется DNS rule для A, реальный default server и явный
`route.default_domain_resolver`; deprecated environment overrides не используются.

Воспроизведение из корня checkout (каталог evidence заранее создан):

```sh
flock -s TASK_ROOT /home/myrs/projects/yonik/local/orchestration/with-rust-slot \
  env CARGO_TARGET_DIR=TASK_ROOT/target CARGO_BUILD_JOBS=1 \
  YONIK_DNS_EVIDENCE=ABSOLUTE_EVIDENCE_DIRECTORY \
  cargo +1.85.1 test -p yonik-engine --test native_dns_modes \
  live_dns_and_fake_mapping -- --ignored
```

Opt-in тест включает Python fixture непосредственно в `native_dns_modes.rs`,
использует существующий `tools/dns_fixture.py`, проверяет SHA-256 pinned binaries.
Бинарники ожидаются в `/home/myrs/projects/yonik/local/engines/`.

Evidence вне TTL: `local/validation/native-dns-modes/` этого worktree:
JSON, native логи, `native-red.log`, `live-results.json`, `tests.log`,
`live-test.log`, `clippy.log`, `fmt.log`, снимки pinned source и их SHA-256.

Первичные источники:
[Xray v26.10.31 DNS config](https://github.com/XTLS/Xray-core/blob/v26.10.31/infra/conf/dns.go),
[sing-box v1.14.4 DNS options](https://github.com/SagerNet/sing-box/blob/v1.14.4/option/dns.go),
[Xray DNS](https://xtls.github.io/en/config/dns.html),
[sing-box FakeIP](https://sing-box.sagernet.org/configuration/dns/server/fakeip/).
Бинарные digests соответствуют `tools/dns_fixture.py`.

## Открытый gate

**G-DNS/P14 не закрыт.** Не доказаны D01…D07 целиком, IPv6/TCP DNS/UDP traffic,
TTL/epochs/provenance leases, смена сети/ядра, устаревшие mappings, pool exhaustion,
private/LAN exclusions, full-family CIDR и systemd-resolved host broker. Этот API
не подключён к desktop/helper; `system_tun` остаётся false. Профиль предназначен
для явно ограниченного локального использования, а не managed TUN activation.

Итог: Rust 1.85.1 tests domain+engine PASS; opt-in live gate PASS (4 сценария);
clippy обоих пакетов --all-targets -- -D warnings PASS; fmt check PASS.
Rust 1.97 clippy выявляет прежний collapsible_match в lib.rs:165 (credentials);
этот участок не менялся. Автоматические перестановки Cargo.lock исключены.
