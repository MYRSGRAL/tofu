//! Только multicast RTNETLINK: отслеживание начинается после bind, без snapshot/dump.

use std::collections::BTreeMap;
use std::os::fd::OwnedFd;

use rustix::net::netlink::SocketAddrNetlink;
use rustix::net::{self, AddressFamily, RecvFlags, SocketFlags, SocketType};
use tokio::io::unix::AsyncFd;
use tokio::sync::watch;

use super::{NetworkPublisher, NetworkSource};

const DATAGRAM_LIMIT: usize = 65_536;
const ENTRY_LIMIT: usize = 4_096;
const CACHE_LIMIT: usize = 4 * 1024 * 1024;
// RTNLGRP_LINK, IPV4_IFADDR/ROUTE/RULE, IPV6_IFADDR/ROUTE/RULE, NEXTHOP.
const GROUPS: u32 =
    (1 << 0) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 10) | (1 << 18) | (1 << 31);

fn open_socket() -> std::io::Result<AsyncFd<OwnedFd>> {
    let socket = net::socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )?;
    net::bind(&socket, &SocketAddrNetlink::new(0, GROUPS))?;
    // OwnedFd хранит один открытый fd до Drop; новый register требует unsafe,
    // запрещённый политикой crate.
    #[allow(deprecated)]
    AsyncFd::new(socket)
}

pub(super) async fn run(publisher: NetworkPublisher, mut cancel: watch::Receiver<bool>) {
    if *cancel.borrow() || cancel.has_changed().is_err() {
        return;
    }
    let Ok(socket) = open_socket() else {
        publisher.lost(NetworkSource::Routes);
        return;
    };
    publisher.ready(NetworkSource::Routes);
    let mut bytes = vec![0; DATAGRAM_LIMIT];
    let mut observations = Observations::default();
    loop {
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow_and_update() {
                    return;
                }
            }
            readable = socket.readable() => {
                let Ok(mut ready) = readable else { break; };
                let result = ready.try_io(|fd| {
                    net::recvfrom(fd.get_ref(), bytes.as_mut_slice(), RecvFlags::TRUNC)
                        .map(|(_, length, address)| (length, address))
                        .map_err(std::io::Error::from)
                });
                let (length, address) = match result {
                    Err(_) => continue,
                    Ok(Err(error)) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Ok(Err(_)) => break,
                    Ok(Ok(received)) => received,
                };
                let Some(sender) = address.and_then(|address| SocketAddrNetlink::try_from(address).ok()) else {
                    break;
                };
                match observations.accept(&bytes[..length.min(bytes.len())], length, sender.pid()) {
                    Ok(true) => publisher.changed(NetworkSource::Routes),
                    Ok(false) => {},
                    Err(()) => break,
                }
            }
        }
    }
    publisher.lost(NetworkSource::Routes);
}

#[derive(Default)]
struct Observations {
    // Tombstone сохраняет дедупликацию DEL до повторного NEW.
    records: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    bytes: usize,
}

impl Observations {
    fn accept(&mut self, bytes: &[u8], length: usize, sender: u32) -> Result<bool, ()> {
        if sender != 0 {
            return Ok(false);
        }
        if length == 0 || length > DATAGRAM_LIMIT || length != bytes.len() {
            return Err(());
        }
        let mut rest = bytes;
        let mut changed = false;
        while !rest.is_empty() {
            if rest.len() < 16 {
                return Err(());
            }
            let size = read_u32(rest) as usize;
            if size < 16 || size > rest.len() {
                return Err(());
            }
            let kind = read_u16(&rest[4..]);
            let flags = read_u16(&rest[6..]);
            // NLMSG_ERROR, NLMSG_OVERRUN, NLM_F_DUMP_INTR.
            if matches!(kind, 2 | 4) || flags & 0x10 != 0 {
                return Err(());
            }
            if let Some(record) = Record::parse(kind, &rest[16..size])? {
                changed |= self.update(record)?;
            }
            rest = advance(rest, size)?;
        }
        Ok(changed)
    }

    fn update(&mut self, record: Record) -> Result<bool, ()> {
        if self.records.get(&record.key) == Some(&record.value) {
            return Ok(false);
        }
        let old_size = self.records.get(&record.key).map_or(0, |value| {
            record.key.len() + value.as_ref().map_or(0, Vec::len)
        });
        let new_size = record.key.len() + record.value.as_ref().map_or(0, Vec::len);
        let total = self.bytes - old_size + new_size;
        if total > CACHE_LIMIT
            || (!self.records.contains_key(&record.key) && self.records.len() >= ENTRY_LIMIT)
        {
            return Err(());
        }
        self.bytes = total;
        self.records.insert(record.key, record.value);
        Ok(true)
    }
}

struct Record {
    key: Vec<u8>,
    value: Option<Vec<u8>>,
}

impl Record {
    fn parse(kind: u16, payload: &[u8]) -> Result<Option<Self>, ()> {
        let (class, base_len) = match kind {
            16 | 17 => (16, 16),
            108 | 109 => (108, 16),
            20 | 21 => (20, 8),
            24 | 25 => (24, 12),
            32 | 33 => (32, 12),
            104 | 105 => (104, 8),
            _ => return Ok(None),
        };
        if payload.len() < base_len {
            return Err(());
        }
        let mut attributes = attributes(&payload[base_len..])?;
        attributes.sort_unstable();
        let mut key = vec![class];
        let mut value = match class {
            16 | 108 => {
                key.extend_from_slice(&payload[4..8]);
                // ifi_change — маска уведомления, не состояние интерфейса.
                payload[2..12].to_vec()
            }
            20 => {
                key.extend_from_slice(&payload[..2]);
                key.extend_from_slice(&payload[4..8]);
                payload[..base_len].to_vec()
            }
            24 => {
                key.extend_from_slice(&payload[..5]);
                payload[..base_len].to_vec()
            }
            32 => {
                key.extend_from_slice(&payload[..4]);
                payload[..base_len].to_vec()
            }
            _ => payload[..base_len].to_vec(),
        };
        for (kind, bytes) in attributes {
            let ty = kind & 0x3fff;
            // Счётчики и убывающие lifetime не меняют сетевую конфигурацию.
            let noise = match class {
                16 | 108 => matches!(ty, 7 | 11 | 23 | 35 | 47 | 48),
                20 => ty == 6,
                24 => matches!(ty, 12 | 23),
                104 => matches!(ty, 14 | 15 | 17),
                _ => false,
            };
            if noise {
                continue;
            }
            let identity = match class {
                20 => matches!(ty, 1 | 2),
                24 => matches!(ty, 1 | 2 | 6 | 15),
                32 => ty == 6,
                104 => ty == 1,
                _ => false,
            };
            let normalized;
            let bytes = if matches!(class, 16 | 108) && ty == 26 {
                normalized = normalize_link_families(bytes)?;
                normalized.as_slice()
            } else if matches!(class, 16 | 108) && ty == 12 && payload[0] == 10 {
                normalized = normalize_ipv6(bytes)?;
                normalized.as_slice()
            } else {
                bytes
            };
            if identity {
                append_canonical(&mut key, kind, bytes);
            }
            append_canonical(&mut value, kind, bytes);
        }
        Ok(Some(Self {
            key,
            value: (kind == u16::from(class)).then_some(value),
        }))
    }
}

// IFLA_AF_SPEC содержит атрибуты семейств, IPv6 — вложенные статистики.
fn normalize_link_families(bytes: &[u8]) -> Result<Vec<u8>, ()> {
    let mut families = attributes(bytes)?;
    families.sort_unstable();
    let mut result = Vec::new();
    for (family, payload) in families {
        if family & 0x3fff == 10 {
            append_canonical(&mut result, family, &normalize_ipv6(payload)?);
        } else {
            append_canonical(&mut result, family, payload);
        }
    }
    Ok(result)
}

fn normalize_ipv6(bytes: &[u8]) -> Result<Vec<u8>, ()> {
    let mut fields = attributes(bytes)?;
    fields.sort_unstable();
    let mut result = Vec::new();
    for (kind, payload) in fields {
        // IFLA_INET6_STATS, CACHEINFO, ICMP6STATS.
        if !matches!(kind & 0x3fff, 3 | 5 | 6) {
            append_canonical(&mut result, kind, payload);
        }
    }
    Ok(result)
}

fn append_canonical(target: &mut Vec<u8>, kind: u16, bytes: &[u8]) {
    target.extend_from_slice(&kind.to_ne_bytes());
    target.extend_from_slice(&(bytes.len() as u32).to_ne_bytes());
    target.extend_from_slice(bytes);
}

fn attributes(mut bytes: &[u8]) -> Result<Vec<(u16, &[u8])>, ()> {
    let mut result = Vec::new();
    while !bytes.is_empty() {
        if bytes.len() < 4 {
            return Err(());
        }
        let size = usize::from(read_u16(bytes));
        if size < 4 || size > bytes.len() {
            return Err(());
        }
        result.push((read_u16(&bytes[2..]), &bytes[4..size]));
        bytes = advance(bytes, size)?;
    }
    Ok(result)
}

fn advance(bytes: &[u8], size: usize) -> Result<&[u8], ()> {
    if size == bytes.len() {
        return Ok(&[]);
    }
    bytes.get((size + 3) & !3..).ok_or(())
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_ne_bytes([bytes[0], bytes[1]])
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribute(kind: u16, value: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::from(((value.len() + 4) as u16).to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(value);
        bytes.resize((bytes.len() + 3) & !3, 0);
        bytes
    }

    fn message(kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::from(((payload.len() + 16) as u32).to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(&[0; 10]);
        bytes.extend_from_slice(payload);
        bytes.resize((bytes.len() + 3) & !3, 0);
        bytes
    }

    fn link(index: u32, mtu: u32, stats: u32) -> Vec<u8> {
        let mut payload = vec![0; 16];
        payload[4..8].copy_from_slice(&index.to_ne_bytes());
        payload.extend(attribute(4, &mtu.to_ne_bytes()));
        payload.extend(attribute(23, &stats.to_ne_bytes()));
        message(16, &payload)
    }

    fn accept(cache: &mut Observations, bytes: &[u8]) -> Result<bool, ()> {
        cache.accept(bytes, bytes.len(), 0)
    }

    #[tokio::test]
    #[ignore = "Требуется AF_NETLINK/NETLINK_ROUTE в окружении исполнения"]
    async fn native_read_only_subscription_owns_nonblocking_close_on_exec_socket() {
        let socket = open_socket().expect("Подписка на read-only RTNETLINK");
        let address = net::getsockname(socket.get_ref()).expect("Адрес подписки");
        let address = SocketAddrNetlink::try_from(address).expect("NETLINK адрес");
        assert_ne!(address.pid(), 0);
        assert_eq!(address.groups(), GROUPS);
        assert!(
            rustix::io::fcntl_getfd(socket.get_ref())
                .expect("FD flags")
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        assert!(
            rustix::fs::fcntl_getfl(socket.get_ref())
                .expect("Open flags")
                .contains(rustix::fs::OFlags::NONBLOCK)
        );
    }

    #[test]
    fn attribute_order_and_link_change_mask_are_noise() {
        let mut first = link(2, 1500, 1);
        first[28..32].copy_from_slice(&u32::MAX.to_ne_bytes());
        let mut reordered = first.clone();
        reordered[28..32].copy_from_slice(&0u32.to_ne_bytes());
        reordered[32..40].copy_from_slice(&first[40..48]);
        reordered[40..48].copy_from_slice(&first[32..40]);
        let mut cache = Observations::default();
        assert_eq!(accept(&mut cache, &first), Ok(true));
        assert_eq!(accept(&mut cache, &reordered), Ok(false));
    }

    #[test]
    fn duplicate_and_statistics_do_not_invalidate_but_mtu_does() {
        let mut cache = Observations::default();
        assert_eq!(accept(&mut cache, &link(2, 1500, 1)), Ok(true));
        assert_eq!(accept(&mut cache, &link(2, 1500, 2)), Ok(false));
        assert_eq!(accept(&mut cache, &link(2, 1400, 2)), Ok(true));
        assert_eq!(accept(&mut cache, &link(2, 1500, 3)), Ok(true));
    }

    #[test]
    fn malformed_and_truncated_datagrams_are_loss() {
        let mut cache = Observations::default();
        for bytes in [
            vec![],
            vec![0; 15],
            message(16, &[0; 3]),
            message(2, &[0; 4]),
            message(4, &[]),
        ] {
            assert_eq!(accept(&mut cache, &bytes), Err(()));
        }
        let bytes = link(2, 1500, 0);
        assert_eq!(cache.accept(&bytes, bytes.len() + 1, 0), Err(()));
        let mut malformed = bytes.clone();
        malformed[32..34].copy_from_slice(&3u16.to_ne_bytes());
        assert_eq!(accept(&mut cache, &malformed), Err(()));
    }

    #[test]
    fn untrusted_sender_cannot_mutate_dedup_cache() {
        let mut cache = Observations::default();
        let mut bytes = link(2, 1500, 0);
        assert_eq!(cache.accept(&bytes, bytes.len(), 10), Ok(false));
        // nlmsg_pid не удостоверяет отправителя: адрес recvfrom остаётся kernel pid=0.
        bytes[12..16].copy_from_slice(&10u32.to_ne_bytes());
        assert_eq!(accept(&mut cache, &bytes), Ok(true));
    }

    #[test]
    fn address_cache_lifetimes_are_noise_but_flags_change() {
        let mut payload = vec![2, 24, 0, 0, 2, 0, 0, 0];
        payload.extend(attribute(1, &[192, 0, 2, 1]));
        payload.extend(attribute(6, &[1; 16]));
        let mut cache = Observations::default();
        assert_eq!(accept(&mut cache, &message(20, &payload)), Ok(true));
        *payload.last_mut().expect("fixture") = 2;
        assert_eq!(accept(&mut cache, &message(20, &payload)), Ok(false));
        payload[2] = 64;
        assert_eq!(accept(&mut cache, &message(20, &payload)), Ok(true));
    }

    #[test]
    fn route_expiration_is_noise_gateway_changes_are_events() {
        let mut payload = vec![2, 24, 0, 0, 254, 9, 0, 1, 0, 0, 0, 0];
        payload.extend(attribute(1, &[192, 0, 2, 0]));
        payload.extend(attribute(5, &[192, 0, 2, 1]));
        payload.extend(attribute(23, &100u32.to_ne_bytes()));
        let mut cache = Observations::default();
        assert_eq!(accept(&mut cache, &message(24, &payload)), Ok(true));
        let len = payload.len();
        payload[len - 4..].copy_from_slice(&99u32.to_ne_bytes());
        assert_eq!(accept(&mut cache, &message(24, &payload)), Ok(false));
        payload[27] = 2;
        assert_eq!(accept(&mut cache, &message(24, &payload)), Ok(true));
    }

    #[test]
    fn nested_ipv6_statistics_are_noise_but_ra_mtu_is_configuration() {
        let payload = |counter: u32, mtu: u32| {
            let mut base = vec![0; 16];
            base[4..8].copy_from_slice(&2u32.to_ne_bytes());
            let mut ipv6 = attribute(3, &counter.to_ne_bytes());
            ipv6.extend(attribute(6, &counter.to_ne_bytes()));
            ipv6.extend(attribute(9, &mtu.to_ne_bytes()));
            base.extend(attribute(26, &attribute(10, &ipv6)));
            message(16, &base)
        };
        let mut cache = Observations::default();
        assert_eq!(accept(&mut cache, &payload(1, 1500)), Ok(true));
        assert_eq!(accept(&mut cache, &payload(2, 1500)), Ok(false));
        assert_eq!(accept(&mut cache, &payload(2, 1400)), Ok(true));
    }

    #[test]
    fn cache_limit_is_loss_rather_than_silent_eviction() {
        let mut cache = Observations::default();
        for index in 1..=4096 {
            assert_eq!(accept(&mut cache, &link(index, 1500, 0)), Ok(true));
        }
        assert_eq!(accept(&mut cache, &link(4097, 1500, 0)), Err(()));
        assert_eq!(accept(&mut cache, &link(1, 1500, 1)), Ok(false));
        let large = vec![1; 60_000];
        let mut cache = Observations::default();
        let mut lost = false;
        for index in 1..=100u32 {
            let mut payload = vec![0; 16];
            payload[4..8].copy_from_slice(&index.to_ne_bytes());
            payload.extend(attribute(3, &large));
            if accept(&mut cache, &message(16, &payload)).is_err() {
                lost = true;
                break;
            }
        }
        assert!(
            lost,
            "Ограничение байтов должно сработать до лимита записей"
        );
    }

    #[test]
    fn batch_is_coalesced_and_unrelated_message_is_ignored() {
        let mut bytes = link(1, 1500, 0);
        bytes.extend(link(2, 1500, 0));
        bytes.extend(message(28, &[0; 12]));
        let mut cache = Observations::default();
        assert_eq!(accept(&mut cache, &bytes), Ok(true));
        assert_eq!(accept(&mut cache, &bytes), Ok(false));
        assert_eq!(accept(&mut cache, &message(28, &[0; 12])), Ok(false));
    }

    #[test]
    fn rule_and_nexthop_updates_are_observed() {
        let mut cache = Observations::default();
        let mut rule = vec![2, 0, 0, 0, 254, 0, 0, 1, 0, 0, 0, 0];
        rule.extend(attribute(6, &100u32.to_ne_bytes()));
        assert_eq!(accept(&mut cache, &message(32, &rule)), Ok(true));
        assert_eq!(accept(&mut cache, &message(32, &rule)), Ok(false));
        rule[4] = 100;
        assert_eq!(accept(&mut cache, &message(32, &rule)), Ok(true));
        let mut hop = vec![2, 0, 0, 0, 0, 0, 0, 0];
        hop.extend(attribute(1, &1u32.to_ne_bytes()));
        hop.extend(attribute(6, &[192, 0, 2, 1]));
        assert_eq!(accept(&mut cache, &message(104, &hop)), Ok(true));
        assert_eq!(accept(&mut cache, &message(104, &hop)), Ok(false));
        *hop.last_mut().expect("fixture") = 2;
        assert_eq!(accept(&mut cache, &message(104, &hop)), Ok(true));
    }

    #[test]
    fn deletions_invalidate_and_allow_recreation() {
        let mut cache = Observations::default();
        let bytes = link(2, 1500, 0);
        assert_eq!(accept(&mut cache, &bytes), Ok(true));
        let mut deletion = bytes.clone();
        deletion[4..6].copy_from_slice(&17u16.to_ne_bytes());
        assert_eq!(accept(&mut cache, &deletion), Ok(true));
        assert_eq!(accept(&mut cache, &deletion), Ok(false));
        assert_eq!(accept(&mut cache, &bytes), Ok(true));
    }
}
