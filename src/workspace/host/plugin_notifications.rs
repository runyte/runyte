// SPDX-License-Identifier: MPL-2.0

//! Retained, owner-labelled feedback without foreground takeover or timers.
use super::WorkspaceHost;
use crate::{
    notification::{NotificationDraft, NotificationSeverity},
    plugin::{application as api, handoff},
};
use std::time::{Duration, Instant};

const NOTIFICATION_INTERVAL: Duration = Duration::from_millis(500);

impl WorkspaceHost {
    pub(super) fn application_notification_request(
        &mut self,
        owner: usize,
        notification: handoff::Notification,
    ) -> Result<api::ResultValue, api::Error> {
        let instance = &self.app.plugins.instances[&owner];
        if !instance.application.capabilities.contains("notifications") {
            return Err(api::Error::new(
                api::ErrorCode::CapabilityDenied,
                "Notifications capability was not granted",
            ));
        }
        if notification.title.is_empty()
            || notification.title.len() > 160
            || notification.title.chars().any(char::is_control)
            || notification.body.len() > 4096
            || notification.body.split('\n').count() > 64
            || notification
                .body
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(api::Error::new(
                api::ErrorCode::InvalidArgument,
                "Invalid notification title or body",
            ));
        }
        let now = Instant::now();
        if instance
            .application
            .notification_at
            .is_some_and(|last| now.saturating_duration_since(last) < NOTIFICATION_INTERVAL)
        {
            return Err(api::Error::new(
                api::ErrorCode::Busy,
                "Notification publication is limited to two per second",
            ));
        }
        let source = format!("Plugin {}", instance.config.id);
        let severity = match notification.severity {
            handoff::Severity::Info => NotificationSeverity::Info,
            handoff::Severity::Warning => NotificationSeverity::Warning,
            handoff::Severity::Error => NotificationSeverity::Error,
        };
        self.app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application
            .notification_at = Some(now);
        self.app
            .push_background_notification(NotificationDraft::new(
                severity,
                source,
                notification.title,
                notification.body,
            ));
        self.app.plugins.presentation_dirty = true;
        Ok(api::ResultValue::Empty(api::Empty {}))
    }
}
