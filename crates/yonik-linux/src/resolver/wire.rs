//! Ограниченный Varlink transport. Соединение уже проверено endpoint factory.
use std::{fmt, time::Duration};

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    time::Instant,
};

use super::{ResolverConfiguration, ResolverError, config::parse_configuration};
use crate::host::Cancellation;

const MAX_FRAME: usize = 1024 * 1024;
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const DUMP: &[u8] = b"{\"method\":\"io.systemd.Resolve.DumpDNSConfiguration\",\"parameters\":{}}\0";
const SUBSCRIBE: &[u8] = b"{\"method\":\"io.systemd.Resolve.Monitor.SubscribeDNSConfiguration\",\"parameters\":{\"allowInteractiveAuthentication\":false},\"more\":true}\0";

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Fresh,
    Following,
    Closed,
}

pub(super) struct ResolverWire {
    stream: Option<UnixStream>,
    buffer: Vec<u8>,
    partial_since: Option<Instant>,
    last_read: Option<Instant>,
    state: State,
}

impl ResolverWire {
    pub(super) fn new(stream: UnixStream) -> Self {
        Self {
            stream: Some(stream),
            buffer: Vec::new(),
            partial_since: None,
            last_read: None,
            state: State::Fresh,
        }
    }

    pub(super) fn close(&mut self) {
        self.stream.take();
        self.buffer.clear();
        self.partial_since = None;
        self.last_read = None;
        self.state = State::Closed;
    }

    pub(super) fn is_closed(&self) -> bool {
        self.stream.is_none()
    }

    pub(super) async fn snapshot(
        &mut self,
        cancel: &mut Cancellation,
        deadline: Instant,
    ) -> Result<ResolverConfiguration, ResolverError> {
        let result = self.start(cancel, deadline, false).await;
        self.close();
        result
    }

    pub(super) async fn subscribe(
        &mut self,
        cancel: &mut Cancellation,
        deadline: Instant,
    ) -> Result<ResolverConfiguration, ResolverError> {
        let result = self.start(cancel, deadline, true).await;
        if result.is_err() {
            self.close();
        }
        result
    }

    pub(super) async fn next(
        &mut self,
        cancel: &mut Cancellation,
    ) -> Result<ResolverConfiguration, ResolverError> {
        let result = if self.state == State::Following {
            self.receive(cancel, None, true).await
        } else {
            Err(ResolverError::Closed)
        };
        if result.is_err() {
            self.close();
        }
        result
    }

    async fn start(
        &mut self,
        cancel: &mut Cancellation,
        deadline: Instant,
        following: bool,
    ) -> Result<ResolverConfiguration, ResolverError> {
        if self.state != State::Fresh {
            return Err(ResolverError::Closed);
        }
        check_cancel(cancel)?;
        if Instant::now() >= deadline {
            return Err(ResolverError::Timeout);
        }
        let stream = self.stream.as_mut().ok_or(ResolverError::Closed)?;
        let request = if following { SUBSCRIBE } else { DUMP };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => return Err(ResolverError::Timeout),
            result = stream.write_all(request) => result.map_err(io_error)?,
        }
        let configuration = self.receive(cancel, Some(deadline), following).await?;
        self.state = if following {
            State::Following
        } else {
            State::Closed
        };
        Ok(configuration)
    }

    async fn receive(
        &mut self,
        cancel: &mut Cancellation,
        startup: Option<Instant>,
        following: bool,
    ) -> Result<ResolverConfiguration, ResolverError> {
        let frame = self.read_frame(cancel, startup).await?;
        let mut decoder = serde_json::Deserializer::from_slice(&frame);
        let mut response = UniqueValue::deserialize(&mut decoder)
            .map_err(|_| ResolverError::Malformed)?
            .0;
        decoder.end().map_err(|_| ResolverError::Malformed)?;
        let object = response.as_object_mut().ok_or(ResolverError::Malformed)?;
        if let Some(error) = object.get("error") {
            let error = error.as_str().ok_or(ResolverError::Malformed)?;
            return Err(match error {
                "org.varlink.service.UnknownMethod" | "org.varlink.service.UnknownInterface" => {
                    ResolverError::Unsupported
                }
                "io.systemd.InteractiveAuthenticationRequired"
                | "io.systemd.NotAuthorized"
                | "io.systemd.PermissionDenied" => ResolverError::PermissionDenied,
                _ => ResolverError::Malformed,
            });
        }
        match object.get("continues") {
            Some(Value::Bool(true)) if following => {}
            None | Some(Value::Bool(false)) if !following => {}
            _ => return Err(ResolverError::Malformed),
        }
        let parameters = object
            .remove("parameters")
            .ok_or(ResolverError::Malformed)?;
        if !parameters.get("configuration").is_some_and(Value::is_array) {
            return Err(ResolverError::Malformed);
        }
        check_cancel(cancel)?;
        let configuration = parse_configuration(parameters)?;
        if following {
            self.check_visible_eof()?;
        }
        Ok(configuration)
    }

    // Это только уже видимый EOF: наличие живого peer не является lease.
    fn check_visible_eof(&mut self) -> Result<(), ResolverError> {
        if !self.buffer.is_empty() {
            return Ok(());
        }
        let mut chunk = [0; 8192];
        match self
            .stream
            .as_ref()
            .ok_or(ResolverError::Closed)?
            .try_read(&mut chunk)
        {
            Ok(0) => Err(ResolverError::Closed),
            Ok(count) => {
                let received = Instant::now();
                self.partial_since = Some(received);
                self.last_read = Some(received);
                self.buffer.extend_from_slice(&chunk[..count]);
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(io_error(error)),
        }
    }

    async fn read_frame(
        &mut self,
        cancel: &mut Cancellation,
        startup: Option<Instant>,
    ) -> Result<Vec<u8>, ResolverError> {
        let mut searched = 0;
        loop {
            check_cancel(cancel)?;
            if startup.is_some_and(|end| Instant::now() >= end) {
                return Err(ResolverError::Timeout);
            }
            let deadline = match (startup, self.partial_since) {
                (Some(a), Some(b)) => Some(a.min(b + FRAME_TIMEOUT)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b + FRAME_TIMEOUT),
                (None, None) => None,
            };
            if let Some(offset) = self.buffer[searched..].iter().position(|byte| *byte == 0) {
                let end = searched + offset;
                let tail = self.buffer.split_off(end + 1);
                self.buffer.truncate(end);
                let frame = std::mem::replace(&mut self.buffer, tail);
                // Хвост впервые пришёл в последнем read, независимо от возраста A.
                self.partial_since = if self.buffer.is_empty() {
                    None
                } else {
                    self.last_read
                };
                return Ok(frame);
            }
            if deadline.is_some_and(|end| Instant::now() >= end) {
                return Err(ResolverError::Timeout);
            }
            if self.buffer.len() > MAX_FRAME {
                return Err(ResolverError::Limit);
            }
            searched = self.buffer.len();
            let mut chunk = [0; 8192];
            let room = chunk.len().min(MAX_FRAME + 1 - self.buffer.len());
            let stream = self.stream.as_mut().ok_or(ResolverError::Closed)?;
            let read = async { stream.read(&mut chunk[..room]).await.map_err(io_error) };
            let count = if let Some(end) = deadline {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
                    _ = tokio::time::sleep_until(end) => return Err(ResolverError::Timeout),
                    result = read => result?,
                }
            } else {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
                    result = read => result?,
                }
            };
            if count == 0 {
                return Err(ResolverError::Closed);
            }
            let received = Instant::now();
            self.last_read = Some(received);
            self.partial_since.get_or_insert(received);
            self.buffer.extend_from_slice(&chunk[..count]);
        }
    }
}

fn check_cancel(cancel: &Cancellation) -> Result<(), ResolverError> {
    cancel.check().map_err(|_| ResolverError::Cancelled)
}

fn io_error(error: std::io::Error) -> ResolverError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        ResolverError::PermissionDenied
    } else {
        ResolverError::Closed
    }
}

// Value сам по себе затирает повторные ключи. Проверяем каждый объект до вставки.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON без повторных ключей")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|n| UniqueValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("нечисловое значение"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.to_owned())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("повторный JSON ключ"));
                    }
                    values.insert(key, map.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

#[cfg(test)]
mod tests;
