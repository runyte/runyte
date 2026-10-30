// SPDX-License-Identifier: MPL-2.0

//! Native ownership of workspace grants.

// Presentation values remain shared while the native context owner is Unix-only.
#![cfg_attr(not(unix), allow(dead_code))]

use super::App;
use crate::{
    command::{EditorCommand as Command, Mode},
    input::{InputEvent, KeyCode, KeyStroke},
    keymap::{Binding, BindingTarget, KeySequence, Lookup},
    snapshot::*,
    workspace::context::wire::Permission,
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
        permissions: BTreeSet<Permission>,
        remember: bool,
    },
    Revoke(String),
}

/// The grant being chosen, before it is applied.
pub(crate) struct Grant {
    pub identity: String,
    pub permissions: BTreeSet<Permission>,
    pub remember: bool,
}

pub(crate) struct Surface {
    pub grant: Grant,
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
}

/// How a reviewed line is drawn: labels and notes recede, and everything else
/// reads as an ordinary value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tone {
    Value,
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
        let mut rows = wrap(value, false, VALUE_WIDTH - 1).into_iter();
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
const PERMISSIONS: [Permission; 2] = [Permission::Read, Permission::Write];

/// Turning Write on turns Read on, and turning Read off turns Write off, so
/// a grant is always valid.
fn toggle(permissions: &mut BTreeSet<Permission>, permission: Permission) {
    if permissions.remove(&permission) {
        if permission == Permission::Read {
            permissions.remove(&Permission::Write);
        }
    } else {
        permissions.insert(permission);
        permissions.insert(Permission::Read);
    }
}

/// Width of the label column, which holds the longest label and a gap.
const LABEL_WIDTH: usize = 11;
/// Width a value wraps at. With the label column and the selection gutter it
/// fits the overlay inside the smallest editor area that may approve.
const VALUE_WIDTH: usize = 64;

/// Literal visible spelling of text an agent supplied, where every character
/// must be unambiguous: spaces are dots, backslashes double and anything
/// outside printable ASCII is escaped.
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

/// Readable spelling of a label such as a terminal title or a path. Printable Unicode
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
/// words; `anywhere` breaks a value that has no word boundaries to keep.
fn wrap(value: &str, anywhere: bool, width: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    let mut used = 0;
    let words: Vec<&str> = if anywhere {
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
    pub fn new(grant: Grant, title: String, details: Vec<Detail>, attachment: u64) -> Self {
        let lines: Vec<ReviewLine> = details
            .iter()
            .flat_map(|detail| {
                wrap(&detail.value, false, VALUE_WIDTH)
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
            grant,
            title,
            seen: vec![false; lines.len()],
            lines,
            top: 0,
            accept_selected: false,
            attachment,
            presented_top: None,
            prepared: VecDeque::new(),
            pending_keys: KeySequence::default(),
        }
    }
    /// Body rows after the border, position, choices, two-row message and
    /// the permission toggles. `height` is the actual dialog height, not the
    /// surrounding editor.
    pub(crate) fn view_rows(&self, height: u16) -> usize {
        usize::from(height).saturating_sub(11).max(1)
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
                surface.presented_top = None;
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
        let rows = surface.view_rows(self.context_ui.view_height());
        let InputEvent::Key(key) = input else {
            self.context_ui.surface = Some(surface);
            return;
        };
        let key = key.canonical_for_binding();
        let enough_room = self.context_ui.width >= 80 && self.context_ui.height >= 22;
        use crate::keymap::ui::{Action, ContextReviewRoute};
        let route = self.keymap.ui().context_review_route(key);
        let action = match route {
            ContextReviewRoute::Owned(action) | ContextReviewRoute::ReviewThenFallback(action) => {
                action
            }
        };
        if matches!(route, ContextReviewRoute::ReviewThenFallback(_)) {
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
                let action = review_action(command);
                // Replaced inherited motions still own their old keys. They
                // cannot fall through into structural revoke or option input.
                if !self
                    .keymap
                    .ui()
                    .explicitly_configured(crate::keymap::ui::Owner::ContextReview, action)
                {
                    surface.scroll_to(review_top(action, surface.top, rows), rows);
                }
                self.context_ui.surface = Some(surface);
                return;
            }
            if continuing || !surface.pending_keys.is_empty() {
                self.context_ui.surface = Some(surface);
                return;
            }
        }
        match (action, key.code) {
            (Some(Action::Cancel), _) => return,
            (Some(Action::ToggleSelection), _) => {
                surface.accept_selected = !surface.accept_selected
            }
            (Some(Action::Previous), _) => surface.accept_selected = false,
            (Some(Action::Next), _) => surface.accept_selected = true,
            (
                Some(
                    action @ (Action::First
                    | Action::Last
                    | Action::PageUp
                    | Action::PageDown
                    | Action::HalfPageUp
                    | Action::HalfPageDown),
                ),
                _,
            ) => surface.scroll_to(review_top(action, surface.top, rows), rows),
            (None, KeyCode::Char('x')) if key.modifiers.is_empty() => {
                self.context_ui.decision = Some(Decision::Revoke(surface.grant.identity));
                return;
            }
            (None, KeyCode::Char('r')) if key.modifiers.is_empty() => {
                surface.grant.remember = !surface.grant.remember;
            }
            (None, KeyCode::Char('n')) if key.modifiers.is_empty() => {
                if let Some(index) = self
                    .context_ui
                    .identities
                    .iter()
                    .position(|id| *id == surface.grant.identity)
                {
                    self.context_ui.requested_identity = Some(
                        self.context_ui.identities[(index + 1) % self.context_ui.identities.len()]
                            .clone(),
                    );
                    return;
                }
            }
            (None, KeyCode::Char(character @ '1'..='2')) if key.modifiers.is_empty() => {
                toggle(
                    &mut surface.grant.permissions,
                    PERMISSIONS[character as usize - '1' as usize],
                );
            }
            (Some(Action::Accept), _) if approval => {
                let accepted = surface.accept_selected
                    && enough_room
                    && surface.fully_reviewed()
                    && surface.presented_top == Some(surface.top);
                if surface.accept_selected && !accepted {
                    self.context_ui.surface = Some(surface);
                    return;
                }
                if accepted {
                    let Grant {
                        identity,
                        permissions,
                        remember,
                    } = surface.grant;
                    self.context_ui.decision = Some(Decision::Grant {
                        identity,
                        permissions,
                        remember,
                    });
                }
                return;
            }
            _ => {}
        }
        self.context_ui.surface = Some(surface);
    }

    pub(super) fn context_accept_key(&self, key: KeyStroke) -> bool {
        self.context_overlay_active()
            && self.keymap.ui().context_review_route(key)
                == crate::keymap::ui::ContextReviewRoute::Owned(Some(
                    crate::keymap::ui::Action::Accept,
                ))
    }

    fn context_structural_key(&self, key: KeyStroke) -> bool {
        use crate::keymap::ui::ContextReviewRoute;
        let key = key.canonical_for_binding();
        if !key.modifiers.is_empty()
            || !matches!(key.code, KeyCode::Char('x' | 'r' | 'n' | '1' | '2'))
            || self
                .context_ui
                .surface
                .as_ref()
                .is_none_or(|s| !s.pending_keys.is_empty())
            || !matches!(
                self.keymap.ui().context_review_route(key),
                ContextReviewRoute::ReviewThenFallback(None)
            )
        {
            return false;
        }
        match self.keymap.lookup(Mode::Normal, &KeySequence::from(key)) {
            Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                review_motion(binding).is_none()
            }
            Lookup::Prefix(bindings) => !bindings
                .iter()
                .any(|binding| review_motion(binding).is_some()),
            Lookup::NoMatch => true,
        }
    }

    pub(super) fn context_input_requires_presented_approval(&self, key: KeyStroke) -> bool {
        self.context_accept_key(key)
            || (key.canonical_for_binding().code == KeyCode::Char('x')
                && self.context_structural_key(key))
    }

    fn context_review_motion_keys(&self, command: Command) -> String {
        use crate::keymap::ui::{Action, ContextReviewRoute, Owner};
        let fallback = match command {
            Command::MoveFileStart => Some(Action::First),
            Command::MoveFileEnd => Some(Action::Last),
            _ => None,
        };
        let action = review_action(command);
        if self
            .keymap
            .ui()
            .explicitly_configured(Owner::ContextReview, action)
        {
            return self
                .keymap
                .ui()
                .compact_label(Owner::ContextReview, action, 1);
        }
        for binding in self.keymap.bindings_for_mode(Mode::Normal) {
            if review_motion(binding) != Some(command) {
                continue;
            }
            let keys = binding.sequence.as_slice();
            let mut reachable = false;
            for (index, key) in keys.iter().enumerate() {
                if matches!(
                    self.keymap.ui().context_review_route(*key),
                    ContextReviewRoute::Owned(_)
                ) {
                    break;
                }
                match self.keymap.lookup(
                    Mode::Normal,
                    &KeySequence::new(keys[..=index].iter().copied()),
                ) {
                    Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                        reachable =
                            index + 1 == keys.len() && review_motion(binding) == Some(command);
                        break;
                    }
                    Lookup::Prefix(bindings)
                        if bindings.iter().any(|b| review_motion(b).is_some()) => {}
                    _ => break,
                }
            }
            if reachable {
                return binding.sequence.to_string();
            }
        }
        if let Some(action) = fallback {
            // The plain Home/End fallbacks run only after the narrow motion
            // registry has declined the key, exactly as in dispatch.
            let code = if action == Action::First {
                KeyCode::Home
            } else {
                KeyCode::End
            };
            let key = KeyStroke::plain(code);
            if self.keymap.ui().context_review_route(key)
                == ContextReviewRoute::ReviewThenFallback(Some(action))
            {
                let idle = match self.keymap.lookup(Mode::Normal, &KeySequence::from(key)) {
                    Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                        review_motion(binding).is_none()
                    }
                    Lookup::Prefix(bindings) => {
                        !bindings.iter().any(|b| review_motion(b).is_some())
                    }
                    Lookup::NoMatch => true,
                };
                if idle {
                    return key.label();
                }
            }
        }
        "unbound".into()
    }

    pub(crate) fn context_overlay(&self) -> Option<OverlaySnapshot> {
        let surface = self.context_ui.surface.as_ref()?;
        let mut rows = Vec::new();
        let Grant {
            permissions,
            remember,
            ..
        } = &surface.grant;
        let structural_label = |character: char| {
            if self.context_structural_key(KeyStroke::char(character)) {
                character.to_string()
            } else {
                "unbound".into()
            }
        };
        for (index, permission) in PERMISSIONS.iter().enumerate() {
            rows.push(review_row(
                format!(
                    "{} [{}] {}",
                    structural_label(char::from_digit((index + 1) as u32, 10).unwrap()),
                    if permissions.contains(permission) {
                        "x"
                    } else {
                        " "
                    },
                    match permission {
                        Permission::Read =>
                            "Read: buffers, terminals and history, the agent channel",
                        Permission::Write => "Write: edit buffers, type in terminals, press Enter",
                    }
                ),
                Vec::new(),
                vec![0],
            ));
        }
        let remember_text = format!(
            "{} [{}] Remember for this workspace; ",
            structural_label('r'),
            if *remember { "x" } else { " " }
        );
        let revoke_text = format!("{} Revoke; ", structural_label('x'));
        let next_text = format!("{} Next identity", structural_label('n'));
        let emphasis = vec![
            0,
            remember_text.chars().count(),
            remember_text.chars().count() + revoke_text.chars().count(),
        ];
        rows.push(review_row(
            format!("{remember_text}{revoke_text}{next_text}"),
            Vec::new(),
            emphasis,
        ));
        rows.push(review_row(String::new(), Vec::new(), Vec::new()));
        let view_rows = surface.view_rows(self.context_ui.view_height());
        let top = surface.top.min(surface.last_top(view_rows));
        for line in surface.lines.iter().skip(top).take(view_rows) {
            let label = format!("{:<LABEL_WIDTH$}", line.label);
            let label_end = label.chars().count();
            let end = label_end + line.value.chars().count();
            let muted = match line.tone {
                Tone::Value => (0..label_end).collect(),
                Tone::Note => (0..end).collect(),
            };
            rows.push(review_row(
                format!("{label}{}", line.value),
                muted,
                Vec::new(),
            ));
        }
        let total = surface.lines.len();
        let scrollable = total > view_rows;
        if scrollable {
            let text = format!(
                "Lines {}–{} of {total} · {}/{} scroll · {}/{} first/last",
                top + 1,
                (top + view_rows).min(total),
                self.context_review_motion_keys(Command::HalfPageUp),
                self.context_review_motion_keys(Command::HalfPageDown),
                self.context_review_motion_keys(Command::MoveFileStart),
                self.context_review_motion_keys(Command::MoveFileEnd),
            );
            let end = text.chars().count();
            rows.push(review_row(text, (0..end).collect(), Vec::new()));
        }
        rows.push(review_row(String::new(), Vec::new(), Vec::new()));
        rows.push(review_row("Reject".into(), Vec::new(), Vec::new()));
        rows.push(review_row(
            "Apply permissions".into(),
            Vec::new(),
            Vec::new(),
        ));
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
        use crate::keymap::ui::{Action, Owner};
        let label = |action| {
            self.keymap
                .ui()
                .compact_label(Owner::ContextReview, action, 1)
        };
        let choices = [Action::Previous, Action::Next, Action::ToggleSelection]
            .into_iter()
            .map(label)
            .filter(|label| label != "unbound")
            .collect::<Vec<_>>();
        let choices = if choices == ["Up", "Down", "Tab"] {
            "↑/↓/Tab".into()
        } else if choices.is_empty() {
            "unbound".into()
        } else {
            choices.join("/")
        };
        let actions = vec![
            OverlayAction::new(choices, "choose"),
            OverlayAction::new(label(Action::Accept), "apply choice"),
            OverlayAction::new(label(Action::Cancel), "reject"),
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

fn review_action(command: Command) -> crate::keymap::ui::Action {
    use crate::keymap::ui::Action;
    match command {
        Command::PageUp => Action::PageUp,
        Command::PageDown => Action::PageDown,
        Command::HalfPageUp => Action::HalfPageUp,
        Command::HalfPageDown => Action::HalfPageDown,
        Command::MoveFileStart => Action::First,
        Command::MoveFileEnd => Action::Last,
        _ => unreachable!("only approved review motions reach this conversion"),
    }
}

fn review_top(action: crate::keymap::ui::Action, top: usize, rows: usize) -> usize {
    use crate::keymap::ui::Action;
    match action {
        Action::PageUp => top.saturating_sub(rows),
        Action::PageDown => top.saturating_add(rows),
        Action::HalfPageUp => top.saturating_sub((rows / 2).max(1)),
        Action::HalfPageDown => top.saturating_add((rows / 2).max(1)),
        Action::First => 0,
        Action::Last => usize::MAX,
        _ => unreachable!("only scrolling actions reach review_top"),
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
