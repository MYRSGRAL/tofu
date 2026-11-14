//! Узкие fixture tests fixed adapter; запускаются только после backend receipt.
#![cfg(target_os = "linux")]
use std::{
    os::unix::process::ExitStatusExt,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use yonik_nsfs::local_parent_death::install_kill_on_parent_death;

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture(exit: bool) -> OwnedChild {
    let mut command = Command::new(std::env::current_exe().expect("test ELF"));
    command
        .args(["--ignored", "--exact", "fixed_child_fixture"])
        .env("YONIK_P09_CHILD", if exit { "exit" } else { "live" })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    install_kill_on_parent_death(&mut command);
    OwnedChild(command.spawn().expect("fixed fixture spawn"))
}

#[test]
#[ignore = "только дочерняя fixture, не самостоятельный acceptance"]
fn fixed_child_fixture() {
    match std::env::var("YONIK_P09_CHILD").as_deref() {
        Ok("exit") => {}
        Ok("live") => std::thread::sleep(Duration::from_secs(30)),
        _ => panic!("нет fixture role"),
    }
}

#[test]
fn live_owner_normal_kill_wait_and_already_exited() {
    let mut live = fixture(false);
    assert!(live.0.try_wait().expect("try_wait").is_none());
    live.0.kill().expect("kill own fixture");
    assert_eq!(live.0.wait().expect("wait").signal(), Some(9));
    assert!(live.0.try_wait().expect("already exited").is_some());
    let mut exited = fixture(true);
    assert!(exited.0.wait().expect("normal exit").success());
    assert!(exited.0.try_wait().expect("repeat wait").is_some());
}

#[test]
fn spawning_thread_exit_kills_child_while_process_lives() {
    let (send, receive) = mpsc::channel();
    let owner = std::thread::spawn(move || {
        let child = fixture(false);
        send.send(child).expect("send owned child");
    });
    let mut child = receive.recv_timeout(Duration::from_secs(5)).expect("child");
    owner.join().expect("owner joined");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.0.try_wait().expect("thread-exit wait") {
            assert_eq!(status.signal(), Some(9));
            break;
        }
        assert!(Instant::now() < deadline, "core пережил owner thread");
        std::thread::sleep(Duration::from_millis(10));
    }
}
