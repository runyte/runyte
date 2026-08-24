// SPDX-License-Identifier: MPL-2.0

//! Small, test-oriented access to Runyte's semantic editor behavior.
//!
//! This facade is an evolving migration seam for direct editor tests. It is
//! deliberately not an RPC contract and does not expose the application
//! coordinator it currently delegates to. Callers get semantic commands,
//! transactions, text, and selections without constructing a frontend or
//! starting optional services.

use std::{error::Error, fmt, path::Path};

use anyhow::{Result, bail};

use crate::{
    app::{App, CommandOutcome, FrameGeometry, HostPorts},
    clipboard::SystemClipboard,
    command::{CommandExecutionContext, CommandInvocation, EditorCommand, GrammarKind},
    layout::Rect,
    selection::Selection,
    snapshot::EditorSnapshot,
    syntax::{Outline, SyntaxError, SyntaxEvent, SyntaxEvents, spawn_background},
    text::Transaction,
};

#[derive(Default)]
struct InertClipboard;

impl SystemClipboard for InertClipboard {
    fn read(&mut self) -> Result<String> {
        bail!("clipboard is unavailable in a headless editor")
    }

    fn write(&mut self, _text: &str) -> Result<()> {
        bail!("clipboard is unavailable in a headless editor")
    }
}

/// Why a transaction was rejected before it reached editor state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTransaction {
    change_index: usize,
    from: usize,
    to: usize,
    text_len: usize,
}

impl InvalidTransaction {
    pub const fn change_index(self) -> usize {
        self.change_index
    }

    pub const fn from(self) -> usize {
        self.from
    }

    pub const fn to(self) -> usize {
        self.to
    }

    pub const fn text_len(self) -> usize {
        self.text_len
    }
}

impl fmt::Display for InvalidTransaction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "transaction change {} has range {}..{} outside text length {}",
            self.change_index, self.from, self.to, self.text_len
        )
    }
}

impl Error for InvalidTransaction {}

/// A minimal editor facade for deterministic, frontend-independent tests.
///
/// The backing application is intentionally private: this type can shrink as
/// editor ownership moves out of `App` without making its coordination state
/// part of the public API.
pub struct HeadlessEditor {
    app: App,
}

impl HeadlessEditor {
    /// Creates an editor inside an existing caller-owned isolated directory.
    ///
    /// Construction does not discover a project, read user preferences or
    /// caches, create runtime state, or attach host services.
    pub fn new_in(root: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            app: App::new_in_isolated_project(root, HostPorts::isolated(Box::new(InertClipboard)))?,
        })
    }

    /// Creates a scratch editor seeded through the transactional edit path.
    pub fn with_text_in(root: impl AsRef<Path>, text: &str) -> Result<Self> {
        let mut editor = Self::new_in(root)?;
        if !text.is_empty() {
            editor.apply_transaction(Transaction::insert(0, text))?;
        }
        editor.set_active_selection(Selection::point(0));
        Ok(editor)
    }

    /// Executes a validated semantic invocation through the shared executor.
    pub fn execute(&mut self, invocation: CommandInvocation) -> Result<CommandOutcome> {
        self.app.execute(invocation)
    }

    /// Applies one transaction to the active buffer and maps every shared
    /// view through it.
    pub fn apply_transaction(
        &mut self,
        transaction: Transaction,
    ) -> std::result::Result<bool, InvalidTransaction> {
        let text_len = self.app.active_buffer().len_chars();
        if let Some((change_index, change)) =
            transaction
                .changes()
                .iter()
                .enumerate()
                .find(|(_, change)| {
                    change.from > change.to || change.from > text_len || change.to > text_len
                })
        {
            return Err(InvalidTransaction {
                change_index,
                from: change.from,
                to: change.to,
                text_len,
            });
        }
        Ok(self.app.edit(transaction))
    }

    /// Undoes one active-buffer transaction through the semantic command
    /// boundary.
    pub fn undo(&mut self) -> Result<CommandOutcome> {
        let invocation =
            CommandInvocation::editor(EditorCommand::Undo, CommandExecutionContext::default())?;
        self.execute(invocation)
    }

    /// Returns the active buffer's current text.
    pub fn active_text(&self) -> String {
        self.app.active_buffer().to_string()
    }

    /// Returns an owned snapshot of the active selection.
    pub fn active_selection(&self) -> Selection {
        self.app.active().selection.clone()
    }

    /// Prepares and captures one frontend-independent editor frame.
    ///
    /// This is primarily a characterization seam for host/service tests: a
    /// held background service must not prevent semantic edits or immutable
    /// snapshot generation. The dimensions include the editor surface only;
    /// callers that test terminal chrome continue to use the TUI boundary.
    pub fn snapshot(&mut self, width: u16, height: u16) -> EditorSnapshot {
        let screen = Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        let geometry = FrameGeometry {
            screen,
            editor: screen,
            status: Rect::default(),
            message: Rect::default(),
        };
        let prepared = self.app.prepare_view(geometry);
        self.app.snapshot(&prepared)
    }

    /// Returns the currently selected interactive input grammar.
    pub fn grammar_kind(&self) -> GrammarKind {
        self.app.grammar_kind()
    }

    /// Returns the active document's immediate Tree-sitter outline.
    ///
    /// `None` means the buffer has no syntax tree. Language-level unsupported
    /// and query failures remain typed [`SyntaxError`] values, while every
    /// returned item and jump target is Runyte-owned and revision-safe.
    pub fn active_outline(&self) -> std::result::Result<Option<Outline>, SyntaxError> {
        self.app.active_syntax_outline()
    }

    /// Replaces and normalizes the active selection against the current text.
    pub fn set_active_selection(&mut self, selection: Selection) {
        self.app.replace_active_selection(selection);
    }

    /// Opts this test editor into the production background syntax path.
    /// Must be called inside a Tokio runtime.
    pub fn enable_background_syntax(&mut self) -> SyntaxEvents {
        let (worker, events) = spawn_background(std::sync::Arc::clone(&self.app.registry));
        self.app.attach_syntax_worker(worker);
        events
    }

    pub fn has_pending_syntax(&self) -> bool {
        self.app.has_pending_syntax()
    }

    /// Applies one completed parse at the same explicit boundary as a live
    /// editor's event loop.
    pub fn apply_syntax_event(&mut self, event: SyntaxEvent) -> bool {
        self.app.apply_syntax_event(event)
    }
}
