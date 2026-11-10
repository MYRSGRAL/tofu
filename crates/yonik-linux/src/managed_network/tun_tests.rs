use super::*;
use crate::LinuxLink;

fn owner() -> Ownership {
    Ownership::new(1000, 7, 9, 3, 11).unwrap()
}
fn link(name_id: u32, marker: Option<Vec<u8>>) -> LinuxLink {
    LinuxLink {
        index: 7,
        link_type: 65534,
        flags: 0,
        name: name(name_id).unwrap().into_bytes(),
        alias: marker,
        mtu: Some(1500),
        master: None,
        parent: None,
        kind: Some(b"tun".to_vec()),
        operational_state: None,
        unknown: vec![],
    }
}
#[test]
fn creation_has_only_closed_single_queue_arguments() {
    assert_eq!(
        create_arguments(0x12345678, 1000).unwrap(),
        [
            "tuntap",
            "add",
            "dev",
            "tofu12345678",
            "mode",
            "tun",
            "user",
            "1000"
        ]
    );
    assert!(create_arguments(0, 1000).is_err());
    assert!(alias(owner(), 0).is_err());
    assert!(alias(owner(), 1).unwrap().len() < 255);
}
#[test]
fn ownership_requires_full_alias_kind_and_unambiguous_name() {
    let marker = alias(owner(), 1).unwrap();
    assert_eq!(observed(&[], 1, &marker), Ok(None));
    assert_eq!(
        observed(&[link(1, Some(marker.clone()))], 1, &marker),
        Ok(Some(7))
    );
    assert_eq!(
        observed(&[link(1, None)], 1, &marker),
        Err(TunError::Foreign)
    );
    let mut wrong = link(1, Some(marker.clone()));
    wrong.kind = Some(b"dummy".to_vec());
    assert_eq!(observed(&[wrong], 1, &marker), Err(TunError::Foreign));
    let mut tap = link(1, Some(marker.clone()));
    tap.link_type = 1;
    assert_eq!(observed(&[tap], 1, &marker), Err(TunError::Foreign));
    assert_eq!(
        observed(
            &[link(1, Some(marker.clone())), link(1, Some(marker.clone()))],
            1,
            &marker
        ),
        Err(TunError::Foreign)
    );
    for changed in [
        Ownership::new(1001, 7, 9, 3, 11).unwrap(),
        Ownership::new(1000, 8, 9, 3, 11).unwrap(),
        Ownership::new(1000, 7, 10, 3, 11).unwrap(),
        Ownership::new(1000, 7, 9, 4, 11).unwrap(),
        Ownership::new(1000, 7, 9, 3, 12).unwrap(),
    ] {
        assert_ne!(alias(changed, 1).unwrap(), marker);
    }
    assert_ne!(alias(owner(), 2).unwrap(), marker);
    let mut renamed = link(1, Some(marker.clone()));
    renamed.name = b"renamed0".to_vec();
    assert_eq!(observed(&[renamed], 1, &marker), Err(TunError::Evidence));
}

#[tokio::test]
#[ignore = "Требует отдельные user/net/mount/pid namespaces и приватные /run, /tmp"]
async fn real_namespace_tun_ownership_reopen_and_cleanup() {
    let ns = File::open("/proc/thread-self/ns/net").unwrap();
    let (device, inode) = namespace(&ns).unwrap();
    let owner = Ownership::new(rustix::process::getuid().as_raw(), 7, 9, device, inode).unwrap();
    // Проверка isolation происходит до первого эффекта.
    let mut executor = TunExecutor::isolated_fixture(owner).unwrap();
    let mut journal = Journal::open(std::path::Path::new("/tmp/yonik-tun-journal"), owner).unwrap();
    let index = executor.ensure_tun(&mut journal, 1, 1).await.unwrap();
    // ip tuntap add использует IFF_TUN_EXCL: уже занятый TUN нельзя открыть
    // как успешное новое создание между предварительной сверкой и эффектом.
    assert_eq!(
        executor.create(1, lease().await.unwrap()).await,
        Err(TunError::Rejected)
    );
    assert_eq!(
        observed(
            &executor.links().await.unwrap(),
            1,
            &alias(owner, 1).unwrap()
        ),
        Ok(Some(index))
    );
    assert!(!journal.needs_reconciliation(1));
    assert_eq!(
        executor.ensure_tun(&mut journal, 1, 1).await.unwrap(),
        index
    );
    drop(executor);
    drop(journal);
    let mut executor = TunExecutor::isolated_fixture(owner).unwrap();
    let mut journal = Journal::open(std::path::Path::new("/tmp/yonik-tun-journal"), owner).unwrap();
    assert!(journal.needs_reconciliation(1));
    assert_eq!(
        executor.ensure_tun(&mut journal, 1, 1).await.unwrap(),
        index
    );
    let status = std::process::Command::new("/usr/bin/ip")
        .args(["link", "set", "dev", "tofu00000001", "name", "renamed0"])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        executor.remove_tun(&mut journal, 1).await,
        Err(TunError::Evidence)
    );
    assert!(journal.records().iter().any(|record| record.id() == 1));
    assert!(journal.needs_reconciliation(1));
    let status = std::process::Command::new("/usr/bin/ip")
        .args(["link", "set", "dev", "renamed0", "name", "tofu00000001"])
        .status()
        .unwrap();
    assert!(status.success());
    executor.remove_tun(&mut journal, 1).await.unwrap();
    assert!(journal.records().is_empty());
    assert!(
        executor
            .links()
            .await
            .unwrap()
            .iter()
            .all(|l| l.index != index)
    );

    // Непомеченный persistent TUN после crash-before-alias остаётся неизвестным.
    let status = std::process::Command::new("/usr/bin/ip")
        .args(create_arguments(2, owner.owner_uid()).unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        executor.ensure_tun(&mut journal, 2, 2).await,
        Err(TunError::Foreign)
    );
    assert_eq!(
        executor.remove_tun(&mut journal, 2).await,
        Err(TunError::Foreign)
    );
    let foreign = executor
        .links()
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == b"tofu00000002")
        .unwrap();
    assert!(foreign.alias.as_ref().is_none_or(Vec::is_empty));
    assert!(journal.needs_reconciliation(2));
    // Только test fixture сама удаляет созданный ей чужой объект по ifindex.
    write::run(
        write::Action::Delete {
            index: foreign.index,
        },
        lease().await.unwrap(),
    )
    .await
    .unwrap();
    executor.remove_tun(&mut journal, 2).await.unwrap();
    assert!(journal.records().is_empty());

    let index = executor.ensure_tun(&mut journal, 3, 3).await.unwrap();
    // Подмена alias запрещает cleanup, даже если имя и ifindex совпадают.
    write::run(
        write::Action::SetAlias {
            index,
            alias: b"foreign-owner".to_vec(),
        },
        lease().await.unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        executor.remove_tun(&mut journal, 3).await,
        Err(TunError::Foreign)
    );
    assert!(journal.needs_reconciliation(3));
    write::run(
        write::Action::SetAlias {
            index,
            alias: alias(owner, 3).unwrap(),
        },
        lease().await.unwrap(),
    )
    .await
    .unwrap();
    executor.remove_tun(&mut journal, 3).await.unwrap();
    assert!(journal.records().is_empty());
    assert_eq!(executor.links().await.unwrap().len(), 1);
}

#[tokio::test]
async fn cancelled_child_retains_operation_fence_until_reap() {
    let operation = lease().await.unwrap();
    let child = Command::new("/usr/bin/sleep")
        .arg("20")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let running = own_child(child, operation).unwrap();
    drop(running);
    // Этот permit не выдаётся до kill/wait предыдущего Child.
    let _next = lease().await.unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}
