//! Предложение, НЕ production authority. Только fixed invalid probes.
#![deny(unsafe_code)]
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub unshare: i32,
    pub setns: i32,
    pub clone3: i32,
}
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn observe() -> std::io::Result<Report> {
    kernel::observe()
}
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn observe() -> std::io::Result<Report> {
    Err(std::io::ErrorKind::Unsupported.into())
}
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[allow(unsafe_code)]
mod kernel {
    use std::ffi::c_long;
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
    }
    #[cfg(target_arch = "x86_64")]
    const UNSHARE: c_long = 272;
    #[cfg(target_arch = "x86_64")]
    const SETNS: c_long = 308;
    #[cfg(target_arch = "aarch64")]
    const UNSHARE: c_long = 97;
    #[cfg(target_arch = "aarch64")]
    const SETNS: c_long = 268;
    const CLONE3: c_long = 435;
    fn error(result: c_long) -> std::io::Result<i32> {
        if result != -1 {
            return Err(std::io::Error::other("Невозможный успех invalid probe"));
        }
        std::io::Error::last_os_error()
            .raw_os_error()
            .ok_or_else(|| std::io::Error::other("Нет errno"))
    }
    pub(super) fn observe() -> std::io::Result<super::Report> {
        // SAFETY: неизвестные bits гарантируют invalid flags; нет pointers/изменения namespace.
        let unshare = error(unsafe { syscall(UNSHARE, -1 as c_long) })?;
        // SAFETY: -1 никогда не является живым FD; flags=0, namespace не меняется.
        let setns = error(unsafe { syscall(SETNS, -1 as c_long, 0 as c_long) })?;
        // SAFETY: NULL,size=0 отклоняются до чтения clone_args/создания child.
        let clone3 = error(unsafe { syscall(CLONE3, std::ptr::null::<u8>(), 0usize) })?;
        Ok(super::Report {
            unshare,
            setns,
            clone3,
        })
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn probes_preserve_identity() {
        let before: Vec<_> = ["net", "pid", "mnt"]
            .map(|n| std::fs::read_link(format!("/proc/self/ns/{n}")).unwrap())
            .into();
        let report = super::observe().unwrap();
        eprintln!("Наблюдение host: {report:?}");
        assert!([1, 22].contains(&report.unshare));
        assert!([1, 9].contains(&report.setns));
        assert!([1, 22, 38].contains(&report.clone3));
        let after: Vec<_> = ["net", "pid", "mnt"]
            .map(|n| std::fs::read_link(format!("/proc/self/ns/{n}")).unwrap())
            .into();
        assert_eq!(before, after);
    }
}
