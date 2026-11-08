//! Синтетический ELF для timeout/cancel subprocess-проверок, не proxy engine.
use std::{io::Read, net::TcpListener, time::Duration};

fn block() -> ! {
    loop {
        std::thread::park_timeout(Duration::from_secs(60));
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = std::env::var("SSL_CERT_DIR").unwrap_or_default();
    if args.get(1).map(String::as_str) == Some("version") {
        if mode.ends_with("version-hang") {
            block();
        }
        println!("Xray 26.10.31");
        return;
    }
    let mut config = Vec::new();
    std::io::stdin()
        .take(512 * 1024)
        .read_to_end(&mut config)
        .expect("fixture stdin");
    if args.iter().any(|arg| arg == "-test") {
        if mode.ends_with("check-hang") {
            block();
        }
        return;
    }
    let _listener = TcpListener::bind("127.0.0.1:10808").expect("fixture listener");
    block();
}
