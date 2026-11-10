//! Неизменяемый сетевой план системной сессии. Авторизацию и порядок worker/nft
//! обеспечивает session host; этот модуль не публикует Applied/Connected.
use crate::{
    LinuxTopology,
    managed_network::{
        Journal, Ownership, RecordState, Resource, Scope,
        tun::{TunAddress, TunError, TunExecutor, TunRoute, TunRule, session_external_topology},
    },
};
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
use yonik_engine::{SystemTun, SystemTunAddress};

const MTU: u16 = 1500;
// ID 1 занят действующей public-protection policy; session использует своё окно.
const TUN: u64 = 16;
const ADDRESSES: u64 = TUN + 1;
const ROUTES: u64 = TUN + 3;
const RULES: u64 = TUN + 5;
const ALLOCATION_ATTEMPTS: usize = 8;

/// Числа ресурсов выбираются helper, а не десериализуются из запроса UI.
/// Существование плана не доказывает готовность транспорта или DNS.
pub struct SessionNetworkPlan {
    owner: Ownership,
    name: u32,
    table: u32,
    priority: u32,
    mark: u32,
    addresses: [TunAddress; 2],
}
impl fmt::Debug for SessionNetworkPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionNetworkPlan(<скрыто>)")
    }
}
impl SessionNetworkPlan {
    pub fn reserve(owner: Ownership, baseline: &LinuxTopology) -> Result<Self, TunError> {
        if owner.namespace() != (baseline.namespace.device, baseline.namespace.inode) {
            return Err(TunError::Namespace);
        }
        for _ in 0..ALLOCATION_ATTEMPTS {
            let mut random = [0; 24];
            getrandom::getrandom(&mut random).map_err(|_| TunError::Io)?;
            let plan = Self::candidate(owner, random)?;
            if plan.available(baseline) {
                return Ok(plan);
            }
        }
        Err(TunError::Foreign)
    }
    fn candidate(owner: Ownership, random: [u8; 24]) -> Result<Self, TunError> {
        let number = |offset| {
            u32::from_le_bytes([
                random[offset],
                random[offset + 1],
                random[offset + 2],
                random[offset + 3],
            ])
        };
        Ok(Self {
            owner,
            name: number(0).max(1),
            table: 10000 + number(4) % 20000,
            priority: 10000 + number(8) % 10000,
            mark: 0x5900_0000 | (number(12) & 0x00ff_ffff),
            addresses: [
                TunAddress::new(
                    IpAddr::V4(Ipv4Addr::new(198, 18 + (random[16] & 1), random[17], 1)),
                    30,
                )?,
                TunAddress::new(
                    IpAddr::V6(Ipv6Addr::new(
                        0xfd79,
                        0x6f6e,
                        u16::from_le_bytes([random[18], random[19]]),
                        u16::from_le_bytes([random[20], random[21]]),
                        0,
                        0,
                        0,
                        1,
                    )),
                    126,
                )?,
            ],
        })
    }
    fn available(&self, topology: &LinuxTopology) -> bool {
        let name = format!("tofu{:08x}", self.name);
        !topology
            .links
            .iter()
            .any(|link| link.name == name.as_bytes())
            && !topology
                .routes
                .iter()
                .any(|route| route.table == self.table)
            && !topology.rules.iter().any(|rule| {
                rule.table == self.table
                    || rule.priority == Some(self.priority)
                    || rule.fwmark.is_some_and(|mark| {
                        mark & rule.fwmask.unwrap_or(u32::MAX)
                            == self.mark & rule.fwmask.unwrap_or(u32::MAX)
                    })
            })
            && !self.addresses.iter().any(|planned| {
                topology.addresses.iter().any(|address| {
                    overlaps(
                        planned.address(),
                        planned.prefix_len(),
                        address.address,
                        address.prefix_len,
                    )
                }) || topology.routes.iter().any(|route| {
                    route.destination.length != 0
                        && overlaps(
                            planned.address(),
                            planned.prefix_len(),
                            route.destination.address,
                            route.destination.length,
                        )
                })
            })
    }
    pub fn compiler_tun(&self) -> Result<SystemTun, TunError> {
        let addresses = self
            .addresses
            .iter()
            .map(|address| {
                SystemTunAddress::new(address.address(), address.prefix_len())
                    .map_err(|_| TunError::Invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        SystemTun::new(self.name, MTU, self.mark, addresses).map_err(|_| TunError::Invalid)
    }
    pub fn socket_mark(&self) -> u32 {
        self.mark
    }
    pub fn routing_table(&self) -> u32 {
        self.table
    }
    fn routes(&self) -> Result<[TunRoute; 2], TunError> {
        Ok([
            TunRoute::new(Ipv4Addr::UNSPECIFIED.into(), 0, self.table, 100)?,
            TunRoute::new(Ipv6Addr::UNSPECIFIED.into(), 0, self.table, 100)?,
        ])
    }
    fn rules(&self) -> Result<[TunRule; 2], TunError> {
        Ok([
            TunRule::new(self.priority, self.table, self.mark, false)?,
            TunRule::new(self.priority, self.table, self.mark, true)?,
        ])
    }
    fn resources(&self, index: u32) -> Vec<(u64, Resource)> {
        let mut resources = vec![(TUN, Resource::Tun { name_id: self.name })];
        for (offset, address) in self.addresses.iter().enumerate() {
            resources.push((
                ADDRESSES + offset as u64,
                Resource::Address {
                    tun_id: TUN,
                    ifindex: index,
                    address: address.address(),
                    prefix_len: address.prefix_len(),
                },
            ));
        }
        for (offset, destination) in [
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        ]
        .into_iter()
        .enumerate()
        {
            resources.push((
                ROUTES + offset as u64,
                Resource::Route {
                    table: self.table,
                    destination,
                    prefix_len: 0,
                    protocol: 253,
                    priority: 100,
                    ifindex: index,
                },
            ));
            resources.push((
                RULES + offset as u64,
                Resource::TunRule {
                    tun_id: TUN,
                    ifindex: index,
                    priority: self.priority,
                    table: self.table,
                    mark: self.mark,
                    ipv6: offset == 1,
                },
            ));
        }
        resources
    }
    fn check_records(
        &self,
        journal: &Journal,
        index: u32,
        expected_count: Option<usize>,
    ) -> Result<(), TunError> {
        if journal.ownership() != self.owner {
            return Err(TunError::Foreign);
        }
        let expected = self.resources(index);
        if journal.records().iter().any(|record| {
            record.scope() == Scope::PublicBlock && (TUN..RULES + 2).contains(&record.id())
        }) {
            return Err(TunError::Foreign);
        }
        let mut count = 0;
        for record in journal
            .records()
            .iter()
            .filter(|record| record.scope() == Scope::Session)
        {
            count += 1;
            if !expected
                .iter()
                .any(|(id, resource)| *id == record.id() && resource == record.resource())
                || expected_count.is_some()
                    && (record.state() != RecordState::Confirmed
                        || journal.needs_reconciliation(record.id()))
            {
                return Err(TunError::Foreign);
            }
        }
        if expected_count.is_some_and(|expected| count != expected) {
            return Err(TunError::Evidence);
        }
        Ok(())
    }
    /// Только адреса/таблицы, без захвата трафика. При ошибке host обязан перейти
    /// в recovery: частичный durable journal сохраняется для явного cleanup.
    pub async fn configure(
        &self,
        executor: &mut TunExecutor,
        journal: &mut Journal,
    ) -> Result<u32, TunError> {
        self.check_records(journal, 0, Some(0))?;
        let index = executor.ensure_tun(journal, TUN, self.name).await?;
        executor
            .configure_link(journal, TUN, u32::from(MTU))
            .await?;
        for (offset, address) in self.addresses.iter().enumerate() {
            executor
                .ensure_address(journal, TUN, ADDRESSES + offset as u64, *address)
                .await?;
        }
        for (offset, route) in self.routes()?.into_iter().enumerate() {
            executor
                .ensure_route(journal, TUN, ROUTES + offset as u64, route)
                .await?;
        }
        self.check_records(journal, index, Some(5))?;
        Ok(index)
    }
    /// Вызывать после подготовки защиты и проверки transport escape. Успех этого
    /// метода сам по себе не означает readiness ядра или соединения.
    pub async fn capture(
        &self,
        executor: &mut TunExecutor,
        journal: &mut Journal,
        index: u32,
    ) -> Result<(), TunError> {
        self.check_records(journal, index, Some(5))?;
        for (offset, rule) in self.rules()?.into_iter().enumerate() {
            executor
                .ensure_rule(journal, TUN, RULES + offset as u64, rule)
                .await?;
        }
        self.check_records(journal, index, Some(7))
    }
    /// Проверяет только внешнюю топологию. Per-link devconf/token своего TUN
    /// требуют отдельного binding; этот результат нельзя публиковать как Applied.
    pub fn verify_external(
        &self,
        baseline: &LinuxTopology,
        current: LinuxTopology,
        journal: &Journal,
        index: u32,
    ) -> Result<(), TunError> {
        self.check_records(journal, index, Some(7))?;
        let projected = session_external_topology(current, journal, TUN, MTU)?;
        if !crate::network::same_external_topology(&projected, baseline) {
            return Err(TunError::Evidence);
        }
        Ok(())
    }
    /// Host обязан сначала подтвердить пустую worker cgroup. Сам план не умеет
    /// подтверждать остановку процесса и не снимает публичную блокировку.
    pub async fn cleanup_after_worker(
        &self,
        executor: &mut TunExecutor,
        journal: &mut Journal,
        index: u32,
    ) -> Result<(), TunError> {
        self.check_records(journal, index, None)?;
        for id in (TUN..RULES + 2).rev() {
            if !journal
                .records()
                .iter()
                .any(|record| record.scope() == Scope::Session && record.id() == id)
            {
                continue;
            }
            if id >= RULES {
                executor.remove_rule(journal, TUN, id).await?;
            } else if id >= ROUTES {
                executor.remove_route(journal, TUN, id).await?;
            } else if id >= ADDRESSES {
                executor.remove_address(journal, TUN, id).await?;
            } else {
                executor.remove_tun(journal, TUN).await?;
            }
        }
        self.check_records(journal, index, Some(0))
    }
}
fn overlaps(left: IpAddr, left_len: u8, right: IpAddr, right_len: u8) -> bool {
    let length = left_len.min(right_len);
    match (left, right) {
        (IpAddr::V4(left), IpAddr::V4(right)) if length <= 32 => {
            length == 0 || (u32::from(left) >> (32 - length)) == (u32::from(right) >> (32 - length))
        }
        (IpAddr::V6(left), IpAddr::V6(right)) if length <= 128 => {
            length == 0
                || (u128::from(left) >> (128 - length)) == (u128::from(right) >> (128 - length))
        }
        _ => false,
    }
}
#[cfg(test)]
mod tests;
