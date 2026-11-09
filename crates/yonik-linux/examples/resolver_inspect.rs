//! Read-only snapshot/watch systemd-resolved без адресов, доменов и SNI.
//! Факты daemon не подтверждают netns, Apply или strict DNS.

#[cfg(target_os = "linux")]
mod linux {
    use serde_json::json;
    use std::{io::Write, time::Duration};
    use yonik_linux::{
        Cancellation, GlobalModeKnown, ResolverError, ResolverScope, ResolverSnapshot, Supported,
    };

    const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

    fn error_code(error: ResolverError) -> &'static str {
        match error {
            ResolverError::Unavailable => "Unavailable",
            ResolverError::Unsupported => "Unsupported",
            ResolverError::PermissionDenied => "PermissionDenied",
            ResolverError::Cancelled => "Cancelled",
            ResolverError::Timeout => "Timeout",
            ResolverError::Malformed => "Malformed",
            ResolverError::Limit => "Limit",
            ResolverError::Closed => "Closed",
            ResolverError::Untrusted => "Untrusted",
            ResolverError::Exhausted => "Exhausted",
            ResolverError::NetworkChanged => "NetworkChanged",
        }
    }

    async fn emit(
        value: serde_json::Value,
        cancel: &mut Cancellation,
    ) -> Result<(), ResolverError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // Один незавершённый вывод: заблокированный stdout не удерживает reactor.
        // Thread не join-ится: при ошибке/отмене CLI завершается вместе с ним.
        std::thread::Builder::new()
            .name("resolver-output".into())
            .spawn(move || {
                let mut output = std::io::stdout().lock();
                let result = serde_json::to_writer(&mut output, &value)
                    .map_err(|_| ResolverError::Unavailable)
                    .and_then(|()| {
                        output
                            .write_all(b"\n")
                            .and_then(|()| output.flush())
                            .map_err(|_| ResolverError::Unavailable)
                    });
                let _ = sender.send(result);
            })
            .map_err(|_| ResolverError::Unavailable)?;
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ResolverError::Cancelled),
            _ = tokio::time::sleep(Duration::from_secs(1)) => Err(ResolverError::Timeout),
            result = receiver => result.map_err(|_| ResolverError::Unavailable)?,
        }
    }

    async fn emit_snapshot(
        snapshot: &ResolverSnapshot,
        cancel: &mut Cancellation,
    ) -> Result<(), ResolverError> {
        let entries = &snapshot.configuration.entries;
        // current_server может повторять элемент servers и не увеличивает счётчик.
        let servers: usize = entries
            .iter()
            .map(|entry| {
                entry.servers.as_ref().map_or(0, Vec::len)
                    + entry.fallback_servers.as_ref().map_or(0, Vec::len)
            })
            .sum();
        let domains: usize = entries
            .iter()
            .map(|entry| entry.search_domains.as_ref().map_or(0, Vec::len))
            .sum();
        let mode = entries
            .iter()
            .find(|entry| entry.scope == ResolverScope::Global)
            .and_then(|entry| entry.resolv_conf_mode.as_ref())
            .map(|mode| match mode {
                Supported::Known(GlobalModeKnown::Stub) => "stub",
                Supported::Known(GlobalModeKnown::Static) => "static",
                Supported::Known(GlobalModeKnown::Uplink) => "uplink",
                Supported::Known(GlobalModeKnown::Foreign) => "foreign",
                Supported::Known(GlobalModeKnown::Missing) => "missing",
                Supported::Unsupported(_) => "unsupported",
            });
        emit(
            json!({
                "entries": entries.len(),
                "servers": servers,
                "domains": domains,
                "resolv_conf_mode": mode,
                "incomplete": snapshot.configuration.incomplete,
                "streamed": snapshot.streamed,
                "sequence": snapshot.stamp.sequence(),
            }),
            cancel,
        )
        .await
    }

    async fn inspect(watch: bool, cancel: &mut Cancellation) -> Result<(), ResolverError> {
        if !watch {
            let snapshot = yonik_linux::read_resolver_snapshot(cancel, STARTUP_TIMEOUT).await?;
            return emit_snapshot(&snapshot, cancel).await;
        }
        let mut subscription = yonik_linux::subscribe_resolver(cancel, STARTUP_TIMEOUT).await?;
        emit_snapshot(subscription.last_observation(), cancel).await?;
        loop {
            emit_snapshot(subscription.next(cancel).await?, cancel).await?;
        }
    }

    async fn run(watch: bool) -> Result<(), ResolverError> {
        use tokio::signal::unix::{SignalKind, signal};
        let mut interrupt =
            signal(SignalKind::interrupt()).map_err(|_| ResolverError::Unavailable)?;
        let mut terminate =
            signal(SignalKind::terminate()).map_err(|_| ResolverError::Unavailable)?;
        let (sender, mut cancel) = Cancellation::new();
        let operation = inspect(watch, &mut cancel);
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => return result,
            _ = interrupt.recv() => {},
            _ = terminate.recv() => {},
        }
        sender.send_replace(true);
        operation.await
    }

    pub fn main() {
        let mut args = std::env::args_os().skip(1);
        let watch = match (args.next(), args.next()) {
            (None, None) => Ok(false),
            (Some(mode), None) if mode == "snapshot" => Ok(false),
            (Some(mode), None) if mode == "watch" => Ok(true),
            _ => Err(ResolverError::Malformed),
        };
        let result = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(async {
                let result = match watch {
                    Ok(watch) => run(watch).await,
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    let (_sender, mut cancel) = Cancellation::new();
                    let _ = emit(json!({"error": error_code(error)}), &mut cancel).await;
                }
                result
            }),
            Err(_) => Err(ResolverError::Unavailable),
        };
        if result.is_err() {
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    use std::io::Write;
    let _ = std::io::stdout().write_all(b"{\"error\":\"Unsupported\"}\n");
    std::process::exit(1);
}
