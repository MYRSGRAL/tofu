#![forbid(unsafe_code)]

//! Desktop host выполняет I/O вне Qt-потока; GUI получает только снимки и команды.

#[cfg(target_os = "linux")]
mod backend;
#[cfg(target_os = "linux")]
pub use backend::{DesktopError, DesktopHost, HostConfig};

#[cfg(target_os = "linux")]
mod preferences;

pub mod editor;

pub mod diagnostics;
#[cfg(target_os = "linux")]
pub mod system_session;

#[cfg(target_os = "linux")]
pub mod maintenance;

#[cfg(all(target_os = "linux", feature = "startup-diagnostics"))]
pub use backend::StartupPhase;

#[cfg(target_os = "linux")]
pub mod local_import;
