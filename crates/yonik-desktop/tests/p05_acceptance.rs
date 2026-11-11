#![cfg(target_os = "linux")]
#![allow(dead_code)]
#[path = "backend.rs"]
mod backend;
use backend::{Directory, Harness, сделать_overdue};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

#[test]
fn p05_host_manual_scheduled_матрица() {
    for scheduled in [false, true] {
        for case in [
            "full",
            "credentials",
            "partial",
            "empty",
            "corrupt",
            "http",
            "304",
            "size",
        ] {
            let dir = Directory::new();
            let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
            listener.set_nonblocking(true).expect("bounded accept");
            let url = format!("http://{}/sub", listener.local_addr().expect("адрес"));
            let server = thread::spawn(move || {
                for index in 0..2 {
                    let deadline = Instant::now() + Duration::from_secs(15);
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(Instant::now() < deadline, "ограниченное ожидание запроса");
                                thread::sleep(Duration::from_millis(5));
                            }
                            Err(error) => panic!("accept: {error}"),
                        }
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .expect("timeout");
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        assert_eq!(stream.read(&mut byte).expect("запрос"), 1);
                        request.push(byte[0]);
                        assert!(request.len() < 32768);
                    }
                    let body = if index == 0 {
                        "trojan://synthetic-old@example.invalid:443#old"
                    } else {
                        match case {
                            "full" => "trojan://synthetic-old@example.invalid:443#renamed",
                            "credentials" => "trojan://synthetic-new@example.invalid:443#renamed",
                            "partial" => {
                                "trojan://synthetic-old@example.invalid:443#renamed\nunknown://broken"
                            }
                            "corrupt" => "unknown://broken",
                            _ => "",
                        }
                    };
                    let response = if index == 1 {
                        match case {
                            "http" => "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".into(),
                            "304" => "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into(),
                            "size" => "HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n".into(),
                            _ => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: baseline\r\nConnection: close\r\n\r\n{body}", body.len()),
                        }
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: baseline\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    };
                    stream.write_all(response.as_bytes()).expect("ответ");
                }
            });
            let mut app = Harness::new(dir.config());
            app.wait(|v| v["ready"] == true);
            app.send(json!({"command":"import_source","name":"P05","url":url,"local_exception":true,"route":"direct"}));
            let initial = app.wait(|v| v["total"] == 1 && v["busy"] == false);
            let source = initial["source_id"].clone();
            let profile = initial["profiles"][0]["id"].clone();
            if scheduled {
                app.close();
                сделать_overdue(&dir);
                app = Harness::new(dir.config());
                app.wait(|v| v["ready"] == true);
                app.send(json!({"command":"subscription_visibility","visible":true}));
                app.host
                    .set_subscription_policy(yonik_linux::SubscriptionPolicy {
                        online: true,
                        ..Default::default()
                    })
                    .expect("policy");
            } else {
                app.send(json!({"command":"refresh"}));
            }
            if matches!(case, "full" | "credentials" | "partial") {
                if scheduled || case == "partial" {
                    let pending = app.wait(|v| !v["pending"].is_null());
                    assert_eq!(pending["total"], 1);
                    assert_eq!(pending["profiles"][0]["id"], profile);
                    assert_eq!(pending["pending"]["accepted"], 1);
                    assert_eq!(
                        pending["pending"]["rejected"],
                        usize::from(case == "partial")
                    );
                    app.send(json!({"command":"approve_refresh"}));
                }
                let updated =
                    app.wait(|v| v["pending"].is_null() && v["profiles"][0]["name"] == "renamed");
                assert_eq!(updated["source_id"], source);
                assert_eq!(
                    updated["profiles"][0]["id"] == profile,
                    case != "credentials"
                );
            } else if case == "304" {
                // 304 сохраняет snapshot; ожидание ответа подтверждается join сервера.
                server.join().expect("fixture");
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime")
                    .block_on(async {
                        let repo = yonik_linux::EncryptedSubscriptionRepository::open(
                            dir.0.join("subscriptions.sqlite"),
                            zeroize::Zeroizing::new([77; 32]),
                        )
                        .await
                        .expect("БД");
                        let service = yonik_linux::SubscriptionService::new(repo);
                        let deadline = Instant::now() + Duration::from_secs(5);
                        let snapshot = loop {
                            let snapshot = service
                                .offline(source.as_i64().expect("ID"))
                                .await
                                .expect("offline");
                            if snapshot.revision == 2 {
                                break snapshot;
                            }
                            assert!(Instant::now() < deadline, "304 commit deadline");
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        };
                        assert_eq!(snapshot.revision, 2, "304 обработан до shutdown");
                        assert!(
                            snapshot.report.original().expose()
                                == "trojan://synthetic-old@example.invalid:443#old"
                        );
                        service.close().await.expect("закрытие");
                    });
                app.close();
                continue;
            } else {
                let failed = app.wait(|v| !v["error"].is_null() && v["busy"] == false);
                assert_eq!(failed["source_id"], source);
                assert_eq!(failed["profiles"][0]["id"], profile);
                assert!(failed["pending"].is_null());
            }
            app.close();
            server.join().expect("fixture");
        }
    }
}
