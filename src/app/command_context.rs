// SPDX-License-Identifier: MPL-2.0

//! What the active view supplies to commands that need a target.
//!
//! Command discovery hides a command whose target is absent, and an
//! explicitly typed one is refused with the reason given here. Each rule
//! mirrors the first refusal the command's own workflow makes, reading only
//! state the editor already holds: no filesystem, Git, provider or plugin
//! work happens per keystroke or redraw. A pane showing a terminal has no
//! document, whatever buffer it returns to when the terminal leaves.

use super::App;
use crate::{
    buffer::BufferKind,
    command::CommandContext,
    service_health::{CommandAvailability, ContextSnapshot},
};

/// Reason a document command gives in a pane showing a terminal.
const TERMINAL_PANE_REASON: &str = "the active pane is showing a terminal";

impl App {
    /// Captures every context once, for one projection of the palette.
    pub(super) fn command_contexts(&self) -> ContextSnapshot {
        ContextSnapshot::capture(|context| match self.context_supplied(context) {
            Ok(()) => CommandAvailability::Available,
            Err(reason) => CommandAvailability::Unavailable(reason),
        })
    }

    fn context_supplied(&self, context: CommandContext) -> Result<(), String> {
        use CommandContext as Context;
        if needs_document(context) && self.active_terminal().is_some() {
            return Err(TERMINAL_PANE_REASON.to_owned());
        }
        let buffer = self.active_buffer();
        let refuse = |reason: &str| Err(reason.to_owned());
        match context {
            Context::Document => Ok(()),
            Context::FileDirectory => {
                if buffer.is_directory() {
                    refuse("use :terminal-directory-root for a directory view")
                } else if buffer
                    .path
                    .as_deref()
                    .and_then(|path| path.parent())
                    .is_none()
                {
                    refuse("the active view has no file directory")
                } else {
                    Ok(())
                }
            }
            Context::Explorer => {
                if buffer.is_directory() {
                    Ok(())
                } else {
                    refuse("the active view is not a directory buffer")
                }
            }
            Context::ExplorerDirectoryEntry => {
                if !buffer.is_directory() {
                    return refuse("the active view is not a directory buffer");
                }
                let row = buffer.offset_to_row(self.active().head());
                // A symbolic link may name a directory; following it would
                // read the filesystem, so the workflow is left to decide.
                match buffer.directory_row_kind(row) {
                    Some(
                        crate::fs_plan::EntryKind::Directory | crate::fs_plan::EntryKind::Symlink,
                    ) => Ok(()),
                    Some(_) => refuse("the selected entry is not a directory"),
                    None => refuse("there is no directory entry on this row"),
                }
            }
            Context::TerminalSessions => {
                if self.terminals.is_empty() {
                    refuse("there are no terminal sessions")
                } else {
                    Ok(())
                }
            }
            Context::ActiveTerminal => {
                if self.active_terminal().is_some() {
                    Ok(())
                } else {
                    refuse("this pane is not showing a terminal")
                }
            }
            Context::TerminalOutput => {
                if self.terminal_output_target().is_some() {
                    Ok(())
                } else {
                    refuse("no terminal to copy output from")
                }
            }
            Context::TerminalSend => {
                if buffer.media_path.is_some() {
                    refuse("media has no text to send")
                } else if !self.terminals.any_live() {
                    refuse("no running terminal to send to")
                } else {
                    Ok(())
                }
            }
            Context::LocalFile => {
                if buffer.kind == BufferKind::File {
                    Ok(())
                } else {
                    refuse(":diff-disk requires an ordinary file buffer")
                }
            }
            Context::ProviderDocument => {
                if buffer.provider().is_some() {
                    Ok(())
                } else {
                    refuse(":diff-remote requires a provider document")
                }
            }
            Context::Comparison => {
                let pane = self.active_pane;
                let buffer = self.active().buffer;
                if self.pending_diff.is_some()
                    || self
                        .diffs
                        .iter()
                        .any(|diff| diff.has_pane(pane) || diff.has_buffer(buffer))
                {
                    Ok(())
                } else {
                    refuse("no comparison is open")
                }
            }
            Context::ComparableBuffer => {
                if buffer.is_directory() {
                    refuse("a directory listing cannot be compared")
                } else if buffer.media_path.is_some() {
                    refuse("media cannot be compared")
                } else {
                    Ok(())
                }
            }
            Context::GitFile
            | Context::GitDiffTarget
            | Context::GitFileChange
            | Context::GitUnstagedHunk
            | Context::GitStagedHunk
            | Context::GitSelectedLines
            | Context::GitComparisonRow
            | Context::GitBranchRow
            | Context::GitStashRow
            | Context::GitWork => self.git_target_context(context),
            Context::GitNetworkNextPage
            | Context::GitNetworkPreviousPage
            | Context::GitNetworkRefs => self.network_context(context),
            Context::PipeTarget => {
                if !cfg!(any(unix, windows)) {
                    refuse("shell pipes are unsupported on this platform")
                } else if self.pipe.cancellation.is_some() {
                    refuse("a pipe is already running; use :pipe-cancel")
                } else if buffer.is_read_only() {
                    refuse("pipe buffer is read-only")
                } else {
                    Ok(())
                }
            }
            Context::RunningPipe => {
                if self.pipe.cancellation.is_some() {
                    Ok(())
                } else {
                    refuse("no pipe is running")
                }
            }
            Context::FilePath => {
                if buffer.path.is_some() {
                    Ok(())
                } else {
                    refuse("buffer has no path")
                }
            }
            Context::Reloadable => match buffer.kind {
                BufferKind::File
                | BufferKind::Provider(_)
                | BufferKind::Directory
                | BufferKind::GitStatus
                | BufferKind::GitBranches
                | BufferKind::GitWorktrees
                | BufferKind::GitNetwork
                | BufferKind::GitLog
                | BufferKind::GitStash => Ok(()),
                _ => refuse("this view has no source to reload"),
            },
            Context::Markdown => {
                let active = self.active().buffer;
                match buffer.markdown_render_source() {
                    Some(source) if self.closed_buffers.contains(&source) => {
                        refuse("the rendered document is no longer open")
                    }
                    Some(_) => Ok(()),
                    None if self.is_markdown_document(active) => Ok(()),
                    None => refuse("not a Markdown document"),
                }
            }
            Context::Preview => {
                if !self.native_media {
                    refuse(crate::cli::edition::PREVIEW_WINDOW)
                } else if buffer.media_path.is_some() {
                    refuse(":preview requires a text buffer")
                } else {
                    Ok(())
                }
            }
            Context::Writable | Context::ForceWritable | Context::WritableInPlace => {
                if let Some(reason) = buffer.read_only_reason() {
                    refuse(reason)
                } else if context == Context::ForceWritable && buffer.provider().is_some() {
                    refuse("provider documents cannot be force-written")
                } else if context == Context::WritableInPlace
                    && buffer.path.is_none()
                    && buffer.provider().is_none()
                    && !buffer.is_commit_message()
                {
                    refuse("this buffer has no path; use :write <path>")
                } else {
                    Ok(())
                }
            }
            Context::ClosablePane => {
                if self.directory_tree.focused {
                    Ok(())
                } else if let Some(maximized) = self.maximized {
                    Err(format!(
                        "leave {} before closing the pane",
                        maximized.view.label()
                    ))
                } else if self.panes.len() == 1 {
                    refuse("this is the last pane")
                } else {
                    Ok(())
                }
            }
            Context::Fullscreen => {
                let leaving = self
                    .maximized
                    .is_some_and(|maximized| maximized.view == super::MaximizedView::Fullscreen);
                if leaving || self.panes.len() > 1 {
                    Ok(())
                } else {
                    refuse("only one pane")
                }
            }
            Context::PaneEdgeRight => self.pane_edge_context(1, 0).map_err(str::to_owned),
            Context::PaneEdgeLeft => self.pane_edge_context(-1, 0).map_err(str::to_owned),
            Context::PaneEdgeTop => self.pane_edge_context(0, -1).map_err(str::to_owned),
            Context::PaneEdgeBottom => self.pane_edge_context(0, 1).map_err(str::to_owned),
            Context::PersistentAttachment => {
                if self.persistent_session {
                    Ok(())
                } else {
                    refuse(":detach is available only in persistent mode")
                }
            }
            Context::QuitDirectoryHandoff => {
                if self.quit_directory_handoff {
                    Ok(())
                } else {
                    refuse(":qh requires the runyte() shell function from README.md")
                }
            }
            Context::PluginStop => {
                if self.plugins.manager_entries.iter().any(|entry| entry.valid) {
                    Ok(())
                } else {
                    refuse("no valid plugin is configured")
                }
            }
            Context::PluginRestart => {
                use crate::plugin::manager::Phase;
                if self.plugins.manager_entries.iter().any(|entry| {
                    entry.valid
                        && entry.enabled
                        && !matches!(entry.phase, Phase::Starting | Phase::RestartPending)
                }) {
                    Ok(())
                } else {
                    refuse("no enabled plugin can be restarted")
                }
            }
        }
    }
}

/// Whether `context` reads the pane's document, which a pane showing a
/// terminal does not have.
const fn needs_document(context: CommandContext) -> bool {
    use CommandContext as Context;
    !matches!(
        context,
        Context::TerminalSessions
            | Context::ActiveTerminal
            | Context::TerminalOutput
            | Context::Comparison
            | Context::GitNetworkRefs
            | Context::GitWork
            | Context::RunningPipe
            | Context::ClosablePane
            | Context::Fullscreen
            | Context::PaneEdgeRight
            | Context::PaneEdgeLeft
            | Context::PaneEdgeTop
            | Context::PaneEdgeBottom
            | Context::PersistentAttachment
            | Context::QuitDirectoryHandoff
            | Context::PluginStop
            | Context::PluginRestart
    )
}
