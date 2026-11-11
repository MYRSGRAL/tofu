//! Одна явная страница журнала: только коды и числа из доверенного core.
use serde::{Deserialize, Serialize};
use yonik_linux::event_log::{EventLog, EventSource, LogFilter, MAX_PAGE};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    All,
    App,
    Engine,
}

#[derive(Default, Serialize)]
pub(crate) struct Page {
    pub request_id: String,
    pub status: &'static str,
    pub cursor: String,
    pub evicted: String,
    pub rejected: String,
    pub missed: String,
    pub events: Vec<Row>,
    pub export: Option<String>,
}
#[derive(Serialize)]
pub(crate) struct Row {
    id: String,
    source: &'static str,
    code: String,
    value: Option<i32>,
    session_id: Option<String>,
    generation: Option<String>,
}
impl Page {
    pub fn read(log: &EventLog, after: u64, source: Source, request_id: String) -> Self {
        let filter = LogFilter {
            source: match source {
                Source::All => None,
                Source::App => Some(EventSource::App),
                Source::Engine => Some(EventSource::Engine),
            },
            code: None,
        };
        let Some(scoped) = log.read_scoped(after, filter, MAX_PAGE) else {
            return Self {
                request_id,
                status: "busy",
                ..Self::default()
            };
        };
        let page = &scoped.page;
        Self {
            request_id,
            status: if page.events.is_empty() {
                "empty"
            } else {
                "ready"
            },
            cursor: page.cursor.to_string(),
            evicted: page.evicted.to_string(),
            rejected: page.rejected.to_string(),
            missed: page.missed.to_string(),
            export: Some(format!(
                "{}scope=page partial=true after={after}\n",
                scoped.export()
            )),
            events: page
                .events
                .iter()
                .zip(&scoped.contexts)
                .map(|(event, context)| Row {
                    session_id: context.map(|c| c.session_id.to_string()),
                    generation: context.map(|c| c.generation.to_string()),
                    id: event.id.to_string(),
                    source: match event.source {
                        EventSource::App => "app",
                        EventSource::Engine => "engine",
                    },
                    code: format!("{:?}", event.code),
                    value: event.value,
                })
                .collect(),
        }
    }
}
pub(crate) fn valid_number(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 20
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_untrusted_numbers_and_native_canary() {
        for value in ["", "-1", "1.0", "18446744073709551616", "https://SECRET"] {
            assert!(!valid_number(value));
        }
        assert!(valid_number("18446744073709551615"));
        let log = EventLog::new();
        log.reject_native(b"https://user:SECRET@host/ --password=SECRET\x1b[31m");
        let page = Page::read(&log, 0, Source::Engine, "1".into());
        assert_eq!(page.rejected, "1");
        let encoded = serde_json::to_string(&page).expect("DTO");
        assert!(!encoded.contains("SECRET"));
        assert!(!encoded.contains("https"));
        assert!(page.events.is_empty());
    }
}

/// Публикует только принятую backend-страницу; содержимое от UI не принимается.
#[cfg(test)]
pub(crate) fn save(page: &Page, page_id: &str, destination: &str) -> &'static str {
    if page.request_id != page_id || !matches!(page.status, "ready" | "empty") {
        return "refused";
    }
    let Some(export) = &page.export else {
        return "refused";
    };
    match publish(destination, export.as_bytes()) {
        Ok(true) => "saved",
        Ok(false) => "saved_durability_unknown",
        Err(_) => "refused",
    }
}

fn publish(destination: &str, bytes: &[u8]) -> std::io::Result<bool> {
    use rustix::fs::{self, AtFlags, Mode, OFlags, RenameFlags};
    use std::{
        fs::File,
        io::Write,
        path::{Component, Path},
    };
    let invalid = || std::io::Error::other("недопустимый путь экспорта");
    if destination.len() > 4096 || destination.chars().any(char::is_control) || bytes.len() > 65536
    {
        return Err(invalid());
    }
    let path = Path::new(destination);
    if !path.is_absolute() {
        return Err(invalid());
    }
    let name = path.file_name().ok_or_else(invalid)?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = fs::open("/", flags, Mode::empty())?;
    for component in path.parent().ok_or_else(invalid)?.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(part) => {
                directory = fs::openat(&directory, part, flags, Mode::empty())?
            }
            _ => return Err(invalid()),
        }
    }
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|_| invalid())?;
    let temp = format!(".yonik-log-{:032x}", u128::from_ne_bytes(random));
    let fd = fs::openat(
        &directory,
        temp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )?;
    let mut file = File::from(fd);
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::renameat_with(
            &directory,
            temp.as_str(),
            &directory,
            name,
            RenameFlags::NOREPLACE,
        )?;
        Ok(fs::fsync(&directory).is_ok())
    })();
    if let Err(error) = fs::unlinkat(&directory, temp.as_str(), AtFlags::empty()) {
        if error != rustix::io::Errno::NOENT {
            log::warn!("Не удалось удалить временный экспорт журнала");
        }
    }
    result
}

#[cfg(test)]
mod save_tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    #[test]
    fn export_is_private_atomic_and_refuses_existing_files_and_symlinks() {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).expect("random");
        let directory =
            std::env::temp_dir().join(format!("yonik-save-{:032x}", u128::from_ne_bytes(random)));
        fs::create_dir(&directory).expect("fixture");
        let destination = directory.join("events.txt");
        let log = EventLog::new();
        log.reject_native(b"SECRET\x1b[31m https://user:password@host");
        let page = Page::read(&log, 0, Source::All, "1".into());
        assert_eq!(
            save(&page, "2", destination.to_str().expect("path")),
            "refused"
        );
        assert!(!destination.exists());
        assert_eq!(
            save(&page, "1", destination.to_str().expect("path")),
            "saved"
        );
        let bytes = fs::read(&destination).expect("export");
        assert_eq!(bytes, page.export.as_ref().expect("export").as_bytes());
        assert!(!String::from_utf8_lossy(&bytes).contains("SECRET"));
        assert_eq!(
            fs::metadata(&destination)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            save(&page, "1", destination.to_str().expect("path")),
            "refused"
        );
        assert_eq!(fs::read(&destination).expect("preserved"), bytes);
        let link = directory.join("link");
        symlink(&destination, &link).expect("symlink");
        assert_eq!(save(&page, "1", link.to_str().expect("path")), "refused");
        let parent_link = directory.join("parent-link");
        symlink(&directory, &parent_link).expect("parent symlink");
        assert_eq!(
            save(&page, "1", parent_link.join("new").to_str().expect("path")),
            "refused"
        );
        assert!(!directory.join("new").exists());
        assert_eq!(fs::read_dir(&directory).expect("fixture").count(), 3);
        fs::remove_file(parent_link).expect("cleanup");
        fs::remove_file(link).expect("cleanup");
        fs::remove_file(destination).expect("cleanup");
        fs::remove_dir(directory).expect("cleanup");
    }
}

#[derive(Default)]
pub(crate) struct SaveJob {
    pending: Option<tokio::sync::oneshot::Receiver<(String, &'static str)>>,
}
impl SaveJob {
    pub fn active(&self) -> bool {
        self.pending.is_some()
    }
    pub fn start(
        &mut self,
        page: &Page,
        page_id: &str,
        destination: String,
        request_id: String,
    ) -> &'static str {
        if self.active() {
            return "busy";
        }
        if page.request_id != page_id || !matches!(page.status, "ready" | "empty") {
            return "refused";
        }
        let Some(export) = page.export.clone().filter(|s| s.len() <= 65536) else {
            return "refused";
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.pending = Some(receiver);
        tokio::task::spawn_blocking(move || {
            let status = match publish(&destination, export.as_bytes()) {
                Ok(true) => "saved",
                Ok(false) => "saved_durability_unknown",
                Err(_) => "refused",
            };
            let _ = sender.send((request_id, status));
        });
        "saving"
    }
    pub async fn complete(&mut self) -> Option<(String, &'static str)> {
        let result = match self.pending.as_mut() {
            Some(receiver) => receiver.await.ok(),
            None => std::future::pending().await,
        };
        self.pending = None;
        result
    }
}

#[cfg(test)]
mod job_tests {
    use super::*;
    #[tokio::test]
    async fn only_one_job_is_accepted_and_ack_keeps_id() {
        let page = Page::read(&EventLog::new(), 0, Source::All, "1".into());
        let mut job = SaveJob::default();
        assert_eq!(
            job.start(&page, "1", "relative-path-refused".into(), "2".into()),
            "saving"
        );
        assert_eq!(
            job.start(&page, "1", "relative-path-refused".into(), "3".into()),
            "busy"
        );
        assert_eq!(job.complete().await, Some(("2".into(), "refused")));
        assert!(!job.active());
    }
}
