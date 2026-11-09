use std::{
    ffi::OsString,
    io::{self, Write},
    num::NonZeroU16,
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use serde_json::{Value, json};
use tokio::signal::unix::{SignalKind, signal};
use yonik_engine::{Engine, PolicyMode, compile, compile_policy, policy_input::read_policy};
use yonik_import::import_reader;
use yonik_linux::{
    BinarySnapshot, Cancellation, CheckedConfig, HostError, LocalSession, ProbeResult, ProbeTarget,
    Sha256,
};

struct Arguments {
    engine: Engine,
    binary: PathBuf,
    hash: Sha256,
    server: usize,
    port: NonZeroU16,
    target: ProbeTarget,
    once: bool,
    accept_partial: bool,
    policy: Option<(PathBuf, PolicyMode, NonZeroU16)>,
}

fn arguments() -> Result<Arguments, &'static str> {
    let mut args = std::env::args_os().skip(1);
    let name = args.next();
    let engine = match name.as_ref().and_then(|s| s.to_str()) {
        Some("xray") => Engine::Xray,
        Some("sing-box") => Engine::SingBox,
        _ => {
            return Err(
                "Использование: yonik-local xray|sing-box --binary PATH --sha256 HEX [--server N] [--accept-partial] [--listen-port PORT] [--check-url HTTPS_URL] [--check-timeout SECONDS] [--once]; тело подписки через stdin",
            );
        }
    };
    let mut options = std::collections::BTreeMap::<String, OsString>::new();
    let mut once = false;
    let mut accept_partial = false;
    while let Some(key) = args.next() {
        let key = key.to_str().ok_or("Некорректный аргумент")?;
        if key == "--once" && !once {
            once = true;
            continue;
        }
        if key == "--accept-partial" && !accept_partial {
            accept_partial = true;
            continue;
        }
        if ![
            "--binary",
            "--sha256",
            "--server",
            "--listen-port",
            "--check-url",
            "--check-timeout",
            "--check-status",
            "--policy",
            "--policy-mode",
            "--probe-port",
        ]
        .contains(&key)
        {
            return Err("Неизвестный или повторный аргумент");
        }
        let value = args.next().ok_or("Пропущено значение аргумента")?;
        if options.insert(key.into(), value).is_some() {
            return Err("Повторный аргумент");
        }
    }
    if accept_partial && !options.contains_key("--server") {
        return Err("Для частичного импорта нужен явный --server N");
    }
    let binary = PathBuf::from(options.remove("--binary").ok_or("Нужен путь --binary")?);
    let text = |key: &str| {
        options
            .get(key)
            .map(|v| v.to_str().ok_or("Некорректный аргумент"))
            .transpose()
    };
    let hash = text("--sha256")?
        .ok_or("Нужен ожидаемый --sha256 бинарника")?
        .parse()
        .map_err(|_| "Некорректный SHA-256")?;
    let server = text("--server")?
        .unwrap_or("0")
        .parse()
        .map_err(|_| "Некорректный индекс")?;
    let port = text("--listen-port")?
        .unwrap_or("10808")
        .parse()
        .map_err(|_| "Некорректный порт")?;
    let seconds: u64 = text("--check-timeout")?
        .unwrap_or("10")
        .parse()
        .map_err(|_| "Некорректный timeout")?;
    let expected_status = text("--check-status")?
        .unwrap_or("204")
        .parse()
        .map_err(|_| "Ожидаемый статус должен быть числом 200–299")?;
    let target = ProbeTarget::new(
        text("--check-url")?.unwrap_or("https://www.gstatic.com/generate_204"),
        Duration::from_secs(seconds),
    )
    .and_then(|target| target.with_expected_status(expected_status))
    .map_err(|_| "Некорректный HTTPS endpoint, ожидаемый статус или timeout")?;
    let policy = match (
        options.get("--policy"),
        text("--policy-mode")?,
        text("--probe-port")?,
    ) {
        (None, None, None) => None,
        (Some(path), Some(mode), Some(port)) => Some((
            PathBuf::from(path),
            mode.parse().map_err(|_| "Нужен режим names или ip-only")?,
            port.parse().map_err(|_| "Некорректный порт проверки")?,
        )),
        _ => {
            return Err("Нужны вместе --policy FILE --policy-mode names|ip-only --probe-port PORT");
        }
    };
    Ok(Arguments {
        engine,
        binary,
        hash,
        server,
        port,
        target,
        once,
        accept_partial,
        policy,
    })
}

fn emit(value: Value) -> Result<(), HostError> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &value).map_err(|_| HostError::Process)?;
    stdout
        .write_all(b"\n")
        .and_then(|_| stdout.flush())
        .map_err(|_| HostError::Process)
}

async fn run(args: Arguments, config: yonik_engine::CompiledConfig) -> Result<u8, HostError> {
    // Устанавливаем обработчики до первого subprocess; сигнал во время version
    // или config-check проходит тем же cancel/kill/wait, что и остановка сессии.
    let mut interrupt = signal(SignalKind::interrupt()).map_err(|_| HostError::Process)?;
    let mut terminate = signal(SignalKind::terminate()).map_err(|_| HostError::Process)?;
    let (source, mut cancel) = Cancellation::new();
    let signal_task = tokio::spawn(async move {
        tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
        let _ = source.send(true); // Получатель мог завершить обычный --once.
    });
    let operation = async {
        emit(json!({"state":"Preparing"}))?;
        let binary = BinarySnapshot::load(&args.binary, args.hash, args.engine)?;
        let checked = match CheckedConfig::validate(binary, config, &mut cancel).await {
            Ok(checked) => checked,
            Err(HostError::Cancelled) => { emit(json!({"state":"Disconnected"}))?; return Err(HostError::Cancelled); },
            Err(error) => return Err(error),
        };
        let mut session = LocalSession::new()?;
        let operation = async {
            session.start(checked, &mut cancel).await?;
            emit(json!({"state":format!("{:?}",session.state()),"pid":session.engine_pid(),"listen":"127.0.0.1","port":args.port.get()}))?;
            let result = session.check(&args.target, &mut cancel).await?;
            let probe = match result {
                ProbeResult::Response { status, elapsed } => json!({"kind":"http","status":status,"elapsed_ms":elapsed.as_millis()}),
                ProbeResult::Timeout => json!({"kind":"timeout"}),
                ProbeResult::Tls => json!({"kind":"tls"}),
                ProbeResult::Transport => json!({"kind":"transport"}),
            };
            emit(json!({"state":format!("{:?}",session.state()),"probe":probe}))?;
            if args.once { return Ok(if args.target.accepts(result) { 0 } else { 2 }); }
            session.wait_exit(&mut cancel).await?;
            Ok(0)
        }.await;
        // Cleanup не отменяется повторным сигналом: подтверждение только после wait.
        session.stop().await?;
        emit(json!({"state":format!("{:?}",session.state())}))?;
        operation
    }.await;
    signal_task.abort();
    if let Err(error) = signal_task.await {
        if !error.is_cancelled() {
            return Err(HostError::Process);
        }
    }
    operation
}

pub fn main() -> ExitCode {
    let result = (|| -> Result<u8, String> {
        let args = arguments()?;
        let report = import_reader(io::stdin().lock()).map_err(|e| e.to_string())?;
        if report.native_json.is_some() {
            return Err("Native JSON ещё не поддерживается host".into());
        }
        if !report.issues.is_empty() && !args.accept_partial {
            return Err("Есть отклонённые записи: проверьте yonik-import; явный выбор требует --accept-partial --server N".into());
        }
        let server = report.servers.get(args.server).ok_or("Индекс вне списка")?;
        let config = if let Some((path, mode, probe_port)) = &args.policy {
            use std::os::unix::fs::OpenOptionsExt;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
                .open(path)
                .map_err(|_| "Не удалось открыть политику")?;
            if !file
                .metadata()
                .map_err(|_| "Не удалось проверить файл политики")?
                .is_file()
            {
                return Err("Политика должна быть обычным файлом".into());
            }
            let policy = read_policy(file).map_err(|e| e.to_string())?;
            compile_policy(server, args.engine, args.port, *probe_port, &policy, *mode)
                .map_err(|e| e.to_string())?
        } else {
            compile(server, args.engine, args.port).map_err(|e| e.to_string())?
        };
        if !report.issues.is_empty() {
            emit(json!({"event":"partial_import","parsed":report.servers.len(),"rejected":report.issues.len(),"server_index":args.server})).map_err(|e| e.to_string())?;
        }
        drop(report);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "Не удалось создать runtime")?;
        match runtime.block_on(run(args, config)) {
            Ok(code) => Ok(code),
            Err(HostError::Cancelled) => Ok(0),
            Err(error) => Err(error.to_string()),
        }
    })();
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
