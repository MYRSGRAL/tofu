//! Библиотечная приёмка; собирается отдельно, чтобы не менять граф зависимостей.
use serde_json::{Value, json};
use std::{num::NonZeroU16, path::Path, process::Command};
use yonik_engine::{Engine, compile};
use yonik_import::{ServerIntent, import_bytes};

fn main() {
    let output = std::env::args().nth(1).expect("Каталог evidence");
    let output = Path::new(&output);
    std::fs::create_dir_all(output).unwrap();
    for protocol in ["vless", "vmess", "trojan"] {
        for transport in ["ws", "grpc"] {
            let mut node = json!({"type":protocol,"server":"origin.example.test","server_port":8443,
                "tls":{"enabled":true,"server_name":"sni.example.test","alpn":["h2","http/1.1"]}});
            if protocol == "trojan" {
                node["password"] = json!("synthetic-native-password");
            } else {
                node["uuid"] = json!("00112233-4455-6677-8899-aabbccddeeff");
            }
            node["transport"] = if transport == "ws" {
                json!({"type":"ws","path":"/exact/path","headers":{"Host":"cdn.example.test:8443"}})
            } else {
                json!({"type":"grpc","service_name":"exact-service"})
            };
            let report =
                import_bytes(json!({"outbounds":[node]}).to_string().into_bytes()).unwrap();
            assert!(report.issues.is_empty());
            assert_eq!(report.servers.len(), 1);
            let intent = ServerIntent::from_server(&report.servers[0]).unwrap();
            let wire = serde_json::to_string(&intent).unwrap();
            let value: Value = serde_json::from_str(&wire).unwrap();
            assert_eq!(value["host"], "origin.example.test");
            assert_eq!(value["port"], 8443);
            assert_eq!(value["protocol"], protocol);
            if protocol == "trojan" {
                assert_eq!(
                    value["credentials"]["password"],
                    "synthetic-native-password"
                );
            } else {
                assert_eq!(
                    value["credentials"]["uuid"],
                    "00112233-4455-6677-8899-aabbccddeeff"
                );
            }
            assert_eq!(value["security"], "tls");
            assert_eq!(value["options"]["sni"], "sni.example.test");
            assert_eq!(value["options"]["alpn"], json!(["h2", "http/1.1"]));
            if transport == "ws" {
                assert_eq!(value["options"]["path"], "/exact/path");
                assert_eq!(value["options"]["host"], "cdn.example.test:8443");
            } else {
                assert_eq!(value["options"]["service_name"], "exact-service");
            }
            let server = ServerIntent::parse_json(&wire)
                .unwrap()
                .into_server()
                .unwrap();
            assert_eq!(
                wire,
                serde_json::to_string(&ServerIntent::from_server(&server).unwrap()).unwrap()
            );
            for (engine, name, binary) in [
                (
                    Engine::Xray,
                    "xray",
                    "/home/myrs/projects/yonik/local/engines/xray-26.10.31/xray",
                ),
                (
                    Engine::SingBox,
                    "sing-box",
                    "/home/myrs/projects/yonik/local/engines/sing-box-1.14.4/sing-box",
                ),
            ] {
                let config = compile(&server, engine, NonZeroU16::new(10808).unwrap()).unwrap();
                let mut bytes = Vec::new();
                config.write_sensitive(&mut bytes).unwrap();
                let original =
                    compile(&report.servers[0], engine, NonZeroU16::new(10808).unwrap()).unwrap();
                let mut original_bytes = Vec::new();
                original.write_sensitive(&mut original_bytes).unwrap();
                assert_eq!(bytes, original_bytes);
                let native: Value = serde_json::from_slice(&bytes).unwrap();
                let outbound = &native["outbounds"][0];
                if name == "xray" {
                    let stream = &outbound["streamSettings"];
                    assert_eq!(stream["security"], "tls");
                    assert_eq!(stream["tlsSettings"]["serverName"], "sni.example.test");
                    assert_eq!(stream["tlsSettings"]["alpn"], json!(["h2", "http/1.1"]));
                    assert_eq!(stream["network"], transport);
                    if transport == "ws" {
                        assert_eq!(
                            stream["wsSettings"],
                            json!({"host":"cdn.example.test:8443","path":"/exact/path"})
                        );
                    } else {
                        assert_eq!(
                            stream["grpcSettings"],
                            json!({"serviceName":"exact-service"})
                        );
                    }
                } else {
                    assert_eq!(
                        outbound["tls"],
                        json!({"enabled":true,"server_name":"sni.example.test","insecure":false,"alpn":["h2","http/1.1"]})
                    );
                    assert_eq!(outbound["transport"], node["transport"]);
                }
                let path = output.join(format!("{protocol}-{transport}-{name}.json"));
                std::fs::write(&path, bytes).unwrap();
                let mut command = Command::new(binary);
                if name == "xray" {
                    command.args(["run", "-test", "-config"]);
                } else {
                    command.args(["check", "-c"]);
                }
                let result = command.arg(&path).output().unwrap();
                std::fs::write(
                    output.join(format!("{protocol}-{transport}-{name}.log")),
                    [result.stdout, result.stderr].concat(),
                )
                .unwrap();
                assert!(result.status.success(), "{protocol}/{transport}/{name}");
                println!(
                    "PASS {protocol}/{transport}/{name}: exact roundtrip + compile + pinned native check"
                );
            }
        }
    }
}
