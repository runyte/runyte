// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::{notification::NotificationSeverity, plugin::handoff};
use std::time::{Duration, Instant};

fn notification(title: &str, body: &str) -> handoff::Notification {
    handoff::Notification {
        severity: handoff::Severity::Warning,
        title: title.into(),
        body: body.into(),
    }
}
fn ready(host: &mut WorkspaceHost, owner: usize) {
    host.app
        .plugins
        .instances
        .get_mut(&owner)
        .unwrap()
        .application
        .notification_at = Some(Instant::now() - Duration::from_secs(1));
}
fn present(host: &mut WorkspaceHost) {
    host.prepare_frame(crate::ui::frame_geometry(ratatui::layout::Rect::new(
        0, 0, 80, 24,
    )));
}

#[test]
fn detached_notifications_are_owner_labelled_and_do_not_take_foreground_feedback() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["notifications"]);
    next(&mut output);
    host.app.note_plugin_frontend(false);
    seed(&mut host, "unchanged editor document");
    host.app.status = "An existing action is still active".into();
    let pane = host.app.active_pane;
    let selection = host.app.active().selection.clone();
    let buffer = host.app.active().buffer;
    host.take_plugin_presentation_change();
    request(
        &mut host,
        0,
        1,
        api::Request::NotificationPublish(notification(
            "Transfer failed",
            "Inspect before retrying.",
        )),
    );
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Response {
            outcome: api::Response::Success {
                result: api::ResultValue::Empty(_)
            },
            ..
        }
    ));
    assert_eq!(host.app.status, "An existing action is still active");
    assert!(
        host.take_plugin_presentation_change(),
        "background feedback must wake presentation without further input"
    );
    assert!(
        !host.plugin_presentation_pending(),
        "one update does not leave an idle wake armed"
    );
    assert_eq!(host.app.active_pane, pane);
    assert_eq!(host.app.active().buffer, buffer);
    assert_eq!(host.app.active().selection, selection);
    assert_eq!(
        host.app.buffers[buffer].text().to_string(),
        "unchanged editor document"
    );
    let record = host.app.notifications().entries().first().unwrap();
    assert_eq!(record.source, "Plugin app-0");
    assert_eq!(record.severity, NotificationSeverity::Warning);
    assert_eq!(record.title, "Transfer failed");
    assert_eq!(host.app.unread_notification_counts().warnings, 1);
}

#[test]
fn capability_and_malformed_feedback_refusals_leave_history_and_pacing_untouched() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &[]);
    next(&mut output);
    assert_eq!(
        host.application_notification_request(0, notification("Title", "Body"))
            .unwrap_err()
            .code,
        api::ErrorCode::CapabilityDenied
    );
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .capabilities
        .insert("notifications".into());
    let count = host.app.notifications().entries().len();
    for value in [
        notification("", "body"),
        notification("bad\x1btitle", "body"),
        notification(&"é".repeat(81), "body"),
        notification("Title", &"猫".repeat(1366)),
        notification("Title", &"line\n".repeat(64)),
        notification("Title", "bad\rbody"),
        notification("Title", "bad\0body"),
    ] {
        assert_eq!(
            host.application_notification_request(0, value)
                .unwrap_err()
                .code,
            api::ErrorCode::InvalidArgument
        );
        assert!(
            host.app.plugins.instances[&0]
                .application
                .notification_at
                .is_none()
        );
        assert_eq!(host.app.notifications().entries().len(), count);
    }
    host.application_notification_request(0, notification(&"é".repeat(80), "line\n\tindent"))
        .unwrap();
}

#[test]
fn publication_pacing_is_per_owner_and_rejections_do_not_extend_the_interval() {
    let (_root, mut host) = host();
    for owner in [0, 1] {
        let mut output = setup(&mut host, owner, &["notifications"]);
        next(&mut output);
    }
    host.application_notification_request(0, notification("One", ""))
        .unwrap();
    let accepted = host.app.plugins.instances[&0].application.notification_at;
    assert_eq!(
        host.application_notification_request(0, notification("Two", ""))
            .unwrap_err()
            .code,
        api::ErrorCode::Busy
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.notification_at,
        accepted
    );
    host.application_notification_request(1, notification("Other owner", ""))
        .unwrap();
    ready(&mut host, 0);
    host.application_notification_request(0, notification("Two", ""))
        .unwrap();
    assert_eq!(host.app.notifications().entries().len(), 3);
}

#[test]
fn a_background_batch_updates_live_notification_documents_at_presentation() {
    let (_root, mut host) = host();
    for owner in [0, 1] {
        let mut output = setup(&mut host, owner, &["notifications"]);
        next(&mut output);
    }
    type_command(&mut host, "notifications");
    let buffer = host.app.active().buffer;
    assert!(host.app.buffers[buffer].is_notifications());
    let before = host.app.buffers[buffer].text().to_string();
    let selection = host.app.active().selection.clone();
    host.application_notification_request(0, notification("First background event", "body"))
        .unwrap();
    host.application_notification_request(1, notification("Second background event", "body"))
        .unwrap();
    assert_eq!(
        host.app.buffers[buffer].text().to_string(),
        before,
        "projection waits for one presentation checkpoint"
    );
    present(&mut host);
    let rendered = host.app.buffers[buffer].text().to_string();
    assert!(rendered.contains("First background event"));
    assert!(rendered.contains("Second background event"));
    assert_eq!(host.app.active().selection, selection);
    assert_eq!(host.app.unread_notification_counts().warnings, 2);
    present(&mut host);
    assert_eq!(host.app.buffers[buffer].text().to_string(), rendered);
}

#[test]
fn repeated_feedback_aggregates_and_survives_owner_stop_without_protecting_retirement() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["notifications"]);
    next(&mut output);
    for _ in 0..3 {
        ready(&mut host, 0);
        host.application_notification_request(
            0,
            notification("Connection failed", "Inspect configuration"),
        )
        .unwrap();
    }
    assert_eq!(host.app.notifications().entries().len(), 1);
    assert_eq!(host.app.notifications().entries()[0].occurrences, 3);
    assert!(host.may_retire_idle());
    host.stop_plugin(0, "Stopped");
    assert!(
        host.app
            .notifications()
            .entries()
            .iter()
            .any(|entry| entry.title == "Connection failed" && entry.occurrences == 3)
    );
    assert!(host.may_retire_idle());
}
