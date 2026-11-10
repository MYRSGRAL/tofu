//! Явный ограниченный профиль локального native DNS, без владения DNS ОС.
use std::net::SocketAddrV4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeDnsMode {
    Real,
    FakeIp,
}

/// Native срез: IPv4 UDP upstream с literal адресом, без persistent cache.
/// Поколения, полный набор A/AAAA и приватный split DNS сюда не входят.
#[derive(Debug, Clone, Copy)]
pub struct DnsPolicy {
    pub mode: NativeDnsMode,
    pub upstream: SocketAddrV4,
    pub listen_port: std::num::NonZeroU16,
}
