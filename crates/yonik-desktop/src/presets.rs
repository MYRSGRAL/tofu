//! Продуктовая проекция snapshots без URL и персональных правил.
use serde::Serialize;
use yonik_linux::presets::{Error, Metadata, Store};

#[derive(Default, Serialize)]
pub struct Status {
    pub current: Option<Version>,
    pub previous: Option<Version>,
    pub current_error: bool,
    pub previous_error: bool,
    pub prepared: bool,
    pub decision: Option<String>,
    pub activation_available: bool,
}
#[derive(Serialize)]
pub struct Version {
    source: String,
    version: String,
    checked_at_unix: u64,
}
impl From<Metadata> for Version {
    fn from(value: Metadata) -> Self {
        Self {
            source: value.source_id,
            version: value.version,
            checked_at_unix: value.checked_at_unix,
        }
    }
}
pub struct Controller {
    store: Store,
}
impl Controller {
    pub fn open(path: &std::path::Path) -> Result<Self, Error> {
        Ok(Self {
            store: Store::open(path)?,
        })
    }
    pub fn status(&self) -> Status {
        let current = self.store.active();
        let previous = self.store.previous();
        Status {
            current_error: current.is_err(),
            previous_error: previous.is_err(),
            current: current.ok().flatten().map(|(m, _)| m.into()),
            previous: previous.ok().flatten().map(|(m, _)| m.into()),
            ..Status::default()
        }
    }
    pub fn rollback(&self) -> Result<Status, Error> {
        self.store.rollback()?;
        Ok(self.status())
    }
    /// Отдельный preset tier сохраняет приоритеты и пользовательские правила.
    pub fn prepare(&self, mut policy: yonik_domain::Policy) -> Result<yonik_domain::Policy, Error> {
        let (_, candidate) = self.store.active()?.ok_or(Error::Invalid)?;
        policy.preset_rules = candidate.into_rules([1, 2], true);
        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_store_has_no_activation_and_no_prepared_policy() {
        let mut id = [0; 16];
        getrandom::getrandom(&mut id).expect("случайный путь");
        let path = std::env::temp_dir().join(format!("yonik-preset-controller-{id:x?}"));
        let controller = Controller::open(&path).expect("хранилище");
        let status = controller.status();
        assert!(status.current.is_none());
        assert!(status.previous.is_none());
        assert!(!status.activation_available);
        assert!(controller.prepare(yonik_domain::Policy::default()).is_err());
        assert!(controller.rollback().is_err());
        std::fs::remove_dir(path).expect("собственный пустой каталог");
    }
    #[test]
    fn preparation_uses_preset_tier_and_preserves_user_rules() {
        use yonik_domain::{Action, Conditions, Flow, Route, Rule, RuleOrigin};
        let mut id = [0; 16];
        getrandom::getrandom(&mut id).expect("случайный путь");
        let path = std::env::temp_dir().join(format!("yonik-preset-policy-{id:x?}"));
        let controller = Controller::open(&path).expect("store");
        let resource = |hash: &str| yonik_linux::presets::Resource {
            url: "https://example.org/list".into(),
            revision: "v1".into(),
            sha256: hash.into(),
        };
        controller
            .store
            .install(
                Metadata {
                    source_id: "fixture".into(),
                    version: "v1".into(),
                    format: "plain-domain-cidr-v1".into(),
                    checked_at_unix: 1,
                    enabled: true,
                    domains: resource(
                        "04e512879df7acb18392404d982a1b179e94c20b53ee03b75538624863d89e96",
                    ),
                    ips: resource(
                        "1056040935d29c506274717d77a7388683810fd7ae430176f81a59de1270c994",
                    ),
                },
                b"example.org\n".to_vec(),
                b"0.0.0.0/0\n".to_vec(),
            )
            .expect("install");
        let mut policy = yonik_domain::Policy::default();
        policy.user_rules.push(Rule {
            id: 42,
            enabled: true,
            conditions: Conditions {
                destinations: vec!["1.1.1.0/24".parse().expect("CIDR")],
                ..Conditions::default()
            },
            action: Action::Proxy,
        });
        let prepared = controller.prepare(policy).expect("prepare");
        assert_eq!(prepared.user_rules.len(), 1);
        for (ip, action, origin) in [
            ("1.1.1.1", Route::Proxy, RuleOrigin::User(42)),
            ("8.8.8.8", Route::Direct, RuleOrigin::Preset(2)),
        ] {
            let result = prepared.evaluate(&Flow {
                executable: None,
                destination: ip.parse().expect("IP"),
            });
            assert_eq!((result.action, result.origin), (action, origin));
        }
        std::fs::remove_dir_all(path).expect("собственная fixture");
    }
    #[test]
    fn version_projection_does_not_export_resources() {
        let resource = yonik_linux::presets::Resource {
            url: "https://private.example/secret".into(),
            revision: "v1".into(),
            sha256: "a".repeat(64),
        };
        let version = Version::from(Metadata {
            source_id: "fixture".into(),
            version: "v1".into(),
            format: "plain-domain-cidr-v1".into(),
            checked_at_unix: 10,
            enabled: true,
            domains: resource.clone(),
            ips: resource,
        });
        let json = serde_json::to_string(&version).expect("DTO");
        assert!(!json.contains("private"));
        assert!(!json.contains("secret"));
        assert!(!json.contains("url"));
    }
}
