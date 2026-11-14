//! Приватный supplemental witness; публичный N5 receipt v2 не меняется.
use super::{hash::sha256, protocol::Core};
use rustix::fs::{self, Mode, OFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
};

type Result<T> = std::result::Result<T, &'static str>;
const BOUND: usize = 65536;
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
#[serde(deny_unknown_fields)]
struct Row {
    id: String,
    source: String,
    code: String,
    value: Option<i32>,
    technical_details: Option<String>,
    session_id: Option<String>,
    generation: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Page {
    request_id: String,
    status: String,
    cursor: String,
    evicted: String,
    rejected: String,
    missed: String,
    events: Vec<Row>,
    export: Option<String>,
}
fn linked_export(page: &Page) -> Result<String> {
    use yonik_linux::event_log::{
        EventCode, EventContext, EventSource, LogEvent, LogPage, ScopedPage,
    };
    let mut events = Vec::new();
    let mut contexts = Vec::new();
    for row in &page.events {
        let code = match row.code.as_str() {
            "StartRequested" => EventCode::StartRequested,
            "ProcessSpawned" => EventCode::ProcessSpawned,
            "StopRequested" => EventCode::StopRequested,
            "CleanupConfirmed" => EventCode::CleanupConfirmed,
            "CleanupFailed" => EventCode::CleanupFailed,
            "ProcessExited" => EventCode::ProcessExited,
            "NativeSingStarted" => EventCode::NativeSingStarted,
            "NativeSingMissingInterface" => EventCode::NativeSingMissingInterface,
            "NativeXrayStarted" => EventCode::NativeXrayStarted,
            "NativeXrayLocalDns" => EventCode::NativeXrayLocalDns,
            _ => return Err("LOG_EVENT_CODE"),
        };
        let event = LogEvent {
            id: row.id.parse().map_err(|_| "LOG_EVENT_ID")?,
            source: match row.source.as_str() {
                "app" => EventSource::App,
                "engine" => EventSource::Engine,
                _ => return Err("LOG_SOURCE"),
            },
            code,
            value: row.value,
        };
        if event.technical_details() != row.technical_details {
            return Err("LOG_TECHNICAL_BODY");
        }
        contexts.push(match (&row.session_id, &row.generation) {
            (Some(session), Some(generation)) => Some(EventContext {
                session_id: session.parse().map_err(|_| "LOG_CONTEXT")?,
                generation: generation.parse().map_err(|_| "LOG_CONTEXT")?,
            }),
            (None, None) => None,
            _ => return Err("LOG_CONTEXT"),
        });
        events.push(event);
    }
    let parse = |s: &str| s.parse::<u64>().map_err(|_| "LOG_COUNTER");
    let scoped = ScopedPage {
        page: LogPage {
            events,
            cursor: parse(&page.cursor)?,
            evicted: parse(&page.evicted)?,
            rejected: parse(&page.rejected)?,
            missed: parse(&page.missed)?,
        },
        contexts,
    };
    Ok(format!(
        "{}scope=page partial=true after=0\n",
        scoped.export()
    ))
}
#[derive(Default)]
pub struct Observer {
    seen: BTreeMap<String, Row>,
}
fn number(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 20
        && s.bytes().all(|b| b.is_ascii_digit())
        && s.parse::<u64>().is_ok()
}
impl Observer {
    pub fn page(
        &mut self,
        value: &Value,
        source: &str,
        request: &str,
        core: &Core,
    ) -> Result<Value> {
        let encoded = serde_json::to_vec(value).map_err(|_| "LOG_JSON")?;
        if encoded.len() > BOUND {
            return Err("LOG_PAGE_BOUND");
        }
        let page: Page = serde_json::from_slice(&encoded).map_err(|_| "LOG_PAGE_TYPE")?;
        if page.request_id != request
            || !matches!(page.status.as_str(), "ready" | "empty")
            || page.events.len() > 256
            || page.export.as_ref().is_none_or(|s| s.len() > BOUND)
            || [&page.cursor, &page.evicted, &page.rejected, &page.missed]
                .iter()
                .any(|s| !number(s))
        {
            return Err("LOG_PAGE_CONTRACT");
        }
        if page.export.as_deref() != Some(linked_export(&page)?.as_str()) {
            return Err("LOG_PAGE_EXPORT_LINKAGE");
        }
        let mut native = Vec::new();
        let mut previous = 0;
        for row in &page.events {
            let id = row.id.parse::<u64>().map_err(|_| "LOG_EVENT_ID")?;
            if !number(&row.id)
                || id <= previous
                || id > page.cursor.parse::<u64>().map_err(|_| "LOG_CURSOR")?
                || !matches!(row.source.as_str(), "app" | "engine")
                || (source != "all" && row.source != source)
                || row.session_id.as_ref().is_some_and(|s| !number(s))
                || row.generation.as_ref().is_some_and(|s| !number(s))
            {
                return Err("LOG_ROW_CONTRACT");
            }
            previous = id;
            if self.seen.get(&row.id).is_some_and(|old| old != row) {
                return Err("LOG_CONTEXT_CHANGED");
            }
            let expected = match core {
                Core::Xray => row.code == "NativeXrayLocalDns" && row.value.is_none(),
                Core::SingBox => {
                    row.code == "NativeSingStarted"
                        && row.value.is_some_and(|n| (0..=8_640_000).contains(&n))
                }
            };
            let technical_matches = {
                use yonik_linux::event_log::{EventCode, EventSource, LogEvent};
                let event = LogEvent {
                    id,
                    source: EventSource::Engine,
                    code: match core {
                        Core::Xray => EventCode::NativeXrayLocalDns,
                        Core::SingBox => EventCode::NativeSingStarted,
                    },
                    value: row.value,
                };
                event
                    .technical_details()
                    .is_some_and(|body| row.technical_details.as_deref() == Some(body.as_str()))
            };
            if expected
                && technical_matches
                && row.source == "engine"
                && row.session_id.is_some()
                && row.generation.is_some()
                && row.technical_details.is_some()
            {
                native.push(json!({"id":row.id,"session_id":row.session_id,"generation":row.generation,
                    "nativeInfo":match core { Core::Xray => 10, Core::SingBox => 7 },"value":row.value}));
            }
        }
        // Неограниченный lifetime cache не нужен: всего шесть bounded pages.
        if self.seen.len() + page.events.len() > 1536 {
            return Err("LOG_CONTEXT_BOUND");
        }
        for row in &page.events {
            self.seen.insert(row.id.clone(), row.clone());
        }
        Ok(
            json!({"class":"ACTUAL","page":value,"page_sha256":sha256(&encoded),
            "native_info":{"class":if native.is_empty(){"UNAVAILABLE"}else{"ACTUAL"},"events":native}}),
        )
    }
}
// Только заранее разрешённый data root; HOME и входные пути не используются.
pub fn directory() -> Result<File> {
    use std::os::unix::fs::MetadataExt;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut fd = fs::open("/", flags, Mode::empty()).map_err(|_| "LOG_ROOT")?;
    for part in ["run", "yonik-acceptance", "data"] {
        fd = fs::openat(&fd, part, flags, Mode::empty()).map_err(|_| "LOG_DIRECTORY")?;
    }
    let file = File::from(fd);
    let meta = file.metadata().map_err(|_| "LOG_DIRECTORY_STAT")?;
    if meta.uid() != rustix::process::getuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err("LOG_DIRECTORY_OWNER");
    }
    Ok(file)
}
pub fn persist(dir: &File, name: &str, bytes: &[u8]) -> Result<()> {
    if bytes.len() > 524288 {
        return Err("LOG_ARTIFACT_BOUND");
    }
    let fd = fs::openat(
        dir,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|_| "LOG_ARTIFACT_CREATE")?;
    let mut file = File::from(fd);
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "LOG_ARTIFACT_WRITE")
}
pub fn verify_save(
    dir: &File,
    name: &str,
    ack: &Value,
    request: &str,
    export: &[u8],
) -> Result<Value> {
    use std::os::unix::fs::MetadataExt;
    if ack["request_id"] != request
        || !matches!(
            ack["status"].as_str(),
            Some("saved" | "saved_durability_unknown")
        )
    {
        return Err("LOG_SAVE_ACK");
    }
    let fd = fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| "LOG_SAVE_OPEN")?;
    let file = File::from(fd);
    let meta = file.metadata().map_err(|_| "LOG_SAVE_STAT")?;
    if !meta.is_file()
        || meta.uid() != rustix::process::getuid().as_raw()
        || meta.mode() & 0o777 != 0o600
        || meta.nlink() != 1
    {
        return Err("LOG_SAVE_OWNER");
    }
    let mut bytes = Vec::new();
    file.take((BOUND + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "LOG_SAVE_READ")?;
    save_linkage(&bytes, export)?;
    Ok(
        json!({"class":"ACTUAL","ack":ack["status"],"request_id":request,"bytes":bytes.len(),"sha256":sha256(&bytes),"page_export_equal":true}),
    )
}
fn save_linkage(bytes: &[u8], export: &[u8]) -> Result<()> {
    if bytes.len() > BOUND || bytes != export {
        return Err("LOG_SAVE_LINKAGE");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(events: Value) -> Value {
        let mut value = json!({"request_id":"1","status":"ready","cursor":"5","evicted":"0","rejected":"0","missed":"0","events":events,"export":null});
        let typed: Page = serde_json::from_value(value.clone()).expect("typed fixture");
        value["export"] = json!(linked_export(&typed).expect("export fixture"));
        value
    }
    fn row() -> Value {
        json!({"id":"5","source":"engine","code":"NativeSingStarted","value":123,"technical_details":"sing-box started (1.23s)","session_id":"2","generation":"3"})
    }
    #[test]
    fn missing_info_is_unavailable_and_core_classification_is_numeric() {
        let mut observer = Observer::default();
        let result = observer
            .page(&page(json!([])), "all", "1", &Core::Xray)
            .expect("пустая страница");
        assert_eq!(result["native_info"]["class"], "UNAVAILABLE");
        let result = observer
            .page(&page(json!([row()])), "engine", "1", &Core::SingBox)
            .expect("INFO");
        assert_eq!(result["native_info"]["events"][0]["nativeInfo"], 7);
        assert_eq!(
            observer
                .page(&page(json!([row()])), "engine", "1", &Core::Xray)
                .expect("другое ядро")["native_info"]["class"],
            "UNAVAILABLE"
        );
    }
    #[test]
    fn rejects_filter_violation_rebinding_and_request_mismatch() {
        let mut observer = Observer::default();
        let value = page(json!([row()]));
        assert!(observer.page(&value, "app", "1", &Core::SingBox).is_err());
        assert!(observer.page(&value, "all", "2", &Core::SingBox).is_err());
        observer
            .page(&value, "all", "1", &Core::SingBox)
            .expect("первая страница");
        let mut changed = value;
        changed["events"][0]["generation"] = json!("4");
        let typed: Page = serde_json::from_value(changed.clone()).expect("typed fixture");
        changed["export"] = json!(linked_export(&typed).expect("export fixture"));
        assert_eq!(
            observer.page(&changed, "engine", "1", &Core::SingBox),
            Err("LOG_CONTEXT_CHANGED")
        );
    }
    #[test]
    fn export_linkage_rejects_changed_bytes_and_projection() {
        assert_eq!(save_linkage(b"exact", b"exact"), Ok(()));
        assert_eq!(save_linkage(b"changed", b"exact"), Err("LOG_SAVE_LINKAGE"));
        assert_eq!(
            save_linkage(&vec![0; BOUND + 1], &vec![0; BOUND + 1]),
            Err("LOG_SAVE_LINKAGE")
        );
        let mut value = page(json!([row()]));
        value["export"] = json!("unrelated export");
        assert_eq!(
            Observer::default().page(&value, "all", "1", &Core::SingBox),
            Err("LOG_PAGE_EXPORT_LINKAGE")
        );
        let mut xray = row();
        xray["code"] = json!("NativeXrayLocalDns");
        xray["value"] = Value::Null;
        xray["technical_details"] = json!(
            yonik_linux::event_log::LogEvent {
                id: 5,
                source: yonik_linux::event_log::EventSource::Engine,
                code: yonik_linux::event_log::EventCode::NativeXrayLocalDns,
                value: None
            }
            .technical_details()
            .expect("статический template")
        );
        let result = Observer::default()
            .page(&page(json!([xray])), "engine", "1", &Core::Xray)
            .expect("xray INFO");
        assert_eq!(result["native_info"]["events"][0]["nativeInfo"], 10);
    }
    #[test]
    fn rejects_oversized_pages_and_duplicate_ids() {
        let mut observer = Observer::default();
        assert!(
            observer
                .page(&page(json!([row(), row()])), "all", "1", &Core::SingBox)
                .is_err()
        );
        let mut value = page(json!([]));
        value["export"] = json!("x".repeat(BOUND + 1));
        assert_eq!(
            observer.page(&value, "all", "1", &Core::SingBox),
            Err("LOG_PAGE_BOUND")
        );
    }
}
