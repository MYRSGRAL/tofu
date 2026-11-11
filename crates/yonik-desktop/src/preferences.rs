//! Ограниченные настройки без подписок и ключей; исходник ошибки не перезаписывается.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 4096;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preferences {
    pub version: u8,
    pub engine: String,
    pub port: Option<std::num::NonZeroU16>,
    pub probe_url: String,
    pub probe_status: u16,
    pub language: String,
    #[serde(default)]
    pub proxy_fetch_dns: Option<std::net::SocketAddr>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            engine: "xray".into(),
            port: None,
            probe_url: super::backend::DEFAULT_PROBE_URL.into(),
            probe_status: 204,
            language: "ru".into(),
            proxy_fetch_dns: None,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SaveOutcome {
    Durable,
    PublishedDurabilityUnknown,
}

pub(crate) struct Store {
    path: PathBuf,
    original: Option<Vec<u8>>,
    blocked: bool,
    pub(crate) sync_directory: fn(&Path) -> std::io::Result<()>,
}
impl Store {
    pub fn open(directory: &Path) -> (Self, Preferences, bool) {
        let path = directory.join("preferences.json");
        let mut store = Self {
            path,
            original: None,
            blocked: false,
            sync_directory: |path| File::open(path)?.sync_all(),
        };
        let settings = match store.read() {
            Ok(None) => Some(Preferences::default()),
            Ok(Some(bytes)) => {
                let parsed = serde_json::from_slice::<Preferences>(&bytes)
                    .ok()
                    .filter(valid);
                store.original = Some(bytes);
                parsed
            }
            Err(_) => None,
        };
        store.blocked = settings.is_none();
        let warning = store.blocked;
        (store, settings.unwrap_or_default(), warning)
    }
    fn read(&self) -> std::io::Result<Option<Vec<u8>>> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(&self.path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.len() > MAX_BYTES as u64
            || meta.uid() != rustix::process::getuid().as_raw()
            || meta.mode() & 0o777 != 0o600
        {
            return Err(std::io::Error::other("небезопасные настройки"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(std::io::Error::other("лимит настроек"));
        }
        Ok(Some(bytes))
    }
    pub fn save(&mut self, settings: &Preferences) -> std::io::Result<SaveOutcome> {
        if self.blocked || !valid(settings) || self.read()? != self.original {
            return Err(std::io::Error::other(
                "исходник настроек сохранён; запись заблокирована",
            ));
        }
        let bytes = serde_json::to_vec(settings)?;
        if bytes.len() > MAX_BYTES {
            return Err(std::io::Error::other("лимит настроек"));
        }
        let temp = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            if self.read()? != self.original {
                return Err(std::io::Error::other("настройки изменились извне"));
            }
            if self.original.is_none() {
                fs::hard_link(&temp, &self.path)?;
            } else {
                fs::rename(&temp, &self.path)?;
            }
            Ok(())
        })();
        if temp.exists() {
            if let Err(error) = fs::remove_file(temp) {
                log::warn!("Не удалось убрать временный файл настроек: {error}");
            }
        }
        result?;
        // После публикации откат не обещаем: память должна принять видимый файл.
        self.original = Some(bytes);
        Ok(
            if (self.sync_directory)(self.path.parent().expect("каталог настроек")).is_ok()
            {
                SaveOutcome::Durable
            } else {
                SaveOutcome::PublishedDurabilityUnknown
            },
        )
    }
}
fn valid(p: &Preferences) -> bool {
    p.version == 1
        && p.proxy_fetch_dns.is_none_or(valid_proxy_fetch_dns)
        && matches!(p.engine.as_str(), "xray" | "sing-box")
        && matches!(p.language.as_str(), "ru" | "en")
        && p.probe_url.len() <= 2048
        && !p.probe_url.contains(['@', '?', '#'])
        && yonik_linux::ProbeTarget::new(&p.probe_url, std::time::Duration::from_secs(5))
            .and_then(|t| t.with_expected_status(p.probe_status))
            .is_ok()
}

/// Проверка тем же API, которое выполняет Proxy fetch; сетевых обращений нет.
pub(crate) fn valid_proxy_fetch_dns(address: std::net::SocketAddr) -> bool {
    yonik_linux::FetchHost::new(std::time::Duration::from_secs(30))
        .and_then(|host| host.with_socks_endpoint(std::net::SocketAddr::from(([127, 0, 0, 1], 1))))
        .and_then(|host| host.with_proxy_dns(address))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{SaveOutcome, Store, fs};
    #[test]
    fn legacy_preferences_default_to_no_proxy_dns() {
        let mut json = serde_json::to_value(super::Preferences::default()).expect("JSON");
        json.as_object_mut()
            .expect("объект")
            .remove("proxy_fetch_dns");
        let legacy: super::Preferences = serde_json::from_value(json).expect("версия 1");
        assert!(legacy.proxy_fetch_dns.is_none());
        assert!(super::valid(&legacy));
    }

    #[test]
    fn directory_sync_failure_reports_publication_and_reopens_new_settings() {
        let dir =
            std::env::temp_dir().join(format!("yonik-preferences-sync-{}", std::process::id()));
        fs::create_dir(&dir).expect("fixture");
        let (mut store, mut preferences, _) = Store::open(&dir);
        store.save(&preferences).expect("начальное сохранение");
        store.sync_directory = |_| Err(std::io::Error::other("инъекция dir fsync"));
        preferences.language = "en".into();
        let result = store.save(&preferences);
        let (_, reopened, warning) = Store::open(&dir);
        assert!(!warning);
        assert_eq!(reopened.language, "en");
        assert_eq!(
            result.expect("опубликовано"),
            SaveOutcome::PublishedDurabilityUnknown
        );
        store.sync_directory = |path| super::File::open(path)?.sync_all();
        preferences.language = "ru".into();
        assert_eq!(
            store.save(&preferences).expect("повторная запись"),
            SaveOutcome::Durable
        );
        assert_eq!(Store::open(&dir).1.language, "ru");
        fs::remove_dir_all(dir).expect("очистка fixture");
    }

    #[test]
    fn foreign_temp_and_changed_file_are_preserved() {
        let dir =
            std::env::temp_dir().join(format!("yonik-preferences-foreign-{}", std::process::id()));
        fs::create_dir(&dir).expect("fixture");
        let (mut store, p, _) = Store::open(&dir);
        let temp = store
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temp, b"foreign").expect("fixture");
        assert!(store.save(&p).is_err());
        assert_eq!(fs::read(&temp).expect("fixture"), b"foreign");
        fs::remove_file(&temp).expect("fixture");
        store.save(&p).expect("сохранение");
        fs::write(&store.path, b"changed").expect("fixture");
        assert!(store.save(&p).is_err());
        assert_eq!(fs::read(&store.path).expect("fixture"), b"changed");
        fs::remove_dir_all(dir).expect("очистка fixture");
    }

    #[test]
    fn reopen_and_preserve_invalid_truncated_unknown_and_symlink() {
        let dir = std::env::temp_dir().join(format!("yonik-preferences-{}", std::process::id()));
        fs::create_dir(&dir).expect("fixture");
        let (mut store, mut p, warning) = Store::open(&dir);
        assert!(!warning);
        p.language = "en".into();
        p.engine = "sing-box".into();
        store.save(&p).expect("сохранение");
        assert_eq!(Store::open(&dir).1.language, "en");
        let mut unknown = serde_json::to_value(&p).expect("JSON");
        unknown["version"] = 99.into();
        let unknown = serde_json::to_vec(&unknown).expect("JSON");
        let oversized = vec![b' '; 4097];
        for raw in [
            b"{".as_slice(),
            b"invalid",
            unknown.as_slice(),
            oversized.as_slice(),
        ] {
            fs::write(&store.path, raw).expect("fixture");
            let (mut broken, _, warning) = Store::open(&dir);
            assert!(warning);
            assert!(broken.save(&p).is_err());
            assert_eq!(fs::read(&store.path).expect("исходник"), raw);
        }
        fs::remove_file(&store.path).expect("fixture");
        let victim = dir.join("victim");
        fs::write(&victim, "чужой файл".as_bytes()).expect("fixture");
        std::os::unix::fs::symlink(&victim, &store.path).expect("fixture");
        let (mut broken, _, warning) = Store::open(&dir);
        assert!(warning);
        assert!(broken.save(&p).is_err());
        assert_eq!(fs::read(&victim).expect("fixture"), "чужой файл".as_bytes());
        fs::remove_dir_all(dir).expect("очистка fixture");
    }
}
