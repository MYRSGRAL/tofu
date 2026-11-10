#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    use yonik_linux::worker::{WorkerMode, serve};
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let mode = match arguments.as_slice() {
        [argument] if argument == "--check" => WorkerMode::Check,
        [argument] if argument == "--run" => WorkerMode::Run,
        _ => return std::process::ExitCode::FAILURE,
    };
    match serve(mode).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {}
