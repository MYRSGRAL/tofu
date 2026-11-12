//! Контролируемые loopback body и private policy bus, настоящий desktop scheduler.
#![deny(unsafe_code)]
#[allow(unsafe_code)]
#[path = "../src/bridge.rs"]
mod bridge;
#[path = "../src/bridge_impl.rs"]
mod bridge_impl;
#[path = "../src/dispatch_error.rs"]
mod dispatch_error;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{os::unix::fs::DirBuilderExt, path::PathBuf, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("Нужен root")?);
    let positive = std::env::args().nth(2).as_deref() == Some("valid");
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS")?;
    if std::env::var("DBUS_SYSTEM_BUS_ADDRESS")? != address {
        return Err("Нужен изолированный system bus".into());
    }
    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    let data = root.join("data");
    std::fs::DirBuilder::new().mode(0o700).create(&data)?;
    let mut key = zeroize::Zeroizing::new([0; 32]);
    getrandom::getrandom(key.as_mut()).map_err(|_| "random")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (bus, server, id) = runtime.block_on(async {
        eprintln!("PHASE bus-start");
        let bus = yonik_linux::policy_test_support::PolicyBus::new(&address).await?;
        eprintln!("PHASE bus-ready");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/subscription", listener.local_addr()?);
        let server = tokio::spawn(async move {
            for body in ["socks5://127.0.0.1:1080#Рабочий", if positive {"socks5://127.0.0.1:1080#Рабочий"} else {"{broken json"}] {
                tokio::time::timeout(Duration::from_secs(30), async {
                    let (mut stream, _) = listener.accept().await?;
                    let mut request = Vec::new(); let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        if stream.read(&mut byte).await? != 1 || request.len() >= 32768 { return Err(std::io::Error::other("HTTP bounds")); }
                        request.push(byte[0]);
                    }
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await
                }).await.map_err(|_| std::io::Error::other("server deadline"))??;
            }
            Ok::<_,std::io::Error>(())
        });
        let repo = yonik_linux::EncryptedSubscriptionRepository::open(data.join("subscriptions.sqlite"), zeroize::Zeroizing::new(*key)).await?;
        let service = yonik_linux::SubscriptionService::new(repo);
        let id = service.add(yonik_linux::SubscriptionSource::new("Импорт".into(),url,true,yonik_linux::FetchRoute::Direct)?).await?;
        eprintln!("PHASE seed-fetch");
        let (_sender,cancel) = tokio::sync::watch::channel(false);
        let yonik_linux::RefreshResult::Pending(pending) = service.refresh(id,Duration::from_secs(3),cancel).await? else {return Err("Нет seed".into());};
        service.commit(pending.accept_clean()?).await?;
        eprintln!("PHASE seed-committed");
        let token = service.schedule_metadata(id).await?.ok_or("metadata")?;
        service.persist_schedule(token,yonik_linux::ScheduleState {version:1,recorded_wall:1,due_wall:Some(1),failures:0,paused:false}).await?;
        tokio::time::timeout(Duration::from_secs(3), service.close()).await??;
        Ok::<_,Box<dyn std::error::Error>>((bus,server,id))
    })?;
    let config = yonik_desktop::HostConfig::new(data.clone(), root.join("engines"))
        .with_ephemeral_key(*key)
        .with_system_subscription_policy(true);
    bridge_impl::configure(config).map_err(|_| "Конфигурация уже задана")?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let driver = std::thread::spawn(move || {
        runtime.block_on(async {
            let _ = stopped.await;
        });
        runtime
    });
    eprintln!("PHASE gui-start");
    let code = bridge::ffi::run_application(
        &data.to_str().ok_or("UTF-8")?.into(),
        true,
        &"".into(),
        &"".into(),
        &"".into(),
    );
    let _ = stop.send(());
    let runtime = driver
        .join()
        .map_err(|_| "Поток runtime завершился аварийно")?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), server).await???;
        let repo = yonik_linux::EncryptedSubscriptionRepository::open(
            data.join("subscriptions.sqlite"),
            zeroize::Zeroizing::new(*key),
        )
        .await?;
        let saved = repo
            .load(id)
            .await?
            .ok_or("source")?
            .snapshot
            .ok_or("snapshot")?;
        if saved.body.as_slice() != "socks5://127.0.0.1:1080#Рабочий".as_bytes()
            || saved.revision != 1
        {
            return Err("original изменён".into());
        }
        tokio::time::timeout(
            Duration::from_secs(3),
            yonik_linux::SubscriptionService::new(repo).close(),
        )
        .await??;
        drop(bus);
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    if code != 0 {
        return Err(format!("Qt: {code}").into());
    }
    Ok(())
}
