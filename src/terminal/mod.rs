// SPDX-License-Identifier: MPL-2.0

//! Live terminal sessions, and the only module aware that Runyte can run a
//! child process on a tty.
//!
//! A terminal is not a document. It has no rope, no transaction log, no undo
//! stack, no saved text and no disk state, because none of those answer a
//! question anyone asks of `htop`. It is therefore a *pane content type*
//! rather than a [`BufferKind`](crate::buffer::BufferKind): a pane showing a
//! terminal reads its rows from here and never from its buffer.
//!
//! The discipline matches [`crate::git`] and [`crate::lsp`]: one owner, one
//! module that knows about the subsystem, bounded state, and a boundary above
//! which nobody has heard of an escape sequence.
//!
//! What a pane shows is a [`TerminalView`] — a rectangle of styled cells and a
//! cursor. That is the deliberate hole in the otherwise semantic frame
//! protocol: a tree-sitter scope cannot name the colour a child chose, so
//! these cells carry literal colour and the frontend resolves only
//! [`Color::Default`] against the theme.

pub mod emulator;
pub mod grid;
pub mod keys;
pub mod parser;
#[cfg(unix)]
pub mod pty;

use std::{
    collections::{BTreeMap, VecDeque},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::SystemTime,
};

use tokio::sync::Notify;

use crate::input::KeyStroke;
use crate::jump_labels::{JumpLabels, LabelPart};
use crate::selection::{Range, Selection};

use emulator::Emulator;
pub use grid::{Attributes, Cell, Color};

/// The effective default colours a child may ask its terminal to report.
///
/// Either side may be unknown when the editor theme delegates it to the
/// outer terminal with [`crate::config::Color::Reset`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DefaultColors {
    foreground: Option<(u8, u8, u8)>,
    background: Option<(u8, u8, u8)>,
}

impl DefaultColors {
    pub(crate) const fn new(
        foreground: Option<(u8, u8, u8)>,
        background: Option<(u8, u8, u8)>,
    ) -> Self {
        Self {
            foreground,
            background,
        }
    }

    const fn foreground(self) -> Option<(u8, u8, u8)> {
        self.foreground
    }

    const fn background(self) -> Option<(u8, u8, u8)> {
        self.background
    }
}

/// Stable identity of one terminal session.
///
/// Ids are never reused inside a session, so a pane holding one that has been
/// closed learns that the terminal is gone rather than finding a different
/// one.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerminalId(u64);

impl TerminalId {
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for TerminalId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// What a running child produced, addressed to the session that owns it.
#[derive(Debug)]
pub enum TerminalOutput {
    Bytes { id: TerminalId, bytes: Vec<u8> },
    Exited { id: TerminalId, code: Option<i32> },
}

impl TerminalOutput {
    pub fn id(&self) -> TerminalId {
        match self {
            Self::Bytes { id, .. } | Self::Exited { id, .. } => *id,
        }
    }
}

/// What to run, and where.
#[derive(Clone, Debug)]
pub struct TerminalRequest {
    pub program: OsString,
    pub arguments: Vec<String>,
    pub directory: PathBuf,
    /// The name the pane carries until the child sets its own title.
    pub label: String,
}

/// Maximum output messages parsed in one host turn across all ready sessions.
pub const OUTPUT_QUEUE: usize = 32;

/// One child may retain this many unread PTY chunks before its own reader
/// blocks. The global ready list contains identities, not chunks, so a noisy
/// child occupies one slot and cannot crowd quiet sessions out of readiness.
/// A full queue blocks that reader, then the PTY and child, instead of growing
/// host memory without limit.
const PER_SESSION_OUTPUT_QUEUE: usize = 8;

/// A single host turn may parse at most this much terminal output in addition
/// to the message bound. This returns control to input and service events
/// quickly even when every read contains a complete 64 KiB PTY chunk.
const OUTPUT_BYTE_BUDGET: usize = 256 * 1024;

#[derive(Debug, Default)]
struct PendingOutput {
    bytes: VecDeque<Vec<u8>>,
    exit: Option<Option<i32>>,
    ready: bool,
}

#[derive(Debug, Default)]
struct OutputState {
    sessions: BTreeMap<TerminalId, PendingOutput>,
    ready: VecDeque<TerminalId>,
}

#[derive(Debug)]
struct OutputShared {
    state: Mutex<OutputState>,
    space: Condvar,
    available: Notify,
}

#[derive(Clone, Debug)]
struct TerminalEventSender(Arc<OutputShared>);

impl TerminalEventSender {
    fn register(&self, id: TerminalId) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sessions
            .entry(id)
            .or_default();
    }

    fn remove(&self, id: TerminalId) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.sessions.remove(&id);
        state.ready.retain(|ready| *ready != id);
        self.0.space.notify_all();
    }

    /// Called only by a PTY reader thread. A full queue blocks that reader,
    /// applying backpressure to this child alone rather than to other PTYs.
    fn send(&self, output: TerminalOutput) -> bool {
        let id = output.id();
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match output {
            TerminalOutput::Bytes { bytes, .. } => {
                while state
                    .sessions
                    .get(&id)
                    .is_some_and(|pending| pending.bytes.len() >= PER_SESSION_OUTPUT_QUEUE)
                {
                    state = self
                        .0
                        .space
                        .wait(state)
                        .unwrap_or_else(|error| error.into_inner());
                }
                let Some(pending) = state.sessions.get_mut(&id) else {
                    return false;
                };
                pending.bytes.push_back(bytes);
            }
            TerminalOutput::Exited { code, .. } => {
                let Some(pending) = state.sessions.get_mut(&id) else {
                    return false;
                };
                // Exit has its own slot and therefore cannot be hidden behind
                // a full data queue. It remains ordered after retained bytes.
                pending.exit = Some(code);
            }
        }
        let pending = state.sessions.get_mut(&id).expect("session is registered");
        if !pending.ready {
            pending.ready = true;
            state.ready.push_back(id);
        }
        drop(state);
        self.0.available.notify_one();
        true
    }
}

/// Fair, bounded output stream consumed by the editor event loop.
#[derive(Debug)]
pub struct TerminalEvents(Arc<OutputShared>);

impl TerminalEvents {
    pub async fn recv(&mut self) -> Option<TerminalOutput> {
        loop {
            // Register before checking, so a notification between the check
            // and await remains recorded by `Notify`.
            let available = self.0.available.notified();
            if let Ok(output) = self.try_recv() {
                return Some(output);
            }
            available.await;
        }
    }

    pub fn try_recv(&self) -> Result<TerminalOutput, tokio::sync::mpsc::error::TryRecvError> {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while let Some(id) = state.ready.pop_front() {
            let Some(pending) = state.sessions.get_mut(&id) else {
                continue;
            };
            pending.ready = false;
            let output = if let Some(bytes) = pending.bytes.pop_front() {
                Some(TerminalOutput::Bytes { id, bytes })
            } else {
                pending
                    .exit
                    .take()
                    .map(|code| TerminalOutput::Exited { id, code })
            };
            let remains = !pending.bytes.is_empty() || pending.exit.is_some();
            if remains {
                pending.ready = true;
                state.ready.push_back(id);
            }
            self.0.space.notify_all();
            if let Some(output) = output {
                return Ok(output);
            }
        }
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    }

    #[cfg(test)]
    fn pending_for(&self, id: TerminalId) -> usize {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sessions
            .get(&id)
            .map_or(0, |pending| pending.bytes.len())
    }
}

/// Total payload retained by workspace scrollback and immutable review
/// snapshots. Deriving the unit from the actual `Cell` representation keeps
/// the nominal 64 MiB bound honest if that representation changes. Review
/// text, indices, and search matches are converted into the same unit when the
/// budget is enforced. Live screens are excluded and are never evicted.
pub const WORKSPACE_TERMINAL_CELL_BYTES: usize = 64 * 1024 * 1024;
pub const WORKSPACE_SCROLLBACK_CELLS: usize =
    WORKSPACE_TERMINAL_CELL_BYTES / std::mem::size_of::<Cell>();

/// One row of a prepared terminal view.
pub type TerminalRow = Vec<Cell>;

/// The rectangle of cells a pane draws, prepared for one frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalView {
    /// Monotonic session-local drawing revision used as the base for row
    /// damage. It is unrelated to buffer revisions.
    pub revision: u64,
    pub columns: usize,
    pub rows: Vec<TerminalRow>,
    pub line_ids: Vec<Option<u64>>,
    /// Where the child's cursor sits inside `rows`, when it is visible and on
    /// screen. Absent while scrolled back into history, because the cursor is
    /// not there.
    pub cursor: Option<(usize, usize)>,
    /// How many lines above the live screen this view starts.
    pub scrollback: usize,
    /// False once the child has exited; the last screen stays readable.
    pub live: bool,
    pub review: bool,
    pub newer_output: bool,
    pub highlights: Vec<TerminalHighlight>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalHighlightKind {
    Match,
    ActiveMatch,
    Selection,
    JumpLabelImmediate,
    JumpLabelPrefix,
    JumpLabelSuffix,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewMotion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    FirstNonWhitespace,
    FileStart,
    FileEnd,
    WordForward,
    WordBackward,
    WordEnd,
    LongWordForward,
    LongWordBackward,
    LongWordEnd,
    NextParagraph,
    PreviousParagraph,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    WindowTop,
    WindowCenter,
    WindowBottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalHighlight {
    pub row: usize,
    pub start_column: usize,
    pub end_column: usize,
    pub kind: TerminalHighlightKind,
}

#[derive(Clone, Debug)]
struct ReviewLine {
    id: u64,
    cells: TerminalRow,
    text_start: usize,
    text_end: usize,
    char_columns: Vec<usize>,
}

#[derive(Clone, Debug)]
struct TerminalReview {
    source_revision: u64,
    lines: Vec<ReviewLine>,
    text: String,
    selection: Selection,
    matches: Vec<Range>,
    active_match: Option<usize>,
    scroll: usize,
    /// Blank rows retained below the snapshot so the caret can keep the same
    /// bottom margin an ordinary file has near its end.
    bottom_padding: usize,
}

/// Text Runyte itself put into a child's input, described only as much as
/// taking it back again needs.
#[derive(Clone, Copy, Debug)]
struct SentText {
    /// Characters rather than bytes: a line editor erases one per delete, and
    /// a pasted `\u{754c}` is one of them however many bytes carried it.
    characters: usize,
    /// Whether a delete per character still describes what the child holds.
    erasable: bool,
}

/// What asking a child to take back the last text Runyte sent it did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SentTextUndo {
    /// That many deletes were sent for that many pasted characters.
    Erased(usize),
    /// Nothing Runyte sent is still the child's last input.
    NothingSent,
    /// The paste carried a line break the child was free to act on.
    AlreadyRun,
    /// The child's input queue would not take the deletes.
    Refused,
}

/// One terminal: its child, its screen, and where the reader is looking.
#[derive(Debug)]
pub struct TerminalSession {
    id: TerminalId,
    label: String,
    user_name: Option<String>,
    directory: PathBuf,
    initial_directory: PathBuf,
    created_at: SystemTime,
    last_activity: SystemTime,
    unread_activity: bool,
    bell: bool,
    history_truncated: bool,
    content_revision: u64,
    review: Option<TerminalReview>,
    emulator: Emulator,
    #[cfg(unix)]
    pty: Option<pty::Pty>,
    /// Set once the child has ended, carrying its status code when known.
    exit: Option<Option<i32>>,
    /// The last text Runyte itself put into this child's input, kept only for
    /// as long as it is still the last thing the child received.
    sent_text: Option<SentText>,
    /// Lines above the live screen the reader has scrolled to.
    scroll: usize,
    /// Bumped whenever anything a frame would draw has changed.
    revision: u64,
}

impl TerminalSession {
    pub fn id(&self) -> TerminalId {
        self.id
    }

    /// The name a pane shows: whatever the child last called itself, or the
    /// program it was started as.
    pub fn name(&self) -> String {
        if let Some(name) = self.user_name.as_ref() {
            return name.clone();
        }
        match self.emulator.title() {
            Some(title) => title.to_owned(),
            None => self.label.clone(),
        }
    }

    /// The session name as presented alongside Runyte's other view types.
    ///
    /// The prefix is deliberately presentation-only: typed commands continue
    /// to resolve the concise child or user name, such as `agent`, rather than
    /// requiring the decoration shown in pane and manager titles.
    pub fn display_name(&self) -> String {
        format!("[terminal] {}", self.name())
    }

    pub fn user_name(&self) -> Option<&str> {
        self.user_name.as_deref()
    }

    pub fn child_title(&self) -> Option<&str> {
        self.emulator.title()
    }

    pub fn launch_label(&self) -> &str {
        &self.label
    }

    pub fn initial_directory(&self) -> &Path {
        &self.initial_directory
    }

    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub fn last_activity(&self) -> SystemTime {
        self.last_activity
    }

    pub fn unread_activity(&self) -> bool {
        self.unread_activity
    }

    pub fn bell(&self) -> bool {
        self.bell
    }

    pub fn history_truncated(&self) -> bool {
        self.history_truncated
    }

    pub fn reviewing(&self) -> bool {
        self.review.is_some()
    }

    pub fn review_has_newer_output(&self) -> bool {
        self.review
            .as_ref()
            .is_some_and(|review| review.source_revision != self.content_revision)
    }

    pub fn discard_review(&mut self) {
        if self.review.take().is_some() {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn ensure_review(&mut self) -> &mut TerminalReview {
        if self.review.is_none() {
            let grid = self.emulator.grid();
            let mut text = String::new();
            let mut text_chars = 0;
            let mut lines = Vec::new();
            for (id, cells) in grid.retained_lines() {
                let text_start = text_chars;
                let end = cells
                    .iter()
                    .rposition(|cell| cell.width != 0 && cell.character != ' ')
                    .map_or(0, |index| index + 1);
                let mut char_columns = Vec::new();
                for (column, cell) in cells[..end].iter().enumerate() {
                    if cell.width != 0 {
                        char_columns.push(column);
                        text.push(cell.character);
                        for combining in &cell.combining[..usize::from(cell.combining_len)] {
                            char_columns.push(column);
                            text.push(*combining);
                        }
                    }
                }
                let text_end = text_start + char_columns.len();
                lines.push(ReviewLine {
                    id,
                    cells: cells.clone(),
                    text_start,
                    text_end,
                    char_columns,
                });
                text.push('\n');
                text_chars = text_end + 1;
            }
            while text.ends_with("\n\n") {
                text.pop();
            }
            let visible_end = lines.len().saturating_sub(self.scroll);
            let visible_start = visible_end.saturating_sub(grid.rows());
            let caret = if self.scroll == 0 {
                let cursor_line = grid.scrollback_len() + grid.cursor.row;
                lines
                    .get(cursor_line)
                    .filter(|line| line.text_start < line.text_end)
                    .map(|line| {
                        let relative = line
                            .char_columns
                            .iter()
                            .position(|column| *column >= grid.cursor.column)
                            .unwrap_or_else(|| line.char_columns.len().saturating_sub(1));
                        line.text_start + relative
                    })
                    .or_else(|| {
                        lines[..cursor_line.min(lines.len())]
                            .iter()
                            .rev()
                            .find(|line| line.text_start < line.text_end)
                            .map(|line| line.text_end.saturating_sub(1))
                    })
            } else {
                None
            }
            .or_else(|| {
                lines[visible_start..visible_end]
                    .iter()
                    .find(|line| line.text_start < line.text_end)
                    .map(|line| line.text_start)
            })
            .unwrap_or(0);
            self.review = Some(TerminalReview {
                source_revision: self.content_revision,
                lines,
                text,
                selection: Selection::point(caret),
                matches: Vec::new(),
                active_match: None,
                scroll: self.scroll,
                bottom_padding: 0,
            });
            self.revision = self.revision.wrapping_add(1);
        }
        self.review.as_mut().expect("review was created")
    }

    /// Captures the retained output as the immutable surface Normal mode
    /// navigates. Repeated entry keeps the same snapshot so live output cannot
    /// move a caret or selection underneath the reader.
    pub fn begin_review(&mut self) {
        self.ensure_review();
    }

    /// Selects every meaningful character in the retained review snapshot.
    ///
    /// The synthetic newline after the last terminal row is not child output,
    /// so the range ends at the final occupied cell just as a file selection
    /// ends at its last row's text boundary.
    pub fn select_all_review(&mut self) {
        let review = self.ensure_review();
        let end = review
            .lines
            .iter()
            .rev()
            .find(|line| line.text_start < line.text_end)
            .map_or(0, |line| line.text_end);
        review.selection = if end == 0 {
            Selection::point(0)
        } else {
            Selection::single(Range::new(0, end - 1))
        };
        review.matches.clear();
        review.active_match = None;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn collapse_review_selection(&mut self) {
        if let Some(review) = self.review.as_mut() {
            review.selection = review.selection.collapse();
            review.matches.clear();
            review.active_match = None;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn keep_primary_review_selection(&mut self) {
        if let Some(review) = self.review.as_mut() {
            review.selection = review.selection.keep_primary();
            review.matches.clear();
            review.active_match = None;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn search_review(&mut self, pattern: &str, regex: bool) -> Result<usize, regex::Error> {
        let viewport_rows = self.emulator.grid().rows();
        let expression = if regex {
            regex::Regex::new(pattern)?
        } else {
            regex::RegexBuilder::new(&regex::escape(pattern))
                .case_insensitive(true)
                .build()?
        };
        let count = {
            let review = self.ensure_review();
            let mut byte_cursor = 0;
            let mut char_cursor = 0;
            review.matches.clear();
            for matched in expression.find_iter(&review.text) {
                // Regex offsets are bytes and review offsets are characters.
                // Matches are ordered and non-overlapping, so advance one
                // cursor through the text instead of rescanning every prefix.
                char_cursor += review.text[byte_cursor..matched.start()].chars().count();
                let from = char_cursor;
                char_cursor += review.text[matched.start()..matched.end()].chars().count();
                byte_cursor = matched.end();
                if from < char_cursor {
                    review.matches.push(Range::new(from, char_cursor));
                }
            }
            review.active_match = (!review.matches.is_empty()).then_some(0);
            if let Some(range) = review.matches.first().copied() {
                let selection = inclusive_review_range(range);
                review.selection = Selection::single(selection);
                focus_review_range(review, selection, viewport_rows);
            }
            review.matches.len()
        };
        self.revision = self.revision.wrapping_add(1);
        Ok(count)
    }

    pub fn step_review_match(&mut self, forward: bool) -> bool {
        let viewport_rows = self.emulator.grid().rows();
        let Some(review) = self.review.as_mut() else {
            return false;
        };
        if review.matches.is_empty() {
            return false;
        }
        let active = review.active_match.unwrap_or(0);
        let next = if forward {
            (active + 1) % review.matches.len()
        } else {
            (active + review.matches.len() - 1) % review.matches.len()
        };
        review.active_match = Some(next);
        let range = inclusive_review_range(review.matches[next]);
        review.selection = Selection::single(range);
        focus_review_range(review, range, viewport_rows);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn review_selection_text(&mut self) -> String {
        let review = self.ensure_review();
        let text = review.text.chars().collect::<Vec<_>>();
        let textless = review
            .lines
            .iter()
            .all(|line| line.text_start == line.text_end);
        review
            .selection
            .ranges()
            .iter()
            .map(|range| {
                let from = range.from();
                let to = if range.is_empty() && textless {
                    from
                } else if range.is_empty() {
                    (from + 1).min(text.len())
                } else {
                    range.to().saturating_add(1).min(text.len())
                };
                text[from.min(text.len())..to.min(text.len())]
                    .iter()
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn move_review(&mut self, motion: ReviewMotion, extend: bool) -> bool {
        let viewport_rows = self.emulator.grid().rows();
        let review = self.ensure_review();
        let selection = review.selection.clone();
        review.selection = selection.transform(|range| {
            let target = review_motion_target(review, range.head, motion, viewport_rows);
            if extend {
                range.extend_to(target)
            } else {
                Range::point(target)
            }
        });
        review.matches.clear();
        review.active_match = None;
        focus_review_range(review, review.selection.primary(), viewport_rows);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Resolves one cell in the visible immutable review to the character it
    /// displays. Trailing cells clamp to the row's last character, matching
    /// keyboard motion and document pointer selection; top/bottom padding has
    /// no review coordinate and therefore returns no target.
    pub fn review_offset_at_view_cell(
        &self,
        viewport_rows: usize,
        row: usize,
        column: usize,
    ) -> Option<usize> {
        let review = self.review.as_ref()?;
        let (start, end, padding) = review_visible_bounds(review, viewport_rows);
        let line_index = row.checked_sub(padding)?.saturating_add(start);
        if line_index >= end {
            return None;
        }
        let line = &review.lines[line_index];
        review_offset_at_column(line, column)
            .or_else(|| (line.text_start < line.text_end).then(|| line.text_end - 1))
    }

    /// The fixed end a Shift-click or drag extends from in review mode.
    pub fn review_selection_anchor(&self) -> Option<usize> {
        self.review
            .as_ref()
            .map(|review| review.selection.primary().anchor)
    }

    /// Installs an inclusive character selection on the immutable review.
    /// Terminal review follows Runyte's selection model: both the pressed and
    /// current cells are covered, whichever direction the drag takes.
    pub fn set_review_selection(&mut self, anchor: usize, head: usize) -> bool {
        let Some(review) = self.review.as_mut() else {
            return false;
        };
        let end = review_document_end(review);
        review.selection = Selection::single(Range::new(anchor.min(end), head.min(end)));
        review.matches.clear();
        review.active_match = None;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Finds one character in the immutable review text, skipping line
    /// separators just as file-buffer character motions do.
    pub fn find_review_character(
        &mut self,
        character: char,
        forward: bool,
        till: bool,
        extend: bool,
    ) -> bool {
        let viewport_rows = self.emulator.grid().rows();
        let review = self.ensure_review();
        let characters = review.text.chars().collect::<Vec<_>>();
        let mut missed = false;
        let selection = review.selection.clone();
        review.selection = selection.transform(|range| {
            let found = if forward {
                (range.head.saturating_add(1)..characters.len())
                    .find(|offset| characters[*offset] == character)
            } else {
                (0..range.head.min(characters.len()))
                    .rev()
                    .find(|offset| characters[*offset] == character)
            };
            let Some(mut target) = found else {
                missed = true;
                return range;
            };
            if till {
                target = if forward {
                    (0..target)
                        .rev()
                        .find(|offset| characters[*offset] != '\n')
                        .unwrap_or(range.head)
                } else {
                    (target + 1..characters.len())
                        .find(|offset| characters[*offset] != '\n')
                        .unwrap_or(range.head)
                };
            }
            if extend {
                range.extend_to(target)
            } else {
                Range::point(target)
            }
        });
        review.matches.clear();
        review.active_match = None;
        focus_review_range(review, review.selection.primary(), viewport_rows);
        self.revision = self.revision.wrapping_add(1);
        !missed
    }

    /// Moves every review caret to one one-based retained row. This is the
    /// terminal-review counterpart of counted `gg`/`G`; it must not address
    /// the buffer hidden behind the terminal pane.
    pub fn goto_review_line(&mut self, one_based: usize, extend: bool) {
        let viewport_rows = self.emulator.grid().rows();
        let review = self.ensure_review();
        let row = one_based
            .saturating_sub(1)
            .min(review.lines.len().saturating_sub(1));
        let target = review.lines[row].text_start;
        let selection = review.selection.clone();
        review.selection = selection.transform(|range| {
            if extend {
                range.extend_to(target)
            } else {
                Range::point(target)
            }
        });
        review.matches.clear();
        review.active_match = None;
        focus_review_range(review, review.selection.primary(), viewport_rows);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Applies the editor's configured vertical margin to the active review
    /// caret without realigning a caret that is already comfortably visible.
    pub fn focus_review_selection(&mut self, viewport_rows: usize, scroll_offset: usize) {
        let Some(review) = self.review.as_mut() else {
            return;
        };
        let before = (review.scroll, review.bottom_padding);
        focus_review_range_with_offset(
            review,
            review.selection.primary(),
            viewport_rows,
            scroll_offset,
        );
        if (review.scroll, review.bottom_padding) != before {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Moves every review caret to one labelled terminal-text offset.
    pub fn goto_review_offset(&mut self, offset: usize, extend: bool) {
        let viewport_rows = self.emulator.grid().rows();
        let review = self.ensure_review();
        let target = offset.min(review_document_end(review));
        let selection = review.selection.clone();
        review.selection = selection.transform(|range| {
            if extend {
                range.extend_to(target)
            } else {
                Range::point(target)
            }
        });
        review.matches.clear();
        review.active_match = None;
        focus_review_range(review, review.selection.primary(), viewport_rows);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Returns visible word starts ranked from the review caret outwards.
    /// The second value is how many label cells remain visible from that
    /// start, allowing the shared jump-label allocator to reject a two-key
    /// label at the right edge while retaining a one-key label there.
    pub fn visible_review_word_targets(&mut self, viewport_rows: usize) -> Vec<(usize, usize)> {
        #[derive(Clone, Copy)]
        struct Candidate {
            offset: usize,
            screen_row: usize,
            screen_column: usize,
            visible_label_cells: usize,
            order: usize,
        }

        let columns = self.emulator.grid().columns();
        let review = self.ensure_review();
        let (start, end, padding) = review_visible_bounds(review, viewport_rows);
        let (cursor_row, cursor_column) = review_cursor_position(review).unwrap_or((start, 0));
        let cursor_screen_row = if cursor_row < start {
            0
        } else if cursor_row >= end {
            viewport_rows.saturating_sub(1)
        } else {
            padding + cursor_row - start
        };
        let characters = review.text.chars().collect::<Vec<_>>();
        let mut candidates = Vec::new();
        for row in start..end {
            let line = &review.lines[row];
            let mut word_start = None;
            let line_length = line.text_end.saturating_sub(line.text_start);
            for relative in 0..=line_length {
                let is_word = relative < line_length
                    && characters
                        .get(line.text_start + relative)
                        .is_some_and(|character| character.is_alphanumeric() || *character == '_');
                match (is_word, word_start) {
                    (true, None) => word_start = Some(relative),
                    (false, Some(begin)) => {
                        let first_column = line.char_columns[begin];
                        let second_column = line.char_columns.get(begin + 1).copied();
                        let two_single_cells = relative - begin >= 2
                            && line
                                .cells
                                .get(first_column)
                                .is_some_and(|cell| cell.width == 1)
                            && second_column == Some(first_column + 1)
                            && second_column
                                .and_then(|column| line.cells.get(column))
                                .is_some_and(|cell| cell.width == 1);
                        if two_single_cells && first_column < columns {
                            candidates.push(Candidate {
                                offset: line.text_start + begin,
                                screen_row: padding + row - start,
                                screen_column: first_column,
                                visible_label_cells: columns.saturating_sub(first_column).min(2),
                                order: candidates.len(),
                            });
                        }
                        word_start = None;
                    }
                    _ => {}
                }
            }
        }
        candidates.sort_by_key(|candidate| {
            (
                candidate.screen_row.abs_diff(cursor_screen_row) * 10
                    + candidate.screen_column.abs_diff(cursor_column),
                candidate.order,
            )
        });
        candidates
            .into_iter()
            .map(|candidate| (candidate.offset, candidate.visible_label_cells))
            .collect()
    }

    /// Selects complete terminal-review lines. The first `x`/`X` snaps each
    /// range to its current line; repeated presses walk the moving edge down
    /// or up just like ordinary buffer line selection.
    pub fn select_review_line(&mut self, down: bool, extend: bool) {
        let viewport_rows = self.emulator.grid().rows();
        let review = self.ensure_review();
        let selection = review.selection.clone();
        review.selection = selection.transform(|range| {
            let anchor_row = review_line_for_offset(review, range.anchor);
            let mut head_row = review_line_for_offset(review, range.head);
            if extend {
                head_row = if down {
                    (head_row + 1).min(review.lines.len().saturating_sub(1))
                } else {
                    head_row.saturating_sub(1)
                };
            }
            if head_row >= anchor_row {
                Range::new(
                    review.lines[anchor_row].text_start,
                    review_line_last_offset(&review.lines[head_row]),
                )
            } else {
                Range::new(
                    review_line_last_offset(&review.lines[anchor_row]),
                    review.lines[head_row].text_start,
                )
            }
        });
        review.matches.clear();
        review.active_match = None;
        focus_review_range(review, review.selection.primary(), viewport_rows);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Adds a caret on the nearest row in the requested direction that holds
    /// a character at each current caret's terminal-cell column.
    pub fn copy_review_selection(&mut self, down: bool) -> bool {
        let viewport_rows = self.emulator.grid().rows();
        let review = self.ensure_review();
        let mut added = Vec::new();
        for range in review.selection.ranges() {
            let row = review_line_for_offset(review, range.head);
            let column = review_column_for_offset(&review.lines[row], range.head);
            let candidates: Box<dyn Iterator<Item = usize>> = if down {
                Box::new(row + 1..review.lines.len())
            } else {
                Box::new((0..row).rev())
            };
            if let Some(offset) = candidates
                .filter_map(|candidate| review_offset_at_column(&review.lines[candidate], column))
                .next()
            {
                added.push(Range::point(offset));
            }
        }
        if added.is_empty() {
            return false;
        }
        let before = review.selection.len();
        let mut ranges = review.selection.ranges().to_vec();
        let primary = review.selection.primary_index();
        ranges.extend(added);
        review.selection = Selection::new(ranges, primary);
        if review.selection.len() == before {
            return false;
        }
        review.matches.clear();
        review.active_match = None;
        focus_review_range(review, review.selection.primary(), viewport_rows);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    fn review_cells(&self) -> usize {
        self.review.as_ref().map_or(0, |review| {
            let cell_size = std::mem::size_of::<Cell>();
            let cells = review
                .lines
                .iter()
                .map(|line| line.cells.capacity())
                .sum::<usize>();
            let auxiliary_bytes = review.text.capacity()
                + review.matches.capacity() * std::mem::size_of::<Range>()
                + review.selection.len() * std::mem::size_of::<Range>()
                + review.lines.capacity() * std::mem::size_of::<ReviewLine>()
                + review
                    .lines
                    .iter()
                    .map(|line| line.char_columns.capacity() * std::mem::size_of::<usize>())
                    .sum::<usize>();
            cells.saturating_add(auxiliary_bytes.div_ceil(cell_size))
        })
    }

    pub fn rename(&mut self, name: Option<String>) -> Result<(), &'static str> {
        if let Some(name) = name.as_ref()
            && (name.is_empty() || name.len() > 128 || name.chars().any(char::is_control))
        {
            return Err("terminal name must be 1 to 128 characters without controls");
        }
        self.user_name = name;
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn mark_viewed(&mut self) {
        self.unread_activity = false;
        self.bell = false;
    }

    pub fn directory(&self) -> &std::path::Path {
        &self.directory
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn live(&self) -> bool {
        self.exit.is_none()
    }

    pub fn exit_code(&self) -> Option<Option<i32>> {
        self.exit
    }

    pub fn scroll(&self) -> usize {
        self.review
            .as_ref()
            .map_or(self.scroll, |review| review.scroll)
    }

    pub fn alternate_screen(&self) -> bool {
        self.emulator.alternate_screen()
    }

    pub fn sgr_mouse_reporting(&self) -> bool {
        self.emulator.modes.mouse_reporting && self.emulator.modes.mouse_sgr
    }

    pub fn send_mouse(&mut self, event: crate::input::PointerEvent, column: u16, row: u16) -> bool {
        self.send_mouse_repeated(event, column, row, 1)
    }

    /// Sends a bounded run of identical reports as one PTY write. Keeping the
    /// run in one queue entry prevents a fast physical wheel from filling the
    /// terminal input queue with individually scheduled packets.
    pub fn send_mouse_repeated(
        &mut self,
        event: crate::input::PointerEvent,
        column: u16,
        row: u16,
        repetitions: u16,
    ) -> bool {
        if !self.sgr_mouse_reporting() {
            return false;
        }
        let bytes = Self::sgr_mouse_bytes_repeated(event, column, row, repetitions);
        #[cfg(unix)]
        {
            self.pty.as_ref().is_some_and(|pty| pty.write(bytes))
        }
        #[cfg(not(unix))]
        {
            let _ = bytes;
            false
        }
    }

    fn sgr_mouse_bytes(event: crate::input::PointerEvent, column: u16, row: u16) -> Vec<u8> {
        use crate::input::{Modifiers, PointerButton, PointerEventKind};

        let mut code = match event.kind {
            PointerEventKind::Down(PointerButton::Left) => 0,
            PointerEventKind::Down(PointerButton::Middle) => 1,
            PointerEventKind::Down(PointerButton::Right) => 2,
            PointerEventKind::Up(_) => 3,
            PointerEventKind::Drag(PointerButton::Left) => 32,
            PointerEventKind::Drag(PointerButton::Middle) => 33,
            PointerEventKind::Drag(PointerButton::Right) => 34,
            PointerEventKind::Moved => 35,
            PointerEventKind::ScrollUp => 64,
            PointerEventKind::ScrollDown => 65,
            PointerEventKind::ScrollLeft => 66,
            PointerEventKind::ScrollRight => 67,
        };
        if event.modifiers.contains(Modifiers::SHIFT) {
            code += 4;
        }
        if event.modifiers.contains(Modifiers::ALT) {
            code += 8;
        }
        if event.modifiers.contains(Modifiers::CONTROL) {
            code += 16;
        }
        let suffix = if matches!(event.kind, PointerEventKind::Up(_)) {
            'm'
        } else {
            'M'
        };
        format!("\x1b[<{code};{};{}{suffix}", column + 1, row + 1).into_bytes()
    }

    fn sgr_mouse_bytes_repeated(
        event: crate::input::PointerEvent,
        column: u16,
        row: u16,
        repetitions: u16,
    ) -> Vec<u8> {
        let packet = Self::sgr_mouse_bytes(event, column, row);
        let mut bytes = Vec::with_capacity(packet.len() * usize::from(repetitions));
        for _ in 0..repetitions {
            bytes.extend_from_slice(&packet);
        }
        bytes
    }

    /// The screen and its history as plain text, for yanking or for freezing
    /// into an ordinary buffer where the whole editor works on it.
    pub fn plain_text(&self) -> String {
        self.emulator.plain_text()
    }

    /// Number of decoded presentation rows available to incremental search.
    pub fn plain_line_count(&self) -> usize {
        self.emulator.grid().plain_line_count()
    }

    /// One decoded presentation row without materializing the whole terminal.
    pub fn plain_line(&self, row: usize) -> Option<String> {
        self.emulator.grid().plain_line(row)
    }

    /// One decoded row together with its monotonic identity. Unlike the row
    /// index, the identity does not change when older scrollback is evicted.
    pub fn plain_line_with_id(&self, row: usize) -> Option<(u64, String)> {
        let grid = self.emulator.grid();
        Some((grid.retained_line_id(row)?, grid.plain_line(row)?))
    }

    /// Resolves a stable retained-line identity to its current row.
    pub fn retained_line_row(&self, line_id: u64) -> Option<usize> {
        self.emulator.grid().retained_row(line_id)
    }

    /// Captures current retained output and places the review caret on one
    /// stable line. An identity evicted from bounded history is not silently
    /// retargeted to the row that reused its former index.
    pub fn begin_review_at_line(&mut self, line_id: u64) -> bool {
        if self.retained_line_row(line_id).is_none() {
            return false;
        }
        self.discard_review();
        self.begin_review();
        let Some(row) = self
            .review
            .as_ref()
            .and_then(|review| review.lines.iter().position(|line| line.id == line_id))
        else {
            return false;
        };
        self.goto_review_line(row + 1, false);
        true
    }

    /// Applies bytes the child wrote, answering any query they contained.
    fn feed(&mut self, bytes: &[u8]) {
        let retired = self.emulator.grid().retired();
        self.emulator.feed(bytes);
        if let Some(report) = self.emulator.take_directory_report()
            && let Some(directory) = validated_osc7_directory(&report)
        {
            self.directory = directory;
        }
        self.bell |= self.emulator.take_bell();
        self.last_activity = SystemTime::now();
        self.unread_activity = true;
        self.content_revision = self.content_revision.wrapping_add(1);
        // A reader scrolled back into history is holding a position in the
        // text, not a distance from the bottom. Every line the child pushes off
        // the top moves the bottom away, so the distance has to grow by the
        // same amount or the window walks forward through what is being read.
        // Following the live screen is the other case, and it is the one where
        // the distance stays zero.
        if self.scroll > 0 {
            let grown = self.emulator.grid().retired().saturating_sub(retired);
            let scroll = self.scroll.saturating_add(grown as usize);
            // Clamped to what is still kept: past the limit the line being
            // read has genuinely been dropped, and the oldest one left is as
            // close as the view can stay to it.
            self.scroll = scroll.min(self.emulator.grid().scrollback_len());
        }
        let replies = self.emulator.take_replies();
        if !replies.is_empty() {
            let _ = self.write(replies);
        }
        self.revision = self.revision.wrapping_add(1);
    }

    fn write(&mut self, bytes: Vec<u8>) -> bool {
        #[cfg(unix)]
        if let Some(pty) = self.pty.as_ref() {
            return pty.write(bytes);
        }
        let _ = bytes;
        false
    }

    /// Sends one keystroke to the child, reporting whether it had an encoding.
    ///
    /// Scrolled-back views jump to the live screen first: typing at history is
    /// a request to be back where the typing will appear.
    pub fn send_key(&mut self, key: KeyStroke) -> bool {
        let Some(bytes) = keys::encode(key, self.emulator.modes) else {
            return false;
        };
        // Whatever this key does to the child's input area, the pasted text is
        // no longer the last thing at the cursor, so deletes counted from its
        // length would land on the wrong characters.
        self.sent_text = None;
        self.scroll = 0;
        self.discard_review();
        self.write(bytes)
    }

    /// Sends literal text, bracketed when the child asked for that.
    pub fn send_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return true;
        }
        let bracketed = self.emulator.modes.bracketed_paste;
        let bytes = keys::encode_paste(text, self.emulator.modes);
        self.scroll = 0;
        self.discard_review();
        if !self.write(bytes) {
            return false;
        }
        self.sent_text = Some(SentText {
            characters: text.chars().count(),
            // Bare text goes to a line discipline that runs a line the moment
            // it sees the return `encode_paste` turned every break into. What
            // has run is the child's, and no number of deletes takes it back.
            // Between paste brackets the same break is only data in the line
            // editor's buffer, which a delete removes like any other character.
            erasable: bracketed || !text.contains(['\n', '\r']),
        });
        true
    }

    /// Asks the child's line editor to take back the text Runyte last sent.
    ///
    /// A terminal has no undo history to roll back: the text is the child's
    /// now, and the only thing Runyte can say about it afterwards is what a
    /// person holding the delete key would say. So this sends one delete per
    /// character sent, which a line editor at a prompt answers by erasing
    /// exactly the paste, and it is offered only while that paste is still the
    /// last input the child received.
    pub fn undo_sent_text(&mut self) -> SentTextUndo {
        const DELETE: u8 = 0x7f;

        let Some(sent) = self.sent_text else {
            return SentTextUndo::NothingSent;
        };
        if !sent.erasable {
            return SentTextUndo::AlreadyRun;
        }
        if sent.characters == 0 {
            self.sent_text = None;
            return SentTextUndo::NothingSent;
        }
        self.scroll = 0;
        self.discard_review();
        if !self.write(vec![DELETE; sent.characters]) {
            return SentTextUndo::Refused;
        }
        // One undo per paste: a second would erase what the paste replaced.
        self.sent_text = None;
        SentTextUndo::Erased(sent.characters)
    }

    /// Scrolls back into history, clamped to what is kept.
    ///
    /// The alternate screen has no history, so a full-screen program scrolls
    /// nowhere rather than into the shell output behind it.
    pub fn scroll_back(&mut self, lines: usize) {
        if let Some(review) = self.review.as_mut() {
            review.bottom_padding = 0;
            let maximum = review
                .lines
                .len()
                .saturating_sub(self.emulator.grid().rows().max(1));
            review.scroll = (review.scroll + lines).min(maximum);
            self.revision = self.revision.wrapping_add(1);
            return;
        }
        let limit = if self.emulator.alternate_screen() {
            0
        } else {
            self.emulator.grid().scrollback_len()
        };
        let scroll = (self.scroll + lines).min(limit);
        if scroll != self.scroll {
            self.scroll = scroll;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn scroll_forward(&mut self, lines: usize) {
        if let Some(review) = self.review.as_mut() {
            review.bottom_padding = 0;
            review.scroll = review.scroll.saturating_sub(lines);
            self.revision = self.revision.wrapping_add(1);
            return;
        }
        let scroll = self.scroll.saturating_sub(lines);
        if scroll != self.scroll {
            self.scroll = scroll;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn scroll_to_oldest(&mut self) {
        if let Some(review) = self.review.as_mut() {
            review.bottom_padding = 0;
            review.scroll = review
                .lines
                .len()
                .saturating_sub(self.emulator.grid().rows().max(1));
            self.revision = self.revision.wrapping_add(1);
            return;
        }
        let limit = if self.emulator.alternate_screen() {
            0
        } else {
            self.emulator.grid().scrollback_len()
        };
        if self.scroll != limit {
            self.scroll = limit;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn scroll_to_live(&mut self) {
        self.discard_review();
        if self.scroll != 0 {
            self.scroll = 0;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Tells the child the pane's new shape.
    pub fn resize(&mut self, columns: usize, rows: usize) -> bool {
        let columns = columns.max(1);
        let rows = rows.max(1);
        if columns == self.emulator.columns() && rows == self.emulator.rows() {
            return false;
        }
        self.emulator.resize(columns, rows);
        #[cfg(unix)]
        if let Some(pty) = self.pty.as_ref() {
            let _ = pty.resize(columns as u16, rows as u16);
        }
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// The visible terminal cursor row, counted from the oldest line kept.
    ///
    /// A terminal's row numbers only mean anything against its whole history,
    /// which is what a status line is reporting when it names one.
    pub fn cursor_row(&self) -> usize {
        if let Some(review) = self.review.as_ref()
            && let Some((row, _)) = review_cursor_position(review)
        {
            return row;
        }
        let grid = self.emulator.grid();
        grid.scrollback_len() + grid.cursor.row
    }

    /// The visible terminal cursor column.
    pub fn cursor_column(&self) -> usize {
        if let Some(review) = self.review.as_ref()
            && let Some((_, column)) = review_cursor_position(review)
        {
            return column;
        }
        self.emulator.grid().cursor.column
    }

    /// Lines this session holds, history and screen together.
    pub fn line_count(&self) -> usize {
        if let Some(review) = self.review.as_ref() {
            return review.lines.len();
        }
        let grid = self.emulator.grid();
        grid.scrollback_len() + grid.rows()
    }

    /// Number of normalized selections on the active review snapshot.
    pub fn review_selection_count(&self) -> usize {
        self.review
            .as_ref()
            .map_or(1, |review| review.selection.len())
    }

    /// Prepares the rows a pane of `rows` lines draws.
    pub fn view(&self, rows: usize) -> TerminalView {
        if let Some(review) = self.review.as_ref() {
            return review_view(
                review,
                self.emulator.grid().columns(),
                rows,
                self.live(),
                self.content_revision,
                self.revision,
            );
        }
        let grid = self.emulator.grid();
        let columns = grid.columns();
        let rows = rows.max(1);
        let history = grid.scrollback_len();
        let scroll = self.scroll.min(history);
        // The view is the last `rows` lines of history-then-screen, moved back
        // by however far the reader has scrolled.
        let total = history + grid.rows();
        let end = total.saturating_sub(scroll);
        let start = end.saturating_sub(rows);
        let mut lines = Vec::with_capacity(rows);
        let mut line_ids = Vec::with_capacity(rows);
        for index in start..end {
            let line = if index < history {
                grid.scrollback_line(index)
            } else {
                grid.line(index - history)
            };
            let mut line = line.cloned().unwrap_or_default();
            line.resize(columns, Cell::default());
            lines.push(line);
            line_ids.push(Some(
                grid.retired().saturating_sub(history as u64) + index as u64,
            ));
        }
        // A pane taller than the whole session pads at the top, so the first
        // output stays where it was written rather than floating.
        while lines.len() < rows {
            lines.insert(0, vec![Cell::default(); columns]);
            line_ids.insert(0, None);
        }
        let cursor = if self.emulator.modes.cursor_visible {
            let cursor = grid.cursor;
            let absolute = history + cursor.row;
            (absolute >= start && absolute < end).then(|| {
                (
                    absolute - start,
                    cursor.column.min(columns.saturating_sub(1)),
                )
            })
        } else {
            None
        };
        TerminalView {
            revision: self.revision,
            columns,
            rows: lines,
            line_ids,
            cursor,
            scrollback: scroll,
            live: self.live(),
            review: false,
            newer_output: false,
            highlights: Vec::new(),
        }
    }

    /// Prepares a review view with the shared `goto-word` labels painted over
    /// their terminal cells. The immutable session stays untouched, so
    /// dismissing labels restores the child's exact output on the next frame.
    pub fn view_with_jump_labels(&self, rows: usize, labels: &JumpLabels) -> TerminalView {
        let mut view = self.view(rows);
        let Some(review) = self.review.as_ref() else {
            return view;
        };
        let (start, end, padding) = review_visible_bounds(review, rows);
        for row in start..end {
            let line = &review.lines[row];
            for (relative, column) in line.char_columns.iter().copied().enumerate() {
                let offset = line.text_start + relative;
                let Some((character, part)) = labels.label_at(offset) else {
                    continue;
                };
                let view_row = padding + row - start;
                let Some(cell) = view
                    .rows
                    .get_mut(view_row)
                    .and_then(|cells| cells.get_mut(column))
                else {
                    continue;
                };
                cell.character = character;
                cell.combining = ['\0'; 3];
                cell.combining_len = 0;
                cell.width = 1;
                view.highlights.push(TerminalHighlight {
                    row: view_row,
                    start_column: column,
                    end_column: column + 1,
                    kind: match part {
                        LabelPart::Immediate => TerminalHighlightKind::JumpLabelImmediate,
                        LabelPart::Prefix => TerminalHighlightKind::JumpLabelPrefix,
                        LabelPart::Suffix => TerminalHighlightKind::JumpLabelSuffix,
                    },
                });
            }
        }
        view
    }
}

fn focus_review_range(review: &mut TerminalReview, range: Range, viewport_rows: usize) {
    focus_review_range_with_offset(review, range, viewport_rows, 0);
}

fn focus_review_range_with_offset(
    review: &mut TerminalReview,
    range: Range,
    viewport_rows: usize,
    scroll_offset: usize,
) {
    let Some(line) = review
        .lines
        .iter()
        .position(|line| range.head >= line.text_start && range.head <= line.text_end)
    else {
        return;
    };
    let rows = viewport_rows.max(1);
    let margin = scroll_offset.min(rows / 2);
    let content_rows = rows.saturating_sub(review.bottom_padding.min(rows.saturating_sub(1)));
    let end = review.lines.len().saturating_sub(review.scroll);
    let start = end.saturating_sub(content_rows);
    let top_edge = start.saturating_add(margin);
    let bottom_edge = start.saturating_add(rows.saturating_sub(margin));
    let desired_start = if line < top_edge {
        line.saturating_sub(margin)
    } else if line >= bottom_edge {
        line.saturating_add(margin + 1).saturating_sub(rows)
    } else {
        return;
    };

    let natural_start = review.lines.len().saturating_sub(rows);
    if desired_start > natural_start {
        review.scroll = 0;
        review.bottom_padding = desired_start
            .saturating_sub(natural_start)
            .min(rows.saturating_sub(1));
    } else {
        review.bottom_padding = 0;
        review.scroll = review
            .lines
            .len()
            .saturating_sub(desired_start.saturating_add(rows));
    }
}

fn review_line_for_offset(review: &TerminalReview, offset: usize) -> usize {
    review
        .lines
        .iter()
        .position(|line| offset >= line.text_start && offset <= line.text_end)
        .unwrap_or_else(|| review.lines.len().saturating_sub(1))
}

fn inclusive_review_range(range: Range) -> Range {
    if range.is_empty() {
        range
    } else if range.anchor <= range.head {
        Range::new(range.anchor, range.head.saturating_sub(1))
    } else {
        Range::new(range.anchor.saturating_sub(1), range.head)
    }
}

/// The offset a whole-line selection ends on. A blank row holds no
/// character, so its own start is the only offset that still resolves back to
/// it; `text_end - 1` would land on the previous row's separator and stall
/// `x`/`X` there.
fn review_line_last_offset(line: &ReviewLine) -> usize {
    line.text_end.saturating_sub(1).max(line.text_start)
}

fn review_column_for_offset(line: &ReviewLine, offset: usize) -> usize {
    line.char_columns
        .get(offset.saturating_sub(line.text_start))
        .copied()
        .unwrap_or_else(|| review_line_end_column(line))
}

/// Returns the character occupying a terminal cell column. A point at the
/// visual end of a row is deliberately not a candidate: `C` skips short rows
/// in buffers, and review mode follows that same rule without inventing text.
fn review_offset_at_column(line: &ReviewLine, column: usize) -> Option<usize> {
    line.char_columns
        .iter()
        .enumerate()
        .find_map(|(relative, start)| {
            let width = line
                .cells
                .get(*start)
                .map_or(1, |cell| usize::from(cell.width.max(1)));
            (*start <= column && column < start.saturating_add(width))
                .then_some(line.text_start + relative)
        })
}

fn review_word_class(character: char, long: bool) -> u8 {
    if character.is_whitespace() {
        0
    } else if long || character.is_alphanumeric() || character == '_' {
        1
    } else {
        2
    }
}

fn review_document_end(review: &TerminalReview) -> usize {
    review
        .lines
        .iter()
        .rev()
        .find(|line| line.text_start < line.text_end)
        .map_or(0, |line| line.text_end.saturating_sub(1))
}

fn review_word_forward(review: &TerminalReview, head: usize, long: bool) -> usize {
    let characters = review.text.chars().collect::<Vec<_>>();
    let mut previous_class = characters
        .get(head)
        .copied()
        .map(|character| review_word_class(character, long));
    let mut candidate = head;
    while candidate + 1 < characters.len() {
        candidate += 1;
        let class = review_word_class(characters[candidate], long);
        if class != 0 && previous_class != Some(class) {
            return candidate;
        }
        previous_class = Some(class);
    }
    review_document_end(review)
}

fn review_word_backward(review: &TerminalReview, head: usize, long: bool) -> usize {
    let characters = review.text.chars().collect::<Vec<_>>();
    let mut candidate = head.min(characters.len());
    while let Some(previous) = candidate.checked_sub(1) {
        let class = review_word_class(characters[previous], long);
        let preceding = previous
            .checked_sub(1)
            .map(|offset| review_word_class(characters[offset], long));
        if class != 0 && preceding != Some(class) {
            return previous;
        }
        candidate = previous;
    }
    0
}

fn review_word_end(review: &TerminalReview, head: usize, long: bool) -> usize {
    let characters = review.text.chars().collect::<Vec<_>>();
    let mut candidate = head.min(characters.len());
    while candidate < characters.len() {
        let class = review_word_class(characters[candidate], long);
        if class != 0 && candidate != head {
            let next_class = characters
                .get(candidate + 1)
                .copied()
                .map(|character| review_word_class(character, long));
            if next_class != Some(class) {
                return candidate;
            }
        }
        candidate += 1;
    }
    review_document_end(review)
}

fn review_motion_target(
    review: &TerminalReview,
    head: usize,
    motion: ReviewMotion,
    viewport_rows: usize,
) -> usize {
    let characters = review.text.chars().collect::<Vec<_>>();
    let text_len = characters.len();
    let head = head.min(text_len);
    let line_index = review_line_for_offset(review, head);
    let line = review.lines.get(line_index);
    match motion {
        ReviewMotion::Left => (0..head.min(text_len))
            .rev()
            .find(|offset| characters[*offset] != '\n')
            .unwrap_or(head),
        ReviewMotion::Right => (head.saturating_add(1)..text_len)
            .find(|offset| characters[*offset] != '\n')
            .unwrap_or(head),
        ReviewMotion::LineStart => line.map_or(0, |line| line.text_start),
        ReviewMotion::LineEnd => line.map_or(0, |line| line.text_end.saturating_sub(1)),
        ReviewMotion::FirstNonWhitespace => line.map_or(0, |line| {
            let relative = review
                .text
                .chars()
                .skip(line.text_start)
                .take(line.text_end.saturating_sub(line.text_start))
                .position(|character| !character.is_whitespace())
                .unwrap_or(0);
            line.text_start + relative
        }),
        ReviewMotion::FileStart => review.lines.first().map_or(0, |line| line.text_start),
        ReviewMotion::FileEnd => review_document_end(review),
        ReviewMotion::Up
        | ReviewMotion::Down
        | ReviewMotion::PageUp
        | ReviewMotion::PageDown
        | ReviewMotion::HalfPageUp
        | ReviewMotion::HalfPageDown => {
            let down = matches!(
                motion,
                ReviewMotion::Down | ReviewMotion::PageDown | ReviewMotion::HalfPageDown
            );
            let amount = match motion {
                ReviewMotion::PageUp | ReviewMotion::PageDown => viewport_rows.max(1),
                ReviewMotion::HalfPageUp | ReviewMotion::HalfPageDown => (viewport_rows / 2).max(1),
                _ => 1,
            };
            let target_line = if down {
                line_index
                    .saturating_add(amount)
                    .min(review.lines.len().saturating_sub(1))
            } else {
                line_index.saturating_sub(amount)
            };
            let column = line.map_or(0, |line| review_column_for_offset(line, head));
            review.lines.get(target_line).map_or(head, |line| {
                let relative = line
                    .char_columns
                    .iter()
                    .position(|candidate| {
                        let width = line
                            .cells
                            .get(*candidate)
                            .map_or(1, |cell| usize::from(cell.width.max(1)));
                        *candidate >= column || candidate.saturating_add(width) > column
                    })
                    .unwrap_or(line.char_columns.len());
                line.text_start + relative
            })
        }
        ReviewMotion::WordForward | ReviewMotion::LongWordForward => {
            review_word_forward(review, head, motion == ReviewMotion::LongWordForward)
        }
        ReviewMotion::WordBackward | ReviewMotion::LongWordBackward => {
            review_word_backward(review, head, motion == ReviewMotion::LongWordBackward)
        }
        ReviewMotion::WordEnd | ReviewMotion::LongWordEnd => {
            review_word_end(review, head, motion == ReviewMotion::LongWordEnd)
        }
        ReviewMotion::NextParagraph | ReviewMotion::PreviousParagraph => {
            let down = motion == ReviewMotion::NextParagraph;
            let mut target = line_index;
            if down {
                while target < review.lines.len()
                    && review.lines[target].text_start < review.lines[target].text_end
                {
                    target += 1;
                }
                while target < review.lines.len()
                    && review.lines[target].text_start == review.lines[target].text_end
                {
                    target += 1;
                }
                target = target.min(review.lines.len().saturating_sub(1));
            } else {
                target = target.saturating_sub(1);
                while target > 0 && review.lines[target].text_start == review.lines[target].text_end
                {
                    target -= 1;
                }
                while target > 0
                    && review.lines[target - 1].text_start < review.lines[target - 1].text_end
                {
                    target -= 1;
                }
            }
            review
                .lines
                .get(target)
                .map_or(head, |line| line.text_start)
        }
        ReviewMotion::WindowTop | ReviewMotion::WindowCenter | ReviewMotion::WindowBottom => {
            let rows = viewport_rows.max(1);
            let content_rows = rows.saturating_sub(review.bottom_padding.min(rows - 1));
            let end = review.lines.len().saturating_sub(review.scroll);
            let start = end.saturating_sub(content_rows);
            let target_line = match motion {
                ReviewMotion::WindowTop => start,
                ReviewMotion::WindowCenter => start + end.saturating_sub(start + 1) / 2,
                ReviewMotion::WindowBottom => end.saturating_sub(1),
                _ => unreachable!(),
            };
            let column = line.map_or(0, |line| review_column_for_offset(line, head));
            review.lines.get(target_line).map_or(head, |line| {
                review_offset_at_column(line, column).unwrap_or(line.text_end)
            })
        }
    }
}

fn push_review_highlights(
    highlights: &mut Vec<TerminalHighlight>,
    review: &TerminalReview,
    visible: std::ops::Range<usize>,
    padding: usize,
    range: Range,
    kind: TerminalHighlightKind,
) {
    for (line_index, line) in review.lines[visible].iter().enumerate() {
        let from = range.from().max(line.text_start);
        let to = range.to().min(line.text_end);
        if from >= to {
            continue;
        }
        let start_column = review_column_for_offset(line, from);
        let end_column = review_column_for_offset(line, to);
        highlights.push(TerminalHighlight {
            row: padding + line_index,
            start_column,
            end_column,
            kind,
        });
    }
}

fn review_visible_bounds(review: &TerminalReview, rows: usize) -> (usize, usize, usize) {
    let rows = rows.max(1);
    let bottom_padding = review.bottom_padding.min(rows.saturating_sub(1));
    let content_rows = rows.saturating_sub(bottom_padding);
    let end = review.lines.len().saturating_sub(review.scroll);
    let start = end.saturating_sub(content_rows);
    let padding = content_rows.saturating_sub(end.saturating_sub(start));
    (start, end, padding)
}

fn review_view(
    review: &TerminalReview,
    columns: usize,
    rows: usize,
    live: bool,
    content_revision: u64,
    revision: u64,
) -> TerminalView {
    let rows = rows.max(1);
    let bottom_padding = review.bottom_padding.min(rows.saturating_sub(1));
    let (start, end, padding) = review_visible_bounds(review, rows);
    let mut visible = review.lines[start..end]
        .iter()
        .map(|line| {
            let mut cells = line.cells.clone();
            cells.resize(columns, Cell::default());
            cells
        })
        .collect::<Vec<_>>();
    let mut line_ids = review.lines[start..end]
        .iter()
        .map(|line| Some(line.id))
        .collect::<Vec<_>>();
    for _ in 0..padding {
        visible.insert(0, vec![Cell::default(); columns]);
        line_ids.insert(0, None);
    }
    for _ in 0..bottom_padding {
        visible.push(vec![Cell::default(); columns]);
        line_ids.push(None);
    }

    let mut highlights = Vec::new();
    for (index, range) in review.matches.iter().copied().enumerate() {
        let kind = if review.active_match == Some(index) {
            TerminalHighlightKind::ActiveMatch
        } else {
            TerminalHighlightKind::Match
        };
        push_review_highlights(&mut highlights, review, start..end, padding, range, kind);
    }
    if review.active_match.is_none() {
        for (index, range) in review.selection.ranges().iter().copied().enumerate() {
            if range.is_empty() {
                if index == review.selection.primary_index() {
                    continue;
                }
                if let Some((row, column)) = review_position(review, range.head)
                    && row >= start
                    && row < end
                {
                    let width = review.lines[row]
                        .cells
                        .get(column)
                        .map_or(1, |cell| usize::from(cell.width.max(1)));
                    highlights.push(TerminalHighlight {
                        row: padding + row - start,
                        start_column: column,
                        end_column: column.saturating_add(width),
                        kind: TerminalHighlightKind::Selection,
                    });
                }
            } else {
                let highlighted = Range::new(range.from(), range.to().saturating_add(1));
                push_review_highlights(
                    &mut highlights,
                    review,
                    start..end,
                    padding,
                    highlighted,
                    TerminalHighlightKind::Selection,
                );
            }
        }
    }

    let cursor = review_cursor_position(review).and_then(|(row, column)| {
        (row >= start && row < end).then_some((
            padding + row.saturating_sub(start),
            column.min(columns.saturating_sub(1)),
        ))
    });

    TerminalView {
        revision,
        columns,
        rows: visible,
        line_ids,
        cursor,
        scrollback: review.scroll,
        live,
        review: true,
        newer_output: review.source_revision != content_revision,
        highlights,
    }
}

fn review_line_end_column(line: &ReviewLine) -> usize {
    line.char_columns
        .last()
        .and_then(|column| {
            line.cells
                .get(*column)
                .map(|cell| *column + usize::from(cell.width.max(1)))
        })
        .unwrap_or(0)
}

fn review_cursor_position(review: &TerminalReview) -> Option<(usize, usize)> {
    review_position(review, review.selection.primary().head)
}

fn review_position(review: &TerminalReview, offset: usize) -> Option<(usize, usize)> {
    review.lines.iter().enumerate().find_map(|(row, line)| {
        if offset < line.text_start || offset > line.text_end {
            return None;
        }
        Some((row, review_column_for_offset(line, offset)))
    })
}

/// Applies whatever child output is already queued, up to one full queue.
///
/// Draining beyond the one message that woke the loop is worth doing: a
/// repainting program produces many small writes, and drawing a frame for each
/// of them would show the repaint arriving instead of the finished screen.
///
/// Draining without a limit is not. A program writing faster than the editor
/// applies — `yes` is the plain case — refills the queue while the loop empties
/// it, so `try_recv` keeps succeeding and the loop never returns to rendering
/// or to the keyboard. One queue's worth coalesces a repaint and is bounded by
/// construction. Reports how many were applied.
pub fn drain(events: &mut TerminalEvents, mut apply: impl FnMut(TerminalOutput)) -> usize {
    let mut bytes = 0;
    for applied in 0..OUTPUT_QUEUE {
        match events.try_recv() {
            Ok(output) => {
                bytes += match &output {
                    TerminalOutput::Bytes { bytes, .. } => bytes.len(),
                    TerminalOutput::Exited { .. } => 0,
                };
                apply(output);
                if bytes >= OUTPUT_BYTE_BUDGET {
                    return applied + 1;
                }
            }
            Err(_) => return applied,
        }
    }
    OUTPUT_QUEUE
}

/// Every terminal this editor owns.
#[derive(Debug)]
pub struct TerminalSessions {
    sessions: BTreeMap<TerminalId, TerminalSession>,
    next: u64,
    events: TerminalEventSender,
    receiver: Option<TerminalEvents>,
    default_colors: DefaultColors,
}

impl Default for TerminalSessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TerminalSessions {
    fn drop(&mut self) {
        // A noisy child can leave its PTY reader blocked on a full per-session
        // queue. Remove every registration first so those readers wake and
        // release their descriptors before dropping the PTYs kills and reaps
        // the children.
        self.close_all();
    }
}

impl TerminalSessions {
    pub fn new() -> Self {
        let shared = Arc::new(OutputShared {
            state: Mutex::new(OutputState::default()),
            space: Condvar::new(),
            available: Notify::new(),
        });
        Self {
            sessions: BTreeMap::new(),
            next: 1,
            events: TerminalEventSender(Arc::clone(&shared)),
            receiver: Some(TerminalEvents(shared)),
            default_colors: DefaultColors::default(),
        }
    }

    /// Changes what every terminal reports for its default foreground and
    /// background. Existing sessions receive the new values too, so a query
    /// after an editor theme switch observes the live theme.
    pub(crate) fn set_default_colors(&mut self, colors: DefaultColors) {
        self.default_colors = colors;
        for session in self.sessions.values_mut() {
            session.emulator.set_default_colors(colors);
        }
    }

    #[cfg(test)]
    pub(crate) fn default_colors(&self) -> DefaultColors {
        self.default_colors
    }

    /// Hands the output stream to the loop that will drive it. Once.
    ///
    /// The receiver lives outside the editor so an event loop can wait on it
    /// beside its other sources without holding the editor borrowed.
    pub fn take_events(&mut self) -> Option<TerminalEvents> {
        self.receiver.take()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn get(&self, id: TerminalId) -> Option<&TerminalSession> {
        self.sessions.get(&id)
    }

    pub fn get_mut(&mut self, id: TerminalId) -> Option<&mut TerminalSession> {
        self.sessions.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TerminalSession> {
        self.sessions.values()
    }

    pub fn ids(&self) -> Vec<TerminalId> {
        self.sessions.keys().copied().collect()
    }

    /// Resolves a stable decimal identity first, then an exact user/display
    /// name. Duplicate names are rejected rather than made dependent on
    /// picker or creation order.
    pub fn resolve(&self, target: &str) -> Result<TerminalId, String> {
        let target = target.trim();
        if target.is_empty() {
            return Err("terminal identity or name is empty".to_owned());
        }
        if let Ok(raw) = target.parse::<u64>() {
            let id = TerminalId(raw);
            return self
                .sessions
                .contains_key(&id)
                .then_some(id)
                .ok_or_else(|| format!("terminal {raw} does not exist"));
        }
        let matches = self
            .sessions
            .values()
            .filter(|session| session.name() == target)
            .map(TerminalSession::id)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [id] => Ok(*id),
            [] => Err(format!("terminal {target:?} does not exist")),
            _ => Err(format!(
                "terminal name {target:?} is ambiguous; use its stable numeric ID"
            )),
        }
    }

    /// Whether any session still has a child running.
    pub fn any_live(&self) -> bool {
        self.sessions.values().any(TerminalSession::live)
    }

    /// Starts a child on a new pseudoterminal.
    #[cfg(unix)]
    pub fn open(
        &mut self,
        request: TerminalRequest,
        columns: usize,
        rows: usize,
    ) -> std::io::Result<TerminalId> {
        let id = TerminalId(self.next);
        self.next += 1;
        let events = self.events.clone();
        events.register(id);
        let columns = columns.max(1);
        let rows = rows.max(1);
        let child = match pty::Pty::spawn(
            &request.program,
            &request.arguments,
            &request.directory,
            columns as u16,
            rows as u16,
            move |event| {
                let message = match event {
                    pty::PtyEvent::Output(bytes) => TerminalOutput::Bytes { id, bytes },
                    pty::PtyEvent::Exited(code) => TerminalOutput::Exited { id, code },
                };
                // Called on the reader's own thread, which is a plain thread
                // rather than one of the runtime's, so blocking here is
                // allowed — and blocking here is the backpressure.
                let _ = events.send(message);
            },
        ) {
            Ok(child) => child,
            Err(error) => {
                self.events.remove(id);
                return Err(error);
            }
        };
        let mut emulator = Emulator::new(columns, rows);
        emulator.set_default_colors(self.default_colors);
        self.sessions.insert(
            id,
            TerminalSession {
                id,
                label: request.label,
                user_name: None,
                directory: request.directory.clone(),
                initial_directory: request.directory,
                created_at: SystemTime::now(),
                last_activity: SystemTime::now(),
                unread_activity: false,
                bell: false,
                history_truncated: false,
                content_revision: 1,
                review: None,
                emulator,
                pty: Some(child),
                exit: None,
                sent_text: None,
                scroll: 0,
                revision: 1,
            },
        );
        Ok(id)
    }

    #[cfg(not(unix))]
    pub fn open(
        &mut self,
        _request: TerminalRequest,
        _columns: usize,
        _rows: usize,
    ) -> std::io::Result<TerminalId> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "terminals need a pseudoterminal, which this platform does not provide here",
        ))
    }

    /// Applies one piece of child output.
    pub fn apply(&mut self, output: TerminalOutput) {
        match output {
            TerminalOutput::Bytes { id, bytes } => {
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.feed(&bytes);
                }
            }
            TerminalOutput::Exited { id, code } => {
                if let Some(session) = self.sessions.get_mut(&id) {
                    // The reader thread reports the end without a status; the
                    // child itself has the code. Prefer whichever is known.
                    #[cfg(unix)]
                    let code = match code {
                        Some(code) => Some(code),
                        None => session.pty.as_mut().and_then(pty::Pty::finished).flatten(),
                    };
                    session.exit = Some(code);
                    session.content_revision = session.content_revision.wrapping_add(1);
                    session.last_activity = SystemTime::now();
                    session.unread_activity = true;
                    #[cfg(unix)]
                    {
                        session.pty = None;
                    }
                    session.revision = session.revision.wrapping_add(1);
                }
            }
        }
        self.enforce_memory_budget();
    }

    pub fn enforce_memory_budget(&mut self) {
        let mut cells = self
            .sessions
            .values()
            .map(|session| session.emulator.grid().scrollback_cells() + session.review_cells())
            .sum::<usize>();
        while cells > WORKSPACE_SCROLLBACK_CELLS {
            // Review snapshots are reproducible convenience state and retain
            // a second copy of cells. Evict the least-recently-active one as a
            // unit before discarding the sole retained scrollback copy.
            let review_candidate = self
                .sessions
                .iter()
                .filter(|(_, session)| session.review.is_some())
                .min_by_key(|(id, session)| (session.last_activity, **id))
                .map(|(id, _)| *id);
            if let Some(id) = review_candidate {
                let session = self.sessions.get_mut(&id).expect("candidate exists");
                cells = cells.saturating_sub(session.review_cells());
                session.discard_review();
                session.history_truncated = true;
                continue;
            }
            let candidate = self
                .sessions
                .iter()
                .filter(|(_, session)| session.emulator.grid().scrollback_len() > 0)
                .min_by_key(|(id, session)| (session.last_activity, **id))
                .map(|(id, _)| *id);
            let Some(id) = candidate else {
                break;
            };
            let session = self.sessions.get_mut(&id).expect("candidate is live");
            let width = session.emulator.grid().columns();
            if session.emulator.grid_mut().drop_oldest_scrollback() {
                cells = cells.saturating_sub(width);
                session.scroll = session.scroll.min(session.emulator.grid().scrollback_len());
                session.history_truncated = true;
                session.revision = session.revision.wrapping_add(1);
            }
        }
    }

    /// Ends a session and forgets it.
    pub fn close(&mut self, id: TerminalId) -> bool {
        self.events.remove(id);
        self.sessions.remove(&id).is_some()
    }

    /// Ends every session, for editor shutdown.
    pub fn close_all(&mut self) {
        for id in self.sessions.keys().copied().collect::<Vec<_>>() {
            self.events.remove(id);
        }
        self.sessions.clear();
    }
}

/// Accepts only a bounded local `file:` URL naming an existing absolute
/// directory. A shell may legitimately report a directory outside the
/// project; opening editor content there remains governed by path safety.
fn validated_osc7_directory(report: &[u8]) -> Option<PathBuf> {
    if report.len() > 4096 || report.iter().any(|byte| byte.is_ascii_control()) {
        return None;
    }
    let report = std::str::from_utf8(report).ok()?;
    let rest = report.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    if !host.is_empty() && host != "localhost" && !local_hostname_is(host) {
        return None;
    }
    let decoded = percent_decode(&rest[slash..])?;
    let path = PathBuf::from(decoded);
    (path.is_absolute() && path.is_dir()).then_some(path)
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1)?;
            let low = *bytes.get(index + 2)?;
            decoded.push(hex(high)? * 16 + hex(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (!decoded.chars().any(char::is_control)).then_some(decoded)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(unix)]
fn local_hostname_is(value: &str) -> bool {
    let mut buffer = [0_u8; 256];
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if result != 0 {
        return false;
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    std::str::from_utf8(&buffer[..end]).is_ok_and(|hostname| hostname == value)
}

#[cfg(not(unix))]
fn local_hostname_is(_value: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(columns: usize, rows: usize) -> TerminalSession {
        TerminalSession {
            id: TerminalId(1),
            label: "test".to_owned(),
            user_name: None,
            directory: PathBuf::from("/"),
            initial_directory: PathBuf::from("/"),
            created_at: SystemTime::now(),
            last_activity: SystemTime::now(),
            unread_activity: false,
            bell: false,
            history_truncated: false,
            content_revision: 1,
            review: None,
            emulator: Emulator::new(columns, rows),
            #[cfg(unix)]
            pty: None,
            exit: None,
            sent_text: None,
            scroll: 0,
            revision: 1,
        }
    }

    fn view_text(view: &TerminalView) -> Vec<String> {
        view.rows
            .iter()
            .map(|row| {
                row.iter()
                    .filter(|cell| cell.width != 0)
                    .map(|cell| cell.character)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn a_view_shows_the_live_screen_by_default() {
        let mut session = session(8, 2);
        session.feed(b"one\r\ntwo\r\nthree");
        let view = session.view(2);
        assert_eq!(view_text(&view), vec!["two", "three"]);
        assert_eq!(view.cursor, Some((1, 5)));
        assert_eq!(view.scrollback, 0);
    }

    #[test]
    fn scrolling_back_reaches_history_and_leaves_the_cursor_behind() {
        let mut session = session(8, 2);
        session.feed(b"one\r\ntwo\r\nthree");
        session.scroll_back(1);
        let view = session.view(2);
        assert_eq!(view_text(&view), vec!["one", "two"]);
        assert_eq!(view.cursor, None);
        assert_eq!(view.scrollback, 1);
    }

    #[test]
    fn inline_tui_output_scrolled_above_a_fixed_composer_is_reviewable() {
        let mut session = session(8, 4);
        session.feed(b"\x1b[1;1Hanswer 1\x1b[2;1Hanswer 2\x1b[3;1Hcomposer\x1b[4;1Hstatus");

        // Ratatui's inline viewport uses a top-anchored scrolling region to
        // commit completed output without moving its composer and status rows.
        session.feed(b"\x1b[1;2r\x1b[S\x1b[r");

        assert_eq!(session.emulator.grid().scrollback_len(), 1);
        assert!(session.plain_text().starts_with("answer 1\nanswer 2\n"));
        session.scroll_back(1);
        assert_eq!(
            view_text(&session.view(4)),
            vec!["answer 1", "answer 2", "", "composer"]
        );
        assert!(
            session
                .ensure_review()
                .text
                .starts_with("answer 1\nanswer 2\n")
        );
    }

    #[test]
    fn scrolling_back_stops_at_the_oldest_line_kept() {
        let mut session = session(8, 2);
        session.feed(b"one\r\ntwo\r\nthree");
        session.scroll_back(100);
        assert_eq!(session.scroll(), 1);
        session.scroll_forward(100);
        assert_eq!(session.scroll(), 0);
    }

    /// The view has to follow the text, not the bottom of the screen. A
    /// reader who has scrolled back is holding a place in what was printed,
    /// and every new line the child prints moves the live screen away from it.
    #[test]
    fn a_scrolled_back_view_holds_still_while_the_child_keeps_printing() {
        let mut session = session(8, 2);
        session.feed(b"one\r\ntwo\r\nthree");
        session.scroll_back(1);
        assert_eq!(view_text(&session.view(2)), vec!["one", "two"]);

        session.feed(b"\r\nfour\r\nfive");
        assert_eq!(view_text(&session.view(2)), vec!["one", "two"]);
        assert_eq!(session.scroll(), 3);
    }

    #[test]
    fn a_view_following_the_live_screen_keeps_following_it() {
        let mut session = session(8, 2);
        session.feed(b"one\r\ntwo");
        session.feed(b"\r\nthree");
        assert_eq!(session.scroll(), 0);
        assert_eq!(view_text(&session.view(2)), vec!["two", "three"]);
    }

    /// Past the limit the line being read has genuinely been dropped, so the
    /// view settles on the oldest one still kept rather than running away.
    #[test]
    fn a_held_view_clamps_to_what_the_scrollback_still_keeps() {
        let mut session = session(8, 2);
        session.feed(b"one\r\ntwo\r\nthree");
        session.scroll_back(1);
        assert_eq!(session.scroll(), 1);
        for _ in 0..grid::SCROLLBACK_LIMIT + 50 {
            session.feed(b"\r\nx");
        }
        assert_eq!(session.scroll(), grid::SCROLLBACK_LIMIT);
    }

    #[test]
    fn the_alternate_screen_never_scrolls_into_the_history_behind_it() {
        let mut session = session(8, 2);
        session.feed(b"one\r\ntwo\r\nthree");
        session.feed(b"\x1b[?1049h");
        session.scroll_back(10);
        assert_eq!(session.scroll(), 0);
    }

    #[test]
    fn a_pane_taller_than_the_session_pads_above_the_output() {
        let mut session = session(8, 2);
        session.feed(b"one");
        let view = session.view(5);
        assert_eq!(view.rows.len(), 5);
        assert_eq!(view_text(&view), vec!["", "", "", "one", ""]);
    }

    #[test]
    fn review_oldest_and_old_matches_keep_a_full_viewport() {
        let mut session = session(8, 3);
        session.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
        assert_eq!(session.search_review("six", false).unwrap(), 1);
        session.scroll_to_oldest();
        assert_eq!(view_text(&session.view(3)), vec!["one", "two", "three"]);

        assert_eq!(session.search_review("one", false).unwrap(), 1);
        assert_eq!(
            view_text(&session.view(3)),
            vec!["one", "two", "three"],
            "focusing the oldest match must not pad above a one-line view"
        );
    }

    #[test]
    fn a_hidden_cursor_is_not_placed() {
        let mut session = session(8, 2);
        session.feed(b"\x1b[?25l");
        assert_eq!(session.view(2).cursor, None);
    }

    #[test]
    fn changed_default_colours_reach_existing_session_emulators() {
        let mut sessions = TerminalSessions::new();
        let existing = session(8, 2);
        let id = existing.id();
        sessions.sessions.insert(id, existing);

        sessions.set_default_colors(DefaultColors::new(
            Some((0xd6, 0xda, 0xe0)),
            Some((0x16, 0x18, 0x1d)),
        ));
        let emulator = &mut sessions.sessions.get_mut(&id).unwrap().emulator;
        emulator.feed(b"\x1b]11;?\x07");

        assert_eq!(emulator.take_replies(), b"\x1b]11;rgb:1616/1818/1d1d\x1b\\");
    }

    /// The bound is what keeps a runaway child from starving the event loop.
    /// The producer here never stops, exactly as `yes` never stops, so a drain
    /// without a limit would never return.
    #[test]
    fn draining_stops_at_one_queue_however_much_the_child_writes() {
        let mut sessions = TerminalSessions::new();
        let mut events = sessions.take_events().expect("the stream is available");
        let id = TerminalId(1);
        sessions.events.register(id);
        let sender = sessions.events.clone();
        let producer = std::thread::spawn(move || {
            // More than the loop could ever take in one pass, and blocking on
            // a full queue rather than growing it.
            for index in 0..OUTPUT_QUEUE * 8 {
                let message = TerminalOutput::Bytes {
                    id,
                    bytes: vec![b'y', b'\n'],
                };
                if !sender.send(message) {
                    return;
                }
                let _ = index;
            }
        });
        // Give the producer time to fill and refill the queue.
        while events.pending_for(id) < PER_SESSION_OUTPUT_QUEUE {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut applied = 0;
        let taken = drain(&mut events, |_| applied += 1);
        assert!(taken >= PER_SESSION_OUTPUT_QUEUE);
        assert!(taken <= OUTPUT_QUEUE);
        assert_eq!(applied, taken);

        sessions.events.remove(id);
        drop(events);
        producer.join().unwrap();
    }

    #[test]
    fn the_output_queue_is_bounded_so_a_child_cannot_grow_it_without_limit() {
        let mut sessions = TerminalSessions::new();
        let events = sessions.take_events().expect("the stream is available");
        let id = TerminalId(1);
        sessions.events.register(id);
        for _ in 0..PER_SESSION_OUTPUT_QUEUE {
            assert!(sessions.events.send(TerminalOutput::Bytes {
                id,
                bytes: vec![b'x'],
            }));
        }
        assert_eq!(events.pending_for(id), PER_SESSION_OUTPUT_QUEUE);
    }

    #[test]
    fn ready_sessions_are_drained_round_robin_and_exit_has_its_own_slot() {
        let mut sessions = TerminalSessions::new();
        let events = sessions.take_events().unwrap();
        let noisy = TerminalId(1);
        let quiet = TerminalId(2);
        sessions.events.register(noisy);
        sessions.events.register(quiet);
        for _ in 0..PER_SESSION_OUTPUT_QUEUE {
            assert!(sessions.events.send(TerminalOutput::Bytes {
                id: noisy,
                bytes: vec![b'n'],
            }));
        }
        assert!(sessions.events.send(TerminalOutput::Bytes {
            id: quiet,
            bytes: vec![b'q'],
        }));
        assert!(sessions.events.send(TerminalOutput::Exited {
            id: noisy,
            code: Some(7),
        }));

        assert_eq!(events.try_recv().unwrap().id(), noisy);
        assert_eq!(events.try_recv().unwrap().id(), quiet);
        for _ in 1..PER_SESSION_OUTPUT_QUEUE {
            assert!(matches!(
                events.try_recv().unwrap(),
                TerminalOutput::Bytes { id, .. } if id == noisy
            ));
        }
        assert!(matches!(
            events.try_recv().unwrap(),
            TerminalOutput::Exited {
                id,
                code: Some(7)
            } if id == noisy
        ));
    }

    #[test]
    fn workspace_cell_budget_is_derived_from_the_measured_cell_size() {
        assert!(std::mem::size_of::<Cell>() > 0);
        assert!(std::mem::size_of::<Cell>() <= 64);
        let measured = WORKSPACE_SCROLLBACK_CELLS * std::mem::size_of::<Cell>();
        assert!(measured <= WORKSPACE_TERMINAL_CELL_BYTES);
        assert!(WORKSPACE_TERMINAL_CELL_BYTES - measured < std::mem::size_of::<Cell>());
    }

    #[test]
    fn review_search_keeps_wide_text_and_new_output_stable() {
        let mut session = session(12, 3);
        session.feed("before\r\nwide 界 text\r\nafter".as_bytes());
        assert_eq!(session.search_review("界 text", false).unwrap(), 1);
        assert_eq!(session.review_selection_text(), "界 text");
        let before = view_text(&session.view(3));
        session.feed(b"\r\nnew output");
        let view = session.view(3);
        assert_eq!(view_text(&view), before);
        assert!(view.review);
        assert!(view.newer_output);
    }

    #[test]
    fn dense_review_search_maps_matches_in_one_forward_pass() {
        let mut session = session(200, 500);
        session.feed(&vec![b'a'; 100_000]);

        assert_eq!(session.search_review("a", false).unwrap(), 100_000);
        let review = session.review.as_ref().unwrap();
        assert_eq!(review.matches.first(), Some(&Range::new(0, 1)));
        assert_eq!(review.matches.last().unwrap().len(), 1);
    }

    #[test]
    fn review_memory_accounting_includes_search_matches() {
        let mut session = session(80, 20);
        session.feed(&vec![b'a'; 1_000]);
        session.ensure_review();
        let before = session.review_cells();

        assert_eq!(session.search_review("a", false).unwrap(), 1_000);
        assert!(session.review_cells() > before);
    }

    #[test]
    fn review_copy_preserves_unicode_and_line_breaks_and_motions_replace_or_extend() {
        let mut session = session(12, 3);
        session.feed("one\r\ntwo界\r\nthree".as_bytes());
        assert_eq!(session.search_review("one\ntwo界", false).unwrap(), 1);
        assert_eq!(session.review_selection_text(), "one\ntwo界");

        assert!(session.move_review(ReviewMotion::Right, false));
        assert_eq!(session.review_selection_text(), "t");
        assert!(session.move_review(ReviewMotion::Right, true));
        assert_eq!(session.review_selection_text(), "th");
        assert!(session.move_review(ReviewMotion::Right, true));
        assert_eq!(session.review_selection_text(), "thr");
        assert!(session.move_review(ReviewMotion::Up, false));
        assert_eq!(session.review_selection_text(), "o");
    }

    #[test]
    fn selecting_all_textless_review_does_not_copy_its_synthetic_newline() {
        for rows in [1, 3] {
            for output in [b"".as_slice(), b"   ".as_slice()] {
                let mut session = session(12, rows);
                session.feed(output);

                session.select_all_review();

                assert_eq!(session.review_selection_text(), "");
                assert_eq!(session.cursor_row(), 0);
                assert!(session.view(rows).cursor.is_some());
            }
        }
    }

    #[test]
    fn entering_review_places_a_visible_caret_at_the_child_cursor() {
        let mut session = session(12, 3);
        session.feed(b"alpha\r\nbeta");

        session.begin_review();
        let view = session.view(3);

        assert!(view.review);
        assert_eq!(view.cursor, Some((1, 3)));
        assert_eq!(session.review_selection_text(), "a");
        assert_eq!(session.cursor_row(), 1);
        assert_eq!(session.cursor_column(), 3);
    }

    #[test]
    fn vertical_review_motion_preserves_terminal_cell_columns() {
        let mut session = session(12, 3);
        session.feed("界a\r\nxyz".as_bytes());
        assert_eq!(session.search_review("a", false).unwrap(), 1);

        assert!(session.move_review(ReviewMotion::Left, false));
        assert_eq!(session.review_selection_text(), "界");
        assert!(session.move_review(ReviewMotion::Down, false));

        assert_eq!(session.review_selection_text(), "x");
    }

    #[test]
    fn review_line_selection_snaps_then_walks_both_directions() {
        let mut session = session(12, 4);
        session.feed(b"one\r\ntwo\r\nthree");
        assert_eq!(session.search_review("two", false).unwrap(), 1);

        session.select_review_line(true, false);
        assert_eq!(session.review_selection_text(), "two");
        session.select_review_line(true, true);
        assert_eq!(session.review_selection_text(), "two\nthree");
        session.select_review_line(false, true);
        assert_eq!(session.review_selection_text(), "two");
        session.select_review_line(false, true);
        assert_eq!(session.review_selection_text(), "one\ntwo");
    }

    #[test]
    fn review_line_selection_walks_over_blank_rows() {
        let mut session = session(12, 6);
        session.feed(b"one\r\n\r\ntwo\r\n\r\nthree");
        assert_eq!(session.search_review("one", false).unwrap(), 1);

        session.select_review_line(true, false);
        assert_eq!(session.review_selection_text(), "one");
        session.select_review_line(true, true);
        assert_eq!(session.review_selection_text(), "one\n\n");
        session.select_review_line(true, true);
        assert_eq!(session.review_selection_text(), "one\n\ntwo");

        assert_eq!(session.search_review("three", false).unwrap(), 1);
        session.select_review_line(false, false);
        assert_eq!(session.review_selection_text(), "three");
        session.select_review_line(false, true);
        assert_eq!(session.review_selection_text(), "\nthree");
        session.select_review_line(false, true);
        assert_eq!(session.review_selection_text(), "two\n\nthree");
    }

    #[test]
    fn review_line_selection_starting_on_a_blank_row_walks_on() {
        let mut session = session(12, 6);
        session.feed(b"one\r\n\r\ntwo");
        assert_eq!(session.search_review("one", false).unwrap(), 1);
        assert!(session.move_review(ReviewMotion::Down, false));

        session.select_review_line(true, false);
        assert_eq!(session.review_selection_text(), "\n");
        session.select_review_line(true, true);
        assert_eq!(session.review_selection_text(), "\ntwo");
    }

    #[test]
    fn extending_a_review_selection_scrolls_with_its_head() {
        let mut line_selection = session(12, 3);
        line_selection.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
        assert_eq!(line_selection.search_review("one", false).unwrap(), 1);

        line_selection.select_review_line(true, false);
        for _ in 0..3 {
            line_selection.select_review_line(true, true);
        }
        assert_eq!(
            view_text(&line_selection.view(3)),
            vec!["two", "three", "four"],
            "repeated x must bring the moving end of the selection into view"
        );

        let mut motion_selection = session(12, 3);
        motion_selection.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
        assert_eq!(motion_selection.search_review("one", false).unwrap(), 1);
        for _ in 0..3 {
            assert!(motion_selection.move_review(ReviewMotion::Down, true));
        }
        assert_eq!(
            view_text(&motion_selection.view(3)),
            vec!["two", "three", "four"],
            "v plus motion must bring the moving end of the selection into view"
        );
    }

    #[test]
    fn review_focus_keeps_a_file_like_margin_without_scrolling_every_motion() {
        let mut session = session(12, 5);
        session
            .feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\nseven\r\neight\r\nnine\r\nten");
        assert_eq!(session.search_review("ten", false).unwrap(), 1);

        session.focus_review_selection(5, 1);
        assert_eq!(
            view_text(&session.view(5)),
            vec!["seven", "eight", "nine", "ten", ""],
            "the review caret gets the same trailing margin as a file caret"
        );

        assert!(session.move_review(ReviewMotion::Up, false));
        session.focus_review_selection(5, 1);
        assert_eq!(
            view_text(&session.view(5)),
            vec!["seven", "eight", "nine", "ten", ""],
            "moving inside the margin must not realign the whole viewport"
        );
        assert_eq!(session.view(5).cursor.map(|cursor| cursor.0), Some(2));

        assert!(session.move_review(ReviewMotion::Up, false));
        assert!(session.move_review(ReviewMotion::Up, false));
        session.focus_review_selection(5, 1);
        assert_eq!(
            view_text(&session.view(5)),
            vec!["six", "seven", "eight", "nine", "ten"],
            "the viewport moves only after the caret crosses its top margin"
        );
        assert_eq!(session.view(5).cursor.map(|cursor| cursor.0), Some(1));
    }

    #[test]
    fn review_goto_motions_move_the_caret_instead_of_only_the_view() {
        let mut session = session(12, 4);
        session.feed(b"alpha\r\n  beta\r\n\r\ncharlie\r\ndelta\r\necho");
        session.begin_review();

        assert!(session.move_review(ReviewMotion::FileStart, false));
        assert_eq!(session.cursor_row(), 0);
        assert_eq!(session.review_selection_text(), "a");

        session.goto_review_line(2, false);
        assert!(session.move_review(ReviewMotion::FirstNonWhitespace, false));
        assert_eq!(session.cursor_row(), 1);
        assert_eq!(session.review_selection_text(), "b");

        assert!(session.move_review(ReviewMotion::NextParagraph, false));
        assert_eq!(session.cursor_row(), 3);
        assert!(session.move_review(ReviewMotion::PreviousParagraph, false));
        assert_eq!(session.cursor_row(), 0);

        assert!(session.move_review(ReviewMotion::FileEnd, false));
        assert_eq!(session.cursor_row(), 5);
        assert_eq!(session.review_selection_text(), "o");

        session.focus_review_selection(4, 1);
        assert!(session.move_review(ReviewMotion::WindowTop, false));
        assert_eq!(session.cursor_row(), 3);
        assert!(session.move_review(ReviewMotion::WindowCenter, false));
        assert_eq!(session.cursor_row(), 4);
        assert!(session.move_review(ReviewMotion::WindowBottom, false));
        assert_eq!(session.cursor_row(), 5);
    }

    #[test]
    fn review_word_page_and_character_motions_match_file_navigation() {
        let mut words = session(24, 4);
        words.feed(b"one, two\r\nthree_four! five");
        assert!(words.move_review(ReviewMotion::FileStart, false));

        assert!(words.move_review(ReviewMotion::WordForward, false));
        assert_eq!(words.review_selection_text(), ",");
        assert!(words.move_review(ReviewMotion::WordForward, false));
        assert_eq!(words.review_selection_text(), "t");
        assert!(words.move_review(ReviewMotion::WordEnd, false));
        assert_eq!(words.review_selection_text(), "o");
        assert!(words.move_review(ReviewMotion::LongWordBackward, false));
        assert_eq!(words.review_selection_text(), "t");
        assert!(words.move_review(ReviewMotion::LongWordForward, false));
        assert_eq!(words.review_selection_text(), "t");
        assert!(words.move_review(ReviewMotion::LongWordEnd, false));
        assert_eq!(words.review_selection_text(), "!");

        assert!(words.move_review(ReviewMotion::FileStart, false));
        assert!(words.find_review_character('t', true, true, false));
        assert_eq!(words.review_selection_text(), " ");
        assert!(words.find_review_character('n', false, false, false));
        assert_eq!(words.review_selection_text(), "n");

        let mut pages = session(12, 4);
        pages.feed(b"0\r\n1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8\r\n9");
        pages.goto_review_line(1, false);
        assert!(pages.move_review(ReviewMotion::PageDown, false));
        assert_eq!(pages.cursor_row(), 4);
        assert!(pages.move_review(ReviewMotion::HalfPageUp, false));
        assert_eq!(pages.cursor_row(), 2);
        assert!(pages.move_review(ReviewMotion::HalfPageDown, false));
        assert_eq!(pages.cursor_row(), 4);
        assert!(pages.move_review(ReviewMotion::PageUp, false));
        assert_eq!(pages.cursor_row(), 0);
    }

    #[test]
    fn review_goto_word_targets_and_labels_use_visible_terminal_cells() {
        let mut session = session(16, 3);
        session.feed(b"zulu omega\r\nlast row");
        assert!(session.move_review(ReviewMotion::FileStart, false));

        let targets = session.visible_review_word_targets(3);
        assert_eq!(targets[0], (0, 2));
        assert_eq!(targets[1], (5, 2));
        let labels = JumpLabels::with_visible_lengths(targets).unwrap();
        let view = session.view_with_jump_labels(3, &labels);
        assert_eq!(view_text(&view)[0], "aulu smega");
        assert!(view.highlights.iter().any(|highlight| {
            highlight.kind == TerminalHighlightKind::JumpLabelImmediate
                && highlight.row == 0
                && highlight.start_column == 0
        }));

        session.goto_review_offset(5, false);
        assert_eq!(session.review_selection_text(), "o");
    }

    #[test]
    fn review_copy_selection_skips_short_rows_and_line_selects_every_caret() {
        let mut session = session(12, 4);
        session.feed(b"ab\r\nx\r\ncd");
        assert_eq!(session.search_review("b", false).unwrap(), 1);

        assert!(session.copy_review_selection(true));
        assert_eq!(session.review_selection_count(), 2);
        assert_eq!(session.review_selection_text(), "b\nd");

        let view = session.view(4);
        assert!(
            view.highlights
                .iter()
                .any(|highlight| highlight.kind == TerminalHighlightKind::Selection)
        );

        session.select_review_line(true, false);
        assert_eq!(session.review_selection_text(), "ab\ncd");
    }

    #[test]
    fn sgr_mouse_encoding_preserves_coordinates_buttons_and_modifiers() {
        use crate::input::{Modifiers, PointerButton, PointerEvent, PointerEventKind};

        assert_eq!(
            TerminalSession::sgr_mouse_bytes(
                PointerEvent {
                    kind: PointerEventKind::Down(PointerButton::Left),
                    column: 0,
                    row: 0,
                    modifiers: Modifiers::CONTROL,
                },
                4,
                2,
            ),
            b"\x1b[<16;5;3M"
        );
        assert_eq!(
            TerminalSession::sgr_mouse_bytes(
                PointerEvent {
                    kind: PointerEventKind::Up(PointerButton::Left),
                    column: 0,
                    row: 0,
                    modifiers: Modifiers::NONE,
                },
                4,
                2,
            ),
            b"\x1b[<3;5;3m"
        );
        assert_eq!(
            TerminalSession::sgr_mouse_bytes_repeated(
                PointerEvent {
                    kind: PointerEventKind::ScrollDown,
                    column: 0,
                    row: 0,
                    modifiers: Modifiers::NONE,
                },
                4,
                2,
                3,
            ),
            b"\x1b[<65;5;3M\x1b[<65;5;3M\x1b[<65;5;3M"
        );
    }

    #[test]
    fn a_session_is_named_by_the_title_its_child_sets() {
        let mut session = session(8, 2);
        assert_eq!(session.name(), "test");
        assert_eq!(session.display_name(), "[terminal] test");
        session.feed(b"\x1b]0;claude\x07");
        assert_eq!(session.name(), "claude");
        assert_eq!(session.display_name(), "[terminal] claude");
    }

    #[test]
    fn a_user_name_wins_without_erasing_the_child_title() {
        let mut session = session(8, 2);
        session.feed(b"\x1b]0;child\x07");
        session.rename(Some("build".to_owned())).unwrap();
        assert_eq!(session.name(), "build");
        assert_eq!(session.child_title(), Some("child"));
        assert!(session.rename(Some("bad\nname".to_owned())).is_err());
    }

    #[test]
    fn explicit_terminal_targets_prefer_ids_and_refuse_ambiguous_names() {
        let mut sessions = TerminalSessions::new();
        let mut first = session(8, 2);
        first.id = TerminalId(7);
        first.rename(Some("agent".to_owned())).unwrap();
        let mut second = session(8, 2);
        second.id = TerminalId(9);
        second.rename(Some("agent".to_owned())).unwrap();
        sessions.sessions.insert(first.id(), first);
        sessions.sessions.insert(second.id(), second);

        assert_eq!(sessions.resolve("7"), Ok(TerminalId(7)));
        assert!(sessions.resolve("agent").unwrap_err().contains("ambiguous"));
        sessions
            .get_mut(TerminalId(9))
            .unwrap()
            .rename(Some("tests".to_owned()))
            .unwrap();
        assert_eq!(sessions.resolve("tests"), Ok(TerminalId(9)));
    }

    #[test]
    fn osc7_accepts_only_existing_local_directories() {
        let root = std::env::temp_dir().join(format!(
            "runyte-osc7-{}-{}",
            std::process::id(),
            session(1, 1).revision()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut session = session(8, 2);
        let encoded = root.to_string_lossy().replace(' ', "%20");
        session.feed(format!("\x1b]7;file://{encoded}\x07").as_bytes());
        assert_eq!(session.directory(), root);

        session.feed(b"\x1b]7;https://example.invalid/tmp\x07");
        assert_eq!(session.directory(), root);
        session.feed(b"\x1b]7;file://remote.invalid/tmp\x07");
        assert_eq!(session.directory(), root);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn typing_returns_a_scrolled_view_to_the_live_screen() {
        let mut session = session(8, 2);
        session.feed(b"one\r\ntwo\r\nthree");
        session.scroll_back(1);
        assert_eq!(session.scroll(), 1);
        session.send_key(KeyStroke::char('x'));
        assert_eq!(session.scroll(), 0);
    }
}
