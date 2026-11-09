//! Linux host локального SOCKS. Не меняет системные маршруты, proxy или DNS.

#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
pub use host::*;
#[cfg(target_os = "linux")]
mod fetch;
#[cfg(target_os = "linux")]
pub use fetch::*;
#[cfg(target_os = "linux")]
mod credentials;
#[cfg(target_os = "linux")]
pub use credentials::*;
#[cfg(target_os = "linux")]
mod repository;
#[cfg(target_os = "linux")]
pub use repository::*;
#[cfg(target_os = "linux")]
mod subscriptions;
#[cfg(target_os = "linux")]
pub use subscriptions::*;
#[cfg(target_os = "linux")]
mod tcp_probe;
#[cfg(target_os = "linux")]
pub use tcp_probe::{
    TCP_SERIES_SAMPLES, TcpProbeResult, TcpProbeSeriesResult, TcpProbeStatus, TcpProbeTarget,
    check_tcp, check_tcp_series,
};
#[cfg(target_os = "linux")]
mod network;
#[cfg(target_os = "linux")]
pub use network::{
    IpPrefix, KernelRoute, LinuxAddress, LinuxLink, LinuxNexthop, LinuxRoute, LinuxRule,
    LinuxTopology, LookupIncomplete, NetworkAttribute, NetworkNamespace, NetworkReadError,
    NexthopMember, RouteKind, RouteNexthop, RouteQuery, TransportProtocol, lookup_route,
    read_topology,
};
#[cfg(target_os = "linux")]
mod resolver;
#[cfg(target_os = "linux")]
pub use network::{
    NetworkCoverage, NetworkMonitor, NetworkMonitorError, NetworkSnapshot, NetworkStamp,
    NetworkWatch,
};
#[cfg(target_os = "linux")]
pub use resolver::*;
