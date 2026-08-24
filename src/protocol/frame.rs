// SPDX-License-Identifier: MPL-2.0

//! Wire-owned immutable frame values and core conversions.

use serde::{Deserialize, Serialize};

use crate::{
    config::{Color as CoreColor, Theme as CoreTheme},
    diff::Change as CoreChange,
    git::{CountKind as CoreCountKind, DiffLine as CoreDiffLine, LineChange as CoreLineChange},
    jump_labels::LabelPart as CoreLabelPart,
    lsp::Severity as CoreSeverity,
    notification::NotificationCounts as CoreNotificationCounts,
    snapshot as core,
    syntax::Scope,
    workspace::{FrameId as CoreFrameId, HostFrame as CoreHostFrame},
};

use super::{BufferId, BufferRevision, FrameGeometry, Rect, decode_path, encode_path};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FrameId(u64);

impl FrameId {
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<CoreFrameId> for FrameId {
    fn from(value: CoreFrameId) -> Self {
        Self(value.get())
    }
}

impl From<FrameId> for CoreFrameId {
    fn from(value: FrameId) -> Self {
        Self::from_raw(value.0)
    }
}

macro_rules! unit_enum {
    ($wire:ident, $core:path, [$($variant:ident),+ $(,)?]) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
        pub enum $wire { $($variant),+ }
        impl From<$core> for $wire {
            fn from(value: $core) -> Self {
                match value { $(<$core>::$variant => Self::$variant),+ }
            }
        }
        impl From<$wire> for $core {
            fn from(value: $wire) -> Self {
                match value { $($wire::$variant => Self::$variant),+ }
            }
        }
    };
}

unit_enum!(
    Mode,
    crate::command::Mode,
    [Normal, Insert, Select, Command]
);
unit_enum!(
    NotificationSeverity,
    crate::notification::NotificationSeverity,
    [Error, Warning, Info]
);
unit_enum!(
    OverlayKind,
    core::OverlayKind,
    [
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
        Prompt,
        Completion,
        Signature,
        Hover,
        KeyHints,
    ]
);
unit_enum!(
    OverlayPurpose,
    core::OverlayPurpose,
    [
        Picker,
        Choice,
        Manager,
        Report,
        Confirmation,
        CommandPalette,
        Context,
        Input,
        Info,
    ]
);
unit_enum!(OverlayInput, core::OverlayInput, [None, Filter, Text]);
unit_enum!(
    OverlayLayout,
    core::OverlayLayout,
    [Standard, Preview, Setting, SettingChoice, Anchored, Bottom]
);
unit_enum!(
    TextRole,
    core::TextRole,
    [
        Plain,
        Selected,
        PrimarySelected,
        PrimaryCaret,
        ReplaceCaret,
        Caret,
    ]
);
unit_enum!(Severity, CoreSeverity, [Hint, Information, Warning, Error]);
unit_enum!(
    LineChange,
    CoreLineChange,
    [Added, Modified, RemovedAbove, RemovedBelow]
);
unit_enum!(DiffLine, CoreDiffLine, [Added, Removed, Hunk, Meta]);
unit_enum!(Change, CoreChange, [Added, Removed, Changed]);
unit_enum!(LabelPart, CoreLabelPart, [Immediate, Prefix, Suffix]);
unit_enum!(CountKind, CoreCountKind, [Added, Removed]);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostFrame {
    pub id: FrameId,
    pub active_buffer: BufferId,
    pub active_revision: BufferRevision,
    pub editor: EditorSnapshot,
    pub overlays: Vec<OverlaySnapshot>,
}

/// Output-only update against one complete host frame. The reliable local
/// transport preserves order, while both frame and terminal revisions make a
/// stale update detectable instead of guessable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalDamageFrame {
    pub base: FrameId,
    pub id: FrameId,
    pub status: StatusSnapshot,
    pub panes: Vec<TerminalPaneDamage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalPaneDamage {
    pub pane_id: usize,
    pub title: PaneTitle,
    pub base_revision: u64,
    pub revision: u64,
    pub rows: Vec<TerminalRowDamage>,
    pub cursor: Option<(usize, usize)>,
    pub scrollback: usize,
    pub live: bool,
    pub review: bool,
    pub newer_output: bool,
    pub highlights: Vec<TerminalHighlight>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalRowDamage {
    pub row: usize,
    pub cells: Vec<TerminalCell>,
    pub line_id: Option<u64>,
}

impl TerminalDamageFrame {
    /// Produces a compact revision advance when at least one terminal pane is
    /// present and the editor panes otherwise stayed identical. Changed
    /// terminal rows are included when there are any. Layout, overlays, buffer
    /// rows, and mode changes require a complete frame; the compact status
    /// snapshot travels with terminal damage so concurrent service status is
    /// not held back.
    pub fn between(base: &HostFrame, next: &HostFrame) -> Option<Self> {
        if base.active_buffer != next.active_buffer
            || base.active_revision != next.active_revision
            || base.overlays != next.overlays
            || base.editor.geometry != next.editor.geometry
            || base.editor.theme != next.editor.theme
            || base.editor.mode != next.editor.mode
            || base.editor.panes.len() != next.editor.panes.len()
        {
            return None;
        }
        let mut panes = Vec::new();
        let mut terminal_present = false;
        for (old, new) in base.editor.panes.iter().zip(&next.editor.panes) {
            let mut old_shell = old.clone();
            let mut new_shell = new.clone();
            old_shell.title = new_shell.title.clone();
            old_shell.terminal = None;
            new_shell.terminal = None;
            if old_shell != new_shell {
                return None;
            }
            match (&old.terminal, &new.terminal) {
                (None, None) => {}
                (Some(old_terminal), Some(new_terminal))
                    if old_terminal.columns == new_terminal.columns
                        && old_terminal.rows.len() == new_terminal.rows.len()
                        && old_terminal.line_ids.len() == new_terminal.line_ids.len() =>
                {
                    terminal_present = true;
                    if old_terminal == new_terminal && old.title == new.title {
                        continue;
                    }
                    let rows = old_terminal
                        .rows
                        .iter()
                        .zip(&new_terminal.rows)
                        .zip(old_terminal.line_ids.iter().zip(&new_terminal.line_ids))
                        .enumerate()
                        .filter(|(_, ((old_cells, new_cells), (old_id, new_id)))| {
                            old_cells != new_cells || old_id != new_id
                        })
                        .map(|(row, ((_, new_cells), (_, new_id)))| TerminalRowDamage {
                            row,
                            cells: new_cells.clone(),
                            line_id: *new_id,
                        })
                        .collect();
                    panes.push(TerminalPaneDamage {
                        pane_id: new.pane_id,
                        title: new.title.clone(),
                        base_revision: old_terminal.revision,
                        revision: new_terminal.revision,
                        rows,
                        cursor: new_terminal.cursor,
                        scrollback: new_terminal.scrollback,
                        live: new_terminal.live,
                        review: new_terminal.review,
                        newer_output: new_terminal.newer_output,
                        highlights: new_terminal.highlights.clone(),
                    });
                }
                _ => return None,
            }
        }
        if !terminal_present {
            return None;
        }
        Some(Self {
            base: base.id,
            id: next.id,
            status: next.editor.status.clone(),
            panes,
        })
    }

    /// Applies atomically. A missing pane, stale frame, or stale terminal
    /// revision leaves the complete cached frame untouched.
    pub fn apply(&self, frame: &mut HostFrame) -> bool {
        if frame.id != self.base
            || self.panes.iter().any(|damage| {
                frame
                    .editor
                    .panes
                    .iter()
                    .find(|pane| pane.pane_id == damage.pane_id)
                    .and_then(|pane| pane.terminal.as_ref())
                    .is_none_or(|terminal| terminal.revision != damage.base_revision)
            })
        {
            return false;
        }
        let mut updated = frame.clone();
        for damage in &self.panes {
            let pane = updated
                .editor
                .panes
                .iter_mut()
                .find(|pane| pane.pane_id == damage.pane_id)
                .expect("damage pane was validated");
            let terminal = pane.terminal.as_mut().expect("terminal was validated");
            if damage.rows.iter().any(|row| row.row >= terminal.rows.len()) {
                return false;
            }
            for row in &damage.rows {
                terminal.rows[row.row] = row.cells.clone();
                terminal.line_ids[row.row] = row.line_id;
            }
            pane.title = damage.title.clone();
            terminal.revision = damage.revision;
            terminal.cursor = damage.cursor;
            terminal.scrollback = damage.scrollback;
            terminal.live = damage.live;
            terminal.review = damage.review;
            terminal.newer_output = damage.newer_output;
            terminal.highlights = damage.highlights.clone();
        }
        updated.id = self.id;
        updated.editor.status = self.status.clone();
        *frame = updated;
        true
    }
}

impl From<CoreHostFrame> for HostFrame {
    fn from(value: CoreHostFrame) -> Self {
        Self {
            id: value.id.into(),
            active_buffer: value.active_buffer.into(),
            active_revision: value.active_revision.into(),
            editor: value.editor.into(),
            overlays: value.overlays.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<HostFrame> for CoreHostFrame {
    type Error = String;
    fn try_from(value: HostFrame) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id.into(),
            active_buffer: value.active_buffer.into(),
            active_revision: value.active_revision.into(),
            editor: value.editor.try_into()?,
            overlays: value
                .overlays
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EditorSnapshot {
    pub geometry: FrameGeometry,
    pub theme: Theme,
    pub mode: Mode,
    pub panes: Vec<PaneSnapshot>,
    pub status: StatusSnapshot,
}

impl From<core::EditorSnapshot> for EditorSnapshot {
    fn from(value: core::EditorSnapshot) -> Self {
        Self {
            geometry: value.geometry.into(),
            theme: value.theme.into(),
            mode: value.mode.into(),
            panes: value.panes.into_iter().map(Into::into).collect(),
            status: value.status.into(),
        }
    }
}

impl TryFrom<EditorSnapshot> for core::EditorSnapshot {
    type Error = String;
    fn try_from(value: EditorSnapshot) -> Result<Self, Self::Error> {
        Ok(Self {
            geometry: value.geometry.into(),
            theme: value.theme.into(),
            mode: value.mode.into(),
            panes: value
                .panes
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            status: value.status.into(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OverlaySnapshot {
    pub kind: OverlayKind,
    pub purpose: OverlayPurpose,
    pub input: OverlayInput,
    pub layout: OverlayLayout,
    pub actions: Vec<OverlayAction>,
    pub title: String,
    pub query: String,
    pub rows: Vec<OverlayRow>,
    pub selected: Option<usize>,
    pub scroll_anchor: Option<usize>,
    pub row_offset: usize,
    pub message: Option<String>,
    pub omitted_rows: usize,
    pub total_rows: usize,
    pub query_cursor: Option<usize>,
    pub show_preview: bool,
    pub preview_title: Option<String>,
    pub preview: Option<OverlayPreview>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OverlayAction {
    pub key_hint: String,
    pub label: String,
}

impl From<core::OverlayAction> for OverlayAction {
    fn from(value: core::OverlayAction) -> Self {
        Self {
            key_hint: value.key_hint,
            label: value.label,
        }
    }
}

impl From<OverlayAction> for core::OverlayAction {
    fn from(value: OverlayAction) -> Self {
        Self {
            key_hint: value.key_hint,
            label: value.label,
        }
    }
}

impl From<core::OverlaySnapshot> for OverlaySnapshot {
    fn from(value: core::OverlaySnapshot) -> Self {
        Self {
            kind: value.kind.into(),
            purpose: value.purpose.into(),
            input: value.input.into(),
            layout: value.layout.into(),
            actions: value.actions.into_iter().map(Into::into).collect(),
            title: value.title,
            query: value.query,
            rows: value.rows.into_iter().map(Into::into).collect(),
            selected: value.selected,
            scroll_anchor: value.scroll_anchor,
            row_offset: value.row_offset,
            message: value.message,
            omitted_rows: value.omitted_rows,
            total_rows: value.total_rows,
            query_cursor: value.query_cursor,
            show_preview: value.show_preview,
            preview_title: value.preview_title,
            preview: value.preview.map(Into::into),
        }
    }
}

impl TryFrom<OverlaySnapshot> for core::OverlaySnapshot {
    type Error = String;
    fn try_from(value: OverlaySnapshot) -> Result<Self, Self::Error> {
        Ok(Self {
            kind: value.kind.into(),
            purpose: value.purpose.into(),
            input: value.input.into(),
            layout: value.layout.into(),
            actions: value.actions.into_iter().map(Into::into).collect(),
            title: value.title,
            query: value.query,
            rows: value.rows.into_iter().map(Into::into).collect(),
            selected: value.selected,
            scroll_anchor: value.scroll_anchor,
            row_offset: value.row_offset,
            message: value.message,
            omitted_rows: value.omitted_rows,
            total_rows: value.total_rows,
            query_cursor: value.query_cursor,
            show_preview: value.show_preview,
            preview_title: value.preview_title,
            preview: value.preview.map(Into::into),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OverlayRow {
    pub identity: OverlayIdentity,
    pub label: String,
    pub detail: String,
    pub available: bool,
    pub dimmed: bool,
    pub emphasis: Vec<usize>,
}

impl From<core::OverlayRow> for OverlayRow {
    fn from(value: core::OverlayRow) -> Self {
        Self {
            identity: value.identity.into(),
            label: value.label,
            detail: value.detail,
            available: value.available,
            dimmed: value.dimmed,
            emphasis: value.emphasis,
        }
    }
}
impl From<OverlayRow> for core::OverlayRow {
    fn from(value: OverlayRow) -> Self {
        Self {
            identity: value.identity.into(),
            label: value.label,
            detail: value.detail,
            available: value.available,
            dimmed: value.dimmed,
            emphasis: value.emphasis,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_row_availability_survives_the_wire_round_trip() {
        let row = core::OverlayRow {
            identity: core::OverlayIdentity::Text("Space l".to_owned()),
            label: "Space l".to_owned(),
            detail: "Language (LSP) · unavailable: no server".to_owned(),
            available: false,
            dimmed: true,
            emphasis: Vec::new(),
        };

        let wire = OverlayRow::from(row.clone());
        let encoded = serde_json::to_vec(&wire).unwrap();
        let decoded: OverlayRow = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(core::OverlayRow::from(decoded), row);
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OverlayPreview {
    Text(Vec<String>),
    MatchedText {
        lines: Vec<String>,
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
impl From<core::OverlayPreview> for OverlayPreview {
    fn from(value: core::OverlayPreview) -> Self {
        match value {
            core::OverlayPreview::Text(rows) => Self::Text(rows),
            core::OverlayPreview::MatchedText { lines, emphasis } => {
                Self::MatchedText { lines, emphasis }
            }
            core::OverlayPreview::Snippet {
                lines,
                start_row,
                focus_row,
                emphasis,
            } => Self::Snippet {
                lines,
                start_row,
                focus_row,
                emphasis,
            },
            core::OverlayPreview::Binary => Self::Binary,
            core::OverlayPreview::Unavailable(reason) => Self::Unavailable(reason),
            core::OverlayPreview::Empty => Self::Empty,
        }
    }
}
impl From<OverlayPreview> for core::OverlayPreview {
    fn from(value: OverlayPreview) -> Self {
        match value {
            OverlayPreview::Text(rows) => Self::Text(rows),
            OverlayPreview::MatchedText { lines, emphasis } => {
                Self::MatchedText { lines, emphasis }
            }
            OverlayPreview::Snippet {
                lines,
                start_row,
                focus_row,
                emphasis,
            } => Self::Snippet {
                lines,
                start_row,
                focus_row,
                emphasis,
            },
            OverlayPreview::Binary => Self::Binary,
            OverlayPreview::Unavailable(reason) => Self::Unavailable(reason),
            OverlayPreview::Empty => Self::Empty,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OverlayIdentity {
    Text(String),
    Path(Vec<u8>),
    Index(usize),
}
impl From<core::OverlayIdentity> for OverlayIdentity {
    fn from(value: core::OverlayIdentity) -> Self {
        match value {
            core::OverlayIdentity::Text(text) => Self::Text(text),
            core::OverlayIdentity::Path(path) => Self::Path(encode_path(&path)),
            core::OverlayIdentity::Index(index) => Self::Index(index),
        }
    }
}
impl From<OverlayIdentity> for core::OverlayIdentity {
    fn from(value: OverlayIdentity) -> Self {
        match value {
            OverlayIdentity::Text(text) => Self::Text(text),
            OverlayIdentity::Path(path) => Self::Path(decode_path(path)),
            OverlayIdentity::Index(index) => Self::Index(index),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneSnapshot {
    pub pane_id: usize,
    pub area: Rect,
    pub body: Rect,
    pub active: bool,
    pub jump_active: bool,
    pub dimmed: bool,
    pub drawable: bool,
    pub title: PaneTitle,
    pub line_numbers: bool,
    pub line_digits: usize,
    pub signs: bool,
    pub changes: bool,
    pub text_width: usize,
    pub gutter_width: usize,
    pub content_indent: usize,
    pub scroll_row: usize,
    pub scroll_wrap: usize,
    pub wrap_width: usize,
    pub cursor_screen_row: Option<usize>,
    pub rows: Vec<SnapshotRow>,
    pub terminal: Option<TerminalView>,
}

/// A terminal pane's rectangle of styled cells.
///
/// The deliberate exception to this protocol's rule that the host ships
/// semantics and the client resolves colour. A `TextRun` names a tree-sitter
/// scope because the editor knows what a run of text *is*; a child process on
/// a pty has only ever said what colour it wants. There is nothing semantic
/// left to send, so the colour itself is what crosses the wire, and only
/// [`TerminalColor::Default`] is still the client's to resolve.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalView {
    pub revision: u64,
    pub columns: usize,
    pub rows: Vec<Vec<TerminalCell>>,
    pub line_ids: Vec<Option<u64>>,
    pub cursor: Option<(usize, usize)>,
    pub scrollback: usize,
    pub live: bool,
    pub review: bool,
    pub newer_output: bool,
    pub highlights: Vec<TerminalHighlight>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalHighlight {
    pub row: usize,
    pub start_column: usize,
    pub end_column: usize,
    pub kind: TerminalHighlightKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TerminalHighlightKind {
    Match,
    ActiveMatch,
    Selection,
    JumpLabelImmediate,
    JumpLabelPrefix,
    JumpLabelSuffix,
}

impl From<crate::terminal::TerminalHighlightKind> for TerminalHighlightKind {
    fn from(value: crate::terminal::TerminalHighlightKind) -> Self {
        match value {
            crate::terminal::TerminalHighlightKind::Match => Self::Match,
            crate::terminal::TerminalHighlightKind::ActiveMatch => Self::ActiveMatch,
            crate::terminal::TerminalHighlightKind::Selection => Self::Selection,
            crate::terminal::TerminalHighlightKind::JumpLabelImmediate => Self::JumpLabelImmediate,
            crate::terminal::TerminalHighlightKind::JumpLabelPrefix => Self::JumpLabelPrefix,
            crate::terminal::TerminalHighlightKind::JumpLabelSuffix => Self::JumpLabelSuffix,
        }
    }
}

impl From<TerminalHighlightKind> for crate::terminal::TerminalHighlightKind {
    fn from(value: TerminalHighlightKind) -> Self {
        match value {
            TerminalHighlightKind::Match => Self::Match,
            TerminalHighlightKind::ActiveMatch => Self::ActiveMatch,
            TerminalHighlightKind::Selection => Self::Selection,
            TerminalHighlightKind::JumpLabelImmediate => Self::JumpLabelImmediate,
            TerminalHighlightKind::JumpLabelPrefix => Self::JumpLabelPrefix,
            TerminalHighlightKind::JumpLabelSuffix => Self::JumpLabelSuffix,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalCell {
    pub character: char,
    pub combining: Vec<char>,
    pub width: u8,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    /// The attribute bit set, carried as its bits so a frontend that gains an
    /// attribute needs no protocol change to ignore one it does not know.
    pub attributes: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TerminalColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<crate::terminal::Color> for TerminalColor {
    fn from(value: crate::terminal::Color) -> Self {
        match value {
            crate::terminal::Color::Default => Self::Default,
            crate::terminal::Color::Indexed(index) => Self::Indexed(index),
            crate::terminal::Color::Rgb(red, green, blue) => Self::Rgb(red, green, blue),
        }
    }
}
impl From<TerminalColor> for crate::terminal::Color {
    fn from(value: TerminalColor) -> Self {
        match value {
            TerminalColor::Default => Self::Default,
            TerminalColor::Indexed(index) => Self::Indexed(index),
            TerminalColor::Rgb(red, green, blue) => Self::Rgb(red, green, blue),
        }
    }
}

impl From<crate::terminal::Cell> for TerminalCell {
    fn from(value: crate::terminal::Cell) -> Self {
        Self {
            character: value.character,
            combining: value.combining[..usize::from(value.combining_len)].to_vec(),
            width: value.width,
            foreground: value.foreground.into(),
            background: value.background.into(),
            attributes: value.attributes.bits(),
        }
    }
}
impl From<TerminalCell> for crate::terminal::Cell {
    fn from(value: TerminalCell) -> Self {
        Self {
            character: value.character,
            combining: {
                let mut combining = ['\0'; 3];
                for (target, source) in combining.iter_mut().zip(value.combining.iter().copied()) {
                    *target = source;
                }
                combining
            },
            combining_len: value.combining.len().min(3) as u8,
            width: value.width,
            foreground: value.foreground.into(),
            background: value.background.into(),
            attributes: crate::terminal::Attributes::from_bits(value.attributes),
        }
    }
}

impl From<crate::terminal::TerminalView> for TerminalView {
    fn from(value: crate::terminal::TerminalView) -> Self {
        Self {
            revision: value.revision,
            columns: value.columns,
            rows: value
                .rows
                .into_iter()
                .map(|row| row.into_iter().map(Into::into).collect())
                .collect(),
            line_ids: value.line_ids,
            cursor: value.cursor,
            scrollback: value.scrollback,
            live: value.live,
            review: value.review,
            newer_output: value.newer_output,
            highlights: value
                .highlights
                .into_iter()
                .map(|highlight| TerminalHighlight {
                    row: highlight.row,
                    start_column: highlight.start_column,
                    end_column: highlight.end_column,
                    kind: highlight.kind.into(),
                })
                .collect(),
        }
    }
}
impl From<TerminalView> for crate::terminal::TerminalView {
    fn from(value: TerminalView) -> Self {
        Self {
            revision: value.revision,
            columns: value.columns,
            rows: value
                .rows
                .into_iter()
                .map(|row| row.into_iter().map(Into::into).collect())
                .collect(),
            line_ids: value.line_ids,
            cursor: value.cursor,
            scrollback: value.scrollback,
            live: value.live,
            review: value.review,
            newer_output: value.newer_output,
            highlights: value
                .highlights
                .into_iter()
                .map(|highlight| crate::terminal::TerminalHighlight {
                    row: highlight.row,
                    start_column: highlight.start_column,
                    end_column: highlight.end_column,
                    kind: highlight.kind.into(),
                })
                .collect(),
        }
    }
}

impl From<core::PaneSnapshot> for PaneSnapshot {
    fn from(value: core::PaneSnapshot) -> Self {
        Self {
            pane_id: value.pane_id,
            area: value.area.into(),
            body: value.body.into(),
            active: value.active,
            jump_active: value.jump_active,
            dimmed: value.dimmed,
            drawable: value.drawable,
            title: value.title.into(),
            line_numbers: value.line_numbers,
            line_digits: value.line_digits,
            signs: value.signs,
            changes: value.changes,
            text_width: value.text_width,
            gutter_width: value.gutter_width,
            content_indent: value.content_indent,
            scroll_row: value.scroll_row,
            scroll_wrap: value.scroll_wrap,
            wrap_width: value.wrap_width,
            cursor_screen_row: value.cursor_screen_row,
            rows: value.rows.into_iter().map(Into::into).collect(),
            terminal: value.terminal.map(Into::into),
        }
    }
}
impl TryFrom<PaneSnapshot> for core::PaneSnapshot {
    type Error = String;
    fn try_from(value: PaneSnapshot) -> Result<Self, Self::Error> {
        Ok(Self {
            pane_id: value.pane_id,
            area: value.area.into(),
            body: value.body.into(),
            active: value.active,
            jump_active: value.jump_active,
            dimmed: value.dimmed,
            drawable: value.drawable,
            title: value.title.into(),
            line_numbers: value.line_numbers,
            line_digits: value.line_digits,
            signs: value.signs,
            changes: value.changes,
            text_width: value.text_width,
            gutter_width: value.gutter_width,
            content_indent: value.content_indent,
            scroll_row: value.scroll_row,
            scroll_wrap: value.scroll_wrap,
            wrap_width: value.wrap_width,
            cursor_screen_row: value.cursor_screen_row,
            rows: value
                .rows
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            terminal: value.terminal.map(Into::into),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneTitle {
    pub name: String,
    pub dirty: bool,
    pub read_only: bool,
}
impl From<core::PaneTitle> for PaneTitle {
    fn from(value: core::PaneTitle) -> Self {
        Self {
            name: value.name,
            dirty: value.dirty,
            read_only: value.read_only,
        }
    }
}
impl From<PaneTitle> for core::PaneTitle {
    fn from(value: PaneTitle) -> Self {
        Self {
            name: value.name,
            dirty: value.dirty,
            read_only: value.read_only,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SnapshotRow {
    Placeholder,
    Filler,
    Padding,
    Text(VisibleRow),
}
impl From<core::SnapshotRow> for SnapshotRow {
    fn from(value: core::SnapshotRow) -> Self {
        match value {
            core::SnapshotRow::Placeholder => Self::Placeholder,
            core::SnapshotRow::Filler => Self::Filler,
            core::SnapshotRow::Padding => Self::Padding,
            core::SnapshotRow::Text(row) => Self::Text(row.into()),
        }
    }
}
impl TryFrom<SnapshotRow> for core::SnapshotRow {
    type Error = String;
    fn try_from(value: SnapshotRow) -> Result<Self, Self::Error> {
        match value {
            SnapshotRow::Placeholder => Ok(Self::Placeholder),
            SnapshotRow::Filler => Ok(Self::Filler),
            SnapshotRow::Padding => Ok(Self::Padding),
            SnapshotRow::Text(row) => Ok(Self::Text(row.try_into()?)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VisibleRow {
    pub document_row: usize,
    pub continuation: bool,
    pub folded: bool,
    pub cursor_row: bool,
    pub diagnostic_sign: Option<Severity>,
    pub change: Option<LineChange>,
    pub diff: Option<DiffLine>,
    pub compared: Option<Change>,
    pub notification_severity: Option<NotificationSeverity>,
    pub runs: Vec<TextRun>,
}
impl From<core::VisibleRow> for VisibleRow {
    fn from(value: core::VisibleRow) -> Self {
        Self {
            document_row: value.document_row,
            continuation: value.continuation,
            folded: value.folded,
            cursor_row: value.cursor_row,
            diagnostic_sign: value.diagnostic_sign.map(Into::into),
            change: value.change.map(Into::into),
            diff: value.diff.map(Into::into),
            compared: value.compared.map(Into::into),
            notification_severity: value.notification_severity.map(Into::into),
            runs: value.runs.into_iter().map(Into::into).collect(),
        }
    }
}
impl TryFrom<VisibleRow> for core::VisibleRow {
    type Error = String;
    fn try_from(value: VisibleRow) -> Result<Self, Self::Error> {
        Ok(Self {
            document_row: value.document_row,
            continuation: value.continuation,
            folded: value.folded,
            cursor_row: value.cursor_row,
            diagnostic_sign: value.diagnostic_sign.map(Into::into),
            change: value.change.map(Into::into),
            diff: value.diff.map(Into::into),
            compared: value.compared.map(Into::into),
            notification_severity: value.notification_severity.map(Into::into),
            runs: value
                .runs
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextRun {
    pub text: String,
    pub kind: TextRunKind,
}
impl From<core::TextRun> for TextRun {
    fn from(value: core::TextRun) -> Self {
        Self {
            text: value.text,
            kind: value.kind.into(),
        }
    }
}
impl TryFrom<TextRun> for core::TextRun {
    type Error = String;
    fn try_from(value: TextRun) -> Result<Self, Self::Error> {
        Ok(Self {
            text: value.text,
            kind: value.kind.try_into()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TextRunKind {
    Text {
        role: TextRole,
        scope: Option<String>,
        diagnostic: Option<Severity>,
        directory: bool,
        count: Option<CountKind>,
    },
    JumpLabel(LabelPart),
    InlineDiagnostic(Severity),
    FoldMarker,
    Hint,
}
impl From<core::TextRunKind> for TextRunKind {
    fn from(value: core::TextRunKind) -> Self {
        match value {
            core::TextRunKind::Text {
                role,
                scope,
                diagnostic,
                directory,
                count,
            } => Self::Text {
                role: role.into(),
                scope: scope.map(|scope| scope.name().to_owned()),
                diagnostic: diagnostic.map(Into::into),
                directory,
                count: count.map(Into::into),
            },
            core::TextRunKind::JumpLabel(part) => Self::JumpLabel(part.into()),
            core::TextRunKind::InlineDiagnostic(severity) => {
                Self::InlineDiagnostic(severity.into())
            }
            core::TextRunKind::FoldMarker => Self::FoldMarker,
            core::TextRunKind::Hint => Self::Hint,
        }
    }
}
impl TryFrom<TextRunKind> for core::TextRunKind {
    type Error = String;
    fn try_from(value: TextRunKind) -> Result<Self, Self::Error> {
        match value {
            TextRunKind::Text {
                role,
                scope,
                diagnostic,
                directory,
                count,
            } => Ok(Self::Text {
                role: role.into(),
                scope: scope
                    .map(|name| {
                        Scope::named(&name).ok_or_else(|| format!("unknown snapshot scope {name}"))
                    })
                    .transpose()?,
                diagnostic: diagnostic.map(Into::into),
                directory,
                count: count.map(Into::into),
            }),
            TextRunKind::JumpLabel(part) => Ok(Self::JumpLabel(part.into())),
            TextRunKind::InlineDiagnostic(severity) => Ok(Self::InlineDiagnostic(severity.into())),
            TextRunKind::FoldMarker => Ok(Self::FoldMarker),
            TextRunKind::Hint => Ok(Self::Hint),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StatusSnapshot {
    pub mode: Mode,
    #[serde(default)]
    pub workspace_number: Option<u8>,
    pub workspace_directory: String,
    pub dirty: bool,
    pub read_only: bool,
    pub cursor: Position,
    pub line_count: usize,
    pub selection_count: usize,
    pub lsp_summary: Option<String>,
    pub git_summary: Option<String>,
    pub long_running_action: Option<LongRunningActionSnapshot>,
    pub notification_counts: NotificationCounts,
    pub interaction_line: String,
    pub interaction_line_error: bool,
    pub prompt_cursor_column: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NotificationCounts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

impl From<CoreNotificationCounts> for NotificationCounts {
    fn from(value: CoreNotificationCounts) -> Self {
        Self {
            errors: value.errors,
            warnings: value.warnings,
            infos: value.infos,
        }
    }
}

impl From<NotificationCounts> for CoreNotificationCounts {
    fn from(value: NotificationCounts) -> Self {
        Self {
            errors: value.errors,
            warnings: value.warnings,
            infos: value.infos,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LongRunningActionSnapshot {
    pub label: String,
    pub detail: String,
    pub elapsed_millis: u64,
    pub cancel_hint: Option<String>,
}

impl From<core::LongRunningActionSnapshot> for LongRunningActionSnapshot {
    fn from(value: core::LongRunningActionSnapshot) -> Self {
        Self {
            label: value.label,
            detail: value.detail,
            elapsed_millis: value.elapsed_millis,
            cancel_hint: value.cancel_hint,
        }
    }
}

impl From<LongRunningActionSnapshot> for core::LongRunningActionSnapshot {
    fn from(value: LongRunningActionSnapshot) -> Self {
        Self {
            label: value.label,
            detail: value.detail,
            elapsed_millis: value.elapsed_millis,
            cancel_hint: value.cancel_hint,
        }
    }
}

impl From<core::StatusSnapshot> for StatusSnapshot {
    fn from(value: core::StatusSnapshot) -> Self {
        Self {
            mode: value.mode.into(),
            workspace_number: value.workspace_number,
            workspace_directory: value.workspace_directory,
            dirty: value.dirty,
            read_only: value.read_only,
            cursor: value.cursor.into(),
            line_count: value.line_count,
            selection_count: value.selection_count,
            lsp_summary: value.lsp_summary,
            git_summary: value.git_summary,
            long_running_action: value.long_running_action.map(Into::into),
            notification_counts: value.notification_counts.into(),
            interaction_line: value.interaction_line,
            interaction_line_error: value.interaction_line_error,
            prompt_cursor_column: value.prompt_cursor_column,
        }
    }
}
impl From<StatusSnapshot> for core::StatusSnapshot {
    fn from(value: StatusSnapshot) -> Self {
        Self {
            mode: value.mode.into(),
            workspace_number: value.workspace_number,
            workspace_directory: value.workspace_directory,
            dirty: value.dirty,
            read_only: value.read_only,
            cursor: value.cursor.into(),
            line_count: value.line_count,
            selection_count: value.selection_count,
            lsp_summary: value.lsp_summary,
            git_summary: value.git_summary,
            long_running_action: value.long_running_action.map(Into::into),
            notification_counts: value.notification_counts.into(),
            interaction_line: value.interaction_line,
            interaction_line_error: value.interaction_line_error,
            prompt_cursor_column: value.prompt_cursor_column,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}
impl From<crate::text::Position> for Position {
    fn from(value: crate::text::Position) -> Self {
        Self {
            row: value.row,
            col: value.col,
        }
    }
}
impl From<Position> for crate::text::Position {
    fn from(value: Position) -> Self {
        Self::new(value.row, value.col)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub muted: Color,
    pub jump_text_muted: Color,
    pub accent: Color,
    pub cursor_normal: Color,
    pub cursor_insert: Color,
    pub cursor_select: Color,
    pub cursor_command: Color,
    pub directory: Color,
    pub selection: Color,
    pub selection_primary: Color,
    pub fuzzy_match_secondary: Color,
    pub fuzzy_match_primary: Color,
    pub status_background: Color,
    pub status_foreground: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub jump_label_immediate: Color,
    pub jump_label_primary: Color,
    pub jump_label_secondary: Color,
    pub change_added: Color,
    pub change_modified: Color,
    pub change_removed: Color,
    pub diff_added: Option<Color>,
    pub diff_removed: Option<Color>,
    pub diff_changed: Option<Color>,
    pub syntax: Vec<Option<Color>>,
}

macro_rules! theme {
    ($value:ident, $map:expr) => {
        Self {
            background: $map($value.background),
            foreground: $map($value.foreground),
            muted: $map($value.muted),
            jump_text_muted: $map($value.jump_text_muted),
            accent: $map($value.accent),
            cursor_normal: $map($value.cursor_normal),
            cursor_insert: $map($value.cursor_insert),
            cursor_select: $map($value.cursor_select),
            cursor_command: $map($value.cursor_command),
            directory: $map($value.directory),
            selection: $map($value.selection),
            selection_primary: $map($value.selection_primary),
            fuzzy_match_secondary: $map($value.fuzzy_match_secondary),
            fuzzy_match_primary: $map($value.fuzzy_match_primary),
            status_background: $map($value.status_background),
            status_foreground: $map($value.status_foreground),
            error: $map($value.error),
            warning: $map($value.warning),
            info: $map($value.info),
            jump_label_immediate: $map($value.jump_label_immediate),
            jump_label_primary: $map($value.jump_label_primary),
            jump_label_secondary: $map($value.jump_label_secondary),
            change_added: $map($value.change_added),
            change_modified: $map($value.change_modified),
            change_removed: $map($value.change_removed),
            diff_added: $value.diff_added.map($map),
            diff_removed: $value.diff_removed.map($map),
            diff_changed: $value.diff_changed.map($map),
            syntax: $value
                .syntax
                .into_iter()
                .map(|color| color.map($map))
                .collect(),
        }
    };
}
impl From<CoreTheme> for Theme {
    fn from(value: CoreTheme) -> Self {
        theme!(value, Color::from)
    }
}
impl From<Theme> for CoreTheme {
    fn from(value: Theme) -> Self {
        theme!(value, CoreColor::from)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Color {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Gray,
    DarkGray,
    Rgb(u8, u8, u8),
}
impl From<CoreColor> for Color {
    fn from(value: CoreColor) -> Self {
        match value {
            CoreColor::Reset => Self::Reset,
            CoreColor::Black => Self::Black,
            CoreColor::Red => Self::Red,
            CoreColor::Green => Self::Green,
            CoreColor::Yellow => Self::Yellow,
            CoreColor::Blue => Self::Blue,
            CoreColor::Magenta => Self::Magenta,
            CoreColor::Cyan => Self::Cyan,
            CoreColor::White => Self::White,
            CoreColor::Gray => Self::Gray,
            CoreColor::DarkGray => Self::DarkGray,
            CoreColor::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}
impl From<Color> for CoreColor {
    fn from(value: Color) -> Self {
        match value {
            Color::Reset => Self::Reset,
            Color::Black => Self::Black,
            Color::Red => Self::Red,
            Color::Green => Self::Green,
            Color::Yellow => Self::Yellow,
            Color::Blue => Self::Blue,
            Color::Magenta => Self::Magenta,
            Color::Cyan => Self::Cyan,
            Color::White => Self::White,
            Color::Gray => Self::Gray,
            Color::DarkGray => Self::DarkGray,
            Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}
