//! Read-only факты systemd-resolved. Не меняет DNS и не разрешает Apply.
//! Требуется Tokio runtime с включёнными I/O и time. Снимок описывает daemon,
//! а не системный resolver или namespace; peer credentials не дают права Apply.

mod config;
mod socket;
mod wire;

pub use config::*;
pub use socket::ResolverPeer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolverError {
    Unavailable,
    Unsupported,
    PermissionDenied,
    Cancelled,
    Timeout,
    Malformed,
    Limit,
    Closed,
    Untrusted,
    Exhausted,
    NetworkChanged,
}
impl std::fmt::Display for ResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Недоступен systemd-resolved",
            Self::Unsupported => "Не поддержан DNS configuration API",
            Self::PermissionDenied => "Нет права наблюдать DNS-конфигурацию",
            Self::Cancelled => "Чтение DNS отменено",
            Self::Timeout => "Истёк срок чтения DNS",
            Self::Malformed => "Повреждена DNS-конфигурация",
            Self::Limit => "Превышен лимит DNS-конфигурации",
            Self::Closed => "Источник DNS закрыт",
            Self::Untrusted => "Не подтверждён источник DNS",
            Self::Exhausted => "Исчерпано поколение DNS",
            Self::NetworkChanged => "Окружение изменилось во время чтения DNS",
        })
    }
}
impl std::error::Error for ResolverError {}

use crate::Cancellation;
use std::time::Duration;
use tokio::time::Instant;

/// Корреляция наблюдений одного соединения; не lease Prepare/Apply.
/// Каждый разовый снимок и каждая подписка имеют отдельный случайный instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolverStamp {
    instance: [u8; 16],
    sequence: u64,
}
impl ResolverStamp {
    pub fn sequence(self) -> u64 {
        self.sequence
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolverSnapshot {
    pub configuration: ResolverConfiguration,
    pub peer: ResolverPeer,
    pub stamp: ResolverStamp,
    pub streamed: bool,
}

pub struct ResolverSubscription {
    wire: wire::ResolverWire,
    last: ResolverSnapshot,
}
impl ResolverSubscription {
    /// Последний принятый frame, а не проверка текущей живости daemon.
    pub fn last_observation(&self) -> &ResolverSnapshot {
        &self.last
    }
    pub fn is_closed(&self) -> bool {
        self.wire.is_closed()
    }
    pub fn close(&mut self) {
        self.wire.close();
    }
    pub async fn next(
        &mut self,
        cancel: &mut Cancellation,
    ) -> Result<&ResolverSnapshot, ResolverError> {
        let sequence = match advance_sequence(self.last.stamp.sequence) {
            Ok(value) => value,
            Err(error) => {
                self.wire.close();
                return Err(error);
            }
        };
        let configuration = self.wire.next(cancel).await?;
        self.last.configuration = configuration;
        self.last.stamp.sequence = sequence;
        Ok(&self.last)
    }
}

/// Факты daemon, не подтверждение system resolver/namespace или права Apply.
pub async fn read_resolver_snapshot(
    cancel: &mut Cancellation,
    timeout: Duration,
) -> Result<ResolverSnapshot, ResolverError> {
    let deadline = startup_deadline(timeout)?;
    let connected = socket::connect(socket::Endpoint::Main, cancel, deadline).await?;
    let mut wire = wire::ResolverWire::new(connected.stream);
    let configuration = wire.snapshot(cancel, deadline).await?;
    Ok(ResolverSnapshot {
        configuration,
        peer: connected.peer,
        stamp: new_stamp()?,
        streamed: false,
    })
}
/// Подписка требует API и авторизации; интерактивная авторизация не запрашивается.
pub async fn subscribe_resolver(
    cancel: &mut Cancellation,
    timeout: Duration,
) -> Result<ResolverSubscription, ResolverError> {
    let deadline = startup_deadline(timeout)?;
    let connected = socket::connect(socket::Endpoint::Monitor, cancel, deadline).await?;
    subscribe_connected(connected, cancel, deadline).await
}
async fn subscribe_connected(
    connected: socket::Connected,
    cancel: &mut Cancellation,
    deadline: Instant,
) -> Result<ResolverSubscription, ResolverError> {
    let mut wire = wire::ResolverWire::new(connected.stream);
    let configuration = wire.subscribe(cancel, deadline).await?;
    let last = ResolverSnapshot {
        configuration,
        peer: connected.peer,
        stamp: new_stamp()?,
        streamed: true,
    };
    Ok(ResolverSubscription { wire, last })
}
fn advance_sequence(sequence: u64) -> Result<u64, ResolverError> {
    sequence.checked_add(1).ok_or(ResolverError::Exhausted)
}
fn startup_deadline(timeout: Duration) -> Result<Instant, ResolverError> {
    if timeout.is_zero() || timeout > Duration::from_secs(5) {
        return Err(ResolverError::Limit);
    }
    Ok(Instant::now() + timeout)
}
fn new_stamp() -> Result<ResolverStamp, ResolverError> {
    let mut instance = [0; 16];
    getrandom::getrandom(&mut instance).map_err(|_| ResolverError::Unavailable)?;
    Ok(ResolverStamp {
        instance,
        sequence: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::UnixStream,
    };
    #[tokio::test]
    async fn invalid_startup_budget_never_reaches_daemon() {
        let (_source, mut cancel) = Cancellation::new();
        for budget in [Duration::ZERO, Duration::from_secs(6)] {
            assert_eq!(
                read_resolver_snapshot(&mut cancel, budget).await.err(),
                Some(ResolverError::Limit)
            );
            assert_eq!(
                subscribe_resolver(&mut cancel, budget).await.err(),
                Some(ResolverError::Limit)
            );
        }
    }
    #[test]
    fn observation_counter_never_wraps() {
        assert_eq!(advance_sequence(0), Ok(1));
        assert_eq!(advance_sequence(u64::MAX), Err(ResolverError::Exhausted));
    }
    #[tokio::test]
    async fn subscription_preserves_source_advances_sequence_and_closes_on_cancel() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let task = tokio::spawn(async move {
            let mut request = Vec::new();
            let mut byte = [0];
            while server.read(&mut byte).await.unwrap() > 0 {
                request.push(byte[0]);
                if byte[0] == 0 {
                    break;
                }
            }
            let request: serde_json::Value =
                serde_json::from_slice(&request[..request.len() - 1]).unwrap();
            assert_eq!(
                request["method"],
                "io.systemd.Resolve.Monitor.SubscribeDNSConfiguration"
            );
            assert_eq!(
                request["parameters"]["allowInteractiveAuthentication"],
                false
            );
            server.write_all(b"{\"parameters\":{\"configuration\":[{\"resolvConfMode\":\"stub\"}]},\"continues\":true}\0{\"parameters\":{\"configuration\":[{\"resolvConfMode\":\"foreign\"}]},\"continues\":true}\0").await.unwrap();
            assert_eq!(
                server.read(&mut byte).await.unwrap(),
                0,
                "Клиент закрыл соединение"
            );
        });
        let peer = ResolverPeer {
            uid: rustix::process::getuid().as_raw(),
            pid: Some(std::process::id()),
            socket_device: 7,
            socket_inode: 9,
        };
        let (_source, mut cancel) = Cancellation::new();
        let mut subscription = subscribe_connected(
            socket::Connected {
                stream: client,
                peer,
            },
            &mut cancel,
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .expect("subscription");
        let first = subscription.last_observation().stamp;
        assert_eq!(first.sequence(), 0);
        let second = subscription.next(&mut cancel).await.unwrap();
        assert_eq!(second.stamp.sequence(), 1);
        assert_ne!(first, second.stamp);
        assert_eq!(second.peer, peer);
        assert!(second.streamed);
        assert_eq!(
            second.configuration.entries[0].resolv_conf_mode,
            Some(Supported::Known(GlobalModeKnown::Foreign))
        );
        let (source, mut cancel) = Cancellation::new();
        source.send_replace(true);
        assert_eq!(
            subscription.next(&mut cancel).await.err(),
            Some(ResolverError::Cancelled)
        );
        assert!(subscription.is_closed());
        task.await.unwrap();
    }
}
