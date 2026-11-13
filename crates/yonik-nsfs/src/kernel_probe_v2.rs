//! Только proposal observation: errno никогда не даёт authority.
#![deny(unsafe_code)]
#[derive(Debug, PartialEq, Eq)]
pub struct Observation {
    pub namespace: &'static str,
    pub unshare_errno: i32,
    pub clone_selector_errno: i32,
}

/// Не принимает flags, FD, pointers или caller-controlled syscall number.
/// Вызывать только после отдельного разрешения на новые fixed probes.
pub fn observe_fixed() -> std::io::Result<Vec<Observation>> {
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    ))]
    {
        kernel::observe()
    }
    #[cfg(not(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    )))]
    {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_pointer_width = "64"
))]
#[allow(unsafe_code)]
mod kernel {
    use std::ffi::{c_long, c_ulong};
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
    }
    const SELECTORS: [(&str, c_ulong); 8] = [
        ("time", 0x00000080),
        ("mount", 0x00020000),
        ("cgroup", 0x02000000),
        ("uts", 0x04000000),
        ("ipc", 0x08000000),
        ("user", 0x10000000),
        ("pid", 0x20000000),
        ("net", 0x40000000),
    ];
    fn errno(result: c_long) -> std::io::Result<i32> {
        if result != -1 {
            return Err(std::io::Error::other(
                "Нарушен контракт invalid probe; закрыть admission",
            ));
        }
        std::io::Error::last_os_error()
            .raw_os_error()
            .ok_or_else(|| std::io::Error::other("Отсутствует errno"))
    }
    pub(super) fn observe() -> std::io::Result<Vec<super::Observation>> {
        let mut observations = Vec::with_capacity(SELECTORS.len());
        for (namespace, flag) in SELECTORS {
            // SAFETY: Linux v6.18 check_unshare_flags отвергает CLONE_PIDFD
            // до unshare_fs/userns/nsproxy; никакие указатели не передаются.
            let unshare_errno = errno(unsafe { syscall(272 as c_long, flag | 0x1000) })?;
            // SAFETY: native x86_64 clone, все 5 аргументов заданы фиксированно.
            // CLONE_THREAD без CLONE_SIGHAND отвергается copy_process до allocation.
            // NULL stack/TID/TLS не разыменовываются. time — только seccomp selector:
            // legacy clone удаляет low CSIGNAL bits, включая CLONE_NEWTIME.
            let clone_selector_errno = errno(unsafe {
                syscall(
                    56 as c_long,
                    flag | 0x10000,
                    0 as c_ulong,
                    std::ptr::null_mut::<i32>(),
                    std::ptr::null_mut::<i32>(),
                    0 as c_ulong,
                )
            })?;
            observations.push(super::Observation {
                namespace,
                unshare_errno,
                clone_selector_errno,
            });
        }
        Ok(observations)
    }
}
