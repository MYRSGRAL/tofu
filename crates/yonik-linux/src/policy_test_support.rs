//! Только feature test-support: fake services используют тот же zbus, что provider.
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};
use tokio::sync::Notify;
use zbus::{Connection, zvariant::Value};
const NM: &str = "org.freedesktop.NetworkManager";
const UP: &str = "org.freedesktop.UPower";
const PP: &str = "org.freedesktop.UPower.PowerProfiles";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const UP_PATH: &str = "/org/freedesktop/UPower";
const PP_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
struct Nm {
    metered: Arc<AtomicU32>,
    hold: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Nm {
    #[zbus(property)]
    fn state(&self) -> u32 {
        70
    }
    #[zbus(property)]
    async fn metered(&self) -> u32 {
        if self.hold.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.metered.load(Ordering::SeqCst)
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
struct Profiles(Arc<AtomicBool>);
#[zbus::interface(name = "org.freedesktop.UPower.PowerProfiles")]
impl Profiles {
    #[zbus(property)]
    fn active_profile(&self) -> &str {
        if self.0.load(Ordering::SeqCst) {
            "power-saver"
        } else {
            "balanced"
        }
    }
}
pub struct PolicyBus {
    connection: Connection,
    metered: Arc<AtomicU32>,
    saver: Arc<AtomicBool>,
    hold: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
impl PolicyBus {
    pub async fn new(address: &str) -> zbus::Result<Self> {
        let metered = Arc::new(AtomicU32::new(2));
        let saver = Arc::new(AtomicBool::new(false));
        let hold = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let connection = zbus::connection::Builder::address(address)?
            .name(NM)?
            .name(UP)?
            .name(PP)?
            .serve_at(
                NM_PATH,
                Nm {
                    metered: metered.clone(),
                    hold: hold.clone(),
                    entered: entered.clone(),
                    release: release.clone(),
                },
            )?
            .serve_at(UP_PATH, Power)?
            .serve_at(PP_PATH, Profiles(saver.clone()))?
            .build()
            .await?;
        Ok(Self {
            connection,
            metered,
            saver,
            hold,
            entered,
            release,
        })
    }
    pub fn hold_next_snapshot(&self) {
        self.hold.store(true, Ordering::SeqCst);
    }
    pub async fn snapshot_entered(&self) {
        self.entered.notified().await;
    }
    pub fn release_snapshot(&self) {
        self.release.notify_one();
    }
    pub async fn set(&self, metered: u32, saver: bool) -> zbus::Result<()> {
        let profile_changed = self.saver.swap(saver, Ordering::SeqCst) != saver;
        self.metered.store(metered, Ordering::SeqCst);
        let (name, path, property) = if profile_changed {
            (PP, PP_PATH, "ActiveProfile")
        } else {
            (NM, NM_PATH, "Metered")
        };
        self.connection
            .emit_signal(
                None::<&str>,
                path,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(name, HashMap::<String, Value<'_>>::new(), vec![property]),
            )
            .await?;
        Ok(())
    }
    pub async fn owner(&self, owned: bool) -> zbus::Result<()> {
        if owned {
            self.connection.request_name(NM).await?;
        } else {
            self.connection.release_name(NM).await?;
        }
        Ok(())
    }
    pub async fn clients(&self) -> zbus::Result<usize> {
        Ok(zbus::fdo::DBusProxy::new(&self.connection)
            .await?
            .list_names()
            .await?
            .iter()
            .filter(|n| n.as_str().starts_with(':'))
            .count())
    }
}
