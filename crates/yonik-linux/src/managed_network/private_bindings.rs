//! Привязки внешних сетей без владения их интерфейсами и маршрутами.
//! Host обязан атомарно заменить nft policy при событии/потере наблюдения.
use super::firewall::{IpPrefix, LanException, PrivateRoute};
use crate::network::{LinuxTopology, NetworkSnapshot, NetworkStamp};

const MAX_BINDINGS: usize = 64;
const MAX_LINKS: usize = 256;
const MAX_ROUTES: usize = 4096;

/// Явно выбранный пользователем путь; не эвристика по RFC1918 или имени link.
#[derive(Clone)]
pub struct BindingRequest {
    pub prefix: IpPrefix,
    pub ifindex: u32,
    pub table: u32,
    pub wireguard: bool,
}

/// Не Policy: проверенные входы существующего PublicBlockPolicy.
pub struct PrivateBindings {
    stamp: NetworkStamp,
    topology: LinuxTopology,
    requests: Vec<BindingRequest>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingError {
    Limit,
    Stale,
    MissingPath,
}

impl PrivateBindings {
    /// Ограниченное чтение по запросу; watcher принадлежит host и живёт до
    /// снятия guard. Никакого периодического сканирования или сетевых эффектов.
    pub async fn capture(
        watch: &crate::network::NetworkWatch,
        cancel: &mut crate::Cancellation,
        requests: Vec<BindingRequest>,
    ) -> Result<Self, BindingError> {
        if requests.len() > MAX_BINDINGS {
            return Err(BindingError::Limit);
        }
        let before = watch.current();
        let topology = crate::network::read_topology(cancel, std::time::Duration::from_secs(3))
            .await
            .map_err(|_| BindingError::Stale)?;
        Self::verify(before, watch.current(), topology, requests)
    }

    /// before/after снимаются с одного NetworkWatch вокруг read_topology().
    /// Данные принимаются только от доверенного адаптера, не из IPC клиента.
    pub(crate) fn verify(
        before: NetworkSnapshot,
        after: NetworkSnapshot,
        topology: LinuxTopology,
        requests: Vec<BindingRequest>,
    ) -> Result<Self, BindingError> {
        if requests.len() > MAX_BINDINGS
            || topology.links.len() > MAX_LINKS
            || topology.routes.len() > MAX_ROUTES
        {
            return Err(BindingError::Limit);
        }
        if !current(before, after.stamp) || !current(after, before.stamp) {
            return Err(BindingError::Stale);
        }
        for request in &requests {
            let links: Vec<_> = topology
                .links
                .iter()
                .filter(|l| l.index == request.ifindex)
                .collect();
            if links.len() != 1 || request.ifindex == 0 || request.table == 0 {
                return Err(BindingError::MissingPath);
            }
            let link = links[0];
            if link.flags & 1 == 0
                || link.master.is_some()
                || (request.wireguard && link.kind.as_deref() != Some(b"wireguard"))
            {
                return Err(BindingError::MissingPath);
            }
            // Сравнение через конструктор избегает второго CIDR-парсера.
            let found = topology.routes.iter().any(|route| {
                IpPrefix::new(route.destination.address, route.destination.length).ok()
                    == Some(request.prefix)
                    && route.table == request.table
                    && route.output_interface == Some(request.ifindex)
                    && route.kind == 1
                    && route.source.length == 0
                    && route.tos == 0
                    && route.input_interface.is_none()
                    && route.multipath.is_empty()
                    && route.nexthop_id.is_none()
                    && route.unknown.iter().all(|a| match a.kind {
                        // Linux UAPI rta_cacheinfo: только бессрочный маршрут без error.
                        12 => a.value.len() == 32 && a.value[8..16] == [0; 8],
                        // RTA_PREF: только стандартное medium preference.
                        20 => a.value == [0],
                        _ => false,
                    })
                    && (request.wireguard
                        || (route.gateway.is_none()
                            && (route.scope == 253
                                || (route.family == 10
                                    && route.scope == 0
                                    && route.protocol == 2))))
            });
            if !found {
                return Err(BindingError::MissingPath);
            }
        }
        Ok(Self {
            stamp: before.stamp,
            topology,
            requests,
        })
    }

    /// Любое изменение требует новой явной verify. Старый ifindex не оживает
    /// автоматически после reconnect/reuse. Приватные guards сохраняются.
    pub fn resolve(
        &self,
        snapshot: NetworkSnapshot,
        topology: &LinuxTopology,
    ) -> (Vec<PrivateRoute>, Vec<LanException>) {
        let valid = current(snapshot, self.stamp)
            && crate::network::same_external_topology(topology, &self.topology);
        let mut private = Vec::new();
        let mut lan = Vec::new();
        for request in &self.requests {
            if request.wireguard {
                private.push(
                    PrivateRoute::new(request.prefix, valid.then_some(request.ifindex))
                        .expect("Индекс проверен при создании привязки"),
                );
            } else if valid {
                lan.push(
                    LanException::new(request.prefix, request.ifindex)
                        .expect("Индекс проверен при создании привязки"),
                );
            }
        }
        (private, lan)
    }
}
fn current(snapshot: NetworkSnapshot, stamp: NetworkStamp) -> bool {
    snapshot.coverage.routes && snapshot.is_current(stamp)
}

#[cfg(test)]
mod tests;
