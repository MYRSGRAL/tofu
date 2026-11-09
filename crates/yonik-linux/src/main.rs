#[cfg(target_os = "linux")]
mod cli;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    cli::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("yonik-local требует Linux");
    std::process::ExitCode::FAILURE
}
