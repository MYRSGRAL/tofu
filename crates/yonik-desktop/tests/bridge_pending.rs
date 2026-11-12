#[path = "../src/bridge_pending.rs"]
mod bridge_pending;

use bridge_pending::PendingSnapshot;

#[test]
fn старая_очередь_не_перетирает_повторный_initialize() {
    let mut first = PendingSnapshot::default();
    assert!(first.publish("typed failure".into()));
    let mut visible = first.take_start_failure().expect("синхронная публикация");
    let mut second = PendingSnapshot::default();
    assert!(second.publish("next initialize".into()));
    if let Some(value) = second.take_queued() {
        visible = value;
    }
    if let Some(value) = first.take_queued() {
        visible = value;
    }
    assert_eq!(visible, "next initialize");
}

#[test]
fn fallback_только_при_отсутствии_публикации() {
    let mut pending = PendingSnapshot::default();
    assert_eq!(
        pending.take_start_failure().as_deref().unwrap_or("startup"),
        "startup"
    );
    assert!(pending.publish("typed failure".into()));
    assert_eq!(
        pending.take_start_failure().as_deref().unwrap_or("startup"),
        "typed failure"
    );
    assert_eq!(pending.take_queued(), None);
}

#[test]
fn coalescing_и_отказ_очереди_сохраняют_прежнюю_семантику() {
    let mut pending = PendingSnapshot::default();
    assert!(pending.publish("first".into()));
    assert!(!pending.publish("latest".into()));
    assert_eq!(pending.take_queued().as_deref(), Some("latest"));
    assert!(pending.publish("rejected".into()));
    pending.queue_rejected();
    assert_eq!(pending.take_start_failure().as_deref(), Some("rejected"));
    assert_eq!(pending.take_queued(), None);
    assert!(pending.publish("retry".into()));
}

#[test]
fn отказ_qt_queue_не_теряет_typed_startup() {
    let mut pending = PendingSnapshot::default();
    assert!(pending.publish("typed failure".into()));
    pending.queue_rejected();
    assert_eq!(
        pending.take_start_failure().as_deref(),
        Some("typed failure")
    );
    assert_eq!(pending.take_queued(), None);
}
