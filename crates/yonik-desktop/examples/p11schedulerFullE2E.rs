//! Настоящий Scheduler Full из валидного encrypted repository через общий production Bridge.
#![deny(unsafe_code)]
#[allow(unsafe_code)]
#[path = "../src/bridge.rs"]
mod bridge;
#[path = "../src/bridge_impl.rs"]
mod bridge_impl;
#[path = "../src/dispatch_error.rs"]
mod dispatch_error;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{os::unix::fs::DirBuilderExt, path::PathBuf};
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("Нужен отдельный root")?);
    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    let data = root.join("data");
    std::fs::DirBuilder::new().mode(0o700).create(&data)?;
    let mut key = zeroize::Zeroizing::new([0_u8; 32]);
    getrandom::getrandom(key.as_mut()).map_err(|_| "Ошибка random")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let repo = yonik_linux::EncryptedSubscriptionRepository::open(
            data.join("subscriptions.sqlite"),
            zeroize::Zeroizing::new(*key),
        )
        .await?;
        let service = yonik_linux::SubscriptionService::new(repo);
        // На одну запись больше capacity отдельного test-support host: отказ создаёт actual Scheduler.
        for index in 0..2 {
            service
                .add(yonik_linux::SubscriptionSource::new(
                    format!("Подписка {index}"),
                    format!("https://example.invalid/{index}"),
                    false,
                    yonik_linux::FetchRoute::Direct,
                )?)
                .await?;
        }
        service.close().await?;
        Ok::<_, yonik_linux::SubscriptionError>(())
    })?;
    drop(runtime);
    let config = yonik_desktop::HostConfig::ephemeral_scheduler_example(
        data.clone(),
        root.join("engines"),
        *key,
        1,
    )?;
    bridge_impl::configure(config).map_err(|_| "Конфигурация уже задана")?;
    let code = bridge::ffi::run_application(
        &data.to_str().ok_or("UTF-8")?.into(),
        true,
        &"".into(),
        &"".into(),
        &"".into(),
    );
    if code != 0 {
        return Err(format!("Qt: {code}").into());
    }
    Ok(())
}
