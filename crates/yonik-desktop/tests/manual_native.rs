//! Приёмка закрытой grammar формы через encrypted repository и оба pinned core.
use serde_json::json;
use std::{fs, num::NonZeroU16, os::unix::fs::DirBuilderExt};
use yonik_linux::{Cancellation, EncryptedSubscriptionRepository, SubscriptionService};
use zeroize::Zeroizing;

#[tokio::test]
#[ignore = "Требует YONIK_LOCAL_CORE_ROOT с обоими pinned cores"]
async fn manual_matrix_repository_reopen_native() {
    let root = std::env::temp_dir().join(format!("yonik-manual-native-{}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .expect("приватный каталог");
    let engines = std::path::PathBuf::from(std::env::var("YONIK_LOCAL_CORE_ROOT").expect("cores"));
    let mut cases = Vec::new();
    for protocol in ["vless", "vmess", "trojan", "shadowsocks", "socks", "http"] {
        let mut value =
            json!({"type":protocol,"tag":"manual","server":"example.com","server_port":443});
        match protocol {
            "vless" | "vmess" => value["uuid"] = json!("11111111-2222-4333-8444-555555555555"),
            _ => value["password"] = json!("synthetic-manual-secret"),
        }
        if protocol == "vmess" {
            value["security"] = json!("auto");
            value["alter_id"] = json!(0);
        }
        if protocol == "shadowsocks" {
            value["method"] = json!("aes-128-gcm");
        }
        if matches!(protocol, "socks" | "http") {
            value["username"] = json!("alice");
        }
        if protocol == "socks" {
            value["version"] = json!("5");
        }
        if matches!(protocol, "trojan" | "vless") {
            value["tls"] = json!({"enabled":true,"server_name":"tls.example.com"});
        }
        cases.push(value.clone());
        if matches!(protocol, "vless" | "vmess" | "trojan") {
            for transport in [
                json!({"type":"ws","path":"/ws","headers":{"Host":"cdn.example.com"}}),
                json!({"type":"grpc","service_name":"service-name"}),
            ] {
                let mut transported = value.clone();
                transported["tls"] = json!({"enabled":true,"server_name":"tls.example.com","alpn":["h2","http/1.1"]});
                transported["transport"] = transport;
                cases.push(transported);
            }
        }
    }
    for (index, outbound) in cases.into_iter().enumerate() {
        let case = root.join(index.to_string());
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&case)
            .expect("case");
        let database = case.join("repository.sqlite");
        let repo =
            EncryptedSubscriptionRepository::open(database.clone(), Zeroizing::new([93; 32]))
                .await
                .expect("open");
        repo.close().await.expect("close");
        let backup = case.join("backup.sqlite");
        EncryptedSubscriptionRepository::backup_closed(
            database.clone(),
            backup.clone(),
            Zeroizing::new([93; 32]),
        )
        .await
        .expect("backup");
        let repo =
            EncryptedSubscriptionRepository::open(database.clone(), Zeroizing::new([93; 32]))
                .await
                .expect("open");
        let service = SubscriptionService::new(repo.clone());
        let original = serde_json::to_vec(&json!({"outbounds":[outbound]})).expect("json");
        let report = yonik_import::import_bytes(original.clone()).expect("import");
        assert_eq!(report.servers.len(), 1);
        let (_sender, cancel) = Cancellation::new();
        let id = service
            .approve_local_import("manual".into(), &report, cancel, Some(backup))
            .await
            .expect("approve");
        drop(service);
        repo.close().await.expect("close");
        let bytes = fs::read(&database).expect("db");
        assert!(
            !bytes
                .windows(b"synthetic-manual-secret".len())
                .any(|w| w == b"synthetic-manual-secret")
        );
        let repo = EncryptedSubscriptionRepository::open(database, Zeroizing::new([93; 32]))
            .await
            .expect("reopen");
        let service = SubscriptionService::new(repo.clone());
        let snapshot = service.offline(id).await.expect("offline");
        assert_eq!(snapshot.report.original().expose().as_bytes(), original);
        let server = yonik_import::ServerIntent::from_server(&snapshot.report.servers[0])
            .expect("intent")
            .into_server()
            .expect("server");
        for (engine, relative) in [
            (yonik_engine::Engine::Xray, "xray-26.10.31/xray"),
            (yonik_engine::Engine::SingBox, "sing-box-1.14.4/sing-box"),
        ] {
            let config =
                yonik_engine::compile(&server, engine, NonZeroU16::new(10808).expect("port"))
                    .expect("compile");
            let mut emitted = Vec::new();
            config
                .write_sensitive(&mut emitted)
                .expect("конфигурация для assertions");
            let value: serde_json::Value = serde_json::from_slice(&emitted).expect("JSON core");
            let native = &value["outbounds"][0];
            let network = match index {
                1 | 4 | 7 => "ws",
                2 | 5 | 8 => "grpc",
                _ => "tcp",
            };
            let protected = matches!(index, 0..=2 | 4..=8);
            if relative.starts_with("xray") {
                let endpoint = if index < 6 {
                    &native["settings"]["vnext"][0]
                } else {
                    &native["settings"]["servers"][0]
                };
                assert_eq!(endpoint["address"], "example.com");
                assert_eq!(endpoint["port"], 443);
                if index < 6 {
                    assert_eq!(
                        endpoint["users"][0]["id"],
                        "11111111-2222-4333-8444-555555555555"
                    );
                }
                if (6..=9).contains(&index) {
                    assert_eq!(endpoint["password"], "synthetic-manual-secret");
                }
                if index >= 10 {
                    assert_eq!(endpoint["users"][0]["user"], "alice");
                    assert_eq!(endpoint["users"][0]["pass"], "synthetic-manual-secret");
                }
                if index == 9 {
                    assert_eq!(endpoint["method"], "aes-128-gcm");
                }
                let stream = &native["streamSettings"];
                assert_eq!(stream["network"], network);
                if protected {
                    assert_eq!(stream["tlsSettings"]["serverName"], "tls.example.com");
                }
                if network == "ws" {
                    assert_eq!(stream["wsSettings"]["path"], "/ws");
                    assert_eq!(stream["wsSettings"]["host"], "cdn.example.com");
                }
                if network == "grpc" {
                    assert_eq!(stream["grpcSettings"]["serviceName"], "service-name");
                }
                if network != "tcp" {
                    assert_eq!(stream["tlsSettings"]["alpn"], json!(["h2", "http/1.1"]));
                }
            } else {
                assert_eq!(native["server"], "example.com");
                assert_eq!(native["server_port"], 443);
                if index < 6 {
                    assert_eq!(native["uuid"], "11111111-2222-4333-8444-555555555555");
                }
                if index >= 6 {
                    assert_eq!(native["password"], "synthetic-manual-secret");
                }
                if index >= 10 {
                    assert_eq!(native["username"], "alice");
                }
                if index == 9 {
                    assert_eq!(native["method"], "aes-128-gcm");
                }
                if protected {
                    assert_eq!(native["tls"]["server_name"], "tls.example.com");
                }
                if network != "tcp" {
                    assert_eq!(native["transport"]["type"], network);
                    assert_eq!(native["tls"]["alpn"], json!(["h2", "http/1.1"]));
                }
                if network == "ws" {
                    assert_eq!(native["transport"]["path"], "/ws");
                    assert_eq!(native["transport"]["headers"]["Host"], "cdn.example.com");
                }
                if network == "grpc" {
                    assert_eq!(native["transport"]["service_name"], "service-name");
                }
            }
            let path = case.join("config.json");
            config
                .write_sensitive(&mut fs::File::create(&path).expect("file"))
                .expect("config");
            let mut command = std::process::Command::new(engines.join(relative));
            if relative.starts_with("xray") {
                command.args(["run", "-test", "-config"]);
            } else {
                command.args(["check", "-c"]);
            }
            assert!(
                command.arg(path).output().expect("core").status.success(),
                "case {index}"
            );
        }
        drop(service);
        repo.close().await.expect("close");
    }
    fs::remove_dir_all(root).expect("очистка собственного fixture");
}
