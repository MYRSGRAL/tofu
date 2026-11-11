//! Явный ограниченный профиль локального native DNS, без владения DNS ОС.
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeDnsMode {
    Real,
    FakeIp,
}

/// Локальный UDP resolver. Требования lease и exclusions проверяются compiler.
/// Выбор одного семейства не обещает full-family lease или владение DNS ОС.
#[derive(Debug, Clone)]
pub struct DnsPolicy {
    pub mode: NativeDnsMode,
    pub upstream: SocketAddr,
    /// Разрешённое семейство DNS-ответов; это не покрытие TUN/потоков.
    pub ipv4: bool,
    pub ipv6: bool,
    /// Исключения нельзя молча потерять или отправить публичному resolver.
    pub fake_exclusions: Vec<crate::DomainPattern>,
    /// Требуются TTL, epochs, отказ старых/неизвестных mapping и reset сети.
    pub require_mapping_lease: bool,
    pub listen_port: std::num::NonZeroU16,
}
