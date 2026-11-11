//! Внутренняя диагностика тестовых сборок; содержимое БД и ключ не читаются.
use super::*;
use serde_json::{Value, json};

fn attributes(meta: &fs::Metadata) -> Value {
    json!({"mode": format!("{:06o}", meta.mode()), "uid": meta.uid(), "gid": meta.gid(),
        "nlink": meta.nlink(), "dev": meta.dev(), "ino": meta.ino(), "len": meta.len(),
        "ctime": meta.ctime(), "ctime_nsec": meta.ctime_nsec(),
        "is_file": meta.is_file(), "is_dir": meta.is_dir(), "is_symlink": meta.file_type().is_symlink()})
}
fn stat(path: &Path) -> Value {
    match fs::symlink_metadata(path) {
        Ok(meta) => attributes(&meta),
        Err(error) => json!({"errno": error.raw_os_error(), "kind": format!("{:?}", error.kind())}),
    }
}

pub(super) fn record(
    path: &Path,
    branch: &str,
    node: &Path,
    observed: Option<&fs::Metadata>,
    error: Option<&std::io::Error>,
) {
    let root = path.parent();
    // Полные имена разрешены только для синтетического test root непосредственно
    // в temp_dir. Произвольный пользовательский путь в output не попадает.
    let synthetic = root.is_some_and(|parent| {
        parent.parent() == Some(std::env::temp_dir().as_path())
            && parent
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|name| {
                    (name.starts_with("yonik-") || name.starts_with("tofu-"))
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                })
    });
    if !synthetic
        && std::env::var_os("YONIK_REPOSITORY_TEST_DIAGNOSTICS").as_deref()
            != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    let reference = std::env::var("YONIK_REPOSITORY_TEST_REF")
        .or_else(|_| std::env::var("GITHUB_SHA"))
        .ok()
        .filter(|s| (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let mut nodes = Vec::new();
    if let Some(parent) = root {
        nodes.push(json!({"node": "parent", "stat": stat(parent)}));
    }
    nodes.push(json!({"node": "database", "stat": stat(path)}));
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        nodes.push(json!({"node": suffix, "stat": stat(Path::new(&sidecar))}));
    }
    let record = json!({"event": "yonik_repository_unsafe_path", "branch": branch,
        "pid": std::process::id(), "uid": rustix::process::getuid().as_raw(),
        "test_ref": reference,
        "path": if synthetic { path.to_str() } else { None },
        "failed_node": if synthetic { node.to_str() } else { None },
        "observed": observed.map(attributes), "errno": error.and_then(|e| e.raw_os_error()), "nodes": nodes});
    eprintln!("{record}");
}
