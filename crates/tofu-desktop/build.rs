fn main() {
    #[cfg(all(feature = "gui", target_os = "linux"))]
    {
        use cxx_qt_build::{CxxQtBuilder, QmlModule};

        CxxQtBuilder::new_qml_module(
            QmlModule::new("cloud.highpriority.tofu").qml_file("qml/Main.qml"),
        )
        .file("src/bridge.rs")
        .cpp_file("src/application.cpp")
        .include_dir("src")
        .qt_module("Quick")
        .qt_module("Widgets")
        .qt_module("Network")
        .qt_module("DBus")
        .build();
        println!("cargo::rerun-if-changed=src/application.h");
        println!("cargo::rerun-if-changed=src/application.cpp");
    }
}
