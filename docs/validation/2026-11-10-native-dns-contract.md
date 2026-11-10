# Ограниченная граница DNS-контракта pinned native cores

База: `2be9a1bc8fcd553594e36e8a4c91614c15ba6251`.
Проверено 2026-11-10, Rust 1.85.1, Cargo jobs=1.

В `yonik-domain::Policy` отсутствует DnsPolicy. Существующий compiler не принимает
режим DNS, исключения Fake-IP или ограничение семейств. Добавлен отдельный вход
`compile_policy_with_dns` и проверка `DnsContract::validate`: неподдержанное
отклоняется до генерации конфигурации и создания probe credentials. Это граница
engine API; подключения DNS-настроек GUI/host данным изменением не производится.
Существующий `compile_policy` сохраняет прежний контракт внешнего DNS.

`External` означает отсутствие native DNS-конфигурации, оба семейства разрешены.
Это не обещание полноты lookup или защиты DNS внешнего приложения. `Real` и
`FakeIp` отклоняются даже без CIDR: нет подтверждённых lease/dial/mapping hooks.
Даже исключения при External отклоняются, чтобы запрос не потерялся молча.

| Случай | Xray 26.10.31 | sing-box 1.14.4 |
| --- | --- | --- |
| External, IPv4+IPv6, без исключений | Генерация без dns/fakedns | Генерация без dns/fakedns |
| Real / FakeIp, Names и IpOnly | Unsupported(DnsContract) | Unsupported(DnsContract) |
| Fake-IP исключение `.local` | Unsupported(DnsContract) | Unsupported(DnsContract) |
| Только IPv4 / только IPv6 / ни одного семейства | Unsupported(DnsContract) | Unsupported(DnsContract) |
| Недоступная private zone при IpOnly | DomainsInIpOnly | DomainsInIpOnly |
| Доступная private zone | PrivateNetworkBinding | PrivateNetworkBinding |
| IPv4+IPv6 CIDR в IpOnly | Оба CIDR сохранены | Оба CIDR сохранены |
| Native check Names с недоступной private zone | exit 0 | exit 0 |
| Native check IpOnly с обоими CIDR | exit 0 | exit 0 |

Регрессии: `crates/yonik-engine/tests/dns_contract.rs`. RED зафиксирован до
реализации: отсутствующий API и capability; GREEN — три интеграционных теста.
Затем один полный релевантный `cargo +1.85.1 test -p yonik-engine`,
`cargo +1.85.1 clippy -p yonik-engine --all-targets -- -D warnings`, fmt check.
Зависимости не добавлялись; автоматическая перестановка Cargo.lock исключена.

Native checks используют уже сохранённые локальные бинарники. SHA-256 проверены
до запуска и совпали с `tools/dns_fixture.py`:

- Xray: `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`.
- sing-box: `1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a`.

Команды: `xray run -test -c CONFIG`, `sing-box check -c CONFIG`.
Конфигурации получены настоящим `yonik-compile` из синтетического SOCKS endpoint
192.0.2.1:1080, без пользовательских credentials. Сеть ОС не менялась.
Evidence вне TTL: `local/validation/native-dns-contract/` этого worktree:
`red.log`, `green.log`, `tests.log`, `clippy.log`, `check.py`, четыре JSON и native
лога, `native-results.json`. Этот локальный каталог не включён в Git.

Native config-check подтверждает только принятие конфигурации. Live DNS/Fake-IP,
G-DNS D-01…D-07, split DNS, bootstrap, TTL/epochs, pool exhaustion и защита
приватных зон при смене сети остаются открытыми. Для полного DnsPolicy нужны
изменения domain/host вне ownership. Предыдущие native-контрпримеры описаны в
[отчёте 2026-11-09](2026-11-09-native-dns-contracts.md).
