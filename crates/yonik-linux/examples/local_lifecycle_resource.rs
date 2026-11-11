//! Opt-in P-05: LocalSession, реальные pinned cores и синтетический SOCKS peer.

#[derive(Clone, Copy, Debug)]
struct Resources {
    fd: usize,
    threads: usize,
    pss: u64,
    private_dirty: u64,
}

fn verify_stable(rows: &[Resources], traffic: usize, reaped: usize) -> Result<(), &'static str> {
    if rows.len() != 100 || traffic != 100 || reaped != 100 {
        return Err("Нужны сто завершённых циклов с трафиком и reap");
    }
    let warmed = rows[9];
    for current in &rows[10..] {
        if current.fd > warmed.fd + 2 || current.threads > warmed.threads + 1 {
            return Err("Рост дескрипторов или потоков после прогрева");
        }
        if current.pss.saturating_sub(warmed.pss) > 16 * 1024 * 1024
            || current.private_dirty.saturating_sub(warmed.private_dirty) > 16 * 1024 * 1024
        {
            return Err("Рост удерживаемой памяти после прогрева больше 16 MiB");
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine_dir = std::env::args_os()
        .nth(1)
        .ok_or("Нужен каталог закреплённых ядер")?;
    let parent_netns = std::env::args()
        .nth(2)
        .ok_or("Нужен netns inode родителя")?
        .parse::<u64>()?;
    if std::env::args_os().count() != 3 {
        return Err("Нужны каталог ядер и netns inode родителя".into());
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(linux::run(std::path::Path::new(&engine_dir), parent_netns))
}

#[cfg(not(target_os = "linux"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Стенд доступен только на Linux".into())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        error::Error,
        fs,
        io::{Read, Write},
        num::NonZeroU16,
        os::unix::fs::MetadataExt,
        path::Path,
        time::{Duration, Instant},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        signal::unix::{SignalKind, signal},
        time::timeout,
    };
    use yonik_engine::Engine;
    use yonik_linux::event_log::{EventCode, EventLog, EventSource, LogFilter, MAX_PAGE};
    use yonik_linux::{BinarySnapshot, Cancellation, CheckedConfig, LocalSession};

    use super::{Resources, verify_stable};

    type Result<T> = std::result::Result<T, Box<dyn Error>>;
    const REQUEST: [u8; 10] = [5, 1, 0, 1, 198, 18, 0, 1, 70, 160];
    const PINS: [(&str, &str, Engine); 2] = [
        (
            "xray-26.10.31/xray",
            "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1",
            Engine::Xray,
        ),
        (
            "sing-box-1.14.4/sing-box",
            "1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a",
            Engine::SingBox,
        ),
    ];

    fn marker(cycle: u32) -> [u8; 16] {
        let mut value = [0; 16];
        value[..4].copy_from_slice(&cycle.to_be_bytes());
        value[4..].copy_from_slice(b"yonik-p05-ok");
        value
    }

    async fn peer(listener: TcpListener) -> std::io::Result<()> {
        for cycle in 0..100 {
            let (mut stream, _) = listener.accept().await?;
            let mut greeting = [0; 3];
            stream.read_exact(&mut greeting).await?;
            if greeting != [5, 1, 0] {
                return Err(std::io::Error::other("Некорректный greeting upstream"));
            }
            stream.write_all(&[5, 0]).await?;
            let mut request = [0; 10];
            stream.read_exact(&mut request).await?;
            if request != REQUEST {
                return Err(std::io::Error::other("Изменён адрес выбранного upstream"));
            }
            stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
            let mut payload = [0; 16];
            stream.read_exact(&mut payload).await?;
            if payload != marker(cycle) {
                return Err(std::io::Error::other("Не подтверждён уникальный marker"));
            }
            stream.write_all(&payload).await?;
            stream.shutdown().await?;
        }
        Ok(())
    }

    async fn traffic(port: u16, cycle: u32) -> Result<()> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(&[5, 1, 0]).await?;
        let mut greeting = [0; 2];
        stream.read_exact(&mut greeting).await?;
        if greeting != [5, 0] {
            return Err("Локальный SOCKS не подтвердил noauth".into());
        }
        stream.write_all(&REQUEST).await?;
        let mut header = [0; 4];
        stream.read_exact(&mut header).await?;
        if header[..3] != [5, 0, 0] {
            return Err("Ядро не подтвердило CONNECT через выбранный upstream".into());
        }
        match header[3] {
            1 => {
                let mut address = [0; 6];
                stream.read_exact(&mut address).await?;
            }
            4 => {
                let mut address = [0; 18];
                stream.read_exact(&mut address).await?;
            }
            _ => return Err("Некорректная форма bound address".into()),
        }
        stream.write_all(&marker(cycle)).await?;
        let mut reply = [0; 16];
        stream.read_exact(&mut reply).await?;
        if reply != marker(cycle) {
            return Err("Трафик не достиг собственного upstream".into());
        }
        Ok(())
    }

    fn resources() -> Result<Resources> {
        let fd = fs::read_dir("/proc/self/fd")?.count();
        let threads = fs::read_dir("/proc/self/task")?.count();
        let mut smaps = String::new();
        fs::File::open("/proc/self/smaps_rollup")?
            .take(64 * 1024 + 1)
            .read_to_string(&mut smaps)?;
        if smaps.len() > 64 * 1024 {
            return Err("smaps превышает лимит".into());
        }
        let field = |name: &str| -> Result<u64> {
            let line = smaps
                .lines()
                .find(|line| line.starts_with(name))
                .ok_or("Нет обязательного поля smaps")?;
            let kb: u64 = line
                .split_whitespace()
                .nth(1)
                .ok_or("Нет значения smaps")?
                .parse()?;
            kb.checked_mul(1024)
                .ok_or_else(|| "Переполнение smaps".into())
        };
        Ok(Resources {
            fd,
            threads,
            pss: field("Pss:")?,
            private_dirty: field("Private_Dirty:")?,
        })
    }

    fn process_start(pid: u32) -> Result<Option<String>> {
        let text = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let (_, tail) = text.rsplit_once(") ").ok_or("Некорректный stat процесса")?;
        Ok(Some(
            tail.split_whitespace()
                .nth(19)
                .ok_or("Нет starttime")?
                .to_owned(),
        ))
    }

    async fn cycles(engine_dir: &Path, upstream: u16, cancel: &mut Cancellation) -> Result<()> {
        let log = EventLog::new();
        let mut session = LocalSession::new()?.with_event_log(log.clone());
        let mut rows = Vec::with_capacity(100);
        for cycle in 0..100_u32 {
            cancel.check()?;
            let (relative, digest, engine) = PINS[cycle as usize % 2];
            let uri = format!("socks5://127.0.0.1:{upstream}");
            let report = yonik_import::import_bytes(uri.into_bytes())?;
            if report.servers.len() != 1 || !report.issues.is_empty() {
                return Err("Синтетический профиль не принят".into());
            }
            let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
            let port = NonZeroU16::new(reservation.local_addr()?.port()).ok_or("Нулевой порт")?;
            let config = yonik_engine::compile(&report.servers[0], engine, port)?;
            let snapshot =
                BinarySnapshot::load(&engine_dir.join(relative), digest.parse()?, engine)?;
            let checked = CheckedConfig::validate(snapshot, config, cancel).await?;
            let started = Instant::now();
            // Держим порт во время hash/version/config-check; освобождаем только перед spawn.
            drop(reservation);
            if let Err(error) = session.start(checked, cancel).await {
                session.stop().await?;
                let diagnostics = log
                    .read(
                        0,
                        LogFilter {
                            source: Some(EventSource::Engine),
                            code: Some(EventCode::ProcessExited),
                        },
                        MAX_PAGE,
                    )
                    .ok_or("Недоступен журнал failure")?
                    .export();
                return Err(format!("cycle={} engine={engine:?} port={port} stage=start error={error:?} {diagnostics}", cycle + 1).into());
            }
            let result = async {
                let pid = session.engine_pid().ok_or("Нет запущенного core")?;
                let original_start = process_start(pid)?.ok_or("Core исчез до traffic")?;
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err("Стенд отменён".into()),
                    result = timeout(Duration::from_secs(3), traffic(port.get(), cycle)) => {
                        match result {
                            Ok(value) => value?,
                            Err(_) => return Err("Таймаут traffic".into()),
                        }
                    }
                }
                Ok::<_, Box<dyn Error>>((pid, original_start))
            }
            .await;
            // Любая ошибка после успешного start проходит через явный Stop/reap.
            let stopped = session.stop().await;
            stopped?;
            let (pid, original_start) = result?;
            if session.engine_pid().is_some()
                || process_start(pid)?.as_ref() == Some(&original_start)
                || session.state() != yonik_application::State::Disconnected
            {
                return Err("Не подтверждён Stop/reap своего core".into());
            }
            let sample = resources()?;
            rows.push(sample);
            writeln!(
                std::io::stdout().lock(),
                "{}",
                serde_json::json!({
                    "cycle": cycle + 1, "engine": format!("{engine:?}"), "traffic": true,
                    "reaped": true, "state": "disconnected", "elapsed_ms": started.elapsed().as_millis(),
                    "fd": sample.fd, "threads": sample.threads, "pss": sample.pss,
                    "private_dirty": sample.private_dirty,
                })
            )?;
        }
        verify_stable(&rows, rows.len(), rows.len()).map_err(|error| error.into())
    }

    #[cfg(test)]
    mod tests {
        use super::{Duration, Result, TcpListener, TcpStream, marker, peer, timeout};
        use tokio::io::AsyncWriteExt;

        #[test]
        fn marker_is_fixed_size_and_binds_cycle() {
            assert_ne!(marker(0), marker(1));
            assert_eq!(&marker(99)[..4], &99_u32.to_be_bytes());
        }

        #[tokio::test]
        async fn peer_protocol_failure_is_propagated() -> Result<()> {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            let server = tokio::spawn(peer(listener));
            let mut stream = TcpStream::connect(address).await?;
            stream.write_all(&[5, 1, 2]).await?;
            assert!(timeout(Duration::from_secs(2), server).await??.is_err());
            Ok(())
        }
    }

    pub async fn run(engine_dir: &Path, parent_netns: u64) -> Result<()> {
        let current_netns = fs::metadata("/proc/self/ns/net")?.ino();
        let devices = fs::read_to_string("/proc/net/dev")?;
        let names: Vec<_> = devices
            .lines()
            .skip(2)
            .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim()))
            .collect();
        if current_netns == parent_netns || names != ["lo"] {
            return Err("Нужен отдельный пустой network namespace с loopback".into());
        }
        writeln!(
            std::io::stderr().lock(),
            "{}",
            serde_json::json!({"isolated_netns":current_netns, "parent_netns":parent_netns, "interfaces":names})
        )?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let upstream = listener.local_addr()?.port();
        let mut peer = tokio::spawn(peer(listener));
        let (source, mut cancel) = Cancellation::new();
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut terminate = signal(SignalKind::terminate())?;
        let signals = tokio::spawn(async move {
            tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
            source.send_replace(true);
        });
        let result = cycles(engine_dir, upstream, &mut cancel).await;
        signals.abort();
        let signal_result = signals.await;
        if let Err(error) = signal_result {
            if !error.is_cancelled() {
                return Err(error.into());
            }
        }
        if result.is_err() {
            peer.abort();
        }
        match timeout(Duration::from_secs(2), &mut peer).await {
            Ok(Ok(peer_result)) => peer_result?,
            Ok(Err(error)) if result.is_err() && error.is_cancelled() => {}
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {
                peer.abort();
                if let Err(error) = peer.await {
                    if !error.is_cancelled() {
                        return Err(error.into());
                    }
                }
                return Err("Таймаут завершения собственного peer".into());
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::{Resources, verify_stable};

    fn stable() -> Vec<Resources> {
        vec![
            Resources {
                fd: 12,
                threads: 1,
                pss: 8 * 1024 * 1024,
                private_dirty: 4 * 1024 * 1024
            };
            100
        ]
    }

    #[test]
    fn all_hundred_cycles_need_traffic_and_confirmed_reap() {
        let rows = stable();
        assert!(verify_stable(&rows, 100, 100).is_ok());
        assert!(verify_stable(&rows, 99, 100).is_err());
        assert!(verify_stable(&rows, 100, 99).is_err());
        assert!(verify_stable(&rows[..99], 100, 100).is_err());
    }

    #[test]
    fn growing_descriptors_or_threads_are_not_a_pass() {
        let mut rows = stable();
        rows[99].fd += 8;
        assert!(verify_stable(&rows, 100, 100).is_err());
        let mut rows = stable();
        rows[99].threads += 3;
        assert!(verify_stable(&rows, 100, 100).is_err());
    }

    #[test]
    fn large_retained_memory_growth_is_not_a_pass() {
        let mut rows = stable();
        rows[99].pss += 32 * 1024 * 1024;
        assert!(verify_stable(&rows, 100, 100).is_err());
        let mut rows = stable();
        rows[99].private_dirty += 32 * 1024 * 1024;
        assert!(verify_stable(&rows, 100, 100).is_err());
    }
}
