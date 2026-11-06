// SPDX-License-Identifier: MPL-2.0

//! Startup and deliberate attachment feedback is owned by the editor, so all
//! frontends receive the same interaction line from its ordinary snapshot.

use super::{ActionFeedback, App, Mode};
use crate::{
    notification::{NotificationDraft, NotificationSeverity},
    project_root::{self, VersionControl, WorkspaceOrigin},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub(super) struct WorkspaceStartup {
    origin: Option<WorkspaceOrigin>,
    pending: bool,
    notice: Option<u64>,
    echo: Option<u64>,
    opened: bool,
}

impl WorkspaceStartup {
    pub(super) fn constructing() -> Self {
        Self {
            pending: true,
            ..Self::default()
        }
    }
}

impl App {
    /// Called before startup notices and service initialization. `origin` is
    /// the discovery result, never a second scan performed for presentation.
    pub(crate) fn note_workspace_origin(&mut self, origin: WorkspaceOrigin) {
        self.workspace_startup.origin = Some(origin);
        self.workspace_startup.pending = true;
        if matches!(origin, WorkspaceOrigin::Created(_)) {
            self.push_notification(NotificationDraft::new(
                NotificationSeverity::Info,
                "Workspace",
                "Created a workspace",
                format!(
                    "Created a workspace in {}; its state is in {}",
                    self.project_root.display(),
                    project_root::resolve_state_root(
                        &self.project_root,
                        &self.config.workspace.state
                    )
                    .display()
                ),
            ));
        }
    }

    /// A single ownership rule for all startup warnings and errors. Retained
    /// notifications still exist; their actionable text also wins the line.
    pub(super) fn note_startup_notification(&mut self, notice: &NotificationDraft) {
        let opening_feedback = self.action_feedback.as_ref().is_some_and(|feedback| {
            Some(feedback.id) == self.workspace_startup.echo
                || Some(feedback.id) == self.workspace_startup.notice
        });
        if (self.workspace_startup.pending || opening_feedback)
            && notice.severity != NotificationSeverity::Info
        {
            let id = self.next_action_id;
            self.next_action_id = self.next_action_id.wrapping_add(1).max(1);
            self.action_feedback = Some(ActionFeedback {
                id,
                spelling: String::new(),
                text: notice.body.clone(),
                is_error: notice.severity == NotificationSeverity::Error,
            });
            self.workspace_startup.notice = Some(id);
        }
    }

    /// A host started for this opening says started on its first attachment;
    /// a pre-existing foreground host is already running when a client arrives.
    pub(crate) fn note_persistent_launch(&mut self, started_for_open: bool) {
        self.workspace_startup.opened = !started_for_open;
    }

    /// Finish after synchronous service setup. Persistent hosts initially say
    /// started; deliberate frontend attachment adds its own opening echo.
    pub(crate) fn finish_workspace_startup(&mut self) {
        self.workspace_startup.pending = false;
        if self.action_feedback.is_none() {
            self.echo_workspace(false);
        }
    }

    /// Only deliberate launch/attach/switch handshakes call this. Reconnecting
    /// and resynchronizing transport do not claim the person's interaction line.
    pub(crate) fn note_workspace_opened(&mut self) {
        let attached = self.workspace_startup.opened;
        self.workspace_startup.opened = true;
        self.echo_workspace(attached);
    }

    fn echo_workspace(&mut self, attached: bool) {
        let Some(origin) = self.workspace_startup.origin else {
            return;
        };
        let notice_owns_line = self
            .action_feedback
            .as_ref()
            .is_some_and(|feedback| Some(feedback.id) == self.workspace_startup.notice);
        if self.is_editor_mode()
            || notice_owns_line
            || self.mode == Mode::Command
            || self.live_pending_display().is_some()
            || self.tree_interaction().is_some()
        {
            return;
        }
        let root = project_root::display_workspace_path(
            &self.project_root,
            self.home_directory.as_deref(),
        );
        let state = project_root::display_workspace_path(
            &project_root::resolve_state_root(&self.project_root, &self.config.workspace.state),
            self.home_directory.as_deref(),
        );
        let session = if self.persistent_session {
            if attached {
                ", attached to the running session"
            } else {
                ", started a session"
            }
        } else {
            ""
        };
        let configured_marker = shorten(&self.config.workspace.state.to_string_lossy(), 24);
        let how = match origin {
            WorkspaceOrigin::VersionControl(VersionControl::Git) => "Git root",
            WorkspaceOrigin::VersionControl(VersionControl::Jujutsu) => "Jujutsu root",
            WorkspaceOrigin::VersionControl(VersionControl::Subversion) => "Subversion root",
            WorkspaceOrigin::StateDirectory => &configured_marker,
            WorkspaceOrigin::Created(_) => "created, state in ",
        };
        // Reserve the explanation before shortening paths. Eighty columns is
        // the portable baseline; frontends retain their usual clipping below it.
        let available = 80usize.saturating_sub(10 + 3 + how.width() + session.width());
        let text = if matches!(origin, WorkspaceOrigin::Created(_)) {
            let root_budget = available / 2;
            format!(
                "Workspace {} ({how}{}){session}",
                shorten(&root, root_budget),
                shorten(&state, available - root_budget)
            )
        } else {
            format!("Workspace {} ({how}){session}", shorten(&root, available))
        };
        let id = self.next_action_id;
        self.next_action_id = self.next_action_id.wrapping_add(1).max(1);
        self.workspace_startup.echo = Some(id);
        self.action_feedback = Some(ActionFeedback {
            id,
            spelling: String::new(),
            text,
            is_error: false,
        });
    }
}

fn shorten(path: &str, budget: usize) -> String {
    if path.width() <= budget {
        return path.to_owned();
    }
    if budget == 0 {
        return String::new();
    }
    let mut width = 1;
    let tail = path
        .graphemes(true)
        .rev()
        .take_while(|cluster| {
            width += cluster.width();
            width <= budget
        })
        .collect::<Vec<_>>();
    let mut shortened = String::from("…");
    shortened.extend(tail.into_iter().rev());
    shortened
}
