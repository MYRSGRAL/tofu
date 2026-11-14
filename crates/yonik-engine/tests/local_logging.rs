//! INFO включается только явным LocalSocks преобразованием.
use std::num::NonZeroU16;
use yonik_engine::{Engine, compile};
use yonik_import::import_bytes;

#[test]
fn local_info_is_explicit_and_does_not_change_default_compiler() {
    let report =
        import_bytes(b"vless://00112233-4455-6677-8899-aabbccddeeff@127.0.0.1:24443".to_vec())
            .unwrap();
    let server = &report.servers[0];
    for engine in [Engine::Xray, Engine::SingBox] {
        let config = compile(server, engine, NonZeroU16::new(24444).unwrap()).unwrap();
        let mut original = Vec::new();
        config.write_sensitive(&mut original).unwrap();
        let original: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let mut local = Vec::new();
        config
            .with_local_runtime_logging()
            .write_sensitive(&mut local)
            .unwrap();
        let local: serde_json::Value = serde_json::from_slice(&local).unwrap();
        match engine {
            Engine::Xray => {
                assert_eq!(original["log"]["loglevel"], "none");
                assert_eq!(
                    local["log"],
                    serde_json::json!({"loglevel":"info","access":"none"})
                );
            }
            Engine::SingBox => {
                assert_eq!(original["log"]["disabled"], true);
                assert_eq!(
                    local["log"],
                    serde_json::json!({"disabled":false,"level":"info","timestamp":false})
                );
            }
        }
    }
}
