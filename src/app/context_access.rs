// SPDX-License-Identifier: MPL-2.0

//! Native ownership of workspace grants and one-shot terminal text review.

// Presentation values remain shared while the native context owner is Unix-only.
#![cfg_attr(not(unix), allow(dead_code))]

use super::App;
use crate::{
    command::{EditorCommand as Command, Mode},
    input::{InputEvent, KeyCode, Modifiers, PointerEventKind},
    keymap::{Binding, BindingTarget, KeySequence, Lookup},
    layout::Rect,
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

    fn review_area(&self) -> Rect {
        crate::context_review_layout::area(Rect {
            x: 0,
            y: self.height.saturating_sub(self.view_height()),
            width: self.width,
            height: self.view_height(),
        })
    }

    fn surface_height(&self, surface: &Surface) -> u16 {
        if surface.proposal().is_some() {
            self.review_area().height
        } else {
            self.view_height()
        }
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
    /// Pending registry motion prefix, normally `g` for `gg` / `ge`.
    pub pending_keys: KeySequence,
    details: Vec<Detail>,
    wrap_width: usize,
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
            .iter()
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
        let mut surface = Self {
            kind,
            title,
            seen: vec![false; lines.len()],
            lines,
            top: 0,
            accept_selected: false,
            attachment,
            presented_top: None,
            prepared: VecDeque::new(),
            pending_keys: KeySequence::default(),
            details,
            wrap_width: VALUE_WIDTH,
        };
        surface.reflow(76);
        surface
    }
    fn reflow(&mut self, width: usize) {
        if self.proposal().is_none() || self.wrap_width == width {
            return;
        }
        self.wrap_width = width;
        self.lines = self
            .details
            .iter()
            .flat_map(|detail| {
                let value = if detail.label.is_empty() || detail.label == "Text" {
                    detail.value.clone()
                } else {
                    format!("{}: {}", detail.label, detail.value)
                };
                wrap(&value, detail.tone, width)
                    .into_iter()
                    .map(move |value| ReviewLine {
                        label: "",
                        value,
                        tone: detail.tone,
                    })
            })
            .collect();
        // Rewrapping changes which characters each row contains. Only newly
        // presented frames may acknowledge those rows.
        self.seen = vec![false; self.lines.len()];
        self.top = 0;
        self.presented_top = None;
        self.prepared.clear();
    }
    /// Body rows after the border, position, choices and two-row message.
    /// Proposals also pin a two-row destination summary; grants list scopes.
    /// `height` is the actual dialog height, not the surrounding editor.
    pub(crate) fn view_rows(&self, height: u16) -> usize {
        let chrome = match self.kind {
            Kind::Grant { .. } => 16,
            Kind::Proposal { .. } => 10,
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
        let review_area = self.context_ui.review_area();
        if let Some(surface) = self.context_ui.surface.as_mut() {
            surface.reflow(usize::from(review_area.width.saturating_sub(4)).max(1));
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
                surface.presented_top = None;
            }
            let rows = surface.view_rows(if surface.proposal().is_some() {
                review_area.height
            } else {
                overlay_height
            });
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
    /// Every pane showing it moves to the last line, so the newest message
    /// is in view.
    pub(crate) fn refresh_agent_channel_page(&mut self, page: &str) {
        if let Some(index) = self.agent_channel_page() {
            let buffer = &mut self.buffers[index];
            buffer.replace_virtual_text(page);
            // The page ends in a newline; the row after it holds nothing.
            let mut row = buffer.last_row();
            if row > 0 && buffer.line_len(row) == 0 {
                row -= 1;
            }
            let last = buffer.line_to_offset(row);
            for pane in self.panes.values_mut().filter(|pane| pane.buffer == index) {
                pane.replace_selection(crate::selection::Selection::point(last));
            }
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
        // Non-key input spends a pending motion prefix.
        let mut pending = std::mem::take(&mut surface.pending_keys);
        let continuing = !pending.is_empty();
        if self.recording_macro.is_some()
            || self.macro_replay.is_some()
            || surface.attachment != self.plugins.attachment_generation
        {
            return;
        }
        let rows = surface.view_rows(self.context_ui.surface_height(&surface));
        if let InputEvent::Pointer(pointer) = input {
            let area = self.context_ui.review_area();
            let in_body = pointer.column > area.x
                && pointer.column < area.x.saturating_add(area.width).saturating_sub(1)
                && pointer.row >= area.y.saturating_add(3)
                && usize::from(pointer.row.saturating_sub(area.y + 3)) < rows;
            if surface.proposal().is_some() && in_body {
                match pointer.kind {
                    PointerEventKind::ScrollUp => {
                        surface.scroll_to(surface.top.saturating_sub(3), rows)
                    }
                    PointerEventKind::ScrollDown => {
                        surface.scroll_to(surface.top.saturating_add(3), rows)
                    }
                    _ => {}
                }
            }
            self.context_ui.surface = Some(surface);
            return;
        }
        let InputEvent::Key(key) = input else {
            self.context_ui.surface = Some(surface);
            return;
        };
        let key = key.canonical_for_binding();
        let enough_room = self.context_ui.width >= 80 && self.context_ui.height >= 22;
        // Choice keys retain their meaning even when the text overflows.
        // Only approved motion commands from the live registry can scroll;
        // configured action chains and arbitrary editor commands cannot run.
        let choice = matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Enter
                | KeyCode::Escape
        );
        if !choice {
            pending.push(key);
            let binding = match self.keymap().lookup(Mode::Normal, &pending) {
                Lookup::Exact(binding) => Some(binding),
                Lookup::ExactAndPrefix { exact, .. } => Some(exact),
                Lookup::Prefix(bindings) => {
                    if bindings
                        .iter()
                        .any(|binding| review_motion(binding).is_some())
                    {
                        surface.pending_keys = pending;
                    }
                    None
                }
                Lookup::NoMatch => None,
            };
            if let Some(command) = binding.and_then(review_motion) {
                let top = match command {
                    Command::PageUp => surface.top.saturating_sub(rows),
                    Command::PageDown => surface.top.saturating_add(rows),
                    Command::HalfPageUp => surface.top.saturating_sub((rows / 2).max(1)),
                    Command::HalfPageDown => surface.top.saturating_add((rows / 2).max(1)),
                    Command::MoveFileStart => 0,
                    Command::MoveFileEnd => usize::MAX,
                    _ => unreachable!(),
                };
                surface.scroll_to(top, rows);
                self.context_ui.surface = Some(surface);
                return;
            }
            if continuing || !surface.pending_keys.is_empty() {
                self.context_ui.surface = Some(surface);
                return;
            }
        }
        if key.code == KeyCode::BackTab
            || (key.code == KeyCode::Tab && key.modifiers == Modifiers::SHIFT)
        {
            surface.accept_selected = !surface.accept_selected;
            self.context_ui.surface = Some(surface);
            return;
        }
        if !key.modifiers.is_empty() {
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
            KeyCode::Up => surface.accept_selected = false,
            KeyCode::Down => surface.accept_selected = true,
            KeyCode::Home => surface.scroll_to(0, rows),
            KeyCode::End => surface.scroll_to(usize::MAX, rows),
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
        if surface.proposal().is_some() {
            let target = surface
                .details
                .iter()
                .find(|detail| detail.label == "Terminal")
                .map(|detail| detail.value.as_str())
                .unwrap_or("Terminal");
            rows.push(review_row(format!("To: {target}"), Vec::new(), Vec::new()));
            rows.push(review_row(
                "Inserts text without pressing Enter.".into(),
                Vec::new(),
                Vec::new(),
            ));
        }
        let view_rows = surface.view_rows(self.context_ui.surface_height(surface));
        let top = surface.top.min(surface.last_top(view_rows));
        for line in surface.lines.iter().skip(top).take(view_rows) {
            let label = if surface.proposal().is_some() {
                String::new()
            } else {
                format!("{:<LABEL_WIDTH$}", line.label)
            };
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
        if surface.proposal().is_some() {
            for _ in surface.lines.len().saturating_sub(top).min(view_rows)..view_rows {
                rows.push(review_row(String::new(), Vec::new(), Vec::new()));
            }
        }
        let total = surface.lines.len();
        let scrollable = total > view_rows;
        if scrollable {
            let text = format!(
                "Lines {}–{} of {total} · {}",
                top + 1,
                (top + view_rows).min(total),
                self.key_text("{binding:Ctrl-u}/{binding:Ctrl-d} scroll · {binding:g g}/{binding:g e} first/last"),
            );
            let end = text.chars().count();
            rows.push(review_row(text, (0..end).collect(), Vec::new()));
        }
        if !scrollable && surface.proposal().is_some() {
            rows.push(review_row(String::new(), Vec::new(), Vec::new()));
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
            OverlayAction::new("↑/↓/Tab", "choose"),
            OverlayAction::new("Enter", "apply choice"),
            OverlayAction::new("Esc", "reject"),
        ];
        Some(OverlaySnapshot {
            kind: OverlayKind::Confirmation,
            purpose: OverlayPurpose::Confirmation,
            input: OverlayInput::None,
            layout: if surface.proposal().is_some() {
                OverlayLayout::ContextReview
            } else {
                OverlayLayout::Standard
            },
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

/// Navigation is shared with the editor registry without granting editor action authority.
fn review_motion(binding: &Binding) -> Option<Command> {
    if !binding.actions.is_empty() {
        return None;
    }
    match binding.target {
        BindingTarget::Editor(
            command @ (Command::PageUp
            | Command::PageDown
            | Command::HalfPageUp
            | Command::HalfPageDown
            | Command::MoveFileStart
            | Command::MoveFileEnd),
        ) => Some(command),
        _ => None,
    }
}
