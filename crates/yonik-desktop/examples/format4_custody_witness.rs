//! Supplemental storage witness; запускать только внутри одноразовой VM.
use std::path::PathBuf;
use yonik_linux::{EncryptedSubscriptionRepository, SubscriptionService, subscription_master_key};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("YONIK_CREDENTIAL_VM").as_deref() != Ok("disposable-v1")
        || std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")?.trim() != "QEMU"
        || rustix::process::getuid().is_root()
    {
        return Err("Требуется отдельный пользователь одноразовой QEMU VM".into());
    }
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("Нет каталога fixture")?);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // Ошибки backend не выводятся: evidence содержит только итог gate.
    let passed = runtime.block_on(async {
        let key = subscription_master_key(false).await.map_err(|_| ())?;
        let repository =
            EncryptedSubscriptionRepository::open(root.join("subscriptions.sqlite"), key)
                .await
                .map_err(|_| ())?;
        let service = SubscriptionService::new(repository);
        let mut original_matches = true;
        let mut revision_matches = true;
        for (id, name) in [(1, "text.txt"), (2, "file.txt")] {
            let expected = std::fs::read(root.join(name)).map_err(|_| ())?;
            let snapshot = service.offline(id).await.map_err(|_| ())?;
            original_matches &= snapshot.report.original().expose().as_bytes() == expected;
            revision_matches &= snapshot.revision == 1;
        }
        service.close().await.map_err(|_| ())?;
        Ok::<_, ()>((original_matches, revision_matches))
    });
    match passed {
        Ok((original_matches, revision_matches)) => {
            println!(
                "{}",
                serde_json::json!({"original_matches": original_matches,
                "revision_matches": revision_matches, "count": 2})
            );
            if original_matches && revision_matches {
                Ok(())
            } else {
                Err("Witness gate failed".into())
            }
        }
        Err(()) => Err("Storage witness недоступен".into()),
    }
}
