// SPDX-License-Identifier: MPL-2.0

use super::{App, prompt_backspace, prompt_delete, prompt_insert};
use crate::{
    input::{InputEvent, KeyCode, Modifiers},
    plugin::{
        application::CapturedContext,
        interaction::{Field, Kind, Submission, Value},
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
}
impl Surface {
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
                values,
            },
        ));
        self.plugins.presentation_dirty = true;
    }
    pub(crate) fn sync_plugin_input(&mut self) {
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
                InputEvent::Key(key) if key.code == KeyCode::Enter => {
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
        match input {
            InputEvent::Key(key)
                if key.code == KeyCode::Escape
                    || (key.code == KeyCode::Char('c') && key.modifiers == Modifiers::CONTROL) =>
            {
                self.finish_plugin_input(false)
            }
            InputEvent::Key(key) if key.code == KeyCode::Enter => {
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
        self.plugins.presentation_dirty = true;
    }
}
