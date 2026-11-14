//! Замкнутый вход runner: никакого argv/config/env executor.
#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Core {
    Xray,
    SingBox,
}
impl Core {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Xray => "xray",
            Self::SingBox => "sing-box",
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Case {
    Positive,
    MissingProfile,
    TamperedProfile,
    TamperedReport,
    Revoke,
    CancelBeforeHandoff,
    CancelAfterHandoff,
    HostLoss,
    UnknownStop,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Await,
    Revoke,
    Cancel,
    CloseDesktopOwner,
    Stop,
    Cleanup,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u8,
    pub seq: u64,
    pub core: Core,
    pub case: Case,
    pub action: Action,
}
pub fn decode(bytes: &[u8], previous: u64) -> Result<Request, &'static str> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err("BYTE_BOUND");
    }
    let request: Request = serde_json::from_slice(bytes).map_err(|_| "JSON_PROTOCOL")?;
    if request.schema != 2 || previous.checked_add(1) != Some(request.seq) {
        return Err("REQUEST_BINDING");
    }
    Ok(request)
}
// None означает отсутствие наблюдения, а не нулевой счётчик событий.
#[derive(Debug, Default, Serialize)]
pub struct InternalObservations {
    pub apply_sends: Option<u8>,
    pub admitted: Option<u8>,
    pub role_sends: Option<u8>,
    pub ack: Option<u8>,
    pub coverage: Option<u32>,
    pub owned_child_reaped: Option<bool>,
    pub network_restored: Option<bool>,
}
#[cfg(test)]
mod tests {
    use super::*;
    const VALID: &[u8] =
        br#"{"schema":2,"seq":1,"core":"xray","case":"positive","action":"start"}"#;
    #[test]
    fn strict_request_accepts_fixed_command() {
        assert!(decode(VALID, 0).is_ok());
    }
    #[test]
    fn strict_request_rejects_replay_and_overflow() {
        assert!(decode(VALID, 1).is_err());
        assert!(decode(VALID, u64::MAX).is_err());
    }
    #[test]
    fn strict_request_rejects_executor_duplicate_and_noninteger() {
        for value in [
            r#"{"schema":2,"seq":1,"core":"xray","case":"positive","action":"start","argv":[]}"#,
            r#"{"schema":2,"schema":2,"seq":1,"core":"xray","case":"positive","action":"start"}"#,
            r#"{"schema":2,"seq":true,"core":"xray","case":"positive","action":"start"}"#,
            r#"{"schema":2,"seq":1.0,"core":"xray","case":"positive","action":"start"}"#,
        ] {
            assert!(decode(value.as_bytes(), 0).is_err());
        }
    }
    #[test]
    fn missing_observations_are_null_not_zero() {
        let value = serde_json::to_value(InternalObservations::default()).unwrap();
        assert!(value
            .as_object()
            .unwrap()
            .values()
            .all(serde_json::Value::is_null));
    }
}
