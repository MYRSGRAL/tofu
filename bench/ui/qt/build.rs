fn main() {
    use cxx_qt_build::{CxxQtBuilder, QmlModule};
    CxxQtBuilder::new_qml_module(QmlModule::new("tofu.benchmark").qml_file("qml/Main.qml"))
        .file("src/bridge.rs")
        .cpp_file("src/application.cpp")
        .include_dir("src")
        .qt_module("Gui")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .build();
    println!("cargo::rerun-if-changed=src/application.h");
}
