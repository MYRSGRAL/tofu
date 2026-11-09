use super::{Document, JournalError, MAX_JOURNAL_BYTES};
use rustix::fs::{self, AtFlags, FileType, FlockOperation, Mode, OFlags};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path},
};

const JOURNAL: &str = "journal.json";
const PENDING: &str = "journal.next";
const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const FILE_FLAGS: OFlags = OFlags::NOFOLLOW
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

fn safe_file(file: &File) -> Result<(), JournalError> {
    let stat = fs::fstat(file).map_err(|_| JournalError::Io)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o600
        || stat.st_nlink != 1
    {
        return Err(JournalError::UnsafePath);
    }
    Ok(())
}

/// Каждый компонент открывается относительно закреплённого FD с NOFOLLOW.
/// Предки — root/euid-owned, без чужой записи (кроме root-owned sticky /tmp).
/// Только последний компонент разрешено создать; права существующих не чиним.
pub(super) fn directory(path: &Path) -> Result<File, JournalError> {
    directory_with_sync(path, &mut File::sync_all)
}

fn directory_with_sync(
    path: &Path,
    sync_parent: &mut impl FnMut(&File) -> std::io::Result<()>,
) -> Result<File, JournalError> {
    if !path.is_absolute() {
        return Err(JournalError::UnsafePath);
    }
    let components: Vec<_> = path.components().collect();
    if components.len() < 2
        || components
            .iter()
            .skip(1)
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(JournalError::UnsafePath);
    }
    let mut current =
        File::from(fs::open("/", DIR_FLAGS, Mode::empty()).map_err(|_| JournalError::UnsafePath)?);
    for (index, part) in components.iter().enumerate().skip(1) {
        let Component::Normal(name) = part else {
            return Err(JournalError::UnsafePath);
        };
        let last = index == components.len() - 1;
        let opened = match fs::openat(&current, *name, DIR_FLAGS, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) if last => {
                match fs::mkdirat(&current, *name, Mode::from_raw_mode(0o700)) {
                    Ok(()) => {}
                    Err(rustix::io::Errno::EXIST) => {}
                    Err(_) => return Err(JournalError::Io),
                }
                fs::openat(&current, *name, DIR_FLAGS, Mode::empty())
                    .map_err(|_| JournalError::UnsafePath)?
            }
            Err(_) => return Err(JournalError::UnsafePath),
        };
        let stat = fs::fstat(&opened).map_err(|_| JournalError::Io)?;
        let effective = rustix::process::geteuid().as_raw();
        let safe = if last {
            stat.st_uid == effective && stat.st_mode & 0o7777 == 0o700
        } else {
            (stat.st_uid == 0 || stat.st_uid == effective)
                && (stat.st_mode & 0o022 == 0 || stat.st_uid == 0 && stat.st_mode & 0o1000 != 0)
        };
        if !safe {
            return Err(JournalError::UnsafePath);
        }
        // Предыдущий процесс мог создать каталог и умереть до parent fsync.
        // Поэтому подтверждаем его имя и при повторном открытии.
        if last {
            sync_parent(&current).map_err(|_| JournalError::Io)?;
        }
        current = File::from(opened);
    }
    fs::flock(&current, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        if error == rustix::io::Errno::WOULDBLOCK {
            JournalError::Busy
        } else {
            JournalError::Io
        }
    })?;
    Ok(current)
}

fn existing(directory: &File, name: &str, flags: OFlags) -> Result<Option<File>, JournalError> {
    match fs::openat(directory, name, FILE_FLAGS | flags, Mode::empty()) {
        Ok(fd) => {
            let file = File::from(fd);
            safe_file(&file)?;
            Ok(Some(file))
        }
        Err(rustix::io::Errno::NOENT) => {
            // Dangling symlink даёт ELOOP с NOFOLLOW, но сохраняем проверку
            // имени явно: отсутствующим считается только отсутствие inode.
            match fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
                Err(rustix::io::Errno::NOENT) => Ok(None),
                _ => Err(JournalError::UnsafePath),
            }
        }
        Err(_) => Err(JournalError::UnsafePath),
    }
}

pub(super) fn read(directory: &File) -> Result<Option<Document>, JournalError> {
    let Some(file) = existing(directory, JOURNAL, OFlags::RDONLY)? else {
        return Ok(None);
    };
    if file.metadata().map_err(|_| JournalError::Io)?.len() > MAX_JOURNAL_BYTES as u64 {
        return Err(JournalError::Limit);
    }
    let mut bytes = Vec::new();
    file.take(MAX_JOURNAL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| JournalError::Io)?;
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(JournalError::Limit);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| JournalError::Corrupt)
}

pub(super) fn write(directory: &File, document: &Document) -> Result<(), JournalError> {
    let bytes = serde_json::to_vec(document).map_err(|_| JournalError::Invalid)?;
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(JournalError::Limit);
    }
    // Нельзя заменить небезопасный существующий journal, даже атомарным rename.
    existing(directory, JOURNAL, OFlags::RDONLY)?;
    let mut pending = match existing(directory, PENDING, OFlags::WRONLY)? {
        Some(file) => file,
        None => File::from(
            fs::openat(
                directory,
                PENDING,
                FILE_FLAGS | OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| JournalError::Io)?,
        ),
    };
    safe_file(&pending)?;
    pending.set_len(0).map_err(|_| JournalError::Io)?;
    pending.write_all(&bytes).map_err(|_| JournalError::Io)?;
    pending.sync_all().map_err(|_| JournalError::Io)?;
    fs::renameat(directory, PENDING, directory, JOURNAL).map_err(|_| JournalError::Io)?;
    directory
        .sync_all()
        .map_err(|_| JournalError::UncertainWrite)
}

#[cfg(test)]
mod tests {
    use super::{JournalError, directory_with_sync};
    use std::{fs, os::unix::fs::DirBuilderExt, path::PathBuf};

    struct Area(PathBuf);
    impl Drop for Area {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("Очистка собственного тестового каталога");
        }
    }

    #[test]
    fn reopen_retries_parent_sync_after_interrupted_directory_creation() {
        let mut nonce = [0; 8];
        getrandom::getrandom(&mut nonce).expect("Тестовая случайность");
        let area_path = std::env::temp_dir().join(format!(
            "yonik-parent-sync-{:016x}",
            u64::from_le_bytes(nonce)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&area_path)
            .expect("Тестовый каталог");
        let area = Area(area_path);
        let path = area.0.join("state");
        let mut first_syncs = 0;
        let failed = directory_with_sync(&path, &mut |_| {
            first_syncs += 1;
            Err(std::io::Error::other("Инъекция ошибки parent fsync"))
        });
        assert!(matches!(failed, Err(JournalError::Io)));
        assert_eq!(first_syncs, 1);
        assert!(path.is_dir());

        let mut retry_syncs = 0;
        let reopened = directory_with_sync(&path, &mut |parent| {
            retry_syncs += 1;
            parent.sync_all()
        })
        .expect("Повторное открытие");
        drop(reopened);
        assert_eq!(
            retry_syncs, 1,
            "Существующее имя каталога тоже требует parent fsync"
        );
    }
}
