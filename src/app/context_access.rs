// SPDX-License-Identifier: MPL-2.0

//! Native ownership of workspace grants and one-shot terminal text review.

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
    pub requested_identity: Option<String>,
    pub surface: Option<Surface>,
    pub decision: Option<Decision>,
    pub width: u16,
    pub height: u16,
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
    pub page: usize,
    pub reviewed_through: Option<usize>,
    pub accept_selected: bool,
    pub attachment: u64,
    pub presented_page: Option<usize>,
    pub prepared: VecDeque<(u64, usize, bool)>,
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
        let lines = details
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
            lines,
            page: 0,
            reviewed_through: None,
            accept_selected: false,
            attachment,
            presented_page: None,
            prepared: VecDeque::new(),
        }
    }
    /// Rows per page. A grant also lists its scopes above the page, so its
    /// pages are shorter; both still fit the smallest approving editor area.
    fn page_rows(&self) -> usize {
        match self.kind {
            Kind::Grant { .. } => 8,
            Kind::Proposal { .. } => 12,
        }
    }
    pub(crate) fn last_page(&self) -> usize {
        self.lines.len().saturating_sub(1) / self.page_rows()
    }
    pub(crate) fn fully_reviewed(&self) -> bool {
        self.reviewed_through == Some(self.last_page())
    }
    pub fn proposal(&self) -> Option<&str> {
        match &self.kind {
            Kind::Proposal { id } => Some(id),
            _ => None,
        }
    }
}

impl App {
    pub(crate) fn note_context_frame(&mut self, width: u16, height: u16, frame: u64) {
        self.context_ui.width = width;
        self.context_ui.height = height;
        if let Some(surface) = self.context_ui.surface.as_mut() {
            surface
                .prepared
                .push_back((frame, surface.page, width >= 80 && height >= 22));
            while surface.prepared.len() > 8 {
                surface.prepared.pop_front();
            }
        }
    }
    pub(crate) fn note_context_presented(&mut self, frame: u64) {
        if let Some(surface) = self.context_ui.surface.as_mut()
            && surface
                .prepared
                .iter()
                .any(|&(id, page, enough)| id == frame && page == surface.page && enough)
            && (surface.page == 0
                || surface
                    .reviewed_through
                    .is_some_and(|last| surface.page <= last + 1))
        {
            surface.reviewed_through =
                Some(surface.reviewed_through.unwrap_or(0).max(surface.page));
            surface.presented_page = Some(surface.page);
        }
    }

    pub fn context_overlay_active(&self) -> bool {
        self.context_ui.surface.is_some()
    }

    pub(super) fn request_context_access(&mut self, identity: Option<String>) {
        if self.recording_macro.is_some() || self.macro_replay.is_some() {
            self.status("Context access requires native input outside a macro");
            return;
        }
        self.context_ui.requested_identity = Some(identity.unwrap_or_else(|| "agent".into()));
    }

    pub(super) fn handle_context_input(&mut self, input: InputEvent) {
        let Some(mut surface) = self.context_ui.surface.take() else {
            return;
        };
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
        if !key.modifiers.is_empty() {
            self.context_ui.surface = Some(surface);
            return;
        }
        let enough_room = self.context_ui.width >= 80 && self.context_ui.height >= 22;
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
            // The arrows move between the two choice rows, which sit one
            // above the other; paging stays on j/k.
            KeyCode::Up => surface.accept_selected = false,
            KeyCode::Down => surface.accept_selected = true,
            KeyCode::Char('j') => {
                if surface.presented_page == Some(surface.page) {
                    surface.page = (surface.page + 1).min(surface.last_page());
                    surface.presented_page = None;
                }
            }
            KeyCode::Char('k') => surface.page = surface.page.saturating_sub(1),
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
            KeyCode::Char(character @ '1'..='4') => {
                if let Kind::Grant { scopes, .. } = &mut surface.kind {
                    let scope = [
                        Scope::TerminalRead,
                        Scope::EditorContextRead,
                        Scope::BufferEdit,
                        Scope::TerminalPropose,
                    ][character as usize - '1' as usize];
                    if !scopes.remove(&scope) {
                        scopes.insert(scope);
                    }
                }
            }
            KeyCode::Enter => {
                let accepted = surface.accept_selected
                    && enough_room
                    && surface.fully_reviewed()
                    && surface.presented_page == Some(surface.page);
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
            for (index, scope) in [
                Scope::TerminalRead,
                Scope::EditorContextRead,
                Scope::BufferEdit,
                Scope::TerminalPropose,
            ]
            .iter()
            .enumerate()
            {
                rows.push(review_row(
                    format!(
                        "{} [{}] {}",
                        index + 1,
                        if scopes.contains(scope) { "x" } else { " " },
                        scope.capability()
                    ),
                    Vec::new(),
                    vec![0],
                ));
            }
            rows.push(review_row(
                format!(
                    "r [{}] Remember; x Revoke",
                    if *remember { "x" } else { " " }
                ),
                Vec::new(),
                vec![0, 16],
            ));
            rows.push(review_row(String::new(), Vec::new(), Vec::new()));
        }
        let page_rows = surface.page_rows();
        for line in surface
            .lines
            .iter()
            .skip(surface.page * page_rows)
            .take(page_rows)
        {
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
        let last_page = surface.last_page();
        if last_page > 0 {
            let text = format!(
                "Page {}/{} · j next page · k previous page",
                surface.page + 1,
                last_page + 1
            );
            let end = text.chars().count();
            rows.push(review_row(text, (0..end).collect(), Vec::new()));
        }
        rows.push(review_row(String::new(), Vec::new(), Vec::new()));
        let action = match surface.kind {
            Kind::Grant { .. } => "Grant access",
            Kind::Proposal { .. } => "Insert text (no Enter)",
        };
        rows.push(review_row("Reject".into(), Vec::new(), Vec::new()));
        rows.push(review_row(action.into(), Vec::new(), Vec::new()));
        let selected = rows.len() - if surface.accept_selected { 1 } else { 2 };
        // Say why Enter would not apply the selected action, instead of
        // leaving it to look like an unresponsive key.
        let message = if self.context_ui.width < 80 || self.context_ui.height < 22 {
            Some("Enlarge terminal to at least 80 × 24 to approve".to_owned())
        } else if surface.accept_selected && last_page > 0 && !surface.fully_reviewed() {
            let next = surface.reviewed_through.map_or(0, |last| last + 1);
            Some(format!(
                "Read page {}/{} first: press j until the last page has been shown.",
                (next + 1).min(last_page + 1),
                last_page + 1
            ))
        } else {
            None
        };
        let mut actions = vec![
            OverlayAction::new("↑/↓ or Tab", "choose"),
            OverlayAction::new("Enter", "apply choice"),
            OverlayAction::new("Esc", "reject"),
        ];
        if last_page > 0 {
            actions.insert(0, OverlayAction::new("j/k", "page"));
        }
        Some(OverlaySnapshot {
            kind: OverlayKind::Confirmation,
            purpose: OverlayPurpose::Confirmation,
            input: OverlayInput::None,
            layout: OverlayLayout::Standard,
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
        identity: OverlayIdentity::Text(label.clone()),
        label,
        detail: String::new(),
        trailing_detail: String::new(),
        available: true,
        dimmed: false,
        muted,
        emphasis,
        detail_emphasis: Vec::new(),
    }
}
