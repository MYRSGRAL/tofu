//! Одно ручное TCP-измерение: профили только из stdin, индекс принятой записи — argv.

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    let mut arguments = std::env::args().skip(1);
    let index = arguments
        .next()
        .unwrap_or_else(|| "0".into())
        .parse::<usize>()?;
    if arguments.next().is_some() {
        return Err("Допустим только индекс принятого профиля; источник читается из stdin".into());
    }
    let report = tofu_import::import_reader(std::io::stdin().lock())?;
    let server = report
        .servers
        .get(index)
        .ok_or("Нет принятого профиля с этим индексом")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(measure(server))?;
    let millis = |value: std::time::Duration| value.as_millis();
    let summary = serde_json::json!({
        "server_index": index,
        "accepted": report.servers.len(),
        "rejected": report.issues.len(),
        "status": format!("{:?}", result.status),
        "dns_ms": result.dns_elapsed.map(millis),
        "connect_ms": result.connect_elapsed.map(millis),
    });
    writeln!(std::io::stdout().lock(), "{summary}")?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn measure(
    server: &tofu_import::ParsedServer,
) -> Result<tofu_linux::TcpProbeResult, tofu_linux::HostError> {
    use tofu_linux::{Cancellation, HostError, TcpProbeTarget, check_tcp};
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = signal(SignalKind::interrupt()).map_err(|_| HostError::Process)?;
    let mut terminate = signal(SignalKind::terminate()).map_err(|_| HostError::Process)?;
    let (source, mut cancel) = Cancellation::new();
    let target = TcpProbeTarget::for_server(server);
    let check = check_tcp(target, std::time::Duration::from_secs(5), &mut cancel);
    tokio::pin!(check);
    tokio::select! {
        biased;
        _ = interrupt.recv() => { source.send_replace(true); check.await }
        _ = terminate.recv() => { source.send_replace(true); check.await }
        result = &mut check => result,
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("TCP adapter доступен только в Linux".into())
}
