//! Типизированная публичная блокировка. Проверка принадлежности интерфейсов — задача адаптера.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    net::IpAddr,
    num::{NonZeroU16, NonZeroU32},
};

const MAX_EXCEPTIONS: usize = 128;
const OUTPUT_CHAIN: &str = "output";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    Prefix,
    Interface,
    Port,
    Mark,
    Limit,
    ConflictingPrivateRoute,
    PrivateTransport,
}
impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Prefix => "Нужен канонический CIDR с ненулевой длиной префикса",
            Self::Interface => "Индекс интерфейса должен быть ненулевым",
            Self::Port => "Порт должен быть ненулевым",
            Self::Mark => "Метка сокета должна быть ненулевой",
            Self::Limit => "Превышен лимит 128 исключений",
            Self::PrivateTransport => "Служебное назначение пересекает защищённую приватную сеть",
            Self::ConflictingPrivateRoute => "Один приватный префикс связан с разными VPN",
        })
    }
}
impl std::error::Error for PolicyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "IpPrefixDto")]
pub struct IpPrefix {
    address: IpAddr,
    length: u8,
}
impl IpPrefix {
    pub fn new(address: IpAddr, length: u8) -> Result<Self, PolicyError> {
        let canonical = match address {
            IpAddr::V4(address) if (1..=32).contains(&length) => {
                length == 32 || u32::from(address) & (u32::MAX >> length) == 0
            }
            IpAddr::V6(address) if (1..=128).contains(&length) => {
                // Сдвиг на полную ширину запрещён даже для host-префиксов.
                length == 128 || u128::from(address) & (u128::MAX >> length) == 0
            }
            _ => false,
        };
        if !canonical {
            return Err(PolicyError::Prefix);
        }
        Ok(Self { address, length })
    }
    fn contains(self, address: IpAddr) -> bool {
        match (self.address, address) {
            (IpAddr::V4(prefix), IpAddr::V4(address)) => {
                u32::from(prefix) == u32::from(address) & (u32::MAX << (32 - self.length))
            }
            (IpAddr::V6(prefix), IpAddr::V6(address)) => {
                u128::from(prefix) == u128::from(address) & (u128::MAX << (128 - self.length))
            }
            _ => false,
        }
    }
    fn expression(self) -> Value {
        json!({"prefix": {"addr": self.address.to_string(), "len": self.length}})
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "PrivateRouteDto")]
pub struct PrivateRoute {
    prefix: IpPrefix,
    vpn_ifindex: Option<NonZeroU32>,
}
impl PrivateRoute {
    pub fn new(prefix: IpPrefix, vpn_ifindex: Option<u32>) -> Result<Self, PolicyError> {
        Ok(Self {
            prefix,
            vpn_ifindex: vpn_ifindex.map(interface).transpose()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "LanExceptionDto")]
pub struct LanException {
    prefix: IpPrefix,
    ifindex: NonZeroU32,
}
impl LanException {
    pub fn new(prefix: IpPrefix, ifindex: u32) -> Result<Self, PolicyError> {
        Ok(Self {
            prefix,
            ifindex: interface(ifindex)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TransportProtocol {
    Tcp,
    Udp,
}
impl TransportProtocol {
    fn name(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "BoundTransportDto")]
pub struct BoundTransport {
    destination: IpAddr,
    protocol: TransportProtocol,
    port: NonZeroU16,
    ifindex: NonZeroU32,
    uid: u32,
    mark: NonZeroU32,
}
impl BoundTransport {
    pub fn new(
        destination: IpAddr,
        protocol: TransportProtocol,
        port: u16,
        ifindex: u32,
        uid: u32,
        mark: u32,
    ) -> Result<Self, PolicyError> {
        Ok(Self {
            destination,
            protocol,
            port: NonZeroU16::new(port).ok_or(PolicyError::Port)?,
            ifindex: interface(ifindex)?,
            uid,
            mark: NonZeroU32::new(mark).ok_or(PolicyError::Mark)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PublicBlockPolicyDto")]
pub struct PublicBlockPolicy {
    loopback_ifindex: NonZeroU32,
    tun_ifindex: Option<NonZeroU32>,
    private_routes: Vec<PrivateRoute>,
    lan: Vec<LanException>,
    transport: Vec<BoundTransport>,
    bootstrap_dns: Vec<BoundTransport>,
    link_control: Vec<NonZeroU32>,
}
impl PublicBlockPolicy {
    pub fn new(
        loopback_ifindex: u32,
        tun_ifindex: Option<u32>,
        mut private_routes: Vec<PrivateRoute>,
        mut lan: Vec<LanException>,
        mut transport: Vec<BoundTransport>,
        mut bootstrap_dns: Vec<BoundTransport>,
    ) -> Result<Self, PolicyError> {
        // Проверяем до dedup: дубликаты не обходят ограничение входных данных.
        if private_routes
            .len()
            .saturating_add(lan.len())
            .saturating_add(transport.len())
            .saturating_add(bootstrap_dns.len())
            > MAX_EXCEPTIONS
        {
            return Err(PolicyError::Limit);
        }
        if transport.iter().chain(&bootstrap_dns).any(|endpoint| {
            private_routes
                .iter()
                .any(|route| route.prefix.contains(endpoint.destination))
        }) {
            return Err(PolicyError::PrivateTransport);
        }
        private_routes.sort_unstable();
        if private_routes.windows(2).any(|pair| {
            pair[0].prefix == pair[1].prefix && pair[0].vpn_ifindex != pair[1].vpn_ifindex
        }) {
            return Err(PolicyError::ConflictingPrivateRoute);
        }
        private_routes.dedup();
        // Более специфичная приватная сеть определяет свой VPN раньше широкой.
        private_routes
            .sort_unstable_by_key(|route| (std::cmp::Reverse(route.prefix.length), *route));
        lan.sort_unstable();
        lan.dedup();
        transport.sort_unstable();
        transport.dedup();
        bootstrap_dns.sort_unstable();
        bootstrap_dns.dedup();
        Ok(Self {
            loopback_ifindex: interface(loopback_ifindex)?,
            tun_ifindex: tun_ifindex.map(interface).transpose()?,
            private_routes,
            lan,
            transport,
            bootstrap_dns,
            link_control: Vec::new(),
        })
    }

    /// Разрешает только служебные IPv6 RS/NS/NA с hoplimit=255 и code=0
    /// на явно выбранных интерфейсах; это не широкое разрешение ICMPv6.
    pub fn with_link_control(mut self, interfaces: Vec<u32>) -> Result<Self, PolicyError> {
        let used = self.private_routes.len()
            + self.lan.len()
            + self.transport.len()
            + self.bootstrap_dns.len();
        if interfaces.len() > MAX_EXCEPTIONS.saturating_sub(used) / 3 {
            return Err(PolicyError::Limit);
        }
        self.link_control = interfaces
            .into_iter()
            .map(interface)
            .collect::<Result<_, _>>()?;
        self.link_control.sort_unstable();
        self.link_control.dedup();
        Ok(self)
    }

    /// Отпечаток канонических правил не зависит от имени принадлежащей helper таблицы.
    pub fn digest(&self) -> [u8; 32] {
        // Value сериализуется однозначно; здесь не бывает пользовательских map/чисел float.
        Sha256::digest(Value::Array(self.commands("policy")).to_string().as_bytes()).into()
    }

    /// Только chain/rules; таблицу и её owner-comment создаёт executor.
    pub(super) fn commands(&self, table_name: &str) -> Vec<Value> {
        let mut commands = vec![
            // Фильтруем после обычного DNAT (-100): LAN-адрес не должен
            // превратиться в разрешённый публичный выход после нашего accept.
            json!({"add": {"chain": {"family": "inet", "table": table_name, "name": OUTPUT_CHAIN, "type": "filter", "hook": "output", "prio": 0, "policy": "drop"}}}),
        ];
        commands.push(rule(
            table_name,
            vec![
                meta("oif", json!(self.loopback_ifindex.get())),
                json!({"accept": null}),
            ],
        ));
        for ifindex in &self.link_control {
            for kind in [133, 135, 136] {
                commands.push(rule(
                    table_name,
                    vec![
                        meta("oif", json!(ifindex.get())),
                        compare(
                            json!({"payload":{"protocol":"ip6","field":"hoplimit"}}),
                            json!(255),
                        ),
                        compare(
                            json!({"payload":{"protocol":"icmpv6","field":"type"}}),
                            json!(kind),
                        ),
                        compare(
                            json!({"payload":{"protocol":"icmpv6","field":"code"}}),
                            json!(0),
                        ),
                        json!({"accept":null}),
                    ],
                ));
            }
        }
        for endpoint in self.transport.iter().chain(&self.bootstrap_dns) {
            let protocol = endpoint.protocol.name();
            commands.push(rule(
                table_name,
                vec![
                    destination(
                        endpoint.destination,
                        json!(endpoint.destination.to_string()),
                    ),
                    meta("oif", json!(endpoint.ifindex.get())),
                    meta("l4proto", json!(protocol)),
                    compare(
                        json!({"payload": {"protocol": protocol, "field": "dport"}}),
                        json!(endpoint.port.get()),
                    ),
                    meta("skuid", json!(endpoint.uid)),
                    meta("mark", json!(endpoint.mark.get())),
                    json!({"accept": null}),
                ],
            ));
        }
        for route in &self.private_routes {
            if let Some(ifindex) = route.vpn_ifindex {
                commands.push(rule(
                    table_name,
                    vec![
                        destination(route.prefix.address, route.prefix.expression()),
                        meta("oif", json!(ifindex.get())),
                        json!({"accept": null}),
                    ],
                ));
            }
            // Терминальная блокировка не даёт широкой LAN/TUN обойти назначенный VPN.
            commands.push(rule(
                table_name,
                vec![
                    destination(route.prefix.address, route.prefix.expression()),
                    json!({"drop": null}),
                ],
            ));
        }
        for exception in &self.lan {
            commands.push(rule(
                table_name,
                vec![
                    destination(exception.prefix.address, exception.prefix.expression()),
                    meta("oif", json!(exception.ifindex.get())),
                    json!({"accept": null}),
                ],
            ));
        }
        if let Some(ifindex) = self.tun_ifindex {
            commands.push(rule(
                table_name,
                vec![meta("oif", json!(ifindex.get())), json!({"accept": null})],
            ));
        }
        commands
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IpPrefixDto {
    address: IpAddr,
    length: u8,
}
impl TryFrom<IpPrefixDto> for IpPrefix {
    type Error = PolicyError;
    fn try_from(value: IpPrefixDto) -> Result<Self, Self::Error> {
        Self::new(value.address, value.length)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateRouteDto {
    prefix: IpPrefix,
    vpn_ifindex: Option<u32>,
}
impl TryFrom<PrivateRouteDto> for PrivateRoute {
    type Error = PolicyError;
    fn try_from(value: PrivateRouteDto) -> Result<Self, Self::Error> {
        Self::new(value.prefix, value.vpn_ifindex)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanExceptionDto {
    prefix: IpPrefix,
    ifindex: u32,
}
impl TryFrom<LanExceptionDto> for LanException {
    type Error = PolicyError;
    fn try_from(value: LanExceptionDto) -> Result<Self, Self::Error> {
        Self::new(value.prefix, value.ifindex)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundTransportDto {
    destination: IpAddr,
    protocol: TransportProtocol,
    port: u16,
    ifindex: u32,
    uid: u32,
    mark: u32,
}
impl TryFrom<BoundTransportDto> for BoundTransport {
    type Error = PolicyError;
    fn try_from(value: BoundTransportDto) -> Result<Self, Self::Error> {
        Self::new(
            value.destination,
            value.protocol,
            value.port,
            value.ifindex,
            value.uid,
            value.mark,
        )
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicBlockPolicyDto {
    loopback_ifindex: u32,
    tun_ifindex: Option<u32>,
    private_routes: Vec<PrivateRoute>,
    lan: Vec<LanException>,
    transport: Vec<BoundTransport>,
    bootstrap_dns: Vec<BoundTransport>,
    link_control: Vec<u32>,
}
impl TryFrom<PublicBlockPolicyDto> for PublicBlockPolicy {
    type Error = PolicyError;
    fn try_from(value: PublicBlockPolicyDto) -> Result<Self, Self::Error> {
        Self::new(
            value.loopback_ifindex,
            value.tun_ifindex,
            value.private_routes,
            value.lan,
            value.transport,
            value.bootstrap_dns,
        )?
        .with_link_control(value.link_control)
    }
}

fn interface(value: u32) -> Result<NonZeroU32, PolicyError> {
    NonZeroU32::new(value).ok_or(PolicyError::Interface)
}
fn compare(left: Value, right: Value) -> Value {
    json!({"match": {"op": "==", "left": left, "right": right}})
}
fn meta(key: &str, right: Value) -> Value {
    compare(json!({"meta": {"key": key}}), right)
}
fn destination(address: IpAddr, right: Value) -> Value {
    compare(
        json!({"payload": {"protocol": if address.is_ipv4() { "ip" } else { "ip6" }, "field": "daddr"}}),
        right,
    )
}
fn rule(table: &str, mut expr: Vec<Value>) -> Value {
    let family = match expr
        .first()
        .and_then(|value| value.pointer("/match/left/payload/protocol"))
        .and_then(Value::as_str)
    {
        Some("ip") => Some("ipv4"),
        Some("ip6") => Some("ipv6"),
        _ => None,
    };
    if let Some(family) = family {
        expr.insert(0, meta("nfproto", json!(family)));
    }
    json!({"add": {"rule": {"family": "inet", "table": table, "chain": OUTPUT_CHAIN, "expr": expr}}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefix(address: &str, length: u8) -> IpPrefix {
        IpPrefix::new(address.parse().expect("Тестовый IP"), length).expect("Тестовый CIDR")
    }
    fn policy(
        private: Vec<PrivateRoute>,
        lan: Vec<LanException>,
        transport: Vec<BoundTransport>,
    ) -> PublicBlockPolicy {
        PublicBlockPolicy::new(1, Some(8), private, lan, transport, vec![])
            .expect("Тестовая политика")
    }
    fn endpoint(address: &str) -> BoundTransport {
        BoundTransport::new(
            address.parse().expect("Тестовый IP"),
            TransportProtocol::Tcp,
            443,
            2,
            987,
            11,
        )
        .expect("Тестовый endpoint")
    }
    #[test]
    fn policy_codec_validates_every_nested_constructor_and_unknown_field() {
        let original = policy(
            vec![PrivateRoute::new(prefix("10.0.0.0", 8), Some(7)).unwrap()],
            vec![LanException::new(prefix("192.168.0.0", 16), 2).unwrap()],
            vec![endpoint("192.0.2.1")],
        )
        .with_link_control(vec![2])
        .unwrap();
        let value = serde_json::to_value(&original).unwrap();
        assert_eq!(
            serde_json::from_value::<PublicBlockPolicy>(value.clone()).unwrap(),
            original
        );
        for pointer in [
            "/loopback_ifindex",
            "/tun_ifindex",
            "/private_routes/0/vpn_ifindex",
            "/lan/0/ifindex",
            "/transport/0/port",
            "/transport/0/ifindex",
            "/transport/0/mark",
            "/link_control/0",
        ] {
            let mut invalid = value.clone();
            *invalid.pointer_mut(pointer).unwrap() = 0.into();
            assert!(
                serde_json::from_value::<PublicBlockPolicy>(invalid).is_err(),
                "{pointer}"
            );
        }
        for pointer in [
            "",
            "/private_routes/0",
            "/private_routes/0/prefix",
            "/lan/0",
            "/transport/0",
        ] {
            let mut invalid = value.clone();
            invalid
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unknown".into(), true.into());
            assert!(
                serde_json::from_value::<PublicBlockPolicy>(invalid).is_err(),
                "{pointer}"
            );
        }
        let mut invalid = value.clone();
        invalid["private_routes"][0]["prefix"]["address"] = "10.0.0.1".into();
        assert!(serde_json::from_value::<PublicBlockPolicy>(invalid).is_err());
        let mut invalid = value.clone();
        invalid["transport"][0]["destination"] = "10.1.2.3".into();
        assert!(serde_json::from_value::<PublicBlockPolicy>(invalid).is_err());
        let mut invalid = value;
        invalid["transport"] = serde_json::Value::Array(vec![invalid["transport"][0].clone(); 129]);
        assert!(serde_json::from_value::<PublicBlockPolicy>(invalid).is_err());
    }
    #[test]
    fn rejects_noncanonical_and_default_prefixes_and_accepts_host_prefixes() {
        for (address, length) in [
            ("10.0.0.1", 24),
            ("10.0.0.0", 0),
            ("10.0.0.0", 33),
            ("fd00::1", 64),
            ("::", 0),
            ("::", 129),
        ] {
            assert_eq!(
                IpPrefix::new(address.parse().expect("Тестовый IP"), length),
                Err(PolicyError::Prefix)
            );
        }
        assert!(IpPrefix::new("192.0.2.1".parse().expect("IP"), 32).is_ok());
        assert!(IpPrefix::new("2001:db8::1".parse().expect("IP"), 128).is_ok());
    }
    #[test]
    fn rejects_zero_interface_port_mark_and_excessive_exceptions() {
        assert_eq!(
            LanException::new(prefix("10.0.0.0", 24), 0),
            Err(PolicyError::Interface)
        );
        assert_eq!(
            PrivateRoute::new(prefix("fd00::", 64), Some(0)),
            Err(PolicyError::Interface)
        );
        assert_eq!(
            PublicBlockPolicy::new(0, None, vec![], vec![], vec![], vec![]),
            Err(PolicyError::Interface)
        );
        assert_eq!(
            PublicBlockPolicy::new(1, Some(0), vec![], vec![], vec![], vec![]),
            Err(PolicyError::Interface)
        );
        let address = "192.0.2.1".parse().expect("IP");
        assert_eq!(
            BoundTransport::new(address, TransportProtocol::Udp, 0, 2, 5, 1),
            Err(PolicyError::Port)
        );
        assert_eq!(
            BoundTransport::new(address, TransportProtocol::Udp, 53, 2, 5, 0),
            Err(PolicyError::Mark)
        );
        assert_eq!(
            PublicBlockPolicy::new(
                1,
                None,
                vec![],
                vec![],
                vec![endpoint("192.0.2.1"); 129],
                vec![]
            ),
            Err(PolicyError::Limit)
        );
    }
    #[test]
    fn private_destinations_cannot_bypass_vpn_through_service_exception() {
        for (cidr, length, destination) in [("10.0.0.0", 8, "10.2.3.4"), ("fd00::", 8, "fd12::1")] {
            let private = vec![PrivateRoute::new(prefix(cidr, length), Some(7)).expect("VPN")];
            assert_eq!(
                PublicBlockPolicy::new(
                    1,
                    None,
                    private.clone(),
                    vec![],
                    vec![endpoint(destination)],
                    vec![]
                ),
                Err(PolicyError::PrivateTransport)
            );
            assert_eq!(
                PublicBlockPolicy::new(
                    1,
                    None,
                    private,
                    vec![],
                    vec![],
                    vec![endpoint(destination)]
                ),
                Err(PolicyError::PrivateTransport)
            );
        }
    }
    #[test]
    fn private_guard_precedes_lan_and_tun_and_uses_longest_prefix() {
        let rules = policy(
            vec![
                PrivateRoute::new(prefix("10.0.0.0", 8), Some(7)).expect("VPN"),
                PrivateRoute::new(prefix("10.1.0.0", 16), None).expect("VPN"),
            ],
            vec![LanException::new(prefix("10.0.0.0", 8), 2).expect("LAN")],
            vec![],
        )
        .commands("owned");
        assert_eq!(rules[0]["add"]["chain"]["policy"], "drop");
        assert_eq!(rules[0]["add"]["chain"]["family"], "inet");
        assert_eq!(rules[0]["add"]["chain"]["prio"], 0);
        assert_eq!(
            rules[2]["add"]["rule"]["expr"][1]["match"]["right"],
            json!({"prefix": {"addr":"10.1.0.0", "len":16}})
        );
        assert_eq!(rules[2]["add"]["rule"]["expr"][2], json!({"drop":null}));
        assert_eq!(rules[3]["add"]["rule"]["expr"][2]["match"]["right"], 7);
        assert_eq!(rules[4]["add"]["rule"]["expr"][2], json!({"drop":null}));
        assert_eq!(rules[5]["add"]["rule"]["expr"][2]["match"]["right"], 2);
        assert_eq!(rules[6]["add"]["rule"]["expr"][0]["match"]["right"], 8);
    }
    #[test]
    fn endpoint_has_complete_identity_and_ipv6_destination() {
        let rules = policy(vec![], vec![], vec![endpoint("2001:db8::1")]).commands("owned");
        let expr = &rules[2]["add"]["rule"]["expr"];
        assert_eq!(expr[0]["match"]["right"], "ipv6");
        assert_eq!(expr[1]["match"]["left"]["payload"]["protocol"], "ip6");
        assert_eq!(expr[2]["match"]["right"], 2);
        assert_eq!(expr[3]["match"]["right"], "tcp");
        assert_eq!(expr[4]["match"]["right"], 443);
        assert_eq!(expr[5]["match"]["left"]["meta"]["key"], "skuid");
        assert_eq!(expr[5]["match"]["right"], 987);
        assert_eq!(expr[6]["match"]["left"]["meta"]["key"], "mark");
        assert_eq!(expr[6]["match"]["right"], 11);
        assert_eq!(expr[7], json!({"accept":null}));
    }
    #[test]
    fn canonical_order_and_digest_ignore_duplicate_input_and_bind_identity() {
        let a = endpoint("192.0.2.1");
        let b = endpoint("192.0.2.2");
        let first = policy(vec![], vec![], vec![a, b]);
        let second = policy(vec![], vec![], vec![b, a, a]);
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.commands("a"), second.commands("a"));
        assert_ne!(first.digest(), policy(vec![], vec![], vec![a]).digest());
        assert_ne!(
            first.digest(),
            PublicBlockPolicy::new(1, Some(9), vec![], vec![], vec![a, b], vec![])
                .expect("Policy")
                .digest()
        );
        assert_ne!(first.commands("a"), first.commands("b"));
    }
    #[test]
    fn rejects_ambiguous_private_prefix_binding() {
        let cidr = prefix("fd00::", 8);
        assert_eq!(
            PublicBlockPolicy::new(
                1,
                None,
                vec![
                    PrivateRoute::new(cidr, Some(7)).expect("VPN"),
                    PrivateRoute::new(cidr, None).expect("VPN")
                ],
                vec![],
                vec![],
                vec![]
            ),
            Err(PolicyError::ConflictingPrivateRoute)
        );
    }
    #[test]
    fn link_control_is_explicit_bounded_and_not_generic_icmp() {
        let base = PublicBlockPolicy::new(1, None, vec![], vec![], vec![], vec![]).unwrap();
        let nd = base.clone().with_link_control(vec![3, 2, 3]).unwrap();
        assert_ne!(nd.digest(), base.digest());
        assert_eq!(
            nd.digest(),
            base.clone().with_link_control(vec![2, 3]).unwrap().digest()
        );
        assert_eq!(
            base.clone().with_link_control(vec![0]),
            Err(PolicyError::Interface)
        );
        assert_eq!(base.with_link_control(vec![2; 43]), Err(PolicyError::Limit));
        let rules = nd.commands("fixture");
        assert_eq!(rules.len(), 8);
        for command in &rules[2..] {
            let expr = command["add"]["rule"]["expr"].as_array().unwrap();
            assert_eq!(expr[1]["match"]["right"], 255);
            assert!([133, 135, 136].contains(&expr[2]["match"]["right"].as_u64().unwrap()));
            assert_eq!(expr[3]["match"]["right"], 0);
        }
    }
}
