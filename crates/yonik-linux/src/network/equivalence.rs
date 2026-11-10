//! Эквивалентность сетевого контракта без packet counters и служебных timestamps.
//! Не обновляет поколения и не заменяет guards потери Netlink/DNS/manager events.
use super::topology::TopologyCapture;
use super::{KernelRoute, LinuxAddress, LinuxLink, LinuxRoute, LinuxTopology, NetworkAttribute};
use std::{collections::BTreeSet, time::Duration};
use tokio::time::Instant;

#[derive(Clone, Copy)]
pub(crate) struct CaptureWindow {
    pub(crate) start: Instant,
    pub(crate) end: Instant,
}

#[derive(Clone, Copy)]
enum Context {
    Link,
    Address(Option<(CaptureWindow, CaptureWindow)>),
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
    same_topology(left, right, None)
}
pub(crate) fn same_captured_topology(
    left: &LinuxTopology,
    left_time: TopologyCapture,
    right: &LinuxTopology,
    right_time: TopologyCapture,
) -> bool {
    same_topology(left, right, Some((left_time, right_time)))
}
fn same_topology(
    left: &LinuxTopology,
    right: &LinuxTopology,
    times: Option<(TopologyCapture, TopologyCapture)>,
) -> bool {
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
        && left.addresses.iter().zip(&right.addresses).all(|(a, b)| {
            same_address(
                a,
                b,
                times.map(|(a_time, b_time)| (a_time.family(a.family), b_time.family(b.family))),
            )
        })
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
fn same_address(
    left: &LinuxAddress,
    right: &LinuxAddress,
    times: Option<(CaptureWindow, CaptureWindow)>,
) -> bool {
    equal_record!(LinuxAddress,left,right,unknown,Context::Address(times);family,index,address,local,prefix_len,scope,flags)
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
        (Context::Address(times), 6) => address_cache_equal(left, right, times),
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
// Linux вычитает целое число секунд от tstamp. Поэтому expiry лежит
// в (read_start + remaining - 1s, read_end + remaining]. Сравниваем
// пересечение этих интервалов, без допуска, растущего при каждом sweep.
fn address_cache_equal(
    left: &[u8],
    right: &[u8],
    times: Option<(CaptureWindow, CaptureWindow)>,
) -> bool {
    address_cache_equal_at(left, right, times, Instant::now())
}
fn address_cache_equal_at(
    left: &[u8],
    right: &[u8],
    times: Option<(CaptureWindow, CaptureWindow)>,
    now: Instant,
) -> bool {
    if left.len() != 16 || right.len() != 16 || left[8..] != right[8..] {
        return false;
    }
    let Some((before, after)) = times else {
        return left == right;
    };
    if before.start > before.end || after.start > after.end || before.end > after.start {
        return false;
    }
    let now = now.max(after.end);
    [0, 4].into_iter().all(|offset| {
        let a = u32::from_ne_bytes(
            left[offset..offset + 4]
                .try_into()
                .expect("Проверены 16 байт"),
        );
        let b = u32::from_ne_bytes(
            right[offset..offset + 4]
                .try_into()
                .expect("Проверены 16 байт"),
        );
        if a == u32::MAX || b == u32::MAX {
            return a == b;
        }
        if a == 0 || b == 0 {
            return false;
        }
        let low_a = before.start + Duration::from_secs(u64::from(a - 1));
        let high_a = before.end + Duration::from_secs(u64::from(a));
        let low_b = after.start + Duration::from_secs(u64::from(b - 1));
        let high_b = after.end + Duration::from_secs(u64::from(b));
        // Не сохраняем сессию, если к концу чтения expiry уже возможно.
        now < low_a && now < low_b && low_a < high_b && low_b < high_a
    })
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
    fn window(t: Instant, ms: u64) -> CaptureWindow {
        let t = t + Duration::from_millis(ms);
        CaptureWindow { start: t, end: t }
    }
    fn cache(preferred: u32, valid: u32) -> Vec<u8> {
        [preferred, valid, 7, 9]
            .into_iter()
            .flat_map(u32::to_ne_bytes)
            .collect()
    }
    #[test]
    fn finite_address_natural_countdown_remains_equivalent() {
        let t = Instant::now();
        assert!(address_cache_equal(
            &cache(100, 200),
            &cache(99, 199),
            Some((window(t, 0), window(t, 1000)))
        ));
    }
    #[test]
    fn finite_lifetimes_use_capture_intervals_and_keep_renewal_and_expiry_fences() {
        let t = Instant::now();
        let equal = |a: Vec<u8>, b: Vec<u8>, elapsed: u64, now: u64| {
            address_cache_equal_at(
                &a,
                &b,
                Some((window(t, 0), window(t, elapsed))),
                t + Duration::from_millis(now),
            )
        };
        // Пересечение секундной границы возможно даже за 2 ms.
        assert!(equal(cache(100, 200), cache(99, 199), 2, 2));
        assert!(equal(cache(100, 200), cache(100, 200), 2, 2));
        assert!(equal(cache(100, 200), cache(90, 190), 10_000, 10_000));
        assert!(!equal(cache(100, 200), cache(100, 200), 10_000, 10_000));
        assert!(!equal(cache(100, 200), cache(88, 190), 10_000, 10_000));
        assert!(!equal(cache(100, 200), cache(90, 188), 10_000, 10_000));
        let mut renewed = cache(90, 190);
        renewed[12] = 10;
        assert!(!equal(cache(100, 200), renewed, 10_000, 10_000));
        let mut replaced = cache(90, 190);
        replaced[8] = 10;
        assert!(!equal(cache(100, 200), replaced, 10_000, 10_000));
        // Нулевой остаток и возможный expiry во время последующих dump/check
        // запрещены, даже если RTM_DELADDR/DEPRECATED ещё не обработаны.
        assert!(!equal(cache(2, 200), cache(0, 198), 2_000, 2_000));
        assert!(!equal(cache(2, 200), cache(2, 200), 0, 2_000));
        assert!(!equal(cache(u32::MAX, 2), cache(u32::MAX, 0), 2_000, 2_000));
        assert!(!equal(cache(u32::MAX, 2), cache(u32::MAX, 2), 0, 2_000));
        assert!(equal(
            cache(u32::MAX, u32::MAX),
            cache(u32::MAX, u32::MAX),
            1_000_000,
            1_000_000
        ));
        for (a, b) in [
            (cache(100, 200), cache(u32::MAX, u32::MAX)),
            (cache(u32::MAX, u32::MAX), cache(99, 199)),
        ] {
            assert!(!equal(a, b, 1_000, 1_000));
        }
        assert!(!address_cache_equal_at(
            &cache(100, 200),
            &cache(100, 200),
            Some((window(t, 1000), window(t, 0))),
            t
        ));
    }
    #[test]
    fn address_countdown_does_not_hide_address_link_flags_or_prefix_changes() {
        let t = Instant::now();
        let address = LinuxAddress {
            family: 2,
            index: 2,
            address: "192.0.2.1".parse().unwrap(),
            local: None,
            prefix_len: 24,
            scope: 0,
            flags: 0,
            unknown: vec![NetworkAttribute {
                kind: 6,
                value: cache(100, 200),
            }],
        };
        let mut current = address.clone();
        current.unknown[0].value = cache(99, 199);
        let times = Some((window(t, 0), window(t, 1000)));
        assert!(same_address(&address, &current, times));
        for change in 0..4 {
            let mut changed = current.clone();
            match change {
                0 => changed.address = "192.0.2.2".parse().unwrap(),
                1 => changed.index += 1,
                2 => changed.flags ^= 0x20,
                _ => changed.prefix_len -= 1,
            }
            assert!(!same_address(&address, &changed, times));
        }
        assert!(!same_address(&address, &current, None));
    }
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
        assert!(!value_equal(
            0x4006,
            &[1; 16],
            &[2; 16],
            Context::Address(None)
        ));
    }
}
