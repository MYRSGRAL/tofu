//! Искусственное ядро только для одноразовой systemd VM; не registry artifact.
use std::{
    io::{self, Read},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["version"] {
        println!("Xray 26.10.31");
        return Ok(());
    }
    let checking = args == ["run", "-test", "-c", "stdin:"];
    if !checking && args != ["run", "-c", "stdin:"] {
        return Err(io::Error::other("Недопустимые аргументы fixture"));
    }
    // Конфигурация ограничена даже в искусственном ядре и никогда не печатается.
    let size = io::copy(&mut io::stdin().take(1024 * 1024 + 1), &mut io::sink())?;
    if size == 0 || size > 1024 * 1024 {
        return Err(io::Error::other("Недопустимый размер конфигурации"));
    }
    if checking {
        return Ok(());
    }
    let status = Command::new("/usr/bin/setsid")
        .args(["--fork", "/usr/bin/sleep", "300"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(io::Error::other("Не создана отдельная сессия fixture"));
    }
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
