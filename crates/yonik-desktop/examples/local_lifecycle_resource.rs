//! Ограниченная компонентная приёмка actual native cancellation; production API неизменён.
use std::{error::Error, fs, io::Write, num::NonZeroU16, path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use yonik_engine::Engine;
use yonik_linux::{BinarySnapshot, Cancellation, CheckedConfig, LocalSession};
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
    for cycle in 0..1 {
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

fn identity(pid: u32) -> Result<Option<String>> {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(text) => Ok(Some(
            text.rsplit_once(") ")
                .ok_or("stat")?
                .1
                .split_whitespace()
                .nth(19)
                .ok_or("starttime")?
                .into(),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
async fn axis(dir: &Path, relative: &str, digest: &str, engine: Engine, axis: &str) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream = listener.local_addr()?.port();
    let report = yonik_import::import_bytes(format!("socks5://127.0.0.1:{upstream}").into_bytes())?;
    let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = NonZeroU16::new(reservation.local_addr()?.port()).ok_or("port")?;
    let config = yonik_engine::compile(&report.servers[0], engine, port)?;
    let (source, mut cancel) = Cancellation::new();
    let binary = BinarySnapshot::load(&dir.join(relative), digest.parse()?, engine)?;
    let checked = CheckedConfig::validate(binary, config, &mut cancel).await?;
    let mut session = LocalSession::new()?;
    drop(reservation);
    let mut tracked = Vec::new();
    let started;
    if axis == "startup_cancel" {
        let watch = async {
            loop {
                let children =
                    fs::read_to_string(format!("/proc/self/task/{}/children", std::process::id()))?;
                for child in children.split_whitespace() {
                    let pid: u32 = child.parse()?;
                    if let Some(start) = identity(pid)? {
                        source.send_replace(true);
                        return Ok::<_, Box<dyn Error>>((pid, start));
                    }
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        let (result, witness) = tokio::join!(
            session.start(checked, &mut cancel),
            timeout(Duration::from_secs(3), watch)
        );
        tracked.push(witness??);
        if result != Err(yonik_linux::HostError::Cancelled) {
            return Err(format!("Нужен exact Cancelled: {result:?}").into());
        }
        started = result.is_ok();
        if started {
            session.stop().await?;
            return Err("Startup успел завершиться до cancel: ось не принята".into());
        }
    } else {
        session.start(checked, &mut cancel).await?;
        started = true;
        let pid = session.engine_pid().ok_or("PID")?;
        tracked.push((pid, identity(pid)?.ok_or("core исчез")?));
        if axis == "positive" {
            let task = tokio::spawn(peer(listener));
            timeout(Duration::from_secs(3), traffic(port.get(), 0)).await??;
            timeout(Duration::from_secs(3), task).await???;
        } else if axis == "traffic_cancel" {
            // Listener удерживается, но upstream не отвечает: отмена во время живого I/O.
            let operation = traffic(port.get(), 0);
            tokio::pin!(operation);
            tokio::select! {
                result = &mut operation => return Err(format!("Раннее завершение traffic: {result:?}").into()),
                result = timeout(Duration::from_secs(3), async {
                    let (mut upstream, _) = listener.accept().await?;
                    let mut greeting = [0; 3];
                    upstream.read_exact(&mut greeting).await?;
                    if greeting != [5, 1, 0] { return Err(std::io::Error::other("Нет native SOCKS greeting")); }
                    source.send_replace(true);
                    Ok::<_, std::io::Error>(())
                }) => { result??; }
            }
            cancel.cancelled().await;
        } else {
            drop(listener);
            if timeout(Duration::from_secs(3), traffic(port.get(), 0))
                .await?
                .is_ok()
            {
                return Err("Отказ peer не обнаружен".into());
            }
        }
    }
    session.stop().await?;
    for (pid, start) in &tracked {
        if identity(*pid)?.as_ref() == Some(start) {
            return Err("Core не reaped".into());
        }
    }
    if session.engine_pid().is_some() || session.state() != yonik_application::State::Disconnected {
        return Err("Stop не подтверждён".into());
    }
    writeln!(
        std::io::stdout().lock(),
        "{}",
        serde_json::json!({"engine":format!("{engine:?}"),"axis":axis,"started":started,"tracked":tracked,"stop_ack":true})
    )?;
    Ok(())
}
fn main() -> Result<()> {
    let dir = std::env::args_os().nth(1).ok_or("Каталог cores")?;
    let axis_name = std::env::args().nth(2).ok_or("Ось")?;
    let engine_index: usize = std::env::args().nth(3).ok_or("Индекс")?.parse()?;
    if ![
        "positive",
        "startup_cancel",
        "traffic_cancel",
        "peer_failure",
    ]
    .contains(&axis_name.as_str())
    {
        return Err("Неизвестная ось".into());
    }
    let (relative, digest, engine) = *PINS.get(engine_index).ok_or("Индекс ядра")?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(axis(Path::new(&dir), relative, digest, engine, &axis_name))
}
