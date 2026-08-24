// SPDX-License-Identifier: MPL-2.0

//! Bounded, workspace-lifetime notifications and their searchable document.

use std::time::SystemTime;

use chrono::{DateTime, Local};

pub const DEFAULT_HISTORY_LIMIT: usize = 50;
pub const MIN_HISTORY_LIMIT: usize = 1;
pub const MAX_HISTORY_LIMIT: usize = 1_000;
/// Maximum retained payload for one notification, including source and title.
///
/// Individual producers should normally impose a tighter, domain-specific
/// bound. This is the final workspace-state backstop for producers that do not.
pub const MAX_NOTIFICATION_BYTES: usize = 1024 * 1024;
/// Maximum notification payload retained by one workspace, independent of the
/// configured entry-count limit.
pub const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;
pub const NOTIFICATIONS_BUFFER_NAME: &str = "[notifications]";

const FIELD_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NotificationId(u64);

impl NotificationId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NotificationSeverity {
    Error,
    Warning,
    Info,
}

impl NotificationSeverity {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warning => "WARNING",
            Self::Info => "INFO",
        }
    }

    pub const fn indicator(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warning => 'W',
            Self::Info => 'I',
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationDraft {
    pub severity: NotificationSeverity,
    pub source: String,
    pub title: String,
    pub body: String,
}

impl NotificationDraft {
    pub fn new(
        severity: NotificationSeverity,
        source: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            source: source.into(),
            title: title.into(),
            body: body.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notification {
    pub id: NotificationId,
    pub severity: NotificationSeverity,
    pub source: String,
    pub title: String,
    pub body: String,
    pub first_occurred: SystemTime,
    pub last_occurred: SystemTime,
    pub occurrences: usize,
    event_sequence: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NotificationCounts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

impl NotificationCounts {
    pub const fn total(self) -> usize {
        self.errors + self.warnings + self.infos
    }

    pub const fn highest(self) -> Option<NotificationSeverity> {
        if self.errors > 0 {
            Some(NotificationSeverity::Error)
        } else if self.warnings > 0 {
            Some(NotificationSeverity::Warning)
        } else if self.infos > 0 {
            Some(NotificationSeverity::Info)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct NotificationCenter {
    entries: Vec<Notification>,
    limit: usize,
    next_id: u64,
    event_sequence: u64,
    acknowledged_sequence: u64,
}

impl NotificationCenter {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit: limit.clamp(MIN_HISTORY_LIMIT, MAX_HISTORY_LIMIT),
            next_id: 1,
            event_sequence: 0,
            acknowledged_sequence: 0,
        }
    }

    pub fn entries(&self) -> &[Notification] {
        &self.entries
    }

    pub fn retained_bytes(&self) -> usize {
        self.entries.iter().map(Notification::retained_bytes).sum()
    }

    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.clamp(MIN_HISTORY_LIMIT, MAX_HISTORY_LIMIT);
        self.trim_to_limits();
    }

    pub fn push(&mut self, draft: NotificationDraft) -> NotificationId {
        self.push_at(draft, SystemTime::now())
    }

    pub fn push_at(&mut self, draft: NotificationDraft, now: SystemTime) -> NotificationId {
        let draft = bound_draft(draft);
        self.event_sequence = self.event_sequence.wrapping_add(1).max(1);
        if let Some(latest) = self.entries.first_mut()
            && latest.severity == draft.severity
            && latest.source == draft.source
            && latest.title == draft.title
            && latest.body == draft.body
        {
            latest.last_occurred = now;
            latest.occurrences = latest.occurrences.saturating_add(1);
            latest.event_sequence = self.event_sequence;
            return latest.id;
        }
        let id = NotificationId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.entries.insert(
            0,
            Notification {
                id,
                severity: draft.severity,
                source: draft.source,
                title: draft.title,
                body: draft.body,
                first_occurred: now,
                last_occurred: now,
                occurrences: 1,
                event_sequence: self.event_sequence,
            },
        );
        self.trim_to_limits();
        id
    }

    fn trim_to_limits(&mut self) {
        self.entries.truncate(self.limit);
        while self.entries.len() > 1 && self.retained_bytes() > MAX_HISTORY_BYTES {
            self.entries.pop();
        }
    }

    pub fn acknowledge(&mut self) {
        self.acknowledged_sequence = self.event_sequence;
    }

    pub fn unread_counts(&self) -> NotificationCounts {
        let mut counts = NotificationCounts::default();
        for notification in self
            .entries
            .iter()
            .filter(|entry| entry.event_sequence > self.acknowledged_sequence)
        {
            match notification.severity {
                NotificationSeverity::Error => counts.errors += 1,
                NotificationSeverity::Warning => counts.warnings += 1,
                NotificationSeverity::Info => counts.infos += 1,
            }
        }
        counts
    }

    pub fn render(&self) -> NotificationDocument {
        let mut text = String::new();
        let mut rows = Vec::new();
        text.push_str("Notifications · newest first\n");
        rows.push(NotificationRow::plain());
        text.push('\n');
        rows.push(NotificationRow::plain());
        if self.entries.is_empty() {
            text.push_str("No notifications.\n");
            rows.push(NotificationRow::plain());
            return NotificationDocument { text, rows };
        }
        for notification in &self.entries {
            let timestamp: DateTime<Local> = notification.last_occurred.into();
            let repeated = if notification.occurrences > 1 {
                format!(" · repeated {} times", notification.occurrences)
            } else {
                String::new()
            };
            text.push_str(&format!(
                "{} · {} · {} · {}{}\n",
                notification.severity.label(),
                timestamp.format("%Y-%m-%d %H:%M:%S"),
                sanitize_inline(&notification.source),
                sanitize_inline(&notification.title),
                repeated
            ));
            rows.push(NotificationRow {
                notification: Some(notification.id),
                severity: Some(notification.severity),
            });
            for line in sanitize_body(&notification.body).lines() {
                text.push_str(line);
                text.push('\n');
                rows.push(NotificationRow {
                    notification: Some(notification.id),
                    severity: None,
                });
            }
            text.push_str("────────────────────────────────────────\n\n");
            rows.push(NotificationRow::plain());
            rows.push(NotificationRow::plain());
        }
        NotificationDocument { text, rows }
    }
}

impl Notification {
    fn retained_bytes(&self) -> usize {
        self.source.len() + self.title.len() + self.body.len()
    }
}

impl Default for NotificationCenter {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_LIMIT)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationRow {
    pub notification: Option<NotificationId>,
    pub severity: Option<NotificationSeverity>,
}

impl NotificationRow {
    const fn plain() -> Self {
        Self {
            notification: None,
            severity: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationDocument {
    pub text: String,
    pub rows: Vec<NotificationRow>,
}

fn sanitize_inline(value: &str) -> String {
    sanitize(value).replace(['\n', '\r'], " ")
}

fn sanitize_body(value: &str) -> String {
    let sanitized = sanitize(value);
    if sanitized.is_empty() {
        "(no details)".to_owned()
    } else {
        sanitized
    }
}

fn sanitize(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\n' | '\t' => output.push(character),
            character if character.is_control() => {
                output.push_str(&format!("\\u{{{:x}}}", character as u32));
            }
            character => output.push(character),
        }
    }
    output
}

fn bound_draft(mut draft: NotificationDraft) -> NotificationDraft {
    draft.source = truncate_utf8(&draft.source, FIELD_BYTES, "notification source");
    draft.title = truncate_utf8(&draft.title, FIELD_BYTES, "notification title");
    let body_budget = MAX_NOTIFICATION_BYTES
        .saturating_sub(draft.source.len())
        .saturating_sub(draft.title.len());
    draft.body = truncate_utf8(&draft.body, body_budget, "notification details");
    draft
}

fn truncate_utf8(value: &str, limit: usize, label: &str) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let marker = format!("\n[Runyte truncated {label} after {limit} bytes]");
    if marker.len() >= limit {
        let mut end = limit.min(value.len());
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        return value[..end].to_owned();
    }
    let mut end = limit - marker.len();
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut bounded = value[..end].to_owned();
    bounded.push_str(&marker);
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(severity: NotificationSeverity, body: &str) -> NotificationDraft {
        NotificationDraft::new(severity, "Runyte", "Test", body)
    }

    #[test]
    fn history_is_newest_first_bounded_and_acknowledged() {
        let mut center = NotificationCenter::new(2);
        center.push(draft(NotificationSeverity::Info, "one"));
        center.push(draft(NotificationSeverity::Warning, "two"));
        center.push(draft(NotificationSeverity::Error, "three"));
        assert_eq!(center.entries.len(), 2);
        assert_eq!(center.entries[0].body, "three");
        assert_eq!(center.unread_counts().total(), 2);
        center.acknowledge();
        assert_eq!(center.unread_counts(), NotificationCounts::default());
    }

    #[test]
    fn consecutive_duplicates_coalesce_and_become_unread_again() {
        let mut center = NotificationCenter::default();
        let first = center.push(draft(NotificationSeverity::Error, "same"));
        center.acknowledge();
        let second = center.push(draft(NotificationSeverity::Error, "same"));
        assert_eq!(first, second);
        assert_eq!(center.entries[0].occurrences, 2);
        assert_eq!(center.unread_counts().errors, 1);
    }

    #[test]
    fn document_preserves_multiline_details_and_escapes_controls() {
        let mut center = NotificationCenter::default();
        center.push(draft(
            NotificationSeverity::Error,
            "first\nsecond\u{1b}[31m",
        ));
        let document = center.render();
        assert!(document.text.contains("first\nsecond\\u{1b}[31m"));
        assert_eq!(
            document
                .rows
                .iter()
                .filter(|row| row.severity.is_some())
                .count(),
            1
        );
    }

    #[test]
    fn entry_and_workspace_byte_budgets_are_explicit_and_unicode_safe() {
        let mut center = NotificationCenter::new(MAX_HISTORY_LIMIT);
        for index in 0..10 {
            center.push(NotificationDraft::new(
                NotificationSeverity::Error,
                "third-party",
                format!("failure {index}"),
                "界".repeat(MAX_NOTIFICATION_BYTES / 3 + 10),
            ));
        }

        assert!(center.retained_bytes() <= MAX_HISTORY_BYTES);
        assert!(center.entries().len() < 10);
        assert!(
            center.entries()[0]
                .body
                .is_char_boundary(center.entries()[0].body.len())
        );
        assert!(
            center.entries()[0]
                .body
                .contains("Runyte truncated notification details")
        );
    }
}
