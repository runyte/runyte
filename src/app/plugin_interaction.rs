// SPDX-License-Identifier: MPL-2.0

use super::{App, prompt_backspace, prompt_delete, prompt_insert};
use crate::{
    input::{InputEvent, KeyCode, Modifiers},
    plugin::{
        application::CapturedContext,
        interaction::{Field, FieldValidation, Kind, Submission, ValidationStatus, Value},
    },
};

pub(crate) struct Surface {
    pub owner: usize,
    pub handle: String,
    pub context: CapturedContext,
    pub title: String,
    pub fields: Vec<Field>,
    pub values: Vec<Value>,
    pub selected: usize,
    pub cursor: usize,
    pub error: bool,
    pub picker: Option<crate::picker::ListPicker>,
    pub confirmation: bool,
    pub validation: ValidationState,
}
#[derive(Clone)]
pub(crate) struct ValidationIntent {
    pub owner: usize,
    pub surface: String,
    pub revision: String,
    pub fields: Vec<String>,
    pub values: std::collections::BTreeMap<String, Value>,
    pub foreground: u64,
}

#[derive(Default)]
pub(crate) struct ValidationState {
    revision: u64,
    queued: Option<ValidationIntent>,
    in_flight: Option<(String, u64)>,
    submit: Option<(u64, u64)>,
    results: Vec<FieldValidation>,
    unavailable: bool,
}

impl Surface {
    pub fn validation_feedback(&self) -> Option<String> {
        if self.validation.unavailable {
            return Some("Validation unavailable; press Enter to retry or Escape to cancel".into());
        }
        if self.validation.queued.is_some()
            || self
                .validation
                .in_flight
                .as_ref()
                .is_some_and(|(revision, _)| revision == &format!("i:{}", self.validation.revision))
        {
            return Some("Checking fields…".into());
        }
        self.validation
            .results
            .iter()
            .find(|result| result.status != ValidationStatus::Valid)
            .and_then(|result| self.fields.iter().find(|field| field.id == result.field))
            .map(|field| {
                format!(
                    "{}: {}",
                    field.label,
                    field
                        .validation_message
                        .as_deref()
                        .unwrap_or("Value was not accepted; press Enter to retry")
                )
            })
    }

    pub fn display(&self, index: usize) -> String {
        match &self.values[index] {
            Value::Text(value) if self.fields[index].kind == Kind::Secret => {
                "•".repeat(value.chars().count())
            }
            Value::Text(value) => value.clone(),
            Value::Boolean(value) => value.to_string(),
        }
    }
}
impl App {
    pub fn plugin_input_active(&self) -> bool {
        self.plugins.input.is_some()
    }
    pub(crate) fn cancel_plugin_input(&mut self, owner: usize, handle: Option<&str>) {
        if self
            .plugins
            .input
            .as_ref()
            .is_some_and(|s| s.owner == owner && handle.is_none_or(|h| h == s.handle))
        {
            self.finish_plugin_input(false);
        }
    }
    fn finish_plugin_input(&mut self, accepted: bool) {
        let Some(surface) = self.plugins.input.take() else {
            return;
        };
        let mut context = surface.context;
        context.foreground_allowed = accepted;
        context.foreground = self.plugins.foreground_generation;
        context.action = self.active_action_id;
        let sensitive = accepted
            && surface
                .fields
                .iter()
                .any(|field| field.kind == Kind::Secret);
        let values = if accepted {
            surface
                .fields
                .into_iter()
                .zip(surface.values)
                .map(|(f, v)| (f.id, v))
                .collect()
        } else {
            Default::default()
        };
        self.plugins.input_finished.push((
            surface.owner,
            context,
            Submission {
                surface: surface.handle,
                accepted,
                sensitive,
                values,
            },
        ));
        self.plugins.presentation_dirty = true;
    }
    pub(crate) fn sync_plugin_input(&mut self) {
        if self
            .plugins
            .input
            .as_ref()
            .and_then(|surface| surface.validation.submit)
            .is_some_and(|(_, foreground)| foreground != self.plugins.foreground_generation)
        {
            self.cancel_plugin_validation_intent();
        }
        if let Some(surface) = &self.plugins.input
            && (self.has_native_input_overlay()
                || self.mode == super::Mode::Command
                || !self.plugins.frontend_attached
                || self.plugins.attachment_generation != surface.context.attachment
                || self.active_pane != surface.context.pane
                || self.active().buffer != surface.context.buffer
                || self.active().terminal != surface.context.terminal
                || self.host_buffer_is_closed(surface.context.buffer))
        {
            self.finish_plugin_input(false);
        }
    }
    pub(super) fn handle_plugin_input(&mut self, input: InputEvent) {
        self.cancel_plugin_validation_intent();
        let foreground = self.plugins.foreground_generation;
        let Some(surface) = self.plugins.input.as_mut() else {
            return;
        };
        if surface.confirmation {
            if let InputEvent::Key(key) = input {
                if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                    surface.values[0] = Value::Boolean(true);
                    self.finish_plugin_input(true);
                } else if key.code == KeyCode::Escape
                    || (key.code == KeyCode::Char('c') && key.modifiers == Modifiers::CONTROL)
                {
                    self.finish_plugin_input(false);
                }
            }
            return;
        }
        if let Some(picker) = &mut surface.picker {
            match input {
                InputEvent::Key(key)
                    if key.code == KeyCode::Escape
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers == Modifiers::CONTROL) =>
                {
                    self.finish_plugin_input(false)
                }
                InputEvent::Key(key) if key.code == KeyCode::Enter && key.modifiers.is_empty() => {
                    if let Some(item) = picker.selected_item() {
                        surface.values[0] = Value::Text(item.label.clone());
                        self.finish_plugin_input(true);
                    }
                }
                InputEvent::Key(key)
                    if matches!(key.code, KeyCode::Up | KeyCode::Down | KeyCode::BackTab)
                        || (key.modifiers == Modifiers::CONTROL
                            && matches!(key.code, KeyCode::Char('n' | 'p'))) =>
                {
                    let n = picker.visible_indices().len();
                    picker.selected = if matches!(
                        key.code,
                        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('p')
                    ) {
                        picker.selected.saturating_sub(1)
                    } else {
                        (picker.selected + 1).min(n.saturating_sub(1))
                    };
                }
                InputEvent::Key(key) if key.code == KeyCode::Backspace => {
                    picker.filter.pop();
                    picker.selected = 0;
                }
                input => {
                    let text = match input {
                        InputEvent::Text(text) => text,
                        InputEvent::Key(key)
                            if key.modifiers.is_empty() || key.modifiers == Modifiers::SHIFT =>
                        {
                            match key.code {
                                KeyCode::Char(c) => c.to_string(),
                                _ => String::new(),
                            }
                        }
                        _ => String::new(),
                    };
                    if !text.is_empty()
                        && picker.filter.len() + text.len() <= 4096
                        && !text.chars().any(char::is_control)
                    {
                        picker.filter.push_str(&text);
                        picker.selected = 0;
                    }
                }
            }
            self.plugins.presentation_dirty = true;
            return;
        }
        let i = surface.selected;
        let before = surface.values[i].clone();
        match input {
            InputEvent::Key(key)
                if key.code == KeyCode::Escape
                    || (key.code == KeyCode::Char('c') && key.modifiers == Modifiers::CONTROL) =>
            {
                self.finish_plugin_input(false)
            }
            InputEvent::Key(key) if key.code == KeyCode::Enter && key.modifiers.is_empty() => {
                if let Some(index) = surface
                    .fields
                    .iter()
                    .zip(&surface.values)
                    .position(|(f, v)| !f.accepts(v))
                {
                    surface.selected = index;
                    surface.error = true;
                    surface.cursor = match &surface.values[index] {
                        Value::Text(v) => v.chars().count(),
                        _ => 0,
                    };
                } else if surface.fields.iter().any(|field| field.validate) {
                    surface.validation.results.clear();
                    surface.validation.unavailable = false;
                    surface.validation.submit = Some((surface.validation.revision, foreground));
                    surface.validation.queued = Some(ValidationIntent {
                        owner: surface.owner,
                        surface: surface.handle.clone(),
                        revision: format!("i:{}", surface.validation.revision),
                        fields: surface
                            .fields
                            .iter()
                            .filter(|field| field.validate)
                            .map(|field| field.id.clone())
                            .collect(),
                        values: surface
                            .fields
                            .iter()
                            .zip(&surface.values)
                            .filter(|(field, _)| field.kind != Kind::Secret || field.validate)
                            .map(|(field, value)| (field.id.clone(), value.clone()))
                            .collect(),
                        foreground,
                    });
                } else {
                    self.finish_plugin_input(true);
                }
            }
            InputEvent::Key(key)
                if matches!(
                    key.code,
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down
                ) =>
            {
                let back = matches!(key.code, KeyCode::BackTab | KeyCode::Up);
                surface.selected =
                    (i + if back { surface.fields.len() - 1 } else { 1 }) % surface.fields.len();
                surface.cursor = match &surface.values[surface.selected] {
                    Value::Text(v) => v.chars().count(),
                    _ => 0,
                };
                surface.error = false;
            }
            input => {
                let field = &surface.fields[i];
                if matches!(field.kind, Kind::Boolean | Kind::Choice) {
                    if let InputEvent::Key(key) = input
                        && matches!(
                            key.code,
                            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                        )
                        && key.modifiers.is_empty()
                    {
                        match &mut surface.values[i] {
                            Value::Boolean(v) => *v = !*v,
                            Value::Text(v) => {
                                let current =
                                    field.choices.iter().position(|c| c == v).unwrap_or(0);
                                let step = if key.code == KeyCode::Left {
                                    field.choices.len() - 1
                                } else {
                                    1
                                };
                                *v = field.choices[(current + step) % field.choices.len()].clone();
                            }
                        }
                    }
                } else if let Value::Text(value) = &mut surface.values[i] {
                    let text = match input {
                        InputEvent::Text(text) => Some(text),
                        InputEvent::Key(key) => match key.code {
                            KeyCode::Char(c)
                                if key.modifiers.is_empty()
                                    || key.modifiers == Modifiers::SHIFT =>
                            {
                                Some(c.to_string())
                            }
                            KeyCode::Backspace => {
                                prompt_backspace(value, &mut surface.cursor);
                                None
                            }
                            KeyCode::Delete => {
                                prompt_delete(value, surface.cursor);
                                None
                            }
                            KeyCode::Left => {
                                surface.cursor = surface.cursor.saturating_sub(1);
                                None
                            }
                            KeyCode::Right => {
                                surface.cursor = (surface.cursor + 1).min(value.chars().count());
                                None
                            }
                            KeyCode::Home => {
                                surface.cursor = 0;
                                None
                            }
                            KeyCode::End => {
                                surface.cursor = value.chars().count();
                                None
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(text) = text {
                        if value.len().saturating_add(text.len())
                            <= crate::plugin::interaction::MAX_VALUE_BYTES
                            && value.chars().count() + text.chars().count() <= field.maximum_length
                            && !text.chars().any(char::is_control)
                        {
                            for c in text.chars() {
                                prompt_insert(value, surface.cursor, c);
                                surface.cursor += 1;
                            }
                            surface.error = false;
                        } else {
                            surface.error = true;
                        }
                    }
                }
            }
        }
        if let Some(surface) = &mut self.plugins.input
            && surface.values[i] != before
        {
            surface.validation.revision += 1;
            surface.validation.results.clear();
            surface.validation.unavailable = false;
        }
        self.plugins.presentation_dirty = true;
    }
}

impl App {
    pub(crate) fn cancel_plugin_validation_intent(&mut self) {
        if let Some(surface) = &mut self.plugins.input {
            if surface.validation.queued.take().is_some() {
                self.plugins.presentation_dirty = true;
            }
            surface.validation.submit = None;
        }
    }

    pub(crate) fn plugin_validation_current(
        &self,
        owner: usize,
        handle: &str,
        revision: &str,
    ) -> bool {
        self.plugins.input.as_ref().is_some_and(|surface| {
            surface.owner == owner
                && surface.handle == handle
                && revision == format!("i:{}", surface.validation.revision)
        })
    }

    pub(crate) fn peek_plugin_validation(&self) -> Option<ValidationIntent> {
        let surface = self.plugins.input.as_ref()?;
        if surface.validation.in_flight.is_some() {
            return None;
        }
        surface
            .validation
            .queued
            .as_ref()
            .filter(|intent| intent.foreground == self.plugins.foreground_generation)
            .cloned()
    }

    pub(crate) fn plugin_validation_pending(&mut self, intent: &ValidationIntent) -> bool {
        self.sync_plugin_input();
        let Some(surface) = &mut self.plugins.input else {
            return false;
        };
        if surface.owner != intent.owner
            || surface.handle != intent.surface
            || surface.validation.in_flight.is_some()
            || intent.foreground != self.plugins.foreground_generation
            || !surface.validation.queued.as_ref().is_some_and(|queued| {
                queued.revision == intent.revision && queued.foreground == intent.foreground
            })
        {
            return false;
        }
        surface.validation.queued = None;
        surface.validation.unavailable = false;
        surface.validation.results.clear();
        surface.validation.in_flight = Some((intent.revision.clone(), intent.foreground));
        self.plugins.presentation_dirty = true;
        true
    }

    pub(crate) fn apply_plugin_validation(
        &mut self,
        owner: usize,
        handle: &str,
        revision: &str,
        foreground: u64,
        fields: &[FieldValidation],
    ) -> bool {
        self.sync_plugin_input();
        let Some(surface) = &mut self.plugins.input else {
            return false;
        };
        if surface.owner != owner
            || surface.handle != handle
            || surface.validation.in_flight.as_ref() != Some(&(revision.to_owned(), foreground))
        {
            return false;
        }
        surface.validation.in_flight = None;
        if revision != format!("i:{}", surface.validation.revision) {
            self.plugins.presentation_dirty = true;
            return true;
        }
        let expected = surface
            .fields
            .iter()
            .filter(|field| field.validate)
            .map(|field| field.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let received = fields
            .iter()
            .map(|field| field.field.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if expected != received || received.len() != fields.len() {
            surface.validation.unavailable = true;
            if surface.validation.submit == Some((surface.validation.revision, foreground)) {
                surface.validation.submit = None;
            }
            self.plugins.presentation_dirty = true;
            return true;
        }
        surface.validation.results = fields.to_vec();
        surface.validation.unavailable = fields
            .iter()
            .any(|field| field.status == ValidationStatus::Unavailable);
        let valid = fields
            .iter()
            .all(|field| field.status == ValidationStatus::Valid);
        let current_intent = surface.validation.submit
            == Some((surface.validation.revision, foreground))
            && self.plugins.foreground_generation == foreground;
        if current_intent
            && let Some(invalid) = fields
                .iter()
                .find(|field| field.status != ValidationStatus::Valid)
            && let Some(index) = surface
                .fields
                .iter()
                .position(|field| field.id == invalid.field)
        {
            surface.selected = index;
            surface.cursor = match &surface.values[index] {
                Value::Text(value) => value.chars().count(),
                Value::Boolean(_) => 0,
            };
        }
        let submit = valid
            && surface.validation.submit == Some((surface.validation.revision, foreground))
            && self.plugins.foreground_generation == foreground;
        if submit {
            self.finish_plugin_input(true);
        } else if surface.validation.submit == Some((surface.validation.revision, foreground)) {
            surface.validation.submit = None;
        }
        self.plugins.presentation_dirty = true;
        true
    }

    pub(crate) fn plugin_validation_failed(
        &mut self,
        owner: usize,
        handle: &str,
        revision: &str,
        foreground: u64,
    ) -> bool {
        self.sync_plugin_input();
        let Some(surface) = &mut self.plugins.input else {
            return false;
        };
        if surface.owner != owner
            || surface.handle != handle
            || surface.validation.in_flight.as_ref() != Some(&(revision.to_owned(), foreground))
        {
            return false;
        }
        surface.validation.in_flight = None;
        if revision == format!("i:{}", surface.validation.revision) {
            surface.validation.unavailable = true;
            if surface.validation.submit == Some((surface.validation.revision, foreground)) {
                surface.validation.submit = None;
            }
        }
        self.plugins.presentation_dirty = true;
        true
    }
}
