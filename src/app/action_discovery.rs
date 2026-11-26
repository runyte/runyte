// SPDX-License-Identifier: MPL-2.0
//! Native read-only discovery over the current keymap.
use super::{
    App, Buffer, InputEvent, InputGrammar, KeyCode, KeySequence, KeyStroke, ListAction, ListPicker,
    Mode, PickerItem, Selection,
};
use crate::{
    command::discovery,
    keymap::{BindingScope, Lookup},
};
use std::sync::Arc;

pub(super) struct Capture {
    pane: usize,
    buffer: usize,
    mode: Mode,
    scope: BindingScope,
    map: Arc<crate::keymap::Keymap>,
    sequence: KeySequence,
}

impl App {
    pub(super) fn start_describe_key(&mut self) {
        if self.has_input_overlay() || self.macro_replay.is_some() {
            self.action_failed("Close the active overlay before describing a key");
            return;
        }
        self.grammar.reset();
        self.described_binding = None;
        self.describe_key = Some(Capture {
            pane: self.active_pane,
            buffer: self.active().buffer,
            mode: self.mode,
            scope: self.key_binding_scope(),
            map: self.keymap.clone(),
            sequence: KeySequence::default(),
        });
        self.status("Describe key: press a key sequence · Esc cancels");
    }

    pub(super) fn capture_described_input(&mut self, input: InputEvent) {
        let Some(mut capture) = self.describe_key.take() else {
            return;
        };
        if capture.pane != self.active_pane
            || capture.buffer != self.active().buffer
            || capture.mode != self.mode
            || capture.scope != self.key_binding_scope()
            || !Arc::ptr_eq(&capture.map, &self.keymap)
        {
            self.status("Describe key cancelled: view, mode or bindings changed");
            return;
        }
        let key = match input {
            InputEvent::Key(key) => key,
            InputEvent::Text(text) if text.chars().count() == 1 => {
                KeyStroke::char(text.chars().next().unwrap())
            }
            _ => {
                self.status("Describe key cancelled");
                return;
            }
        };
        if key.code == KeyCode::Escape {
            self.grammar.reset();
            self.status("Describe key cancelled");
            return;
        }
        let child = capture.sequence.is_empty()
            && capture.scope == BindingScope::Terminal
            && capture.mode == Mode::Insert
            && capture.map.terminal_input_route(
                key,
                self.persistent_session,
                self.config.editor.fast_pane_keys,
            ) == crate::keymap::terminal_input::Route::Child;
        capture.sequence.push(key.canonical_for_binding());
        let mut text = format!(
            "Describe key: {}\n\nView: {}\nMode: {}\n\n",
            capture.sequence,
            capture.scope.name(),
            crate::keymap::actions::mode_name(capture.mode)
        );
        if child {
            text.push_str("Terminal input: this key is sent to the child process. Nothing was sent during inspection.\n");
        } else {
            match capture
                .map
                .lookup_in(capture.mode, capture.scope, &capture.sequence)
            {
                Lookup::Prefix(_) if capture.sequence.len() < 8 => {
                    self.status(format!(
                        "Describe key: {} … · Esc cancels",
                        capture.sequence
                    ));
                    self.describe_key = Some(capture);
                    return;
                }
                Lookup::ExactAndPrefix { exact, .. } if capture.sequence.len() < 8 => {
                    self.status(format!(
                        "Describe key: {} … (exact: {}; longer chord pending) · Esc cancels",
                        capture.sequence, exact.description
                    ));
                    self.describe_key = Some(capture);
                    return;
                }
                Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                    text.push_str(&format!(
                        "Binding: {}\nAvailability: {}\n",
                        binding.description,
                        discovery::display::binding_availability(binding.availability)
                    ));
                    if let crate::keymap::BindingTarget::Plugin(id) = binding.target {
                        text.push_str(&format!("Source: runtime plugin action {id}\n"));
                    } else if binding.source_paths.is_empty() {
                        text.push_str("Source: built-in keymap\n");
                    } else {
                        for path in &binding.source_paths {
                            match self.config.key_sources.location(path) {
                                Some(location) => text.push_str(&format!(
                                    "Source: {}{} (keys.{})\n",
                                    location.file.display(),
                                    location
                                        .line
                                        .map(|line| format!(":{line}"))
                                        .unwrap_or_else(|| " (line unavailable)".into()),
                                    path.join(".")
                                )),
                                None => text.push_str(&format!(
                                    "Source: configuration keys.{} (file/line unavailable)\n",
                                    path.join(".")
                                )),
                            }
                        }
                    }
                    text.push('\n');
                    text.push_str(&discovery::binding_steps(binding, capture.scope));
                    match discovery::binding_yaml(
                        binding,
                        capture.mode,
                        self.config.keys.as_ref(),
                        &capture.map,
                    ) {
                        Ok(yaml) => {
                            text.push_str(&format!(
                                "\nCopy with :{}\n\n{}",
                                discovery::COPY_BINDING,
                                yaml
                            ));
                            self.described_binding = Some(yaml);
                        }
                        Err(reason) => {
                            text.push_str(&format!("\nYAML copy unavailable: {reason}\n"))
                        }
                    }
                }
                _ => {
                    let mut prefix = capture.sequence.clone();
                    prefix.pop();
                    if let Lookup::ExactAndPrefix { exact, .. } =
                        capture.map.lookup_in(capture.mode, capture.scope, &prefix)
                    {
                        text.push_str(&format!("Prefix {prefix} also binds {}. In Insert/Replace, an unmatched suffix resolves that exact binding and reprocesses the suffix; modal input reports no binding. Neither was executed here.\n", exact.description));
                    }
                    text.push_str(match capture.mode {
                        Mode::Insert | Mode::Replace => "No keymap binding. Literal text and editing fallback remain owned by the input grammar.\n",
                        _ => "No keymap binding. Counts, registers and UI-owned keys retain their input-layer meaning.\n",
                    });
                }
            }
        }
        text.push_str("\nInspection did not execute the binding.\n");
        self.open_discovery_page(&text);
    }

    pub(super) fn open_discovery_page(&mut self, text: &str) {
        self.leave_directory_tree();
        let buffer = self
            .discovery_buffer
            .filter(|id| !self.closed_buffers.contains(id));
        let buffer = match buffer {
            Some(id) => {
                self.buffers[id].replace_virtual_text(text);
                id
            }
            None => {
                self.buffers
                    .push(Buffer::virtual_text("[key discovery]", text));
                self.syntax.push(None);
                self.buffers.len() - 1
            }
        };
        self.discovery_buffer = Some(buffer);
        self.push_jump();
        let pane = self.active_mut();
        pane.retarget(buffer);
        pane.replace_selection(Selection::point(0));
        pane.scroll_row = 0;
        pane.scroll_col = 0;
        pane.scroll_wrap = 0;
        self.mode = Mode::Normal;
    }

    pub(super) fn open_action_browser(&mut self) {
        let mode = self.mode;
        let scope = self.key_binding_scope();
        let capabilities = self.command_capabilities();
        let defaults = discovery::BindingIndex::new(
            &crate::keymap::keymap_for(self.keymap.fast_pane_keys()),
            mode,
            scope,
        );
        let effective = discovery::BindingIndex::new(self.keymap(), mode, scope);
        let mut items = Vec::new();
        let mut actions = Vec::new();
        for id in discovery::identities() {
            let d = id.descriptor().unwrap();
            let availability = capabilities.id_availability(id);
            let bindable = discovery::admitted_here(id, mode, scope);
            let writable = self.active_terminal().is_some()
                || scope == BindingScope::DirectoryTree
                || !self.active_buffer().is_read_only()
                || !d.policy.requires_writable_buffer;
            let usable = bindable && writable && availability.is_available();
            let details = format!(
                "{}\nBinding admission here: {}\nWritable-buffer gate: {}\nCapability/context: {}\n",
                discovery::details_indexed(id, mode, scope, &defaults, &effective),
                bindable,
                writable,
                discovery::display::command_availability(&availability)
            );
            let index = actions.len();
            let mut item = PickerItem::new(d.metadata.name, d.metadata.description, index)
                .with_preview(&details);
            if usable {
                item = item.with_tag("Current view and mode");
            }
            items.push(item);
            actions.push(ListAction::ActionDetails(details));
        }
        self.list_actions = actions;
        self.settings_view = None;
        self.list = Some(
            ListPicker::new("Actions", items)
                .with_primary_action("inspect")
                .with_tags(vec!["Current view and mode".into()])
                .with_key_legend(),
        );
        self.list.as_mut().unwrap().show_preview = true;
    }

    pub(super) fn copy_described_binding(&mut self) {
        let Some(yaml) = self.described_binding.as_ref() else {
            self.action_failed(format!(
                "Inspect a copyable binding with :{} first",
                discovery::DESCRIBE_KEY
            ));
            return;
        };
        match self.ports.clipboard().write(yaml) {
            Ok(()) => self.status("Copied binding as YAML"),
            Err(error) => self.action_failed(error.to_string()),
        }
    }
}
