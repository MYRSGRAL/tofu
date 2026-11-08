//! Linux host локального SOCKS. Не меняет системные маршруты, proxy или DNS.

#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
pub use host::*;
