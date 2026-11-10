use yonik_linux::applications::{Catalog, Limits};
#[test]
fn empty_roots_are_empty() {
    let catalog = Catalog::new(
        vec![],
        "/proc".into(),
        "ru_RU.UTF-8".into(),
        Limits::default(),
    )
    .unwrap();
    assert!(catalog.desktop_snapshot().unwrap().items.is_empty());
}
#[test]
fn relative_roots_are_rejected() {
    assert!(
        Catalog::new(
            vec!["relative".into()],
            "/proc".into(),
            "C".into(),
            Limits::default()
        )
        .is_err()
    );
}
use std::{fs, os::unix::fs::symlink, path::PathBuf};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "yonik-catalog-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(p.join("high")).unwrap();
        fs::create_dir_all(p.join("low")).unwrap();
        Self(p)
    }
    fn catalog(&self, limits: Limits) -> Catalog {
        Catalog::new(
            vec![self.0.join("high"), self.0.join("low")],
            self.0.clone(),
            "sr_RS.UTF-8@latin".into(),
            limits,
        )
        .unwrap()
    }
    fn put(&self, path: &str, body: &str) {
        fs::write(self.0.join(path), body).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn precedence_localization_and_hidden_masking() {
    let f = Fixture::new();
    f.put(
        "low/a.desktop",
        "[Desktop Entry]\nType=Application\nName=Low\n",
    );
    f.put("high/a.desktop", "[Desktop Entry]\nHidden=true\n");
    f.put("high/b.desktop","[Desktop Entry]\nType=Application\nName=Base\nName[sr]=Language\nName[sr_RS]=Country\nName[sr@latin]=Modifier\nNoDisplay=true\nExec=sh -c touch /tmp/never\n");
    let s = f.catalog(Limits::default()).desktop_snapshot().unwrap();
    assert_eq!(s.items.len(), 1);
    assert_eq!(s.items[0].name, "Country");
    assert!(s.items[0].no_display);
    assert!(s.items[0].executable.is_err());
}
#[test]
fn symlinks_and_limits_are_bounded() {
    let f = Fixture::new();
    symlink(&f.0, f.0.join("high/loop")).unwrap();
    f.put(
        "high/a.desktop",
        "[Desktop Entry]\nType=Application\nName=A\nExec=/bin/sh -c evil\n",
    );
    let s = f
        .catalog(Limits {
            max_bytes: 4,
            ..Limits::default()
        })
        .desktop_snapshot()
        .unwrap();
    assert!(s.items.is_empty());
    assert!(s.incomplete);
}
#[test]
fn process_exit_and_reuse_are_not_persistent_selectors() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("42")).unwrap();
    let stat = format!("42 (name with ) parens) S {} 99", vec!["0"; 18].join(" "));
    f.put("42/stat", &stat);
    symlink("/usr/bin/example", f.0.join("42/exe")).unwrap();
    let c = f.catalog(Limits::default());
    let s = c.process_snapshot().unwrap();
    assert_eq!(s.items.len(), 1);
    assert_eq!(s.items[0].start_time, 99);
    f.put("42/stat", &stat.replace("99", "100"));
    assert!(!c.process_is_current(&s.items[0]));
    fs::remove_file(f.0.join("42/stat")).unwrap();
    assert!(!c.process_is_current(&s.items[0]));
}
#[test]
fn fifo_device_symlink_and_wrong_type_are_not_applications() {
    let f = Fixture::new();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        f.0.join("high/fifo.desktop"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR,
        0,
    )
    .unwrap();
    symlink("/dev/zero", f.0.join("high/device.desktop")).unwrap();
    f.put(
        "high/link.desktop",
        "[Desktop Entry]\nType=Link\nName=Link\n",
    );
    let s = f.catalog(Limits::default()).desktop_snapshot().unwrap();
    assert!(s.items.is_empty());
    assert!(s.incomplete);
}
#[test]
fn nested_ids_and_entry_limit() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("high/vendor")).unwrap();
    f.put(
        "high/vendor/a.desktop",
        "[Desktop Entry]\nType=Application\nName=Nested\nExec=flatpak run org.example.App\n",
    );
    let s = f.catalog(Limits::default()).desktop_snapshot().unwrap();
    assert_eq!(s.items[0].id, "vendor-a.desktop");
    assert!(s.items[0].executable.is_err());
    f.put(
        "high/b.desktop",
        "[Desktop Entry]\nType=Application\nName=B\n",
    );
    let s = f
        .catalog(Limits {
            max_entries: 1,
            ..Limits::default()
        })
        .desktop_snapshot()
        .unwrap();
    assert_eq!(s.items.len(), 1);
    assert!(s.incomplete);
}
#[test]
fn exact_path_requires_native_regular_executable() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let executable = f.0.join("native");
    fs::write(&executable, b"\x7fELFfixture").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    f.put(
        "high/a.desktop",
        &format!(
            "[Desktop Entry]\nType=Application\nName=A\nExec={} %U\n",
            executable.display()
        ),
    );
    let c = f.catalog(Limits::default());
    assert_eq!(
        c.desktop_snapshot().unwrap().items[0]
            .executable
            .as_ref()
            .unwrap(),
        &executable
    );
    for exec in [
        "env /bin/program",
        "python3 script.py",
        "wine game.exe",
        "./portable.AppImage",
        "$(touch /tmp/yonik-should-never-exist)",
        "program %U",
        "/bin/sh -c bad",
    ] {
        f.put(
            "high/a.desktop",
            &format!("[Desktop Entry]\nType=Application\nName=A\nExec={exec}\n"),
        );
        assert!(
            c.desktop_snapshot().unwrap().items[0].executable.is_err(),
            "{exec}"
        );
    }
}

#[test]
fn inaccessible_nested_directory_marks_snapshot_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().as_raw() == 0 {
        use std::os::unix::process::CommandExt;
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "inaccessible_nested_directory_marks_snapshot_incomplete",
                "--nocapture",
            ])
            .uid(65534)
            .gid(65534)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "EACCES fixture в непривилегированном процессе"
        );
        return;
    }
    // Настоящий EACCES; дочерний процесс root создаёт только собственную fixture.
    let f = Fixture::new();
    let private = f.0.join("high/private");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o000)).unwrap();
    let error = fs::read_dir(&private).unwrap_err();
    let snapshot = f.catalog(Limits::default()).desktop_snapshot().unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(
        snapshot.incomplete,
        "Недоступный вложенный каталог потерян без диагностики"
    );
}
