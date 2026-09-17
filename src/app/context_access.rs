// SPDX-License-Identifier: MPL-2.0

//! Native ownership of workspace grants and one-shot terminal text review.

use super::App;
use crate::{
    input::{InputEvent, KeyCode},
    snapshot::*,
    workspace::context::wire::Scope,
};
use std::collections::{BTreeSet, VecDeque};

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
    pub lines: Vec<String>,
    pub page: usize,
    pub reviewed_through: Option<usize>,
    pub accept_selected: bool,
    pub attachment: u64,
    pub presented_page: Option<usize>,
    pub prepared: VecDeque<(u64, usize, bool)>,
}

/// Literal visible spelling shared by the approval display and source labels.
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

impl Surface {
    pub fn new(
        kind: Kind,
        title: String,
        explanation: Vec<String>,
        text: &str,
        attachment: u64,
    ) -> Self {
        let mut escaped = explanation.join("\n");
        if !text.is_empty() {
            escaped.push_str("\nLiteral text (spaces = ·; Unicode = escaped):\n");
            escaped.push_str(&visible(text));
        }
        let lines = escaped
            .split('\n')
            .flat_map(|line| {
                let chars = line.chars().collect::<Vec<_>>();
                if chars.is_empty() {
                    vec![String::new()]
                } else {
                    chars
                        .chunks(40)
                        .map(|chunk| chunk.iter().collect::<String>())
                        .collect()
                }
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
    fn last_page(&self) -> usize {
        self.lines.len().saturating_sub(1) / 6
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
            KeyCode::Down | KeyCode::Char('j') => {
                if surface.presented_page == Some(surface.page) {
                    surface.page = (surface.page + 1).min(surface.last_page());
                    surface.presented_page = None;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => surface.page = surface.page.saturating_sub(1),
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
                    && surface.reviewed_through == Some(surface.last_page())
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
        let mut lines = Vec::new();
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
                lines.push(format!(
                    "{} [{}] {}",
                    index + 1,
                    if scopes.contains(scope) { "x" } else { " " },
                    scope.capability()
                ));
            }
            lines.push(format!(
                "r [{}] Remember; x Revoke",
                if *remember { "x" } else { " " }
            ));
        }
        lines.extend(surface.lines.iter().skip(surface.page * 6).take(6).cloned());
        if !surface.lines.is_empty() {
            lines.push(format!(
                "Page {}/{}; j/k review",
                surface.page + 1,
                surface.last_page() + 1
            ));
        }
        let action = match surface.kind {
            Kind::Grant { .. } => "Grant access",
            Kind::Proposal { .. } => "Insert text (no Enter)",
        };
        lines.push(format!(
            "{} Reject    {} {action}",
            if surface.accept_selected { " " } else { ">" },
            if surface.accept_selected { ">" } else { " " }
        ));
        if self.context_ui.width < 80 || self.context_ui.height < 22 {
            lines.push("Enlarge terminal to at least 80 × 24 to approve".into());
        }
        Some(OverlaySnapshot {
            kind: OverlayKind::Confirmation,
            purpose: OverlayPurpose::Confirmation,
            input: OverlayInput::None,
            layout: OverlayLayout::Standard,
            actions: vec![
                OverlayAction::new("Tab", "choose"),
                OverlayAction::new("Enter", "apply choice"),
                OverlayAction::new("Esc", "reject"),
            ],
            title: surface.title.clone(),
            query: String::new(),
            query_placeholder: String::new(),
            column_header: None,
            rows: Vec::new(),
            selected: None,
            scroll_anchor: None,
            row_offset: 0,
            message: Some(lines.join("\n")),
            omitted_rows: 0,
            total_rows: 0,
            query_cursor: None,
            show_preview: false,
            preview_title: None,
            preview: None,
        })
    }
}
