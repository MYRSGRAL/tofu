//! Ограниченное read-only наблюдение D-Bus; значения свойств не сохраняются.

use std::collections::HashMap;

use futures_util::StreamExt;
use tokio::sync::watch;
use zbus::{Connection, MatchRule, Message, MessageStream, message::Type};

use super::{NetworkPublisher, NetworkSource};

const RESOLVER: &str = "org.freedesktop.resolve1";
const MANAGER: &str = "org.freedesktop.NetworkManager";
const MAX_BODY: usize = 256 * 1024;

fn relevant_property(service: &str, interface: &str, property: &str) -> bool {
    match (service, interface) {
        (RESOLVER, "org.freedesktop.resolve1.Manager" | "org.freedesktop.resolve1.Link") => {
            matches!(
                property,
                "DNS"
                    | "DNSEx"
                    | "FallbackDNS"
                    | "FallbackDNSEx"
                    | "Domains"
                    | "DefaultRoute"
                    | "CurrentDNSServer"
                    | "CurrentDNSServerEx"
                    | "DNSOverTLS"
                    | "DNSSEC"
                    | "DNSSECSupported"
                    | "LLMNR"
                    | "MulticastDNS"
                    | "ResolvConfMode"
            )
        }
        (MANAGER, "org.freedesktop.NetworkManager") => matches!(
            property,
            "ActiveConnections"
                | "PrimaryConnection"
                | "PrimaryConnectionType"
                | "ActivatingConnection"
                | "State"
                | "Connectivity"
                | "NetworkingEnabled"
                | "WirelessEnabled"
                | "WirelessHardwareEnabled"
                | "Devices"
                | "AllDevices"
        ),
        (MANAGER, "org.freedesktop.NetworkManager.Device") => matches!(
            property,
            "State"
                | "StateReason"
                | "Ip4Config"
                | "Ip6Config"
                | "Dhcp4Config"
                | "Dhcp6Config"
                | "ActiveConnection"
                | "Interface"
                | "IpInterface"
                | "Managed"
                | "Mtu"
        ),
        (MANAGER, "org.freedesktop.NetworkManager.Device.Wireless") => {
            matches!(property, "ActiveAccessPoint" | "WirelessCapabilities")
        }
        (MANAGER, "org.freedesktop.NetworkManager.Device.Wired") => property == "Carrier",
        (MANAGER, "org.freedesktop.NetworkManager.Connection.Active") => matches!(
            property,
            "State"
                | "StateFlags"
                | "Default"
                | "Default6"
                | "Ip4Config"
                | "Ip6Config"
                | "Dhcp4Config"
                | "Dhcp6Config"
                | "Devices"
                | "Vpn"
        ),
        (
            MANAGER,
            "org.freedesktop.NetworkManager.IP4Config" | "org.freedesktop.NetworkManager.IP6Config",
        ) => matches!(
            property,
            "Addresses"
                | "AddressData"
                | "Gateway"
                | "Routes"
                | "RouteData"
                | "Nameservers"
                | "NameserverData"
                | "Domains"
                | "Searches"
                | "DnsOptions"
                | "DnsPriority"
                | "WinsServers"
                | "WinsServerData"
        ),
        (
            MANAGER,
            "org.freedesktop.NetworkManager.DHCP4Config"
            | "org.freedesktop.NetworkManager.DHCP6Config",
        ) => property == "Options",
        (MANAGER, "org.freedesktop.NetworkManager.VPN.Connection") => property == "VpnState",
        (MANAGER, "org.freedesktop.NetworkManager.DnsManager") => {
            matches!(property, "Mode" | "RcManager" | "Configuration")
        }
        _ => false,
    }
}

struct Service {
    name: &'static str,
    source: NetworkSource,
    owner: Option<String>,
    observed: bool,
    initialized: bool,
}

impl Service {
    fn set_owner(&mut self, owner: &str, publisher: &NetworkPublisher) -> Result<(), ()> {
        if !owner.is_empty() && zbus::names::UniqueName::try_from(owner).is_err() {
            return Err(());
        }
        // Замена владельца означает разрыв наблюдения, даже без промежуточной пустой строки.
        publisher.lost(self.source);
        self.owner = (!owner.is_empty()).then(|| owner.to_owned());
        if self.initialized && self.owner.is_some() {
            publisher.ready(self.source);
        }
        Ok(())
    }

    fn properties(&self, message: &Message, publisher: &NetworkPublisher) -> Result<(), ()> {
        let header = message.header();
        if header.sender().map(|sender| sender.as_str()) != self.owner.as_deref()
            || self.owner.is_none()
        {
            return Ok(());
        }
        let prefix = if self.name == RESOLVER {
            "/org/freedesktop/resolve1"
        } else {
            "/org/freedesktop/NetworkManager"
        };
        let Some(path) = header.path() else {
            return Ok(());
        };
        if path.as_str() != prefix
            && !path
                .as_str()
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
        {
            return Ok(());
        }
        let body = message.body();
        if body.len() > MAX_BODY {
            return Err(());
        }
        let (interface, changed, invalidated): (
            &str,
            HashMap<&str, zbus::zvariant::Value<'_>>,
            Vec<&str>,
        ) = body.deserialize().map_err(|_| ())?;
        if changed
            .keys()
            .chain(invalidated.iter())
            .any(|property| relevant_property(self.name, interface, property))
        {
            publisher.changed(self.source);
        }
        Ok(())
    }
}

fn owners(
    message: &Message,
    services: &mut [Service; 2],
    publisher: &NetworkPublisher,
) -> Result<(), ()> {
    let header = message.header();
    if header.sender().map(|sender| sender.as_str()) != Some("org.freedesktop.DBus") {
        return Ok(());
    }
    let body = message.body();
    if body.len() > 4096 {
        return Err(());
    }
    let (name, _old, new): (&str, &str, &str) = body.deserialize().map_err(|_| ())?;
    if let Some(service) = services.iter_mut().find(|service| service.name == name) {
        service.observed = true;
        service.set_owner(new, publisher)?;
    }
    Ok(())
}

fn signal_rule(interface: &str, member: &str) -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(interface.to_owned())?
        .member(member.to_owned())?
        .build())
}

pub(super) async fn run(publisher: NetworkPublisher, mut cancel: watch::Receiver<bool>) {
    if *cancel.borrow() {
        return;
    }
    // Отмена охватывает аутентификацию, AddMatch и начальные запросы владельцев.
    let connection = tokio::select! {
        _ = cancel.changed() => return,
        result = Connection::system() => result,
    };
    if let Ok(connection) = connection {
        listen(connection, publisher, cancel).await;
    } else {
        publisher.lost(NetworkSource::Resolver);
        publisher.lost(NetworkSource::Manager);
    }
}

async fn listen(
    connection: Connection,
    publisher: NetworkPublisher,
    mut cancel: watch::Receiver<bool>,
) {
    if !*cancel.borrow() {
        tokio::select! {
            _ = cancel.changed() => {},
            _ = observe(&connection, &publisher) => {},
        }
    }
    publisher.lost(NetworkSource::Resolver);
    publisher.lost(NetworkSource::Manager);
}

async fn observe(connection: &Connection, publisher: &NetworkPublisher) -> Result<(), ()> {
    let mut services = [
        Service {
            name: RESOLVER,
            source: NetworkSource::Resolver,
            owner: None,
            observed: false,
            initialized: false,
        },
        Service {
            name: MANAGER,
            source: NetworkSource::Manager,
            owner: None,
            observed: false,
            initialized: false,
        },
    ];
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| ())?
        .path("/org/freedesktop/DBus")
        .map_err(|_| ())?
        .interface("org.freedesktop.DBus")
        .map_err(|_| ())?
        .member("NameOwnerChanged")
        .map_err(|_| ())?
        .build();
    // zbus 4.4 socket_reader применяет broadcast_direct(...).await:
    // заполнение очереди останавливает чтение, а не теряет сигнал молча.
    let mut owner_stream = MessageStream::for_match_rule(owner_rule, connection, Some(64))
        .await
        .map_err(|_| ())?;
    let properties_rule =
        signal_rule("org.freedesktop.DBus.Properties", "PropertiesChanged").map_err(|_| ())?;
    let subscribe = MessageStream::for_match_rule(properties_rule, connection, Some(128));
    tokio::pin!(subscribe);
    let mut properties_stream = loop {
        tokio::select! {
            next = owner_stream.next() => owners(&next.ok_or(())?.map_err(|_| ())?, &mut services, publisher)?,
            result = &mut subscribe => break result.map_err(|_| ())?,
        }
    };
    let query = async {
        let proxy = zbus::fdo::DBusProxy::new(connection).await?;
        let resolver = proxy.get_name_owner(RESOLVER.try_into()?);
        let manager = proxy.get_name_owner(MANAGER.try_into()?);
        Ok::<_, zbus::Error>(futures_util::future::join(resolver, manager).await)
    };
    tokio::pin!(query);
    loop {
        tokio::select! {
            next = owner_stream.next() => owners(&next.ok_or(())?.map_err(|_| ())?, &mut services, publisher)?,
            next = properties_stream.next() => {
                let message = next.ok_or(())?.map_err(|_| ())?;
                for service in &services { service.properties(&message, publisher)?; }
            }
            result = &mut query => {
                let (resolver, manager) = result.map_err(|_| ())?;
                for (service, result) in services.iter_mut().zip([resolver, manager]) {
                    // Подписка установлена до GetNameOwner. Уже увиденный сигнал новее
                    // начального снимка либо содержит то же текущее состояние.
                    if service.observed { continue; }
                    match result {
                        Ok(owner) => service.set_owner(owner.as_str(), publisher)?,
                        Err(zbus::fdo::Error::NameHasNoOwner(_)) => publisher.lost(service.source),
                        Err(_) => return Err(()),
                    }
                }
                for service in &mut services {
                    service.initialized = true;
                    if service.owner.is_some() { publisher.ready(service.source); }
                }
                break;
            }
        }
    }
    loop {
        tokio::select! {
            next = owner_stream.next() => owners(&next.ok_or(())?.map_err(|_| ())?, &mut services, publisher)?,
            next = properties_stream.next() => {
                let message = next.ok_or(())?.map_err(|_| ())?;
                for service in &services { service.properties(&message, publisher)?; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_dns_properties_and_invalidations_are_relevant() {
        for property in [
            "DNS",
            "DNSEx",
            "Domains",
            "DefaultRoute",
            "CurrentDNSServer",
            "DNSOverTLS",
            "DNSSEC",
            "LLMNR",
            "MulticastDNS",
            "ResolvConfMode",
        ] {
            assert!(
                relevant_property(RESOLVER, "org.freedesktop.resolve1.Link", property),
                "{property}"
            );
        }
        assert!(!relevant_property(
            RESOLVER,
            "org.freedesktop.resolve1.Manager",
            "CacheStatistics"
        ));
    }

    #[test]
    fn manager_tracks_configuration_but_ignores_strength_and_statistics() {
        for (interface, property) in [
            ("org.freedesktop.NetworkManager", "ActiveConnections"),
            ("org.freedesktop.NetworkManager.Device", "Ip4Config"),
            (
                "org.freedesktop.NetworkManager.Device.Wireless",
                "ActiveAccessPoint",
            ),
            ("org.freedesktop.NetworkManager.IP4Config", "NameserverData"),
            ("org.freedesktop.NetworkManager.IP6Config", "RouteData"),
            ("org.freedesktop.NetworkManager.Connection.Active", "State"),
        ] {
            assert!(
                relevant_property(MANAGER, interface, property),
                "{property}"
            );
        }
        assert!(!relevant_property(
            MANAGER,
            "org.freedesktop.NetworkManager.AccessPoint",
            "Strength"
        ));
        assert!(!relevant_property(
            MANAGER,
            "org.freedesktop.NetworkManager.Device.Statistics",
            "TxBytes"
        ));
        assert!(!relevant_property(
            MANAGER,
            "org.freedesktop.NetworkManager.Impostor",
            "State"
        ));
    }

    fn publisher() -> (
        NetworkPublisher,
        watch::Receiver<super::super::NetworkSnapshot>,
    ) {
        let (sender, receiver) = watch::channel(super::super::NetworkSnapshot {
            stamp: super::super::NetworkStamp {
                instance: [1; 16],
                generation: 0,
            },
            coverage: super::super::NetworkCoverage::default(),
            exhausted: false,
        });
        (NetworkPublisher(sender), receiver)
    }

    fn service() -> Service {
        Service {
            name: RESOLVER,
            source: NetworkSource::Resolver,
            owner: Some(":1.42".to_owned()),
            observed: false,
            initialized: true,
        }
    }

    fn properties(sender: &str, invalidated: &[&str]) -> Message {
        Message::signal(
            "/org/freedesktop/resolve1/link/_2",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .unwrap()
        .sender(sender)
        .unwrap()
        .build(&(
            "org.freedesktop.resolve1.Link",
            HashMap::<&str, zbus::zvariant::Value<'_>>::new(),
            invalidated,
        ))
        .unwrap()
    }

    #[test]
    fn invalidated_properties_invalidate_and_foreign_senders_do_not() {
        let (publisher, receiver) = publisher();
        let service = service();
        publisher.ready(NetworkSource::Resolver);
        let before = receiver.borrow().stamp;
        service
            .properties(&properties(":1.666", &["DNS"]), &publisher)
            .unwrap();
        assert_eq!(receiver.borrow().stamp, before);
        service
            .properties(&properties(":1.42", &["CacheStatistics"]), &publisher)
            .unwrap();
        assert_eq!(receiver.borrow().stamp, before);
        service
            .properties(&properties(":1.42", &["DNS"]), &publisher)
            .unwrap();
        assert_ne!(receiver.borrow().stamp, before);
    }

    #[test]
    fn malformed_or_oversized_properties_are_loss_before_deserialization() {
        let (publisher, _) = publisher();
        let malformed = Message::signal(
            "/org/freedesktop/resolve1",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .unwrap()
        .sender(":1.42")
        .unwrap()
        .build(&42u32)
        .unwrap();
        assert!(service().properties(&malformed, &publisher).is_err());
        let oversized = Message::signal(
            "/org/freedesktop/resolve1",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .unwrap()
        .sender(":1.42")
        .unwrap()
        .build(&vec![0u8; MAX_BODY + 1])
        .unwrap();
        assert!(service().properties(&oversized, &publisher).is_err());
    }

    #[test]
    fn owner_discovered_before_subscriptions_ready_does_not_claim_coverage() {
        let (publisher, receiver) = publisher();
        let mut service = service();
        service.initialized = false;
        service.set_owner(":1.43", &publisher).unwrap();
        assert!(!receiver.borrow().coverage.resolver);
    }

    #[test]
    fn ownership_replacement_and_disappearance_invalidate() {
        let (publisher, receiver) = publisher();
        let mut service = service();
        publisher.ready(NetworkSource::Resolver);
        let before = receiver.borrow().stamp;
        service.set_owner(":1.43", &publisher).unwrap();
        assert!(receiver.borrow().coverage.resolver);
        assert_ne!(receiver.borrow().stamp, before);
        let replacement = receiver.borrow().stamp;
        service.set_owner("", &publisher).unwrap();
        assert!(!receiver.borrow().coverage.resolver);
        assert_ne!(receiver.borrow().stamp, replacement);
        assert!(
            service
                .set_owner("org.impostor.Service", &publisher)
                .is_err()
        );
    }

    #[test]
    fn only_bus_daemon_can_change_service_owner_and_malformed_owner_is_loss() {
        let (publisher, receiver) = publisher();
        let mut services = [
            service(),
            Service {
                name: MANAGER,
                source: NetworkSource::Manager,
                owner: None,
                observed: false,
                initialized: true,
            },
        ];
        let signal = |sender: &str| {
            Message::signal(
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameOwnerChanged",
            )
            .unwrap()
            .sender(sender)
            .unwrap()
            .build(&(RESOLVER, ":1.42", ":1.43"))
            .unwrap()
        };
        owners(&signal(":1.666"), &mut services, &publisher).unwrap();
        assert_eq!(services[0].owner.as_deref(), Some(":1.42"));
        owners(&signal("org.freedesktop.DBus"), &mut services, &publisher).unwrap();
        assert_eq!(services[0].owner.as_deref(), Some(":1.43"));
        assert!(receiver.borrow().coverage.resolver);
        let malformed = Message::signal(
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
        )
        .unwrap()
        .sender("org.freedesktop.DBus")
        .unwrap()
        .build(&42u32)
        .unwrap();
        assert!(owners(&malformed, &mut services, &publisher).is_err());
    }

    struct PrivateBus {
        child: std::process::Child,
        address: String,
    }
    impl PrivateBus {
        fn start() -> Self {
            use std::io::BufRead;
            let mut child = std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut address = String::new();
            std::io::BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            Self {
                child,
                address: address.trim().to_owned(),
            }
        }
        async fn connect(&self) -> Connection {
            zbus::connection::Builder::address(self.address.as_str())
                .unwrap()
                .build()
                .await
                .unwrap()
        }
    }
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    async fn until(
        receiver: &mut watch::Receiver<super::super::NetworkSnapshot>,
        condition: impl Fn(super::super::NetworkSnapshot) -> bool,
    ) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if condition(*receiver.borrow()) {
                    break;
                }
                receiver.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn private_bus_discovers_services_recovers_loses_and_cancels() {
        let bus = PrivateBus::start();
        let connection = bus.connect().await;
        let resolver = bus.connect().await;
        let manager = bus.connect().await;
        // Один владелец существует до подписки; второй появляется во время startup.
        resolver.request_name(RESOLVER).await.unwrap();
        let (publisher, mut receiver) = publisher();
        let (cancel, cancelled) = watch::channel(false);
        let task = tokio::spawn(listen(connection, publisher, cancelled));
        manager.request_name(MANAGER).await.unwrap();
        until(&mut receiver, |state| {
            state.coverage.resolver && state.coverage.manager
        })
        .await;
        let before = receiver.borrow().stamp;
        resolver
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/resolve1",
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(
                    "org.freedesktop.resolve1.Manager",
                    HashMap::<&str, zbus::zvariant::Value<'_>>::new(),
                    vec!["DNS"],
                ),
            )
            .await
            .unwrap();
        until(&mut receiver, |state| state.stamp != before).await;
        resolver.release_name(RESOLVER).await.unwrap();
        until(&mut receiver, |state| !state.coverage.resolver).await;
        resolver.request_name(RESOLVER).await.unwrap();
        until(&mut receiver, |state| state.coverage.resolver).await;
        cancel.send_replace(true);
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(!receiver.borrow().coverage.resolver);
        assert!(!receiver.borrow().coverage.manager);
    }

    #[tokio::test]
    async fn cancellation_before_startup_closes_without_claiming_sources() {
        let bus = PrivateBus::start();
        let connection = bus.connect().await;
        let (publisher, receiver) = publisher();
        let (_cancel, cancelled) = watch::channel(true);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            listen(connection, publisher, cancelled),
        )
        .await
        .unwrap();
        assert!(!receiver.borrow().coverage.resolver);
        assert!(!receiver.borrow().coverage.manager);
    }

    #[tokio::test]
    async fn malformed_current_owner_message_terminates_and_revokes_coverage() {
        let bus = PrivateBus::start();
        let connection = bus.connect().await;
        let resolver = bus.connect().await;
        resolver.request_name(RESOLVER).await.unwrap();
        let (publisher, mut receiver) = publisher();
        let (_cancel, cancelled) = watch::channel(false);
        let task = tokio::spawn(listen(connection, publisher, cancelled));
        until(&mut receiver, |state| state.coverage.resolver).await;
        resolver
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/resolve1",
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &42u32,
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(!receiver.borrow().coverage.resolver);
        assert!(!receiver.borrow().coverage.manager);
    }

    #[tokio::test]
    async fn private_bus_disconnect_invalidates_ready_sources() {
        let mut bus = PrivateBus::start();
        let connection = bus.connect().await;
        let resolver = bus.connect().await;
        resolver.request_name(RESOLVER).await.unwrap();
        let (publisher, mut receiver) = publisher();
        let (_cancel, cancelled) = watch::channel(false);
        let task = tokio::spawn(listen(connection, publisher, cancelled));
        until(&mut receiver, |state| state.coverage.resolver).await;
        bus.child.kill().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(!receiver.borrow().coverage.resolver);
    }
}
