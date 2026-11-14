//! Одна явная страница журнала: только коды и числа из доверенного core.
use crate::diagnostics::context::Cause;
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
    #[serde(skip)]
    pub cause: Option<Failure>,
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
    technical_details: Option<String>,
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
                cause: Some(Failure::ReadBusy),
                ..Self::default()
            };
        };
        let page = &scoped.page;
        Self {
            cause: None,
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
                .filter(|(event, _)| !event.code.is_native() || event.technical_details().is_some())
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
                    technical_details: event.technical_details(),
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

#[derive(Clone, Copy, Debug)]
pub(crate) enum Failure {
    ReadBusy,
    SaveBusy,
    PageIdentity,
    PageUnavailable,
    DurabilityUnknown,
    Io(std::io::ErrorKind),
}
impl Failure {
    pub fn context(self) -> Option<Cause> {
        Some(match self {
            Self::ReadBusy => Cause::LogsReadBusy,
            Self::SaveBusy => Cause::LogsSaveBusy,
            Self::PageIdentity => Cause::LogsPageIdentity,
            Self::PageUnavailable => Cause::LogsPageUnavailable,
            Self::DurabilityUnknown => Cause::LogsDurabilityUnknown,
            Self::Io(std::io::ErrorKind::PermissionDenied) => Cause::LogsIoPermissionDenied,
            Self::Io(std::io::ErrorKind::AlreadyExists) => Cause::LogsIoAlreadyExists,
            Self::Io(std::io::ErrorKind::NotFound) => Cause::LogsIoNotFound,
            Self::Io(std::io::ErrorKind::InvalidInput) => Cause::LogsIoInvalidInput,
            Self::Io(std::io::ErrorKind::Other) => Cause::LogsIoOther,
            Self::Io(_) => return None,
        })
    }
}

#[derive(Default)]
pub(crate) struct SaveJob {
    pub cause: Option<Failure>,
    pending: Option<tokio::sync::oneshot::Receiver<(String, &'static str, Option<Failure>)>>,
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
        self.cause = None;
        if self.active() {
            self.cause = Some(Failure::SaveBusy);
            return "busy";
        }
        if page.request_id != page_id || !matches!(page.status, "ready" | "empty") {
            self.cause = Some(Failure::PageIdentity);
            return "refused";
        }
        let Some(export) = page.export.clone().filter(|s| s.len() <= 65536) else {
            self.cause = Some(Failure::PageUnavailable);
            return "refused";
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.pending = Some(receiver);
        tokio::task::spawn_blocking(move || {
            let (status, cause) = match publish(&destination, export.as_bytes()) {
                Ok(true) => ("saved", None),
                Ok(false) => ("saved_durability_unknown", Some(Failure::DurabilityUnknown)),
                Err(error) => ("refused", Some(Failure::Io(error.kind()))),
            };
            let _ = sender.send((request_id, status, cause));
        });
        "saving"
    }
    pub async fn complete(&mut self) -> Option<(String, &'static str)> {
        let result = match self.pending.as_mut() {
            Some(receiver) => receiver.await.ok(),
            None => std::future::pending().await,
        };
        self.pending = None;
        result.map(|(request_id, status, cause)| {
            self.cause = cause;
            (request_id, status)
        })
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

#[cfg(test)]
mod companion_tests {
    use super::*;
    #[tokio::test]
    async fn actual_save_conflict_preserves_typed_io_kind() {
        let root = std::env::temp_dir().join(format!("p11-log-save-{}", std::process::id()));
        std::fs::create_dir(&root).expect("собственный каталог");
        let destination = root.join("existing");
        std::fs::write(&destination, b"previous").expect("существующий файл");
        let page = Page::read(&EventLog::new(), 0, Source::All, "31".into());
        let mut job = SaveJob::default();
        assert_eq!(
            job.start(
                &page,
                "31",
                destination.to_string_lossy().into(),
                "37".into()
            ),
            "saving"
        );
        assert_eq!(job.complete().await, Some(("37".into(), "refused")));
        assert!(matches!(
            job.cause,
            Some(Failure::Io(std::io::ErrorKind::AlreadyExists))
        ));
        assert_eq!(
            job.cause.and_then(Failure::context),
            Some(Cause::LogsIoAlreadyExists)
        );
        assert!(
            Failure::Io(std::io::ErrorKind::Unsupported)
                .context()
                .is_none()
        );
        assert_eq!(std::fs::read(&destination).expect("файл"), b"previous");
        std::fs::remove_file(destination).expect("очистка");
        std::fs::remove_dir(root).expect("очистка каталога");
    }
}

#[cfg(test)]
mod native_projection_tests {
    use super::*;
    use yonik_linux::event_log::{EventCode, LogEvent};
    #[test]
    fn technical_body_is_shared_by_page_export_and_plain_dto() {
        let event = LogEvent {
            id: 1,
            source: EventSource::Engine,
            code: EventCode::NativeSingStarted,
            value: Some(123),
        };
        let body = event.technical_details().unwrap();
        assert_eq!(body, "sing-box started (1.23s)");
        let page = yonik_linux::event_log::LogPage {
            events: vec![event],
            cursor: 1,
            evicted: 0,
            rejected: 0,
            missed: 0,
        };
        assert!(page.export().contains(&body));
        let row = Row {
            id: "1".into(),
            source: "engine",
            code: "NativeSingStarted".into(),
            value: Some(123),
            technical_details: event.technical_details(),
            session_id: Some("1".into()),
            generation: Some("7".into()),
        };
        assert!(serde_json::to_string(&row).unwrap().contains(&body));
        let bad = LogEvent {
            value: Some(-1),
            ..event
        };
        assert!(bad.technical_details().is_none());
        let page = yonik_linux::event_log::LogPage {
            events: vec![bad],
            ..page
        };
        assert!(!page.export().contains("NativeSingStarted"));
    }
}
