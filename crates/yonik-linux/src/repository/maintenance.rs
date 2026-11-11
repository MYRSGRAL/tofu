//! Непрозрачный доступ к maintenance без передачи ключа desktop-слою.
use super::*;

/// Не содержит SQLite handle и намеренно не реализует Debug/Serialize/Clone.
/// Владелец обязан закрыть все repository handles до запуска копирования.
pub struct MaintenanceAccess {
    source: PathBuf,
    key: Zeroizing<[u8; 32]>,
}

impl EncryptedSubscriptionRepository {
    pub async fn maintenance_access(&self) -> Result<MaintenanceAccess> {
        self.run(|database| {
            let source = database
                .connection
                .path()
                .ok_or(RepositoryError::UnsafePath)?;
            let source = PathBuf::from(source);
            secure_path(&source)?;
            if !source.is_absolute() {
                return Err(RepositoryError::UnsafePath);
            }
            Ok(MaintenanceAccess {
                source,
                key: database.key.clone(),
            })
        })
        .await
    }
}

impl MaintenanceAccess {
    /// Путь получен из открытого соединения, а не из GUI-запроса.
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// Reopen не получает instance lock повторно: его удерживает supervisor.
    pub async fn reopen(&self) -> Result<EncryptedSubscriptionRepository> {
        EncryptedSubscriptionRepository::open(self.source.clone(), self.key.clone()).await
    }
}
