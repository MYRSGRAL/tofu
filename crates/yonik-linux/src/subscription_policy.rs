//! Событийная политика допуска автоматических обновлений.

use crate::SubscriptionPolicy;
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::watch;
use zbus::{Connection, MatchRule, MessageStream, message::Type};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const UP: &str = "org.freedesktop.UPower";
const UP_PATH: &str = "/org/freedesktop/UPower";
const PP: &str = "org.freedesktop.UPower.PowerProfiles";
const PP_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
const SERVICES: [(&str, &str); 3] = [(NM, NM_PATH), (UP, UP_PATH), (PP, PP_PATH)];
type Properties = HashMap<String, zbus::zvariant::OwnedValue>;
type Owners = HashMap<&'static str, String>;

fn retry_delay(previous: Duration, healthy_for: Option<Duration>) -> Duration {
    if healthy_for.is_some_and(|duration| duration >= Duration::from_secs(30)) {
        Duration::from_secs(1)
    } else {
        previous
    }
}

fn policy(
    state: Option<u32>,
    metered: Option<u32>,
    battery: Option<bool>,
    profile: Option<&str>,
) -> SubscriptionPolicy {
    // NMState::CONNECTED_GLOBAL=70; NMMetered::NO=2, GUESS_NO=4.
    let unmetered = matches!(metered, Some(2 | 4));
    let known_power =
        battery.is_some() && matches!(profile, Some("balanced" | "performance" | "power-saver"));
    SubscriptionPolicy {
        online: state == Some(70) && unmetered && known_power,
        metered: !unmetered,
        battery_saver: !known_power || profile == Some("power-saver"),
        ..Default::default()
    }
}

/// Один future на desktop host, без таймеров и отдельных tasks на источник.
/// Повторное соединение имеет backoff только после ошибки транспорта;
/// исправная шина не создаёт таймеров и не опрашивает свойства.
pub async fn system_subscription_policy(sender: watch::Sender<SubscriptionPolicy>) {
    let (_keep, shutdown) = crate::Cancellation::new();
    let _ = system_subscription_policy_until(sender, shutdown).await;
}

/// Отмена дожидается закрытия transport; ошибка не является cleanup acknowledgement.
pub async fn system_subscription_policy_until(
    sender: watch::Sender<SubscriptionPolicy>,
    mut shutdown: crate::Cancellation,
) -> zbus::Result<()> {
    let mut delay = std::time::Duration::from_secs(1);
    loop {
        sender.send_replace(Default::default());
        let connection = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return Ok(()),
            connection = Connection::system() => connection,
        };
        let mut healthy_since = None;
        if let Ok(connection) = connection {
            if run_until(
                connection,
                sender.clone(),
                &mut healthy_since,
                &mut shutdown,
            )
            .await?
            {
                return Ok(());
            }
        }
        sender.send_replace(Default::default());
        delay = retry_delay(delay, healthy_since.map(|since| since.elapsed()));
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return Ok(()),
            _ = tokio::time::sleep(delay) => {},
        }
        delay = (delay * 2).min(std::time::Duration::from_secs(30));
    }
}

async fn snapshot(
    connection: &Connection,
    owners: &Mutex<Owners>,
) -> zbus::Result<SubscriptionPolicy> {
    let bus = zbus::fdo::DBusProxy::new(connection).await?;
    let mut maps = Vec::with_capacity(3);
    for (name, path) in SERVICES {
        // Уникальный адрес исключает ответ нового владельца в старой эпохе.
        let owner = bus.get_name_owner(name.try_into()?).await?;
        owners
            .lock()
            .map_err(|_| zbus::Error::Failure("Таблица владельцев недоступна".into()))?
            .insert(name, owner.as_str().to_owned());
        let proxy = zbus::Proxy::new(
            connection,
            owner.as_str(),
            path,
            "org.freedesktop.DBus.Properties",
        )
        .await?;
        let properties: Properties = proxy.call("GetAll", &(name,)).await?;
        maps.push(properties);
    }
    let state = maps[0].get("State").and_then(|v| u32::try_from(v).ok());
    let metered = maps[0].get("Metered").and_then(|v| u32::try_from(v).ok());
    let battery = maps[1]
        .get("OnBattery")
        .and_then(|v| bool::try_from(v).ok());
    let profile = maps[2]
        .get("ActiveProfile")
        .and_then(|v| <&str>::try_from(v).ok());
    Ok(policy(state, metered, battery, profile))
}

fn relevant(message: &zbus::Message, owners: &Owners) -> bool {
    let header = message.header();
    if header.message_type() != Type::Signal {
        return false;
    }
    match header.member().map(|v| v.as_str()) {
        Some("NameOwnerChanged")
            if header
                .sender()
                .is_some_and(|s| s.as_str() == "org.freedesktop.DBus")
                && header
                    .interface()
                    .is_some_and(|i| i.as_str() == "org.freedesktop.DBus")
                && header
                    .path()
                    .is_some_and(|p| p.as_str() == "/org/freedesktop/DBus") =>
        {
            message
                .body()
                .deserialize::<(String, String, String)>()
                .is_ok_and(|(name, _, _)| SERVICES.iter().any(|(n, _)| *n == name))
        }
        Some("PropertiesChanged")
            if header
                .interface()
                .is_some_and(|i| i.as_str() == "org.freedesktop.DBus.Properties") =>
        {
            let Ok((interface, _, _)) = message
                .body()
                .deserialize::<(String, Properties, Vec<String>)>()
            else {
                return false;
            };
            SERVICES.iter().any(|(name, path)| {
                interface == *name
                    && header.path().is_some_and(|p| p.as_str() == *path)
                    && header.sender().is_some_and(|sender| {
                        owners
                            .get(name)
                            .is_some_and(|owner| owner == sender.as_str())
                    })
            })
        }
        _ => false,
    }
}

async fn run_until(
    connection: Connection,
    sender: watch::Sender<SubscriptionPolicy>,
    healthy_since: &mut Option<Instant>,
    shutdown: &mut crate::Cancellation,
) -> zbus::Result<bool> {
    let stopped = tokio::select! {
        biased;
        _ = shutdown.cancelled() => true,
        _ = run_tracked(connection.clone(), sender.clone(), healthy_since) => false,
    };
    sender.send_replace(Default::default());
    // Close закрывает socket даже при clones в отложенных RemoveMatch tasks.
    let closed = tokio::time::timeout(Duration::from_secs(3), connection.close())
        .await
        .map_err(|_| zbus::Error::Failure("Timeout закрытия policy transport".into()))
        .and_then(|result| result);
    if stopped {
        closed?;
    }
    Ok(stopped)
}

async fn run_tracked(
    connection: Connection,
    sender: watch::Sender<SubscriptionPolicy>,
    healthy_since: &mut Option<Instant>,
) -> zbus::Result<()> {
    // Одна bounded очередь сохраняет порядок owner/property событий. Подписка
    // завершена до снимка; любое событие во время снимка отменяет весь снимок.
    let rule = MatchRule::builder().msg_type(Type::Signal).build();
    let mut events = MessageStream::for_match_rule(rule, &connection, Some(64)).await?;
    loop {
        sender.send_replace(Default::default());
        let owners = Mutex::new(Owners::new());
        let query = snapshot(&connection, &owners);
        tokio::pin!(query);
        let result = loop {
            tokio::select! {
                biased;
                event = events.next() => {
                    let event = event.ok_or(zbus::Error::Failure("Шина закрыта".into()))??;
                    if relevant(&event, &*owners.lock().map_err(|_| zbus::Error::Failure("Таблица владельцев недоступна".into()))?) { break None; }
                }
                result = &mut query => break Some(result),
            }
        };
        let Some(result) = result else {
            continue;
        };
        if result.is_ok() {
            healthy_since.get_or_insert_with(Instant::now);
        } else {
            *healthy_since = None;
        }
        sender.send_replace(result.unwrap_or_default());
        loop {
            let event = events
                .next()
                .await
                .ok_or(zbus::Error::Failure("Шина закрыта".into()))??;
            if relevant(
                &event,
                &*owners
                    .lock()
                    .map_err(|_| zbus::Error::Failure("Таблица владельцев недоступна".into()))?,
            ) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn посторонний_sender_не_инвалидирует_снимок() {
        let owners = Owners::from([(NM, ":1.7".into())]);
        for (sender, interface, path, payload, expected) in [
            (
                ":1.99",
                "org.freedesktop.DBus.Properties",
                NM_PATH,
                NM,
                false,
            ),
            (":1.7", "org.example.Other", NM_PATH, NM, false),
            (
                ":1.7",
                "org.freedesktop.DBus.Properties",
                UP_PATH,
                NM,
                false,
            ),
            (
                ":1.7",
                "org.freedesktop.DBus.Properties",
                NM_PATH,
                UP,
                false,
            ),
            (":1.7", "org.freedesktop.DBus.Properties", NM_PATH, NM, true),
        ] {
            let message = zbus::Message::signal(path, interface, "PropertiesChanged")
                .unwrap()
                .sender(sender)
                .unwrap()
                .build(&(payload, Properties::new(), Vec::<String>::new()))
                .unwrap();
            assert_eq!(relevant(&message, &owners), expected);
        }
    }

    #[test]
    fn backoff_сбрасывается_только_после_здоровой_работы() {
        use std::time::Duration;
        let mut delay = Duration::from_secs(1);
        for expected in [1, 2, 4, 8, 16, 30, 30] {
            // Успешное подключение без успешного снимка не является восстановлением.
            let wait = retry_delay(delay, None);
            assert_eq!(wait, Duration::from_secs(expected));
            delay = (wait * 2).min(Duration::from_secs(30));
        }
        assert_eq!(retry_delay(delay, Some(Duration::from_secs(29))), delay);
        assert_eq!(
            retry_delay(delay, Some(Duration::from_secs(30))),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn допускается_только_подтвержденная_политика() {
        assert!(policy(Some(70), Some(2), Some(false), Some("balanced")).online);
        for metered in [None, Some(0), Some(1), Some(3), Some(99)] {
            assert!(!policy(Some(70), metered, Some(false), Some("balanced")).online);
        }
        assert!(!policy(Some(70), Some(2), None, Some("balanced")).online);
        assert!(!policy(Some(70), Some(2), Some(false), None).online);
        assert!(policy(Some(70), Some(2), Some(false), Some("power-saver")).battery_saver);
        for current in [
            policy(Some(70), Some(2), Some(false), Some("balanced")),
            policy(Some(70), Some(2), Some(true), Some("power-saver")),
            policy(None, Some(2), Some(false), Some("balanced")),
            policy(Some(70), None, Some(false), Some("balanced")),
        ] {
            let mut scheduler = yonik_subscription::scheduler::Scheduler::new(
                std::num::NonZeroU64::new(1).unwrap(),
                1,
                1,
            )
            .unwrap();
            scheduler.schedule(1, 1, std::time::Duration::ZERO).unwrap();
            let visible = SubscriptionPolicy {
                ui_visible: true,
                ..current
            };
            assert_eq!(
                scheduler
                    .take_due(std::time::Duration::ZERO, visible)
                    .unwrap()
                    .is_some(),
                current.online && !current.battery_saver
            );
        }
    }
}

#[cfg(test)]
mod bus_tests {
    use super::*;
    use std::io::BufRead;

    struct Nm {
        metered: std::sync::Arc<std::sync::atomic::AtomicU32>,
        reached: std::sync::Arc<tokio::sync::Notify>,
        release: std::sync::Arc<tokio::sync::Notify>,
        first: std::sync::atomic::AtomicBool,
    }
    #[zbus::interface(name = "org.freedesktop.NetworkManager")]
    impl Nm {
        #[zbus(property)]
        fn state(&self) -> u32 {
            70
        }
        #[zbus(property)]
        async fn metered(&self) -> u32 {
            let value = self.metered.load(std::sync::atomic::Ordering::SeqCst);
            if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                self.reached.notify_one();
                self.release.notified().await;
            }
            value
        }
    }
    struct Power;
    #[zbus::interface(name = "org.freedesktop.UPower")]
    impl Power {
        #[zbus(property)]
        fn on_battery(&self) -> bool {
            false
        }
    }
    struct Profiles(std::sync::Arc<std::sync::atomic::AtomicBool>);
    #[zbus::interface(name = "org.freedesktop.UPower.PowerProfiles")]
    impl Profiles {
        #[zbus(property)]
        fn active_profile(&self) -> &str {
            if self.0.load(std::sync::atomic::Ordering::SeqCst) {
                "power-saver"
            } else {
                "balanced"
            }
        }
    }
    struct Bus(std::process::Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[tokio::test]
    async fn снимок_потеря_владельца_и_завершение() {
        let mut bus = Bus(std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap());
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let metered = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(2));
        let saver = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reached = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let server = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .name(NM)
            .unwrap()
            .name(UP)
            .unwrap()
            .name(PP)
            .unwrap()
            .serve_at(
                NM_PATH,
                Nm {
                    metered: metered.clone(),
                    reached: reached.clone(),
                    release: release.clone(),
                    first: std::sync::atomic::AtomicBool::new(true),
                },
            )
            .unwrap()
            .serve_at(UP_PATH, Power)
            .unwrap()
            .serve_at(PP_PATH, Profiles(saver.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let client_name = client.unique_name().unwrap().to_owned();
        // Такой clone моделирует удержание соединения отложенной cleanup task.
        let retained = client.clone();
        let (sender, mut receiver) = tokio::sync::watch::channel(SubscriptionPolicy::default());
        let (stop, mut shutdown) = crate::Cancellation::new();
        let task =
            tokio::spawn(async move { run_until(client, sender, &mut None, &mut shutdown).await });
        tokio::time::timeout(std::time::Duration::from_secs(5), reached.notified())
            .await
            .unwrap();
        metered.store(1, std::sync::atomic::Ordering::SeqCst);
        server
            .emit_signal(
                None::<&str>,
                NM_PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(
                    NM,
                    HashMap::<String, zbus::zvariant::Value<'_>>::new(),
                    vec!["Metered"],
                ),
            )
            .await
            .unwrap();
        release.notify_one();
        // Старый GetAll вернёт unmetered после уже отправленного изменения.
        // Ни один наблюдаемый снимок не должен разрешить dispatch.
        let observation = tokio::time::timeout(std::time::Duration::from_millis(100), async {
            loop {
                receiver.changed().await.unwrap();
                assert!(!receiver.borrow().online);
            }
        })
        .await;
        assert!(observation.is_err());
        metered.store(2, std::sync::atomic::Ordering::SeqCst);
        server
            .emit_signal(
                None::<&str>,
                NM_PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(
                    NM,
                    HashMap::<String, zbus::zvariant::Value<'_>>::new(),
                    vec!["Metered"],
                ),
            )
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !receiver.borrow().online {
                receiver.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        receiver.borrow_and_update();
        let stranger = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        stranger
            .emit_signal(
                None::<&str>,
                NM_PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(NM, Properties::new(), vec!["Metered"]),
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), receiver.changed())
                .await
                .is_err(),
            "чужой sender не публикует даже временный unknown"
        );
        for value in [1, 0, 2] {
            metered.store(value, std::sync::atomic::Ordering::SeqCst);
            server
                .emit_signal(
                    None::<&str>,
                    NM_PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(
                        NM,
                        HashMap::<String, zbus::zvariant::Value<'_>>::new(),
                        vec!["Metered"],
                    ),
                )
                .await
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    receiver.changed().await.unwrap();
                    let current = *receiver.borrow_and_update();
                    if current.online == (value == 2) {
                        let mut scheduler = yonik_subscription::scheduler::Scheduler::new(
                            std::num::NonZeroU64::new(1).unwrap(),
                            1,
                            1,
                        )
                        .unwrap();
                        scheduler.schedule(1, 1, std::time::Duration::ZERO).unwrap();
                        let visible = SubscriptionPolicy {
                            ui_visible: true,
                            ..current
                        };
                        assert_eq!(
                            scheduler
                                .take_due(std::time::Duration::ZERO, visible)
                                .unwrap()
                                .is_some(),
                            value == 2
                        );
                        break;
                    }
                }
            })
            .await
            .unwrap();
        }
        for saving in [true, false] {
            saver.store(saving, std::sync::atomic::Ordering::SeqCst);
            server
                .emit_signal(
                    None::<&str>,
                    PP_PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(
                        PP,
                        HashMap::<String, zbus::zvariant::Value<'_>>::new(),
                        vec!["ActiveProfile"],
                    ),
                )
                .await
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    receiver.changed().await.unwrap();
                    let current = *receiver.borrow_and_update();
                    if current.online && current.battery_saver == saving {
                        let mut scheduler = yonik_subscription::scheduler::Scheduler::new(
                            std::num::NonZeroU64::new(1).unwrap(),
                            1,
                            1,
                        )
                        .unwrap();
                        scheduler.schedule(1, 1, std::time::Duration::ZERO).unwrap();
                        assert_eq!(
                            scheduler
                                .take_due(
                                    std::time::Duration::ZERO,
                                    SubscriptionPolicy {
                                        ui_visible: true,
                                        ..current
                                    }
                                )
                                .unwrap()
                                .is_some(),
                            !saving
                        );
                        break;
                    }
                }
            })
            .await
            .unwrap();
        }
        server.release_name(NM).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while receiver.borrow().online {
                receiver.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        server.request_name(NM).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !receiver.borrow().online {
                receiver.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        stop.send_replace(true);
        assert!(task.await.unwrap().unwrap());
        let bus_proxy = zbus::fdo::DBusProxy::new(&server).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while bus_proxy
                .name_has_owner(client_name.clone().into())
                .await
                .unwrap()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(retained);
    }
}
