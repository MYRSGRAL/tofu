use std::num::NonZeroU16;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tofu_engine::{CompileError, Engine, compile};
use tofu_import::import_bytes;

const UUID: &str = "00112233-4455-6677-8899-aabbccddeeff";

fn config(engine: Engine, uri: &str) -> Result<Value, CompileError> {
    let report = import_bytes(uri.as_bytes().to_vec()).expect("fixture импортируется");
    assert!(report.issues.is_empty());
    let config = compile(&report.servers[0], engine, NonZeroU16::new(10808).unwrap())?;
    let debug = format!("{config:?}");
    assert!(!debug.contains(UUID));
    assert!(!debug.contains("secret.example"));
    let mut bytes = Vec::new();
    config.write_sensitive(&mut bytes).unwrap();
    Ok(serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn vless_is_loopback_only_and_has_no_direct_fallback() {
    let uri = format!("vless://{UUID}@secret.example:443?encryption=none&security=tls");
    for engine in [Engine::Xray, Engine::SingBox] {
        let value = config(engine, &uri).unwrap();
        assert_eq!(value["inbounds"].as_array().unwrap().len(), 1);
        assert_eq!(value["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(value["outbounds"].as_array().unwrap().len(), 1);
        assert!(!value.to_string().contains("direct"));
        assert!(value.to_string().contains(UUID));
        assert!(value.to_string().contains("secret.example"));
    }
}

#[test]
fn tls_websocket_preserves_host_path_alpn_and_fingerprint() {
    let uri = format!(
        "vless://{UUID}@secret.example:443?security=tls&type=ws&sni=verify.example&host=edge.example&path=%2Fsocket&alpn=http%2F1.1&fp=chrome"
    );
    let xray = config(Engine::Xray, &uri).unwrap();
    let stream = &xray["outbounds"][0]["streamSettings"];
    assert_eq!(stream["wsSettings"]["path"], "/socket");
    assert_eq!(stream["wsSettings"]["host"], "edge.example");
    assert_eq!(stream["tlsSettings"]["serverName"], "verify.example");
    assert_eq!(stream["tlsSettings"]["allowInsecure"], false);
    assert_eq!(stream["tlsSettings"]["useSystemCA"], true);
    assert_eq!(stream["tlsSettings"]["alpn"], json!(["http/1.1"]));
    let sing = config(Engine::SingBox, &uri).unwrap();
    let outbound = &sing["outbounds"][0];
    assert_eq!(outbound["transport"]["headers"]["Host"], "edge.example");
    assert_eq!(outbound["tls"]["insecure"], false);
    assert_eq!(outbound["tls"]["utls"]["fingerprint"], "chrome");
}

#[test]
fn six_protocols_and_credentials_survive_compilation() {
    let vmess = json!({"v":"2","add":"secret.example","port":"443","id":UUID,
        "aid":"0","scy":"auto","net":"tcp","tls":""});
    let sources = [
        format!("vless://{UUID}@secret.example:443?security=tls"),
        format!("vmess://{}", STANDARD.encode(vmess.to_string())),
        "trojan://secret-pass@secret.example:443?sni=verify.example".into(),
        "ss://aes-128-gcm:secret-pass@secret.example:443".into(),
        "https://user:secret-pass@secret.example:443".into(),
        "socks5://user:secret-pass@secret.example:1080".into(),
    ];
    for engine in [Engine::Xray, Engine::SingBox] {
        for uri in &sources {
            let value = config(engine, uri).unwrap();
            assert!(value.to_string().contains("secret.example"));
        }
    }
}

#[test]
fn unsafe_unknown_and_inapplicable_options_never_silently_disappear() {
    let queries = [
        "unknown=secret",
        "allowInsecure=1",
        "encryption=unsupported",
        "flow=unknown",
        "headerType=http",
        "seed=1",
        "type=quic",
        "type=h2",
        "security=unknown",
        "host=unused.example",
        "path=%2Funused",
        "sni=unused.example",
        "fp=chrome",
        "pbk=unused",
        "sid=abcd",
        "mode=auto",
        "serviceName=unused",
        "spx=%2F",
        "security=tls&fp=unknown",
        "security=tls&alpn=h2,,http%2F1.1",
        "security=tls&sni=bad%0Aname",
        "type=ws&path=relative",
        "flow=xtls-rprx-vision",
        "packetEncoding=unknown",
    ];
    for engine in [Engine::Xray, Engine::SingBox] {
        for query in queries {
            assert!(
                config(engine, &format!("vless://{UUID}@127.0.0.1:443?{query}")).is_err(),
                "{engine:?}: {query}"
            );
        }
    }
}

#[test]
fn reality_validates_key_short_id_and_transport() {
    let key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let base = format!(
        "vless://{UUID}@secret.example:443?security=reality&sni=verify.example&fp=chrome&pbk={key}&sid=ab12&flow=xtls-rprx-vision"
    );
    for engine in [Engine::Xray, Engine::SingBox] {
        assert!(config(engine, &base).is_ok());
        for suffix in ["&type=ws", "&alpn=h2"] {
            assert!(config(engine, &format!("{base}{suffix}")).is_err());
        }
        for bad in [
            base.replace(key, "bad"),
            base.replace("sid=ab12", "sid=abc"),
            base.replace("sid=ab12", "sid=zz"),
            base.replace("&fp=chrome", ""),
        ] {
            assert!(config(engine, &bad).is_err());
        }
    }
}

#[test]
fn invalid_shadowsocks_2022_key_and_vmess_options_are_rejected() {
    for engine in [Engine::Xray, Engine::SingBox] {
        assert!(
            config(
                engine,
                "ss://2022-blake3-aes-128-gcm:bad@secret.example:443"
            )
            .is_err()
        );
        for extra in [
            json!({"aid":1}),
            json!({"scy":"none"}),
            json!({"type":"http"}),
            json!({"unknown":true}),
            json!({"host":123}),
        ] {
            let mut value = json!({"add":"secret.example","port":443,"id":UUID,"net":"tcp"});
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let uri = format!("vmess://{}", STANDARD.encode(value.to_string()));
            assert!(config(engine, &uri).is_err());
        }
    }
}

#[test]
fn grpc_and_xhttp_have_engine_specific_capabilities() {
    let grpc = format!("vless://{UUID}@secret.example:443?type=grpc&serviceName=svc&security=tls");
    for engine in [Engine::Xray, Engine::SingBox] {
        assert!(config(engine, &grpc).is_ok());
    }
    let xhttp = format!(
        "vless://{UUID}@secret.example:443?type=xhttp&path=%2Fstream&mode=auto&security=tls"
    );
    assert!(config(Engine::Xray, &xhttp).is_ok());
    assert_eq!(
        config(Engine::SingBox, &xhttp),
        Err(CompileError::UnsupportedTransport)
    );
}

#[test]
fn recursive_listener_and_public_xray_plaintext_are_rejected() {
    for engine in [Engine::Xray, Engine::SingBox] {
        assert_eq!(
            config(engine, &format!("vless://{UUID}@127.0.0.1:10808")),
            Err(CompileError::ListenerConflict)
        );
    }
    assert_eq!(
        config(Engine::Xray, &format!("vless://{UUID}@1.1.1.1:443")),
        Err(CompileError::UnsupportedSecurity)
    );
    assert!(config(Engine::Xray, &format!("vless://{UUID}@10.0.0.1:443")).is_ok());
}

#[test]
fn mapped_ipv4_and_localhost_dot_cannot_create_self_proxy() {
    for engine in [Engine::Xray, Engine::SingBox] {
        for address in ["[::ffff:127.0.0.1]", "localhost.", "LOCALHOST"] {
            assert_eq!(
                config(engine, &format!("socks5://{address}:10808")),
                Err(CompileError::ListenerConflict)
            );
        }
        assert!(config(engine, "socks5://[::ffff:127.0.0.1]:10809").is_ok());
    }
}
