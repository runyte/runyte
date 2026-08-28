// SPDX-License-Identifier: MPL-2.0

use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    ffi::{OsStr, OsString},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, bail, ensure};
use regex::{Regex, RegexBuilder};
use unicode_width::UnicodeWidthChar;

#[cfg(unix)]
use crate::workspace::{
    MAX_WORKSPACE_NUMBER, SessionPreview, WorkspaceEvent, WorkspaceRow, WorkspaceServiceHandle,
};

use crate::{
    buffer::{
        BinaryFileError, Buffer, BufferKind, FileObservation, FileObservationEvent,
        FileObservationRequest, GeneratedViewIdentity, ObservationApply, Position,
        WorkspaceSearchTarget,
    },
    clipboard::{CommandClipboard, SystemClipboard},
    command::{
        ArgumentKind, COMMANDS, ColonCommand, CommandArguments, CommandCategory,
        CommandExecutionContext, CommandId, CommandInvocation, CommandSpec, CommandUnavailable,
        EditorCommand, HelpInvocation, InvocationParameters, parse_colon_command, resolve_command,
    },
    config::{self, Config, Theme, ThemeAppearance, WorkspaceMode},
    content_alignment::{ContentAlignment, ContentLayout},
    diff::{Alignment, Side},
    diff_view::{DiffSession, DiffSide, MAX_DIFF_BYTES},
    directory_buffer::DirectoryTransfer,
    directory_listing::DirectoryListings,
    external_open::{self, ProgramCache},
    file_picker::{
        CONTENT_ENTRY_LIMIT, FilePicker, FilePickerEvent, FilePickerKind, FilePreview, FileScanner,
        PickerTarget, line_hits, scan_content, scan_files,
    },
    finder::{FinderMode, ResourceFinder, ResourceItem, ResourceKind, ResourceTarget},
    fs_plan::{ApplyReport, DeletionMode, EntryKind, FsOperation, FsPlan, TransferMode},
    git::{
        BlameLine, BlameRequest, BlameSource, Branch, BranchDeletionPlan, BufferRevisionGuard,
        CommitDetail, CommitSearchResult, CommitSummary, DeletionAuthorization, DiffScope,
        FileComparison, GitMutation, GitOperation, GitProvider, GitRequestId, GitResponse,
        GitServiceEvent, GitServiceHandle, GitServiceProgress, GitServiceState, GitTracker,
        LineChange, LogCursor, LogPage, LogRequest, MAX_BLAME_INPUT_BYTES, MAX_BLAME_LINES,
        PartialStageSelection, PatchHunk, RefreshSpec, Repository, RepositoryGeneration,
        RepositorySnapshot, StashEntry, StashMutation, StashScope, StatusSide, Worktree,
        WorktreeCreate, WorktreeRemovalPlan,
    },
    help::HelpTopic,
    input::{
        InputEvent, KeyCode, KeyStroke, Modifiers, PointerButton, PointerEvent, PointerEventKind,
    },
    input_grammar::{
        ActiveGrammar, EditorIntent, GrammarContext, GrammarNotice, GrammarOutput, InputGrammar,
        LineDirection, RangeIntent, VimMotion, VimOperator, VimRangeTarget, VimTextObject,
    },
    jump_labels::{JumpLabels, Press},
    jumplist::{Jump, JumpList, SelectionSemantics},
    keymap::{BindingScope, BindingTarget, ContextAction, KeySequence, Keymap, keymap_for},
    launch::{LaunchPosition, LaunchTarget},
    layout::{Axis, Layout, Rect},
    lsp::{
        ActionEntry, Capabilities, ChangeSync, Completion, DiagnosticStore, DocumentEdit,
        DocumentSync, Encoding, LspCommand, LspEvent, LspHandle, LspRange, RequestKind, Response,
        SignatureContext, SignatureLine, TextDocumentContentChangeEvent, checked_lsp_range,
        from_lsp_position, from_lsp_range, to_lsp_position,
    },
    notification::{
        NotificationCenter, NotificationCounts, NotificationDraft, NotificationSeverity,
    },
    picker::{ListPicker, ListPurpose, PickerItem},
    project_root,
    selection::{Range, Selection},
    service_health::{
        AppCapabilitySnapshot, CommandAvailability, ServiceHealthEntry, ServiceHealthSnapshot,
        ServiceState, persistent_session_availability,
    },
    settings::{
        PreviewPolicy, SettingId, SettingType, SettingValue, persist_setting, render_settings_page,
    },
    startup::{StartupPhase, StartupTrace},
    structural_selection::{
        ExpansionHistory, HistoryReset, ShrinkResult, navigate_text_object, select_delimiter,
        select_text_object, transform_selection,
    },
    syntax::{
        DelimiterPair, DocumentSyntax, LanguageId, Outline, OutlineItem, OutlineKind, Registry,
        RegistryError, Scope, Span, StaleSyntax, SyntaxError, SyntaxEvent, SyntaxFoldRange,
        SyntaxHandle, SyntaxObject, SyntaxObjectPart, SyntaxSelectionRange,
        SyntaxSelectionTransform,
    },
    terminal::{
        DefaultColors, SentTextUndo, TerminalId, TerminalOutput, TerminalRequest, TerminalSession,
        TerminalSessions,
    },
    text::{Assoc, Change, Offset, Text, Transaction},
    tutorial::{MotionHints, TutorialState},
    word_index::WordIndexHandle,
};

pub use crate::command::Mode;

#[derive(Clone, Debug)]
struct ReplaceStep {
    before: Selection,
    after: Selection,
    inverse: Transaction,
}

#[derive(Clone, Debug)]
struct ReplaceSession {
    buffer: usize,
    steps: Vec<ReplaceStep>,
}

// Application behavior is grouped by editor-level workflow below. These
// modules coordinate the lower-level owners imported above; they do not take
// over Git processes, LSP transport, terminal emulation, buffer mutation, or
// snapshot rendering from those existing boundaries.
mod completion_support;
mod editing;
mod file_workflows;
mod git_workflows;
mod input;
mod language_workflows;
mod movement;
mod picker_workflows;
mod presentation;
mod prompt_editing;
mod search_history;
mod settings_workflows;
mod syntax_workflows;
mod terminal_workflows;
mod tutorial_workflows;
mod workspace_workflows;

use completion_support::*;
use git_workflows::GitWorkflowState;
use movement::*;
use prompt_editing::*;

/// Frontend-independent regions allocated for one complete frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameGeometry {
    pub screen: Rect,
    pub editor: Rect,
    /// Global status line below the editor area.
    pub status: Rect,
    /// Interaction line reserved for prompts and action echo.
    pub message: Rect,
}

/// Immutable geometry and viewport values for one prepared pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedPane {
    pub pane_id: usize,
    pub area: Rect,
    pub body: Rect,
    pub buffer_id: usize,
    /// Set when this pane shows a live terminal instead of its buffer.
    ///
    /// The buffer id stays alongside it: it names the document the pane goes
    /// back to, not anything the terminal is drawn from.
    pub terminal: Option<TerminalId>,
    pub drawable: bool,
    pub body_width: usize,
    pub body_height: usize,
    pub line_digits: usize,
    pub signs: bool,
    /// Whether this pane reserves a column for Git change marks.
    pub changes: bool,
    pub gutter_width: usize,
    /// Cells left for text after the gutter and any content indent.
    pub text_width: usize,
    /// Blank cells between the gutter and the text of every row, held open by
    /// this buffer's [content alignment](crate::content_alignment).
    ///
    /// Zero for everything but a generated page that asked to be centred. It
    /// is presentation only: no offset in the buffer moves, so a column means
    /// what it says and a frontend translates a pointer through this one
    /// value.
    pub content_indent: usize,
    pub scroll_row: usize,
    pub scroll_wrap: usize,
    pub scroll_col: usize,
    pub wrap_width: usize,
    /// The central fold- and wrap-aware row projection consumed by snapshots
    /// and frontends. No renderer is allowed to derive a second viewport.
    pub rows: Vec<PreparedRow>,
}

/// One screen row in a prepared pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedRow {
    /// The line this row shows, or `None` for a diff pane's filler: a row
    /// held open so the line facing it on the other side stays level, which
    /// belongs to no line of this buffer and can be clicked, labelled, or
    /// moved to no more than the blank area below the last line can.
    pub document_row: Option<usize>,
    pub segment: Option<crate::wrap::Segment>,
    pub continuation: bool,
    /// Whether this logical row anchors a pane-local collapsed syntax region.
    pub folded: bool,
    /// Number of complete document lines hidden after this row.
    pub folded_lines: Option<usize>,
    /// Whether this row is blank space held open by content alignment.
    ///
    /// Padding and a diff's filler are both rows belonging to no line, so
    /// neither can be clicked, labelled, or moved to. They differ in what
    /// they say: filler stands for lines the other side of a comparison has,
    /// and padding stands for nothing at all, which is why a frontend draws
    /// one hatched and the other blank.
    pub padding: bool,
}

impl PreparedRow {
    /// A row of blank space above or below vertically centred content.
    fn padding() -> Self {
        Self {
            document_row: None,
            segment: None,
            continuation: false,
            folded: false,
            folded_lines: None,
            padding: true,
        }
    }
}

/// Owned result of preparing all pane viewports for one frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedView {
    pub geometry: FrameGeometry,
    pub panes: Vec<PreparedPane>,
}

impl PreparedView {
    pub fn pane(&self, pane_id: usize) -> Option<&PreparedPane> {
        self.panes.iter().find(|pane| pane.pane_id == pane_id)
    }
}

/// A pane temporarily presented across the complete editor area, and the way
/// its content is laid out while it is.
///
/// The ordinary layout and its panes stay intact underneath, so toggling the
/// view off restores them exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaximizedPane {
    pub pane: usize,
    pub view: MaximizedView,
}

/// Which of the two maximized presentations a pane is showing.
///
/// They differ only in the content layout the pane's text is given: the
/// geometry each one takes is the same complete editor area.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaximizedView {
    /// `:zen`: the text is centred in a viewport `editor.zen_width` wide.
    Zen,
    /// `:fullscreen`: the pane keeps the content layout it has in a split, so
    /// the only change is how much room it has.
    Fullscreen,
}

impl MaximizedView {
    /// Names the view as a status message refers to it.
    fn label(self) -> &'static str {
        match self {
            Self::Zen => "zen mode",
            Self::Fullscreen => "the full-screen view",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pane {
    pub buffer: usize,
    /// Buffers previously displayed by this pane, oldest to newest.
    ///
    /// Buffer fallback is view-local: closing a shared buffer can reveal a
    /// different predecessor in each pane without coupling it to jumps.
    buffer_history: Vec<usize>,
    /// The live terminal this pane shows instead of its buffer, if any.
    ///
    /// A terminal is not a document — no rope, no undo, no disk state — so it
    /// is a second thing a pane can contain rather than a twenty-third
    /// [`BufferKind`](crate::buffer::BufferKind). `buffer` keeps its ordinary
    /// meaning while this is set: it is the document the pane returns to when
    /// the terminal ends or is switched away from.
    pub terminal: Option<TerminalId>,
    pub selection: Selection,
    /// The model used by the operation that most recently produced this
    /// selection. This is provenance, not a coordinate witness: two different
    /// operations can intentionally produce equal ranges with different
    /// semantics.
    selection_semantics: SelectionSemantics,
    /// Bumped only when an operation semantically produces a selection.
    /// Coordinate mapping through a text transaction deliberately does not
    /// bump this revision.
    selection_revision: u64,
    pub scroll_row: usize,
    /// Wrapped sub-row within `scroll_row`; always zero when wrapping is off.
    pub scroll_wrap: usize,
    pub scroll_col: usize,
    /// Last rendered text width, used so visual-line motions and rendering
    /// share the same wrapping boundary.
    pub wrap_width: usize,
    pub preserve_scroll: bool,
    /// Where this pane has been. Per-pane rather than global because a jump is
    /// a property of a view: two panes looking at the same buffer have
    /// genuinely different histories.
    pub jumps: JumpList,
    /// Revision-tagged structural expansions for this view only.
    syntax_history: ExpansionHistory,
    /// Collapsed syntax ranges belong to this view, not to the shared buffer.
    folds: FoldState,
    /// The one directory buffer this pane browses with, retargeted as it
    /// walks the tree.
    ///
    /// Per-pane rather than global so two panes can still show two different
    /// directories, and singular so walking a deep tree leaves one explorer in
    /// the buffer list instead of one entry per directory visited.
    pub directory_buffer: Option<usize>,
    /// Most recent explorer directory this pane displayed, retained after the
    /// ephemeral explorer buffer itself is retired for `:quit-here`.
    last_explorer_directory: Option<PathBuf>,
}

impl Pane {
    fn new(buffer: usize) -> Self {
        Self {
            buffer,
            buffer_history: Vec::new(),
            terminal: None,
            selection: Selection::point(0),
            selection_semantics: SelectionSemantics::Runyte,
            selection_revision: 0,
            scroll_row: 0,
            scroll_wrap: 0,
            scroll_col: 0,
            wrap_width: 1,
            preserve_scroll: false,
            jumps: JumpList::default(),
            syntax_history: ExpansionHistory::default(),
            folds: FoldState::default(),
            directory_buffer: None,
            last_explorer_directory: None,
        }
    }

    fn retarget(&mut self, buffer: usize) {
        // Even retargeting to the buffer already named leaves the terminal:
        // asking for a document is asking to stop looking at a terminal.
        self.terminal = None;
        if self.buffer != buffer {
            self.remember_buffer(self.buffer);
            self.buffer = buffer;
            self.selection_semantics = SelectionSemantics::Runyte;
            self.selection_revision = self.selection_revision.wrapping_add(1);
            self.syntax_history.clear();
            self.folds.clear();
        }
    }

    fn remember_buffer(&mut self, buffer: usize) {
        self.buffer_history.retain(|previous| *previous != buffer);
        self.buffer_history.push(buffer);
    }

    /// Replaces a retired backing buffer without exposing a covered terminal
    /// or adding the retired identity back to history.
    fn replace_closed_buffer(&mut self, buffer: usize) {
        self.buffer = buffer;
        self.selection_semantics = SelectionSemantics::Runyte;
        self.selection_revision = self.selection_revision.wrapping_add(1);
        self.syntax_history.clear();
        self.folds.clear();
    }

    /// Offset of the primary caret.
    pub fn head(&self) -> Offset {
        self.selection.primary().head
    }

    pub(crate) fn selection_semantics(&self) -> SelectionSemantics {
        self.selection_semantics
    }

    fn mark_selection_semantics(&mut self, semantics: SelectionSemantics) {
        self.selection_semantics = semantics;
    }

    fn replace_selection(&mut self, selection: Selection) {
        self.selection = selection;
        self.selection_revision = self.selection_revision.wrapping_add(1);
    }

    /// View coordinate of the primary caret.
    pub fn cursor(&self, buffer: &Buffer) -> Position {
        buffer.position_of(self.head())
    }
}

/// Where a pane would show a directory, before it has committed to showing it.
enum PaneDirectory {
    /// A buffer the editor already holds.
    Existing(usize),
    /// A listing read for this navigation, not yet part of the editor. Boxed
    /// because a buffer dwarfs the index it is the alternative to.
    New(Box<Buffer>),
}

#[derive(Clone, Debug, Default)]
struct FoldState {
    collapsed: Vec<SyntaxFoldRange>,
}

impl FoldState {
    fn clear(&mut self) {
        self.collapsed.clear();
    }
}

#[derive(Clone, Copy, Debug)]
struct ResolvedFold {
    source: SyntaxFoldRange,
    anchor_row: usize,
    first_hidden_row: usize,
    end_hidden_row: usize,
}

impl ResolvedFold {
    fn hidden_lines(self) -> usize {
        self.end_hidden_row.saturating_sub(self.first_hidden_row)
    }

    fn hides(self, row: usize) -> bool {
        row >= self.first_hidden_row && row < self.end_hidden_row
    }
}

fn adjust_scroll(pane: &mut Pane, cursor: Position, height: usize, width: usize, offset: usize) {
    if height == 0 {
        return;
    }
    if !pane.preserve_scroll {
        if cursor.row < pane.scroll_row + offset {
            pane.scroll_row = cursor.row.saturating_sub(offset);
        } else if cursor.row >= pane.scroll_row + height.saturating_sub(offset.min(height / 2)) {
            pane.scroll_row = cursor.row + offset + 1 - height;
        }
    }
    if width == 0 {
        return;
    }
    if cursor.col < pane.scroll_col {
        pane.scroll_col = cursor.col;
    } else if cursor.col >= pane.scroll_col + width {
        pane.scroll_col = cursor.col + 1 - width;
    }
}

fn adjust_scroll_wrapped(
    pane: &mut Pane,
    buffer: &Buffer,
    cursor: Position,
    height: usize,
    width: usize,
    offset: usize,
    tab_width: usize,
) {
    if height == 0 || width == 0 {
        return;
    }
    pane.scroll_row = pane.scroll_row.min(buffer.last_row());
    let start_count =
        crate::wrap::segments(&buffer.line_string(pane.scroll_row), width, tab_width).len();
    pane.scroll_wrap = pane.scroll_wrap.min(start_count.saturating_sub(1));
    pane.scroll_col = 0;
    if pane.preserve_scroll {
        return;
    }

    let cursor_segment = crate::wrap::segment_index(
        &buffer.line_string(cursor.row),
        cursor.col,
        width,
        tab_width,
    );
    let distance = crate::wrap::visual_distance(
        buffer,
        pane.scroll_row,
        pane.scroll_wrap,
        cursor.row,
        cursor_segment,
        width,
        tab_width,
        height,
    );
    let top_margin = offset.min(height / 2);
    let bottom = height.saturating_sub(offset.min(height / 2) + 1);
    let needs_top = distance.is_none_or(|distance| distance < top_margin);
    let needs_bottom = distance.is_some_and(|distance| distance > bottom);
    if needs_top || needs_bottom {
        let amount = if needs_top { top_margin } else { bottom };
        (pane.scroll_row, pane.scroll_wrap) = crate::wrap::move_visual_start_backward(
            buffer,
            cursor.row,
            cursor_segment,
            amount,
            width,
            tab_width,
        );
    }
}

fn fold_hiding_row(folds: &[ResolvedFold], row: usize) -> Option<ResolvedFold> {
    folds
        .iter()
        .copied()
        .filter(|fold| fold.hides(row))
        .min_by_key(|fold| (fold.anchor_row, std::cmp::Reverse(fold.end_hidden_row)))
}

fn previous_visible_row(folds: &[ResolvedFold], row: usize) -> usize {
    let mut candidate = row.saturating_sub(1);
    loop {
        match fold_hiding_row(folds, candidate) {
            Some(fold) if candidate > 0 => candidate = fold.anchor_row,
            Some(_) => return 0,
            None => return candidate,
        }
    }
}

fn next_visible_row(folds: &[ResolvedFold], row: usize, last_row: usize) -> usize {
    let origin = row;
    let mut candidate = row.saturating_add(1).min(last_row);
    while let Some(fold) = fold_hiding_row(folds, candidate) {
        if fold.end_hidden_row > last_row {
            return origin;
        }
        candidate = fold.end_hidden_row;
    }
    candidate
}

/// One pane's side of a live diff, as the row projection needs it.
///
/// The two panes of a session are handed the same `start`, and that is the
/// entirety of how they stay level with each other: each one independently
/// projects the same stretch of the aligned row space, showing its own lines
/// where it has them and filler where it does not.
#[derive(Clone, Copy)]
struct DiffProjection<'a> {
    alignment: &'a Alignment,
    side: Side,
    /// Where this viewport starts in the aligned row space.
    start: usize,
}

/// The aligned-space projection for a pane, borrowed from the sessions alone.
///
/// Taking the sessions rather than the whole editor is what lets a caller hold
/// this across a mutable borrow of the panes it is about to project.
fn diff_projection(diffs: &[DiffSession], pane_id: usize) -> Option<DiffProjection<'_>> {
    let session = diffs.iter().find(|session| session.has_pane(pane_id))?;
    Some(DiffProjection {
        alignment: session.alignment(),
        side: session.side_of_pane(pane_id)?,
        start: session.aligned_start(),
    })
}

/// One deterministic projection powers viewport preparation, snapshots,
/// motions, scrolling, jump labels, and future pointer hit-testing.
#[allow(clippy::too_many_arguments)]
fn project_visible_rows(
    buffer: &Buffer,
    folds: &[ResolvedFold],
    start_row: usize,
    start_segment: usize,
    height: usize,
    width: usize,
    tab_width: usize,
    soft_wrap: bool,
    diff: Option<DiffProjection<'_>>,
) -> Vec<PreparedRow> {
    // A diff pane walks the aligned row space instead of the document's own
    // rows, because that is the space in which the two sides correspond. It
    // is still this function that produces the rows: there is one projection,
    // with two ways of choosing which row comes next.
    if let Some(diff) = diff {
        return project_aligned_rows(buffer, diff, height);
    }
    let mut rows = Vec::with_capacity(height);
    let mut row = start_row.min(buffer.last_row());
    if let Some(fold) = fold_hiding_row(folds, row) {
        row = fold.anchor_row;
    }
    let mut initial_segment = start_segment;
    while rows.len() < height && row < buffer.len_lines() {
        if let Some(fold) = fold_hiding_row(folds, row) {
            row = fold.end_hidden_row;
            initial_segment = 0;
            continue;
        }
        let folded_lines = folds
            .iter()
            .filter(|fold| fold.anchor_row == row)
            .map(|fold| fold.hidden_lines())
            .max()
            .filter(|lines| *lines > 0);
        if soft_wrap {
            let spans = crate::wrap::segments(&buffer.line_string(row), width, tab_width);
            for (segment, span) in spans.iter().copied().enumerate().skip(initial_segment) {
                if rows.len() == height {
                    break;
                }
                rows.push(PreparedRow {
                    document_row: Some(row),
                    segment: Some(span),
                    continuation: segment > 0,
                    folded: folded_lines.is_some(),
                    folded_lines: (segment + 1 == spans.len())
                        .then_some(folded_lines)
                        .flatten(),
                    padding: false,
                });
            }
        } else {
            rows.push(PreparedRow {
                document_row: Some(row),
                segment: None,
                continuation: false,
                folded: folded_lines.is_some(),
                folded_lines,
                padding: false,
            });
        }
        let next = next_visible_row(folds, row, buffer.last_row());
        if next == row {
            break;
        }
        row = next;
        initial_segment = 0;
    }
    rows
}

/// The rows one side of a diff shows, walking the aligned row space.
///
/// Wrapping is off in a diff pane and folds are cleared when a session opens,
/// so every aligned row is exactly one screen row: either a line this side
/// has, or filler standing in for lines only the other side has.
fn project_aligned_rows(
    buffer: &Buffer,
    diff: DiffProjection<'_>,
    height: usize,
) -> Vec<PreparedRow> {
    let mut rows = Vec::with_capacity(height);
    for aligned in diff.start.. {
        if rows.len() == height {
            break;
        }
        match diff.alignment.row_at(diff.side, aligned) {
            // Past the last line there is nothing left to show, and the rows
            // below are the frontend's own blank area rather than filler.
            Some(row) if row >= buffer.len_lines() => break,
            Some(row) => rows.push(PreparedRow {
                document_row: Some(row),
                segment: None,
                continuation: false,
                folded: false,
                folded_lines: None,
                padding: false,
            }),
            None => rows.push(PreparedRow {
                document_row: None,
                segment: None,
                continuation: false,
                folded: false,
                folded_lines: None,
                padding: false,
            }),
        }
    }
    rows
}

#[allow(clippy::too_many_arguments)]
fn move_projected_start_backward(
    buffer: &Buffer,
    folds: &[ResolvedFold],
    mut row: usize,
    mut segment: usize,
    mut amount: usize,
    width: usize,
    tab_width: usize,
    soft_wrap: bool,
) -> (usize, usize) {
    while amount > 0 {
        if soft_wrap && segment > 0 {
            segment -= 1;
        } else if row > 0 {
            let previous = previous_visible_row(folds, row);
            if previous == row {
                break;
            }
            row = previous;
            segment = if soft_wrap {
                crate::wrap::segments(&buffer.line_string(row), width, tab_width)
                    .len()
                    .saturating_sub(1)
            } else {
                0
            };
        } else {
            break;
        }
        amount -= 1;
    }
    (row, segment)
}

#[derive(Clone, Debug)]
struct DirectoryView {
    selection: Selection,
    scroll_row: usize,
    scroll_wrap: usize,
    scroll_col: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectoryReloadConfirmation {
    buffer: usize,
    destination: Option<PathBuf>,
    /// Entry to select after navigation completes. Parent navigation carries
    /// the directory being left through the confirmation so an accepted
    /// discard behaves exactly like an immediate `-`.
    focus_entry: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileReloadConfirmation {
    buffer: usize,
    path: PathBuf,
    generation: u64,
    observation: FileObservation,
}

impl FileReloadConfirmation {
    fn message(&self, stale: bool) -> String {
        let mut message = format!(
            "Reload {} and discard unsaved Runyte changes and their undo history?",
            self.path.display()
        );
        if stale {
            message.push_str("\nSpace b d compares without discarding changes.");
        }
        message.push_str("\nEnter confirms.\nEscape cancels.");
        message
    }
}

impl DirectoryReloadConfirmation {
    fn message(&self) -> String {
        match self.destination.as_deref() {
            Some(path) => format!(
                "Discard unsaved directory edits and open {}?\nEnter confirms.\nEscape cancels.",
                path.display()
            ),
            None => {
                "Discard unsaved directory edits and refresh?\nEnter confirms.\nEscape cancels."
                    .to_owned()
            }
        }
    }
}

impl DirectoryView {
    fn from_pane(pane: &Pane) -> Self {
        Self {
            selection: pane.selection.clone(),
            scroll_row: pane.scroll_row,
            scroll_wrap: pane.scroll_wrap,
            scroll_col: pane.scroll_col,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    FileStart,
    FileEnd,
    WordForward,
    WordBack,
    WordEnd,
    WordEndBack,
    LongWordForward,
    LongWordBack,
    LongWordEnd,
    LongWordEndBack,
    NextParagraph,
    PreviousParagraph,
    FirstNonWhitespace,
    LastNonWhitespace,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    WindowTop,
    WindowCenter,
    WindowBottom,
}

#[derive(Clone, Copy, Debug)]
enum ViewAlignment {
    Top,
    Center,
    Bottom,
}

/// How a search pattern is turned into matches.
///
/// The two literal flavours escape the pattern before compiling it, which is
/// what lets someone search for `foo(` or `a.b` without knowing that a regular
/// expression is involved at all.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchMode {
    #[default]
    Insensitive,
    Sensitive,
    Regex,
}

impl SearchMode {
    /// The parenthetical a prompt label uses to name this flavour.
    const fn qualifier(self) -> &'static str {
        match self {
            Self::Insensitive => "",
            Self::Sensitive => " (case-sensitive)",
            Self::Regex => " (regex)",
        }
    }

    fn compile(self, pattern: &str) -> Result<Regex, regex::Error> {
        match self {
            Self::Insensitive => RegexBuilder::new(&regex::escape(pattern))
                .case_insensitive(true)
                .build(),
            Self::Sensitive => Regex::new(&regex::escape(pattern)),
            Self::Regex => Regex::new(pattern),
        }
    }
}

/// The committed search, and the region `n`/`N` are allowed to walk.
#[derive(Clone, Debug, Default)]
struct SearchQuery {
    pattern: String,
    mode: SearchMode,
    /// The buffer and spans the search was scoped to. `None` searches all of
    /// the active buffer.
    ///
    /// Scoping is what keeps a search that started inside a selection from
    /// wandering out of it when `n` wraps.
    region: Option<SearchRegion>,
    /// Only the Vim grammar reads this: its `n`/`N` repeat the direction the
    /// search was started in, where Runyte's always step forward and backward.
    forward: bool,
}

#[derive(Clone, Debug)]
struct SearchRegion {
    buffer: usize,
    spans: Vec<Range>,
}

/// Exact pane selection installed by the most recent Runyte search action.
/// A semantic selection revision makes this presentation self-invalidating:
/// motions and selection commands replace the ranges and therefore stop being
/// rendered as pristine search results, while transaction coordinate mapping
/// deliberately preserves it.
#[derive(Clone, Copy, Debug)]
struct SearchSelectionPresentation {
    pane: usize,
    revision: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PromptKind {
    #[default]
    Command,
    /// Runyte's three search flavours, which select every match at once.
    Search(SearchMode),
    /// Search over an immutable retained terminal snapshot rather than the
    /// buffer hidden behind the pane.
    TerminalSearch(SearchMode),
    /// The Vim grammar's directional single-match searches.
    SearchForward,
    SearchBackward,
    GlobalSearch(SearchMode),
    /// Keeps or drops selections by regular expression. `keep` distinguishes
    /// `Alt-k` from `Alt-j`; the editing model is otherwise identical.
    FilterSelections {
        keep: bool,
    },
    /// Collects the new name for `textDocument/rename`. Reuses the search
    /// prompt's editing model rather than adding a third text-entry surface.
    Rename,
    /// Collects a new persistent-session name for the chosen workspace.
    SessionRename,
    SessionNumber,
    /// Selects the program a binary file should be handed to. Reuses the same
    /// editing model, with the default and recently chosen programs above it.
    ExternalProgram,
    /// Collects the name of a branch to create at the selected one.
    NewBranch,
    NewWorktreeBranch,
    WorktreeDestination,
    /// Collects the literal text every line break inside the selection is
    /// replaced with. Empty is a legal answer and joins the lines directly,
    /// which is why this prompt cannot reuse the "pattern is empty" refusal the
    /// search prompts share.
    JoinDelimiter,
    /// Collects a non-enumerated setting value in a popup.
    SettingValue(SettingId),
}

#[derive(Clone, Debug)]
struct GeneralWorktreeRow {
    worktree: Worktree,
}

/// A persistent session standing on something a deletion is about to remove.
///
/// A session is meaningless without the worktree it was opened on, and a
/// worktree is meaningless without its branch, so removing either takes the
/// session with it. This is what the confirmation names so the compound action
/// is visible before it is accepted, rather than discovered afterwards as a
/// host left running on a directory that is no longer there.
#[derive(Clone, Debug, Eq, PartialEq)]
struct AttachedSession {
    name: String,
    number: Option<u8>,
    /// The project root as the catalog holds it, resolved while the directory
    /// still exists. The record has to be reachable after the directory is
    /// gone, and a path can no longer be canonicalized then.
    root: PathBuf,
}

impl AttachedSession {
    /// How the session is named in a confirmation: by its digit when it has
    /// one, because that is the name the reader presses.
    fn describe(&self) -> String {
        match self.number {
            Some(number) => format!("session {number} ({})", self.name),
            None => format!("session {}", self.name),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorktreeRemovalConfirmation {
    plan: WorktreeRemovalPlan,
    /// The running session on this worktree, when there is one.
    session: Option<AttachedSession>,
    input: String,
    cursor: usize,
}

/// How far a compound worktree removal has got.
///
/// The host owns the worktree directory and the runtime state inside it, so it
/// has to be down before Git is asked to remove that directory. Each stage
/// therefore waits for the previous one's event rather than being issued
/// together with it.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorktreeTeardownStage {
    /// Waiting for the session to stop. Nothing on disk has changed yet, so a
    /// failure here leaves the worktree exactly as it was.
    Stopping,
    /// Waiting for Git to report that the directory is gone. With the Git
    /// service attached the removal is queued rather than performed, and its
    /// guarded re-check can still refuse; nothing below this level may happen
    /// until it has actually succeeded.
    Removing,
    /// The worktree is gone; only its history record is still to follow.
    Forgetting,
    /// The session record and worktree are gone; the exact final branch
    /// mutation still owns this cascade until Git reports its outcome.
    BranchDeleting,
}

#[cfg(unix)]
impl WorktreeTeardown {
    /// Whether a stop result for `selector` belongs to this teardown. The
    /// catalog answers with the selector it was given, which is the resolved
    /// root the teardown asked to stop.
    fn awaits_stop(&self, generation: u64, selector: &Path) -> bool {
        self.stage == WorktreeTeardownStage::Stopping
            && self.workspace_request_generation == Some(generation)
            && self.root == selector
    }

    /// Whether a finished `git worktree remove` is this teardown's own. The
    /// mutation is matched by the path it removed, which is the plan's, not the
    /// resolved catalog root the stop and forget use.
    fn awaits_removal(&self, request: Option<GitRequestId>, path: &Path) -> bool {
        self.stage == WorktreeTeardownStage::Removing
            && self.git_request == request
            && self.plan.path == path
    }

    fn awaits_forget(&self, generation: u64, path: &Path) -> bool {
        self.stage == WorktreeTeardownStage::Forgetting
            && self.workspace_request_generation == Some(generation)
            && self.root == path
    }

    fn awaits_branch_deletion(&self, request: Option<GitRequestId>, branch: &str) -> bool {
        self.stage == WorktreeTeardownStage::BranchDeleting
            && self.git_request == request
            && self
                .branch
                .as_ref()
                .is_some_and(|plan| plan.branch == branch)
    }
}

#[cfg(unix)]
#[derive(Clone, Debug)]
struct WorktreeTeardown {
    plan: WorktreeRemovalPlan,
    authorization: DeletionAuthorization,
    /// The session being taken down, or `None` when the worktree had only a
    /// history record and no running host.
    session: Option<AttachedSession>,
    /// The catalog identity to forget once the directory is gone. Always
    /// present, because a worktree that was ever opened as a workspace holds a
    /// number whether or not a host is running in it now.
    root: PathBuf,
    /// The branch to delete once its worktree is gone, when this teardown was
    /// reached from the branch list rather than the worktree list.
    branch: Option<BranchDeletionPlan>,
    /// The stop or forget request this stage is waiting for. Workspace list
    /// refreshes have their own generations and must not make a destructive
    /// cascade ignore the reply to a request it already sent.
    workspace_request_generation: Option<u64>,
    /// The exact asynchronous Git removal this cascade is waiting for. Path
    /// equality alone cannot distinguish a stale or duplicate completion from
    /// the mutation that belongs to this confirmed action.
    git_request: Option<GitRequestId>,
    stage: WorktreeTeardownStage,
}

#[cfg(unix)]
#[derive(Clone, Debug)]
struct PendingWorktreeRemovalCheck {
    plan: WorktreeRemovalPlan,
    authorization: Option<DeletionAuthorization>,
    /// Present before the confirmation is first shown. Once confirmation has
    /// been accepted, the second health check belongs to that explicit action
    /// even though its overlay has closed.
    origin: Option<(usize, u64)>,
    /// The branch whose deletion this removal is one level of, when it was
    /// reached from the branch list. It also settles which list the origin
    /// gate reads: the row still under the cursor is a branch there, not a
    /// worktree.
    branch: Option<BranchDeletionPlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GitDiscardConfirmation {
    paths: Vec<PathBuf>,
    skipped_untracked: usize,
}

impl GitDiscardConfirmation {
    fn message(&self) -> String {
        let what = match self.paths.as_slice() {
            [only] => only.display().to_string(),
            many => format!("{} paths", many.len()),
        };
        let mut sentences = vec![format!("Press Enter to discard changes to {what}.")];
        if self.skipped_untracked > 0 {
            sentences.push(format!(
                "{} untracked file{} will be left alone.",
                self.skipped_untracked,
                if self.skipped_untracked == 1 { "" } else { "s" }
            ));
        }
        sentences.push("This cannot be undone.".to_owned());
        sentences.push("Escape keeps them.".to_owned());
        sentences.join("\n")
    }
}

/// The levels below a branch that deleting it has to take with it.
///
/// A worktree is meaningless without the branch it has checked out, so the
/// branch list no longer refuses a checked-out branch and asks the reader to
/// go and dismantle it by hand. It offers the whole cascade instead, named in
/// full before it is accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
struct BranchCascade {
    worktree: WorktreeRemovalPlan,
    /// The running session on that worktree, when there is one.
    session: Option<AttachedSession>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BranchDeletionConfirmation {
    plan: BranchDeletionPlan,
    /// The worktree and session that go with this branch, when it has one
    /// checked out.
    cascade: Option<BranchCascade>,
    input: String,
    cursor: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BranchSwitch {
    Checkout { branch: String },
    Create { branch: String, start: String },
}

impl BranchSwitch {
    fn branch(&self) -> &str {
        match self {
            Self::Checkout { branch } | Self::Create { branch, .. } => branch,
        }
    }

    fn message(&self) -> String {
        let action = match self {
            Self::Checkout { branch } => format!("Switch to branch {branch}."),
            Self::Create { branch, .. } => format!("Create and switch to branch {branch}."),
        };
        format!(
            "{action}\nA terminal session is still running in this workspace and will keep using the same working directory while Git replaces files.\nType {} exactly to continue.\nEscape keeps the current branch.",
            self.branch()
        )
    }

    fn cancelled_message(&self) -> &'static str {
        match self {
            Self::Checkout { .. } => "checkout cancelled; the branch was not changed",
            Self::Create { .. } => "branch creation cancelled; nothing was changed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BranchSwitchConfirmation {
    repository: Repository,
    action: BranchSwitch,
    input: String,
    cursor: usize,
}

impl BranchDeletionConfirmation {
    /// A cascade always asks for typed text. What the branch tip alone would
    /// have settled for says nothing about the worktree and the session that
    /// go with it.
    fn typed(&self) -> bool {
        self.plan.required_authorization == DeletionAuthorization::Typed || self.cascade.is_some()
    }

    fn message(&self) -> String {
        let mut sentences = vec![format!("Delete branch {}.", self.plan.branch)];
        if let Some(cascade) = &self.cascade {
            sentences.push("This also:".to_owned());
            if let Some(session) = &cascade.session {
                sentences.push(format!("  · stops and forgets {}", session.describe()));
            }
            sentences.push(format!(
                "  · removes worktree {}",
                crate::git::display_path(&cascade.worktree.path)
            ));
        }
        if !self.plan.retaining_branches.is_empty() {
            sentences.push(format!(
                "Its commits are retained by local branch{} {}.",
                if self.plan.retaining_branches.len() == 1 {
                    ""
                } else {
                    "es"
                },
                self.plan.retaining_branches.join(", ")
            ));
        }
        if let Some(upstream) = &self.plan.upstream {
            let state = upstream.divergence.map_or_else(
                || "is gone".to_owned(),
                |divergence| {
                    format!(
                        "is {} commit(s) ahead and {} behind",
                        divergence.ahead, divergence.behind
                    )
                },
            );
            sentences.push(format!(
                "Cached upstream {} {state}; fetch to refresh this information.",
                upstream.name
            ));
        }
        if self.cascade.is_some() {
            sentences.push(format!("Type {} exactly to continue.", self.plan.branch));
        } else if self.typed() {
            sentences.push(format!(
                "Its tip is not retained by another known branch; type {} exactly to continue.",
                self.plan.branch
            ));
        } else {
            sentences.push("Press Enter to continue.".to_owned());
        }
        sentences.push("Escape keeps it.".to_owned());
        sentences.join("\n")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GitStashConfirmation {
    repository: Repository,
    mutation: StashMutation,
    message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfirmationOverlay {
    title: &'static str,
    accept: &'static str,
    message: String,
    input: Option<(String, usize)>,
}

impl WorktreeRemovalConfirmation {
    /// A compound action always asks for typed text, whatever the worktree's
    /// own Git state would have settled for on its own. Stopping somebody's
    /// running editor is not a thing to agree to with one keystroke meant for
    /// a clean directory.
    fn typed(&self) -> bool {
        self.plan.required_authorization == DeletionAuthorization::Typed || self.session.is_some()
    }

    fn expected(&self) -> String {
        self.plan
            .branch
            .clone()
            .unwrap_or_else(|| crate::git::display_path(&self.plan.path))
    }

    fn message(&self) -> String {
        let branch_note = self.plan.branch.as_deref().map_or_else(
            || "No branch will be deleted".to_owned(),
            |branch| format!("Branch {branch} will remain"),
        );
        let mut lines = vec![format!(
            "Remove worktree {}.",
            crate::git::display_path(&self.plan.path)
        )];
        if let Some(session) = &self.session {
            lines.push(format!(
                "This also stops and forgets {}.",
                session.describe()
            ));
        }
        lines.push(format!("{branch_note}."));
        if let Some(upstream) = &self.plan.upstream {
            let state = upstream.divergence.map_or_else(
                || "is gone".to_owned(),
                |divergence| {
                    format!(
                        "is {} commit(s) ahead and {} behind",
                        divergence.ahead, divergence.behind
                    )
                },
            );
            lines.push(format!(
                "Cached upstream {} {state}; fetch to refresh this information.",
                upstream.name
            ));
        }
        if self.typed() {
            lines.push(format!("Type {} exactly to continue.", self.expected()));
        } else {
            lines.push("Press Enter to continue.".to_owned());
        }
        lines.push("Escape keeps it.".to_owned());
        lines.join("\n")
    }
}

/// What a pull found when it could not fast-forward: commits on both sides,
/// and the offer to replay the local ones on top of the remote ones.
///
/// A fast-forward is silent because it decides nothing — the branch had no
/// commits of its own to lose. A rebase does decide something: it rewrites the
/// commits held here, which stay reachable from the reflog but no longer under
/// the identities that were pushed anywhere else. So this asks, and says how
/// much it would replay.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PullRebaseConfirmation {
    branch: String,
    upstream: String,
    ahead: usize,
    behind: usize,
}

impl PullRebaseConfirmation {
    fn message(&self) -> String {
        let Self {
            branch,
            upstream,
            ahead,
            behind,
        } = self;
        format!(
            "Branch {branch} and {upstream} have both moved on.\nPress Enter to replay {ahead} local \
             commit{} on top of the {behind} on {upstream}.\nA conflict undoes the replay and \
             changes nothing.\nEscape leaves the branch as it is.",
            if *ahead == 1 { "" } else { "s" }
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct LogViewRequest {
    /// Existing view that requested a continuation; `None` is an explicit
    /// first open and is allowed to create the view when it completes.
    buffer: Option<usize>,
    /// Page this request will display once it completes. A response for any
    /// other page is stale — the person paged again while it was running.
    page: usize,
}

/// Presentation-neutral result of running one semantic command.
///
/// Commands still update `App` immediately; this value lets a terminal,
/// headless host, or future RPC frontend understand what kind of interaction
/// the command produced without reverse-engineering editor fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandOutcome {
    Completed,
    Status(String),
    UserError(String),
    Unavailable(String),
    Confirmation(String),
    Prompt(PromptKind),
    AsynchronousRequest(Option<String>),
}

/// Whether pointer input changed anything represented by an editor frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerOutcome {
    Changed,
    Unchanged,
}

#[derive(Clone, Debug)]
struct SettingPreview {
    setting: SettingId,
    original_config: Config,
    original_theme: Theme,
    original_theme_name: String,
    original_grammar: ActiveGrammar,
    original_mode: Mode,
}

#[derive(Clone, Debug)]
enum SettingsView {
    Values(Box<SettingPreview>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PointerDrag {
    Selection {
        pane: usize,
        buffer: usize,
        anchor: Offset,
    },
    TerminalSelection {
        pane: usize,
        terminal: TerminalId,
        anchor: Offset,
    },
    Resize {
        first: usize,
        second: usize,
        axis: Axis,
        last_column: u16,
        last_row: u16,
    },
}

#[derive(Clone, Debug)]
struct CommandState {
    status_revision: u64,
    unavailable_revision: u64,
    prompt_revision: u64,
    confirmation_revision: u64,
    lsp_requests: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandOutcomeHint {
    Infer,
    Unavailable,
    Asynchronous,
}

impl CommandState {
    fn capture(app: &App) -> Self {
        Self {
            status_revision: app.status_revision,
            unavailable_revision: app.unavailable_revision,
            prompt_revision: app.prompt_revision,
            confirmation_revision: app.confirmation_revision,
            lsp_requests: app.lsp_requests.len(),
        }
    }

    fn outcome(self, app: &App, hint: CommandOutcomeHint) -> CommandOutcome {
        let status_changed = self.status_revision != app.status_revision;
        let asynchronous =
            app.lsp_requests.len() > self.lsp_requests || hint == CommandOutcomeHint::Asynchronous;

        if hint == CommandOutcomeHint::Unavailable
            || self.unavailable_revision != app.unavailable_revision
        {
            return CommandOutcome::Unavailable(app.status.clone());
        }
        if status_changed && app.status_error {
            return CommandOutcome::UserError(app.status.clone());
        }
        if self.confirmation_revision != app.confirmation_revision {
            return CommandOutcome::Confirmation(app.status.clone());
        }
        if self.prompt_revision != app.prompt_revision {
            return CommandOutcome::Prompt(app.prompt_kind);
        }
        if asynchronous {
            return CommandOutcome::AsynchronousRequest(status_changed.then(|| app.status.clone()));
        }
        if status_changed {
            return CommandOutcome::Status(app.status.clone());
        }
        CommandOutcome::Completed
    }
}

#[derive(Clone, Debug)]
pub struct FsConfirmation {
    pub buffer: usize,
    pub plan: FsPlan,
    /// Operation currently being reviewed. Its plan identity survives redraw
    /// and terminal resize; frontends derive only the visible window.
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferAction {
    Save,
    Discard,
    Close,
}

impl BufferAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Save => "Save",
            Self::Discard => "Discard changes",
            Self::Close => "Close",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BufferActionMenu {
    pub buffer: usize,
    pub actions: Vec<BufferAction>,
    pub selected: usize,
}

#[derive(Clone, Debug)]
struct ContextActionMenu {
    actions: Vec<ContextAction>,
    selected: usize,
}

impl ContextActionMenu {
    fn selected_action(&self) -> Option<ContextAction> {
        self.actions
            .get(self.selected.min(self.actions.len().saturating_sub(1)))
            .copied()
    }
}

/// What the workspace picker offers for one row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionAction {
    Open,
    Rename,
    Number,
    Close,
    ForceClose,
    Forget,
}

impl SessionAction {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Rename => "Rename",
            Self::Number => "Number",
            Self::Close => "Close",
            Self::ForceClose => "Force close",
            Self::Forget => "Forget",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Open => "Attach to this persistent session",
            Self::Rename => "Change this persistent session's name",
            Self::Number => "Set the 1-9 shortcut that reaches it, or clear it",
            Self::Close => "Stop this persistent session",
            Self::ForceClose => "End protected buffers, waiters, and live terminals",
            Self::Forget => "Remove this stopped session's visited-history record",
        }
    }
}

/// Reads a session number from a prompt answer.
///
/// An empty answer clears the number rather than being a mistake: the prompt
/// opens prefilled with the current one, so erasing it is how somebody says
/// this workspace should have no shortcut.
#[cfg(unix)]
fn parse_session_number(value: &str) -> Result<Option<u8>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let number = value.parse::<u8>().map_err(|_| {
        format!("a session number must be a digit from 1 to {MAX_WORKSPACE_NUMBER}")
    })?;
    if (1..=MAX_WORKSPACE_NUMBER).contains(&number) {
        Ok(Some(number))
    } else {
        Err(format!(
            "a session number must be a digit from 1 to {MAX_WORKSPACE_NUMBER}"
        ))
    }
}

#[derive(Clone, Debug)]
struct SessionActionMenu {
    row: usize,
    actions: Vec<SessionAction>,
    selected: usize,
    force_armed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalAction {
    Show,
    Rename,
    Close,
    Create,
}

impl TerminalAction {
    fn label(self) -> &'static str {
        match self {
            Self::Show => "Show",
            Self::Rename => "Rename",
            Self::Close => "Close",
            Self::Create => "Create",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Show => "Show this session in the active pane",
            Self::Rename => "Name this session",
            Self::Close => "End and forget this session",
            Self::Create => "Create a shell in the working directory",
        }
    }
}

#[derive(Clone, Debug)]
struct TerminalActionMenu {
    id: TerminalId,
    actions: Vec<TerminalAction>,
    selected: usize,
    close_armed: bool,
}

impl TerminalActionMenu {
    fn selected_action(&self) -> Option<TerminalAction> {
        self.actions
            .get(self.selected.min(self.actions.len().saturating_sub(1)))
            .copied()
    }
}

impl SessionActionMenu {
    fn selected_action(&self) -> Option<SessionAction> {
        self.actions
            .get(self.selected.min(self.actions.len().saturating_sub(1)))
            .copied()
    }
}

/// Where `:path` can send the active buffer's absolute path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PathClipboardTarget {
    System,
    Register,
}

impl PathClipboardTarget {
    fn label(self) -> &'static str {
        match self {
            Self::System => "copy to system clipboard",
            Self::Register => "copy to Runyte register",
        }
    }

    fn mnemonic(self) -> char {
        match self {
            Self::System => 's',
            Self::Register => 'r',
        }
    }
}

/// The read-only `:path` popup, showing the active buffer's absolute path.
#[derive(Clone, Debug)]
struct PathPopup {
    path: String,
}

/// The `Tab`-opened menu of copy targets nested inside the `:path` popup.
#[derive(Clone, Debug)]
struct PathActionMenu {
    actions: Vec<PathClipboardTarget>,
    selected: usize,
}

impl PathActionMenu {
    fn selected_action(&self) -> Option<PathClipboardTarget> {
        self.actions
            .get(self.selected.min(self.actions.len().saturating_sub(1)))
            .copied()
    }
}

impl BufferActionMenu {
    pub fn selected_action(&self) -> Option<BufferAction> {
        self.actions
            .get(self.selected.min(self.actions.len().saturating_sub(1)))
            .copied()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramChoice {
    pub program: String,
    pub system: bool,
    pub remembered: bool,
    pub is_default: bool,
}

impl ProgramChoice {
    fn launch_value(&self) -> String {
        if self.system {
            String::new()
        } else {
            self.program.clone()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramAction {
    Delete,
    SetDefault,
}

impl ProgramAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Delete => "Delete",
            Self::SetDefault => "Set as default",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProgramActionMenu {
    pub choice: ProgramChoice,
    pub actions: Vec<ProgramAction>,
    pub selected: usize,
}

impl ProgramActionMenu {
    pub fn selected_action(&self) -> Option<ProgramAction> {
        self.actions
            .get(self.selected.min(self.actions.len().saturating_sub(1)))
            .copied()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Register {
    text: String,
    linewise: bool,
    directory: Option<DirectoryRegister>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectoryRegister {
    entries: Vec<DirectoryTransfer>,
    mode: TransferMode,
}

/// The completion popup.
///
/// `anchor` is where the request was made. Filtering happens locally against
/// what has been typed since, so the popup narrows while the person keeps
/// typing instead of waiting for another round trip.
#[derive(Clone, Debug)]
pub struct CompletionState {
    pub items: Vec<Completion>,
    pub selected: usize,
    pub buffer: usize,
    pub anchor: Offset,
    pub filter: String,
    pub source: CompletionSource,
    /// Present only for an explicit `Ctrl-x` language-completion session.
    /// The identity rejects responses from cancelled or superseded sessions.
    explicit_session: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionSource {
    Language,
    Path,
    Word,
}

/// Bounds how many candidates one path popup keeps from each of its
/// at-most-two distinct path roots, so a full directory on one root cannot
/// crowd out the other's names.
///
/// The bound is on what is kept, not on what is read: every entry of the
/// directory is offered to the typed prefix first, because a directory read
/// returns names in whatever order the filesystem holds them, and cutting
/// that order short would hide matches for no reason a person could see.
const PATH_COMPLETION_ITEM_LIMIT_PER_ROOT: usize = 512;
/// The same bound for the command palette's path argument rows.
const COMMAND_PATH_HINT_LIMIT: usize = 512;
/// Bounds one word-completion popup's candidate count, independent of the
/// worker's own per-buffer memory bound.
const WORD_COMPLETION_ITEM_LIMIT: usize = 200;

impl CompletionState {
    pub fn visible_indices(&self) -> Vec<usize> {
        let query = self.filter.to_lowercase();
        let mut indices: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let filter = item.filter_text.as_deref().unwrap_or(&item.label);
                (query.is_empty() || filter.to_lowercase().starts_with(&query)).then_some(index)
            })
            .collect();
        if self.source == CompletionSource::Language {
            indices.sort_by(|left, right| {
                let left = &self.items[*left];
                let right = &self.items[*right];
                left.sort_text
                    .as_deref()
                    .unwrap_or(&left.label)
                    .cmp(right.sort_text.as_deref().unwrap_or(&right.label))
            });
        }
        indices
    }

    pub fn selected_item(&self) -> Option<&Completion> {
        let indices = self.visible_indices();
        indices
            .get(self.selected.min(indices.len().saturating_sub(1)))
            .and_then(|index| self.items.get(*index))
    }

    fn step(&mut self, forward: bool) {
        let count = self.visible_indices().len();
        if count == 0 {
            return;
        }
        let current = self.selected.min(count - 1);
        self.selected = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
    }
}

/// The signature-help popup.
#[derive(Clone, Debug)]
pub struct SignatureState {
    pub signatures: Vec<SignatureLine>,
}

/// The hover popup.
#[derive(Clone, Debug)]
pub struct HoverState {
    pub lines: Vec<String>,
}

const HOVER_PEEK_ROWS: usize = 12;

fn hover_content_rows(editor_height: u16) -> usize {
    usize::from(
        editor_height
            .min(HOVER_PEEK_ROWS as u16 + 2)
            .saturating_sub(2),
    )
}

/// What a language server's negotiated handshake means for one language.
#[derive(Clone, Debug)]
struct ServerState {
    name: String,
    generation: u64,
    encoding: Encoding,
    sync: DocumentSync,
    capabilities: Capabilities,
}

/// A document the editor has told a language server about.
#[derive(Clone, Debug)]
struct DocumentState {
    language: String,
    path: PathBuf,
    version: i32,
    /// A manager-queue refusal means the server did not see the previous
    /// change. The next accepted update must therefore carry the whole file.
    desynced: bool,
}

/// What the editor intends to do with a response it is waiting for.
#[derive(Clone, Debug)]
enum PendingRequest {
    /// Jump to the result, or offer a picker when there is more than one.
    Goto {
        label: &'static str,
    },
    Hover,
    Completion {
        buffer: usize,
        anchor: Offset,
        explicit_session: Option<u64>,
    },
    Signature,
    Symbols {
        title: &'static str,
        path: PathBuf,
    },
    CodeActions,
    /// Apply the resulting edits. `path` fills in the document for a
    /// formatting response, which is scoped to one file and carries no URI.
    Edits {
        label: &'static str,
        path: PathBuf,
    },
}

impl PendingRequest {
    /// Requests tied to one transient source position. A newer request in the
    /// same group makes an older response unusable and should cancel it at the
    /// protocol boundary instead of retaining server work indefinitely.
    fn transient_group(&self) -> Option<u8> {
        match self {
            Self::Hover => Some(0),
            Self::Completion { .. } => Some(1),
            Self::Signature => Some(2),
            _ => None,
        }
    }

    fn source_revision_must_match(&self) -> bool {
        matches!(
            self,
            Self::Goto { .. }
                | Self::Hover
                | Self::Completion { .. }
                | Self::Signature
                | Self::Edits { .. }
                | Self::CodeActions
        ) || matches!(self, Self::Symbols { path, .. } if !path.as_os_str().is_empty())
    }
}

/// A request together with the buffer that originated it.
///
/// The origin survives even for protocol responses that carry no document
/// path (notably resolved code actions), so closing a buffer can retire every
/// response that could otherwise act on its former contents.
#[derive(Clone, Debug)]
struct TrackedRequest {
    buffer: usize,
    revision: u64,
    documents: HashMap<PathBuf, (usize, u64)>,
    pending: PendingRequest,
    cancelled: bool,
    server: Option<(String, u64)>,
}

#[derive(Clone, Debug)]
struct ActionSource {
    buffer: usize,
    revision: u64,
    documents: HashMap<PathBuf, (usize, u64)>,
    language: String,
    generation: u64,
}

impl TrackedRequest {
    fn new(buffer: usize, revision: u64, pending: PendingRequest) -> Self {
        Self {
            buffer,
            revision,
            documents: HashMap::new(),
            pending,
            cancelled: false,
            server: None,
        }
    }

    fn with_documents(mut self, documents: HashMap<PathBuf, (usize, u64)>) -> Self {
        self.documents = documents;
        self
    }

    fn with_server(mut self, language: String, generation: u64) -> Self {
        self.server = Some((language, generation));
        self
    }
}

/// A command paired with the spelling that matched what was typed.
///
/// Aliases are real command names, so a prefix that only an alias matches is
/// listed under that alias rather than under a canonical name the person is
/// not typing: `:sp` offers `:split`, not `:hsplit`.
#[derive(Clone, Debug)]
pub struct CommandMatch {
    pub spec: &'static CommandSpec,
    pub name: &'static str,
    pub category: CommandCategory,
    pub availability: CommandAvailability,
}

#[derive(Clone, Debug)]
struct ActionFeedback {
    id: u64,
    spelling: String,
    text: String,
    /// Whether `text` reports a failure rather than a success, an
    /// unavailable action, or a warning/informational result. Drives
    /// `interaction_line_error`, the same error/non-error styling
    /// distinction the notification center already makes.
    is_error: bool,
}

/// One filesystem entry offered while a path-valued command argument is typed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathHint {
    /// The path spelling that will replace the argument, retaining `~` and
    /// relative prefixes so completion does not unexpectedly rewrite intent.
    pub value: String,
    pub detail: String,
    pub is_directory: bool,
}

impl CommandMatch {
    fn new(
        spec: &'static CommandSpec,
        name: &'static str,
        capabilities: &AppCapabilitySnapshot,
    ) -> Self {
        Self {
            spec,
            name,
            category: spec.category(),
            availability: capabilities.command_availability(spec),
        }
    }

    fn canonical(spec: &'static CommandSpec, capabilities: &AppCapabilitySnapshot) -> Self {
        Self::new(spec, spec.name, capabilities)
    }

    /// The usage line retitled with the matched spelling.
    pub fn usage(&self) -> String {
        match self.spec.usage.strip_prefix(self.spec.name) {
            Some(rest) => format!("{}{rest}", self.name),
            None => self.spec.usage.to_owned(),
        }
    }

    /// The command's remaining spellings, canonical name included.
    pub fn other_names(&self) -> Vec<&'static str> {
        self.spec
            .names()
            .filter(|name| *name != self.name)
            .collect()
    }
}

/// Host-owned capabilities and outbound service work used by the editor.
///
/// Keeping these handles together makes the core coordinator constructible
/// with inert ports while the terminal application can attach live services.
/// The value is crate-visible only so the headless facade can explicitly
/// choose isolated ports; its fields and operations remain narrow.
pub(crate) struct HostPorts {
    clipboard: Box<dyn SystemClipboard>,
    lsp: Option<LspHandle>,
    /// The Git boundary, absent when no `git` executable was found. Every Git
    /// surface is off in that case rather than reporting failures.
    git: Option<Box<dyn GitProvider>>,
    git_service: Option<GitServiceHandle>,
    #[cfg(unix)]
    workspace_service: Option<WorkspaceServiceHandle>,
    word_index: Option<WordIndexHandle>,
}

impl HostPorts {
    fn live() -> Self {
        Self::isolated(Box::new(CommandClipboard))
    }

    pub(crate) fn isolated(clipboard: Box<dyn SystemClipboard>) -> Self {
        Self {
            clipboard,
            lsp: None,
            git: None,
            git_service: None,
            #[cfg(unix)]
            workspace_service: None,
            word_index: None,
        }
    }

    fn replace_clipboard(&mut self, clipboard: Box<dyn SystemClipboard>) {
        self.clipboard = clipboard;
    }

    /// Substitutes the Git boundary.
    ///
    /// Only tests need this: the live provider is chosen once, when the ports
    /// are built. A test that swapped it later would be describing an editor
    /// nobody runs.
    #[cfg(test)]
    pub(crate) fn replace_git(&mut self, git: Box<dyn GitProvider>) {
        self.git = Some(git);
    }

    fn clipboard(&mut self) -> &mut dyn SystemClipboard {
        self.clipboard.as_mut()
    }

    fn attach_lsp(&mut self, handle: LspHandle) {
        self.lsp = Some(handle);
    }

    fn has_lsp(&self) -> bool {
        self.lsp.is_some()
    }

    fn send_lsp(&self, command: LspCommand) -> Option<bool> {
        Some(self.lsp.as_ref()?.send(command))
    }

    fn attach_word_index(&mut self, handle: WordIndexHandle) {
        self.word_index = Some(handle);
    }

    fn word_index(&self) -> Option<&WordIndexHandle> {
        self.word_index.as_ref()
    }
}

/// What a commit message buffer says above the file list.
///
/// Git's own wording, near enough: the instructions belong with the message
/// being written rather than in documentation somebody would have to go and
/// find while the buffer is open in front of them.
const COMMIT_INSTRUCTIONS: &str = "\
# Write this buffer to commit it, or `:c!` to abandon an edited message.
# Lines starting with '#' are not part of the message.
";

type DiffRowIdentity = (String, String, usize);

fn diff_row_identity(text: &str, target: usize) -> Option<DiffRowIdentity> {
    let mut hunk = String::new();
    let mut occurrence = 0usize;
    let lines = text.lines().collect::<Vec<_>>();
    let line = *lines.get(target)?;
    for candidate in lines.iter().take(target + 1) {
        if candidate.starts_with("@@") {
            hunk = (*candidate).to_owned();
            occurrence = 0;
        }
        if *candidate == line {
            occurrence += 1;
        }
    }
    Some((hunk, line.to_owned(), occurrence))
}

fn diff_row_for_identity(text: &str, identity: &DiffRowIdentity) -> Option<usize> {
    let (wanted_hunk, wanted_line, wanted_occurrence) = identity;
    let mut hunk = String::new();
    let mut occurrence = 0usize;
    for (row, line) in text.lines().enumerate() {
        if line.starts_with("@@") {
            hunk = line.to_owned();
        }
        if hunk == *wanted_hunk && line == wanted_line {
            occurrence += 1;
            if occurrence == *wanted_occurrence {
                return Some(row);
            }
        }
    }
    None
}

/// The message a commit buffer holds, with its comments and surrounding blank
/// lines removed.
///
/// Comment stripping matches what Git does after an editor session, which is
/// also why a line of the message itself cannot begin with `#`.
fn commit_message_body(text: &str) -> String {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// Register the unnamed macro is recorded into.
///
/// Named registers are addressed by the key that names them, so the default
/// one needs a register no key would reach by accident: `@` is the register
/// Helix and Vim already spell macros with.
const DEFAULT_MACRO_REGISTER: char = '@';

/// Work admitted by one top-level macro invocation, including raw events,
/// counted command repetitions, text characters, and nested macros.
const MAX_MACRO_REPLAY_WORK: usize = 10_000;

/// Work returned to the host as one cooperative macro-replay slice.
const MACRO_REPLAY_BATCH_INPUTS: usize = 128;

/// Largest grammar-level range count allowed to execute as one action. Range
/// intents carry stateful semantics that cannot always be split into repeated
/// count-one commands, so larger counts are refused before they take effect.
const MAX_MACRO_REPLAY_ATOMIC_REPETITIONS: usize = MACRO_REPLAY_BATCH_INPUTS;

/// Defensive bound on distinct nested macro frames. Cycles are rejected
/// earlier with their register chain; this still bounds malformed state.
const MAX_MACRO_REPLAY_DEPTH: usize = 16;

#[derive(Clone, Debug)]
struct MacroReplayFrame {
    register: char,
    inputs: Vec<InputEvent>,
    repetitions_remaining: usize,
    next_input: usize,
}

#[derive(Clone, Debug)]
struct MacroReplayCommand {
    invocation: CommandInvocation,
    repetitions_remaining: usize,
}

#[derive(Clone, Debug)]
enum MacroReplayAction {
    Input(InputEvent),
    Command(CommandInvocation),
}

#[derive(Clone, Debug)]
struct MacroReplay {
    root_register: char,
    frames: Vec<MacroReplayFrame>,
    commands: Vec<MacroReplayCommand>,
    remaining_work: usize,
    processed_work: usize,
    abort_reason: Option<String>,
    last_action_error: bool,
}

/// Clean special buffers retained across pane switches. The current view and
/// its immediate predecessor are enough for back/forward navigation without
/// letting generated views accumulate in the buffer picker.
const SPECIAL_BUFFER_RETENTION_LIMIT: usize = 2;

/// A workspace selector together with the editor directory in which it was
/// entered.
///
/// The selector stays untouched because a relative-looking value may instead
/// be an exact workspace name or an ID prefix. The attached client uses the
/// captured directory only when interpreting the selector as a path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSwitchRequest {
    pub selector: PathBuf,
    pub working_directory: PathBuf,
}

/// What an editor-level exit means to a persistent session host.
///
/// Standalone mode only reads [`App::should_quit`]. A persistent host also
/// needs the intent: `:detach` leaves its state alive, while an ordinary quit
/// asks the host itself to end after its lifecycle guards pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistentExitRequest {
    Detach,
    Quit { force: bool },
}

pub struct App {
    pub config: Config,
    /// Last successfully loaded or settings-written configuration. Runtime
    /// state stays separate so restart-required settings remain truthful.
    persisted_config: Config,
    pub theme: Theme,
    pub theme_name: String,
    /// The exact YAML document settings writes patch. Set only by the config
    /// loader, and injectable through `note_loaded_config` for isolated tests.
    config_path: Option<PathBuf>,
    pub buffers: Vec<Buffer>,
    /// Current syntax trees, indexed alongside `buffers`. `None` means the
    /// buffer has no known language, its parse failed, or its tree is held by
    /// `stale_syntax` while a background parse is pending.
    pub syntax: Vec<Option<DocumentSyntax>>,
    /// Stale trees retain translated highlighting but expose no structural
    /// query while their replacement is being parsed.
    stale_syntax: HashMap<usize, StaleSyntax>,
    syntax_worker: Option<SyntaxHandle>,
    pub registry: Arc<Registry>,
    /// Every terminal this editor owns, live or finished.
    ///
    /// Held beside the buffers rather than among them: a session outlives the
    /// pane that showed it, so closing a split or opening a file leaves the
    /// child running and reachable from the terminal list.
    pub terminals: TerminalSessions,
    /// The terminal most recently shown, so a send from a document pane has an
    /// answer when no terminal is on screen.
    last_terminal: Option<TerminalId>,
    pub panes: HashMap<usize, Pane>,
    pub layout: Layout,
    pub active_pane: usize,
    /// The pane temporarily presented across the complete editor area by
    /// `:zen` or `:fullscreen`, if either is active.
    maximized: Option<MaximizedPane>,
    /// Live side-by-side comparisons. A pane and a buffer belong to at most
    /// one of these, so a session is identified by either.
    pub diffs: Vec<DiffSession>,
    /// The buffer `:diff-this` armed, waiting for a second one to compare it
    /// with. Cleared when the pairing happens, when the same buffer arms
    /// again, or when the buffer goes away.
    pending_diff: Option<usize>,
    pub mode: Mode,
    /// The reversible overwrite trail owned by a live Replace-mode edit.
    replace_session: Option<ReplaceSession>,
    pub command: String,
    pub command_cursor: usize,
    pub command_selection: usize,
    pub prompt_kind: PromptKind,
    /// The binary file waiting for a program to open it, set while
    /// `PromptKind::ExternalProgram` is collecting one.
    pub external_target: Option<PathBuf>,
    /// Programs previously chosen for binary files and their persisted default.
    pub programs: ProgramCache,
    /// The mode the command palette was opened from.
    ///
    /// The palette leaves the editor in Normal mode whatever it was opened
    /// from, so a command that needs to describe where the reader came from
    /// has to be told; nothing else can recover it.
    prompt_origin_mode: Mode,
    prompt_revision: u64,
    pub picker: Option<FilePicker>,
    /// The project-root picker's in-memory buffer/terminal mode. Directory
    /// pickers and fuzzy grep leave this absent and retain their old keys.
    pub finder: Option<ResourceFinder>,
    file_scanner: Option<FileScanner>,
    next_file_scan_id: u64,
    /// A filesystem plan waiting for a separate, explicit confirmation.
    pub fs_confirmation: Option<FsConfirmation>,
    confirmation_revision: u64,
    /// A directory buffer whose unsaved edits are about to be discarded, and
    /// where it should go afterwards: `None` re-reads the same directory,
    /// `Some(path)` resumes a navigation the edits were blocking.
    directory_reload_confirmation: Option<DirectoryReloadConfirmation>,
    file_reload_confirmation: Option<FileReloadConfirmation>,
    /// Symbols, references, diagnostics, and code actions, which are all the
    /// same list-and-filter interaction.
    pub list: Option<ListPicker>,
    /// Contextual actions for the selected row of the buffer picker.
    pub buffer_action_menu: Option<BufferActionMenu>,
    /// Registry-backed actions for the row or buffer under the editor caret.
    context_action_menu: Option<ContextActionMenu>,
    /// Contextual actions for a remembered external program.
    pub program_action_menu: Option<ProgramActionMenu>,
    /// The active buffer's absolute path, shown by `:path`.
    path_popup: Option<PathPopup>,
    /// The `Tab`-opened copy-target menu nested inside the `:path` popup.
    path_action_menu: Option<PathActionMenu>,
    /// An editable buffer whose unsaved text will be reset after a separate Enter.
    buffer_discard_confirmation: Option<usize>,
    /// Repository-relative paths whose uncommitted changes will be thrown
    /// away after a separate Enter. The one confirmation in the editor that
    /// guards something no history can return.
    git_discard_confirmation: Option<GitDiscardConfirmation>,
    /// Retired buffer identities. Buffer indices are durable within an editor
    /// process because panes, jumps, and asynchronous LSP responses carry
    /// them, so closure tombstones an identity instead of shifting the arena.
    closed_buffers: HashSet<usize>,
    /// Live special buffers from least to most recently active.
    ///
    /// Dirty and visible buffers may temporarily exceed the clean retention
    /// limit: neither can be discarded safely merely because another special
    /// buffer was opened.
    special_buffer_recency: Vec<usize>,
    /// The next buffer id `sync_word_index` has not yet indexed. Buffer ids
    /// are append-only and never reused, so a monotonic cursor checked once
    /// per frame is enough to index every newly opened buffer exactly once
    /// without a hook at every `self.buffers.push` call site.
    word_index_next_buffer: usize,
    /// Startup positions for buffers that have not yet been activated.
    ///
    /// Positions stay one-based until activation so they can be clamped
    /// against the then-current Unicode text without retaining byte offsets.
    launch_positions: HashMap<usize, LaunchPosition>,
    pub completion: Option<CompletionState>,
    pub signature: Option<SignatureState>,
    pub hover: Option<HoverState>,
    pub diagnostics: DiagnosticStore,
    pub status: String,
    pub status_error: bool,
    /// Workspace-lifetime notifications. The persistent host owns `App`, so
    /// this history survives TUI detach/reattach but is never written to disk.
    notifications: NotificationCenter,
    /// Presentation-only action echo for the last interactive command.
    /// Notifications never replace it; the next interaction does.
    action_feedback: Option<ActionFeedback>,
    /// Interactive action currently dispatching. Asynchronous requests retain
    /// this identity so their completion cannot rewrite a newer action echo.
    active_action_id: Option<u64>,
    next_action_id: u64,
    /// When the person last acted in the editor.
    ///
    /// The automatic Git refresh waits out its own interval after this, so
    /// reconciliation happens once they have paused rather than moving the
    /// cursor out from under them mid-navigation.
    last_interaction: Instant,
    /// Lazy grammar configurations already reported to the user, keyed by
    /// public language identity and canonical/injection-free variant.
    reported_registry_errors: HashSet<(LanguageId, bool)>,
    status_revision: u64,
    unavailable_revision: u64,
    pub should_quit: bool,
    persistent_exit_request: Option<PersistentExitRequest>,
    /// Directory handed to a cooperating shell wrapper after `:quit-here`.
    quit_directory: Option<PathBuf>,
    /// Whether the launcher supplied the channel needed to change its shell.
    quit_directory_handoff: bool,
    pub areas: HashMap<usize, Rect>,
    /// The open help window, holding the topic it was opened for.
    ///
    /// The topic is captured rather than derived at render time because the
    /// command palette has already returned to Normal mode by the time `:?`
    /// runs, and the reader asked for help about the view they were in.
    /// The directory used by `:cd`, relative file commands, and `Space E`.
    ///
    /// This is deliberately separate from `project_root`: changing where the
    /// explorer starts must not silently retarget language servers, the
    /// persistent session host, or global search.
    pub working_directory: PathBuf,
    /// Captured once so path expansion and hints agree throughout a session,
    /// and so tests can inject a disposable home without mutating the process
    /// environment shared by concurrently running tests.
    home_directory: Option<PathBuf>,
    /// Directory listings kept for path completion. Behind a cell because the
    /// palette computes its rows while drawing, from a shared editor, and
    /// re-reading a large directory once per frame is the cost this exists to
    /// avoid.
    path_listings: RefCell<DirectoryListings>,
    pub project_root: PathBuf,
    pub state_root: PathBuf,
    /// What Git says about the project and about each open file. Marks are
    /// derived here rather than asked for again on every edit.
    git: GitTracker,
    /// Git-service bookkeeping and the typed rows behind generated Git views.
    git_state: GitWorkflowState,
    /// The branch a confirmed `D` would delete, and whether deleting it needs
    /// to be forced because its commits are not reachable from `HEAD`.
    git_branch_deletion: Option<BranchDeletionConfirmation>,
    /// A checkout or branch creation that needs exact-name authorization
    /// because a terminal child remains live in this workspace.
    git_branch_switch: Option<BranchSwitchConfirmation>,
    /// The drift a refused pull reported, held while the reader decides
    /// whether to replay their commits on top of it.
    git_pull_rebase: Option<PullRebaseConfirmation>,
    /// The branch a new one would start from, held while its name is typed.
    git_branch_start: Option<String>,
    /// The typed path a confirmed `D` would remove from the worktree list.
    /// Never reconstructed from the lossy row text.
    git_worktree_removal: Option<WorktreeRemovalConfirmation>,
    #[cfg(unix)]
    pending_worktree_removal: Option<PendingWorktreeRemovalCheck>,
    /// A confirmed removal partway through taking its session down with it.
    #[cfg(unix)]
    worktree_teardown: Option<WorktreeTeardown>,
    #[cfg(unix)]
    worktree_removal_generation: u64,
    git_worktree_start: Option<String>,
    git_worktree_new_branch: Option<String>,
    git_stash_confirmation: Option<GitStashConfirmation>,
    workspace_switch: Option<WorkspaceSwitchRequest>,
    /// A persistent session can keep owning its buffers after a TUI detaches or
    /// switches roots. Standalone mode leaves this false because replacing its
    /// process would otherwise lose that text.
    persistent_session: bool,
    #[cfg(unix)]
    workspace_rows: Vec<WorkspaceRow>,
    /// This workspace's own session number, shown in the status line.
    ///
    /// Held separately from `workspace_rows` because the status line needs an
    /// answer from the first frame, while the rows exist only once somebody
    /// has opened the session manager. Both ultimately read the same per-user
    /// catalog.
    pub workspace_number: Option<u8>,
    #[cfg(unix)]
    workspace_generation: u64,
    #[cfg(unix)]
    workspace_preview_generation: u64,
    #[cfg(unix)]
    workspace_preview_target: Option<PathBuf>,
    #[cfg(unix)]
    workspace_previews: HashMap<PathBuf, Result<SessionPreview, String>>,
    #[cfg(unix)]
    session_rename_target: Option<PathBuf>,
    #[cfg(unix)]
    session_number_target: Option<PathBuf>,
    session_action_menu: Option<SessionActionMenu>,
    terminal_action_menu: Option<TerminalActionMenu>,
    /// The buffer a commit message was opened over, returned to once the
    /// commit is made. Writing a message is a detour, and a detour should end
    /// where it started rather than wherever closing a buffer happens to land.
    commit_origin: Option<usize>,
    ports: HostPorts,
    registers: HashMap<char, Register>,
    selected_register: char,
    macros: HashMap<char, Vec<InputEvent>>,
    recording_macro: Option<char>,
    /// Recorded events whose key sequence has not resolved yet.
    ///
    /// The sequence that stops a recording is itself typed while recording is
    /// still on, so its keys arrive here first and are dropped rather than
    /// appended once the recording ends. Everything else joins the macro the
    /// moment its sequence completes, in the order it arrived.
    macro_staging: Vec<InputEvent>,
    macro_replay: Option<MacroReplay>,
    /// Last successfully invoked command reached through an actual `Space ...`
    /// binding. The semantic invocation is replayed against current state;
    /// aliases such as `Ctrl-w` never enter this history.
    /// Live `goto-word` jump labels, painted over the active pane until two
    /// keystrokes name one or a stray key spends them.
    pub jump: Option<JumpLabels>,
    /// Live line selection from `x`/`X`, holding the mode to restore when it
    /// ends. Unlike `v`, a line selection is transient: it survives only
    /// consecutive `x`/`X` presses, and any other command drops it.
    line_select: Option<Mode>,
    keymap: &'static Keymap,
    grammar: ActiveGrammar,
    /// Cursor and scroll state per directory, keyed by path rather than by
    /// buffer: one explorer buffer now stands for every directory a pane has
    /// visited, so the buffer cannot say which view belongs to which listing.
    directory_views: HashMap<PathBuf, DirectoryView>,
    search: SearchQuery,
    search_selection: Option<SearchSelectionPresentation>,
    next_pane: usize,
    /// Monotonic pane-history clock used to resolve directional focus when
    /// several panes share the requested edge.
    pane_history_clock: u64,
    /// Creation order is kept separately because a pane that has never been
    /// active still needs a deterministic recency rank.
    pane_opened_at: HashMap<usize, u64>,
    pane_activated_at: HashMap<usize, u64>,
    lsp_servers: HashMap<String, ServerState>,
    lsp_documents: HashMap<usize, DocumentState>,
    lsp_requests: HashMap<u64, TrackedRequest>,
    /// Protocol replies that must not be lost when the manager queue is
    /// briefly full. Retried from the frame lifecycle and before new work.
    pending_lsp_replies: VecDeque<LspCommand>,
    /// What each row of `list` stands for, indexed by `PickerItem::index`.
    list_actions: Vec<ListAction>,
    /// Which registry-backed settings surface owns the shared list picker.
    settings_view: Option<SettingsView>,
    pointer_drag: Option<PointerDrag>,
    /// Code actions backing a code-action picker.
    lsp_actions: Vec<ActionEntry>,
    /// The buffer revision against which the visible code actions were
    /// computed. A chosen action must not edit text that has since changed.
    lsp_action_source: Option<ActionSource>,
    tutorial: Option<TutorialState>,
    next_lsp_token: u64,
    next_completion_session: u64,
}

impl App {
    pub fn file_monitor_requests(&self) -> Vec<FileObservationRequest> {
        self.buffers
            .iter()
            .enumerate()
            .filter(|(buffer, _)| !self.closed_buffers.contains(buffer))
            .filter_map(|(buffer, value)| value.file_observation_request(buffer))
            .collect()
    }

    pub(crate) fn apply_file_observation(&mut self, event: FileObservationEvent) {
        if event.buffer >= self.buffers.len() || self.closed_buffers.contains(&event.buffer) {
            return;
        }
        let result = self.buffers[event.buffer].apply_file_observation(&event);
        if !matches!(result, ObservationApply::Stale { notify: true }) {
            return;
        }
        let shown = event
            .path
            .strip_prefix(&self.project_root)
            .unwrap_or(&event.path)
            .display();
        let body = match &event.observation {
            FileObservation::Text { .. } => {
                format!("{shown} changed on disk · Space b d compares · Space r reloads")
            }
            FileObservation::Deleted => {
                format!("{shown} was deleted on disk · saving recreates it")
            }
            FileObservation::Binary { .. } => {
                format!("{shown} became binary on disk · Runyte kept the text buffer")
            }
            FileObservation::Unreadable { message } => {
                format!("{shown} cannot be read from disk ({message}) · Runyte kept the buffer")
            }
        };
        self.push_notification(NotificationDraft::new(
            NotificationSeverity::Warning,
            "Files",
            "External file change",
            body,
        ));
        if self.finder.is_some() {
            self.rebuild_resource_finder();
        }
    }

    pub fn new(config: Config, file: Option<PathBuf>) -> Result<Self> {
        Self::new_with_targets(config, file.into_iter().map(LaunchTarget::new).collect())
    }

    /// Builds editor state with every requested startup file already open.
    pub fn new_with_targets(config: Config, targets: Vec<LaunchTarget>) -> Result<Self> {
        let launch_directory = std::env::current_dir()?;
        let project_root = project_root::discover(&launch_directory, &config.workspace.state)?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no Git repository or existing project workspace directory found; choose and confirm a project directory"
            )
        })?;
        Self::new_in_project_with_targets(config, targets, project_root)
    }

    /// Builds editor state after startup has discovered or explicitly
    /// confirmed the directory that owns project-scoped runtime data.
    pub fn new_in_project(
        config: Config,
        file: Option<PathBuf>,
        project_root: impl AsRef<Path>,
    ) -> Result<Self> {
        Self::new_in_project_with_targets(
            config,
            file.into_iter().map(LaunchTarget::new).collect(),
            project_root,
        )
    }

    /// Builds editor state with multiple launch targets after project
    /// discovery has completed.
    pub fn new_in_project_with_targets(
        config: Config,
        targets: Vec<LaunchTarget>,
        project_root: impl AsRef<Path>,
    ) -> Result<Self> {
        let mut startup = StartupTrace::new();
        Self::new_in_project_with_targets_and_trace(config, targets, project_root, &mut startup)
    }

    /// Builds editor state while recording optional startup milestones.
    pub fn new_in_project_with_trace(
        config: Config,
        file: Option<PathBuf>,
        project_root: impl AsRef<Path>,
        startup: &mut StartupTrace,
    ) -> Result<Self> {
        Self::new_in_project_with_targets_and_trace(
            config,
            file.into_iter().map(LaunchTarget::new).collect(),
            project_root,
            startup,
        )
    }

    /// Builds editor state for all startup targets while recording optional
    /// milestones. Every open is attempted before this function returns, so
    /// callers can enter the terminal only after launch failures are known.
    pub fn new_in_project_with_targets_and_trace(
        config: Config,
        targets: Vec<LaunchTarget>,
        project_root: impl AsRef<Path>,
        startup: &mut StartupTrace,
    ) -> Result<Self> {
        let project_root = project_root.as_ref().canonicalize()?;
        Self::new_with_boundaries(
            config,
            targets,
            project_root,
            startup,
            std::env::current_dir()?,
            ProgramCache::load(external_open::cache_root()),
            HostPorts::live(),
        )
    }

    /// Deterministic construction seam for the public headless test facade.
    ///
    /// The caller owns an existing isolated root. No project discovery, user
    /// preference/cache lookup, or host clipboard implementation enters this
    /// path.
    pub(crate) fn new_in_isolated_project(
        project_root: impl AsRef<Path>,
        ports: HostPorts,
    ) -> Result<Self> {
        let project_root = project_root.as_ref().canonicalize()?;
        let mut startup = StartupTrace::new();
        Self::new_with_boundaries(
            Config::default(),
            Vec::new(),
            project_root.clone(),
            &mut startup,
            project_root.clone(),
            ProgramCache::default(),
            ports,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_boundaries(
        config: Config,
        targets: Vec<LaunchTarget>,
        project_root: PathBuf,
        startup: &mut StartupTrace,
        working_directory: PathBuf,
        programs: ProgramCache,
        ports: HostPorts,
    ) -> Result<Self> {
        let (theme_name, theme) = config.startup_theme()?;
        startup.mark(StartupPhase::ThemeResolved);
        let state_root = project_root::resolve_state_root(&project_root, &config.workspace.state);
        let reserved_user_roots = [config::default_config_root(), external_open::cache_root()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        project_root::validate_state_root(&state_root, &reserved_user_roots)?;
        let registry = Arc::new(Registry::new());
        startup.mark(StartupPhase::LanguageRegistryReady);
        let OpenedLaunchTargets {
            buffers,
            syntax,
            launch_positions,
            binary_argument,
        } = open_launch_targets(
            targets,
            &working_directory,
            &registry,
            config.editor.show_hidden_files,
        )?;
        startup.mark(StartupPhase::InitialBufferOpened);
        startup.mark(StartupPhase::InitialSyntaxReady);
        let registry_errors = registry.errors();
        let configured_help = ":? or Space+? for help";
        let mut status = startup_status(&registry_errors, configured_help);
        let mut status_error = !registry_errors.is_empty();
        let grammar = match ActiveGrammar::new(config.editor.grammar) {
            Ok(grammar) => grammar,
            Err(error) => {
                status = format!("{status} · {error}; using runyte");
                status_error = true;
                ActiveGrammar::default()
            }
        };
        let initial_mode = grammar.preferred_mode().unwrap_or(Mode::Normal);
        let reported_registry_errors = registry_errors
            .iter()
            .map(|error| (error.language, error.plain))
            .collect();
        let mut panes = HashMap::new();
        panes.insert(0, Pane::new(0));
        let persisted_config = config.clone();
        let notification_limit = config.notifications.history_limit;
        let keymap = keymap_for(config.editor.fast_pane_keys);
        let startup_notification = status_error.then(|| status.clone());
        let mut app = Self {
            config,
            persisted_config,
            theme,
            theme_name,
            config_path: None,
            buffers,
            syntax,
            stale_syntax: HashMap::new(),
            syntax_worker: None,
            registry,
            terminals: TerminalSessions::new(),
            last_terminal: None,
            panes,
            layout: Layout::Pane(0),
            diffs: Vec::new(),
            pending_diff: None,
            active_pane: 0,
            maximized: None,
            mode: initial_mode,
            replace_session: None,
            command: String::new(),
            command_cursor: 0,
            command_selection: 0,
            prompt_kind: PromptKind::Command,
            picker: None,
            finder: None,
            file_scanner: None,
            next_file_scan_id: 1,
            fs_confirmation: None,
            confirmation_revision: 0,
            directory_reload_confirmation: None,
            file_reload_confirmation: None,
            list: None,
            buffer_action_menu: None,
            context_action_menu: None,
            program_action_menu: None,
            path_popup: None,
            path_action_menu: None,
            buffer_discard_confirmation: None,
            git_discard_confirmation: None,
            closed_buffers: HashSet::new(),
            special_buffer_recency: Vec::new(),
            word_index_next_buffer: 0,
            launch_positions,
            completion: None,
            signature: None,
            hover: None,
            diagnostics: DiagnosticStore::default(),
            status,
            status_error,
            notifications: NotificationCenter::new(notification_limit),
            action_feedback: None,
            active_action_id: None,
            next_action_id: 1,
            last_interaction: Instant::now(),
            reported_registry_errors,
            status_revision: 0,
            unavailable_revision: 0,
            should_quit: false,
            persistent_exit_request: None,
            quit_directory: None,
            quit_directory_handoff: false,
            areas: HashMap::new(),
            working_directory,
            home_directory: user_home_directory(),
            path_listings: RefCell::default(),
            external_target: None,
            programs,
            prompt_origin_mode: Mode::Normal,
            prompt_revision: 0,
            project_root,
            state_root,
            git: GitTracker::new(),
            git_state: GitWorkflowState::default(),
            git_branch_deletion: None,
            git_branch_switch: None,
            git_pull_rebase: None,
            git_branch_start: None,
            git_worktree_removal: None,
            #[cfg(unix)]
            pending_worktree_removal: None,
            #[cfg(unix)]
            worktree_teardown: None,
            #[cfg(unix)]
            worktree_removal_generation: 0,
            git_worktree_start: None,
            git_worktree_new_branch: None,
            git_stash_confirmation: None,
            workspace_switch: None,
            persistent_session: false,
            #[cfg(unix)]
            workspace_rows: Vec::new(),
            workspace_number: None,
            #[cfg(unix)]
            workspace_generation: 0,
            #[cfg(unix)]
            workspace_preview_generation: 0,
            #[cfg(unix)]
            workspace_preview_target: None,
            #[cfg(unix)]
            workspace_previews: HashMap::new(),
            #[cfg(unix)]
            session_rename_target: None,
            #[cfg(unix)]
            session_number_target: None,
            session_action_menu: None,
            terminal_action_menu: None,
            commit_origin: None,
            ports,
            registers: HashMap::new(),
            selected_register: '"',
            macros: HashMap::new(),
            recording_macro: None,
            macro_staging: Vec::new(),
            macro_replay: None,
            jump: None,
            line_select: None,
            keymap,
            grammar,
            directory_views: HashMap::new(),
            search: SearchQuery::default(),
            search_selection: None,
            next_pane: 1,
            pane_history_clock: 1,
            pane_opened_at: HashMap::from([(0, 1)]),
            pane_activated_at: HashMap::from([(0, 1)]),
            lsp_servers: HashMap::new(),
            lsp_documents: HashMap::new(),
            lsp_requests: HashMap::new(),
            pending_lsp_replies: VecDeque::new(),
            list_actions: Vec::new(),
            settings_view: None,
            pointer_drag: None,
            lsp_actions: Vec::new(),
            lsp_action_source: None,
            tutorial: None,
            next_lsp_token: 1,
            next_completion_session: 1,
        };
        app.sync_terminal_default_colors();
        if app.grammar.kind() == crate::command::GrammarKind::Vim {
            app.active_mut()
                .mark_selection_semantics(SelectionSemantics::HalfOpen);
        }
        if let Some(message) = startup_notification {
            app.push_notification(NotificationDraft::new(
                NotificationSeverity::Error,
                "Startup",
                "Startup configuration",
                message,
            ));
        }
        app.apply_pending_launch_position(0);
        app.attach_repository();
        if let Some(path) = binary_argument {
            app.ask_for_external_program(path);
        }
        startup.mark(StartupPhase::AppReady);
        Ok(app)
    }

    /// Installs the background word-completion index. Buffers already open
    /// when this is called are picked up by the next `prepare_view` sweep.
    pub fn attach_word_index(&mut self, handle: WordIndexHandle) {
        self.ports.attach_word_index(handle);
    }

    fn word_index_notify_update(&mut self, buffer_id: usize) {
        if let Some(handle) = self.ports.word_index() {
            handle.notify_update(buffer_id, self.buffers[buffer_id].text().clone());
        }
    }

    fn word_index_notify_remove(&mut self, buffer_id: usize) {
        if let Some(handle) = self.ports.word_index() {
            handle.notify_remove(buffer_id);
        }
    }

    /// Indexes every buffer opened since the last call. Buffer ids are
    /// append-only, so this catches every one of the many `self.buffers.push`
    /// call sites (file open, directory adoption, scratch, virtual views,
    /// Git views, commit message, split staging) without a hook at each.
    fn sync_word_index(&mut self) {
        if self.ports.word_index().is_none() {
            // No worker attached yet: leave the cursor where it is rather
            // than marking buffers seen without ever indexing them, so
            // attaching later still catches everything already open.
            return;
        }
        while self.word_index_next_buffer < self.buffers.len() {
            let id = self.word_index_next_buffer;
            self.word_index_next_buffer += 1;
            if self.closed_buffers.contains(&id) || self.buffers[id].is_read_only() {
                continue;
            }
            self.word_index_notify_update(id);
        }
    }
}

/// Formats the `· failed: …` / `· unavailable: …` clause appended to a
/// failed or unavailable action's echo.
///
/// The message is written in full here, untruncated: the echo is composed
/// at command-dispatch time, far from any render pass, and several
/// producers (a Git mutation's asynchronous failure, for one) run with no
/// frame in flight at all, so there is no line width to measure against
/// even in principle. `src/ui.rs`'s `draw_status` truncates the composed
/// line to whatever space the current frame actually has, which is also
/// the one place a multiline message is cut to its first line.
fn outcome_clause(outcome: &str, message: &str) -> String {
    format!(" · {outcome}: {message}")
}

/// What a picker row stands for.
#[derive(Clone, Debug)]
enum ListAction {
    Jump(crate::lsp::Location),
    CodeAction(usize),
    Buffer(usize),
    SettingValue {
        setting: SettingId,
        value: SettingValue,
    },
    SyntaxOutline {
        buffer: usize,
        target: SyntaxSelectionRange,
    },
    Macro(char),
    GitCommit(String),
    Terminal(TerminalId),
    TutorialMotionHints(MotionHints),
    #[cfg(unix)]
    Workspace(usize),
}

/// Whether a keystroke leaves Insert mode instead of reaching a terminal.
///
/// `Escape` cannot be it: `vim` and `htop` inside the pane need it, and every
/// agent uses it too. `Ctrl-\\` begins the staged Normal/review transition;
/// `Ctrl-w` begins the Insert-mode pane-navigation namespace instead.
fn is_terminal_normal_key(key: KeyStroke) -> bool {
    // Two spellings of Ctrl-\\. A terminal implementing the kitty keyboard
    // protocol reports `Ctrl-\` as the character it is; a legacy one has only
    // the control byte `0x1c`, which Crossterm decodes as `Ctrl-4` because
    // that is what the historical table says. Runyte asks for the enhanced
    // protocol but cannot require it — it is off on macOS entirely — so the
    // escape hatch has to answer to both or it would be unreachable on the
    // terminals that need it most.
    key.modifiers.contains(Modifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('\\') | KeyCode::Char('4'))
}

/// Whether a command has no meaning over a live terminal.
///
/// Refusing by category rather than by name so a command added later is
/// refused by default: the failure that matters is an editing command reaching
/// the document hidden behind the terminal, and silence is the worst way for
/// that to happen. Project-wide search is deliberately not refused — it looks
/// past this pane, and its results replace the pane's content anyway.
fn terminal_refuses(command: EditorCommand) -> bool {
    use CommandCategory as Category;

    matches!(
        command,
        EditorCommand::Search
            | EditorCommand::SearchRegex
            | EditorCommand::SearchForward
            | EditorCommand::SearchBackward
            | EditorCommand::SearchNext
            | EditorCommand::SearchPrevious
            | EditorCommand::SearchSelection
    ) || matches!(
        command.category(),
        Category::Editing
            | Category::Movement
            | Category::Selection
            | Category::Syntax
            | Category::Language
            | Category::Register
            | Category::Clipboard
    )
}

/// What a session shows beside its name in the terminal list.
///
/// The end of its output rather than all of it: two shells in the same
/// directory are told apart by what was last run in them, and a picker that
/// rendered five thousand lines to show the last twenty would make opening the
/// list cost more than using it.
fn terminal_preview(session: &TerminalSession) -> String {
    const PREVIEW_LINES: usize = 200;

    let text = session.plain_text();
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(PREVIEW_LINES);
    lines[start..].join("\n")
}

#[cfg(unix)]
/// The manager's right column: what this session is, as a fixed set of fields.
///
/// Every row answers the same questions in the same order, so two sessions can
/// be compared by reading down one place rather than across two sentences. A
/// value nothing can answer is `-`, which is deliberately not the same as `0`:
/// a host that did not reply to the bounded health request may still be
/// holding unsaved work, and a row must not read as clean because it was
/// quiet.
///
/// Buffer and terminal *contents* are not here. They were, and at this width a
/// snippet of a pane is neither readable as text nor useful as identity.
fn session_picker_preview(
    row: &WorkspaceRow,
    preview: Option<&Result<SessionPreview, String>>,
    loading: bool,
    active: &str,
) -> String {
    // A successful health reply populates every live field, including
    // confirmed zeroes. A timed-out reply leaves them unknown, and that is a
    // different answer from zero.
    let health_available = row.unsaved_buffers.is_some()
        && row.open_buffers.is_some()
        && row.pending_wait_requests.is_some()
        && row.live_terminals.is_some()
        && row.terminal_sessions.is_some()
        && row.interactive_attached.is_some();
    let mut status = row.state_label();
    if row.missing_directory {
        status.push_str(" · missing directory");
    }
    if row.running && row.incompatible_protocol.is_none() && !health_available {
        status.push_str(" · health unavailable");
    }

    let terminals = match (row.live_terminals, row.terminal_sessions) {
        (Some(live), Some(total)) => {
            let exited = total.saturating_sub(live);
            if exited > 0 {
                format!("{live} ({exited} exited)")
            } else {
                live.to_string()
            }
        }
        _ => "-".to_owned(),
    };
    let count =
        |value: Option<usize>| value.map_or_else(|| "-".to_owned(), |value| value.to_string());
    let attached = row.interactive_attached.map_or_else(
        || "-".to_owned(),
        |attached| if attached { "yes" } else { "no" }.to_owned(),
    );
    // Panes are the one field a host answers only when asked, because the
    // request that carries them is made for the selected row alone.
    let panes = match (row.running, preview, loading) {
        (false, _, _) => "-".to_owned(),
        (true, Some(Ok(preview)), _) => preview.layout_panes.to_string(),
        (true, Some(Err(_)), _) | (true, None, false) => "-".to_owned(),
        (true, None, true) => "…".to_owned(),
    };
    let git = row.git.as_ref();
    let branch = git
        .and_then(|facts| facts.branch.clone())
        .unwrap_or_else(|| "-".to_owned());
    let directory = crate::git::display_path(&row.project_root);
    let worktree = if git.is_some_and(|facts| facts.worktree.is_some()) {
        "yes".to_owned()
    } else {
        "no".to_owned()
    };
    let remote = git
        .and_then(|facts| facts.remote.clone())
        .unwrap_or_else(|| "-".to_owned());

    let mut lines = vec![format!("Active: {active}")];
    for (field, value) in [
        ("Status", status),
        ("Panes", panes),
        ("Terminals", terminals),
        ("Buffers", count(row.open_buffers)),
        ("Unsaved", count(row.unsaved_buffers)),
        ("Waiting", count(row.pending_wait_requests)),
        ("Attached", attached),
        ("Branch", branch),
        ("Directory", directory),
        ("Worktree", worktree),
        ("Repo", remote),
    ] {
        lines.push(format!("{field:<10}  {value}"));
    }
    if let Some(protocol) = row.incompatible_protocol {
        lines.push(String::new());
        lines.push(format!(
            "This host speaks protocol {protocol}; nothing but stopping it can reach it."
        ));
    } else if !row.running {
        lines.push(String::new());
        lines.push("No live editor state; opening this row starts the session.".to_owned());
    }
    if let Some(Err(error)) = preview {
        lines.push(String::new());
        lines.push(format!("Pane count unavailable: {error}"));
    }
    lines.join("\n")
}

#[cfg(unix)]
/// A short, single-unit age for the session manager and its preview.
///
/// Rounding happens before choosing the next larger unit, so 59 minutes and
/// one second reads `1h ago`, not `60min ago`; the same boundary rule turns
/// 23 hours and one second into `1day ago`.
fn compact_session_elapsed(last_active_unix_seconds: Option<u64>, now: u64) -> String {
    let Some(last_active) = last_active_unix_seconds else {
        return "-".to_owned();
    };
    let elapsed = now.saturating_sub(last_active);
    let minutes = elapsed.div_ceil(60);
    if minutes < 60 {
        return format!("{minutes}min ago");
    }
    let hours = minutes.div_ceil(60);
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours.div_ceil(24);
    let unit = if days == 1 { "day" } else { "days" };
    format!("{days}{unit} ago")
}

/// A bounded view of authoritative buffer text for picker previews.
///
/// This deliberately reads the in-memory buffer rather than its path, so an
/// unsaved file, generated page, scratch buffer, or directory projection is
/// previewed exactly as it would be after opening it.
fn buffer_preview(buffer: &Buffer) -> String {
    const PREVIEW_LINES: usize = 512;
    const PREVIEW_COLUMNS: usize = 512;

    buffer
        .lines()
        .take(PREVIEW_LINES)
        .map(|line| line.chars().take(PREVIEW_COLUMNS).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The program a bare terminal request runs.
fn default_terminal_program() -> OsString {
    #[cfg(unix)]
    {
        crate::terminal::pty::default_shell()
    }
    #[cfg(not(unix))]
    {
        OsString::from("cmd.exe")
    }
}

/// The name a terminal carries until its child sets a title of its own.
fn program_label(program: &OsStr) -> String {
    Path::new(program)
        .file_name()
        .unwrap_or(program)
        .to_string_lossy()
        .into_owned()
}

fn outline_kind_label(kind: OutlineKind) -> &'static str {
    match kind {
        OutlineKind::Module => "module",
        OutlineKind::Type => "type",
        OutlineKind::Class => "class",
        OutlineKind::Struct => "struct",
        OutlineKind::Enum => "enum",
        OutlineKind::Actor => "actor",
        OutlineKind::Extension => "extension",
        OutlineKind::Alias => "alias",
        OutlineKind::Concept => "concept",
        OutlineKind::Interface => "interface",
        OutlineKind::Function => "function",
        OutlineKind::Method => "method",
        OutlineKind::Subscript => "subscript",
        OutlineKind::Property => "property",
        OutlineKind::Constant => "constant",
        OutlineKind::Macro => "macro",
        OutlineKind::Heading => "heading",
    }
}

const OUTLINE_DETAIL_MAX_BYTES: usize = 256;
const OUTLINE_DETAIL_MAX_CELLS: usize = 120;
const OUTLINE_BREADCRUMB_SEPARATOR: &str = " › ";
const OUTLINE_BREADCRUMB_ELLIPSIS: &str = "… › ";

fn outline_item_detail(items: &[OutlineItem], index: usize) -> String {
    let kind = outline_kind_label(items[index].kind);
    let prefix = format!("{kind} · ");
    let byte_budget = OUTLINE_DETAIL_MAX_BYTES.saturating_sub(prefix.len());
    let cell_budget = OUTLINE_DETAIL_MAX_CELLS.saturating_sub(display_cells(&prefix));
    let mut nearest = Vec::new();
    let mut parent = items[index].parent;
    while let Some(parent_index) = parent {
        let parent_item = &items[parent_index];
        nearest.push(parent_item.name.as_ref());
        parent = parent_item.parent;
    }
    if nearest.is_empty() {
        return kind.to_owned();
    }

    // Walk from the nearest parent outward. The nearest names are the useful
    // disambiguators, so distant ancestors are the first thing the bounded
    // row drops. Kept names are reversed only when rendering the conventional
    // root-to-parent breadcrumb.
    let mut kept = Vec::<String>::new();
    let mut used_bytes = 0usize;
    let mut used_cells = 0usize;
    let mut outer_omitted = false;
    for (position, name) in nearest.iter().enumerate() {
        let separator_bytes = usize::from(!kept.is_empty()) * OUTLINE_BREADCRUMB_SEPARATOR.len();
        let separator_cells =
            usize::from(!kept.is_empty()) * display_cells(OUTLINE_BREADCRUMB_SEPARATOR);
        let outer_remains = position + 1 < nearest.len();
        let reserved_bytes = usize::from(outer_remains) * OUTLINE_BREADCRUMB_ELLIPSIS.len();
        let reserved_cells =
            usize::from(outer_remains) * display_cells(OUTLINE_BREADCRUMB_ELLIPSIS);
        let name_cells = display_cells(name);
        if used_bytes
            .saturating_add(separator_bytes)
            .saturating_add(name.len())
            .saturating_add(reserved_bytes)
            <= byte_budget
            && used_cells
                .saturating_add(separator_cells)
                .saturating_add(name_cells)
                .saturating_add(reserved_cells)
                <= cell_budget
        {
            kept.push((*name).to_owned());
            used_bytes += separator_bytes + name.len();
            used_cells += separator_cells + name_cells;
            continue;
        }

        if kept.is_empty() {
            outer_omitted = outer_remains;
            let marker_bytes = usize::from(outer_omitted) * OUTLINE_BREADCRUMB_ELLIPSIS.len();
            let marker_cells =
                usize::from(outer_omitted) * display_cells(OUTLINE_BREADCRUMB_ELLIPSIS);
            kept.push(bounded_outline_component(
                name,
                byte_budget.saturating_sub(marker_bytes),
                cell_budget.saturating_sub(marker_cells),
            ));
        } else {
            outer_omitted = true;
        }
        break;
    }
    outer_omitted |= kept.len() < nearest.len();
    kept.reverse();
    let breadcrumb = kept.join(OUTLINE_BREADCRUMB_SEPARATOR);
    let marker = if outer_omitted {
        OUTLINE_BREADCRUMB_ELLIPSIS
    } else {
        ""
    };
    let detail = format!("{prefix}{marker}{breadcrumb}");
    debug_assert!(detail.len() <= OUTLINE_DETAIL_MAX_BYTES);
    debug_assert!(display_cells(&detail) <= OUTLINE_DETAIL_MAX_CELLS);
    detail
}

fn bounded_outline_component(value: &str, max_bytes: usize, max_cells: usize) -> String {
    if value.len() <= max_bytes && display_cells(value) <= max_cells {
        return value.to_owned();
    }
    const ELLIPSIS: &str = "…";
    let byte_budget = max_bytes.saturating_sub(ELLIPSIS.len());
    let cell_budget = max_cells.saturating_sub(display_cells(ELLIPSIS));
    let mut bounded = String::new();
    let mut cells = 0usize;
    for character in value.chars() {
        let width = UnicodeWidthChar::width(character).unwrap_or(0).max(1);
        if bounded.len() + character.len_utf8() > byte_budget || cells + width > cell_budget {
            break;
        }
        bounded.push(character);
        cells += width;
    }
    if max_bytes >= ELLIPSIS.len() && max_cells >= display_cells(ELLIPSIS) {
        bounded.push_str(ELLIPSIS);
    }
    bounded
}

fn display_cells(value: &str) -> usize {
    value
        .chars()
        .map(|character| UnicodeWidthChar::width(character).unwrap_or(0).max(1))
        .sum()
}

/// Terminal cells before a character boundary, including tab expansion.
fn visual_column(line: &str, column: usize, tab_width: usize) -> usize {
    let tab_width = tab_width.max(1);
    line.chars().take(column).fold(0, |cell, character| {
        cell + if character == '\t' {
            tab_width - cell % tab_width
        } else {
            UnicodeWidthChar::width(character).unwrap_or(0).max(1)
        }
    })
}

/// Finds the character that occupies a display column. Callers pad a short
/// line first, so the target always has a character to land on.
fn column_at_visual_column(line: &str, target: usize, tab_width: usize) -> usize {
    let mut cell = 0;
    for (column, character) in line.chars().enumerate() {
        let next = cell
            + if character == '\t' {
                tab_width.max(1) - cell % tab_width.max(1)
            } else {
                UnicodeWidthChar::width(character).unwrap_or(0).max(1)
            };
        if target < next {
            return column;
        }
        cell = next;
    }
    line.chars().count().saturating_sub(1)
}

fn outline_status(outline: &Outline) -> Option<String> {
    match (!outline.issues.is_empty(), outline.truncated) {
        (false, false) => None,
        (true, false) => Some(format!(
            "document outline is degraded ({} syntax issue{})",
            outline.issues.len(),
            if outline.issues.len() == 1 { "" } else { "s" }
        )),
        (false, true) => Some("document outline is truncated".to_owned()),
        (true, true) => Some(format!(
            "document outline is degraded ({} syntax issue{}) and truncated",
            outline.issues.len(),
            if outline.issues.len() == 1 { "" } else { "s" }
        )),
    }
}

impl PendingRequest {
    fn label(&self) -> &'static str {
        match self {
            Self::Goto { label } | Self::Edits { label, .. } => label,
            Self::Hover => "documentation",
            Self::Completion { .. } => "completion",
            Self::Signature => "signature help",
            Self::Symbols { title, .. } => title,
            Self::CodeActions => "code actions",
        }
    }
}

fn response_name(response: &Response) -> &'static str {
    match response {
        Response::Locations(_) => "location",
        Response::Hover(_) => "hover",
        Response::Completions(_) => "completion",
        Response::Signatures(_) => "signature",
        Response::Symbols(_) => "symbol",
        Response::Actions(_) => "code action",
        Response::Edits { .. } | Response::ActionEdits { .. } => "edit",
        Response::Empty => "empty",
        Response::Failed(_) => "failed",
    }
}

fn edit_summary(
    label: &str,
    (files, edits, _synchronized): (usize, usize, bool),
    skipped: usize,
) -> String {
    let mut message = format!(
        "{label} {edits} change{} in {files} file{}",
        if edits == 1 { "" } else { "s" },
        if files == 1 { "" } else { "s" }
    );
    if skipped > 0 {
        // Creating, renaming, and deleting files is a filesystem mutation, and
        // V4 keeps those behind an explicit plan rather than a language
        // server's say-so.
        message.push_str(&format!(
            " · {skipped} file operation{} not performed",
            if skipped == 1 { "" } else { "s" }
        ));
    }
    message
}

/// A path shortened against the working directory, for picker rows.
fn display_path(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The two columns shown by the open-buffer manager.
///
/// Files and explorers lead with the final path component so similarly named
/// buffers line up, while the second column retains their complete identity.
/// Generated and pathless buffers already carry structural names and have no
/// second-column path to show.
fn buffer_picker_columns(buffer: &Buffer, project_root: &Path, active: bool) -> (String, String) {
    let (mut label, detail) = match &buffer.kind {
        BufferKind::File | BufferKind::Directory => {
            let label = buffer.path.as_deref().map_or_else(
                || buffer.display_name(),
                |path| {
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                },
            );
            let detail = buffer.path.as_deref().map_or_else(String::new, |path| {
                let relative = path.strip_prefix(project_root).unwrap_or(path);
                if relative.as_os_str().is_empty() {
                    ".".to_owned()
                } else {
                    relative.display().to_string()
                }
            });
            let label = if buffer.is_directory() {
                format!("[explorer] {label}")
            } else {
                label
            };
            (label, detail)
        }
        _ => (buffer.display_name(), String::new()),
    };
    if active {
        label = format!("*{label}*");
    }
    if buffer.external_file_status().is_stale() {
        label.push_str(" [STALE]");
    }
    if buffer.is_read_only() {
        label.push_str(" [RO]");
    }
    (label, detail)
}

/// Equivalent spellings of a resource path. The picker keeps these as
/// separate fields so every query term can match the representation a person
/// naturally types without making the displayed row repeat the same path.
fn resource_path_fields(path: &Path, project_root: &Path, home: Option<&Path>) -> Vec<String> {
    let mut fields = vec![path.display().to_string()];
    if let Ok(relative) = path.strip_prefix(project_root) {
        fields.push(if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            relative.display().to_string()
        });
    }
    if let Some(home) = home
        && let Ok(relative) = path.strip_prefix(home)
    {
        fields.push(if relative.as_os_str().is_empty() {
            "~".to_owned()
        } else {
            format!("~/{}", relative.display())
        });
    }
    if let Some(name) = path.file_name() {
        fields.push(name.to_string_lossy().into_owned());
    }
    fields
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceMatch {
    path: PathBuf,
    row: usize,
    column: usize,
    length: usize,
    preview: String,
}

const GLOBAL_SEARCH_FILE_LIMIT: u64 = 4 * 1024 * 1024;
const GLOBAL_SEARCH_RESULT_LIMIT: usize = 10_000;

/// Every match of `query` in `buffer`, in buffer order, as selection ranges
/// whose head sits on the match's last character.
///
/// Matching runs over the whole buffer text rather than line by line so a
/// regular expression can span lines. Ranges point forward — anchor on the
/// first character, head on the last — so the caret lands where typing
/// continues from rather than where the match began.
///
/// `region`, when present, confines matches to those half-open spans: it is how
/// a search started inside a selection stays inside it, including when `n`
/// wraps.
fn buffer_matches(
    buffer: &Buffer,
    pattern: &str,
    mode: SearchMode,
    region: Option<&[(Offset, Offset)]>,
) -> Result<Vec<Range>, regex::Error> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let matcher = mode.compile(pattern)?;
    let text = buffer.to_string();
    let mut ranges = Vec::new();
    // Matches arrive in order and never overlap, so one running byte-to-char
    // cursor replaces re-counting the prefix for every match.
    let mut byte_cursor = 0;
    let mut char_cursor = 0;
    for found in matcher.find_iter(&text) {
        char_cursor += text[byte_cursor..found.start()].chars().count();
        byte_cursor = found.start();
        let start = char_cursor;
        let end = start + text[found.start()..found.end()].chars().count();
        if let Some(spans) = region
            && !spans.iter().any(|(from, to)| *from <= start && end <= *to)
        {
            continue;
        }
        ranges.push(if end == start {
            Range::point(start)
        } else {
            Range::new(start, end - 1)
        });
    }
    Ok(ranges)
}

fn workspace_matches(
    root: &Path,
    matcher: &Regex,
    show_hidden: bool,
) -> Result<(Vec<WorkspaceMatch>, bool)> {
    let mut pending = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut entries: Vec<_> =
            std::fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if matches!(name.as_ref(), ".git" | ".runyte" | "target")
                    || (!show_hidden && name.starts_with('.'))
                {
                    continue;
                }
                pending.push(entry.path());
                continue;
            }
            if !file_type.is_file()
                || (!show_hidden && name.starts_with('.'))
                || entry.metadata()?.len() > GLOBAL_SEARCH_FILE_LIMIT
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            matches.extend(matches_in_text(&entry.path(), &text, matcher));
            if matches.len() >= GLOBAL_SEARCH_RESULT_LIMIT {
                matches.truncate(GLOBAL_SEARCH_RESULT_LIMIT);
                return Ok((matches, true));
            }
        }
    }
    Ok((matches, false))
}

/// Workspace results stay line-scoped: a picker row names one line, so a match
/// that spanned several of them would have no row to be reported on.
fn matches_in_text(path: &Path, text: &str, matcher: &Regex) -> Vec<WorkspaceMatch> {
    let mut matches = Vec::new();
    for (row, line) in text.lines().enumerate() {
        for found in matcher.find_iter(line) {
            matches.push(WorkspaceMatch {
                path: path.to_path_buf(),
                row,
                column: line[..found.start()].chars().count(),
                length: found.as_str().chars().count(),
                preview: line.trim().chars().take(240).collect(),
            });
        }
    }
    matches
}

const fn syntax_object_label(object: SyntaxObject) -> &'static str {
    match object {
        SyntaxObject::Function => "function",
        SyntaxObject::Class => "class",
        SyntaxObject::Parameter => "parameter",
        SyntaxObject::Section => "section",
        SyntaxObject::Paragraph => "paragraph",
    }
}

const fn syntax_object_part_label(part: SyntaxObjectPart) -> &'static str {
    match part {
        SyntaxObjectPart::Around => "around",
        SyntaxObjectPart::Inside => "inside",
    }
}

fn resolved_operation_path(root: &Path, path: &Path) -> PathBuf {
    let joined = root.join(path);
    let Some(parent) = joined.parent() else {
        return joined;
    };
    let Ok(parent) = fs::canonicalize(parent) else {
        return joined;
    };
    joined
        .file_name()
        .map_or_else(|| parent.clone(), |name| parent.join(name))
}

fn mapped_applied_path(root: &Path, path: &Path, operations: &[FsOperation]) -> Option<PathBuf> {
    let mut mapped = path.to_path_buf();
    let mut changed = false;
    for operation in operations {
        let (from, to, kind) = match operation {
            FsOperation::Rename { from, to, kind } | FsOperation::Move { from, to, kind } => (
                resolved_operation_path(root, from),
                resolved_operation_path(root, to),
                *kind,
            ),
            FsOperation::Create { .. } | FsOperation::Copy { .. } | FsOperation::Delete { .. } => {
                continue;
            }
        };
        if mapped == from {
            mapped = to;
            changed = true;
        } else if kind == EntryKind::Directory
            && let Ok(suffix) = mapped.strip_prefix(&from)
        {
            mapped = to.join(suffix);
            changed = true;
        }
    }
    changed.then_some(mapped)
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn pointer_pane(view: &PreparedView, column: u16, row: u16) -> Option<usize> {
    view.panes
        .iter()
        .find(|pane| rect_contains(pane.area, column, row))
        .map(|pane| pane.pane_id)
}

fn pointer_resize_pair(view: &PreparedView, column: u16, row: u16) -> Option<(usize, usize, Axis)> {
    for (index, first) in view.panes.iter().enumerate() {
        for second in view.panes.iter().skip(index + 1) {
            let first_right = first.area.x.saturating_add(first.area.width);
            let second_right = second.area.x.saturating_add(second.area.width);
            let first_bottom = first.area.y.saturating_add(first.area.height);
            let second_bottom = second.area.y.saturating_add(second.area.height);
            let vertical_overlap = first.area.y < second_bottom && second.area.y < first_bottom;
            let horizontal_overlap = first.area.x < second_right && second.area.x < first_right;
            let row_is_on_shared_edge =
                row >= first.area.y.max(second.area.y) && row < first_bottom.min(second_bottom);
            let column_is_on_shared_edge =
                column >= first.area.x.max(second.area.x) && column < first_right.min(second_right);

            if vertical_overlap && row_is_on_shared_edge && first_right == second.area.x {
                if column == second.area.x || column.saturating_add(1) == second.area.x {
                    return Some((first.pane_id, second.pane_id, Axis::Horizontal));
                }
            } else if vertical_overlap
                && row_is_on_shared_edge
                && second_right == first.area.x
                && (column == first.area.x || column.saturating_add(1) == first.area.x)
            {
                return Some((second.pane_id, first.pane_id, Axis::Horizontal));
            }
            if horizontal_overlap && column_is_on_shared_edge && first_bottom == second.area.y {
                if row == second.area.y || row.saturating_add(1) == second.area.y {
                    return Some((first.pane_id, second.pane_id, Axis::Vertical));
                }
            } else if horizontal_overlap
                && column_is_on_shared_edge
                && second_bottom == first.area.y
                && (row == first.area.y || row.saturating_add(1) == first.area.y)
            {
                return Some((second.pane_id, first.pane_id, Axis::Vertical));
            }
        }
    }
    None
}

fn enclosing_area(rectangles: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    let mut rectangles = rectangles.into_iter();
    let first = rectangles.next()?;
    let (left, top, right, bottom) = rectangles.fold(
        (
            first.x,
            first.y,
            first.x.saturating_add(first.width),
            first.y.saturating_add(first.height),
        ),
        |(left, top, right, bottom), rectangle| {
            (
                left.min(rectangle.x),
                top.min(rectangle.y),
                right.max(rectangle.x.saturating_add(rectangle.width)),
                bottom.max(rectangle.y.saturating_add(rectangle.height)),
            )
        },
    );
    Some(Rect {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    })
}

fn absolute(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// The person's home directory, or `None` where the environment does not name
/// one. Public because the launch path needs the same answer the editor uses
/// before an `App` exists.
pub fn user_home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from)
        })
}

fn expand_home_path(path: PathBuf, home: Option<&Path>) -> PathBuf {
    let Some(home) = home else {
        return path;
    };
    if path == Path::new("~") {
        return home.to_path_buf();
    }
    path.strip_prefix("~")
        .map_or(path.clone(), |remainder| home.join(remainder))
}

fn unclosed_or_complete_quoted_path(argument: &str) -> &str {
    let Some(quote @ ('\'' | '"')) = argument.chars().next() else {
        return argument;
    };
    let inner = &argument[quote.len_utf8()..];
    inner.strip_suffix(quote).unwrap_or(inner)
}

fn is_path_separator(character: char) -> bool {
    character == std::path::MAIN_SEPARATOR || cfg!(windows) && character == '/'
}

fn quote_path_hint(value: &str, preferred_quote: Option<char>, directory: bool) -> (String, bool) {
    let quote = preferred_quote.or_else(|| value.chars().any(char::is_whitespace).then_some('"'));
    let Some(quote) = quote else {
        return (value.to_owned(), false);
    };
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character == quote || character == '\\' {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    (format!("{quote}{escaped}{quote}"), directory)
}

fn open_or_new(path: &Path, show_hidden: bool) -> Result<Buffer> {
    let path = absolute(path.to_path_buf())?;
    if path.is_dir() {
        Buffer::open_directory(&path, show_hidden)
    } else if path.exists() {
        Buffer::open(&path)
    } else {
        let mut buffer = Buffer::scratch();
        buffer.path = Some(path);
        buffer.kind = crate::buffer::BufferKind::File;
        Ok(buffer)
    }
}

fn open_or_new_at_identity(
    path: &Path,
    expected_identity: &Path,
    show_hidden: bool,
) -> Result<Buffer> {
    let buffer = open_or_new(path, show_hidden)?;
    ensure!(
        crate::path_safety::path_identity(path)?.as_path() == expected_identity,
        "{} changed its resolved identity while it was being opened; retry the open",
        path.display()
    );
    Ok(buffer)
}

fn workspace_edit_path_identity(path: &Path) -> Result<PathBuf> {
    let resolved = crate::path_safety::canonicalize_existing_prefix(path)?;
    let mut normalized = PathBuf::new();
    for component in resolved.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

/// Opens and parses every distinct text target before terminal entry.
///
/// Existing paths are canonicalized for identity. Nonexistent absolute paths
/// retain their exact component sequence so symlink-sensitive `..` traversal
/// keeps its filesystem meaning. Binary targets retain the historical
/// external-program prompt and do not become text buffers.
struct OpenedLaunchTargets {
    buffers: Vec<Buffer>,
    syntax: Vec<Option<DocumentSyntax>>,
    launch_positions: HashMap<usize, LaunchPosition>,
    binary_argument: Option<PathBuf>,
}

fn open_launch_targets(
    targets: Vec<LaunchTarget>,
    working_directory: &Path,
    registry: &Registry,
    show_hidden: bool,
) -> Result<OpenedLaunchTargets> {
    let mut buffers = Vec::new();
    let mut syntax = Vec::new();
    let mut launch_positions = HashMap::new();
    let mut buffer_by_path = HashMap::new();
    let mut seen_paths = HashSet::new();
    let mut binary_argument = None;

    for target in targets {
        let path = resolve_launch_path(target.path, working_directory);
        let identity = crate::path_safety::path_identity(&path)?;
        if let Some(buffer) = buffer_by_path.get(&identity).copied() {
            if let Some(position) = target.position {
                launch_positions.entry(buffer).or_insert(position);
            }
            continue;
        }
        if !seen_paths.insert(identity.clone()) {
            continue;
        }
        if external_open::looks_binary(&path) {
            ensure!(
                binary_argument.is_none(),
                "multiple binary startup targets are not supported; open binary files one at a time"
            );
            binary_argument = Some(path);
            continue;
        }

        let buffer = match open_or_new_at_identity(&path, &identity, show_hidden) {
            Ok(buffer) => buffer,
            Err(error) if error.is::<BinaryFileError>() => {
                ensure!(
                    binary_argument.is_none(),
                    "multiple binary startup targets are not supported; open binary files one at a time"
                );
                binary_argument = Some(path);
                continue;
            }
            Err(error) => return Err(error),
        };
        let buffer_id = buffers.len();
        if let Some(position) = target.position {
            launch_positions.insert(buffer_id, position);
        }
        syntax.push(parse_buffer(&buffer, registry));
        buffers.push(buffer);
        buffer_by_path.insert(identity, buffer_id);
    }

    if buffers.is_empty() {
        let scratch = Buffer::scratch();
        syntax.push(parse_buffer(&scratch, registry));
        buffers.push(scratch);
    }
    Ok(OpenedLaunchTargets {
        buffers,
        syntax,
        launch_positions,
        binary_argument,
    })
}

fn resolve_launch_path(path: PathBuf, working_directory: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path
    } else {
        working_directory.join(path)
    };
    if let Ok(canonical) = absolute.canonicalize() {
        return canonical;
    }

    absolute
}

fn selection_for_launch_position(buffer: &Buffer, position: LaunchPosition) -> Selection {
    let row = position.line.get().saturating_sub(1).min(buffer.last_row());
    let col = position
        .column
        .map_or(0, |column| column.get().saturating_sub(1));
    let position = buffer.clamp(Position::new(row, col), false);
    Selection::point(buffer.offset_of(position))
}

/// Resolves the one syntax/LSP language identity owned by a buffer.
fn buffer_language(buffer: &Buffer, registry: &Registry) -> Option<LanguageId> {
    if buffer.is_read_only() || buffer.is_directory() {
        return None;
    }
    registry.language_for_document(buffer.path.as_deref(), buffer.text())
}

/// Parses a buffer if its path or bounded first-line metadata maps to a known
/// language.
///
/// Returns `None` for unknown documents, read-only virtual buffers, directory
/// projections, and failed parses — all of which render as plain text.
fn parse_buffer(buffer: &Buffer, registry: &Registry) -> Option<DocumentSyntax> {
    let language = buffer_language(buffer, registry)?;
    DocumentSyntax::new(buffer.text(), language, registry)
}

fn startup_status(errors: &[RegistryError], help: &str) -> String {
    if errors.is_empty() {
        help.to_owned()
    } else {
        format!("{help} │ {}", registry_failure_summary(errors))
    }
}

fn registry_failure_summary(errors: &[RegistryError]) -> String {
    format!(
        "{} grammar(s) unavailable: {}",
        errors.len(),
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    )
}

fn fold_degradation_suffix(issue_count: usize, truncated: bool) -> String {
    match (issue_count, truncated) {
        (0, false) => String::new(),
        (0, true) => " · partial (fold limit reached)".to_owned(),
        (issues, false) => format!(" · degraded ({issues} syntax issue(s))"),
        (issues, true) => {
            format!(" · partial ({issues} syntax issue(s); fold limit reached)")
        }
    }
}

#[cfg(test)]
mod tests;
