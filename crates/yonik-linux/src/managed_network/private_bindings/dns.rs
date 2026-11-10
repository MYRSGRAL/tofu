//! Runtime lease: не serde, не UI DTO, без собственного DNS cache/epoch model.
use super::{BindingError, LinuxTopology, NetworkStamp, PrivateBindings};
use crate::{
    Cancellation,
    network::{
        NetworkWatch, RouteKind, RouteQuery, TransportProtocol, lookup_route, read_topology,
    },
};
use std::{
    num::NonZeroU16,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use yonik_engine::{
    CompiledConfig, Engine, PolicyCompileError, PolicyMode,
    private_dns::{DnsEpoch, DnsPolicy, Policy, PrivateDnsRoute},
};
use yonik_import::ParsedServer;

pub struct VerifiedPrivateResolver {
    route: PrivateDnsRoute,
    stamp: NetworkStamp,
    topology: Arc<LinuxTopology>,
    revoked: Arc<AtomicBool>,
    expires_at: Instant,
    epoch: DnsEpoch,
    index: u32,
    table: u32,
    uid: u32,
    mark: u32,
}
#[derive(Debug)]
pub enum PrivateDnsCompileError {
    Binding(BindingError),
    Compiler(PolicyCompileError),
}
impl PrivateBindings {
    #[allow(clippy::too_many_arguments)]
    pub async fn verify_private_resolver(
        &self,
        route: PrivateDnsRoute,
        expires_at: Instant,
        epoch: DnsEpoch,
        uid: u32,
        mark: u32,
        watch: &NetworkWatch,
        cancel: &mut Cancellation,
    ) -> Result<VerifiedPrivateResolver, BindingError> {
        if self.invalidated.load(Ordering::Acquire) {
            return Err(BindingError::Stale);
        }
        let request =
            self.requests
                .iter()
                .find(|r| {
                    r.wireguard
                        && r.prefix.contains(route.resolver().ip())
                        && self.topology.links.iter().any(|l| {
                            l.index == r.ifindex && l.name == route.bind_ifname().as_bytes()
                        })
                })
                .ok_or(BindingError::MissingPath)?;
        let mut token = VerifiedPrivateResolver {
            route,
            stamp: self.stamp,
            topology: Arc::clone(&self.topology),
            revoked: Arc::clone(&self.invalidated),
            expires_at,
            epoch,
            index: request.ifindex,
            table: request.table,
            uid,
            mark,
        };
        token.revalidate(epoch, watch, cancel).await?;
        Ok(token)
    }
}
impl VerifiedPrivateResolver {
    /// Проверяет BOTH transport contexts без принудительного oif, иначе lookup
    /// мог бы скрыть конфликт RPDB. UID/mark закреплены на весь срок lease.
    pub async fn revalidate(
        &mut self,
        epoch: DnsEpoch,
        watch: &NetworkWatch,
        cancel: &mut Cancellation,
    ) -> Result<(), BindingError> {
        let result = self.check(epoch, watch, cancel).await;
        if result.is_err() {
            self.revoked.store(true, Ordering::Release);
        }
        result
    }
    async fn check(
        &self,
        epoch: DnsEpoch,
        watch: &NetworkWatch,
        cancel: &mut Cancellation,
    ) -> Result<(), BindingError> {
        if self.revoked.load(Ordering::Acquire)
            || epoch != self.epoch
            || Instant::now() >= self.expires_at
            || self.expires_at.saturating_duration_since(Instant::now()) > Duration::from_secs(30)
            || !watch.current().fully_observed()
            || !watch.current().is_current(self.stamp)
        {
            return Err(BindingError::Stale);
        }
        let topology = read_topology(cancel, Duration::from_secs(3))
            .await
            .map_err(|_| BindingError::Stale)?;
        if !crate::network::same_external_topology(&topology, &self.topology) {
            return Err(BindingError::Stale);
        }
        for protocol in [TransportProtocol::Tcp, TransportProtocol::Udp] {
            let query = RouteQuery::new(self.route.resolver().ip(), self.uid)
                .with_mark(self.mark)
                .with_transport(protocol, None, Some(self.route.resolver().port()));
            let route = lookup_route(query, cancel, Duration::from_secs(3))
                .await
                .map_err(|_| BindingError::MissingPath)?;
            if route.namespace != self.topology.namespace
                || route.kind != RouteKind::Unicast
                || route.output_interface.map(|i| i.get()) != Some(self.index)
                || route.table != Some(self.table)
                || !route.incomplete.is_empty()
            {
                return Err(BindingError::MissingPath);
            }
        }
        if self.revoked.load(Ordering::Acquire)
            || !watch.current().fully_observed()
            || !watch.current().is_current(self.stamp)
            || Instant::now() >= self.expires_at
        {
            return Err(BindingError::Stale);
        }
        Ok(())
    }
    /// Поглощает lease непосредственно перед compiler; повторное использование
    /// после epoch/expiry требует нового capture. Host перепроверяет перед dial.
    #[allow(clippy::too_many_arguments)]
    pub async fn compile(
        mut self,
        epoch: DnsEpoch,
        watch: &NetworkWatch,
        cancel: &mut Cancellation,
        server: &ParsedServer,
        engine: Engine,
        port: NonZeroU16,
        probe_port: NonZeroU16,
        policy: &Policy,
        mode: PolicyMode,
        dns: &DnsPolicy,
    ) -> Result<CompiledConfig, PrivateDnsCompileError> {
        self.revalidate(epoch, watch, cancel)
            .await
            .map_err(PrivateDnsCompileError::Binding)?;
        yonik_engine::private_dns::compile_native_dns_with_private(
            server,
            engine,
            port,
            probe_port,
            policy,
            mode,
            dns,
            &[self.route],
        )
        .map_err(PrivateDnsCompileError::Compiler)
    }
}
