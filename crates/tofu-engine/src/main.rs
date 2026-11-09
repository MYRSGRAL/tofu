use std::{
    io,
    num::NonZeroU16,
    path::{Path, PathBuf},
    process::ExitCode,
};

use serde_json::json;
use tofu_engine::{Engine, PolicyMode, compile, compile_policy, policy_input::read_policy};
use tofu_import::import_reader;

struct Arguments {
    engine: Engine,
    server: usize,
    port: NonZeroU16,
    emit: bool,
    policy: Option<(PathBuf, PolicyMode, NonZeroU16)>,
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
    let mut policy = None;
    let mut policy_mode = None;
    let mut probe_port = None;
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
            Some("--policy") if policy.is_none() => {
                policy = Some(PathBuf::from(args.next().ok_or("Нужен файл политики")?))
            }
            Some("--policy-mode") if policy_mode.is_none() => {
                policy_mode = Some(
                    args.next()
                        .and_then(|v| v.to_str().and_then(|v| v.parse().ok()))
                        .ok_or("Нужен режим names или ip-only")?,
                )
            }
            Some("--probe-port") if probe_port.is_none() => {
                probe_port = Some(
                    args.next()
                        .and_then(|v| v.to_str().and_then(|v| v.parse().ok()))
                        .ok_or("Некорректный порт проверки")?,
                )
            }
            _ => return Err("Неизвестный или повторный аргумент"),
        }
    }
    let policy = match (policy, policy_mode, probe_port) {
        (None, None, None) => None,
        (Some(path), Some(mode), Some(port)) => Some((path, mode, port)),
        _ => {
            return Err("Нужны вместе --policy FILE --policy-mode names|ip-only --probe-port PORT");
        }
    };
    Ok(Arguments {
        engine,
        server: server.unwrap_or(0),
        port: port.unwrap_or(DEFAULT_PORT),
        emit,
        policy,
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
    let config = if let Some((path, mode, probe_port)) = &args.policy {
        let policy = read_policy(open_policy(path)?).map_err(|e| e.to_string())?;
        compile_policy(server, args.engine, args.port, *probe_port, &policy, *mode)
            .map_err(|e| e.to_string())?
    } else {
        compile(server, args.engine, args.port).map_err(|e| e.to_string())?
    };
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
            "probe_port":config.probe_port().get(),
            "status":"compiled_not_checked",
        })).map_err(|_| "Не удалось записать сводку")?;
    }
    Ok(())
}

fn open_policy(path: &Path) -> Result<std::fs::File, &'static str> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    let file = options
        .open(path)
        .map_err(|_| "Не удалось открыть политику")?;
    if !file
        .metadata()
        .map_err(|_| "Не удалось проверить файл политики")?
        .is_file()
    {
        return Err("Политика должна быть обычным файлом");
    }
    Ok(file)
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
