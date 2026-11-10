//! Startup fence проверяет отсутствие процессов предыдущего запуска в cgroup.
//!
//! KillMode сам по себе не доказывает завершение задачи, застрявшей в ядре.
//! Проверка не выполняет сетевых эффектов и не принимает пути от клиента.

use std::{fs::File, io::Read};

use rustix::fs::{Dir, FileType, Mode, OFlags, fstatfs, open, openat};

const MAX_PROC_BYTES: u64 = 16 * 1024;
const MAX_PATH_COMPONENTS: usize = 64;
const MAX_DEPTH: usize = 32;
const MAX_DIRECTORIES: usize = 128;
const MAX_ENTRIES: usize = 4096;
// CGROUP2_SUPER_MAGIC из Linux uapi/linux/magic.h.
const CGROUP2_MAGIC: u64 = 0x6367_7270;
const FILE_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);
const DIR_FLAGS: OFlags = FILE_FLAGS.union(OFlags::DIRECTORY);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceError {
    Unavailable,
    InvalidCgroup,
    Capacity,
    PreviousWorker,
}

impl std::fmt::Display for FenceError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(match self {
            Self::Unavailable => "Проверка cgroup helper недоступна",
            Self::InvalidCgroup => "Некорректная cgroup helper",
            Self::Capacity => "Превышен предел проверки cgroup helper",
            Self::PreviousWorker => "Предыдущий процесс helper ещё не завершён",
        })
    }
}

impl std::error::Error for FenceError {}

/// Вызывать из фиксированного ExecStartPre до запуска нового владельца D-Bus.
/// Гарантия ограничена процессами данной cgroup и её потомков в момент проверки.
pub fn verify_worker_fence() -> Result<(), FenceError> {
    let membership = bounded_read(File::open("/proc/self/cgroup").map_err(unavailable)?)?;
    let components = cgroup_components(&membership)?;
    let mut directory = File::from(open("/", DIR_FLAGS, Mode::empty()).map_err(unavailable)?);
    for component in ["sys", "fs", "cgroup"] {
        directory = child_directory(&directory, component)?;
    }
    let filesystem = fstatfs(&directory).map_err(unavailable)?;
    if filesystem.f_type as u64 != CGROUP2_MAGIC {
        return Err(FenceError::InvalidCgroup);
    }
    for component in components {
        directory = child_directory(&directory, component)?;
    }
    let mut budget = WalkBudget::default();
    verify_tree(&directory, std::process::id(), 0, &mut budget)?;
    if !budget.seen_self {
        return Err(FenceError::InvalidCgroup);
    }
    Ok(())
}

fn unavailable<T>(_: T) -> FenceError {
    FenceError::Unavailable
}

fn bounded_read(file: File) -> Result<String, FenceError> {
    let mut bytes = Vec::new();
    file.take(MAX_PROC_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(unavailable)?;
    if bytes.len() as u64 > MAX_PROC_BYTES {
        return Err(FenceError::Capacity);
    }
    String::from_utf8(bytes).map_err(|_| FenceError::InvalidCgroup)
}

fn cgroup_components(membership: &str) -> Result<Vec<&str>, FenceError> {
    let line = membership
        .strip_suffix('\n')
        .ok_or(FenceError::InvalidCgroup)?;
    let path = line.strip_prefix("0::/").ok_or(FenceError::InvalidCgroup)?;
    let components: Vec<_> = path.split('/').collect();
    if components.len() > MAX_PATH_COMPONENTS {
        return Err(FenceError::Capacity);
    }
    if components.iter().any(|component| {
        component.is_empty()
            || matches!(*component, "." | "..")
            || component.len() > 255
            || component.chars().any(char::is_control)
    }) {
        return Err(FenceError::InvalidCgroup);
    }
    Ok(components)
}

fn child_directory(parent: &File, name: &str) -> Result<File, FenceError> {
    openat(parent, name, DIR_FLAGS, Mode::empty())
        .map(File::from)
        .map_err(unavailable)
}

#[derive(Default)]
struct WalkBudget {
    directories: usize,
    entries: usize,
    seen_self: bool,
}

fn check_pids(contents: &str, own_pid: u32) -> Result<bool, FenceError> {
    let mut seen_self = false;
    for line in contents.split_terminator('\n') {
        if line.is_empty() || !line.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(FenceError::InvalidCgroup);
        }
        let pid: u32 = line.parse().map_err(|_| FenceError::InvalidCgroup)?;
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(FenceError::InvalidCgroup);
        }
        if pid != own_pid {
            return Err(FenceError::PreviousWorker);
        }
        seen_self = true;
    }
    if !contents.is_empty() && !contents.ends_with('\n') {
        return Err(FenceError::InvalidCgroup);
    }
    Ok(seen_self)
}

fn verify_tree(
    directory: &File,
    own_pid: u32,
    depth: usize,
    budget: &mut WalkBudget,
) -> Result<(), FenceError> {
    budget.directories += 1;
    if depth > MAX_DEPTH || budget.directories > MAX_DIRECTORIES {
        return Err(FenceError::Capacity);
    }
    let procs = File::from(
        openat(directory, "cgroup.procs", FILE_FLAGS, Mode::empty()).map_err(unavailable)?,
    );
    budget.seen_self |= check_pids(&bounded_read(procs)?, own_pid)?;
    let entries = Dir::read_from(directory).map_err(unavailable)?;
    for entry in entries {
        let entry = entry.map_err(unavailable)?;
        budget.entries += 1;
        if budget.entries > MAX_ENTRIES {
            return Err(FenceError::Capacity);
        }
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        match entry.file_type() {
            FileType::Directory => {
                let child = File::from(
                    openat(directory, name, DIR_FLAGS, Mode::empty()).map_err(unavailable)?,
                );
                verify_tree(&child, own_pid, depth + 1, budget)?;
            }
            FileType::RegularFile => {}
            _ => return Err(FenceError::InvalidCgroup),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_membership_rejects_legacy_and_traversal() {
        assert_eq!(
            cgroup_components("0::/system.slice/yonik-helper.service\n"),
            Ok(vec!["system.slice", "yonik-helper.service"])
        );
        for malformed in [
            "0::/\n",
            "0::/../other\n",
            "0::/a//b\n",
            "0::/a",
            "0::/a\n1:x:/b\n",
            "1:cpu:/a\n",
            "0::/a\0b\n",
            "0::/a/./b\n",
        ] {
            assert!(cgroup_components(malformed).is_err(), "{malformed:?}");
        }
        let deep = format!("0::/{}\n", vec!["a"; MAX_PATH_COMPONENTS + 1].join("/"));
        assert_eq!(cgroup_components(&deep), Err(FenceError::Capacity));
    }

    #[test]
    fn process_parser_fails_closed_and_detects_old_worker() {
        assert_eq!(check_pids("42\n", 42), Ok(true));
        assert_eq!(check_pids("", 42), Ok(false));
        assert_eq!(check_pids("42\n43\n", 42), Err(FenceError::PreviousWorker));
        for malformed in [
            "0\n",
            "4294967296\n",
            "2147483648\n",
            "42",
            " 42\n",
            "\n",
            "a\n",
        ] {
            assert_eq!(check_pids(malformed, 42), Err(FenceError::InvalidCgroup));
        }
    }

    #[test]
    fn bounded_reader_rejects_oversized_input() {
        let directory = Fixture::new();
        let path = directory.0.join("large");
        std::fs::write(&path, vec![b'0'; MAX_PROC_BYTES as usize + 1]).unwrap();
        assert_eq!(
            bounded_read(File::open(path).unwrap()),
            Err(FenceError::Capacity)
        );
    }

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("yonik-fence-{}-{nonce}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("cgroup.procs"), b"42\n").unwrap();
            Self(path)
        }

        fn verify(&self) -> Result<(), FenceError> {
            verify_tree(
                &File::open(&self.0).unwrap(),
                42,
                0,
                &mut WalkBudget::default(),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn tree_checks_descendants_and_rejects_symlinks() {
        let fixture = Fixture::new();
        assert_eq!(fixture.verify(), Ok(()));
        let child = fixture.0.join("descendant");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("cgroup.procs"), b"43\n").unwrap();
        assert_eq!(fixture.verify(), Err(FenceError::PreviousWorker));
        std::fs::write(child.join("cgroup.procs"), b"").unwrap();
        assert_eq!(fixture.verify(), Ok(()));
        std::os::unix::fs::symlink(&child, fixture.0.join("alias")).unwrap();
        assert_eq!(fixture.verify(), Err(FenceError::InvalidCgroup));
    }

    #[test]
    fn tree_budget_is_finite() {
        let fixture = Fixture::new();
        let mut budget = WalkBudget {
            directories: MAX_DIRECTORIES,
            ..WalkBudget::default()
        };
        assert_eq!(
            verify_tree(&File::open(&fixture.0).unwrap(), 42, 0, &mut budget),
            Err(FenceError::Capacity)
        );
    }
}
