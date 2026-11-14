#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    use yonik_linux::worker::{WorkerMode, serve};
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if matches!(arguments.as_slice(), [argument] if argument == "--lookup") {
        return if yonik_linux::attribution::broker::serve_lookup().is_ok() {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        };
    }
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
