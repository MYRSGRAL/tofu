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
    let body = (0..250)
        .map(|id| format!("trojan://fixture-password{id}@127.0.0.1:{port}#Сервер{id}\n"))
        .collect::<String>();
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
