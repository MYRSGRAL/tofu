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
    session.start(checked, &mut cancel).await?;
    let pid = session.engine_pid().ok_or("PID")?;
    tracked.push((pid, identity(pid)?.ok_or("core исчез")?));
    let target =
        yonik_linux::ProbeTarget::new("https://198.18.0.1:18080/", Duration::from_secs(5))?;
    let operation = session.check(&target, &mut cancel);
    let witness = async {
        let (mut stream, _) = listener.accept().await?;
        let mut greeting = [0; 3];
        stream.read_exact(&mut greeting).await?;
        if greeting != [5, 1, 0] {
            return Err("Нет actual probe greeting".into());
        }
        if axis == "cancel" {
            source.send_replace(true);
            tokio::time::sleep(Duration::from_millis(100)).await;
        } else {
            stream.write_all(&[5, 0]).await?;
            let mut request = [0; 10];
            stream.read_exact(&mut request).await?;
            if request != REQUEST {
                return Err("Неверный probe destination".into());
            }
            if axis == "positive" {
                let tls_port: u16 = std::env::var("HEALTH_TLS_PORT")?.parse()?;
                let mut tls = TcpStream::connect(("127.0.0.1", tls_port)).await?;
                stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
                tokio::io::copy_bidirectional(&mut stream, &mut tls).await?;
            } else {
                stream.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            }
        }
        Ok::<_, Box<dyn Error>>(())
    };
    let (result, observed) = timeout(Duration::from_secs(8), async {
        tokio::join!(operation, witness)
    })
    .await?;
    observed?;
    let expected_state = if axis == "cancel" {
        if result != Err(yonik_linux::HostError::Cancelled) {
            return Err(format!("Не exact Cancelled: {result:?}").into());
        }
        yonik_application::State::Checking
    } else if axis == "positive" {
        if !target.accepts(result?) {
            return Err("HTTPS health не прошёл".into());
        }
        yonik_application::State::Connected
    } else {
        if result? != yonik_linux::ProbeResult::Transport {
            return Err("Нет Transport при отказе peer".into());
        }
        yonik_application::State::Limited
    };
    if session.state() != expected_state
        || session.engine_pid() != Some(pid)
        || identity(pid)?.is_none()
    {
        return Err("Нарушен actual active-session contract".into());
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
        serde_json::json!({"engine":format!("{engine:?}"),"axis":axis,"check_called":true,"active_preserved":true,"tracked":tracked,"stop_ack":true})
    )?;
    Ok(())
}

async fn snapshot(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value> {
    timeout(Duration::from_secs(8), async {
        loop {
            let raw = rx.recv().await.ok_or("Snapshot channel закрыт")?;
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            if predicate(&value) {
                return Ok(value);
            }
        }
    })
    .await?
}

async fn desktop(dir: &Path, engine: Engine) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream = listener.local_addr()?.port();
    let http = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://127.0.0.1:{}/", http.local_addr()?.port());
    let server = tokio::spawn(async move {
        let (mut stream, _) = http.accept().await?;
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await?);
            if request.len() > 16384 {
                return Err(std::io::Error::other("Слишком большой fixture request"));
            }
        }
        let body = format!("socks5://127.0.0.1:{upstream}");
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await?;
        Ok::<_, std::io::Error>(())
    });
    let data = std::env::var("HEALTH_DATA_DIR")?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let host = yonik_desktop::DesktopHost::start(
        yonik_desktop::HostConfig::new(data.into(), dir.into()).with_ephemeral_key([77; 32]),
        move |raw| {
            let _ = tx.send(raw);
        },
    )?;
    let send = |v: serde_json::Value| host.dispatch(&v.to_string());
    snapshot(&mut rx, |v| v["ready"] == true).await?;
    let name = if engine == Engine::Xray {
        "xray"
    } else {
        "sing-box"
    };
    send(serde_json::json!({"command":"set_engine","engine":name}))?;
    snapshot(&mut rx, |v| v["engine"] == name).await?;
    send(
        serde_json::json!({"command":"add_source","name":"Fixture","url":url,"local_exception":true,"route":"direct"}),
    )?;
    snapshot(&mut rx, |v| v["source_id"].is_i64()).await?;
    send(serde_json::json!({"command":"refresh"}))?;
    let imported = snapshot(&mut rx, |v| v["total"] == 1 && v["busy"] == false).await?;
    send(
        serde_json::json!({"command":"select_profile","id":imported["profiles"][0]["id"],"revision":imported["revision"]}),
    )?;
    snapshot(&mut rx, |v| v["selected_profile"].is_i64()).await?;
    send(
        serde_json::json!({"command":"set_probe_target","url":"https://198.18.0.1:18080/","expected_status":204}),
    )?;
    snapshot(&mut rx, |v| v["probe_custom"] == true).await?;
    send(serde_json::json!({"command":"connect"}))?;
    let (mut stream, _) = timeout(Duration::from_secs(8), listener.accept()).await??;
    let mut greeting = [0; 3];
    timeout(Duration::from_secs(3), stream.read_exact(&mut greeting)).await??;
    if greeting != [5, 1, 0] {
        return Err("Нет DesktopHost health greeting".into());
    }
    snapshot(&mut rx, |v| v["status"] == "Checking").await?;
    send(serde_json::json!({"command":"stop"}))?;
    let ack = snapshot(&mut rx, |v| v["status"] == "Disconnected").await?;
    host.shutdown();
    if !host.wait_closed(Duration::from_secs(5)) {
        return Err("DesktopHost не закрыт".into());
    }
    timeout(Duration::from_secs(3), server).await???;
    writeln!(
        std::io::stdout().lock(),
        "{}",
        serde_json::json!({"axis":"desktop_cancel","engine":name,"upstream_witness":true,"snapshot":ack,"request_id_ack_available":false})
    )?;
    Ok(())
}
fn main() -> Result<()> {
    let dir = std::env::args_os().nth(1).ok_or("Каталог cores")?;
    let axis_name = std::env::args().nth(2).ok_or("Ось")?;
    let engine_index: usize = std::env::args().nth(3).ok_or("Индекс")?.parse()?;
    if !["positive", "cancel", "peer_failure", "desktop_cancel"].contains(&axis_name.as_str()) {
        return Err("Неизвестная ось".into());
    }
    let (relative, digest, engine) = *PINS.get(engine_index).ok_or("Индекс ядра")?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            if axis_name == "desktop_cancel" {
                desktop(Path::new(&dir), engine).await
            } else {
                axis(Path::new(&dir), relative, digest, engine, &axis_name).await
            }
        })
}
