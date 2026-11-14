//! Замкнутый вход runner: никакого argv/config/env executor.
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
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
#[derive(Debug, Deserialize, Serialize)]
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
#[derive(Debug, Deserialize, Serialize)]
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
#[derive(Debug, Deserialize, Serialize)]
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
}

pub fn strict_json(bytes: &[u8]) -> Result<serde_json::Value, &'static str> {
    use serde::{
        Deserializer,
        de::{Error, MapAccess, SeqAccess, Visitor},
    };
    use serde_json::Value;
    struct Unique(Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Unique;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("bounded unique JSON")
                }
                fn visit_bool<E: Error>(self, v: bool) -> Result<Unique, E> {
                    Ok(Unique(Value::Bool(v)))
                }
                fn visit_u64<E: Error>(self, v: u64) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_i64<E: Error>(self, v: i64) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_f64<E: Error>(self, _: f64) -> Result<Unique, E> {
                    Err(E::custom("integer required"))
                }
                fn visit_str<E: Error>(self, v: &str) -> Result<Unique, E> {
                    Ok(Unique(v.into()))
                }
                fn visit_unit<E: Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_none<E: Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                    let mut values = Vec::new();
                    while let Some(Unique(v)) = a.next_element()? {
                        values.push(v);
                    }
                    Ok(Unique(values.into()))
                }
                fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Unique, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some((k, Unique(v))) = a.next_entry::<String, Unique>()? {
                        if values.insert(k, v).is_some() {
                            return Err(A::Error::custom("duplicate key"));
                        }
                    }
                    Ok(Unique(Value::Object(values)))
                }
            }
            d.deserialize_any(V)
        }
    }
    serde_json::from_slice::<Unique>(bytes)
        .map(|v| v.0)
        .map_err(|_| "JSON_PROTOCOL")
}

#[cfg(test)]
mod unique_tests {
    use super::*;
    #[test]
    fn nested_duplicate_and_float_are_not_sealed_data() {
        assert!(strict_json(br#"{"a":{"x":1,"x":2}}"#).is_err());
        assert!(strict_json(br#"{"a":1.0}"#).is_err());
        assert!(strict_json(br#"{"a":1} garbage"#).is_err());
    }
}
