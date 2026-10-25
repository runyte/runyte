// SPDX-License-Identifier: MPL-2.0

//! Native ownership of workspace grants and one-shot terminal text review.

// Presentation values remain shared while the native context owner is Unix-only.
#![cfg_attr(not(unix), allow(dead_code))]

use super::App;
use crate::{
    input::{InputEvent, KeyCode},
    snapshot::*,
    workspace::context::wire::Scope,
};
use std::collections::{BTreeSet, VecDeque};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Default)]
pub(crate) struct ContextUi {
    pub identities: Vec<String>,
    pub requested_identity: Option<String>,
    /// `:agent-channel` waits for the workspace host to render the page.
    pub channel_page_requested: bool,
    pub surface: Option<Surface>,
    pub decision: Option<Decision>,
    /// The editor area. Approval needs at least 80 × 22 of it.
    pub width: u16,
    pub height: u16,
    /// Rows the overlay can actually use, once the session strip has taken
    /// its row. `None` means the whole editor area.
    pub overlay_height: Option<u16>,
}

impl ContextUi {
    fn view_height(&self) -> u16 {
        self.overlay_height.unwrap_or(self.height)
    }
}

/// A frame prepared while a review was open, and what it would show.
#[derive(Clone, Copy)]
pub(crate) struct PreparedFrame {
    id: u64,
    top: usize,
    shown: usize,
    /// Whether the editor area was large enough to approve.
    room: bool,
    /// The editor area and overlay height it was laid out for.
    area: (u16, u16, u16),
}

#[cfg(test)]
#[path = "tests/context_access.rs"]
mod tests;

pub(crate) enum Decision {
    Grant {
        identity: String,
        scopes: BTreeSet<Scope>,
        remember: bool,
    },
    Revoke(String),
    Proposal {
        id: String,
        accepted: bool,
    },
}

pub(crate) enum Kind {
    Grant {
        identity: String,
        scopes: BTreeSet<Scope>,
        remember: bool,
    },
    Proposal {
        id: String,
    },
}

pub(crate) struct Surface {
    pub kind: Kind,
    pub title: String,
    pub lines: Vec<ReviewLine>,
    /// First line in view.
    pub top: usize,
    /// Lines a presented frame has shown. Approval needs all of them, so a
    /// jump past unread lines is allowed but does not count as reading them.
    pub seen: Vec<bool>,
    pub accept_selected: bool,
    pub attachment: u64,
    /// The first line of the view the frontend last presented, while that is
    /// still the current view.
    pub presented_top: Option<usize>,
    /// Recently prepared frames, all laid out for the current area.
    pub prepared: VecDeque<PreparedFrame>,
    /// `g` was pressed and waits for `g` (first line) or `e` (last line).
    pub pending_g: bool,
}

/// How a reviewed line is drawn: proposed text stands out, labels and notes
/// recede, and everything else reads as an ordinary value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tone {
    Value,
    Literal,
    Note,
}

/// One labelled statement in a review, before it is wrapped into lines.
pub(crate) struct Detail {
    pub label: &'static str,
    pub value: String,
    pub tone: Tone,
}

impl Detail {
    pub fn value(label: &'static str, value: String) -> Self {
        Self {
            label,
            value,
            tone: Tone::Value,
        }
    }
    /// A value cut to one row, ending in an ellipsis when anything was cut.
    pub fn excerpt(label: &'static str, value: &str) -> Self {
        let mut rows = wrap(value, Tone::Literal, VALUE_WIDTH - 1).into_iter();
        let mut value = rows.next().unwrap_or_default();
        if rows.next().is_some() {
            value.push('…');
        }
        Self::value(label, value)
    }
    pub fn note(value: impl Into<String>) -> Self {
        Self {
            label: "",
            value: value.into(),
            tone: Tone::Note,
        }
    }
}

/// One drawn row of a review page. Only the first row of a wrapped detail
/// carries its label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewLine {
    pub label: &'static str,
    pub value: String,
    pub tone: Tone,
}

/// The grant toggles in key order: `1` is the first.
const SCOPES: [Scope; 6] = [
    Scope::TerminalRead,
    Scope::EditorContextRead,
    Scope::BufferEdit,
    Scope::TerminalPropose,
    Scope::TerminalInsert,
    Scope::AgentChannel,
];

/// The scope another one needs. Turning a scope on turns on what it needs,
/// and turning one off turns off what needs it, so a grant is always valid.
fn prerequisite(scope: Scope) -> Option<Scope> {
    match scope {
        Scope::BufferEdit => Some(Scope::EditorContextRead),
        Scope::TerminalPropose => Some(Scope::TerminalRead),
        Scope::TerminalInsert => Some(Scope::TerminalPropose),
        Scope::TerminalRead | Scope::EditorContextRead | Scope::AgentChannel => None,
    }
}

fn toggle(scopes: &mut BTreeSet<Scope>, scope: Scope) {
    if scopes.remove(&scope) {
        while let Some(orphan) = scopes
            .iter()
            .copied()
            .find(|s| prerequisite(*s).is_some_and(|needed| !scopes.contains(&needed)))
        {
            scopes.remove(&orphan);
        }
    } else {
        let mut next = Some(scope);
        while let Some(scope) = next {
            scopes.insert(scope);
            next = prerequisite(scope);
        }
    }
}

/// Width of the label column, which holds the longest label and a gap.
const LABEL_WIDTH: usize = 11;
/// Width a value wraps at. With the label column and the selection gutter it
/// fits the overlay inside the smallest editor area that may approve.
const VALUE_WIDTH: usize = 64;

/// Literal visible spelling of proposed text, where every character must be
/// unambiguous: spaces are dots, backslashes double and anything outside
/// printable ASCII is escaped.
pub(crate) fn visible(text: &str) -> String {
    let mut result = String::new();
    for character in text.chars() {
        if character == ' ' {
            result.push('·');
        } else if character == '\\' {
            result.push_str("\\\\");
        } else if character.is_ascii_graphic() {
            result.push(character);
        } else {
            result.extend(character.escape_unicode());
        }
    }
    result
}

/// Readable spelling of a label that describes the proposal rather than
/// being part of it, such as a terminal title or a path. Printable Unicode
/// and plain spaces stay as they are; anything that draws nothing or could
/// pass for a space is escaped, so nothing in a label can hide.
pub(crate) fn label(text: &str) -> String {
    let mut result = String::new();
    for character in text.chars() {
        if character == '\\' {
            result.push_str("\\\\");
        } else if character == ' '
            || (!character.is_whitespace()
                && !character.is_control()
                && character.width().is_some_and(|width| width > 0))
        {
            result.push(character);
        } else {
            result.extend(character.escape_unicode());
        }
    }
    result
}

/// Split a value into rows no wider than `width` cells. Prose breaks between
/// words; proposed text breaks anywhere, since every character in it is
/// already visible and none may be dropped at a break.
fn wrap(value: &str, tone: Tone, width: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    let mut used = 0;
    let words: Vec<&str> = if tone == Tone::Literal {
        vec![value]
    } else {
        value.split(' ').collect()
    };
    for (index, word) in words.into_iter().enumerate() {
        let word_width = word.width();
        if index > 0 {
            if used > 0 && used + 1 + word_width <= width {
                rows.last_mut().unwrap().push(' ');
                used += 1;
            } else if used > 0 {
                rows.push(String::new());
                used = 0;
            }
        }
        for character in word.chars() {
            let character_width = character.width().unwrap_or(0);
            if used + character_width > width && used > 0 {
                rows.push(String::new());
                used = 0;
            }
            rows.last_mut().unwrap().push(character);
            used += character_width;
        }
    }
    rows
}

impl Surface {
    pub fn new(
        kind: Kind,
        title: String,
        explanation: Vec<Detail>,
        text: &str,
        attachment: u64,
    ) -> Self {
        let mut details = Vec::new();
        if !text.is_empty() {
            details.push(Detail {
                label: "Text",
                value: visible(text),
                tone: Tone::Literal,
            });
            details.push(Detail::note(
                "Spaces show as ·; backslashes and non-ASCII are escaped.",
            ));
        }
        details.extend(explanation);
        let lines: Vec<ReviewLine> = details
            .into_iter()
            .flat_map(|detail| {
                wrap(&detail.value, detail.tone, VALUE_WIDTH)
                    .into_iter()
                    .enumerate()
                    .map(move |(index, value)| ReviewLine {
                        label: if index == 0 { detail.label } else { "" },
                        value,
                        tone: detail.tone,
                    })
            })
            .collect();
        Self {
            kind,
            title,
            seen: vec![false; lines.len()],
            lines,
            top: 0,
            accept_selected: false,
            attachment,
            presented_top: None,
            prepared: VecDeque::new(),
            pending_g: false,
        }
    }
    /// Lines in view for an overlay that may use `height` rows, the editor
    /// area less any session strip: everything but the border, the position
    /// row, the choices, and room for a two-row message. A grant also lists
    /// its scopes above the lines. At the smallest approving area a proposal
    /// shows 14 lines and a grant 6, one fewer of each beside the session
    /// strip, and a taller editor shows more, so a long prompt reads in fewer
    /// screens.
    pub(crate) fn view_rows(&self, height: u16) -> usize {
        let chrome = match self.kind {
            Kind::Grant { .. } => 16,
            Kind::Proposal { .. } => 8,
        };
        usize::from(height).saturating_sub(chrome).max(1)
    }
    fn last_top(&self, rows: usize) -> usize {
        self.lines.len().saturating_sub(rows)
    }
    fn scroll_to(&mut self, top: usize, rows: usize) {
        let top = top.min(self.last_top(rows));
        if top != self.top {
            // The frontend has not shown this view yet.
            self.presented_top = None;
        }
        self.top = top;
    }
    pub(crate) fn fully_reviewed(&self) -> bool {
        self.seen.iter().all(|seen| *seen)
    }
    pub fn proposal(&self) -> Option<&str> {
        match &self.kind {
            Kind::Proposal { id } => Some(id),
            _ => None,
        }
    }
}

impl App {
    /// A frame for an editor area without a session strip.
    #[cfg(test)]
    pub(crate) fn note_context_frame(&mut self, width: u16, height: u16, frame: u64) {
        self.note_context_frame_in(width, height, height, frame);
    }
    /// Records a frame about to be drawn in an editor area `width` × `height`
    /// whose overlay may use `overlay_height` rows.
    pub(crate) fn note_context_frame_in(
        &mut self,
        width: u16,
        height: u16,
        overlay_height: u16,
        frame: u64,
    ) {
        self.context_ui.width = width;
        self.context_ui.height = height;
        self.context_ui.overlay_height = Some(overlay_height);
        if let Some(surface) = self.context_ui.surface.as_mut() {
            let area = (width, height, overlay_height);
            // A frame laid out for an area the window no longer has may be
            // painted clipped, so once the area changes only frames laid out
            // for the new one count.
            if surface
                .prepared
                .back()
                .is_some_and(|last| last.area != area)
            {
                surface.prepared.clear();
            }
            let rows = surface.view_rows(overlay_height);
            surface.scroll_to(surface.top, rows);
            let shown = rows.min(surface.lines.len() - surface.top);
            surface.prepared.push_back(PreparedFrame {
                id: frame,
                top: surface.top,
                shown,
                room: width >= 80 && height >= 22,
                area,
            });
            while surface.prepared.len() > 8 {
                surface.prepared.pop_front();
            }
        }
    }
    pub(crate) fn note_context_presented(&mut self, frame: u64) {
        let Some(surface) = self.context_ui.surface.as_mut() else {
            return;
        };
        if let Some(prepared) = surface
            .prepared
            .iter()
            .find(|prepared| prepared.id == frame && prepared.top == surface.top && prepared.room)
            .copied()
        {
            surface.seen[prepared.top..prepared.top + prepared.shown].fill(true);
            surface.presented_top = Some(prepared.top);
        }
    }

    pub fn context_overlay_active(&self) -> bool {
        self.context_ui.surface.is_some()
    }

    pub(super) fn request_agent_channel(&mut self) {
        self.context_ui.channel_page_requested = true;
    }

    /// Shows the channel page the workspace host rendered.
    pub(crate) fn open_agent_channel_page(&mut self, page: &str) {
        self.open_virtual_page(
            crate::buffer::GeneratedViewIdentity::AgentChannel,
            crate::buffer::AGENT_CHANNEL_NAME.to_owned(),
            page,
            super::ContentAlignment::default(),
        );
    }

    fn agent_channel_page(&self) -> Option<usize> {
        self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.closed_buffers.contains(&index)
                && buffer.generated_view_identity()
                    == Some(&crate::buffer::GeneratedViewIdentity::AgentChannel))
            .then_some(index)
        })
    }

    pub(crate) fn agent_channel_page_open(&self) -> bool {
        self.agent_channel_page().is_some()
    }

    /// Replaces the page's text if it is open; nothing opens otherwise.
    pub(crate) fn refresh_agent_channel_page(&mut self, page: &str) {
        if let Some(index) = self.agent_channel_page() {
            self.buffers[index].replace_virtual_text(page);
            self.normalize_buffer(index);
            self.plugins.presentation_dirty = true;
        }
    }

    pub(super) fn request_context_access(&mut self, identity: Option<String>) {
        if self.recording_macro.is_some() || self.macro_replay.is_some() {
            self.status("Context access requires native input outside a macro");
            return;
        }
        self.context_ui.requested_identity = Some(identity.unwrap_or_default());
    }

    pub(super) fn handle_context_input(&mut self, input: InputEvent, approval: bool) {
        let Some(mut surface) = self.context_ui.surface.take() else {
            return;
        };
        // Whatever arrives next, a pending `g` is spent on it.
        let pending_g = std::mem::take(&mut surface.pending_g);
        if self.recording_macro.is_some()
            || self.macro_replay.is_some()
            || surface.attachment != self.plugins.attachment_generation
        {
            return;
        }
        let InputEvent::Key(key) = input else {
            self.context_ui.surface = Some(surface);
            return;
        };
        // Shift is part of the character (`G`), not a modifier to refuse.
        let key = key.canonical_for_binding();
        if !key.modifiers.is_empty() {
            self.context_ui.surface = Some(surface);
            return;
        }
        let enough_room = self.context_ui.width >= 80 && self.context_ui.height >= 22;
        let rows = surface.view_rows(self.context_ui.view_height());
        let scrollable = surface.lines.len() > rows;
        if pending_g {
            // `gg` and `ge` move; anything else cancels, as in a buffer.
            match key.code {
                KeyCode::Char('g') => surface.scroll_to(0, rows),
                KeyCode::Char('e') => surface.scroll_to(usize::MAX, rows),
                _ => {}
            }
            self.context_ui.surface = Some(surface);
            return;
        }
        match key.code {
            KeyCode::Escape => {
                if let Kind::Proposal { id } = surface.kind {
                    self.context_ui.decision = Some(Decision::Proposal {
                        id,
                        accepted: false,
                    });
                }
                return;
            }
            KeyCode::Tab => surface.accept_selected = !surface.accept_selected,
            // With nothing to scroll the arrows move between the two choice
            // rows, which sit one above the other. Once the text is longer
            // than the view they scroll it like a buffer, and Tab chooses.
            KeyCode::Up if !scrollable => surface.accept_selected = false,
            KeyCode::Down if !scrollable => surface.accept_selected = true,
            KeyCode::Up | KeyCode::Char('k') => {
                surface.scroll_to(surface.top.saturating_sub(1), rows);
            }
            KeyCode::Down | KeyCode::Char('j') => surface.scroll_to(surface.top + 1, rows),
            KeyCode::PageUp => surface.scroll_to(surface.top.saturating_sub(rows), rows),
            KeyCode::PageDown => surface.scroll_to(surface.top + rows, rows),
            KeyCode::Home => surface.scroll_to(0, rows),
            KeyCode::End | KeyCode::Char('G') => surface.scroll_to(usize::MAX, rows),
            KeyCode::Char('g') => surface.pending_g = true,
            KeyCode::Char('x') => {
                if let Kind::Grant { identity, .. } = surface.kind {
                    self.context_ui.decision = Some(Decision::Revoke(identity));
                    return;
                }
            }
            KeyCode::Char('r') => {
                if let Kind::Grant { remember, .. } = &mut surface.kind {
                    *remember = !*remember;
                }
            }
            KeyCode::Char('n') => {
                if let Kind::Grant { identity, .. } = &surface.kind
                    && let Some(index) = self
                        .context_ui
                        .identities
                        .iter()
                        .position(|id| id == identity)
                {
                    self.context_ui.requested_identity = Some(
                        self.context_ui.identities[(index + 1) % self.context_ui.identities.len()]
                            .clone(),
                    );
                    return;
                }
            }
            KeyCode::Char(character @ '1'..='6') => {
                if let Kind::Grant { scopes, .. } = &mut surface.kind {
                    toggle(scopes, SCOPES[character as usize - '1' as usize]);
                }
            }
            KeyCode::Enter if approval => {
                let accepted = surface.accept_selected
                    && enough_room
                    && surface.fully_reviewed()
                    && surface.presented_top == Some(surface.top);
                if surface.accept_selected && !accepted {
                    self.context_ui.surface = Some(surface);
                    return;
                }
                self.context_ui.decision = match surface.kind {
                    Kind::Grant {
                        identity,
                        scopes,
                        remember,
                    } if accepted => Some(Decision::Grant {
                        identity,
                        scopes,
                        remember,
                    }),
                    Kind::Proposal { id } => Some(Decision::Proposal { id, accepted }),
                    _ => None,
                };
                return;
            }
            _ => {}
        }
        self.context_ui.surface = Some(surface);
    }

    pub(crate) fn context_overlay(&self) -> Option<OverlaySnapshot> {
        let surface = self.context_ui.surface.as_ref()?;
        let mut rows = Vec::new();
        if let Kind::Grant {
            scopes, remember, ..
        } = &surface.kind
        {
            for (index, scope) in SCOPES.iter().enumerate() {
                rows.push(review_row(
                    format!(
                        "{} [{}] {}",
                        index + 1,
                        if scopes.contains(scope) { "x" } else { " " },
                        match scope {
                            Scope::TerminalRead => "Read terminal output",
                            Scope::EditorContextRead => "Read buffers and selections",
                            Scope::BufferEdit => "Edit and append to buffers",
                            Scope::TerminalPropose => "Propose terminal text for review",
                            Scope::TerminalInsert =>
                                "Skip review: insert proposals directly (no Enter)",
                            Scope::AgentChannel => "Join the agent channel: read and post messages",
                        }
                    ),
                    Vec::new(),
                    vec![0],
                ));
            }
            rows.push(review_row(
                format!(
                    "r [{}] Remember for this workspace; x Revoke; n Next identity",
                    if *remember { "x" } else { " " }
                ),
                Vec::new(),
                vec![0, 35, 45],
            ));
            rows.push(review_row(String::new(), Vec::new(), Vec::new()));
        }
        let view_rows = surface.view_rows(self.context_ui.view_height());
        let top = surface.top.min(surface.last_top(view_rows));
        for line in surface.lines.iter().skip(top).take(view_rows) {
            let label = format!("{:<LABEL_WIDTH$}", line.label);
            let label_end = label.chars().count();
            let end = label_end + line.value.chars().count();
            let (muted, emphasis) = match line.tone {
                Tone::Value => ((0..label_end).collect(), Vec::new()),
                Tone::Literal => ((0..label_end).collect(), (label_end..end).collect()),
                Tone::Note => ((0..end).collect(), Vec::new()),
            };
            rows.push(review_row(
                format!("{label}{}", line.value),
                muted,
                emphasis,
            ));
        }
        let total = surface.lines.len();
        let scrollable = total > view_rows;
        if scrollable {
            let text = format!(
                "Lines {}–{} of {total} · j/k line · PgUp/PgDn page · gg/G first/last",
                top + 1,
                (top + view_rows).min(total),
            );
            let end = text.chars().count();
            rows.push(review_row(text, (0..end).collect(), Vec::new()));
        }
        rows.push(review_row(String::new(), Vec::new(), Vec::new()));
        let action = match surface.kind {
            Kind::Grant { .. } => "Apply permissions",
            Kind::Proposal { .. } => "Insert text (no Enter)",
        };
        rows.push(review_row("Reject".into(), Vec::new(), Vec::new()));
        rows.push(review_row(action.into(), Vec::new(), Vec::new()));
        let selected = rows.len() - if surface.accept_selected { 1 } else { 2 };
        // Say why Enter would not apply the selected action, instead of
        // leaving it to look like an unresponsive key.
        let message = if self.context_ui.width < 80 || self.context_ui.height < 22 {
            Some("Enlarge terminal to at least 80 × 24 to approve".to_owned())
        } else if surface.accept_selected
            && scrollable
            && let Some(unread) = surface.seen.iter().position(|seen| !seen)
        {
            Some(format!(
                "Read the rest first: line {} of {total} has not been shown.",
                unread + 1
            ))
        } else {
            None
        };
        let actions = vec![
            if scrollable {
                OverlayAction::new("Tab", "choose")
            } else {
                OverlayAction::new("↑/↓/Tab", "choose")
            },
            OverlayAction::new("Enter", "apply choice"),
            OverlayAction::new("Esc", "reject"),
        ];
        Some(OverlaySnapshot {
            kind: OverlayKind::Confirmation,
            purpose: OverlayPurpose::Confirmation,
            input: OverlayInput::None,
            layout: OverlayLayout::Standard,
            legend: Vec::new(),
            actions,
            title: surface.title.clone(),
            query: String::new(),
            query_placeholder: String::new(),
            column_header: None,
            rows,
            selected: Some(selected),
            scroll_anchor: None,
            row_offset: 0,
            message,
            omitted_rows: 0,
            total_rows: 0,
            query_cursor: None,
            show_preview: false,
            preview_title: None,
            preview: None,
        })
    }
}

/// A non-interactive review row; only the two choice rows are ever selected,
/// and only keys choose between them.
fn review_row(label: String, muted: Vec<usize>, emphasis: Vec<usize>) -> OverlayRow {
    OverlayRow {
        heading: false,
        identity: OverlayIdentity::Text(label.clone()),
        label,
        detail: String::new(),
        trailing_detail: String::new(),
        available: true,
        dimmed: false,
        muted,
        emphasis,
        detail_emphasis: Vec::new(),
        tints: Vec::new(),
        elide_from: None,
    }
}
