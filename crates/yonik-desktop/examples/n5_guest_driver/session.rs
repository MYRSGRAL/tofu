//! Реальные normal Host команды. Вход остаётся недостижим до sealed grant binding.
#![allow(dead_code)]
use super::protocol::{Core, InternalObservations};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};

type Result<T> = std::result::Result<T, &'static str>;
const WAIT: Duration = Duration::from_secs(20);
const MAX_SNAPSHOT: usize = 65536;

// Конструктор намеренно отсутствует. IPC-запрос и переменные окружения не grant.
// Будущий verifier должен связать USER grant, source и sealed fixture вне этого API.
pub struct VerifiedFixture {
    data: PathBuf,
    engines: PathBuf,
    source: i64,
    profile: i64,
    core: Core,
}

pub struct Session {
    host: DesktopHost,
    snapshots: mpsc::Receiver<String>,
    overflow: Arc<AtomicBool>,
    closed: bool,
    stop_sent: bool,
}
impl Session {
    pub fn initialize(fixture: VerifiedFixture) -> Result<Self> {
        let (tx, snapshots) = mpsc::sync_channel(64);
        let overflow = Arc::new(AtomicBool::new(false));
        let failed = overflow.clone();
        // Default key loader, backend, SystemSession. Не ephemeral key/override.
        let host = DesktopHost::start(HostConfig::new(fixture.data, fixture.engines), move |v| {
            if v.len() > MAX_SNAPSHOT || tx.try_send(v).is_err() {
                failed.store(true, Ordering::Release);
            }
        })
        .map_err(|_| "HOST_START")?;
        let mut session = Self {
            host,
            snapshots,
            overflow,
            closed: false,
            stop_sent: false,
        };
        session.wait(|v| v["ready"] == true)?;
        session.send(json!({"command":"set_engine","engine":fixture.core.name()}))?;
        session.wait(|v| v["engine"] == fixture.core.name())?;
        session.send(json!({"command":"select_source","id":fixture.source}))?;
        let page = session.wait(|v| v["source_id"] == fixture.source)?;
        let revision = page["revision"].as_i64().ok_or("MISSING_REVISION")?;
        if !page["profiles"].as_array().is_some_and(|profiles| {
            profiles
                .iter()
                .any(|v| v["id"].as_i64() == Some(fixture.profile))
        }) {
            return Err("MISSING_FIXTURE_PROFILE");
        }
        session
            .send(json!({"command":"select_profile","id":fixture.profile,"revision":revision}))?;
        session.wait(|v| v["selected_profile"].as_i64() == Some(fixture.profile))?;
        session.send(json!({"command":"set_connection_mode","mode":"system"}))?;
        let snapshot = session.wait(|v| v["connection_mode"] == "system")?;
        require_attribution(&snapshot)?;
        Ok(session)
    }
    fn send(&self, command: Value) -> Result<()> {
        self.host
            .dispatch(&command.to_string())
            .map_err(|_| "HOST_DISPATCH")
    }
    fn wait(&mut self, predicate: impl Fn(&Value) -> bool) -> Result<Value> {
        let end = Instant::now() + WAIT;
        loop {
            if self.overflow.load(Ordering::Acquire) {
                return Err("OBSERVER_OVERFLOW");
            }
            let remaining = end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("DEADLINE");
            }
            let raw = self
                .snapshots
                .recv_timeout(remaining)
                .map_err(|_| "DEADLINE_OR_CLOSED")?;
            let value: Value = serde_json::from_str(&raw).map_err(|_| "SNAPSHOT_PROTOCOL")?;
            if self.overflow.load(Ordering::Acquire) {
                return Err("OBSERVER_OVERFLOW");
            }
            if predicate(&value) {
                return Ok(value);
            }
        }
    }
    pub fn connect(&mut self) -> Result<Value> {
        if self.closed || self.stop_sent {
            return Err("SESSION_TERMINAL");
        }
        self.send(json!({"command":"connect"}))?;
        // Connected — только наблюдение публичного состояния, не attach ACK/coverage.
        self.wait(|v| {
            matches!(
                v["status"].as_str(),
                Some("Connected" | "Failed" | "Unknown" | "CleanupError")
            )
        })
    }
    pub fn stop(&mut self) -> Result<Value> {
        if self.closed || self.stop_sent {
            return Err("STOP_ALREADY_SENT");
        }
        self.stop_sent = true;
        self.send(json!({"command":"stop"}))?;
        // Unknown не превращается в Disconnected или повторный Apply.
        self.wait(|v| {
            matches!(
                v["status"].as_str(),
                Some("Disconnected" | "Unknown" | "CleanupError")
            )
        })
    }
    pub fn close(&mut self) -> Result<()> {
        self.host.shutdown();
        if !self.host.wait_closed(Duration::from_secs(10)) {
            return Err("MISSING_HOST_CLOSED");
        }
        self.closed = true;
        Ok(())
    }
    pub fn internal_observations(&self) -> InternalObservations {
        // Нет публичного источника canonical child/network/coverage receipt.
        InternalObservations::default()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if !self.closed {
            self.host.shutdown();
            let _closed = self.host.wait_closed(Duration::from_secs(10));
            // Drop не выдаёт cleanup receipt: результат не доказывает worker reap.
        }
    }
}

// Проверяет тот же enabled executable predicate, что InlinePolicy::requires_attribution.
fn require_attribution(snapshot: &Value) -> Result<()> {
    let draft: yonik_desktop::editor::Draft = serde_json::from_value(snapshot["routing"].clone())
        .map_err(|_| "MISSING_ROUTING_POLICY")?;
    let policy = draft.policy().map_err(|_| "INVALID_ROUTING_POLICY")?;
    if !policy
        .user_rules
        .iter()
        .chain(&policy.preset_rules)
        .any(|rule| rule.enabled && !rule.conditions.executables.is_empty())
    {
        return Err("ATTRIBUTION_REQUIRED");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_policy_cannot_enter_native_acceptance() {
        assert_eq!(
            require_attribution(&json!({"routing":{
                "version":1,"rules":[],"default_action":"proxy"
            }})),
            Err("ATTRIBUTION_REQUIRED")
        );
    }
    #[test]
    fn absent_routing_is_not_native_evidence() {
        assert_eq!(
            require_attribution(&json!({})),
            Err("MISSING_ROUTING_POLICY")
        );
    }
}
