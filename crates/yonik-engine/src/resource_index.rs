//! Индексы membership, а не второй policy evaluator.
use crate::resource_bundle::ResourceKind;
use std::{collections::HashSet, io, net::IpAddr};
use yonik_domain::{Cidr, DomainName};
pub struct RuleSetIndex {
    exact: HashSet<String>,
    suffix: HashSet<String>,
    v4: HashSet<(u32, u8)>,
    v6: HashSet<(u128, u8)>,
}
fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Некорректный canonical source")
}
impl RuleSetIndex {
    pub fn parse(source: &[u8], kind: ResourceKind, count: u64) -> io::Result<Self> {
        if source.len() > 128 * 1024 * 1024
            || count == 0
            || count > 1_500_000
            || source.last() != Some(&b'\n')
        {
            return Err(bad());
        }
        let mut result = Self {
            exact: HashSet::new(),
            suffix: HashSet::new(),
            v4: HashSet::new(),
            v6: HashSet::new(),
        };
        let mut actual = 0;
        for line in source[..source.len() - 1].split(|b| *b == b'\n') {
            if line.len() > 299 {
                return Err(bad());
            }
            actual += 1;
            if actual > count {
                return Err(bad());
            }
            let line = std::str::from_utf8(line).map_err(|_| bad())?;
            match kind {
                ResourceKind::Domains => {
                    let (kind, name) = line.split_once(':').ok_or_else(bad)?;
                    let parsed: DomainName = name.parse().map_err(|_| bad())?;
                    if parsed.as_str() != name {
                        return Err(bad());
                    }
                    match kind {
                        "exact" => {
                            result.exact.insert(name.to_owned());
                        }
                        "suffix" => {
                            result.suffix.insert(name.to_owned());
                        }
                        _ => return Err(bad()),
                    }
                }
                ResourceKind::Cidrs => {
                    let cidr: Cidr = line.parse().map_err(|_| bad())?;
                    if cidr.to_string() != line {
                        return Err(bad());
                    }
                    let (ip, prefix) = line.split_once('/').ok_or_else(bad)?;
                    let prefix = prefix.parse().map_err(|_| bad())?;
                    match ip.parse().map_err(|_| bad())? {
                        IpAddr::V4(ip) => {
                            result.v4.insert((u32::from(ip), prefix));
                        }
                        IpAddr::V6(ip) => {
                            result.v6.insert((u128::from(ip), prefix));
                        }
                    }
                }
            }
        }
        if actual != count {
            return Err(bad());
        }
        Ok(result)
    }
    pub fn matches_domain(&self, name: &DomainName) -> bool {
        let mut name = name.as_str();
        if self.exact.contains(name) {
            return true;
        }
        loop {
            if self.suffix.contains(name) {
                return true;
            }
            match name.split_once('.') {
                Some((_, tail)) => name = tail,
                None => return false,
            }
        }
    }
    pub fn matches_ip(&self, ip: IpAddr) -> bool {
        match ip.to_canonical() {
            IpAddr::V4(ip) => (0..=32).any(|p| {
                self.v4.contains(&(
                    u32::from(ip) & if p == 0 { 0 } else { u32::MAX << (32 - p) },
                    p,
                ))
            }),
            IpAddr::V6(ip) => (0..=128).any(|p| {
                self.v6.contains(&(
                    u128::from(ip) & if p == 0 { 0 } else { u128::MAX << (128 - p) },
                    p,
                ))
            }),
        }
    }
}
