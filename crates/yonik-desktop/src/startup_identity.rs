//! Привязка к экземпляру БД и размещению, без чтения секретов.
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    path: String,
    device: u64,
    inode: u64,
    directory_device: u64,
    directory_inode: u64,
    database: [u8; 32],
}
impl Identity {
    pub(crate) fn matches(&self, directory: &Path) -> bool {
        Self::capture(directory, self.database).is_ok_and(|current| current == *self)
    }
    pub(crate) fn capture(directory: &Path, database: [u8; 32]) -> std::io::Result<Self> {
        let canonical = directory.canonicalize()?;
        let parent = std::fs::symlink_metadata(directory)?;
        let path = canonical.join("subscriptions.sqlite");
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(&path)?;
        let meta = file.metadata()?;
        let current = std::fs::symlink_metadata(&path)?;
        let uid = rustix::process::getuid().as_raw();
        if !parent.is_dir()
            || parent.file_type().is_symlink()
            || parent.uid() != uid
            || parent.mode() & 0o7777 != 0o700
            || !meta.is_file()
            || meta.uid() != uid
            || meta.mode() & 0o077 != 0
            || meta.nlink() != 1
            || current.file_type().is_symlink()
            || current.dev() != meta.dev()
            || current.ino() != meta.ino()
        {
            return Err(std::io::Error::other("identity repository недоступна"));
        }
        Ok(Self {
            path: path
                .to_str()
                .ok_or_else(|| std::io::Error::other("путь repository не UTF-8"))?
                .into(),
            device: meta.dev(),
            inode: meta.ino(),
            directory_device: parent.dev(),
            directory_inode: parent.ino(),
            database,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    #[test]
    fn replacement_database_and_directory_require_confirmation() {
        let root =
            std::env::temp_dir().join(format!("yonik-startup-identity-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .expect("fixture");
        let path = root.join("subscriptions.sqlite");
        std::fs::write(&path, b"own fixture, not database").expect("fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("fixture");
        let identity = Identity::capture(&root, [41; 32]).expect("identity");
        assert!(identity.matches(&root));
        assert_ne!(
            identity,
            Identity::capture(&root, [42; 32]).expect("different database")
        );
        std::fs::rename(&path, root.join("old")).expect("fixture");
        std::fs::write(&path, b"replacement").expect("fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("fixture");
        assert!(!identity.matches(&root));
        std::fs::remove_file(&path).expect("fixture");
        std::os::unix::fs::symlink(root.join("old"), &path).expect("fixture");
        assert!(!identity.matches(&root));
        std::fs::remove_dir_all(root).expect("fixture");
    }
}
