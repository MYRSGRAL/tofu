#![cfg(target_os = "linux")]
#![allow(dead_code)]
#[path = "backend.rs"]
mod backend;
use backend::{Directory, Harness};
#[path = "support/p05_service.rs"]
mod service_gate;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use yonik_linux::{
    EncryptedSubscriptionRepository, FetchHost, FetchRoute, RefreshResult, SubscriptionService,
    SubscriptionSource,
};
use zeroize::Zeroizing;

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}
async fn service(dir: &Directory) -> SubscriptionService {
    SubscriptionService::new(
        EncryptedSubscriptionRepository::open(
            dir.0.join("subscriptions.sqlite"),
            Zeroizing::new([77; 32]),
        )
        .await
        .expect("БД"),
    )
}
fn config(dir: &Directory) -> yonik_desktop::HostConfig {
    yonik_desktop::HostConfig::new(
        dir.0.clone(),
        PathBuf::from(std::env::var("YONIK_PROXY_ENGINE_DIR").expect("engines")),
    )
    .with_ephemeral_key([77; 32])
}
fn connect(app: &mut Harness, source: &Value) -> u16 {
    app.send(json!({"command":"set_engine","engine":std::env::var("YONIK_PROXY_ENGINE").expect("engine")}));
    app.wait(|v| v["engine"] == std::env::var("YONIK_PROXY_ENGINE").unwrap());
    app.send(json!({"command":"select_source","id":source}));
    let page = app.wait(|v| v["source_id"] == *source && v["total"] == 1);
    app.send(json!({"command":"select_profile","id":page["profiles"][0]["id"],"revision":page["revision"]}));
    app.wait(|v| v["selected_profile"].is_i64());
    app.send(json!({"command":"set_probe_target","url":std::env::var("YONIK_PROXY_PROBE_URL").unwrap(),"expected_status":204}));
    app.wait(|v| v["probe_custom"] == true);
    let dns = std::env::var("YONIK_PROXY_FETCH_DNS").unwrap();
    app.send(json!({"command":"set_proxy_fetch_dns","upstream":dns}));
    app.wait(|v| v["proxy_fetch_dns"] == dns);
    app.send(json!({"command":"connect"}));
    app.wait(|v| v["status"] == "Connected")["port"]
        .as_u64()
        .unwrap() as u16
}
fn source(url: String) -> SubscriptionSource {
    SubscriptionSource::new("P05".into(), url, true, FetchRoute::Proxy).expect("источник")
}

#[test]
#[ignore = "изолированный p05_native.py с pinned cores и own CA"]
fn p05_native_matrix() {
    let seed = std::env::var("P05_SEED_URL").unwrap();
    let base = std::env::var("P05_TARGET_URL").unwrap();
    let tls = std::env::var("P05_TLS_URL").unwrap();
    let bad_tls = std::env::var("P05_BAD_TLS_URL").unwrap();
    for scheduled in [false, true] {
        for case in [
            "full",
            "credentials",
            "partial",
            "empty",
            "corrupt",
            "http",
            "304",
            "redirect",
            "cross",
            "tls",
            "bad_tls",
            "size",
            "proxyfail",
            "cancel",
            "stale",
        ] {
            let dir = Directory::new();
            let mut app = Harness::new(config(&dir));
            app.wait(|v| v["ready"] == true);
            app.send(json!({"command":"import_source","name":"Bootstrap","url":seed,"local_exception":true,"route":"direct"}));
            let bootstrap =
                app.wait(|v| v["total"] == 1 && v["busy"] == false)["source_id"].clone();
            let port = connect(&mut app, &bootstrap);
            let socks = format!("127.0.0.1:{port}").parse().unwrap();
            let dns = std::env::var("YONIK_PROXY_FETCH_DNS")
                .unwrap()
                .parse()
                .unwrap();
            let host = Arc::new(
                FetchHost::new(Duration::from_secs(4))
                    .unwrap()
                    .with_socks_endpoint(socks)
                    .unwrap()
                    .with_proxy_dns(dns)
                    .unwrap(),
            );
            if !scheduled && case == "full" {
                run(service_gate::matrix(host.clone(), &base));
            }
            let id = run(async {
                let svc = service(&dir).await;
                let id = svc
                    .add(source(format!("{base}/old?token=old-canary")))
                    .await
                    .unwrap();
                let (_sender, cancel) = tokio::sync::watch::channel(false);
                let result = svc
                    .refresh_with_host(id, None, host.as_ref(), cancel)
                    .await
                    .unwrap();
                let RefreshResult::Pending(pending) = result else {
                    panic!("baseline");
                };
                svc.commit(pending.accept_clean().unwrap()).await.unwrap();
                svc.close().await.unwrap();
                id
            });
            let target = match case {
                "tls" => format!("{tls}/full"),
                "bad_tls" => format!("{bad_tls}/full"),
                "proxyfail" => std::env::var("P05_PROXY_FAIL_URL").unwrap(),
                "cancel" | "stale" => format!("{base}/partial"),
                _ => format!("{base}/{case}"),
            };
            app.send(json!({"command":"source_change","request_id":42,"id":id,"expected_generation":1,"name":"P05","url":format!("{target}?token=new-canary"),"local_exception":true,"route":"proxy"}));
            let ack = app.wait(|v| v["source_change_ack"]["request_id"] == 42);
            assert!(ack["source_change_ack"]["error"].is_null());
            assert_eq!(ack["source_change_ack"]["generation"], 2);
            app.close();
            run(async {
                let svc = service(&dir).await;
                if scheduled {
                    let token = svc.schedule_metadata(id).await.unwrap().unwrap();
                    svc.persist_schedule(
                        token,
                        yonik_linux::ScheduleState {
                            version: 1,
                            due_wall: Some(1),
                            recorded_wall: 1,
                            failures: 0,
                            paused: false,
                        },
                    )
                    .await
                    .unwrap();
                }
                svc.close().await.unwrap();
            });
            let mut app = Harness::new(config(&dir));
            app.wait(|v| v["ready"] == true);
            connect(&mut app, &bootstrap);
            app.send(json!({"command":"select_source","id":id}));
            let before = app.wait(|v| v["source_id"] == id && v["total"] == 1);
            let profile = before["profiles"][0]["id"].clone();
            if scheduled {
                app.send(json!({"command":"subscription_visibility","visible":true}));
                app.host
                    .set_subscription_policy(yonik_linux::SubscriptionPolicy {
                        online: true,
                        ..Default::default()
                    })
                    .unwrap();
            } else {
                app.send(json!({"command":"refresh"}));
            }
            let success = matches!(
                case,
                "full"
                    | "credentials"
                    | "partial"
                    | "redirect"
                    | "cross"
                    | "tls"
                    | "cancel"
                    | "stale"
            );
            if success {
                if scheduled || matches!(case, "partial" | "cancel" | "stale") {
                    let pending = app.wait(|v| !v["pending"].is_null());
                    assert_eq!(pending["profiles"][0]["id"], profile);
                    assert_eq!(pending["pending"]["accepted"], 1);
                    if case == "stale" {
                        run(async {
                            let svc = service(&dir).await;
                            svc.change_source(
                                id,
                                2,
                                source(format!("{base}/full?token=third-canary")),
                            )
                            .await
                            .unwrap();
                            svc.close().await.unwrap();
                        });
                    }
                    if case == "cancel" {
                        app.send(json!({"command":"source_change","request_id":43,"id":id,"expected_generation":2,"name":"P05","url":format!("{base}/full?token=cancel-canary"),"local_exception":true,"route":"proxy"}));
                        let ack = app.wait(|v| v["source_change_ack"]["request_id"] == 43);
                        assert!(ack["source_change_ack"]["error"].is_null());
                        assert!(ack["pending"].is_null());
                    } else {
                        app.send(json!({"command":"approve_refresh"}));
                    }
                    if case == "stale" {
                        app.wait(|v| !v["error"].is_null());
                    }
                }
                if !matches!(case, "cancel" | "stale") {
                    let updated = app
                        .wait(|v| v["pending"].is_null() && v["profiles"][0]["name"] == "rotated");
                    assert_eq!(updated["source_id"], id);
                    assert_eq!(
                        updated["profiles"][0]["id"] == profile,
                        case != "credentials"
                    );
                }
            } else {
                app.wait(|v| !v["error"].is_null() && v["busy"] == false);
            }
            app.send(json!({"command":"logs_read","after":"0","source":"all","request_id":"999"}));
            let exported = app.wait(|v| v["logs"]["request_id"] == "999");
            let text = exported["logs"]["export"]
                .as_str()
                .expect("экспорт страницы");
            for canary in ["old-canary", "new-canary", "cancel-canary", "third-canary"] {
                assert!(!text.contains(canary));
                assert!(!exported.to_string().contains(canary));
            }
            println!(
                "p05_negative_export case={case} scheduled={scheduled} bytes={} clean=true",
                text.len()
            );
            app.close();
            run(async {
                let svc = service(&dir).await;
                let snapshot = svc.offline(id).await.unwrap();
                assert_eq!(
                    snapshot.revision,
                    if success && !matches!(case, "cancel" | "stale") {
                        2
                    } else {
                        1
                    },
                    "случай {case}, scheduled={scheduled}"
                );
                svc.close().await.unwrap();
            });
            println!("p05_host case={case} scheduled={scheduled} passed");
        }
    }
}
