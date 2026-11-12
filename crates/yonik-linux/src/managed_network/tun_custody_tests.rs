use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

type Pause = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);
thread_local! { static PAUSE: std::cell::RefCell<Option<Pause>> = const { std::cell::RefCell::new(None) }; }
pub(super) async fn after_attach() {
    let pause = PAUSE.with(|p| p.borrow_mut().take());
    if let Some((entered, resume)) = pause {
        entered.send(()).unwrap();
        let _ = resume.await;
    }
}
fn pause() -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (resume, wait) = tokio::sync::oneshot::channel();
    PAUSE.with(|p| *p.borrow_mut() = Some((entered, wait)));
    (ready, resume)
}
fn fds() -> usize {
    fs::read_dir("/proc/self/fd").unwrap().count()
}
fn ip(args: &[&str]) {
    assert!(
        std::process::Command::new("/usr/bin/ip")
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}

#[tokio::test]
#[ignore = "Только отдельный user/net namespace с YONIK_NFT_TEST_HOST_NETNS"]
async fn real_journal_queue_custody_controls() {
    let ns = fs::File::open("/proc/thread-self/ns/net").unwrap();
    let (dev, ino) = super::super::namespace(&ns).unwrap();
    let owner = Ownership::new(rustix::process::getuid().as_raw(), 7, 9, dev, ino).unwrap();
    let mut executor = TunExecutor::isolated_fixture(owner).unwrap();
    let path = std::env::temp_dir().join(format!("yonik-custody-{}", std::process::id()));
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let mut journal = Journal::open(&path, owner).unwrap();
    for framing in [Framing::Plain, Framing::Vnet10] {
        executor
            .ensure_tun_with_framing(&mut journal, 1, 7, framing == Framing::Vnet10)
            .await
            .unwrap();
        executor
            .configure_link(&mut journal, 1, 1400)
            .await
            .unwrap();
        let (keep, mut cancel) = Cancellation::new();
        let count = fds();
        let operation = super::super::lease().await.unwrap();
        let mut waiting =
            Box::pin(executor.bind_session_queue(&journal, 1, 1400, framing, &mut cancel));
        std::future::poll_fn(|cx| match std::future::Future::poll(waiting.as_mut(), cx) {
            std::task::Poll::Pending => std::task::Poll::Ready(()),
            std::task::Poll::Ready(_) => panic!("Очередь не должна обходить lease"),
        })
        .await;
        keep.send_replace(true);
        assert!(waiting.await.is_err());
        assert_eq!(fds(), count);
        drop(operation);
        let (keep, mut cancel) = Cancellation::new();
        let wrong = if framing == Framing::Plain {
            Framing::Vnet10
        } else {
            Framing::Plain
        };
        assert!(
            executor
                .bind_session_queue(&journal, 1, 1400, wrong, &mut cancel)
                .await
                .is_err()
        );
        assert_eq!(fds(), count);
        assert!(
            executor
                .bind_session_queue(&journal, 1, 1500, framing, &mut cancel)
                .await
                .is_err()
        );
        journal.require_reconciliation(owner, 1).unwrap();
        assert!(
            executor
                .bind_session_queue(&journal, 1, 1400, framing, &mut cancel)
                .await
                .is_err()
        );
        journal
            .reconcile(owner, 1, crate::managed_network::Reconciled::Owned)
            .unwrap();
        let (ready, resume) = pause();
        let mut binding =
            Box::pin(executor.bind_session_queue(&journal, 1, 1400, framing, &mut cancel));
        tokio::select! { result = &mut binding => panic!("Не достигнут attach: {}", result.is_ok()), result = ready => result.unwrap() }
        assert_eq!(fds(), count + 1);
        drop(binding);
        drop(resume);
        assert_eq!(fds(), count);
        let (ready, resume) = pause();
        let mut binding =
            Box::pin(executor.bind_session_queue(&journal, 1, 1400, framing, &mut cancel));
        tokio::select! { result = &mut binding => panic!("Не достигнут attach: {}", result.is_ok()), result = ready => result.unwrap() }
        keep.send_replace(true);
        resume.send(()).unwrap();
        assert!(binding.await.is_err());
        assert_eq!(fds(), count);
        let (keep, mut cancel) = Cancellation::new();
        let mut bound = executor
            .bind_session_queue(&journal, 1, 1400, framing, &mut cancel)
            .await
            .unwrap();
        // Transfer удерживает lease до фактического сохранения owner.
        assert!(
            super::super::OPERATIONS
                .get()
                .unwrap()
                .clone()
                .try_acquire_owned()
                .is_err()
        );
        bound.transferred();
        assert_eq!(fds(), count + 1);
        bound
            .project(
                read_topology(&mut cancel, DEADLINE).await.unwrap(),
                &journal,
            )
            .unwrap();
        let snapshot = read_topology(&mut cancel, DEADLINE).await.unwrap();
        for change in 0..5 {
            let mut foreign = snapshot.clone();
            match change {
                0 => foreign.namespace.inode ^= 1,
                1 => {
                    foreign
                        .links
                        .iter_mut()
                        .find(|l| l.index == bound.index)
                        .unwrap()
                        .index += 100
                }
                2 => {
                    foreign
                        .links
                        .iter_mut()
                        .find(|l| l.index == bound.index)
                        .unwrap()
                        .name = b"foreign0".to_vec()
                }
                3 => {
                    foreign
                        .links
                        .iter_mut()
                        .find(|l| l.index == bound.index)
                        .unwrap()
                        .mtu = Some(1500)
                }
                _ => foreign
                    .links
                    .iter_mut()
                    .find(|l| l.index == bound.index)
                    .unwrap()
                    .unknown
                    .retain(|a| a.kind != 18),
            }
            assert!(bound.project(foreign, &journal).is_err());
        }
        assert!(
            executor
                .bind_session_queue(&journal, 1, 1400, framing, &mut cancel)
                .await
                .is_err()
        );
        ip(&["link", "set", "dev", "tofu00000007", "alias", "foreign"]);
        assert!(
            bound
                .project(
                    read_topology(&mut cancel, DEADLINE).await.unwrap(),
                    &journal
                )
                .is_err()
        );
        let alias = String::from_utf8(super::super::alias(owner, 1).unwrap()).unwrap();
        ip(&["link", "set", "dev", "tofu00000007", "alias", &alias]);
        drop(bound);
        assert_eq!(fds(), count);
        keep.send_replace(true);
        assert!(
            executor
                .bind_session_queue(&journal, 1, 1400, framing, &mut cancel)
                .await
                .is_err()
        );
        assert_eq!(fds(), count);
        let (_keep, mut cancel) = Cancellation::new();
        let mut bound = executor
            .bind_session_queue(&journal, 1, 1400, framing, &mut cancel)
            .await
            .unwrap();
        bound.transferred();
        // Revoke закрывает единственную очередь, затем разрешён durable cleanup.
        drop(bound);
        executor.remove_tun(&mut journal, 1).await.unwrap();
    }
    drop(journal);
    fs::remove_file(path.join("journal.json")).unwrap();
    fs::remove_dir(path).unwrap();
}
