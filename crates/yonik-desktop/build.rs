use std::{env, path::Path, process::Command};

fn valid_build_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
}

fn git_output(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(arguments)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn build_id() -> String {
    match env::var("YONIK_BUILD_ID") {
        Ok(value) if valid_build_id(&value) => return value,
        // Значение не печатается: переменная CI может случайно содержать секрет.
        Ok(_) | Err(env::VarError::NotUnicode(_)) => {
            panic!("Некорректный YONIK_BUILD_ID: требуется 1–80 печатных ASCII-символов")
        }
        Err(env::VarError::NotPresent) => (),
    }
    let Some(revision) = git_output(&["rev-parse", "--short=12", "HEAD"]).filter(|value| {
        !value.is_empty() && value.len() <= 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) else {
        return "0.1.0+source".into();
    };
    let dirty = git_output(&["status", "--porcelain", "--untracked-files=normal"])
        .is_none_or(|status| !status.is_empty());
    format!("0.1.0+{revision}{}", if dirty { "-dirty" } else { "" })
}

fn track_build_id() {
    println!("cargo:rerun-if-env-changed=YONIK_BUILD_ID");
    // Не отслеживаем local/evidence: запись диагностического лога не должна
    // повторно запускать дорогую сборку CXX-Qt. Новые Rust/QML-файлы учитываются.
    for path in ["../../crates", "../../Cargo.toml", "../../Cargo.lock"] {
        println!("cargo:rerun-if-changed={path}");
    }
    for name in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git_output(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git_output(&["symbolic-ref", "--quiet", "HEAD"]) {
        if let Some(path) = git_output(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
            // Новый ref может ещё отсутствовать после переключения ветки.
            if let Some(parent) = Path::new(&path).parent() {
                println!("cargo:rerun-if-changed={}", parent.display());
            }
        }
    }
}

fn main() {
    track_build_id();
    println!("cargo:rustc-env=YONIK_BUILD_ID={}", build_id());
    #[cfg(all(feature = "gui", target_os = "linux"))]
    {
        use cxx_qt_build::{CxxQtBuilder, QmlModule};

        CxxQtBuilder::new_qml_module(
            QmlModule::new("cloud.highpriority.yonik")
                .qml_file("qml/Main.qml")
                .qml_file("qml/DiagnosticsExport.qml")
                .qml_file("qml/LogsView.qml"),
        )
        .file("src/bridge.rs")
        .cpp_file("src/application.cpp")
        .qrc("resources.qrc")
        .include_dir("src")
        .qt_module("Quick")
        .qt_module("Widgets")
        .qt_module("Network")
        .qt_module("DBus")
        .build();
        println!("cargo::rerun-if-changed=src/application.h");
        println!("cargo::rerun-if-changed=src/application.cpp");
        println!("cargo::rerun-if-changed=src/ui_smoke.h");
    }
}

#[cfg(test)]
mod tests {
    use super::valid_build_id;

    #[test]
    fn accepts_bounded_printable_ascii_only() {
        for valid in ["0.1.0+source", "release-2026.11", &"a".repeat(80)] {
            assert!(valid_build_id(valid));
        }
        for invalid in [
            "",
            "\n",
            "v1\rnext",
            "\t",
            "\x7f",
            "версия",
            &"a".repeat(81),
        ] {
            assert!(!valid_build_id(invalid));
        }
    }
}
