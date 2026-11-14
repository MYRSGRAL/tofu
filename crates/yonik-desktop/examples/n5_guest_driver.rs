//! Закрытый N5 driver; протокол не является runtime-разрешением.
#![forbid(unsafe_code)]
#[path = "n5_guest_driver/protocol.rs"]
mod protocol;
#[path = "n5_guest_driver/session.rs"]
mod session;

fn main() {
    // До согласования sealed fixture/grant нет чтений host, stdin или запуска Host.
    let valid = std::env::args().skip(1).collect::<Vec<_>>() == ["--receipt-protocol=n5-v1"];
    let reason = if valid {
        "EXECUTION_CLOSED"
    } else {
        "ARGUMENT_PROTOCOL"
    };
    println!(
        "{}",
        serde_json::json!({"schema":2,"stage":"denied","reason":reason})
    );
    std::process::exit(2);
}
