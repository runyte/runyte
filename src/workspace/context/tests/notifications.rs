// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::time::Duration;

#[tokio::test]
async fn notifications_coalesce_until_a_channel_read_clears_the_outstanding_wake() {
    let lease = Arc::new(Lease::default());
    assert_eq!(lease.queue_notification(), "unavailable");
    lease.enable_notifications(true);
    assert_eq!(lease.queue_notification(), "queued");
    for _ in 0..100 {
        assert_eq!(lease.queue_notification(), "already_pending");
    }
    let connection = Some(lease.clone());
    tokio::time::timeout(Duration::from_secs(1), notification(&connection))
        .await
        .unwrap();
    // Publication consumes the wire event, while unread messages retain the
    // outstanding wake. Further posts must not flood an idle client.
    assert_eq!(lease.queue_notification(), "already_pending");
    assert!(
        tokio::time::timeout(Duration::from_millis(10), notification(&connection))
            .await
            .is_err()
    );
    lease.clear_notification();
    assert_eq!(lease.queue_notification(), "queued");
    tokio::time::timeout(Duration::from_secs(1), notification(&connection))
        .await
        .unwrap();
}

#[tokio::test]
async fn disabling_notifications_discards_pending_wakes_and_reenable_starts_clean() {
    let lease = Arc::new(Lease::default());
    lease.enable_notifications(true);
    assert_eq!(lease.queue_notification(), "queued");
    lease.enable_notifications(false);
    assert!(!lease.notifications_enabled());
    assert_eq!(lease.queue_notification(), "unavailable");
    let connection = Some(lease.clone());
    assert!(
        tokio::time::timeout(Duration::from_millis(10), notification(&connection))
            .await
            .is_err()
    );
    lease.enable_notifications(true);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), notification(&connection))
            .await
            .is_err()
    );
    assert_eq!(lease.queue_notification(), "queued");
    tokio::time::timeout(Duration::from_secs(1), notification(&connection))
        .await
        .unwrap();
}

#[tokio::test]
async fn revoked_connection_does_not_publish_a_previously_queued_wake() {
    let lease = Arc::new(Lease::default());
    lease.enable_notifications(true);
    assert_eq!(lease.queue_notification(), "queued");
    lease.cancel();
    assert_eq!(lease.queue_notification(), "unavailable");
    let connection = Some(lease);
    tokio::time::timeout(Duration::from_secs(1), revoked(&connection))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), notification(&connection))
            .await
            .is_err()
    );
}
