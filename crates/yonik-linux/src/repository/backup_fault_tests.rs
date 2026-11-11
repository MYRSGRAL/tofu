//! Изолированные fixtures; hooks не входят в production-сборку.
use super::{
    Connection, EncryptedSubscriptionRepository, OpenFlags, PathBuf, RepositoryError, Result,
    Zeroizing, copy_closed, fs, validate,
};
use std::os::unix::fs::MetadataExt;
use std::{
    cell::RefCell,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
thread_local! { static ACTION: RefCell<String> = const { RefCell::new(String::new()) }; }
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn checkpoint(name: &str) -> Result<()> {
    let action = ACTION.with(|a| a.borrow().clone());
    if action == format!("crash:{name}") {
        let root = PathBuf::from(std::env::var_os("YONIK_FAULT_ROOT").unwrap());
        fs::write(root.join("checkpoint"), name).unwrap();
        // Родитель держит Child без reap до SIGKILL собственной process group.
        std::thread::sleep(std::time::Duration::from_secs(15));
        std::process::exit(87);
    }
    if action.starts_with(&format!("fail:{name}:")) {
        let errno = action.rsplit(':').next().unwrap().parse::<i32>().unwrap();
        return Err(std::io::Error::from_raw_os_error(errno)).map_err(|_| RepositoryError::Storage);
    }
    if action.starts_with("race:") && name == "link" {
        let root = PathBuf::from(std::env::var_os("YONIK_FAULT_ROOT").unwrap());
        if action == "race:file" {
            fs::copy(root.join("source"), root.join("target")).unwrap();
        } else {
            std::os::unix::fs::symlink(root.join("source"), root.join("target")).unwrap();
        }
    }
    if action == "corrupt" && name == "after-vacuum" {
        let root = std::env::var_os("YONIK_FAULT_ROOT").unwrap();
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".yonik-backup-")
            {
                let db = Connection::open(path).unwrap();
                db.execute("UPDATE sources SET payload=zeroblob(length(payload))", [])
                    .unwrap();
            }
        }
    }
    Ok(())
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yonik-fault-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let repo =
                EncryptedSubscriptionRepository::open(path.join("source"), Zeroizing::new([7; 32]))
                    .await
                    .unwrap();
            repo.create_source(b"private-fault-marker".to_vec())
                .await
                .unwrap();
            repo.close().await.unwrap();
        });
        Self(path)
    }
    fn verify(&self, before: &[u8]) {
        assert_eq!(fs::read(self.0.join("source")).unwrap(), before);
        let target = self.0.join("target");
        if target.exists() {
            let db =
                Connection::open_with_flags(&target, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            validate(&db, &[7; 32]).unwrap();
            assert_eq!(fs::metadata(&target).unwrap().nlink(), 1);
            drop(db);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let repo = EncryptedSubscriptionRepository::open(target, Zeroizing::new([7; 32]))
                    .await
                    .unwrap();
                assert_eq!(
                    repo.load(1)
                        .await
                        .unwrap()
                        .unwrap()
                        .source
                        .payload
                        .as_slice(),
                    b"private-fault-marker"
                );
                repo.close().await.unwrap();
            });
        }
        for entry in fs::read_dir(&self.0).unwrap() {
            let entry = entry.unwrap();
            assert!(
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".yonik-backup-")
            );
            if entry.file_type().unwrap().is_file() {
                assert!(
                    !fs::read(entry.path())
                        .unwrap()
                        .windows(20)
                        .any(|b| b == b"private-fault-marker")
                );
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn isolated_child() {
    let Some(root) = std::env::var_os("YONIK_FAULT_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    ACTION.with(|a| *a.borrow_mut() = std::env::var("YONIK_FAULT_ACTION").unwrap());
    let result = copy_closed(&root.join("source"), &root.join("target"), &[7; 32]);
    assert!(result.is_err(), "повреждённый кандидат опубликован");
    if std::env::var("YONIK_FAULT_ACTION")
        .unwrap()
        .starts_with("race:")
    {
        assert_eq!(
            fs::read(root.join("target")).unwrap(),
            fs::read(root.join("source")).unwrap()
        );
    } else {
        assert!(!root.join("target").exists());
    }
}

#[test]
fn corrupt_vacuum_candidate_is_rejected() {
    let f = Fixture::new();
    let before = fs::read(f.0.join("source")).unwrap();
    let status = child(&f, "corrupt");
    assert!(status.success());
    f.verify(&before);
}
fn child(f: &Fixture, action: &str) -> std::process::ExitStatus {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "repository::backup::fault_tests::isolated_child",
            "--nocapture",
        ])
        .process_group(0)
        .env("YONIK_FAULT_ROOT", &f.0)
        .env("YONIK_FAULT_ACTION", action)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if (action.starts_with("crash:") && f.0.join("checkpoint").exists())
            || std::time::Instant::now() >= deadline
        {
            let pid = rustix::process::Pid::from_raw(child.id().try_into().unwrap()).unwrap();
            rustix::process::kill_process_group(pid, rustix::process::Signal::KILL).unwrap();
            return child.wait().unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
#[test]
fn crash_restart_matrix() {
    for point in [
        "before-vacuum",
        "after-vacuum",
        "after-validation",
        "after-publication",
        "directory-sync",
        "after-directory-sync",
    ] {
        let f = Fixture::new();
        let before = fs::read(f.0.join("source")).unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child(&f, &format!("crash:{point}")).signal(), Some(9));
        assert_eq!(fs::read_to_string(f.0.join("checkpoint")).unwrap(), point);
        // Повтор вызывает recovery даже при уже опубликованном назначении.
        let result = copy_closed(&f.0.join("source"), &f.0.join("target"), &[7; 32]);
        assert!(result.is_ok() || matches!(result, Err(RepositoryError::Conflict)));
        f.verify(&before);
    }
}
#[test]
fn deterministic_failure_matrix() {
    for point in ["before-vacuum", "file-sync", "link", "directory-sync"] {
        for errno in [5, 13, 28] {
            let f = Fixture::new();
            let before = fs::read(f.0.join("source")).unwrap();
            ACTION.with(|a| *a.borrow_mut() = format!("fail:{point}:{errno}"));
            assert!(copy_closed(&f.0.join("source"), &f.0.join("target"), &[7; 32]).is_err());
            ACTION.with(|a| a.borrow_mut().clear());
            f.verify(&before);
        }
    }
}

#[test]
fn path_boundaries_and_foreign_staging() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let source = f.0.join("source");
    let before = fs::read(&source).unwrap();
    let target = f.0.join("target");
    symlink(f.0.join("missing"), &target).unwrap();
    assert!(copy_closed(&source, &target, &[7; 32]).is_err());
    assert!(fs::symlink_metadata(&target).unwrap().is_symlink());
    fs::remove_file(&target).unwrap();
    let alias = f.0.join("alias");
    fs::hard_link(&source, &alias).unwrap();
    assert!(copy_closed(&source, &target, &[7; 32]).is_err());
    fs::remove_file(&alias).unwrap();
    symlink(&source, &alias).unwrap();
    assert!(copy_closed(&alias, &target, &[7; 32]).is_err());
    fs::remove_file(&alias).unwrap();
    let foreign = f.0.join(".yonik-backup-foreign");
    fs::write(&foreign, b"foreign").unwrap();
    copy_closed(&source, &target, &[7; 32]).unwrap();
    assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
    let published = fs::read(&target).unwrap();
    assert!(matches!(
        copy_closed(&source, &target, &[7; 32]),
        Err(RepositoryError::Conflict)
    ));
    assert_eq!(fs::read(&target).unwrap(), published);
    assert_eq!(fs::read(&source).unwrap(), before);
}

#[test]
fn publication_race_never_overwrites() {
    for action in ["race:file", "race:symlink"] {
        let f = Fixture::new();
        let before = fs::read(f.0.join("source")).unwrap();
        assert!(child(&f, action).success());
        assert_eq!(fs::read(f.0.join("source")).unwrap(), before);
        assert_eq!(fs::read(f.0.join("target")).unwrap(), before);
        if action == "race:symlink" {
            assert!(
                fs::symlink_metadata(f.0.join("target"))
                    .unwrap()
                    .is_symlink()
            );
        }
        assert!(!fs::read_dir(&f.0).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".yonik-backup-")
        }));
    }
}
#[test]
fn directory_lock_refuses_parallel_recovery() {
    let f = Fixture::new();
    let directory = fs::File::open(&f.0).unwrap();
    rustix::fs::flock(
        &directory,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .unwrap();
    assert!(matches!(
        copy_closed(&f.0.join("source"), &f.0.join("target"), &[7; 32]),
        Err(RepositoryError::Conflict)
    ));
    assert!(!f.0.join("target").exists());
}
