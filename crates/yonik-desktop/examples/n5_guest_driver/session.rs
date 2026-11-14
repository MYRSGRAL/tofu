//! Реальные normal Host команды. Вход остаётся недостижим до sealed grant binding.
use super::{protocol::Core, sealed::VerifiedFixture};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use yonik_desktop::{DesktopHost, HostConfig};

type Result<T> = std::result::Result<T, &'static str>;
const WAIT: Duration = Duration::from_secs(20);
const MAX_SNAPSHOT: usize = 65536;

pub struct Session {
    host: DesktopHost,
    snapshots: mpsc::Receiver<String>,
    overflow: Arc<AtomicBool>,
    closed: bool,
    stop_sent: bool,
    core: Core,
    logs: super::logs::Observer,
    log_request: u64,
    source_sha256: String,
    capture_sha256: String,
}
impl Session {
    pub fn initialize(fixture: &VerifiedFixture, core: &Core) -> Result<Self> {
        let (tx, snapshots) = mpsc::sync_channel(64);
        let overflow = Arc::new(AtomicBool::new(false));
        let failed = overflow.clone();
        // Default key loader, backend, SystemSession. Не ephemeral key/override.
        let host = DesktopHost::start(
            HostConfig::new(fixture.data(), fixture.engines()),
            move |v| {
                if v.len() > MAX_SNAPSHOT || tx.try_send(v).is_err() {
                    failed.store(true, Ordering::Release);
                }
            },
        )
        .map_err(|_| "HOST_START")?;
        let mut session = Self {
            host,
            snapshots,
            overflow,
            closed: false,
            stop_sent: false,
            core: match core {
                Core::Xray => Core::Xray,
                Core::SingBox => Core::SingBox,
            },
            logs: super::logs::Observer::default(),
            log_request: 0,
            source_sha256: fixture.source_sha256.clone(),
            capture_sha256: fixture.capture_sha256.clone(),
        };
        session.wait(|v| v["ready"] == true)?;
        session.send(json!({"command":"set_engine","engine":core.name()}))?;
        session.wait(|v| v["engine"] == core.name())?;
        session.send(json!({"command":"select_source","id":fixture.controls().source_id}))?;
        let page = session.wait(|v| v["source_id"] == fixture.controls().source_id)?;
        let revision = page["revision"].as_i64().ok_or("MISSING_REVISION")?;
        if !page["profiles"].as_array().is_some_and(|profiles| {
            profiles
                .iter()
                .any(|v| v["id"].as_i64() == Some(fixture.controls().profile_id))
        }) {
            return Err("MISSING_FIXTURE_PROFILE");
        }
        session
            .send(json!({"command":"select_profile","id":fixture.controls().profile_id,"revision":revision}))?;
        session.wait(|v| v["selected_profile"].as_i64() == Some(fixture.controls().profile_id))?;
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
        let snapshot = self.wait(|v| {
            matches!(
                v["status"].as_str(),
                Some("Connected" | "Failed" | "Unknown" | "CleanupError")
            )
        })?;
        if snapshot["status"] == "Connected" {
            self.observe_logs("connected")?;
        }
        Ok(snapshot)
    }
    pub fn stop(&mut self) -> Result<Value> {
        if self.closed || self.stop_sent {
            return Err("STOP_ALREADY_SENT");
        }
        self.stop_sent = true;
        self.send(json!({"command":"stop"}))?;
        // Unknown не превращается в Disconnected или повторный Apply.
        let snapshot = self.wait(|v| {
            matches!(
                v["status"].as_str(),
                Some("Disconnected" | "Unknown" | "CleanupError")
            )
        })?;
        self.observe_logs("post-stop")?;
        Ok(snapshot)
    }
    fn observe_logs(&mut self, phase: &str) -> Result<()> {
        let dir = super::logs::directory()?;
        let mut pages = Vec::new();
        for source in ["all", "app", "engine"] {
            self.log_request += 1;
            let request = self.log_request.to_string();
            self.send(
                json!({"command":"logs_read","after":"0","source":source,"request_id":request}),
            )?;
            let snapshot = self.wait(|v| {
                v["logs"]["request_id"] == request
                    && matches!(
                        v["logs"]["status"].as_str(),
                        Some("ready" | "empty" | "busy" | "cancelled")
                    )
            })?;
            let page = &snapshot["logs"];
            let mut observation = self.logs.page(page, source, &request, &self.core)?;
            let name = format!("fr16-{}-{phase}-{source}.txt", self.core.name());
            self.log_request += 1;
            let save_request = self.log_request.to_string();
            self.send(json!({"command":"logs_save","page_request_id":request,
                "destination":format!("/run/yonik-acceptance/data/{name}"),"request_id":save_request}))?;
            let saved = self.wait(|v| {
                v["logs"]["request_id"] == save_request
                    && matches!(
                        v["logs"]["status"].as_str(),
                        Some(
                            "saved" | "saved_durability_unknown" | "refused" | "busy" | "cancelled"
                        )
                    )
            })?;
            observation["save"] = super::logs::verify_save(
                &dir,
                &name,
                &saved["logs"],
                &save_request,
                page["export"].as_str().ok_or("LOG_EXPORT")?.as_bytes(),
            )?;
            observation["source"] = json!(source);
            pages.push(observation);
        }
        let artifact = json!({"schema":"N5_FR16_HOST_LOGS_V1","core":self.core.name(),"phase":phase,
            "source_sha256":self.source_sha256,"capture_sha256":self.capture_sha256,
            "pages":pages,"qml_ru_en":{"class":"UNAVAILABLE"},"clipboard":{"class":"UNAVAILABLE"},
            "suppression":{"class":"SOURCE_INVARIANT"},"full_fr16":"NOT_PASS"});
        super::logs::persist(
            &dir,
            &format!("fr16-{}-{phase}.json", self.core.name()),
            &serde_json::to_vec(&artifact).map_err(|_| "LOG_ARTIFACT_JSON")?,
        )
    }
    pub fn close(&mut self) -> Result<()> {
        self.host.shutdown();
        if !self.host.wait_closed(Duration::from_secs(10)) {
            return Err("MISSING_HOST_CLOSED");
        }
        self.closed = true;
        Ok(())
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
