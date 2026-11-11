//! Actual Service/scheduler через SOCKS принадлежащей Host native-сессии.
use crate::{Directory, service, source};
use std::{num::NonZeroU64, sync::Arc, time::Duration};
use yonik_linux::SubscriptionPolicy as Policy;
use yonik_linux::{FetchHost, RefreshResult, SubscriptionScheduleEvent, SubscriptionScheduler};

pub async fn matrix(host: Arc<FetchHost>, base: &str) {
    for scheduled in [false, true] {
        for case in [
            "full",
            "credentials",
            "partial",
            "empty",
            "corrupt",
            "http",
            "304",
            "redirect",
            "cross",
            "size",
            "proxyfail",
            "tls",
            "bad_tls",
            "cancel",
            "stale",
            "admitted",
        ] {
            let dir = Directory::new();
            let svc = service(&dir).await;
            let id = svc
                .add(source(format!("{base}/old?token=old-canary")))
                .await
                .unwrap();
            let (_sender, cancel) = tokio::sync::watch::channel(false);
            let RefreshResult::Pending(pending) = svc
                .refresh_with_host(id, None, host.as_ref(), cancel)
                .await
                .unwrap()
            else {
                panic!("baseline")
            };
            svc.commit(pending.accept_clean().unwrap()).await.unwrap();
            let endpoint = if matches!(case, "cancel" | "stale" | "admitted") {
                "partial"
            } else {
                case
            };
            let target = match case {
                "proxyfail" => std::env::var("P05_PROXY_FAIL_URL").unwrap(),
                "tls" => format!("{}/full", std::env::var("P05_TLS_URL").unwrap()),
                "bad_tls" => format!("{}/full", std::env::var("P05_BAD_TLS_URL").unwrap()),
                _ => format!("{base}/{endpoint}?token=new-canary"),
            };
            svc.change_source(id, 1, source(target)).await.unwrap();
            let mut scheduler = SubscriptionScheduler::new(NonZeroU64::MIN, 1, 1).unwrap();
            let success = matches!(
                case,
                "full"
                    | "tls"
                    | "credentials"
                    | "partial"
                    | "redirect"
                    | "cross"
                    | "cancel"
                    | "stale"
                    | "admitted"
            );
            if scheduled {
                scheduler.schedule(id, 2, Duration::ZERO).unwrap();
                scheduler
                    .dispatch(
                        Duration::ZERO,
                        Policy {
                            online: true,
                            ui_visible: true,
                            ..Default::default()
                        },
                        &svc,
                        host.clone(),
                    )
                    .unwrap();
                let event = tokio::time::timeout(
                    Duration::from_secs(5),
                    scheduler.next_event(|| Duration::ZERO),
                )
                .await
                .unwrap()
                .unwrap()
                .unwrap();
                assert_eq!(
                    matches!(event, SubscriptionScheduleEvent::Review { .. }),
                    success,
                    "{case}"
                );
                if success {
                    assert_eq!(svc.offline(id).await.unwrap().revision, 1);
                    if case == "cancel" {
                        svc.cancel(id).await;
                    }
                    if case == "stale" {
                        let other = service(&dir).await;
                        other
                            .change_source(id, 2, source(format!("{base}/full")))
                            .await
                            .unwrap();
                        other.close().await.unwrap();
                    }
                    let result = scheduler
                        .finish_review(&svc, id, true, || Duration::ZERO)
                        .await;
                    assert_eq!(
                        result.is_ok(),
                        !matches!(case, "cancel" | "stale"),
                        "{case}"
                    );
                }
            } else {
                let (_sender, cancel) = tokio::sync::watch::channel(false);
                match svc.refresh_with_host(id, None, host.as_ref(), cancel).await {
                    Ok(RefreshResult::Pending(pending)) => {
                        assert!(success);
                        // Pending выдан только после полного response+parse; commit ещё не вызван.
                        assert_eq!(svc.offline(id).await.unwrap().revision, 1);
                        if case == "cancel" {
                            svc.cancel(id).await;
                        }
                        if case == "stale" {
                            let other = service(&dir).await;
                            other
                                .change_source(id, 2, source(format!("{base}/full")))
                                .await
                                .unwrap();
                            other.close().await.unwrap();
                        }
                        let result = svc.commit(pending.approve_after_review()).await;
                        assert_eq!(
                            result.is_ok(),
                            !matches!(case, "cancel" | "stale"),
                            "{case}"
                        );
                    }
                    Err(_) => assert!(!success, "{case}"),
                    Ok(_) => panic!("304 после rotation не имеет validators"),
                }
            }
            if case == "admitted" {
                svc.cancel(id).await;
            }
            assert_eq!(
                svc.offline(id).await.unwrap().revision,
                if success && !matches!(case, "cancel" | "stale") {
                    2
                } else {
                    1
                },
                "{case}, scheduled={scheduled}"
            );
            scheduler.shutdown(Duration::ZERO).await.unwrap();
            svc.close().await.unwrap();
            println!("p05_service case={case} scheduled={scheduled} passed");
        }
    }
}
