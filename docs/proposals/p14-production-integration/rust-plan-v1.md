# Rust DATA-план V1

Статус: foundation, полный P14 NOT PASS. База 6dc2b788c16c0183f2511fbbf8038c725fce09e4,
решение cf201ab. Публичный `dns_policy_plan::compile_plan(&Policy, &DnsPolicy,
&DesiredPlan)` возвращает закрытый `CompiledPolicyPlan`; доступ только через
`bytes()`. Нет native dispatch, config injection, регистрации профиля, выдачи
Session/Permit/lease или изменения capability. Оба stock core этот формат не понимают.

## Точные данные и кодирование

UTF-8 JSON без BOM, whitespace и завершающего LF; ключи объектов лексикографические
(serde_json::Value без preserve_order), массивы сохраняют порядок. Числа целые,
IP/SocketAddr и CIDR сериализуются стандартным Rust Display/serde, домены —
DomainName (IDNA, lower-case, без root dot). Caller хеширует **все** bytes() существующим
LinuxDigest. Хеш не добавляется в план. Desired fake pools принимаются только
в каноническом написании. Два семантически эквивалентных переставленных массива
могут иметь разные байты: это намеренно, сортировка правил запрещена.

Единственный корневой ключ `yonik_policy` имеет поля:

- `version`: 1; `kind`: `desired_data_only`.
- `desired`: все поля DesiredPlan ниже, без пропусков.
- `bounds`: `bytes=262144`, `rows=256`, `answers=256`, `parallel_lookups=2`,
  `lookup_deadline_ms=10000`. Последние три — требования будущему consumer.
- `policy`: `default_action`, `user_rules`, `preset_rules`, `lan`,
  `private_networks`, `private_domains`.
- `dns`: `mode` (`real|fake_ip`), `upstream` (literal SocketAddr),
  `family_mask` (1=A, 2=AAAA, 3=оба), `listen_port`, `fake_exclusions`
  (массив domain pattern), `require_mapping_lease`.
- `requirements`: булевы поля `full_family`, `first_any_match`, `literal_only`,
  `private_ambiguity_terminal`, `no_public_private_fallback`, `no_local_reresolve`,
  `original_monotonic_deadline`, `positive_negative_cname_min_ttl`,
  `unknown_expired_fake_reject`, `fake_before_rules`, `epoch_nonreuse`,
  `fake_address_nonreuse`, `trusted_binding_before_every_dial`,
  `revoke_closes_sockets`, `service_paths_nonrecursive` равны true;
  `established_tcp_ttl_close` равно false.

Rule: `id` u64, `enabled` bool, `action` (`direct|proxy|block`),
`source={executables:[string],platform_sources:[]}`,
`destination={domains:[pattern],cidrs:[string]}`,
`transport` — существующий TransportInput: отсутствующий protocol/port означает
неограниченность; присутствующие `transport_protocols` и `destination_ports`
передаются из типизированных TransportConditions без изменения.
Pattern: `{kind:exact|suffix,name:string}`, suffix строго по границе DNS-метки.
`lan` — CIDR strings; private network: `{cidr,vpn,available}`;
private domain: `{domain:pattern,vpn,available}`. Приоритет: private, LAN,
user_rules, preset_rules, default согласно существующему domain oracle.
Disabled rules также сохраняются и проверяются; ID уникальны между обоими списками.

DesiredPlan: `version` u32, `network_epoch`, `engine_epoch`, `resolver_epoch` u64,
`policy_transport` (`udp|tcp`), `policy_route` и `bootstrap_route` (`direct|proxy`), `bootstrap` literal SocketAddr,
`bootstrap_transport` (`udp|tcp`), `vpns`, `fake_pools` (CIDR strings),
`reserved_constraints` (только пустой массив).
DesiredVpn: `vpn` ненулевой u64, `ifname` 1–15 ASCII alnum/`_-.`,
`table` ненулевой u32, `resolver` literal SocketAddr, `transport` udp|tcp,
`excluded` только false. VPN set точно совпадает с VPN ссылками policy,
включая unavailable. Перекрывающиеся private сети/зоны разных VPN — ошибка.
Bootstrap, public upstream и VPN resolvers являются отдельными желаемыми
service paths; они не вызывают EvaluateFlow рекурсивно. Никакого DNS здесь нет.

## Границы и отказы

Совокупно ≤256 rules; отдельно ≤256 LAN/private networks/private domains,
fake exclusions, VPN descriptors, fake pools. На правило каждый массив domains,
CIDR, executable ≤256; executable абсолютный, ≤4096 bytes, без control chars.
Итог ≤262144 bytes. DesiredPlan::parse ограничивает вход тем же размером и
container depth 5, запрещает неизвестные поля, enum values и дубликаты полей
структуры. Валидация версии и семантики выполняется compile_plan.
Нельзя десериализовать CompiledPolicyPlan и обойти compiler.

Fake требует lease и хотя бы один pool соответствующего enabled family;
V1 pools ограничены 198.18.0.0/15 и fc00::/18 с более узким prefix,
без exact duplicates и canonical spelling deviations. Real не принимает pools
и mapping lease. Endpoint запрещает port=0, unspecified/multicast, IPv4-mapped
IPv6, scope/flowinfo. Platform selectors и reserved advanced constraints
возвращают Unsupported даже в disabled rules; ничего не теряется молча.

`PlanError`: Bounds, Unsupported, Invalid, Mapping, Ambiguous. Это ошибки
компиляции DATA, не доказанные DNS исходы. `available`, ifname/table и epoch
не являются witness: authority/binding должен выдавать доверенный bootstrap.
Нулевые epoch здесь допустимы как желаемые значения; JSON ничего не выдаёт.

## Общие fixtures и оставшиеся проверки

`crates/yonik-engine/tests/dns_policy_plan.rs` — общие входные Rust fixtures,
семантические mutation и negative vectors для будущих двух consumers.
Они проверяют canonical IDNA, rule order/id, transport/source/destination,
family/mode/resolver/exclusions/epochs, strict unknown/size/version handling,
VPN total mapping/exclusion/ambiguity. Это не утверждение фактического DNS outcome.

D-01…D-07 остаются runtime NOT RUN: private precedence/fail-closed,
source-before-DNS/старший winner, full RRset any-match/literal-only,
private ambiguity, per-family error/NODATA, private resolver isolation,
TTL/epoch/fake reuse. Требуются отдельные Go parser-to-instance adapters,
unknown-field/version rejection там, digest coverage sealed config,
trusted helper route witness и mapping quarantine/nonreuse implementation.
Необходимо также утвердить общие machine-readable oracle fixtures и покрыть
все поля private descriptors и все оси D на обоих cores. Source/ELF hashes
Rust unit tests этого не заменяют. ABI, production UNAVAILABLE и текущие
compile_native_dns/private guards остаются неизменными.
