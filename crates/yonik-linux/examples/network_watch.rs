//! Машинный протокол native fixture: только непрозрачная эпоха и флаги.

#[cfg(target_os = "linux")]
use std::io::{self, Write};
#[cfg(target_os = "linux")]
use yonik_linux::{NetworkMonitor, NetworkSnapshot};

#[cfg(target_os = "linux")]
fn emit(snapshot: NetworkSnapshot, closed: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    serde_json::to_writer(
        &mut output,
        &serde_json::json!({
            "epoch": snapshot.stamp.to_string(),
            "routes": snapshot.coverage.routes,
            "resolver": snapshot.coverage.resolver,
            "manager": snapshot.coverage.manager,
            "exhausted": snapshot.exhausted,
            "closed": closed,
        }),
    )?;
    output.write_all(b"\n")?;
    output.flush()
}

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let monitor = NetworkMonitor::start()?;
    let mut watch = monitor.watch();
    emit(watch.current(), false)?;
    loop {
        tokio::select! {
            _ = terminate.recv() => break,
            _ = interrupt.recv() => break,
            result = watch.changed() => {
                result?;
                emit(watch.current(), false)?;
            }
        }
    }
    monitor.shutdown().await;
    // После join канал обязан закрыться; pending последнее значение допустимо.
    while watch.changed().await.is_ok() {}
    emit(watch.current(), true)?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {}
