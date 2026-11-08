use std::{io, num::NonZeroU16, process::ExitCode};

use serde_json::json;
use tofu_engine::{Engine, compile};
use tofu_import::import_reader;

struct Arguments {
    engine: Engine,
    server: usize,
    port: NonZeroU16,
    emit: bool,
}

const DEFAULT_PORT: NonZeroU16 = match NonZeroU16::new(10808) {
    Some(port) => port,
    None => unreachable!(),
};

fn arguments() -> Result<Arguments, &'static str> {
    let mut args = std::env::args_os().skip(1);
    let engine_arg = args.next();
    let engine = match engine_arg.as_ref().and_then(|arg| arg.to_str()) {
        Some("xray") => Engine::Xray,
        Some("sing-box") => Engine::SingBox,
        _ => {
            return Err(
                "Использование: tofu-compile xray|sing-box [--server N] [--listen-port PORT] [--emit-secret-config]; источник через stdin",
            );
        }
    };
    let mut server = None;
    let mut port = None;
    let mut emit = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--server") if server.is_none() => {
                server = Some(
                    args.next()
                        .and_then(|v| v.to_str().and_then(|v| v.parse().ok()))
                        .ok_or("Некорректный индекс сервера")?,
                );
            }
            Some("--listen-port") if port.is_none() => {
                port = Some(
                    args.next()
                        .and_then(|v| v.to_str().and_then(|v| v.parse::<NonZeroU16>().ok()))
                        .ok_or("Некорректный порт")?,
                );
            }
            Some("--emit-secret-config") if !emit => emit = true,
            _ => return Err("Неизвестный или повторный аргумент"),
        }
    }
    Ok(Arguments {
        engine,
        server: server.unwrap_or(0),
        port: port.unwrap_or(DEFAULT_PORT),
        emit,
    })
}

fn run() -> Result<(), String> {
    let args = arguments()?;
    let report = import_reader(io::stdin().lock()).map_err(|e| e.to_string())?;
    if report.native_json.is_some() || !report.issues.is_empty() {
        return Err(
            "Нужен полный импорт URI без отклонённых записей; native JSON ещё не поддерживается"
                .into(),
        );
    }
    let server = report
        .servers
        .get(args.server)
        .ok_or("Индекс сервера вне списка")?;
    let config = compile(server, args.engine, args.port).map_err(|e| e.to_string())?;
    let stdout = io::stdout();
    let writer = stdout.lock();
    if args.emit {
        config
            .write_sensitive(writer)
            .map_err(|_| "Не удалось записать конфигурацию")?;
    } else {
        serde_json::to_writer(writer, &json!({
            "engine":match args.engine { Engine::Xray => "xray", Engine::SingBox => "sing-box" },
            "version":args.engine.version(), "server_index":args.server,
            "protocol":server.protocol().as_str(),"listen":"127.0.0.1","port":args.port.get(),
            "status":"compiled_not_checked",
        })).map_err(|_| "Не удалось записать сводку")?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
