// SPDX-License-Identifier: MPL-2.0

//! Owned, presentation-neutral snapshots of the normal editor surface.
//!
//! A snapshot contains only the rows that belong to the prepared viewport.
//! Frontends receive semantic runs and choose their own colors, borders, and
//! widgets; they never need to inspect buffers, selections, or diagnostics.

use std::path::PathBuf;

use unicode_width::UnicodeWidthChar;

use crate::{
    app::{App, MaximizedView, Mode, PreparedPane, PreparedView, PromptKind},
    buffer::{Buffer, BufferKind, ExternalFileStatus, GeneratedViewIdentity, Position},
    command::EditorCommand,
    config::Theme,
    diff::Change,
    git::{CountKind, DiffLine, LineChange},
    jump_labels::LabelPart,
    layout::Rect,
    lsp::Severity,
    notification::{NotificationCounts, NotificationSeverity},
    row_hints::RowHints,
    selection::Range,
    syntax::{Scope, Span},
    terminal::TerminalView,
    text::Offset,
    wrap::Segment,
};

const ZERO_WIDTH_SCAN_LIMIT: usize = 256;

/// Walk the normalized ranges in document order while a viewport row is drawn.
/// Both heads and covered spans are ordered, including reversed ranges. A
/// structured table can revisit a document offset, so those lookups reset at
/// that offset rather than assuming the visual order is always monotonic.
struct SelectionRoles<'a> {
    ranges: &'a [Range],
    primary: usize,
    covered: usize,
    head: usize,
    last_offset: Option<Offset>,
    select_mode: bool,
    pristine_search: bool,
    replacing: bool,
    half_open: bool,
    runyte: bool,
    /// Whether a lone range is drawn in Normal mode. Selecting motions leave
    /// one there for `d`, `c`, and `p` to act on, so it has to be seen.
    lone_range_shown: bool,
}

impl<'a> SelectionRoles<'a> {
    fn new(
        ranges: &'a [Range],
        primary: usize,
        first_offset: Offset,
        select_mode: bool,
        pristine_search: bool,
        replacing: bool,
        semantics: crate::jumplist::SelectionSemantics,
    ) -> Self {
        let half_open = matches!(
            semantics,
            crate::jumplist::SelectionSemantics::HalfOpen
                | crate::jumplist::SelectionSemantics::VimLinewise
        );
        Self {
            ranges,
            primary,
            covered: ranges.partition_point(|range| {
                if half_open {
                    range.to() <= first_offset
                } else {
                    range.to() < first_offset
                }
            }),
            head: ranges.partition_point(|range| range.head < first_offset),
            last_offset: None,
            select_mode,
            pristine_search,
            replacing,
            half_open,
            runyte: semantics == crate::jumplist::SelectionSemantics::Runyte,
            lone_range_shown: false,
        }
    }

    fn with_lone_range_shown(mut self, shown: bool) -> Self {
        self.lone_range_shown = shown;
        self
    }

    fn role_at(&mut self, offset: Offset) -> TextRole {
        if self.last_offset.is_some_and(|last| offset < last) {
            self.covered = self.ranges.partition_point(|range| {
                if self.half_open {
                    range.to() <= offset
                } else {
                    range.to() < offset
                }
            });
            self.head = self.ranges.partition_point(|range| range.head < offset);
        } else {
            while self.covered < self.ranges.len()
                && if self.half_open {
                    self.ranges[self.covered].to() <= offset
                } else {
                    self.ranges[self.covered].to() < offset
                }
            {
                self.covered += 1;
            }
            while self.head < self.ranges.len() && self.ranges[self.head].head < offset {
                self.head += 1;
            }
        }
        self.last_offset = Some(offset);
        let head = self
            .ranges
            .get(self.head)
            .is_some_and(|range| range.head == offset);
        if head && self.replacing {
            return if self.ranges[self.primary].head == offset {
                TextRole::ReplaceCaret
            } else {
                TextRole::SecondaryCaret
            };
        }
        if self.select_mode && self.ranges[self.primary].head == offset {
            return TextRole::PrimaryCaret;
        }
        if head && !self.pristine_search {
            return if self.ranges[self.primary].head == offset {
                TextRole::Caret
            } else {
                TextRole::SecondaryCaret
            };
        }
        if head && self.pristine_search && self.runyte && self.ranges[self.head].is_empty() {
            return TextRole::Selected;
        }
        let Some(range) = self.ranges.get(self.covered) else {
            return TextRole::Plain;
        };
        if range.is_empty() || offset < range.from() {
            return TextRole::Plain;
        }
        if self.select_mode && self.covered == self.primary {
            return TextRole::PrimarySelected;
        }
        if self.covered == self.primary
            && !self.select_mode
            && self.ranges.len() == 1
            && self.runyte
            && !self.lone_range_shown
        {
            return TextRole::Plain;
        }
        TextRole::Selected
    }
}

/// Everything needed to draw the normal editor panes and status surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorSnapshot {
    pub geometry: crate::app::FrameGeometry,
    pub theme: Theme,
    pub mode: Mode,
    pub directory_tree: Option<DirectoryTreeSnapshot>,
    pub panes: Vec<PaneSnapshot>,
    pub status: StatusSnapshot,
    pub session_strip: Option<SessionStripSnapshot>,
}

/// A bounded viewport of the sidebar, with real paths kept apart from labels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryTreeSnapshot {
    pub jump_active: bool,
    pub area: Rect,
    pub focused: bool,
    pub title: String,
    pub total_rows: usize,
    pub selected: Option<usize>,
    pub rows: Vec<DirectoryTreeRowSnapshot>,
    pub legend: Vec<String>,
}

/// Keep tree frame payloads bounded even on unusually tall terminals.
pub const MAX_DIRECTORY_TREE_SNAPSHOT_ROWS: usize = 4096;
pub const MAX_DIRECTORY_TREE_ROW_TEXT_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryTreeRowSnapshot {
    pub jump_label: [Option<(char, LabelPart)>; 2],
    pub path: PathBuf,
    pub label: String,
    pub depth: usize,
    pub kind: crate::fs_plan::EntryKind,
    pub expanded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: bool,
}

fn escape_tree_controls(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        let rendered = if character.is_control() {
            character.escape_default().collect::<String>()
        } else {
            character.to_string()
        };
        if escaped.len() + rendered.len() > MAX_DIRECTORY_TREE_ROW_TEXT_BYTES {
            while escaped.len() > MAX_DIRECTORY_TREE_ROW_TEXT_BYTES - '…'.len_utf8() {
                escaped.pop();
            }
            escaped.push('…');
            return escaped;
        }
        escaped.push_str(&rendered);
    }
    escaped
}

/// Running persistent sessions in catalog order; this contains no remote text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStripSnapshot {
    pub entries: Vec<SessionStripEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStripEntry {
    pub name: String,
    pub number: Option<u8>,
    pub current: bool,
    pub health_unknown: bool,
    pub unread: bool,
    pub bell: bool,
}

/// An owned, presentation-neutral description of an application overlay.
///
/// Overlay snapshots deliberately use a small common row shape. Frontends
/// decide borders, dimensions, and colours; no frontend needs to borrow
/// picker, prompt, confirmation, or language-server state from [`App`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlaySnapshot {
    pub kind: OverlayKind,
    /// Semantic role. Frontends must not infer behavior from `title` or
    /// `kind` display text.
    pub purpose: OverlayPurpose,
    pub input: OverlayInput,
    pub layout: OverlayLayout,
    /// Ordered user-visible actions. The first action is primary when one
    /// exists; reports deliberately have only navigation/dismiss actions.
    pub actions: Vec<OverlayAction>,
    /// Keys that act on the surface in its current state, beyond the ones
    /// `actions` names in the title. Frontends draw them as a dimmed legend
    /// pinned to the bottom of the overlay, and drop it before any row.
    pub legend: Vec<OverlayAction>,
    pub title: String,
    pub query: String,
    /// What the query line reads while the query is empty. Frontends draw it
    /// muted in place of the query, so a surface that owns its input keeps
    /// the same shape before and after its first character. Empty for a
    /// surface that has nothing to suggest.
    pub query_placeholder: String,
    /// Optional non-selectable labels for the row columns.
    pub column_header: Option<OverlayColumnHeader>,
    pub rows: Vec<OverlayRow>,
    pub selected: Option<usize>,
    /// Full-result row kept visible even when the surface has no actionable
    /// selection (for example an informational report).
    pub scroll_anchor: Option<usize>,
    /// Index in the full result set represented by `rows[0]`.
    pub row_offset: usize,
    pub message: Option<String>,
    /// Number of rows intentionally left out of this bounded snapshot.
    pub omitted_rows: usize,
    pub total_rows: usize,
    pub query_cursor: Option<usize>,
    pub show_preview: bool,
    pub preview_title: Option<String>,
    pub preview: Option<OverlayPreview>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayColumnHeader {
    pub label: String,
    pub detail: String,
    /// A short final label run preserved under the same clipping rules as the
    /// rows' trailing detail.
    pub trailing_detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayPurpose {
    Picker,
    Choice,
    Manager,
    Report,
    Confirmation,
    CommandPalette,
    Context,
    Input,
    /// Read-only information text with no warning or error connotation, e.g.
    /// `:path`'s popup.
    Info,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayInput {
    None,
    Filter,
    Text,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayLayout {
    GitMergeReview,
    Standard,
    Preview,
    /// A single typed setting value.
    Setting,
    /// A setting's choices, which need room for rows the typed prompt has no
    /// use for.
    SettingChoice,
    Anchored,
    Bottom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayAction {
    pub key_hint: String,
    pub label: String,
}

impl OverlayAction {
    pub fn new(key_hint: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key_hint: key_hint.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayKind {
    GitMergeReview,
    FilesystemConfirmation,
    FilePicker,
    ResultList,
    BufferActions,
    Confirmation,
    CommandPalette,
    ProgramHints,
    ProgramActions,
    Path,
    PathActions,
    /// Bounded path assistance attached to a completing prompt. The
    /// interaction line below it owns the typed value, so this overlay
    /// carries no query of its own and is anchored rather than centred.
    PathCompletion,
    Prompt,
    Completion,
    Signature,
    Hover,
    KeyHints,
}

impl OverlayKind {
    /// Exhaustive producer inventory used by contract tests. Adding an
    /// overlay kind therefore requires classifying it deliberately.
    pub const ALL: &'static [Self] = &[
        Self::GitMergeReview,
        Self::FilesystemConfirmation,
        Self::FilePicker,
        Self::ResultList,
        Self::BufferActions,
        Self::Confirmation,
        Self::CommandPalette,
        Self::ProgramHints,
        Self::ProgramActions,
        Self::Path,
        Self::PathActions,
        Self::PathCompletion,
        Self::Prompt,
        Self::Completion,
        Self::Signature,
        Self::Hover,
        Self::KeyHints,
    ];
}

/// What a coloured run of a row says. Frontends choose the colour; the
/// snapshot names only the meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowTint {
    /// A destination's type cell, by kind.
    File,
    Explorer,
    Generated,
    Scratch,
    Terminal,
    /// A state flag.
    Modified,
    Stale,
    ReadOnly,
    Exited,
    Unread,
    Bell,
}

/// `len` characters of a row's label, or of its trailing detail when
/// `trailing` is set, starting at character `start`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TintedRun {
    pub trailing: bool,
    pub start: usize,
    pub len: usize,
    pub tint: RowTint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayRow {
    /// Non-selectable section heading supplied by a grouped picker.
    pub heading: bool,
    pub identity: OverlayIdentity,
    pub label: String,
    pub detail: String,
    /// A short final run frontends preserve when clipping an overlong row.
    pub trailing_detail: String,
    /// Whether this row's contextual capability is currently available.
    /// Frontends retain unavailable rows for discovery but render them with
    /// reduced emphasis.
    pub available: bool,
    /// Whether what the row stands for is dormant rather than unavailable.
    /// A dimmed row is still selectable and still acts when chosen: a stopped
    /// session starts, it does not refuse. Frontends draw it in the same
    /// dimmed text colour a command prompt dims the panes behind it with, so
    /// the reader separates the rows that are doing something from the ones
    /// that are merely known.
    pub dimmed: bool,
    /// Character positions that carry the row's secondary, muted emphasis,
    /// such as the category prefix of a command-palette entry. This is
    /// distinct from `dimmed`: these characters stay muted even while the
    /// row is selected, while a dimmed row as a whole becomes legible at its
    /// selected position.
    pub muted: Vec<usize>,
    /// Character positions a frontend may emphasize (for fuzzy matches or an
    /// active signature parameter, or the available command in a categorized
    /// palette row).
    pub emphasis: Vec<usize>,
    /// Character positions emphasized in the separate detail column.
    pub detail_emphasis: Vec<usize>,
    /// Runs of the label or trailing detail coloured for what they say.
    pub tints: Vec<TintedRun>,
    /// Character position in `label` from which a label too wide for its row
    /// is shortened in the middle rather than cut at its end, so the end of a
    /// path — the file name — stays readable.
    pub elide_from: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverlayPreview {
    Terminal(TerminalView),
    Text(Vec<String>),
    MatchedText {
        lines: Vec<String>,
        /// Character positions in the newline-joined preview text.
        emphasis: Vec<usize>,
    },
    Snippet {
        lines: Vec<String>,
        start_row: usize,
        focus_row: usize,
        emphasis: Vec<usize>,
    },
    Binary,
    Unavailable(String),
    Empty,
}

/// Stable row identity. Paths stay encoded as paths rather than being
/// reconstructed from their potentially lossy display form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverlayIdentity {
    Text(String),
    Path(PathBuf),
    Index(usize),
}

impl From<String> for OverlayIdentity {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for OverlayIdentity {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<PathBuf> for OverlayIdentity {
    fn from(value: PathBuf) -> Self {
        Self::Path(value)
    }
}

impl From<usize> for OverlayIdentity {
    fn from(value: usize) -> Self {
        Self::Index(value)
    }
}

impl EditorSnapshot {
    pub fn pane(&self, pane_id: usize) -> Option<&PaneSnapshot> {
        self.panes.iter().find(|pane| pane.pane_id == pane_id)
    }
}

/// One pane's owned title, geometry, viewport anchor, and visible rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneSnapshot {
    pub pane_id: usize,
    pub area: Rect,
    pub body: Rect,
    pub active: bool,
    /// Whether `goto-word` is labelling this pane.
    pub jump_active: bool,
    /// Whether this pane's ordinary text is drawn muted.
    ///
    /// `goto-word` dims the pane its labels are in, and an open command prompt
    /// dims every pane at once. They are one flag here because they ask for
    /// the same colour; they are not one flag in the editor, because only the
    /// first also puts labels on the text.
    pub dimmed: bool,
    pub drawable: bool,
    pub title: PaneTitle,
    pub line_numbers: bool,
    pub line_digits: usize,
    pub signs: bool,
    /// Whether this pane reserves a column for Git change marks.
    pub changes: bool,
    pub text_width: usize,
    /// Columns the gutter occupies, so a frontend can draw a row that has no
    /// line number without re-deriving where the text begins.
    pub gutter_width: usize,
    /// Blank columns between the gutter and the text, held open by this
    /// buffer's [content alignment](crate::content_alignment).
    ///
    /// A frontend draws them before every row's runs. They are not in the
    /// buffer, so nothing in them can be selected, searched, or saved, and a
    /// pointer is translated back through them before it names a column.
    pub content_indent: usize,
    pub scroll_row: usize,
    pub scroll_wrap: usize,
    pub wrap_width: usize,
    /// Screen-row index of the caret in the prepared fold/wrap projection.
    pub cursor_screen_row: Option<usize>,
    pub rows: Vec<SnapshotRow>,
    /// The styled cell rectangle a terminal pane draws instead of `rows`.
    ///
    /// This is the one place a frame carries literal colour rather than a
    /// tree-sitter scope for a frontend to resolve against the theme. A child
    /// process chooses its own colours and no scope can name them, so the
    /// alternative would be to render a terminal in the wrong ones. Only
    /// [`Color::Default`](crate::terminal::Color::Default) is left to the
    /// frontend.
    pub terminal: Option<TerminalView>,
}

/// Semantic pane title data. Frontends decide how to delimit it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneTitle {
    pub name: String,
    pub dirty: bool,
    pub external_file_status: ExternalFileStatus,
    /// Whether the buffer refuses text edits. Carried here rather than
    /// re-derived by a frontend so every surface names it the same way.
    pub read_only: bool,
    /// The maximized presentation this pane is drawn with, if it is the one
    /// being maximized. `None` for every pane in an ordinary layout, so a
    /// frontend marks the view only while it is on.
    pub maximized: Option<MaximizedView>,
}

/// A screen row in a prepared pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotRow {
    /// A row below the end of the document.
    Placeholder,
    /// A row one side of a diff holds open so the line facing it on the other
    /// side stays level. It belongs to no line, and is drawn as an absence
    /// rather than as an empty line of the file.
    Filler,
    /// A row of blank space above or below vertically centred content.
    ///
    /// Like filler it belongs to no line, but it stands for nothing rather
    /// than for a line elsewhere, so it is drawn as plain space: neither the
    /// hatch of an absence nor the marker of a row past the end.
    Padding,
    Text(VisibleRow),
}

/// One visible logical or wrapped document row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleRow {
    pub document_row: usize,
    pub continuation: bool,
    /// This logical row anchors a pane-local collapsed syntax region.
    pub folded: bool,
    pub cursor_row: bool,
    pub diagnostic_sign: Option<Severity>,
    /// How this row differs from the text Git has staged for the file.
    pub change: Option<LineChange>,
    /// What this row is, when the buffer holds a unified diff.
    pub diff: Option<DiffLine>,
    /// How this row reads against the other side of a live side-by-side diff.
    /// Independent of `change`, which compares the same buffer against Git.
    pub compared: Option<Change>,
    /// Severity assigned to this notification heading row, if any.
    pub notification_severity: Option<NotificationSeverity>,
    pub runs: Vec<TextRun>,
}

/// A grouped semantic run. Terminal styles are deliberately not stored here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub kind: TextRunKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextRunKind {
    Text {
        role: TextRole,
        scope: Option<Scope>,
        diagnostic: Option<Severity>,
        directory: bool,
        /// This run contains display-only markers for buffer whitespace.
        whitespace: bool,
        /// Which of a Git list's two line counts this run stands in, so
        /// a frontend can paint it in the palette Git changes already use.
        count: Option<CountKind>,
    },
    JumpLabel(LabelPart),
    InlineDiagnostic(Severity),
    /// A pane-local collapsed syntax region.
    FoldMarker,
    /// A read-only annotation drawn after the row's text. Never part of the
    /// buffer, so it cannot be selected, edited, or saved.
    Hint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextRole {
    Plain,
    GitHash,
    GitHead,
    GitLane0,
    GitLane1,
    GitLane2,
    GitLane3,
    Selected,
    PrimarySelected,
    PrimaryCaret,
    ReplaceCaret,
    Caret,
    SecondaryCaret,
}

/// Owned status and prompt values for the bottom two rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusSnapshot {
    pub mode: Mode,
    /// The digit that reaches this workspace from the session manager, when it
    /// has one. Kept in the snapshot for wire compatibility; the global status
    /// line leaves shortcut presentation to the manager itself.
    pub workspace_number: Option<u8>,
    /// The editor working directory selected by startup or `:cd`, rendered as
    /// workspace context independently of the active pane's buffer identity.
    pub workspace_directory: String,
    pub dirty: bool,
    pub external_file_status: ExternalFileStatus,
    pub read_only: bool,
    pub cursor: Position,
    /// Rows in the buffer the cursor sits in, so a frontend can say how far
    /// through it the cursor is without reaching back into the buffer.
    pub line_count: usize,
    pub selection_count: usize,
    pub lsp_summary: Option<String>,
    /// Branch and outstanding-change label, absent outside a repository.
    pub git_summary: Option<String>,
    /// A long-running action temporarily taking over the status row.
    pub long_running_action: Option<LongRunningActionSnapshot>,
    /// Unacknowledged retained notifications, split by assigned severity.
    pub notification_counts: NotificationCounts,
    pub interaction_line: String,
    pub interaction_line_error: bool,
    /// Display-cell column in the interaction line where a frontend should place
    /// the prompt cursor. `None` outside command mode.
    pub prompt_cursor_column: Option<usize>,
}

impl StatusSnapshot {
    /// How far through the buffer the cursor sits, as a whole percentage.
    ///
    /// The cursor row is what "here" means, not the topmost visible row: the
    /// row and column beside it already read the cursor, and scrolling a pane
    /// away from the cursor is a look elsewhere rather than a move. The first
    /// row is `0` and the last is `100`, so the two ends of the file are the
    /// only places those numbers appear — an interior row a rounding step away
    /// from either is pulled back to `1` or `99` rather than claiming an end it
    /// has not reached. A buffer of one row has no distance to cover and reads
    /// `100`.
    ///
    /// Computed here rather than in a frontend so a local and an attached TUI
    /// drawing the same frame cannot disagree about the number.
    pub fn progress_percent(&self) -> u8 {
        let last_row = self.line_count.saturating_sub(1);
        if last_row == 0 {
            return 100;
        }
        let row = self.cursor.row.min(last_row);
        // Rounded to nearest, halves up, in integers.
        let percent = (row as u128 * 200 + last_row as u128) / (last_row as u128 * 2);
        match percent {
            0 if row > 0 => 1,
            100 if row < last_row => 99,
            percent => u8::try_from(percent).unwrap_or(100),
        }
    }
}

/// Frontend-independent progress for work that should stay visible while the
/// editor remains interactive.
///
/// Producers describe the work; frontends decide how to animate it. Keeping
/// the elapsed time in the immutable frame also makes the same presentation
/// available to local and attached TUIs without exposing a process-local
/// clock value over the protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LongRunningActionSnapshot {
    pub label: String,
    pub detail: String,
    pub elapsed_millis: u64,
    pub cancel_hint: Option<String>,
}

#[derive(Clone, Copy)]
struct RowContext {
    row: usize,
    segment: Option<Segment>,
    continuation: bool,
    folded: bool,
    text_width: usize,
    folded_lines: Option<usize>,
}

/// A pane's last visible syntax result. Text mutations receive global
/// revisions; syntax document identities distinguish replacement trees whose
/// own revision counters both start at zero.
pub(crate) struct VisibleHighlightCache {
    buffer: usize,
    text_revision: u64,
    syntax_source: (u64, u64, bool),
    ranges: Vec<(Offset, Offset)>,
    spans: Vec<Span>,
}

/// The inputs that can change an inactive document pane without changing its
/// prepared projection. Active panes are rebuilt because their selection,
/// search preview, jump labels, and inline diagnostic depend on live input.
#[derive(Clone, Eq, PartialEq)]
struct PaneSnapshotKey {
    prepared: PreparedPane,
    kind: std::mem::Discriminant<BufferKind>,
    path: Option<PathBuf>,
    text_revision: u64,
    syntax_source: Option<(u64, u64, bool)>,
    generated_highlights_revision: u64,
    title: PaneTitle,
    cursor: Position,
    mode: Mode,
    theme: Theme,
    tab_width: usize,
    line_numbers: bool,
    render_whitespace: bool,
    command_mode_dim: bool,
    row_hints: RowHints,
    directory: bool,
    diff_start: Option<Offset>,
    notifications: Vec<Option<NotificationSeverity>>,
    diagnostic_revision: u64,
    git_revision: u64,
    compared: Vec<Option<Change>>,
}

pub(crate) struct CachedPaneSnapshot {
    key: PaneSnapshotKey,
    snapshot: PaneSnapshot,
}

impl App {
    /// Captures the normal editor surface after viewport preparation.
    ///
    /// Only prepared pane rows are copied. Overlays remain separate consumers
    /// of immutable application state for now.
    pub fn snapshot(&self, prepared: &PreparedView) -> EditorSnapshot {
        self.visible_highlights
            .borrow_mut()
            .retain(|pane, _| prepared.pane(*pane).is_some());
        self.visible_pane_snapshots
            .borrow_mut()
            .retain(|pane, _| prepared.pane(*pane).is_some());
        let panes = prepared
            .panes
            .iter()
            .map(|pane| {
                let mut snapshot = self.snapshot_cached_pane(pane);
                if let Some(destination) = &self.directory_tree_destination
                    && let Some(index) = destination.panes.iter().position(|id| *id == pane.pane_id)
                {
                    snapshot.title = PaneTitle {
                        name: (index + 1).to_string(),
                        dirty: false,
                        external_file_status: ExternalFileStatus::Synchronized,
                        read_only: false,
                        maximized: None,
                    };
                }
                snapshot
            })
            .collect();
        let active = self.active();
        let buffer = self.active_buffer();
        let prompt_cursor_column = (self.mode == Mode::Command
            && !matches!(self.prompt_kind, PromptKind::SettingValue(_)))
        .then(|| {
            prompt_prefix(self.prompt_kind)
                .chars()
                .chain(self.command.chars().take(self.command_cursor))
                .map(character_cells)
                .sum()
        });
        let is_prompt =
            matches!(self.prompt_kind, PromptKind::SettingValue(_)) || self.mode == Mode::Command;
        let live_pending_display = self.live_pending_display();
        let message = if let Some(interaction) = self.tree_interaction() {
            interaction
        } else if matches!(self.prompt_kind, PromptKind::SettingValue(_)) {
            String::new()
        } else if self.mode == Mode::Command {
            format!("{}{}", prompt_prefix(self.prompt_kind), self.command)
        } else if let Some(pending) = live_pending_display.clone() {
            pending
        } else {
            self.displayed_status_message().to_owned()
        };
        let message_is_error = !is_prompt
            && live_pending_display.is_none()
            && self.displayed_status_message_is_error();

        // A terminal pane's numbers are its visible child/review surface, not
        // those of the document waiting behind it. Reporting the buffer's
        // would say `[RO]` about a shell and put the caret at 1:1 while it is
        // somewhere else entirely.
        let terminal = self
            .active_terminal()
            .and_then(|id| self.terminals.get(id))
            .map(|session| {
                let cursor = Position {
                    row: session.cursor_row(),
                    col: session.cursor_column(),
                };
                (
                    cursor,
                    session.line_count(),
                    session.review_selection_count(),
                )
            });
        EditorSnapshot {
            geometry: prepared.geometry,
            directory_tree: prepared.tree_area.map(|area| {
                let rows = self.directory_tree.rows();
                let body_height = self.tree_body_height(area);
                let selected = rows
                    .iter()
                    .position(|row| row.path == self.directory_tree.selected);
                let start = self.directory_tree.scroll.min(rows.len());
                DirectoryTreeSnapshot {
                    jump_active: self.directory_tree.focused && self.jump.is_some(),
                    legend: self.tree_legend(area),
                    area,
                    focused: self.directory_tree.focused,
                    title: "[dir tree]".into(),
                    total_rows: rows.len(),
                    selected,
                    rows: rows
                        .into_iter()
                        .skip(start)
                        .take(body_height)
                        .map(|row| {
                            let label = row.path.file_name().unwrap_or_default().to_string_lossy();
                            let label = escape_tree_controls(&label);
                            let jump_label = if self.directory_tree.focused && self.jump.is_some() {
                                let jump_offset = self
                                    .directory_tree_jump_paths
                                    .iter()
                                    .position(|path| *path == row.path)
                                    .map(|index| index * 2);
                                std::array::from_fn(|cell| {
                                    self.jump.as_ref()?.label_at(jump_offset? + cell)
                                })
                            } else {
                                [None; 2]
                            };
                            DirectoryTreeRowSnapshot {
                                jump_label,
                                selected: row.path == self.directory_tree.selected,
                                path: row.path,
                                label,
                                depth: row.depth.min(128),
                                kind: row.kind,
                                expanded: row.expanded,
                                loading: row.loading,
                                error: row.error.map(|error| escape_tree_controls(&error)),
                            }
                        })
                        .collect(),
                }
            }),
            session_strip: prepared
                .session_strip
                .as_ref()
                .map(|strip| strip.snapshot.clone()),
            theme: self.theme.clone(),
            mode: self.mode,
            panes,
            status: StatusSnapshot {
                mode: self.mode,
                workspace_number: self.workspace_number,
                workspace_directory: self.working_directory.to_string_lossy().into_owned(),
                dirty: terminal.is_none() && buffer.dirty,
                external_file_status: if terminal.is_none() {
                    buffer.external_file_status()
                } else {
                    ExternalFileStatus::Synchronized
                },
                read_only: terminal.is_none() && buffer.is_read_only(),
                cursor: terminal.map_or_else(|| active.cursor(buffer), |(cursor, _, _)| cursor),
                line_count: terminal.map_or_else(|| buffer.len_lines(), |(_, lines, _)| lines),
                selection_count: terminal.map_or(active.selection.len(), |(_, _, count)| count),
                lsp_summary: self.lsp_summary(),
                git_summary: self.git_summary(),
                long_running_action: self.long_running_action_snapshot(),
                notification_counts: self.unread_notification_counts(),
                interaction_line: message,
                interaction_line_error: message_is_error,
                prompt_cursor_column,
            },
        }
    }

    /// Whether an open prompt currently grays out the panes.
    ///
    /// Every text-entry prompt reads as Command mode, so every one of them
    /// dims: the keyboard belongs to the prompt in all of them equally. A
    /// pending chord or count is not a mode and never reaches here, which is
    /// what keeps `g` and `Space` from dimming anything.
    fn command_prompt_dims_panes(&self) -> bool {
        self.config.editor.command_mode_dim && self.mode == Mode::Command
    }

    fn snapshot_cached_pane(&self, prepared: &PreparedPane) -> PaneSnapshot {
        let buffer = &self.buffers[prepared.buffer_id];
        // Some generated lists read typed row metadata outside the buffer's
        // text and row hints. Keep their live path until that metadata has a
        // presentation revision of its own.
        let cacheable = match &buffer.kind {
            BufferKind::File
            | BufferKind::Provider(_)
            | BufferKind::Scratch
            | BufferKind::CommitMessage => true,
            BufferKind::Virtual { identity, .. } => {
                !matches!(identity, GeneratedViewIdentity::GitComparison { .. })
            }
            _ => false,
        };
        if prepared.terminal.is_some() || prepared.pane_id == self.active_pane || !cacheable {
            self.visible_pane_snapshots
                .borrow_mut()
                .remove(&prepared.pane_id);
            return self.snapshot_pane(prepared);
        }
        let key = PaneSnapshotKey {
            prepared: prepared.clone(),
            kind: std::mem::discriminant(&buffer.kind),
            path: buffer.path.clone(),
            text_revision: buffer.revision(),
            syntax_source: self.visible_syntax_source(prepared.buffer_id),
            generated_highlights_revision: self.generated_highlights_revision,
            title: PaneTitle {
                name: buffer.pane_title(),
                dirty: buffer.dirty,
                external_file_status: buffer.external_file_status(),
                read_only: buffer.is_read_only(),
                maximized: self.maximized_view(prepared.pane_id),
            },
            cursor: self.panes[&prepared.pane_id].cursor(buffer),
            mode: self.mode,
            theme: self.theme.clone(),
            tab_width: self.config.editor.tab_width,
            line_numbers: self.config.editor.line_numbers,
            render_whitespace: self.config.editor.render_whitespace,
            command_mode_dim: self.config.editor.command_mode_dim,
            row_hints: buffer.row_hints(),
            directory: buffer.is_directory(),
            diff_start: buffer.diff_start(),
            notifications: prepared
                .rows
                .iter()
                .map(|row| {
                    row.document_row
                        .and_then(|document_row| buffer.notification_row_at(document_row))
                        .and_then(|notification| notification.severity)
                })
                .collect(),
            diagnostic_revision: self.diagnostics.revision(),
            git_revision: self.git_presentation_revision(),
            compared: prepared
                .rows
                .iter()
                .map(|row| {
                    row.document_row
                        .and_then(|document_row| self.row_compared(prepared.pane_id, document_row))
                })
                .collect(),
        };
        if let Some(cached) = self.visible_pane_snapshots.borrow().get(&prepared.pane_id)
            && cached.key == key
        {
            return cached.snapshot.clone();
        }
        #[cfg(test)]
        self.visible_pane_rebuilds
            .set(self.visible_pane_rebuilds.get() + 1);
        let snapshot = self.snapshot_pane(prepared);
        self.visible_pane_snapshots.borrow_mut().insert(
            prepared.pane_id,
            CachedPaneSnapshot {
                key,
                snapshot: snapshot.clone(),
            },
        );
        snapshot
    }

    fn snapshot_pane(&self, prepared: &PreparedPane) -> PaneSnapshot {
        if let Some(id) = prepared.terminal
            && let Some(session) = self.terminals.get(id)
        {
            let jump_active = prepared.pane_id == self.active_pane
                && !self.directory_tree.focused
                && self.jump.is_some();
            return PaneSnapshot {
                pane_id: prepared.pane_id,
                area: prepared.area,
                body: prepared.body,
                active: prepared.pane_id == self.active_pane && !self.directory_tree.focused,
                jump_active,
                // A terminal under review is frozen: the child has stopped
                // painting and the keys move a cursor over a still image
                // instead. Graying it says which of the two a terminal is,
                // which is the same question the dim answers everywhere else.
                dimmed: jump_active || session.reviewing() || self.command_prompt_dims_panes(),
                drawable: prepared.drawable,
                title: PaneTitle {
                    name: terminal_title(
                        session,
                        (prepared.pane_id == self.active_pane).then_some(self.mode),
                    ),
                    dirty: false,
                    external_file_status: ExternalFileStatus::Synchronized,
                    // A terminal is neither writable nor refusing writes; it
                    // is not a document. Saying "read only" here would answer
                    // a question nobody asked of it.
                    read_only: false,
                    maximized: self.maximized_view(prepared.pane_id),
                },
                line_numbers: false,
                line_digits: 0,
                signs: false,
                changes: false,
                text_width: prepared.text_width,
                gutter_width: 0,
                content_indent: 0,
                scroll_row: 0,
                scroll_wrap: 0,
                wrap_width: prepared.wrap_width,
                cursor_screen_row: None,
                rows: Vec::new(),
                terminal: prepared.drawable.then(|| {
                    self.jump.as_ref().filter(|_| jump_active).map_or_else(
                        || session.view(prepared.body_height),
                        |labels| session.view_with_jump_labels(prepared.body_height, labels),
                    )
                }),
            };
        }
        let buffer = &self.buffers[prepared.buffer_id];
        let cursor = self.panes[&prepared.pane_id].cursor(buffer);
        let cursor_screen_row = prepared.rows.iter().position(|row| {
            row.document_row == Some(cursor.row)
                && row.segment.is_none_or(|segment| {
                    crate::wrap::segment_contains(
                        buffer,
                        cursor.row,
                        segment,
                        cursor.col,
                        prepared.wrap_width,
                        self.config.editor.tab_width,
                    )
                })
        });
        let jump_active = prepared.pane_id == self.active_pane
            && !self.directory_tree.focused
            && self.jump.is_some();
        let mut snapshot = PaneSnapshot {
            pane_id: prepared.pane_id,
            area: prepared.area,
            body: prepared.body,
            active: prepared.pane_id == self.active_pane && !self.directory_tree.focused,
            jump_active,
            dimmed: jump_active || self.command_prompt_dims_panes(),
            drawable: prepared.drawable,
            title: PaneTitle {
                name: buffer.pane_title(),
                dirty: buffer.dirty,
                external_file_status: buffer.external_file_status(),
                read_only: buffer.is_read_only(),
                maximized: self.maximized_view(prepared.pane_id),
            },
            line_numbers: self.config.editor.line_numbers,
            line_digits: prepared.line_digits,
            signs: prepared.signs,
            changes: prepared.changes,
            text_width: prepared.text_width,
            gutter_width: prepared.gutter_width,
            content_indent: prepared.content_indent,
            scroll_row: prepared.scroll_row,
            scroll_wrap: prepared.scroll_wrap,
            wrap_width: prepared.wrap_width,
            cursor_screen_row,
            rows: Vec::new(),
            terminal: None,
        };
        if !prepared.drawable {
            return snapshot;
        }

        // Folded gaps are intentionally queried as disjoint visible rows. A
        // viewport that shows the two sides of a million-line fold must not
        // ask the syntax engine to highlight the million hidden lines.
        //
        // Each row is queried over the columns it actually draws rather than
        // over its whole logical line, for the same reason the row renderer
        // below stays inside the viewport: one minified line can be the entire
        // document, and highlighting it end to end on every frame is the
        // difference between a responsive editor and a stalled one. A narrowed
        // query still reports the scopes of nodes that began earlier, so the
        // colours are the ones the full line would have produced.
        let mut highlight_ranges: Vec<(Offset, Offset)> = Vec::new();
        for row in &prepared.rows {
            let Some(document_row) = row.document_row else {
                continue;
            };
            if let Some(index) = row.segment.and_then(|segment| segment.table)
                && let Some(layout) = crate::wrap::table_layout(
                    buffer,
                    document_row,
                    prepared.wrap_width,
                    self.config.editor.tab_width,
                )
            {
                let scroll = if layout.width > prepared.wrap_width {
                    prepared.scroll_col
                } else {
                    0
                };
                let start = buffer.line_to_offset(document_row);
                for atom in layout.visible(buffer, document_row, index, scroll, prepared.wrap_width)
                {
                    if let Some(column) = atom.offset {
                        highlight_ranges.push((start + column, start + column + 1));
                    }
                }
                continue;
            }
            let line_len = buffer.line_len(document_row);
            let start_col = row
                .segment
                .map_or(prepared.scroll_col, |segment| segment.start)
                .min(line_len);
            let end_col = row
                .segment
                .map_or_else(
                    || {
                        visible_character_end(
                            buffer
                                .text()
                                .line(document_row)
                                .chars_at(start_col)
                                .take(line_len.saturating_sub(start_col)),
                            start_col,
                            prepared.text_width,
                            self.config.editor.tab_width,
                        )
                    },
                    |segment| {
                        segment.end.min(segment.start.saturating_add(
                            prepared.text_width.saturating_add(ZERO_WIDTH_SCAN_LIMIT),
                        ))
                    },
                )
                .min(line_len);
            let from = buffer.line_to_offset(document_row);
            let range = (from + start_col, from + end_col);
            highlight_ranges.push(range);
        }
        highlight_ranges.sort_unstable();
        let mut merged: Vec<(Offset, Offset)> = Vec::new();
        for range in highlight_ranges {
            if let Some(previous) = merged.last_mut()
                && (range.0 <= previous.1
                    || (previous.1.checked_add(1) == Some(range.0)
                        && buffer.char_at(previous.1) == Some('\n')))
            {
                previous.1 = previous.1.max(range.1);
            } else {
                merged.push(range);
            }
        }
        let syntax_source = self.visible_syntax_source(prepared.buffer_id);
        let cached = syntax_source.and_then(|source| {
            self.visible_highlights
                .borrow()
                .get(&prepared.pane_id)
                .filter(|cache| {
                    cache.buffer == prepared.buffer_id
                        && cache.text_revision == buffer.revision()
                        && cache.syntax_source == source
                        && cache.ranges == merged
                })
                .map(|cache| cache.spans.clone())
        });
        let highlights = cached.unwrap_or_else(|| {
            let mut spans = Vec::new();
            for &(from, to) in &merged {
                spans.extend(self.highlights(prepared.buffer_id, from, to));
            }
            spans.sort_by_key(|span| (span.from, span.to));
            spans.dedup();
            if let Some(source) = syntax_source {
                self.visible_highlights.borrow_mut().insert(
                    prepared.pane_id,
                    VisibleHighlightCache {
                        buffer: prepared.buffer_id,
                        text_revision: buffer.revision(),
                        syntax_source: source,
                        ranges: merged,
                        spans: spans.clone(),
                    },
                );
            } else {
                self.visible_highlights
                    .borrow_mut()
                    .remove(&prepared.pane_id);
            }
            spans
        });

        // Once per pane rather than once per row: the column hints align on is
        // a property of the whole listing, not of the rows currently on screen.
        let hints = buffer.row_hints();

        snapshot.rows = (0..prepared.body_height)
            .map(|screen_row| match prepared.rows.get(screen_row) {
                None => SnapshotRow::Placeholder,
                Some(row) if row.padding => SnapshotRow::Padding,
                Some(row) => match row.document_row {
                    None => SnapshotRow::Filler,
                    Some(document_row) => self.snapshot_row(
                        prepared,
                        buffer,
                        &highlights,
                        &hints,
                        RowContext {
                            row: document_row,
                            segment: row.segment,
                            continuation: row.continuation,
                            folded: row.folded,
                            text_width: prepared
                                .text_width
                                .saturating_sub(prepared.row_prefix_width)
                                .max(1),
                            folded_lines: row.folded_lines,
                        },
                    ),
                },
            })
            .collect();
        snapshot
    }

    fn snapshot_row(
        &self,
        prepared: &PreparedPane,
        buffer: &Buffer,
        highlights: &[Span],
        hints: &RowHints,
        context: RowContext,
    ) -> SnapshotRow {
        if context.row >= buffer.len_lines() {
            return SnapshotRow::Placeholder;
        }
        let pane = &self.panes[&prepared.pane_id];
        let cursor = pane.cursor(buffer);
        let cursor_segment = context.segment.is_none_or(|segment| {
            crate::wrap::segment_contains(
                buffer,
                context.row,
                segment,
                cursor.col,
                prepared.wrap_width,
                self.config.editor.tab_width,
            )
        });
        let mut runs = self.snapshot_text_runs(prepared, buffer, highlights, context);
        if let Some(lines) = context.folded_lines {
            let used_cells = runs
                .iter()
                .map(|run| display_cells(&run.text))
                .sum::<usize>();
            let marker = format!(" … {lines} line{}", if lines == 1 { "" } else { "s" });
            let remaining = context.text_width.saturating_sub(used_cells);
            let marker = clip_fragments_to_cells([marker.as_str()], remaining);
            if !marker.is_empty() {
                runs.push(TextRun {
                    text: marker,
                    kind: TextRunKind::FoldMarker,
                });
            }
        }
        // A hint belongs to the row, not to a screen line, so a wrapped row
        // carries it once, after the segment that ends its text.
        if context
            .segment
            .is_none_or(|segment| segment.end == buffer.line_len(context.row))
        {
            let used_cells = runs
                .iter()
                .map(|run| display_cells(&run.text))
                .sum::<usize>();
            let remaining = context.text_width.saturating_sub(used_cells);
            if let Some(text) = hints.rendered(context.row, used_cells, remaining) {
                runs.push(TextRun {
                    text,
                    kind: TextRunKind::Hint,
                });
            }
        }
        if prepared.pane_id == self.active_pane && context.row == cursor.row && cursor_segment {
            let used_cells = runs
                .iter()
                .map(|run| display_cells(&run.text))
                .sum::<usize>();
            let remaining = context.text_width.saturating_sub(used_cells);
            if let Some((text, severity)) =
                self.snapshot_inline_diagnostic(prepared.buffer_id, context.row, remaining)
                && !text.is_empty()
            {
                runs.push(TextRun {
                    text,
                    kind: TextRunKind::InlineDiagnostic(severity),
                });
            }
        }
        if prepared.row_prefix_width > 0 {
            let prefix = if context.continuation {
                String::new()
            } else {
                clip_fragment_cell_range(
                    hints.prefix(context.row).unwrap_or(""),
                    prepared.row_prefix_scroll,
                    prepared.row_prefix_width,
                )
            };
            let padding = prepared
                .row_prefix_width
                .saturating_sub(display_cells(&prefix));
            runs.insert(
                0,
                TextRun {
                    text: format!("{prefix}{}", " ".repeat(padding)),
                    kind: TextRunKind::Hint,
                },
            );
        }
        SnapshotRow::Text(VisibleRow {
            document_row: context.row,
            continuation: context.continuation,
            folded: context.folded,
            cursor_row: context.row == buffer.offset_to_row(pane.head()),
            diagnostic_sign: self.row_severity(prepared.buffer_id, context.row),
            change: self
                .diff_session(prepared.pane_id)
                .is_none()
                .then(|| self.row_change(prepared.buffer_id, context.row, context.continuation))
                .flatten(),
            diff: self.row_diff(buffer, context.row),
            compared: self.row_compared(prepared.pane_id, context.row),
            notification_severity: buffer
                .notification_row_at(context.row)
                .and_then(|row| row.severity),
            runs,
        })
    }

    /// The mark a row carries, shown only on the logical line's first screen
    /// row. Wrapped continuations use the same gutter cell for their arrow.
    fn row_change(&self, buffer_id: usize, row: usize, continuation: bool) -> Option<LineChange> {
        let change = self.git_change(buffer_id, row)?;
        (!continuation).then_some(change)
    }

    /// How one row reads against the other side of a live side-by-side diff.
    ///
    /// Wrapping is off in a diff pane, so there is no continuation row to
    /// decide about: every screen row of one is a whole line or filler.
    fn row_compared(&self, pane_id: usize, row: usize) -> Option<Change> {
        let session = self.diff_session(pane_id)?;
        session.change(session.side_of_pane(pane_id)?, row)
    }

    /// How to read one row of a diff buffer.
    ///
    /// A wrapped row keeps its logical line's classification, because a long
    /// added line is still an added line on its second screen row.
    fn row_diff(&self, buffer: &Buffer, row: usize) -> Option<DiffLine> {
        let diff_start = buffer.diff_start()?;
        if buffer.line_to_offset(row) < diff_start {
            return None;
        }
        // Classification only distinguishes the first byte and the short
        // `--- `/`+++ ` header pair. Do not copy whole patch rows just to
        // decide their gutter colour.
        let line = |row: usize| {
            (row < buffer.len_lines())
                .then(|| buffer.text().line(row).chars().take(4).collect::<String>())
        };
        crate::git::classify_line(
            &line(row)?,
            row.checked_sub(1).and_then(line).as_deref(),
            line(row + 1).as_deref(),
        )
    }

    fn snapshot_text_runs(
        &self,
        prepared: &PreparedPane,
        buffer: &Buffer,
        highlights: &[Span],
        context: RowContext,
    ) -> Vec<TextRun> {
        let pane = &self.panes[&prepared.pane_id];
        let active = prepared.pane_id == self.active_pane;
        let row_start = buffer.line_to_offset(context.row);
        let scope_at = |offset: Offset| {
            highlights
                .binary_search_by(|span| {
                    if offset < span.from {
                        std::cmp::Ordering::Greater
                    } else if offset >= span.to {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .ok()
                .map(|index| highlights[index].scope)
        };
        let diagnostics = self.diagnostic_spans(prepared.buffer_id, context.row);
        let flagged_at = |offset: Offset| {
            diagnostics
                .iter()
                .filter(|(from, to, _)| offset >= *from && offset < *to)
                .map(|(_, _, severity)| *severity)
                .max()
        };
        let pristine_search =
            self.mode == Mode::Select && self.pristine_search_selection(prepared.pane_id);
        let replacing = self.awaiting_character_command() == Some(EditorCommand::ReplaceChar);
        let preview = self.search_preview_for(prepared.pane_id, prepared.buffer_id);
        let first_offset =
            row_start.saturating_add(context.segment.map_or(prepared.scroll_col, |segment| {
                if segment.table.is_some() {
                    0
                } else {
                    segment.start
                }
            }));
        let mut selection_roles = SelectionRoles::new(
            pane.selection.ranges(),
            pane.selection.primary_index(),
            first_offset,
            self.mode == Mode::Select,
            pristine_search,
            replacing,
            pane.selection_semantics(),
        )
        .with_lone_range_shown(self.config.editor.selecting_motions);
        let mut role_at = |offset: Offset| {
            let role = if !active {
                TextRole::Plain
            } else if let Some(preview) = preview {
                preview.role_at(offset)
            } else {
                selection_roles.role_at(offset)
            };
            if role == TextRole::Plain {
                self.network_role_at(pane.buffer, offset).unwrap_or(role)
            } else {
                role
            }
        };
        let label_at = |offset: Offset| {
            active
                .then(|| self.jump.as_ref()?.label_at(offset))
                .flatten()
        };

        if let Some(index) = context.segment.and_then(|segment| segment.table)
            && let Some(layout) = crate::wrap::table_layout(
                buffer,
                context.row,
                prepared.wrap_width,
                self.config.editor.tab_width,
            )
        {
            let scroll = if layout.width > context.text_width {
                prepared.scroll_col
            } else {
                0
            };
            let ranges = pane.selection.ranges();
            let first_head = ranges.partition_point(|range| range.head < row_start);
            let row_end = row_start + buffer.text().line(context.row).len_chars();
            let heads = ranges[first_head..]
                .iter()
                .take_while(|range| range.head <= row_end)
                .filter_map(|range| {
                    let position = buffer.position_of(range.head);
                    if position.row != context.row {
                        return None;
                    }
                    let (line, x) = layout.position(position.col);
                    (line == index && x >= scroll).then_some((x.saturating_sub(scroll), range.head))
                })
                .collect::<Vec<_>>();
            let mut heads = heads.iter().peekable();
            let mut runs: Vec<TextRun> = Vec::new();
            let mut next_cell = 0;
            for atom in layout.visible(buffer, context.row, index, scroll, context.text_width) {
                // A wide glyph intersected by the viewport is blank, while
                // every following column retains its actual screen position.
                if atom.x > next_cell {
                    runs.push(TextRun {
                        text: " ".repeat(atom.x - next_cell),
                        kind: TextRunKind::Hint,
                    });
                }
                let offset = atom.offset.map(|col| row_start + col);
                let mut role = offset.map_or(TextRole::Plain, &mut role_at);
                while heads.peek().is_some_and(|(x, _)| *x < atom.x) {
                    heads.next();
                }
                if let Some((_, head)) = heads.peek().filter(|(x, _)| *x == atom.x) {
                    let head_role = role_at(*head);
                    if matches!(
                        head_role,
                        TextRole::Caret
                            | TextRole::PrimaryCaret
                            | TextRole::ReplaceCaret
                            | TextRole::SecondaryCaret
                    ) {
                        role = head_role;
                    }
                }
                let whitespace = offset.is_some()
                    && self.config.editor.render_whitespace
                    && matches!(atom.ch, ' ' | '\t');
                let kind = offset.and_then(&label_at).map_or_else(
                    || TextRunKind::Text {
                        role,
                        scope: offset.and_then(&scope_at).or_else(|| {
                            atom.offset
                                .is_none()
                                .then(|| Scope::named("comment").unwrap())
                        }),
                        diagnostic: None,
                        directory: false,
                        whitespace,
                        count: None,
                    },
                    |(_, part)| TextRunKind::JumpLabel(part),
                );
                let text = if let Some((label, _)) = offset.and_then(&label_at) {
                    label.to_string()
                } else if atom.ch == '\t' {
                    if whitespace {
                        format!("→{}", " ".repeat(atom.width.saturating_sub(1)))
                    } else {
                        " ".repeat(atom.width)
                    }
                } else if whitespace {
                    "·".to_owned()
                } else {
                    atom.ch.to_string()
                };
                if let Some(last) = runs.last_mut()
                    && last.kind == kind
                {
                    last.text.push_str(&text);
                } else {
                    runs.push(TextRun { text, kind });
                }
                next_cell = atom.x + atom.width;
            }
            return runs;
        }

        let mut runs = Vec::new();
        let mut current = String::new();
        let mut current_role = TextRole::Plain;
        let mut current_scope = None;
        let mut current_diagnostic = None;
        let mut current_count = None;
        let mut current_whitespace = false;
        let segment = context.segment;
        let start_col = segment.map_or(prepared.scroll_col, |segment| segment.start);
        let mut visual_col = segment.map_or(0, |segment| segment.start_cell);
        let initial_cell = visual_col;
        let visible_end = initial_cell.saturating_add(context.text_width);
        // Keep rendering bounded by the viewport. Git commit patches can
        // contain generated or minified lines that are megabytes long; a
        // full `line_string` allocation here made merely paging onto such a
        // row block the input loop.
        let line_len = buffer.line_len(context.row);
        let end_col = segment.map_or_else(
            || {
                visible_character_end(
                    buffer
                        .text()
                        .line(context.row)
                        .chars_at(start_col.min(line_len))
                        .take(line_len.saturating_sub(start_col)),
                    start_col,
                    context.text_width,
                    self.config.editor.tab_width,
                )
            },
            |segment| {
                segment.end.min(
                    segment
                        .start
                        .saturating_add(context.text_width.saturating_add(ZERO_WIDTH_SCAN_LIMIT)),
                )
            },
        );
        let directory = if buffer.is_directory() {
            let line = buffer.line_string(context.row);
            buffer.directory_line_is_directory(context.row, &line)
        } else {
            false
        };
        // Git line counts are read from the projection that
        // wrote them rather than found again in the row: the padding that
        // aligns the column is what would have to be parsed back out.
        let counts = self
            .git_status_count_columns(prepared.buffer_id, context.row)
            .or_else(|| self.git_comparison_count_columns(prepared.buffer_id, context.row));

        for (col, character) in buffer
            .text()
            .line(context.row)
            .chars_at(start_col.min(line_len))
            .take(end_col.saturating_sub(start_col))
            .enumerate()
        {
            let col = start_col + col;
            let remaining = visible_end.saturating_sub(visual_col);
            if let Some((label, part)) = label_at(row_start + col) {
                if remaining == 0 {
                    break;
                }
                push_text_run(
                    &mut runs,
                    &mut current,
                    TextRunMetadata {
                        role: current_role,
                        scope: current_scope,
                        diagnostic: current_diagnostic,
                        directory,
                        count: current_count,
                        whitespace: current_whitespace,
                    },
                );
                runs.push(TextRun {
                    text: label.to_string(),
                    kind: TextRunKind::JumpLabel(part),
                });
                visual_col += 1;
                if visual_col >= visible_end {
                    break;
                }
                continue;
            }
            let role = role_at(row_start + col);
            let scope = scope_at(row_start + col);
            let diagnostic = flagged_at(row_start + col);
            let count = counts.as_ref().and_then(|columns| columns.kind_at(col));
            let whitespace =
                self.config.editor.render_whitespace && matches!(character, ' ' | '\t');
            if (role != current_role
                || scope != current_scope
                || diagnostic != current_diagnostic
                || count != current_count
                || whitespace != current_whitespace)
                && !current.is_empty()
            {
                push_text_run(
                    &mut runs,
                    &mut current,
                    TextRunMetadata {
                        role: current_role,
                        scope: current_scope,
                        diagnostic: current_diagnostic,
                        directory,
                        count: current_count,
                        whitespace: current_whitespace,
                    },
                );
            }
            current_role = role;
            current_scope = scope;
            current_diagnostic = diagnostic;
            current_count = count;
            current_whitespace = whitespace;
            if character == '\t' {
                if remaining == 0 {
                    break;
                }
                let tab_width = self.config.editor.tab_width.max(1);
                let width = (tab_width - (visual_col % tab_width)).min(remaining);
                if self.config.editor.render_whitespace {
                    current.push('→');
                    current.push_str(&" ".repeat(width.saturating_sub(1)));
                } else {
                    current.push_str(&" ".repeat(width));
                }
                visual_col += width;
            } else {
                let width = UnicodeWidthChar::width(character).unwrap_or(0);
                if width > remaining {
                    break;
                }
                current.push(if whitespace { '·' } else { character });
                visual_col += width;
            }
        }
        push_text_run(
            &mut runs,
            &mut current,
            TextRunMetadata {
                role: current_role,
                scope: current_scope,
                diagnostic: current_diagnostic,
                directory,
                count: current_count,
                whitespace: current_whitespace,
            },
        );

        // A line terminator is not part of `line_len`, but it is part of the
        // buffer and gets one display cell when the final visual segment has
        // room. CRLF is one terminator and therefore one marker.
        let final_segment = context
            .segment
            .is_none_or(|segment| segment.end == buffer.line_len(context.row));
        let has_terminator = buffer.text().line(context.row).len_chars() > line_len;
        if self.config.editor.render_whitespace
            && final_segment
            && has_terminator
            && runs
                .iter()
                .map(|run| display_cells(&run.text))
                .sum::<usize>()
                < context.text_width
        {
            let end = row_start + line_len;
            runs.push(TextRun {
                text: "¬".to_owned(),
                kind: TextRunKind::Text {
                    role: role_at(end),
                    scope: None,
                    diagnostic: None,
                    directory,
                    whitespace: true,
                    count: None,
                },
            });
        }

        // A caret parked past the last character of a row still needs a cell,
        // but only when that cell belongs to this segment and fits on screen.
        if active
            && context
                .segment
                .is_none_or(|segment| segment.end == buffer.line_len(context.row))
            && runs
                .iter()
                .map(|run| display_cells(&run.text))
                .sum::<usize>()
                < context.text_width
        {
            let end = row_start + buffer.line_len(context.row);
            let role = role_at(end);
            if matches!(
                role,
                TextRole::PrimaryCaret
                    | TextRole::ReplaceCaret
                    | TextRole::Caret
                    | TextRole::SecondaryCaret
            ) {
                runs.push(TextRun {
                    text: " ".to_owned(),
                    kind: TextRunKind::Text {
                        role,
                        scope: None,
                        diagnostic: None,
                        directory,
                        whitespace: false,
                        count: None,
                    },
                });
            }
        }
        runs
    }

    fn snapshot_inline_diagnostic(
        &self,
        buffer_id: usize,
        row: usize,
        remaining_cells: usize,
    ) -> Option<(String, Severity)> {
        let path = self.buffers.get(buffer_id)?.path.as_deref()?;
        let diagnostic = self.diagnostics.for_row(path, row).into_iter().next()?;
        let severity = diagnostic.severity;
        let text = match diagnostic.source.as_deref() {
            Some(source) => clip_fragments_to_cells(
                [
                    "  ",
                    severity.label(),
                    " [",
                    source,
                    "] ",
                    diagnostic.message.as_str(),
                ],
                remaining_cells,
            ),
            None => clip_fragments_to_cells(
                ["  ", severity.label(), " ", diagnostic.message.as_str()],
                remaining_cells,
            ),
        };
        Some((text, severity))
    }
}

#[derive(Clone, Copy)]
struct TextRunMetadata {
    role: TextRole,
    scope: Option<Scope>,
    diagnostic: Option<Severity>,
    directory: bool,
    count: Option<CountKind>,
    whitespace: bool,
}

fn push_text_run(runs: &mut Vec<TextRun>, current: &mut String, metadata: TextRunMetadata) {
    if current.is_empty() {
        return;
    }
    runs.push(TextRun {
        text: std::mem::take(current),
        kind: TextRunKind::Text {
            role: metadata.role,
            scope: metadata.scope,
            diagnostic: metadata.diagnostic,
            directory: metadata.directory,
            whitespace: metadata.whitespace,
            count: metadata.count,
        },
    });
}

fn display_cells(text: &str) -> usize {
    text.chars().map(character_cells).sum()
}

fn character_cells(character: char) -> usize {
    UnicodeWidthChar::width(character).unwrap_or(0)
}

/// Finds the bounded character boundary that occupies `cell_limit` cells.
///
/// Zero-width marks must accompany their base character, but a hostile line
/// containing only marks must not turn one frame into an unbounded scan.
fn visible_character_end(
    characters: impl Iterator<Item = char>,
    start_column: usize,
    cell_limit: usize,
    tab_width: usize,
) -> usize {
    let tab_width = tab_width.max(1);
    let mut column = start_column;
    let mut cells = 0_usize;
    for (scanned, character) in characters.enumerate() {
        if scanned >= cell_limit.saturating_add(ZERO_WIDTH_SCAN_LIMIT) {
            break;
        }
        let width = if character == '\t' {
            tab_width - cells % tab_width
        } else {
            character_cells(character)
        };
        if width > 0 && cells.saturating_add(width) > cell_limit {
            break;
        }
        column += 1;
        cells = cells.saturating_add(width);
    }
    column
}

fn clip_fragments_to_cells<'a>(
    fragments: impl IntoIterator<Item = &'a str>,
    limit: usize,
) -> String {
    let mut width = 0;
    let mut clipped = String::with_capacity(limit);
    for fragment in fragments {
        for character in fragment.chars() {
            let next = width + character_cells(character);
            if next > limit {
                return clipped;
            }
            width = next;
            clipped.push(character);
        }
    }
    clipped
}

/// A display-cell window into one presentation fragment.
///
/// If the left edge lands inside a wide glyph, the remainder of that glyph is
/// blank. This preserves every following column instead of shifting it left.
fn clip_fragment_cell_range(fragment: &str, start: usize, limit: usize) -> String {
    let end = start.saturating_add(limit);
    let mut position = 0usize;
    let mut clipped = String::with_capacity(limit);
    for character in fragment.chars() {
        let width = character_cells(character);
        let next = position.saturating_add(width);
        if next <= start {
            position = next;
            continue;
        }
        if position < start {
            clipped.push_str(&" ".repeat(next.min(end).saturating_sub(start)));
            position = next;
            continue;
        }
        if next > end {
            break;
        }
        clipped.push(character);
        position = next;
    }
    clipped
}

fn prompt_prefix(kind: crate::app::PromptKind) -> String {
    use crate::app::{PromptKind, SearchMode};
    match kind {
        PromptKind::Command => ":".to_owned(),
        PromptKind::Search(SearchMode::Insensitive) => "search: ".to_owned(),
        PromptKind::Search(SearchMode::Sensitive) => "search (case-sensitive): ".to_owned(),
        PromptKind::Search(SearchMode::Regex) => "search (regex): ".to_owned(),
        PromptKind::TerminalSearch(SearchMode::Insensitive) => "terminal search: ".to_owned(),
        PromptKind::TerminalSearch(SearchMode::Sensitive) => {
            "terminal search (case-sensitive): ".to_owned()
        }
        PromptKind::TerminalSearch(SearchMode::Regex) => "terminal search (regex): ".to_owned(),
        // The Vim grammar keeps Vim's own one-character prompts.
        PromptKind::SearchForward => "/".to_owned(),
        PromptKind::SearchBackward => "?".to_owned(),
        PromptKind::GlobalSearch(SearchMode::Insensitive) => "workspace search: ".to_owned(),
        PromptKind::GlobalSearch(SearchMode::Sensitive) => {
            "workspace search (case-sensitive): ".to_owned()
        }
        PromptKind::GlobalSearch(SearchMode::Regex) => "workspace search (regex): ".to_owned(),
        PromptKind::FilterSelections { keep: true } => "keep (regex): ".to_owned(),
        PromptKind::FilterSelections { keep: false } => "remove (regex): ".to_owned(),
        PromptKind::Rename => "rename to: ".to_owned(),
        PromptKind::SessionRename => "session name: ".to_owned(),
        PromptKind::SessionNumber => "session number (1-9, empty clears): ".to_owned(),
        PromptKind::TerminalRename => "terminal name: ".to_owned(),
        PromptKind::ExternalProgram => "open with: ".to_owned(),
        PromptKind::NewBranch => "new branch: ".to_owned(),
        PromptKind::NewWorktreeBranch => "new worktree branch: ".to_owned(),
        PromptKind::WorktreeDestination => "worktree destination: ".to_owned(),
        PromptKind::JoinDelimiter => "join with (empty joins directly): ".to_owned(),
        PromptKind::SettingValue(setting) => format!("{}: ", setting.descriptor().title),
        PromptKind::FinderPath => "find under path: ".to_owned(),
        PromptKind::DirectoryTreeSearch => "tree search (regex): ".to_owned(),
        PromptKind::DirectoryTreeAction(crate::app::TreePromptAction::New) => {
            "new file or directory: ".to_owned()
        }
        PromptKind::DirectoryTreeAction(crate::app::TreePromptAction::Rename) => {
            "rename to: ".to_owned()
        }
        PromptKind::DirectoryTreeAction(crate::app::TreePromptAction::Move) => {
            "move to: ".to_owned()
        }
    }
}

/// What a terminal pane calls itself.
///
/// The child's own title when it has set one, and the program otherwise, plus
/// the two things the name is the only place to say: that the reader is
/// looking at history rather than at the live screen, and that the child has
/// gone. Neither is a buffer property, so neither has a field of its own.
///
/// The title exposes the number accepted by `:terminal-send` without
/// requiring the terminal list preview. An exited terminal has none, so its
/// title carries only the kind.
///
/// Only `[insert]` is named. The marker answers one question — whether typing
/// reaches the child — and NORMAL is where every other pane already lives, so
/// spelling it out here would repeat the mode line on the title of the one
/// pane that has other things to say.
fn terminal_title(session: &crate::terminal::TerminalSession, active_mode: Option<Mode>) -> String {
    let mut name = match session.number() {
        Some(number) => format!("[terminal #{number}] {}", session.name()),
        None => format!("[terminal] {}", session.name()),
    };
    if active_mode == Some(Mode::Insert) {
        name.push_str(" [insert]");
    }
    if session.scroll() > 0 {
        name.push_str(&format!(" \u{2191}{}", session.scroll()));
    }
    if session.reviewing() {
        name.push_str(" [review]");
        if session.review_has_newer_output() {
            name.push_str(" [new output]");
        }
    }
    if session.history_truncated() {
        name.push_str(" [history truncated]");
    }
    if !session.live() {
        name.push_str(match session.exit_code() {
            Some(Some(code)) if code != 0 => return format!("{name} [exited {code}]"),
            _ => " [exited]",
        });
    }
    name
}

#[cfg(test)]
#[path = "snapshot/tests/long_lines.rs"]
mod long_line_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn escaped_tree_pending_text_fits_its_wire_bound_without_losing_unicode_boundaries() {
        let pending = format!("→ {}", "a".repeat(4094));
        let rendered = super::escape_tree_controls(&pending);
        assert_eq!(rendered.len(), super::MAX_DIRECTORY_TREE_ROW_TEXT_BYTES);
        assert!(rendered.ends_with('…'));
        assert!(rendered.starts_with("→ "));
        assert_eq!(super::escape_tree_controls("雪\n"), "雪\\n");
    }
    use super::*;
    use crate::{
        command::{CommandExecutionContext, CommandInvocation, EditorCommand},
        config::Config,
        input::{KeyCode, KeyStroke, Modifiers},
        jumplist::SelectionSemantics,
        selection::{Range, Selection},
        text::Transaction,
    };

    #[test]
    fn selection_role_walk_matches_range_semantics_at_boundaries_and_after_a_backward_jump() {
        let selections = [
            Selection::new(
                vec![Range::new(3, 5), Range::new(12, 10), Range::point(18)],
                1,
            ),
            Selection::new(vec![Range::new(3, 5), Range::new(5, 8)], 1),
            // Distinct adjacent ranges can point their heads at one offset.
            Selection::new(vec![Range::new(3, 5), Range::new(8, 5)], 1),
            Selection::single(Range::new(3, 5)),
            Selection::new(vec![Range::point(3), Range::point(8)], 0),
        ];
        for selection in &selections {
            for semantics in [
                SelectionSemantics::Runyte,
                SelectionSemantics::HalfOpen,
                SelectionSemantics::VimLinewise,
            ] {
                for select_mode in [false, true] {
                    for pristine in [false, true] {
                        for replacing in [false, true] {
                            let mut roles = SelectionRoles::new(
                                selection.ranges(),
                                selection.primary_index(),
                                10,
                                select_mode,
                                pristine,
                                replacing,
                                semantics,
                            );
                            // The second pass simulates a structured row that
                            // returns to an earlier document offset.
                            for offset in (10..25).chain(0..25) {
                                let ranges = selection.ranges();
                                let head = ranges.iter().any(|range| range.head == offset);
                                let primary_head = selection.primary().head == offset;
                                let expected = if replacing && head {
                                    if primary_head {
                                        TextRole::ReplaceCaret
                                    } else {
                                        TextRole::SecondaryCaret
                                    }
                                } else if select_mode && selection.primary().head == offset {
                                    TextRole::PrimaryCaret
                                } else if head && !pristine {
                                    if primary_head {
                                        TextRole::Caret
                                    } else {
                                        TextRole::SecondaryCaret
                                    }
                                } else if pristine
                                    && semantics == SelectionSemantics::Runyte
                                    && ranges
                                        .iter()
                                        .any(|range| range.is_empty() && range.head == offset)
                                {
                                    TextRole::Selected
                                } else {
                                    ranges
                                        .iter()
                                        .enumerate()
                                        .find(|(_, range)| {
                                            !range.is_empty()
                                                && range.from() <= offset
                                                && if semantics == SelectionSemantics::Runyte {
                                                    offset <= range.to()
                                                } else {
                                                    offset < range.to()
                                                }
                                        })
                                        .map_or(TextRole::Plain, |(index, _)| {
                                            if select_mode && index == selection.primary_index() {
                                                TextRole::PrimarySelected
                                            } else if !select_mode
                                                && index == selection.primary_index()
                                                && selection.len() == 1
                                                && semantics == SelectionSemantics::Runyte
                                            {
                                                TextRole::Plain
                                            } else {
                                                TextRole::Selected
                                            }
                                        })
                                };
                                assert_eq!(
                                    roles.role_at(offset),
                                    expected,
                                    "{selection:?}, {semantics:?}, {select_mode}, {pristine}, {replacing}, {offset}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn offscreen_carets_do_not_change_visible_wrapped_rows_or_selection_state() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        let mut app = App::new(config, None).unwrap();
        let first_line = format!("{}\n", "abcdefghijklmnopqrstuvwxyz ".repeat(8));
        let text = format!("{first_line}{}", "offscreen\n".repeat(1_000));
        app.buffers[0].apply(&Transaction::insert(0, text));
        let visible = [Range::new(0, 3), Range::point(25)];
        app.panes.get_mut(&0).unwrap().selection = Selection::new(visible.to_vec(), 0);
        app.mode = Mode::Select;
        let baseline = prepared_snapshot(&mut app, 24, 12)
            .pane(0)
            .unwrap()
            .rows
            .clone();

        let offscreen = (0..1_000).map(|row| Range::point(first_line.len() + row * 10));
        app.panes.get_mut(&0).unwrap().selection =
            Selection::new(visible.into_iter().chain(offscreen).collect(), 0);
        let with_offscreen = prepared_snapshot(&mut app, 24, 12);
        assert_eq!(with_offscreen.pane(0).unwrap().rows, baseline);
        assert_eq!(app.panes[&0].selection.len(), 1_002);
    }

    #[test]
    fn wrapped_table_keeps_selection_roles_on_continuation_rows() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        let mut app = App::new(config, None).unwrap();
        let source = "| Name | Description |\n| --- | --- |\n| A | **alpha beta gamma** |\n";
        app.buffers[0].apply(&Transaction::insert(0, source));
        app.handle_key(KeyStroke::char('?')).unwrap();
        let rendered = app.active_buffer().to_string();
        let start = rendered[..rendered.find("alpha").unwrap()].chars().count();
        let end = rendered[..rendered.find("gamma").unwrap()].chars().count() + "gamma".len() - 1;
        app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(start, end));
        app.mode = Mode::Select;

        let snapshot = prepared_snapshot(&mut app, 21, 20);
        let selected_rows = snapshot
            .pane(0)
            .unwrap()
            .rows
            .iter()
            .filter(|row| {
                let SnapshotRow::Text(row) = row else {
                    return false;
                };
                row.runs.iter().any(|run| {
                    matches!(
                        run.kind,
                        TextRunKind::Text {
                            role: TextRole::PrimarySelected | TextRole::PrimaryCaret,
                            ..
                        }
                    )
                })
            })
            .count();
        assert!(
            selected_rows >= 2,
            "selection should span wrapped table rows"
        );
        assert!(snapshot.pane(0).unwrap().rows.iter().any(|row| {
            let SnapshotRow::Text(row) = row else {
                return false;
            };
            row.runs.iter().any(|run| {
                matches!(
                    run.kind,
                    TextRunKind::Text {
                        role: TextRole::PrimaryCaret,
                        ..
                    }
                )
            })
        }));
    }

    fn prepared_snapshot(app: &mut App, width: u16, height: u16) -> EditorSnapshot {
        let geometry = crate::app::FrameGeometry {
            screen: Rect {
                x: 0,
                y: 0,
                width,
                height,
            },
            editor: Rect {
                x: 0,
                y: 0,
                width,
                height: height.saturating_sub(2),
            },
            status: Rect {
                x: 0,
                y: height.saturating_sub(2),
                width,
                height: 1,
            },
            message: Rect {
                x: 0,
                y: height.saturating_sub(1),
                width,
                height: 1,
            },
        };
        let prepared = app.prepare_view(geometry);
        app.snapshot(&prepared)
    }

    #[cfg(unix)]
    #[test]
    fn status_snapshot_handles_a_non_utf8_working_directory_lossily() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let mut app = App::new(Config::default(), None).unwrap();
        app.working_directory = PathBuf::from(OsString::from_vec(b"/workspace/\xfftail".to_vec()));

        let snapshot = prepared_snapshot(&mut app, 80, 24);

        assert_eq!(
            snapshot.status.workspace_directory,
            "/workspace/\u{fffd}tail"
        );
    }

    #[test]
    fn typed_setting_prompt_owns_a_popup_and_not_the_message_line() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.mode = Mode::Command;
        app.prompt_kind = crate::app::PromptKind::SettingValue(
            crate::settings::SettingId::GitRefreshIntervalSeconds,
        );
        app.command = "5".to_owned();
        app.command_cursor = 1;

        let overlay = app
            .overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == OverlayKind::Prompt)
            .unwrap();
        assert!(overlay.title.contains("integer 0–3600"));
        assert_eq!(overlay.query, "5");
        assert_eq!(overlay.query_cursor, Some(1));

        let snapshot = prepared_snapshot(&mut app, 80, 24);
        assert!(snapshot.status.interaction_line.is_empty());
        assert_eq!(snapshot.status.prompt_cursor_column, None);
    }

    #[test]
    fn scalar_prompts_exist_only_on_the_interaction_line() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.mode = Mode::Command;
        app.prompt_kind = crate::app::PromptKind::Search(crate::app::SearchMode::Insensitive);
        app.command = "needle".to_owned();
        app.command_cursor = app.command.chars().count();

        assert!(
            app.overlay_snapshots()
                .iter()
                .all(|overlay| overlay.kind != OverlayKind::Prompt)
        );
        let snapshot = prepared_snapshot(&mut app, 80, 24);
        assert_eq!(snapshot.status.interaction_line, "search: needle");
        assert_eq!(snapshot.status.prompt_cursor_column, Some(14));
    }

    #[test]
    fn whitespace_markers_preserve_cells_and_distinguish_real_line_endings() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        config.editor.tab_width = 4;
        config.editor.render_whitespace = true;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "a \tb\r\nlast"));

        let snapshot = prepared_snapshot(&mut app, 20, 8);
        let SnapshotRow::Text(first) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        let rendered = first
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(rendered, "a·→ b¬");
        assert_eq!(display_cells(&rendered), 6);
        assert!(first.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                whitespace: true,
                ..
            } if run.text == "·→ "
        )));
        let ending_marker = first
            .runs
            .iter()
            .find(|run| {
                matches!(
                    run.kind,
                    TextRunKind::Text {
                        whitespace: true,
                        ..
                    } if run.text == "¬"
                )
            })
            .expect("CRLF has a whitespace-styled ending marker");
        assert_eq!(display_cells(&ending_marker.text), 1);

        let SnapshotRow::Text(last) = &snapshot.pane(0).unwrap().rows[1] else {
            panic!("last row is text");
        };
        assert!(!last.runs.iter().any(|run| run.text.contains('¬')));

        app.config.editor.render_whitespace = false;
        let snapshot = prepared_snapshot(&mut app, 20, 8);
        let SnapshotRow::Text(first) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        assert_eq!(
            first
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "a   b"
        );
    }

    #[test]
    fn overlay_kind_inventory_is_exhaustive_and_semantically_typed() {
        assert_eq!(OverlayKind::ALL.len(), 16);
        let mut app = App::new(Config::default(), None).unwrap();
        app.execute(crate::command::CommandInvocation::service_health())
            .unwrap();
        let report = app.overlay_snapshots().pop().unwrap();
        assert_eq!(report.kind, OverlayKind::ResultList);
        assert_eq!(report.purpose, OverlayPurpose::Report);
        assert_eq!(report.input, OverlayInput::None);
        assert_eq!(report.selected, None);
        assert!(
            report
                .actions
                .iter()
                .any(|action| action.label == "dismiss")
        );
    }

    #[test]
    fn long_hover_is_honestly_bounded_and_enter_opens_the_complete_buffer() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.hover = Some(crate::app::HoverState {
            lines: (0..20).map(|row| format!("documentation {row}")).collect(),
        });
        let overlay = app.overlay_snapshots().pop().unwrap();
        assert_eq!(overlay.kind, OverlayKind::Hover);
        assert_eq!(overlay.rows.len(), 12);
        assert_eq!(overlay.total_rows, 20);
        assert_eq!(overlay.omitted_rows, 8);
        assert!(
            overlay
                .actions
                .iter()
                .any(|action| action.label.contains("complete documentation"))
        );

        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Enter,
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        assert_eq!(app.active_buffer().display_name(), "[documentation]");
        assert!(app.active_buffer().to_string().contains("documentation 19"));
    }

    #[test]
    fn large_buffers_copy_only_viewport_rows() {
        let mut app = App::new(Config::default(), None).unwrap();
        let content = (0..20_000)
            .map(|row| format!("row-{row:05}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.buffers[0].apply(&Transaction::insert(0, &content));
        let offset = app.buffers[0].line_to_offset(10_000);
        app.panes.get_mut(&0).unwrap().selection = crate::selection::Selection::point(offset);

        let snapshot = prepared_snapshot(&mut app, 80, 24);
        let pane = snapshot.pane(0).unwrap();

        assert_eq!(pane.rows.len(), pane.body.height as usize);
        assert!(pane.rows.len() < 30);
        assert!(
            pane.rows
                .iter()
                .any(|row| matches!(row, SnapshotRow::Text(row) if row.document_row == 10_000))
        );
        assert!(
            !pane
                .rows
                .iter()
                .any(|row| matches!(row, SnapshotRow::Text(row) if row.document_row == 0))
        );
        assert!(
            !pane
                .rows
                .iter()
                .any(|row| matches!(row, SnapshotRow::Text(row) if row.document_row == 19_999))
        );
    }

    /// A status snapshot whose only meaningful fields are the two the
    /// percentage reads.
    fn progress(row: usize, line_count: usize) -> u8 {
        StatusSnapshot {
            workspace_number: None,
            mode: Mode::Normal,
            workspace_directory: String::new(),
            dirty: false,
            external_file_status: ExternalFileStatus::Synchronized,
            read_only: false,
            cursor: Position { row, col: 0 },
            line_count,
            selection_count: 1,
            lsp_summary: None,
            git_summary: None,
            long_running_action: None,
            notification_counts: NotificationCounts::default(),
            interaction_line: String::new(),
            interaction_line_error: false,
            prompt_cursor_column: None,
        }
        .progress_percent()
    }

    #[test]
    fn progress_runs_from_the_first_row_to_the_last() {
        assert_eq!(progress(0, 101), 0);
        assert_eq!(progress(50, 101), 50);
        assert_eq!(progress(100, 101), 100);
        assert_eq!(progress(1, 5), 25);
        assert_eq!(progress(2, 5), 50);
    }

    #[test]
    fn only_the_two_ends_of_a_file_read_as_nought_and_a_hundred() {
        // A rounding step away from either end is pulled back, so neither
        // number can claim an end the cursor has not reached.
        assert_eq!(progress(0, 1_000), 0);
        assert_eq!(progress(1, 1_000), 1);
        assert_eq!(progress(2, 1_000), 1);
        assert_eq!(progress(997, 1_000), 99);
        assert_eq!(progress(998, 1_000), 99);
        assert_eq!(progress(999, 1_000), 100);
    }

    #[test]
    fn a_buffer_with_no_distance_to_cover_reads_as_complete() {
        assert_eq!(progress(0, 1), 100);
        assert_eq!(progress(0, 0), 100);
    }

    #[test]
    fn progress_follows_the_cursor_row_and_the_buffer_length() {
        let mut app = App::new(Config::default(), None).unwrap();
        let content = (0..201)
            .map(|row| format!("row-{row:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.buffers[0].apply(&Transaction::insert(0, &content));

        let snapshot = prepared_snapshot(&mut app, 80, 24);
        assert_eq!(snapshot.status.line_count, 201);
        assert_eq!(snapshot.status.progress_percent(), 0);

        let offset = app.buffers[0].line_to_offset(100);
        app.panes.get_mut(&0).unwrap().selection = crate::selection::Selection::point(offset);
        let snapshot = prepared_snapshot(&mut app, 80, 24);
        assert_eq!(snapshot.status.cursor.row, 100);
        assert_eq!(snapshot.status.progress_percent(), 50);

        let offset = app.buffers[0].line_to_offset(200);
        app.panes.get_mut(&0).unwrap().selection = crate::selection::Selection::point(offset);
        let snapshot = prepared_snapshot(&mut app, 80, 24);
        assert_eq!(snapshot.status.progress_percent(), 100);
    }

    #[test]
    fn scrolling_a_pane_away_from_the_cursor_leaves_progress_alone() {
        // The row and column beside it read the cursor; the percentage is
        // another reading of the same position, not of the viewport.
        let mut app = App::new(Config::default(), None).unwrap();
        let content = (0..201)
            .map(|row| format!("row-{row:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.buffers[0].apply(&Transaction::insert(0, &content));
        prepared_snapshot(&mut app, 80, 24);

        app.handle_key(KeyStroke::char('Z')).unwrap();
        for _ in 0..40 {
            app.handle_key(KeyStroke::char('j')).unwrap();
        }
        let snapshot = prepared_snapshot(&mut app, 80, 24);

        assert_eq!(snapshot.status.cursor.row, 0);
        assert_eq!(snapshot.status.progress_percent(), 0);
    }

    #[test]
    fn snapshots_cover_narrow_normal_and_wide_geometry() {
        for width in [8, 80, 180] {
            let mut app = App::new(Config::default(), None).unwrap();
            app.buffers[0].apply(&Transaction::insert(0, "alpha\nbeta"));
            let snapshot = prepared_snapshot(&mut app, width, 12);
            let pane = snapshot.pane(0).unwrap();
            assert_eq!(pane.area.width, width);
            assert_eq!(pane.rows.len(), 8);
            assert_eq!(snapshot.geometry.status.width, width);
        }
    }

    #[test]
    fn completed_binding_feedback_is_owned_by_the_snapshot_interaction_line() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "alpha beta"));

        for character in ['g', 'l'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }

        assert_eq!(app.status, "g …", "semantic status remains undecorated");
        let snapshot = prepared_snapshot(&mut app, 80, 8);
        assert_eq!(snapshot.status.interaction_line, "g l (Move to line end)");
    }

    #[test]
    fn a_chord_prefix_echoes_live_on_the_interaction_line_before_it_resolves() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.handle_key(KeyStroke::char('g')).unwrap();

        let snapshot = prepared_snapshot(&mut app, 80, 8);
        assert_eq!(snapshot.status.interaction_line, "g …");
        assert!(!snapshot.status.interaction_line_error);
    }

    #[test]
    fn a_numeric_count_echoes_live_on_the_interaction_line() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.handle_key(KeyStroke::char('3')).unwrap();

        let snapshot = prepared_snapshot(&mut app, 80, 8);
        assert_eq!(snapshot.status.interaction_line, "3 …");
        assert!(!snapshot.status.interaction_line_error);
    }

    #[test]
    fn a_count_followed_by_a_chord_prefix_echoes_both_live() {
        let mut app = App::new(Config::default(), None).unwrap();
        for character in ['3', 'g'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }

        let snapshot = prepared_snapshot(&mut app, 80, 8);
        assert_eq!(snapshot.status.interaction_line, "3 g …");
    }

    #[test]
    fn a_count_applies_and_the_interaction_line_settles_on_the_completed_action() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "alpha beta gamma delta"));
        for character in ['3', 'w'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }

        let snapshot = prepared_snapshot(&mut app, 80, 8);
        assert_eq!(
            snapshot.status.interaction_line,
            "3 w (Move to next word start)"
        );
        // The third `w` selects `gamma ` and leaves its head on the space.
        assert_eq!(app.active().cursor(&app.buffers[0]).col, 16);
    }

    #[test]
    fn a_character_awaiting_command_echoes_its_description_live() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "alpha beta"));
        app.handle_key(KeyStroke::char('f')).unwrap();

        let snapshot = prepared_snapshot(&mut app, 80, 8);
        assert_eq!(snapshot.status.interaction_line, "Find next character …");
        assert!(!snapshot.status.interaction_line_error);
    }

    #[test]
    fn a_failed_binding_marks_the_interaction_line_as_an_error_but_an_unavailable_one_does_not() {
        let mut failed = App::new(Config::default(), None).unwrap();
        for character in ['2', ' ', 'r'] {
            failed.handle_key(KeyStroke::char(character)).unwrap();
        }
        let failed_snapshot = prepared_snapshot(&mut failed, 80, 8);
        assert!(failed_snapshot.status.interaction_line_error);
        assert!(
            failed_snapshot
                .status
                .interaction_line
                .contains("· failed: "),
            "{}",
            failed_snapshot.status.interaction_line
        );

        let mut unavailable = App::new(Config::default(), None).unwrap();
        unavailable.handle_key(KeyStroke::char('|')).unwrap();
        let unavailable_snapshot = prepared_snapshot(&mut unavailable, 80, 8);
        assert!(!unavailable_snapshot.status.interaction_line_error);
        assert!(
            unavailable_snapshot
                .status
                .interaction_line
                .contains("· unavailable: "),
            "{}",
            unavailable_snapshot.status.interaction_line
        );
    }

    #[test]
    fn a_later_host_failure_does_not_replace_completed_binding_feedback() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "alpha beta"));
        for character in ['g', 'l'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }

        app.report_host_error("service dispatch failed");

        let snapshot = prepared_snapshot(&mut app, 80, 8);
        assert_eq!(snapshot.status.interaction_line, "g l (Move to line end)");
        assert!(!snapshot.status.interaction_line_error);
        assert_eq!(snapshot.status.notification_counts.errors, 1);
    }

    #[test]
    fn soft_wrapped_splits_snapshot_only_their_visible_rows() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        config.editor.line_numbers = false;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(
            0,
            "alpha bravo charlie delta echo foxtrot\nsecond wrapped line",
        ));
        for character in [' ', 'w', 'v'] {
            app.handle_key(KeyStroke::plain(KeyCode::Char(character)))
                .unwrap();
        }

        let snapshot = prepared_snapshot(&mut app, 32, 12);

        assert_eq!(snapshot.panes.len(), 2);
        assert!(
            snapshot
                .panes
                .iter()
                .all(|pane| pane.rows.len() == pane.body.height as usize)
        );
        assert!(snapshot.panes.iter().any(|pane| {
            pane.rows
                .iter()
                .any(|row| matches!(row, SnapshotRow::Text(row) if row.continuation))
        }));
    }

    #[test]
    fn semantic_runs_keep_carets_selections_and_jump_labels_owned() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "alpha beta"));
        app.panes.get_mut(&0).unwrap().selection =
            crate::selection::Selection::single(Range::new(0, 2));
        app.mode = Mode::Select;
        app.jump = crate::jump_labels::JumpLabels::new([6]);

        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };

        assert!(row.runs.iter().any(|run| {
            matches!(
                run.kind,
                TextRunKind::Text {
                    role: TextRole::PrimaryCaret,
                    ..
                }
            )
        }));
        assert!(row.runs.iter().any(|run| {
            matches!(
                run.kind,
                TextRunKind::Text {
                    role: TextRole::PrimarySelected,
                    ..
                }
            )
        }));
        assert!(
            row.runs
                .iter()
                .any(|run| { matches!(run.kind, TextRunKind::JumpLabel(LabelPart::Immediate)) })
        );
    }

    #[test]
    fn jump_dimming_belongs_only_to_the_active_pane() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "alpha beta"));
        for key in [
            KeyStroke::new(KeyCode::Char('w'), Modifiers::CONTROL),
            KeyStroke::char('v'),
        ] {
            app.handle_key(key).unwrap();
        }
        app.jump = crate::jump_labels::JumpLabels::new([6]);

        let snapshot = prepared_snapshot(&mut app, 80, 12);
        assert_eq!(snapshot.panes.len(), 2);
        assert!(
            snapshot
                .panes
                .iter()
                .any(|pane| pane.active && pane.jump_active)
        );
        assert!(
            snapshot
                .panes
                .iter()
                .any(|pane| !pane.active && !pane.jump_active)
        );
    }

    #[test]
    fn a_command_prompt_dims_every_pane_and_a_pending_chord_dims_none() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "alpha beta"));
        for key in [
            KeyStroke::new(KeyCode::Char('w'), Modifiers::CONTROL),
            KeyStroke::char('v'),
        ] {
            app.handle_key(key).unwrap();
        }

        let snapshot = prepared_snapshot(&mut app, 80, 12);
        assert_eq!(snapshot.panes.len(), 2);
        assert!(snapshot.panes.iter().all(|pane| !pane.dimmed));

        // A pending `g` is not a mode, so nothing dims while it waits.
        app.handle_key(KeyStroke::char('g')).unwrap();
        let snapshot = prepared_snapshot(&mut app, 80, 12);
        assert!(snapshot.panes.iter().all(|pane| !pane.dimmed));
        app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
            .unwrap();

        app.handle_key(KeyStroke::char(':')).unwrap();
        let snapshot = prepared_snapshot(&mut app, 80, 12);
        assert_eq!(snapshot.mode, Mode::Command);
        assert!(
            snapshot.panes.iter().all(|pane| pane.dimmed),
            "the prompt takes the keyboard from every pane, not just the active one"
        );
        assert!(
            snapshot.panes.iter().all(|pane| !pane.jump_active),
            "dimming for a prompt must not claim `goto-word` is labelling"
        );

        app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
            .unwrap();
        let snapshot = prepared_snapshot(&mut app, 80, 12);
        assert!(snapshot.panes.iter().all(|pane| !pane.dimmed));
    }

    #[test]
    fn a_search_prompt_dims_the_panes_too() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.handle_key(KeyStroke::char('s')).unwrap();

        let snapshot = prepared_snapshot(&mut app, 80, 12);
        assert_eq!(snapshot.mode, Mode::Command);
        assert!(snapshot.panes.iter().all(|pane| pane.dimmed));
    }

    #[test]
    fn the_command_dim_can_be_turned_off() {
        let mut config = Config::default();
        config.editor.command_mode_dim = false;
        let mut app = App::new(config, None).unwrap();
        app.handle_key(KeyStroke::char(':')).unwrap();

        let snapshot = prepared_snapshot(&mut app, 80, 12);
        assert_eq!(snapshot.mode, Mode::Command);
        assert!(snapshot.panes.iter().all(|pane| !pane.dimmed));
    }

    #[test]
    fn an_open_search_prompt_draws_the_matches_enter_would_select() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "at xx at yy"));
        for character in ['s', 'a', 't'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }

        let snapshot = prepared_snapshot(&mut app, 40, 8);
        assert_eq!(snapshot.mode, Mode::Command);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        let roles = row
            .runs
            .iter()
            .filter_map(|run| match run.kind {
                TextRunKind::Text { role, .. } if role != TextRole::Plain => {
                    Some((run.text.as_str(), role))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                ("a", TextRole::PrimarySelected),
                ("t", TextRole::PrimaryCaret),
                ("at", TextRole::Selected),
            ]
        );
    }

    #[test]
    fn pristine_search_hides_secondary_carets_until_the_selection_moves() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "at xx at yy"));
        for character in ['s', 'a', 't'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }
        app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
            .unwrap();

        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        assert!(row.runs.iter().any(|run| matches!(
            &run.kind,
            TextRunKind::Text {
                role: TextRole::PrimarySelected,
                ..
            } if run.text == "a"
        )));
        assert!(row.runs.iter().any(|run| matches!(
            &run.kind,
            TextRunKind::Text {
                role: TextRole::PrimaryCaret,
                ..
            } if run.text == "t"
        )));
        assert!(row.runs.iter().any(|run| matches!(
            &run.kind,
            TextRunKind::Text {
                role: TextRole::Selected,
                ..
            } if run.text == "at"
        )));
        assert!(!row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::Caret | TextRole::ReplaceCaret | TextRole::SecondaryCaret,
                ..
            }
        )));

        app.handle_key(KeyStroke::char('l')).unwrap();
        assert_eq!(app.status, "2 selections");
        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        assert!(row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::PrimaryCaret,
                ..
            }
        )));
        assert!(row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::SecondaryCaret,
                ..
            }
        )));
    }

    #[test]
    fn a_single_character_search_draws_every_match_and_not_only_the_primary() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "sun set sea"));
        for character in ['s', 's'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }
        app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
            .unwrap();

        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        // The primary match is the caret, and the other two are drawn as
        // selected rather than left plain until the selection moves.
        assert_eq!(
            row.runs
                .iter()
                .filter(|run| matches!(
                    &run.kind,
                    TextRunKind::Text {
                        role: TextRole::PrimaryCaret,
                        ..
                    } if run.text == "s"
                ))
                .count(),
            1
        );
        assert_eq!(
            row.runs
                .iter()
                .filter(|run| matches!(
                    &run.kind,
                    TextRunKind::Text {
                        role: TextRole::Selected,
                        ..
                    } if run.text == "s"
                ))
                .count(),
            2
        );
        assert!(!row.runs.iter().any(|run| matches!(
            &run.kind,
            TextRunKind::Text {
                role: TextRole::Plain,
                ..
            } if run.text.contains('s')
        )));
    }

    /// Rotating the primary chooses which match leads. It does not move a
    /// range, so the result must keep reading as a set of matches rather than
    /// turning every head into a caret.
    #[test]
    fn rotating_the_primary_keeps_a_search_result_drawn_as_matches() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "at xx at"));
        for character in ['s', 'a', 't'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }
        app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
            .unwrap();
        app.handle_key(KeyStroke::char(')')).unwrap();

        assert_eq!(app.status, "match 2/2 (all selected): at");
        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        let drawn: Vec<(TextRole, &str)> = row
            .runs
            .iter()
            .map(|run| match &run.kind {
                TextRunKind::Text { role, .. } => (*role, run.text.as_str()),
                kind => panic!("unexpected run: {kind:?}"),
            })
            .collect();
        assert_eq!(
            drawn,
            vec![
                (TextRole::Selected, "at"),
                (TextRole::Plain, " xx "),
                (TextRole::PrimarySelected, "a"),
                (TextRole::PrimaryCaret, "t"),
            ]
        );
    }

    #[test]
    fn pending_replace_distinguishes_the_primary_caret() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "at xx at"));
        for character in ['s', 'a', 't'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }
        app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
            .unwrap();
        app.handle_key(KeyStroke::char('r')).unwrap();

        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        let replace_carets = row
            .runs
            .iter()
            .filter(|run| {
                matches!(
                    run.kind,
                    TextRunKind::Text {
                        role: TextRole::ReplaceCaret,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(replace_carets, 1);
        assert!(row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::SecondaryCaret,
                ..
            }
        )));

        app.handle_key(KeyStroke::char('z')).unwrap();
        assert_eq!(app.buffers[0].to_string(), "zz xx zz");
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn a_lone_normal_range_is_drawn_only_while_motions_select() {
        let selected_text = |app: &mut App| {
            let snapshot = prepared_snapshot(app, 40, 8);
            let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
                panic!("first row is text");
            };
            row.runs
                .iter()
                .filter(|run| {
                    matches!(
                        run.kind,
                        TextRunKind::Text {
                            role: TextRole::Selected,
                            ..
                        }
                    )
                })
                .map(|run| run.text.clone())
                .collect::<String>()
        };
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "one two"));
        app.panes.get_mut(&0).unwrap().selection =
            crate::selection::Selection::single(Range::new(0, 3));
        assert_eq!(app.mode, Mode::Normal);
        // On by default: the head is drawn as the caret and the rest of the
        // range is selected.
        assert_eq!(selected_text(&mut app), "one");

        app.config.editor.selecting_motions = false;
        assert_eq!(selected_text(&mut app), "");
    }

    #[test]
    fn a_multiselection_marks_its_complete_primary_range_separately() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "one two"));
        app.panes.get_mut(&0).unwrap().selection =
            crate::selection::Selection::new(vec![Range::new(0, 2), Range::new(4, 6)], 1);
        app.mode = Mode::Select;

        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        assert!(row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::PrimaryCaret,
                ..
            }
        )));
        assert!(row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::SecondaryCaret,
                ..
            }
        )));
        assert!(row.runs.iter().any(|run| matches!(
            &run.kind,
            TextRunKind::Text {
                role: TextRole::PrimaryCaret,
                ..
            } if run.text == "o"
        )));
        assert!(row.runs.iter().any(|run| matches!(
            &run.kind,
            TextRunKind::Text {
                role: TextRole::PrimarySelected,
                ..
            } if run.text == "tw"
        )));

        app.panes.get_mut(&0).unwrap().selection =
            crate::selection::Selection::new(vec![Range::point(0), Range::point(7)], 1);
        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        assert!(row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::PrimaryCaret,
                ..
            } if run.text == " "
        )));

        app.mode = Mode::Insert;
        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        assert!(!row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::PrimaryCaret,
                ..
            }
        )));
    }

    #[test]
    fn explorer_rows_carry_directory_semantics_into_the_snapshot() {
        let directory = std::env::temp_dir().join(format!(
            "runyte-snapshot-directory-color-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(directory.join("nested")).unwrap();
        std::fs::write(directory.join("file.txt"), "text").unwrap();
        let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();

        let snapshot = prepared_snapshot(&mut app, 100, 12);
        let rows = &snapshot.pane(0).unwrap().rows;
        // Directories group before files in every listing order, so `nested/`
        // leads however it sorts against `file.txt` by name.
        let SnapshotRow::Text(nested) = &rows[0] else {
            panic!("directory row is text");
        };
        let SnapshotRow::Text(file) = &rows[1] else {
            panic!("file row is text");
        };
        assert!(file.runs.iter().all(|run| !matches!(
            run.kind,
            TextRunKind::Text {
                directory: true,
                ..
            }
        )));
        assert!(nested.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                directory: true,
                ..
            }
        )));

        app.handle_key(crate::input::KeyStroke::char('?')).unwrap();
        let snapshot = prepared_snapshot(&mut app, 100, 12);
        let pane = snapshot.pane(0).unwrap();
        let SnapshotRow::Text(file) = &pane.rows[1] else {
            panic!("file row is text");
        };
        assert!(matches!(
            file.runs.first(),
            Some(TextRun {
                text,
                kind: TextRunKind::Hint,
            }) if !cfg!(unix) || text.starts_with("-rw")
        ));
        assert_eq!(
            file.runs
                .iter()
                .filter(|run| matches!(run.kind, TextRunKind::Text { .. }))
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "file.txt"
        );
        assert!(
            file.runs
                .iter()
                .map(|run| display_cells(&run.text))
                .sum::<usize>()
                <= pane.text_width
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn syntax_colors_remain_owned_scopes_instead_of_frontend_styles() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "fn main() {}"));
        let language = app.registry.language_for_name("rust").unwrap();
        app.syntax[0] =
            crate::syntax::DocumentSyntax::new(app.buffers[0].text(), language, &app.registry);

        let snapshot = prepared_snapshot(&mut app, 40, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };

        assert!(row.runs.iter().any(|run| {
            matches!(
                run.kind,
                TextRunKind::Text {
                    scope: Some(scope),
                    ..
                } if scope.name() == "keyword"
            )
        }));
    }

    fn lua_snapshot_fixture(lines: usize) -> App {
        let mut app = App::new(Config::default(), None).unwrap();
        let source = (0..lines)
            .map(|index| format!("local function item_{index}(x) return x + {index} end\n"))
            .collect::<String>();
        app.buffers[0].apply(&Transaction::insert(0, source));
        let language = app.registry.language_for_name("lua").unwrap();
        app.syntax[0] =
            crate::syntax::DocumentSyntax::new(app.buffers[0].text(), language, &app.registry);
        app
    }

    fn split_snapshot_fixture(panes: usize) -> App {
        let mut app = lua_snapshot_fixture(200);
        for _ in 1..panes {
            app.execute(CommandInvocation::split_vertical(None))
                .unwrap();
        }
        app
    }

    #[test]
    fn inactive_panes_reuse_owned_rows_while_active_cursor_and_status_move() {
        for pane_count in [2, 4] {
            let mut app = split_snapshot_fixture(pane_count);
            let first = prepared_snapshot(&mut app, 120, 40);
            assert_eq!(app.visible_pane_rebuilds.get(), pane_count - 1);
            let inactive = first.panes.iter().find(|pane| !pane.active).unwrap();
            let inactive_id = inactive.pane_id;
            let original_rows = inactive.rows.clone();
            let old_cursor = first.status.cursor;
            app.handle_key(KeyStroke::char('l')).unwrap();
            let second = prepared_snapshot(&mut app, 120, 40);
            assert_eq!(app.visible_pane_rebuilds.get(), pane_count - 1);
            assert_eq!(second.pane(inactive_id).unwrap().rows, original_rows);
            assert_ne!(second.status.cursor, old_cursor);
            assert_ne!(
                second.pane(app.active_pane).unwrap().rows,
                first.pane(app.active_pane).unwrap().rows
            );

            // Snapshots returned to a frontend own their rows independently
            // of the cache and of later frames.
            let mut changed = second;
            changed
                .panes
                .iter_mut()
                .find(|pane| pane.pane_id == inactive_id)
                .unwrap()
                .rows
                .clear();
            let third = prepared_snapshot(&mut app, 120, 40);
            assert_eq!(third.pane(inactive_id).unwrap().rows, original_rows);
            assert_eq!(app.visible_pane_rebuilds.get(), pane_count - 1);
            assert_eq!(app.visible_pane_snapshots.borrow().len(), pane_count - 1);
            app.layout = crate::layout::Layout::Pane(app.active_pane);
            let _ = prepared_snapshot(&mut app, 120, 40);
            assert!(app.visible_pane_snapshots.borrow().is_empty());
        }
    }

    #[test]
    fn inactive_pane_cache_invalidates_for_shared_text_projection_theme_and_diagnostics() {
        let mut app = split_snapshot_fixture(2);
        let inactive = app
            .panes
            .keys()
            .copied()
            .find(|&id| id != app.active_pane)
            .unwrap();
        let first = prepared_snapshot(&mut app, 120, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 1);

        // Both panes show the same buffer; an edit in the active pane must
        // rebuild the inactive pane even when its viewport stays put.
        app.buffers[0].apply(&Transaction::insert(0, "-- changed\n"));
        let edited = prepared_snapshot(&mut app, 120, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 2);
        assert_ne!(
            edited.pane(inactive).unwrap().rows,
            first.pane(inactive).unwrap().rows
        );

        let _ = prepared_snapshot(&mut app, 100, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 3);
        app.config.editor.soft_wrap = true;
        let _ = prepared_snapshot(&mut app, 100, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 4);
        app.theme.background = crate::config::Color::White;
        let _ = prepared_snapshot(&mut app, 100, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 5);
        app.config.editor.render_whitespace = true;
        let _ = prepared_snapshot(&mut app, 100, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 6);
        let language = app.registry.language_for_name("lua").unwrap();
        app.syntax[0] =
            crate::syntax::DocumentSyntax::new(app.buffers[0].text(), language, &app.registry);
        let _ = prepared_snapshot(&mut app, 100, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 7);
        app.mode = Mode::Select;
        let _ = prepared_snapshot(&mut app, 100, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 8);

        let path = std::path::PathBuf::from("/tmp/runyte-pane-cache-diagnostic.lua");
        app.buffers[0].path = Some(path.clone());
        let _ = prepared_snapshot(&mut app, 100, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 9);
        app.diagnostics.set(
            "lua",
            path,
            vec![crate::lsp::Diagnostic::new(Default::default())],
        );
        let diagnosed = prepared_snapshot(&mut app, 100, 40);
        assert_eq!(app.visible_pane_rebuilds.get(), 10);
        assert!(matches!(
            &diagnosed.pane(inactive).unwrap().rows[0],
            SnapshotRow::Text(row) if row.diagnostic_sign == Some(Severity::Error)
        ));

        app.active_pane = inactive;
        let focused = prepared_snapshot(&mut app, 100, 40);
        assert!(focused.pane(inactive).unwrap().active);
        assert!(!focused.pane(1).unwrap().active);
        assert!(app.visible_pane_snapshots.borrow().get(&inactive).is_none());
    }

    #[test]
    #[ignore = "manual fixed-geometry pane snapshot timing"]
    fn inactive_pane_snapshot_timing() {
        use std::time::Instant;

        for pane_count in [1, 2, 4] {
            for shared in [false, true]
                .into_iter()
                .filter(|shared| pane_count > 1 || *shared)
            {
                for cached in [false, true] {
                    let mut app = split_snapshot_fixture(pane_count);
                    if !shared {
                        for pane_id in 0..pane_count - 1 {
                            let mut buffer = app.buffers[0].clone();
                            buffer.apply(&Transaction::insert(0, format!("-- pane {pane_id}\n")));
                            let language = app.registry.language_for_name("lua").unwrap();
                            let syntax = crate::syntax::DocumentSyntax::new(
                                buffer.text(),
                                language,
                                &app.registry,
                            );
                            let buffer_id = app.buffers.len();
                            app.buffers.push(buffer);
                            app.syntax.push(syntax);
                            app.panes.get_mut(&pane_id).unwrap().buffer = buffer_id;
                        }
                    }
                    let _ = prepared_snapshot(&mut app, 120, 40);
                    let mut prepare = Vec::new();
                    let mut snapshot = Vec::new();
                    for index in 0..200 {
                        app.handle_key(KeyStroke::char(if index % 2 == 0 { 'l' } else { 'h' }))
                            .unwrap();
                        let start = Instant::now();
                        let geometry = crate::app::FrameGeometry {
                            screen: Rect {
                                x: 0,
                                y: 0,
                                width: 120,
                                height: 40,
                            },
                            editor: Rect {
                                x: 0,
                                y: 0,
                                width: 120,
                                height: 38,
                            },
                            status: Rect {
                                x: 0,
                                y: 38,
                                width: 120,
                                height: 1,
                            },
                            message: Rect {
                                x: 0,
                                y: 39,
                                width: 120,
                                height: 1,
                            },
                        };
                        let view = app.prepare_view(geometry);
                        prepare.push(start.elapsed().as_micros());
                        if !cached {
                            app.visible_pane_snapshots.borrow_mut().clear();
                        }
                        let start = Instant::now();
                        std::hint::black_box(app.snapshot(&view));
                        snapshot.push(start.elapsed().as_micros());
                    }
                    prepare.sort_unstable();
                    snapshot.sort_unstable();
                    eprintln!(
                        "{pane_count} panes, shared {shared}, cached {cached}: prepare {} us, snapshot {} us, inactive rebuilds {}",
                        prepare[100],
                        snapshot[100],
                        app.visible_pane_rebuilds.get()
                    );
                }
            }
        }
    }

    #[test]
    fn cursor_highlights_reuse_visible_spans_and_invalidate_on_edits_trees_and_scroll() {
        let mut app = lua_snapshot_fixture(200);
        let first = prepared_snapshot(&mut app, 120, 40);
        assert!(first.pane(0).unwrap().rows.iter().any(|row| {
            matches!(row, SnapshotRow::Text(row) if row.runs.iter().any(|run| {
                matches!(run.kind, TextRunKind::Text { scope: Some(_), .. })
            }))
        }));
        let (ranges, source, spans_pointer) = {
            let cache = app.visible_highlights.borrow();
            let cache = &cache[&0];
            assert_eq!(cache.ranges.len(), 1, "whole adjacent rows share a query");
            (
                cache.ranges.clone(),
                cache.syntax_source,
                cache.spans.as_ptr(),
            )
        };
        for row in first
            .pane(0)
            .unwrap()
            .rows
            .iter()
            .filter_map(|row| match row {
                SnapshotRow::Text(row) => Some(row.document_row),
                _ => None,
            })
        {
            let from = app.buffers[0].line_to_offset(row);
            let to = from + app.buffers[0].line_len(row);
            let separate = app.highlights(0, from, to);
            let cache = app.visible_highlights.borrow();
            for offset in from..to {
                let scope_at = |spans: &[Span]| {
                    spans
                        .iter()
                        .find(|span| span.from <= offset && offset < span.to)
                        .map(|span| span.scope)
                };
                assert_eq!(scope_at(&cache[&0].spans), scope_at(&separate));
            }
        }

        app.panes.get_mut(&0).unwrap().selection =
            Selection::point(app.buffers[0].line_to_offset(1));
        let _ = prepared_snapshot(&mut app, 120, 40);
        {
            let cache = app.visible_highlights.borrow();
            assert_eq!(cache[&0].ranges, ranges);
            assert_eq!(cache[&0].syntax_source, source);
            assert_eq!(cache[&0].spans.as_ptr(), spans_pointer);
        }

        // A newly published tree may start at syntax revision zero too.
        let language = app.registry.language_for_name("lua").unwrap();
        app.syntax[0] =
            crate::syntax::DocumentSyntax::new(app.buffers[0].text(), language, &app.registry);
        let _ = prepared_snapshot(&mut app, 120, 40);
        assert_ne!(app.visible_highlights.borrow()[&0].syntax_source, source);

        let old_revision = app.visible_highlights.borrow()[&0].text_revision;
        app.buffers[0].apply(&Transaction::insert(0, "-- new comment\n"));
        app.syntax[0] =
            crate::syntax::DocumentSyntax::new(app.buffers[0].text(), language, &app.registry);
        let _ = prepared_snapshot(&mut app, 120, 40);
        assert_ne!(
            app.visible_highlights.borrow()[&0].text_revision,
            old_revision
        );

        app.panes.get_mut(&0).unwrap().selection =
            Selection::point(app.buffers[0].line_to_offset(150));
        let _ = prepared_snapshot(&mut app, 120, 40);
        assert_ne!(app.visible_highlights.borrow()[&0].ranges, ranges);
    }

    #[test]
    fn visible_queries_skip_folded_lines_and_clip_a_long_line() {
        let mut app = App::new(Config::default(), None).unwrap();
        let source = format!(
            "fn outer() {{\n{}\n}}\nfn after() {{}}\n",
            "    let hidden = 1;\n".repeat(500)
        );
        app.buffers[0].apply(&Transaction::insert(0, source));
        let language = app.registry.language_for_name("rust").unwrap();
        app.syntax[0] =
            crate::syntax::DocumentSyntax::new(app.buffers[0].text(), language, &app.registry);
        for key in [' ', 'x', 'f'] {
            app.handle_key(crate::input::KeyStroke::char(key)).unwrap();
        }
        let _ = prepared_snapshot(&mut app, 80, 12);
        let folded = app.visible_highlights.borrow()[&0].ranges.clone();
        assert!(folded.windows(2).any(|pair| pair[1].0 - pair[0].1 > 1));
        assert!(folded.len() >= 2);

        for key in [' ', 'x', 'u'] {
            app.handle_key(crate::input::KeyStroke::char(key)).unwrap();
        }
        let _ = prepared_snapshot(&mut app, 80, 12);
        assert_ne!(app.visible_highlights.borrow()[&0].ranges, folded);

        let mut app = App::new(Config::default(), None).unwrap();
        let source = format!("-- {}\nlocal answer = 42\n", "x".repeat(100_000));
        app.buffers[0].apply(&Transaction::insert(0, source));
        let language = app.registry.language_for_name("lua").unwrap();
        app.syntax[0] =
            crate::syntax::DocumentSyntax::new(app.buffers[0].text(), language, &app.registry);
        let _ = prepared_snapshot(&mut app, 80, 8);
        let ranges = &app.visible_highlights.borrow()[&0].ranges;
        assert!(
            ranges
                .iter()
                .all(|&(from, to)| to - from <= 80 + ZERO_WIDTH_SCAN_LIMIT)
        );
        assert!(
            ranges.len() >= 2,
            "clipped line must not merge with the next"
        );
    }

    #[test]
    #[ignore = "manual snapshot timing fixture"]
    fn cursor_highlight_snapshot_timing() {
        use std::time::Instant;

        let mut app = lua_snapshot_fixture(2_000);
        for (width, height) in [(120, 40), (240, 80)] {
            for (label, invalidate) in [("uncached", true), ("cached", false)] {
                for _ in 0..10 {
                    if invalidate {
                        app.visible_highlights.borrow_mut().clear();
                    }
                    std::hint::black_box(prepared_snapshot(&mut app, width, height));
                }
                let mut samples = Vec::with_capacity(200);
                for _ in 0..200 {
                    if invalidate {
                        app.visible_highlights.borrow_mut().clear();
                    }
                    let start = Instant::now();
                    std::hint::black_box(prepared_snapshot(&mut app, width, height));
                    samples.push(start.elapsed().as_micros());
                }
                samples.sort_unstable();
                eprintln!(
                    "{width}x{height} {label} snapshot median: {} us",
                    samples[100]
                );
            }
        }
    }

    #[test]
    fn generated_help_colours_reach_the_semantic_snapshot() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.execute(
            CommandInvocation::editor(EditorCommand::ShowAbout, CommandExecutionContext::default())
                .unwrap(),
        )
        .unwrap();

        let snapshot = prepared_snapshot(&mut app, 100, 32);
        let scopes = snapshot
            .pane(0)
            .unwrap()
            .rows
            .iter()
            .filter_map(|row| match row {
                SnapshotRow::Text(row) => Some(&row.runs),
                _ => None,
            })
            .flatten()
            .filter_map(|run| match run.kind {
                TextRunKind::Text {
                    scope: Some(scope), ..
                } => Some(scope.name()),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(scopes.contains("markup.heading"));
        assert!(scopes.contains("function"));
        assert!(scopes.contains("keyword"));
    }

    #[test]
    fn prompt_cursor_uses_display_cells_at_a_unicode_midpoint() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.mode = Mode::Command;
        app.prompt_kind = crate::app::PromptKind::SearchForward;
        app.command = "界a".to_owned();
        app.command_cursor = 1;

        let snapshot = prepared_snapshot(&mut app, 40, 8);

        assert_eq!(snapshot.status.interaction_line, "/界a");
        assert_eq!(snapshot.status.prompt_cursor_column, Some(3));

        app.prompt_kind = crate::app::PromptKind::Command;
        app.command = "e\u{301}".to_owned();
        app.command_cursor = 2;
        let snapshot = prepared_snapshot(&mut app, 40, 8);
        assert_eq!(snapshot.status.prompt_cursor_column, Some(2));
    }

    #[test]
    fn combining_marks_do_not_clip_the_last_visible_cell() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "e\u{301}x"));

        let snapshot = prepared_snapshot(&mut app, 4, 6);
        let pane = snapshot.pane(0).unwrap();
        assert_eq!(pane.text_width, 2);
        let SnapshotRow::Text(row) = &pane.rows[0] else {
            panic!("first row is text");
        };
        let text = row
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(text, "e\u{301}x");

        app.buffers[0].apply(&Transaction::change(0, 3, "xe\u{301}"));
        let snapshot = prepared_snapshot(&mut app, 4, 6);
        let pane = snapshot.pane(0).unwrap();
        let SnapshotRow::Text(row) = &pane.rows[0] else {
            panic!("first row is text");
        };
        let text = row
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(text, "xe\u{301}");
    }

    #[test]
    fn zero_width_only_rows_have_a_bounded_scan() {
        assert_eq!(
            visible_character_end(std::iter::repeat_n('\u{301}', 1_000), 0, 2, 4),
            258
        );
    }

    #[test]
    fn soft_wrapped_zero_width_only_rows_have_a_bounded_snapshot() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        config.editor.soft_wrap = true;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "\u{301}".repeat(1_000)));

        let snapshot = prepared_snapshot(&mut app, 4, 6);
        let pane = snapshot.pane(0).unwrap();
        let SnapshotRow::Text(row) = &pane.rows[0] else {
            panic!("first row is text");
        };
        let copied = row
            .runs
            .iter()
            .map(|run| run.text.chars().count())
            .sum::<usize>();

        assert_eq!(pane.text_width, 2);
        assert_eq!(copied, pane.text_width + ZERO_WIDTH_SCAN_LIMIT);
    }

    #[test]
    fn extreme_tabs_and_wide_glyphs_cannot_outgrow_the_viewport() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        config.editor.tab_width = usize::MAX;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "\t界界界 trailing text"));

        let snapshot = prepared_snapshot(&mut app, 8, 6);
        let pane = snapshot.pane(0).unwrap();
        let SnapshotRow::Text(row) = &pane.rows[0] else {
            panic!("first row is text");
        };
        let copied = row
            .runs
            .iter()
            .map(|run| display_cells(&run.text))
            .sum::<usize>();

        assert!(copied <= pane.text_width);
        assert!(!row.runs.iter().any(|run| run.text.contains('界')));
    }

    #[test]
    fn a_full_width_row_does_not_append_an_offscreen_eol_caret() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        config.editor.soft_wrap = true;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "abcdef"));
        app.panes.get_mut(&0).unwrap().selection = crate::selection::Selection::point(6);

        let snapshot = prepared_snapshot(&mut app, 8, 6);
        let pane = snapshot.pane(0).unwrap();
        let SnapshotRow::Text(row) = &pane.rows[0] else {
            panic!("first row is text");
        };
        let cells = row
            .runs
            .iter()
            .map(|run| display_cells(&run.text))
            .sum::<usize>();

        assert_eq!(pane.text_width, 6);
        assert_eq!(cells, pane.text_width);
        assert!(
            !row.runs.iter().any(|run| {
                matches!(
                    run.kind,
                    TextRunKind::Text {
                        role: TextRole::PrimaryCaret | TextRole::Caret,
                        ..
                    }
                )
            }),
            "{row:?}"
        );
    }

    #[test]
    fn a_cell_range_preserves_columns_when_it_starts_inside_a_wide_glyph() {
        assert_eq!(clip_fragment_cell_range("ab界cd", 3, 3), " cd");
        assert_eq!(display_cells(&clip_fragment_cell_range("ab界cd", 3, 3)), 3);
    }

    #[cfg(unix)]
    #[test]
    fn a_narrow_explorer_can_scroll_from_permissions_to_the_filename() {
        let directory = std::env::temp_dir().join(format!(
            "runyte-snapshot-details-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("metadata.txt"), "text").unwrap();
        let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
        app.handle_key(KeyStroke::char('?')).unwrap();
        let full_prefix_width = app.active_buffer().row_prefix_width();

        let first = prepared_snapshot(&mut app, 30, 8);
        let SnapshotRow::Text(first_row) = &first.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        assert!(
            first_row.runs[0].text.starts_with("-rw"),
            "left edge begins at permissions: {first_row:?}"
        );

        app.panes.get_mut(&0).unwrap().row_prefix_scroll = full_prefix_width.saturating_sub(8);
        let last = prepared_snapshot(&mut app, 30, 8);
        let SnapshotRow::Text(last_row) = &last.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        assert_eq!(display_cells(&last_row.runs[0].text), 8);
        let text = last_row
            .runs
            .iter()
            .filter(|run| run.kind != TextRunKind::Hint)
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(
            text, "metadata.txt",
            "the editable filename receives the recovered width: {last_row:?}"
        );

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn explorer_rows_carry_their_symlink_target_as_a_read_only_hint() {
        let directory = std::env::temp_dir().join(format!(
            "runyte-snapshot-symlink-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("true_file.txt"), "text").unwrap();
        std::os::unix::fs::symlink("true_file.txt", directory.join("file.txt")).unwrap();
        let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();

        let snapshot = prepared_snapshot(&mut app, 60, 8);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("first row is text");
        };
        let SnapshotRow::Text(target_row) = &snapshot.pane(0).unwrap().rows[1] else {
            panic!("second row is text");
        };

        assert_eq!(
            row.runs
                .iter()
                .filter(|run| run.kind == TextRunKind::Hint)
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            ["  → true_file.txt"]
        );
        assert!(
            !target_row
                .runs
                .iter()
                .any(|run| run.kind == TextRunKind::Hint),
            "only a link is annotated: {target_row:?}"
        );
        assert!(
            row.runs
                .iter()
                .filter(|run| run.kind != TextRunKind::Hint)
                .all(|run| !run.text.contains('→')),
            "the hint is not part of the row's text: {row:?}"
        );

        std::fs::remove_dir_all(&directory).unwrap();
    }
}
