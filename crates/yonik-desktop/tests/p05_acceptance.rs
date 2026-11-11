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

#[test]
fn p05_source_change_callback_cas_ack_и_закрытая_схема() {
    let dir = Directory::new();
    let mut app = Harness::new(dir.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"add_source","name":"P05","url":"https://example.invalid/old-canary","local_exception":false,"route":"proxy"}));
    let id = app.wait(|v| v["source_id"].is_i64())["source_id"].clone();
    let command = json!({"command":"source_change","request_id":71,"id":id,"expected_generation":1,"name":"P05","url":"https://example.invalid/new-canary","local_exception":false,"route":"proxy"});
    app.send(command.clone());
    let ack = app.wait(|v| v["source_change_ack"]["request_id"] == 71);
    assert_eq!(ack["source_change_ack"]["id"], id);
    assert_eq!(ack["source_change_ack"]["generation"], 2);
    assert!(ack["source_change_ack"]["error"].is_null());
    let mut stale = command.clone();
    stale["request_id"] = json!(72);
    app.send(stale);
    let rejected = app.wait(|v| v["source_change_ack"]["request_id"] == 72);
    assert!(!rejected["source_change_ack"]["error"].is_null());
    assert!(rejected["source_change_ack"]["generation"].is_null());
    let mut unknown = command.clone();
    unknown["unknown"] = json!(true);
    assert!(app.host.dispatch(&unknown.to_string()).is_err());
    let mut invalid = command;
    invalid["expected_generation"] = json!(0);
    assert!(app.host.dispatch(&invalid.to_string()).is_err());
    assert!(!ack.to_string().contains("canary"));
    assert!(!rejected.to_string().contains("canary"));
    app.close();
}

#[test]
fn p05_source_change_отменяет_fetch_до_мутации() {
    for scheduled in [false, true] {
        let dir = Directory::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/sub", listener.local_addr().unwrap());
        let (entered, barrier) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    assert_eq!(stream.read(&mut byte).unwrap(), 1);
                    request.push(byte[0]);
                    assert!(request.len() < 32768);
                }
                if index == 0 {
                    let body = "trojan://synthetic-old@example.invalid:443#old";
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).unwrap();
                } else {
                    entered.send(()).unwrap();
                    // Сначала наблюдаем закрытие клиентом: никакого позднего response
                    // до подтверждённой отмены сетевой операции.
                    assert_eq!(stream.read(&mut byte).unwrap(), 0);
                }
            }
        });
        let mut app = Harness::new(dir.config());
        app.wait(|v| v["ready"] == true);
        app.send(json!({"command":"import_source","name":"P05","url":url,"local_exception":true,"route":"direct"}));
        let initial = app.wait(|v| v["total"] == 1 && v["busy"] == false);
        let id = initial["source_id"].clone();
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
                .unwrap();
        } else {
            app.send(json!({"command":"refresh"}));
        }
        barrier.recv_timeout(Duration::from_secs(5)).unwrap();
        app.send(json!({"command":"source_change","request_id":91,"id":id,"expected_generation":1,"name":"P05","url":"https://example.invalid/new-canary","local_exception":false,"route":"proxy"}));
        let ack = app.wait(|v| v["source_change_ack"]["request_id"] == 91);
        assert!(ack["source_change_ack"]["error"].is_null());
        assert_eq!(ack["source_change_ack"]["generation"], 2);
        assert_eq!(ack["profiles"][0]["id"], profile);
        assert!(ack["pending"].is_null());
        assert_eq!(ack["busy"], false);
        app.close();
        server.join().unwrap();
    }
}

#[test]
fn p05_source_change_ack_сохраняет_generation_при_ошибке_расписания() {
    let dir = Directory::new();
    let mut app = Harness::new(dir.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"add_source","name":"P05","url":"https://example.invalid/old","local_exception":false,"route":"proxy"}));
    let id = app.wait(|v| v["source_id"].is_i64())["source_id"].clone();
    // CAS меняет generation; последующая запись расписания сохраняет её.
    let injected = std::process::Command::new("python3")
        .arg("-c")
        .arg("import sqlite3, sys; db = sqlite3.connect(sys.argv[1]); db.execute(\"CREATE TRIGGER fail_schedule BEFORE UPDATE OF payload ON sources WHEN OLD.generation = 2 AND NEW.generation = 2 BEGIN SELECT RAISE(ABORT, 'injected schedule failure'); END\"); db.commit()")
        .arg(dir.0.join("subscriptions.sqlite"))
        .status().expect("инъекция SQLite");
    assert!(injected.success());
    app.send(json!({"command":"source_change","request_id":93,"id":id,"expected_generation":1,"name":"P05","url":"https://example.invalid/new","local_exception":false,"route":"proxy"}));
    let ack = app.wait(|v| v["source_change_ack"]["request_id"] == 93);
    assert_eq!(ack["source_change_ack"]["generation"], 2);
    assert_eq!(ack["source_change_ack"]["error"], "storage");
    app.close();
}
