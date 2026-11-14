//! Fixed SIGKILL при потере родителя локального sealed core; не worker API.
// Target guard не доказывает backend: требуется отдельный exact-build receipt.
#[cfg(not(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_pointer_width = "64",
    target_env = "gnu"
)))]
compile_error!("local parent-death: требуется verified x86_64-unknown-linux-gnu/raw backend");

use rustix::process::{Signal, getpid, getppid, set_parent_process_death_signal};
use std::{io, os::unix::process::CommandExt, process::Command};

/// Устанавливается только на command локального sealed BinarySnapshot.
/// PID захватывается внутри; caller не задаёт signal, PID или syscall arguments.
#[allow(unsafe_code)]
pub fn install_kill_on_parent_death(command: &mut Command) {
    let expected_parent = getpid();
    // SAFETY: после fork closure использует только getppid/prctl raw wrappers,
    // Copy PID и errno-only io::Error. Нет allocator, locks, logging, env,
    // доступа к унаследованным library state/FD. Proof условен: rustix 1.1.6
    // linux_raw, LP64 и штатный getppid без errno emulation; dependency debug
    // unwrap/assert существуют, но недостижимы при этом предусловии.
    // Иной backend/ABI или неизвестная getppid sandbox policy не поддержаны.
    // SIGKILL фиксирован; несовпадение родителя/ошибка запрещают exec.
    unsafe {
        command.pre_exec(move || {
            if getppid() != Some(expected_parent) {
                return Err(io::Error::from_raw_os_error(
                    rustix::io::Errno::SRCH.raw_os_error(),
                ));
            }
            set_parent_process_death_signal(Some(Signal::KILL))
                .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))?;
            if getppid() != Some(expected_parent) {
                return Err(io::Error::from_raw_os_error(
                    rustix::io::Errno::SRCH.raw_os_error(),
                ));
            }
            Ok(())
        });
    }
}
