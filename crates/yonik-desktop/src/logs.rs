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
        let Some(page) = log.read(after, filter, MAX_PAGE) else {
            return Self {
                request_id,
                status: "busy",
                ..Self::default()
            };
        };
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
            export: Some(page.export()),
            events: page
                .events
                .iter()
                .map(|event| Row {
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
