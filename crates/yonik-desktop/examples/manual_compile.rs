//! Только синтетическая native-приёмка формы; production CLI не меняется.
use std::io::{self, Read};
use std::num::NonZeroU16;

fn main() -> Result<(), &'static str> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[1] != "--emit-secret-config" {
        return Err("Нужны engine и явный --emit-secret-config");
    }
    let engine = match args[0].as_str() {
        "xray" => yonik_engine::Engine::Xray,
        "sing-box" => yonik_engine::Engine::SingBox,
        _ => return Err("Неизвестное ядро"),
    };
    let mut bytes = Vec::new();
    io::stdin()
        .take((yonik_import::MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Чтение")?;
    let report = yonik_import::import_bytes(bytes).map_err(|_| "Импорт")?;
    if report.servers.len() != 1 || !report.issues.is_empty() {
        return Err("Нужен один принятый сервер");
    }
    let server = yonik_import::ServerIntent::from_server(&report.servers[0])
        .map_err(|_| "Intent")?
        .into_server()
        .map_err(|_| "Server")?;
    yonik_engine::compile(&server, engine, NonZeroU16::new(10808).ok_or("Порт")?)
        .map_err(|_| "Компиляция")?
        .write_sensitive(&mut io::stdout().lock())
        .map_err(|_| "Вывод")
}
