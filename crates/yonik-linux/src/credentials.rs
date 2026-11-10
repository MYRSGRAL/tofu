//! Ключи snapshot остаются в Secret Service; plaintext fallback отсутствует.

use futures_util::FutureExt;
use secret_service::{EncryptionType, SecretService};
use std::{
    collections::HashMap,
    fmt,
    fs::{self, DirBuilder, File, OpenOptions},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::PathBuf,
    time::Duration,
};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    Unavailable,
    Locked,
    Missing,
    Ambiguous,
    InvalidKey,
    Random,
    Timeout,
    Busy,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Secret Service недоступен",
            Self::Locked => "Разблокируйте хранилище секретов в KDE",
            Self::Missing => "Ключ существующей базы отсутствует в Secret Service",
            Self::Ambiguous => "Найдено несколько ключей yonik; требуется восстановление",
            Self::InvalidKey => "Неверный формат ключа yonik",
            Self::Random => "Не удалось получить случайный ключ",
            Self::Timeout => "Secret Service не ответил вовремя",
            Self::Busy => "Создание ключа yonik уже выполняется",
        })
    }
}
impl std::error::Error for CredentialError {}

/// Для существующей БД `allow_create` всегда false: потеря ключа не создаёт
/// новую криптографическую идентичность поверх старых данных.
pub async fn subscription_master_key(
    allow_create: bool,
) -> Result<Zeroizing<[u8; 32]>, CredentialError> {
    let _creation_lock = if allow_create {
        Some(
            tokio::task::spawn_blocking(creation_lock)
                .await
                .map_err(|_| CredentialError::Unavailable)??,
        )
    } else {
        None
    };
    tokio::time::timeout(Duration::from_secs(15), load_key(allow_create))
        .await
        .map_err(|_| CredentialError::Timeout)?
}

fn creation_lock() -> Result<File, CredentialError> {
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or(CredentialError::Unavailable)?);
    if !runtime.is_absolute() {
        return Err(CredentialError::Unavailable);
    }
    let uid = rustix::process::getuid().as_raw();
    let check = |path: &std::path::Path| -> Result<(), CredentialError> {
        let metadata = fs::symlink_metadata(path).map_err(|_| CredentialError::Unavailable)?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700 {
            return Err(CredentialError::Unavailable);
        }
        Ok(())
    };
    check(&runtime)?;
    // Старый lock общий с прежними версиями: нельзя создавать два ключа параллельно.
    let path = runtime.join("tofu-key-creation");
    match DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(CredentialError::Unavailable),
    }
    check(&path)?;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::DIRECTORY).bits() as i32)
        .open(path)
        .map_err(|_| CredentialError::Unavailable)?;
    rustix::fs::flock(
        &directory,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .map_err(|_| CredentialError::Busy)?;
    Ok(directory)
}

fn secret_error(error: secret_service::Error) -> CredentialError {
    match error {
        secret_service::Error::Locked => CredentialError::Locked,
        secret_service::Error::Zbus(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.Secret.Error.IsLocked" =>
        {
            CredentialError::Locked
        }
        _ => CredentialError::Unavailable,
    }
}

async fn load_key(allow_create: bool) -> Result<Zeroizing<[u8; 32]>, CredentialError> {
    let service = SecretService::connect(EncryptionType::Dh)
        .await
        .map_err(|_| CredentialError::Unavailable)?;
    // Атрибуты поиска сохраняются для доступа к существующему ключу в Secret Service.
    let attributes = HashMap::from([
        ("application", "cloud.highpriority.tofu"),
        ("purpose", "subscription-master-key"),
        ("schema", "1"),
    ]);
    let found = service
        .search_items(attributes.clone())
        .await
        .map_err(|_| CredentialError::Unavailable)?;
    let count = found.unlocked.len() + found.locked.len();
    if count > 1 {
        return Err(CredentialError::Ambiguous);
    }
    if !found.locked.is_empty() {
        return Err(CredentialError::Locked);
    }
    if let Some(item) = found.unlocked.first() {
        let secret = Zeroizing::new(
            // secret-service 4.0.0 паникует при неверной длине IV от сервиса.
            // Ограничиваем перехват декодированием ответа, не меняя panic hook.
            std::panic::AssertUnwindSafe(item.get_secret())
                .catch_unwind()
                .await
                .map_err(|_| CredentialError::Unavailable)?
                .map_err(secret_error)?,
        );
        let bytes: [u8; 32] = secret
            .as_slice()
            .try_into()
            .map_err(|_| CredentialError::InvalidKey)?;
        return Ok(Zeroizing::new(bytes));
    }
    if !allow_create {
        return Err(CredentialError::Missing);
    }
    let collection = service
        .get_default_collection()
        .await
        .map_err(|_| CredentialError::Unavailable)?;
    if collection
        .is_locked()
        .await
        .map_err(|_| CredentialError::Unavailable)?
    {
        return Err(CredentialError::Locked);
    }
    let mut key = Zeroizing::new([0; 32]);
    getrandom::getrandom(key.as_mut()).map_err(|_| CredentialError::Random)?;
    // Никогда не заменяем найденный ключ и не создаём/разблокируем коллекции.
    collection
        .create_item(
            "yonik — ключ зашифрованных подписок",
            attributes,
            key.as_ref(),
            false,
            "application/octet-stream",
        )
        .await
        .map_err(|_| CredentialError::Unavailable)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Требует доступного Secret Service; только чтение, ключи не создаются"]
    async fn secret_service_readonly() {
        match subscription_master_key(false).await {
            Ok(key) => assert_eq!(key.len(), 32),
            Err(CredentialError::Missing) => {}
            Err(error) => panic!("Secret Service: {error}"),
        }
    }
}
