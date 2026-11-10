#![cfg(target_os = "linux")]
#[path = "backend.rs"]
mod support;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

#[test]
fn source_batch_pages_failures_without_credentials() {
    let directory = support::Directory::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("HTTP fixture");
    let url = format!("http://{}/fixture", listener.local_addr().expect("адрес"));
    let closed = TcpListener::bind("127.0.0.1:0").expect("закрытый endpoint");
    let port = closed.local_addr().expect("адрес").port();
    drop(closed);
    use std::fmt::Write as _;
    let body = (0..250).fold(String::new(), |mut text, id| {
        writeln!(
            &mut text,
            "trojan://fixture-password{id}@127.0.0.1:{port}#Сервер{id}"
        )
        .expect("Строка fixture");
        text
    });
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("HTTP");
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).expect("запрос");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("ответ");
    });
    let mut app = support::Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"import_source", "name":"Fixture", "url":url, "local_exception":true,"route":"direct"}));
    app.wait(|v| v["total"] == 250 && v["busy"] == false);
    app.send(json!({"command":"check_batch"}));
    let done = app.wait(|v| v["batch"]["completed"] == 250 && v["batch"]["busy"] == false);
    assert_eq!(
        done["batch"]["rows"].as_array().expect("страница").len(),
        32
    );
    assert!(!done.to_string().contains("fixture-password"));
    assert!(
        done["batch"]["rows"]
            .as_array()
            .expect("строки")
            .iter()
            .all(|row| row["connect_ms"].is_null())
    );
    app.send(json!({"command":"batch_page", "offset":224}));
    let page = app.wait(|v| v["batch"]["offset"] == 224);
    assert_eq!(
        page["batch"]["rows"].as_array().expect("страница").len(),
        26
    );
    let original = done["source_id"].as_i64().expect("источник");
    app.send(json!({"command":"rename_source","id":original,"name":"Переименованный"}));
    let renamed =
        app.wait(|v| v["source_name"] == "Переименованный" && v["batch"]["invalidated"] == true);
    assert!(
        renamed["batch"]["rows"]
            .as_array()
            .expect("строки")
            .is_empty()
    );
    app.send(json!({"command":"add_source", "name":"Другой", "url":"http://127.0.0.1:1/unused", "local_exception":true, "route":"direct"}));
    let stale = app.wait(|v| v["source_id"] != original && v["batch"]["invalidated"] == true);
    assert!(
        stale["batch"]["rows"]
            .as_array()
            .expect("страница")
            .is_empty()
    );
    app.send(json!({"command":"select_source", "id":original}));
    app.wait(|v| v["source_id"] == original && v["total"] == 250);
    app.send(json!({"command":"check_batch"}));
    app.wait(|v| v["batch"]["busy"] == true);
    app.send(json!({"command":"cancel_batch"}));
    app.wait(|v| v["batch"]["cancelled"] == true && v["batch"]["busy"] == false);
    app.close();
    server.join().expect("fixture завершена");
}

#[test]
fn source_batch_skips_disappeared_favorite_and_keeps_available_results() {
    let directory = support::Directory::new();
    let http = TcpListener::bind("127.0.0.1:0").expect("свой HTTP fixture");
    let url = format!("http://{}/fixture", http.local_addr().expect("адрес"));
    let closed = TcpListener::bind("127.0.0.1:0").expect("свой endpoint");
    let port = closed.local_addr().expect("адрес").port();
    drop(closed);
    let a = format!("trojan://fixture-A@127.0.0.1:{port}#A");
    let b = format!("trojan://fixture-B@127.0.0.1:{port}#B");
    let server = thread::spawn(move || {
        for body in [format!("{a}\n{b}"), b] {
            let (mut stream, _) = http.accept().expect("HTTP");
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .expect("deadline");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                assert_eq!(stream.read(&mut byte).expect("заголовок"), 1);
                request.push(byte[0]);
                assert!(request.len() <= 16384);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("ответ");
        }
    });
    let mut app = support::Harness::new(directory.config());
    app.wait(|v| v["ready"] == true);
    app.send(json!({"command":"import_source","name":"favorite fixture","url":url,"local_exception":true,"route":"direct"}));
    let imported = app.wait(|v| v["total"] == 2 && v["busy"] == false);
    let a = imported["profiles"]
        .as_array()
        .expect("профили")
        .iter()
        .find(|p| p["name"] == "A")
        .expect("A")["id"]
        .clone();
    let b = imported["profiles"]
        .as_array()
        .expect("профили")
        .iter()
        .find(|p| p["name"] == "B")
        .expect("B")["id"]
        .clone();
    app.send(json!({"command":"favorite","id":a,"value":true}));
    app.wait(|v| {
        v["profiles"]
            .as_array()
            .is_some_and(|p| p.iter().any(|p| p["id"] == a && p["favorite"] == true))
    });
    app.send(json!({"command":"refresh"}));
    let refreshed =
        app.wait(|v| v["busy"] == false && (v["revision"] == 2 || v["pending"].is_object()));
    if refreshed["pending"].is_object() {
        app.send(json!({"command":"approve_refresh"}));
        app.wait(|v| v["revision"] == 2 && v["busy"] == false);
    }
    app.send(json!({"command":"check_batch"}));
    let done = app.wait(|v| v["batch"]["completed"] == 1 && v["batch"]["busy"] == false);
    assert_eq!(done["batch"]["total"], 1);
    assert_eq!(done["batch"]["rows"][0]["profile_id"], b);
    assert_eq!(done["batch"]["rows"][0]["revision"], 2);
    assert_ne!(done["error"], "refresh_required");
    app.send(json!({"command":"query","search":"","favorites_only":true,"offset":0}));
    let favorites = app.wait(|v| v["favorites_only"] == true);
    assert_eq!(favorites["profiles"][0]["id"], a);
    assert_eq!(favorites["profiles"][0]["available"], false);
    app.close();
    server.join().expect("fixture завершена");
}
