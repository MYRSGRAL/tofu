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
        }
    }
}
pub(crate) struct Store {
    path: PathBuf,
    original: Option<Vec<u8>>,
    blocked: bool,
}
impl Store {
    pub fn open(directory: &Path) -> (Self, Preferences, bool) {
        let path = directory.join("preferences.json");
        let mut store = Self {
            path,
            original: None,
            blocked: false,
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
    pub fn save(&mut self, settings: &Preferences) -> std::io::Result<()> {
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
                fs::remove_file(&temp)?;
            } else {
                fs::rename(&temp, &self.path)?;
            }
            self.original = Some(bytes);
            File::open(self.path.parent().expect("каталог настроек"))?.sync_all()
        })();
        if result.is_err() && temp.exists() {
            if let Err(error) = fs::remove_file(temp) {
                log::warn!("Не удалось убрать временный файл настроек: {error}");
            }
        }
        result
    }
}
fn valid(p: &Preferences) -> bool {
    p.version == 1
        && matches!(p.engine.as_str(), "xray" | "sing-box")
        && matches!(p.language.as_str(), "ru" | "en")
        && p.probe_url.len() <= 2048
        && !p.probe_url.contains(['@', '?', '#'])
        && yonik_linux::ProbeTarget::new(&p.probe_url, std::time::Duration::from_secs(5))
            .and_then(|t| t.with_expected_status(p.probe_status))
            .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{Store, fs};
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
