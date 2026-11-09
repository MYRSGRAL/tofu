use crate::bridge_impl::DesktopBridgeRust;

// Только сгенерированный CXX-Qt FFI использует unsafe; методы ниже — safe Rust.
#[cxx_qt::bridge(namespace = "tofu")]
pub mod ffi {
    #[namespace = ""]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "C++" {
        include!("application.h");
        fn run_application(
            data_dir: &QString,
            smoke_test: bool,
            screenshot: &QString,
            script: &QString,
            trace: &QString,
        ) -> i32;
    }

    impl cxx_qt::Threading for DesktopBridge {}

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, snapshot_json)]
        type DesktopBridge = super::DesktopBridgeRust;

        #[qinvokable]
        fn initialize(self: Pin<&mut DesktopBridge>);
        #[qinvokable]
        fn dispatch(self: Pin<&mut DesktopBridge>, request: QString);
        #[qinvokable]
        fn shutdown(self: Pin<&mut DesktopBridge>);
    }
}
