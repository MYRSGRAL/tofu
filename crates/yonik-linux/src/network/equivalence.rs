//! Эквивалентность сетевого контракта без packet counters и служебных timestamps.
//! Не обновляет поколения и не заменяет guards потери Netlink/DNS/manager events.
use super::{KernelRoute, LinuxAddress, LinuxLink, LinuxRoute, LinuxTopology, NetworkAttribute};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum Context {
    Link,
    Address,
    Route,
    Families,
    Inet6,
}

macro_rules! equal_record {
    ($ty:ident, $left:ident, $right:ident, $attrs:ident, $context:expr; $($field:ident),+ $(,)?) => {{
        // Новый типизированный selector должен ломать сборку до явного включения.
        let $ty { $($field: _),+, $attrs: _ } = $left;
        true $(&& $left.$field == $right.$field)+
            && attributes_equal(&$left.$attrs, &$right.$attrs, $context)
    }};
}
pub(crate) fn same_external_topology(left: &LinuxTopology, right: &LinuxTopology) -> bool {
    let LinuxTopology {
        namespace: _,
        links: _,
        addresses: _,
        routes: _,
        rules: _,
        nexthops: _,
    } = left;
    left.namespace == right.namespace
        && left.rules == right.rules
        && left.nexthops == right.nexthops
        && left.links.len() == right.links.len()
        && left
            .links
            .iter()
            .zip(&right.links)
            .all(|(a, b)| same_link(a, b))
        && left.addresses.len() == right.addresses.len()
        && left
            .addresses
            .iter()
            .zip(&right.addresses)
            .all(|(a, b)| same_address(a, b))
        && left.routes.len() == right.routes.len()
        && left
            .routes
            .iter()
            .zip(&right.routes)
            .all(|(a, b)| same_route(a, b))
}
fn same_link(left: &LinuxLink, right: &LinuxLink) -> bool {
    equal_record!(LinuxLink,left,right,unknown,Context::Link;index,link_type,flags,name,alias,mtu,master,parent,kind,operational_state)
}
fn same_address(left: &LinuxAddress, right: &LinuxAddress) -> bool {
    equal_record!(LinuxAddress,left,right,unknown,Context::Address;family,index,address,local,prefix_len,scope,flags)
}
fn same_route(left: &LinuxRoute, right: &LinuxRoute) -> bool {
    equal_record!(LinuxRoute,left,right,unknown,Context::Route;family,destination,source,table,preferred_source,gateway,output_interface,input_interface,priority,protocol,kind,scope,tos,flags,multipath,nexthop_id)
}
pub(crate) fn same_kernel_route(left: &KernelRoute, right: &KernelRoute) -> bool {
    equal_record!(KernelRoute,left,right,attributes,Context::Route;namespace,query,table,output_interface,gateway,preferred_source,kind,flags,nexthop_id,incomplete)
}
fn attributes_equal(
    left: &[NetworkAttribute],
    right: &[NetworkAttribute],
    context: Context,
) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut seen = BTreeSet::new();
    left.iter().zip(right).all(|(a, b)| {
        a.kind == b.kind && seen.insert(a.kind) && value_equal(a.kind, &a.value, &b.value, context)
    })
}
fn value_equal(kind: u16, left: &[u8], right: &[u8], context: Context) -> bool {
    match (context, kind) {
        // rtnl_link_stats[64] содержат только счётчики. ABI длина должна совпасть.
        (Context::Link, 7) => left.len() == 96 && right.len() == 96,
        (Context::Link, 23) => {
            left.len() == right.len() && (192..=256).contains(&left.len()) && left.len() % 8 == 0
        }
        (Context::Link, 26 | 0x801a) => nested_equal(left, right, Context::Families),
        (Context::Families, 10 | 0x800a) => nested_equal(left, right, Context::Inet6),
        // Первый u64 сообщает число IPv6 counters; он сам не packet counter.
        (Context::Inet6, 3 | 6) => {
            left.len() == right.len()
                && (8..=2048).contains(&left.len())
                && left.len() % 8 == 0
                && left[..8] == right[..8]
        }
        // ifla_cacheinfo: max_reasm_len, timestamp, reachable_time, retrans_time.
        (Context::Inet6, 5) => {
            left.len() == 16
                && right.len() == 16
                && left[..4] == right[..4]
                && left[8..] == right[8..]
        }
        // ifa_cacheinfo: preferred/valid lifetimes остаются частью binding.
        (Context::Address, 6) => left.len() == 16 && right.len() == 16 && left[..8] == right[..8],
        // rta_cacheinfo: сохраняем expires и error, не сравниваем ref/use/clock.
        (Context::Route, 12) => {
            left.len() == 32
                && right.len() == 32
                && left[8..16] == right[8..16]
                && left[20..24] == right[20..24]
        }
        _ => left == right,
    }
}
fn next_attribute<'a>(rest: &mut &'a [u8]) -> Result<Option<(u16, &'a [u8])>, ()> {
    if rest.is_empty() {
        return Ok(None);
    }
    if rest.len() < 4 {
        return Err(());
    }
    let length = usize::from(u16::from_ne_bytes([rest[0], rest[1]]));
    let aligned = (length + 3) & !3;
    if length < 4 || aligned > rest.len() {
        return Err(());
    }
    let kind = u16::from_ne_bytes([rest[2], rest[3]]);
    let value = &rest[4..length];
    *rest = &rest[aligned..];
    Ok(Some((kind, value)))
}
fn nested_equal(mut left: &[u8], mut right: &[u8], context: Context) -> bool {
    let mut seen = BTreeSet::new();
    loop {
        match (next_attribute(&mut left), next_attribute(&mut right)) {
            (Ok(None), Ok(None)) => return true,
            (Ok(Some((kind, a))), Ok(Some((other, b))))
                if kind == other && seen.len() < 128 && seen.insert(kind) =>
            {
                if !value_equal(kind, a, b, context) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NetworkNamespace, RouteKind, RouteQuery};
    use std::num::NonZeroU32;
    fn link() -> LinuxLink {
        LinuxLink {
            index: 2,
            link_type: 1,
            flags: 0x1001,
            name: b"eth0".to_vec(),
            alias: None,
            mtu: Some(1500),
            master: None,
            parent: None,
            kind: None,
            operational_state: Some(6),
            unknown: vec![NetworkAttribute {
                kind: 23,
                value: vec![0; 200],
            }],
        }
    }
    fn nested(kind: u16, value: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&((value.len() + 4) as u16).to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(value);
        bytes.resize((bytes.len() + 3) & !3, 0);
        bytes
    }
    #[test]
    fn packet_counters_change_without_refreshing_network_selectors() {
        let expected = link();
        let mut current = expected.clone();
        current.unknown[0].value.fill(2);
        assert!(same_link(&expected, &current));
        current.mtu = Some(1400);
        assert!(!same_link(&expected, &current));
        current.mtu = expected.mtu;
        current.flags ^= 0x10000;
        assert!(!same_link(&expected, &current));
        current.flags = expected.flags;
        current.unknown.push(NetworkAttribute {
            kind: 999,
            value: vec![1],
        });
        assert!(!same_link(&expected, &current));
        current = expected.clone();
        current.unknown[0].value.pop();
        assert!(!same_link(&expected, &current));
    }
    #[test]
    fn ipv6_counters_do_not_hide_devconf_token_or_timers() {
        let stats = [2u64.to_ne_bytes(), 3u64.to_ne_bytes()].concat();
        let conf = nested(2, &[1u8; 16]);
        let token = nested(7, &[2u8; 16]);
        let mut payload = conf.clone();
        payload.extend(nested(3, &stats));
        payload.extend(token.clone());
        let expected = NetworkAttribute {
            kind: 26,
            value: nested(10, &payload),
        };
        let mut changed = conf.clone();
        changed.extend(nested(
            3,
            &[2u64.to_ne_bytes(), 99u64.to_ne_bytes()].concat(),
        ));
        changed.extend(token.clone());
        assert!(attributes_equal(
            std::slice::from_ref(&expected),
            &[NetworkAttribute {
                kind: 26,
                value: nested(10, &changed)
            }],
            Context::Link
        ));
        for field in [2, 7] {
            let mut value = if field == 2 {
                nested(2, &[9u8; 16])
            } else {
                conf.clone()
            };
            value.extend(nested(3, &stats));
            value.extend(if field == 7 {
                nested(7, &[9u8; 16])
            } else {
                token.clone()
            });
            assert!(!attributes_equal(
                std::slice::from_ref(&expected),
                &[NetworkAttribute {
                    kind: 26,
                    value: nested(10, &value)
                }],
                Context::Link
            ));
        }
        let bad = NetworkAttribute {
            kind: 26,
            value: vec![1, 2, 3],
        };
        assert!(!attributes_equal(
            std::slice::from_ref(&bad),
            std::slice::from_ref(&bad),
            Context::Link
        ));
    }
    fn route() -> KernelRoute {
        KernelRoute {
            namespace: NetworkNamespace {
                device: 1,
                inode: 1,
            },
            query: RouteQuery::new("192.0.2.1".parse().unwrap(), 969).with_mark(99),
            table: Some(254),
            output_interface: NonZeroU32::new(2),
            gateway: None,
            preferred_source: None,
            kind: RouteKind::Unicast,
            flags: 0,
            nexthop_id: None,
            attributes: vec![NetworkAttribute {
                kind: 12,
                value: vec![0; 32],
            }],
            incomplete: vec![],
        }
    }
    #[test]
    fn route_cache_usage_changes_without_hiding_expiration_error_or_context() {
        let expected = route();
        let mut current = expected.clone();
        current.attributes[0].value[16] = 2;
        assert!(same_kernel_route(&expected, &current));
        current.attributes[0].value[8] = 1;
        assert!(!same_kernel_route(&expected, &current));
        current = expected.clone();
        current.attributes[0].value[12] = 1;
        assert!(!same_kernel_route(&expected, &current));
        current = expected.clone();
        current.attributes[0].value[20] = 1;
        assert!(!same_kernel_route(&expected, &current));
        current = expected.clone();
        current.query = RouteQuery::new("192.0.2.2".parse().unwrap(), 969).with_mark(99);
        assert!(!same_kernel_route(&expected, &current));
    }
    #[test]
    fn flagged_unknown_representations_remain_opaque() {
        for kind in [0x4017, 0x8017] {
            let mut expected = link();
            expected.unknown[0].kind = kind;
            let mut current = expected.clone();
            current.unknown[0].value.fill(9);
            assert!(!same_link(&expected, &current));
        }
        let left = nested(10, &nested(0x4003, &[1; 16]));
        let right = nested(10, &nested(0x4003, &[2; 16]));
        assert!(!value_equal(26, &left, &right, Context::Link));
        assert!(!value_equal(0x4006, &[1; 16], &[2; 16], Context::Address));
    }
}
