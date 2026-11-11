//! Диагностика настоящего VM Secret Service; значения секретов не выводятся.
use secret_service::{EncryptionType, SecretService};
use std::collections::HashMap;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("YONIK_CREDENTIAL_VM").as_deref() != Ok("disposable-v1")
        || std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")?.trim() != "QEMU"
    {
        return Err("Требуется VM".into());
    }
    let mode = std::env::args().nth(1).unwrap_or_default();
    if mode == "create" || mode == "read" {
        let result = yonik_linux::subscription_master_key(mode == "create").await;
        let status = match result {
            Ok(_) => "available".to_owned(),
            Err(error) => format!("{error:?}"),
        };
        println!(
            "{}",
            serde_json::json!({"status":status,"pid":std::process::id()})
        );
        return Ok(());
    }
    let service = SecretService::connect(EncryptionType::Dh).await?;
    let found = service
        .search_items(HashMap::from([
            ("application", "cloud.highpriority.tofu"),
            ("purpose", "subscription-master-key"),
            ("schema", "1"),
        ]))
        .await?;
    if mode == "stale" {
        use std::io::{Read, Write};
        println!(
            "{}",
            serde_json::json!({"state":"session_open","pid":std::process::id()})
        );
        std::io::stdout().flush()?;
        let mut acknowledgement = [0_u8; 1];
        std::io::stdin().read_exact(&mut acknowledgement)?;
    }
    let mut readable = 0;
    let mut refused = 0;
    for item in &found.unlocked {
        match item.get_secret().await {
            Ok(secret) => {
                let secret = zeroize::Zeroizing::new(secret);
                if secret.len() == 32 {
                    readable += 1;
                } else {
                    refused += 1;
                }
            }
            Err(_) => refused += 1,
        }
    }
    println!(
        "{}",
        serde_json::json!({"unlocked":found.unlocked.len(),"locked":found.locked.len(),"get_secret_readable":readable,"get_secret_refused":refused,"pid":std::process::id()})
    );
    Ok(())
}
