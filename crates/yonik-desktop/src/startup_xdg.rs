//! Единственная собственная XDG-запись. Никаких shell, PATH и установки служб.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
const NAME: &str = "yonik.desktop";
const ENTRY: &[u8] = b"[Desktop Entry]\nType=Application\nName=yonik\nExec=/usr/bin/yonik-desktop --start-hidden\nIcon=cloud.highpriority.yonik\nTerminal=false\nX-Yonik-Autostart-Version=1\n";

pub(crate) fn config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .filter(|path| path.is_absolute())
}
fn directory(path: &Path) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::NOFOLLOW).bits() as i32)
        .open(path)?;
    let meta = file.metadata()?;
    if meta.uid() != rustix::process::getuid().as_raw() || meta.mode() & 0o022 != 0 {
        return Err(std::io::Error::other("небезопасный каталог XDG"));
    }
    Ok(file)
}
fn existing(dir: &File) -> std::io::Result<bool> {
    use rustix::fs::{Mode, OFlags, openat};
    let fd = match openat(
        dir,
        NAME,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let file = File::from(fd);
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != rustix::process::getuid().as_raw()
        || meta.nlink() != 1
        || meta.mode() & 0o7777 != 0o600
        || meta.len() != ENTRY.len() as u64
    {
        return Err(std::io::Error::other("чужая запись XDG сохранена"));
    }
    let mut bytes = Vec::new();
    file.take(ENTRY.len() as u64 + 1).read_to_end(&mut bytes)?;
    if bytes != ENTRY {
        return Err(std::io::Error::other("чужая запись XDG сохранена"));
    }
    Ok(true)
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Durable,
    PublishedDurabilityUnknown,
}
/// Вызывается только после durable preferences. Неизвестный fsync не откатывается.
pub(crate) fn apply(root: &Path, enabled: bool) -> std::io::Result<Outcome> {
    apply_with_sync(root, enabled, |directory| directory.sync_all())
}
fn apply_with_sync(
    root: &Path,
    enabled: bool,
    sync: fn(&File) -> std::io::Result<()>,
) -> std::io::Result<Outcome> {
    use rustix::fs::{self as sys, AtFlags, Mode, OFlags};
    if !root.is_absolute() {
        return Err(std::io::Error::other("относительный XDG"));
    }
    // Родитель config должен существовать: не создаём произвольные деревья из env.
    let config = directory(root)?;
    if enabled {
        match sys::mkdirat(&config, "autostart", Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => (),
            Err(error) => return Err(error.into()),
        }
    }
    let fd = match sys::openat(
        &config,
        "autostart",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) if !enabled => return Ok(Outcome::Durable),
        Err(error) => return Err(error.into()),
    };
    let dir = File::from(fd);
    let meta = dir.metadata()?;
    if meta.uid() != rustix::process::getuid().as_raw() || meta.mode() & 0o022 != 0 {
        return Err(std::io::Error::other("небезопасный каталог autostart"));
    }
    sys::flock(&dir, sys::FlockOperation::NonBlockingLockExclusive)?;
    if existing(&dir)? {
        if !enabled {
            sys::unlinkat(&dir, NAME, AtFlags::empty())?;
            if sync(&dir).is_err() {
                return Ok(Outcome::PublishedDurabilityUnknown);
            }
        }
        return Ok(Outcome::Durable);
    }
    if !enabled {
        return Ok(Outcome::Durable);
    }
    let temp = format!(".yonik-{}.tmp", std::process::id());
    let fd = sys::openat(
        &dir,
        temp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?;
    let publication = (|| {
        let mut file = File::from(fd);
        file.write_all(ENTRY)?;
        file.sync_all()?;
        sys::linkat(&dir, temp.as_str(), &dir, NAME, AtFlags::empty())?;
        Ok::<_, std::io::Error>(())
    })();
    let cleanup = sys::unlinkat(&dir, temp.as_str(), AtFlags::empty());
    publication?;
    // После link/unlink ошибка fsync или cleanup не доказывает откат публикации.
    if cleanup.is_err() || sync(&dir).is_err() || sync(&config).is_err() {
        return Ok(Outcome::PublishedDurabilityUnknown);
    }
    Ok(Outcome::Durable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::DirBuilderExt};
    #[test]
    fn published_unknown_is_not_rollback() {
        let root = std::env::temp_dir().join(format!("yonik-xdg-sync-{}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .expect("fixture");
        assert_eq!(
            apply_with_sync(&root, true, |_| Err(std::io::Error::other("fixture")))
                .expect("published"),
            Outcome::PublishedDurabilityUnknown
        );
        assert_eq!(
            fs::read(root.join("autostart").join(NAME)).expect("published"),
            ENTRY
        );
        assert_eq!(
            apply_with_sync(&root, false, |_| Err(std::io::Error::other("fixture")))
                .expect("removed"),
            Outcome::PublishedDurabilityUnknown
        );
        assert!(!root.join("autostart").join(NAME).exists());
        fs::remove_dir_all(root).expect("fixture cleanup");
    }
    #[test]
    fn own_fixture_only_foreign_and_symlink_preserved() {
        let root = std::env::temp_dir().join(format!("yonik-xdg-{}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .expect("fixture");
        apply(&root, true).expect("enable");
        let path = root.join("autostart").join(NAME);
        assert_eq!(fs::read(&path).expect("read"), ENTRY);
        apply(&root, true).expect("idempotent");
        apply(&root, false).expect("disable");
        fs::write(&path, b"foreign").expect("fixture");
        assert!(apply(&root, true).is_err());
        assert!(apply(&root, false).is_err());
        assert_eq!(fs::read(&path).expect("read"), b"foreign");
        fs::remove_file(&path).expect("fixture");
        std::os::unix::fs::symlink(root.join("victim"), &path).expect("fixture");
        assert!(apply(&root, true).is_err());
        assert!(apply(&root, false).is_err());
        assert!(
            fs::symlink_metadata(&path)
                .expect("link")
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(root).expect("fixture cleanup");
    }
}
