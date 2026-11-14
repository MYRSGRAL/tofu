//! P07: компонентный DesktopHost, удерживаемая health-проверка и синтетический журнал.
#[cfg(feature = "test-support")]
use std::{error::Error, io::Write, path::Path, time::Duration};
#[cfg(feature = "test-support")]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
#[cfg(feature = "test-support")]
use yonik_engine::Engine;
#[cfg(feature = "test-support")]
type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[cfg(feature = "test-support")]
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

#[cfg(feature = "test-support")]
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

#[cfg(feature = "test-support")]
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
    let (log_tx, log_rx) = std::sync::mpsc::channel();
    host.test_event_log(log_tx)?;
    let log = log_rx
        .recv_timeout(Duration::from_secs(3))?
        .ok_or("Нет журнала")?;
    send(serde_json::json!({"command":"connect"}))?;
    let (mut stream, _) = timeout(Duration::from_secs(8), listener.accept()).await??;
    let mut greeting = [0; 3];
    timeout(Duration::from_secs(3), stream.read_exact(&mut greeting)).await??;
    if greeting != [5, 1, 0] {
        return Err("Нет DesktopHost health greeting".into());
    }
    snapshot(&mut rx, |v| v["status"] == "Checking").await?;
    let started = std::time::Instant::now();
    let producer_log = log.clone();
    let producer = std::thread::spawn(move || {
        for _ in 0..100 {
            producer_log.test_fill();
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    send(serde_json::json!({"command":"check_tcp"}))?;
    snapshot(&mut rx, |v| {
        v["tcp_busy"] == true && v["status"] == "Checking"
    })
    .await?;
    send(
        serde_json::json!({"command":"import_text","name":"P07","text":"socks5://127.0.0.1:1080"}),
    )?;
    snapshot(&mut rx, |v| v["error"] == "busy").await?;
    let import_ms = started.elapsed().as_secs_f64() * 1000.0;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let overlap = log
        .read(0, yonik_linux::event_log::LogFilter::default(), usize::MAX)
        .ok_or("Журнал занят на overlap witness")?;
    if overlap.evicted == 0 {
        return Err("Нет лог-нагрузки до Stop".into());
    }
    let stopped = std::time::Instant::now();
    send(serde_json::json!({"command":"stop"}))?;
    let ack = snapshot(&mut rx, |v| v["status"] == "Disconnected").await?;
    let stop_ms = stopped.elapsed().as_secs_f64() * 1000.0;
    let mut tail = Vec::new();
    timeout(Duration::from_secs(3), stream.read_to_end(&mut tail)).await??;
    producer.join().map_err(|_| "Producer panic")?;
    let page = log
        .read(0, yonik_linux::event_log::LogFilter::default(), usize::MAX)
        .ok_or("Журнал занят")?;
    if page.events.len() > yonik_linux::event_log::MAX_PAGE || page.evicted == 0 {
        return Err("Нет bounded eviction/page".into());
    }
    writeln!(
        std::io::stdout().lock(),
        "{}",
        serde_json::json!({
            "component_only":true,"engine":name,"import_busy_ms":import_ms,
            "stop_snapshot_ms":stop_ms,"peer_eof":true,"log_batches":100,
            "events_attempted":200500,"batch_period_ms":10,"duration_ms":started.elapsed().as_secs_f64()*1000.0,
            "overlap_evicted_before_stop":overlap.evicted,"page_len":page.events.len(),"evicted":page.evicted,"rejected":page.rejected,
            "manual_tcp_started_while_health_held":true,"manual_health_held":true,"import_refused_busy":true
        })
    )?;
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
#[cfg(feature = "test-support")]
fn main() -> Result<()> {
    let dir = std::env::args_os().nth(1).ok_or("Каталог cores")?;
    let engine_index: usize = std::env::args().nth(3).ok_or("Индекс")?.parse()?;
    let (_, _, engine) = *PINS.get(engine_index).ok_or("Индекс ядра")?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(desktop(Path::new(&dir), engine))
}

#[cfg(not(feature = "test-support"))]
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    Err("Для компонентного fixture нужен feature test-support".into())
}
