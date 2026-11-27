// SPDX-License-Identifier: MPL-2.0
//! Named binding identity alongside stage 4's retained editing semantics.
use super::{App, Mode};
use crate::keymap::{
    BindingScope,
    mode_graph::{ModeId, UnboundKeys},
};

#[derive(Clone, Debug)]
pub(super) struct ActiveMode {
    id: ModeId,
    name: String,
    editing: Mode,
    role_mode: bool,
    scope: BindingScope,
    pane: usize,
    buffer: usize,
}

impl App {
    pub fn current_binding_mode(&self) -> Option<ModeId> {
        let scope = self.key_binding_scope();
        if let Some(active) = &self.binding_mode
            && active.editing == self.mode
            && active.scope == scope
            && active.pane == self.active_pane
            && active.buffer == self.active().buffer
            && self
                .keymap
                .mode_graph()
                .is_some_and(|graph| graph.get(active.id).is_some())
        {
            return Some(active.id);
        }
        self.keymap.named_mode_for(self.mode, scope)
    }

    pub(super) fn remember_binding_mode(&mut self, id: ModeId) {
        let Some(mode) = self.keymap.mode_graph().and_then(|graph| graph.get(id)) else {
            return;
        };
        self.binding_mode = Some(ActiveMode {
            id,
            name: mode.name().into(),
            editing: self.mode,
            role_mode: self
                .keymap
                .named_mode_for(self.mode, self.key_binding_scope())
                == Some(id),
            scope: self.key_binding_scope(),
            pane: self.active_pane,
            buffer: self.active().buffer,
        });
    }

    pub(super) fn lookup_active_binding(
        &self,
        sequence: &crate::keymap::KeySequence,
    ) -> crate::keymap::Lookup<'_> {
        match self.current_binding_mode() {
            Some(id) => self.keymap.lookup_named(id, sequence),
            None => self
                .keymap
                .lookup_in(self.mode, self.key_binding_scope(), sequence),
        }
    }

    pub(super) fn binding_mode_name(&self) -> Option<String> {
        self.current_binding_mode()
            .and_then(|id| self.keymap.mode_graph()?.get(id))
            .map(|mode| mode.name().to_owned())
    }

    pub(super) fn restore_binding_mode(&mut self, mode: Mode, name: Option<&str>) {
        let (mode, name) = if self.role_available(mode) {
            (mode, name)
        } else {
            (Mode::Normal, None)
        };
        if mode == Mode::Normal && matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.enter_normal_mode();
        } else {
            self.set_editing_role(mode);
        }
        if let Some(id) = name.and_then(|name| self.retained_mode(name, mode)) {
            self.remember_binding_mode(id);
        }
    }

    pub(super) fn role_available(&self, mode: Mode) -> bool {
        self.keymap.mode_graph().is_none()
            || !matches!(
                mode,
                Mode::Normal | Mode::Select | Mode::Insert | Mode::Replace
            )
            || self
                .keymap
                .named_mode_for(mode, self.key_binding_scope())
                .is_some()
    }

    /// Ownership changes may carry a role the destination does not provide.
    /// Unlike explicit entry, settlement returns to its required Normal role.
    pub(super) fn settle_carried_binding_mode(&mut self) {
        if !self.role_available(self.mode) {
            if let Some(source) = self.binding_mode.as_ref().map(|active| active.buffer)
                && source != self.active().buffer
                && matches!(self.mode, Mode::Insert | Mode::Replace)
            {
                self.commit_edit_checkpoint(source);
            }
            self.enter_normal_mode();
        }
    }

    pub(super) fn require_editing_role(&mut self, mode: Mode) -> bool {
        if !self.role_available(mode) {
            self.action_failed("editing role is unavailable for this buffer type");
            return false;
        }
        true
    }

    fn retained_mode(&self, name: &str, editing: Mode) -> Option<ModeId> {
        let graph = self.keymap.mode_graph()?;
        let id = graph.id(name)?;
        let scope = self.key_binding_scope();
        let kind = if matches!(scope, BindingScope::Global | BindingScope::Plugin(_)) {
            "text"
        } else {
            scope.name()
        };
        (graph.editing_role(kind, id)?.editing_mode() == editing).then_some(id)
    }

    /// Explicit role entry, including Normal→Normal when leaving a submode.
    pub(super) fn set_editing_role(&mut self, mode: Mode) {
        if self.panes.contains_key(&self.active_pane) && !self.require_editing_role(mode) {
            return;
        }
        self.mode = mode;
        self.binding_mode = None;
        if self.panes.contains_key(&self.active_pane)
            && let Some(id) = self.keymap.named_mode_for(mode, self.key_binding_scope())
        {
            self.remember_binding_mode(id);
        }
    }

    pub(super) fn enter_named_binding_mode(&mut self, argument: &str) {
        if matches!(argument, "list" | "session-manager") {
            self.action_failed("UI list modes cannot be entered as editor modes");
            return;
        }
        let scope = self.key_binding_scope();
        let kind = if scope == BindingScope::Global || matches!(scope, BindingScope::Plugin(_)) {
            "text"
        } else {
            scope.name()
        };
        let Some(graph) = self.keymap.mode_graph() else {
            if let Some(role) = crate::keymap::mode_graph::EditingRole::named(argument) {
                self.enter_mode_preserving_selection(role.editing_mode());
            } else {
                self.action_failed("unknown binding mode");
            }
            return;
        };
        let Some(id) = (if let Some(role) = crate::keymap::mode_graph::EditingRole::named(argument)
        {
            self.keymap.named_mode_for(role.editing_mode(), scope)
        } else {
            graph.entry_mode(kind, argument)
        }) else {
            self.action_failed("mode or editing role is unavailable for this buffer type");
            return;
        };
        let Some(role) = graph.editing_role(kind, id) else {
            self.action_failed("binding mode is unavailable");
            return;
        };
        if self.enter_mode_preserving_selection(role.editing_mode()) {
            self.remember_binding_mode(id);
            self.grammar.reset_prefix();
        }
    }

    pub fn unbound_key_policy(&self) -> UnboundKeys {
        self.current_binding_mode()
            .and_then(|id| self.keymap.mode_graph()?.get(id))
            .map(|mode| mode.unbound_keys())
            .unwrap_or_else(|| {
                if self.keymap.mode_graph().is_some() {
                    UnboundKeys::Report
                } else {
                    match self.mode {
                        Mode::Insert => UnboundKeys::PassToBuffer,
                        Mode::Replace => UnboundKeys::Overwrite,
                        _ => UnboundKeys::Report,
                    }
                }
            })
    }

    /// Re-resolve by name after reload; old graph IDs cannot be reused.
    pub(super) fn rebind_named_mode(&mut self) {
        self.settle_carried_binding_mode();
        let retained = self
            .binding_mode
            .as_ref()
            .filter(|active| {
                !active.role_mode
                    && active.editing == self.mode
                    && active.pane == self.active_pane
                    && active.buffer == self.active().buffer
                    && active.scope == self.key_binding_scope()
            })
            .map(|active| active.name.clone());
        self.binding_mode = None;
        if let Some(id) = retained
            .as_deref()
            .and_then(|name| self.retained_mode(name, self.mode))
            .or_else(|| {
                self.keymap
                    .named_mode_for(self.mode, self.key_binding_scope())
            })
        {
            self.remember_binding_mode(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        input::{InputEvent, KeyStroke},
        keymap::KeySequence,
    };

    fn app() -> App {
        let config: Config = serde_yaml::from_str("editor: {scratch_markdown: false}\nmodes:\n  first: {inherits: normal}\n  second: {inherits: normal}\nbuffer-types:\n  text: first\nkeys:\n  bind:\n    first: {'F6 Down': move-right}\n    second: {'F6 x': move-left}\n").unwrap();
        let mut app = App::new(config, None).unwrap();
        app.insert_text("abc");
        app.active_mut()
            .replace_selection(crate::selection::Selection::point(0));
        app
    }

    #[test]
    fn named_modes_dispatch_and_explicit_role_entry_leaves_submode() {
        let mut app = app();
        assert_eq!(app.binding_mode_name().as_deref(), Some("first"));
        app.enter_named_binding_mode("second");
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.binding_mode_name().as_deref(), Some("second"));
        app.enter_named_binding_mode("normal");
        assert_eq!(app.binding_mode_name().as_deref(), Some("first"));
        for key in KeySequence::parse("F6 Down").unwrap().as_slice() {
            app.handle_input(InputEvent::Key(*key)).unwrap();
        }
        assert_eq!(app.active().selection.primary().head, 1);
        assert_eq!(app.binding_mode_name().as_deref(), Some("first"));
    }

    #[test]
    fn hints_reset_between_same_semantics_modes_and_preserve_bound_arrows() {
        use crate::input::KeyCode;
        use crate::key_hints::{HintEventResult, KeyHintState};
        let mut app = app();
        let mut hints = KeyHintState::default();
        let prefix = KeyStroke::plain(KeyCode::Function(6));
        hints.observe_named(
            prefix,
            app.mode,
            app.key_binding_scope(),
            app.current_binding_mode(),
            app.keymap(),
        );
        assert!(
            !hints
                .rows_named(
                    app.keymap(),
                    app.mode,
                    app.key_binding_scope(),
                    app.current_binding_mode()
                )
                .is_empty()
        );
        assert_eq!(
            hints.observe_named(
                KeyStroke::plain(KeyCode::Down),
                app.mode,
                app.key_binding_scope(),
                app.current_binding_mode(),
                app.keymap()
            ),
            HintEventResult::Forward
        );
        hints.observe_named(
            prefix,
            app.mode,
            app.key_binding_scope(),
            app.current_binding_mode(),
            app.keymap(),
        );
        app.enter_named_binding_mode("second");
        assert!(
            hints
                .rows_named(
                    app.keymap(),
                    app.mode,
                    app.key_binding_scope(),
                    app.current_binding_mode()
                )
                .is_empty()
        );
        hints.observe_named(
            KeyStroke::char('x'),
            app.mode,
            app.key_binding_scope(),
            app.current_binding_mode(),
            app.keymap(),
        );
        assert!(!hints.is_pending());
        assert!(hints.message().is_none());
    }

    #[test]
    fn reload_rebinds_names_and_missing_role_stops_entry() {
        let mut app = app();
        app.enter_named_binding_mode("second");
        let previous = app.current_binding_mode();
        let rebuilt = crate::keymap::configured::compile_configuration(
            &app.config,
            crate::keymap::default_keymap(),
        );
        app.set_keymap(rebuilt.keymap);
        assert_ne!(previous, app.current_binding_mode());
        assert_eq!(app.binding_mode_name().as_deref(), Some("second"));
        let config: Config = serde_yaml::from_str(
            "editor: {scratch_markdown: false}\nbuffer-types:\n  text: {normal: normal}\n",
        )
        .unwrap();
        let mut app = App::new(config, None).unwrap();
        app.enter_named_binding_mode("insert");
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status_error);
    }
    #[test]
    fn missing_roles_refuse_default_and_configured_entry_before_editing() {
        let config: Config = serde_yaml::from_str(
            "editor: {scratch_markdown: false}\nbuffer-types:\n  text: {normal: normal}\nkeys:\n  bind:\n    normal:\n      F6: open-line-below\n      F7: append-after\n",
        ).unwrap();
        let compiled = crate::keymap::configured::compile_configuration(
            &config,
            crate::keymap::default_keymap(),
        );
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        for keys in ["i", "R", "v", "o", "F6", "F7"] {
            let mut app = App::new(config.clone(), None).unwrap();
            app.insert_text("abc");
            app.active_mut()
                .replace_selection(crate::selection::Selection::point(0));
            for key in KeySequence::parse(keys).unwrap().as_slice() {
                app.handle_input(InputEvent::Key(*key)).unwrap();
            }
            assert_eq!(app.mode, Mode::Normal, "{keys}");
            assert_eq!(app.active_buffer().text().to_string(), "abc", "{keys}");
            assert_eq!(app.active().selection.primary().head, 0, "{keys}");
            assert!(app.status_error, "{keys}: {}", app.status);
        }
    }

    #[test]
    fn reload_drops_submode_when_its_editing_role_changes() {
        let mut app = app();
        app.enter_named_binding_mode("second");
        let config: Config = serde_yaml::from_str(
            "modes:\n  first: {inherits: normal}\n  second: {inherits: insert}\nbuffer-types:\n  text: first\n",
        ).unwrap();
        let rebuilt = crate::keymap::configured::compile_configuration(
            &config,
            crate::keymap::default_keymap(),
        );
        assert!(rebuilt.errors.is_empty(), "{:?}", rebuilt.errors);
        app.set_keymap(rebuilt.keymap);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.binding_mode_name().as_deref(), Some("first"));
    }
    #[test]
    fn named_unbound_policy_controls_physical_keys_and_pasted_text() {
        for (policy, expected) in [
            ("report", "abc"),
            ("pass-to-buffer", "Zabc"),
            ("overwrite", "Zbc"),
        ] {
            for paste in [false, true] {
                let config: Config = serde_yaml::from_str(&format!(
                    "editor: {{scratch_markdown: false}}\nmodes:\n  typing: {{unbound-keys: {policy}}}\nbuffer-types:\n  text: typing\n",
                )).unwrap();
                let mut app = App::new(config, None).unwrap();
                app.insert_text("abc");
                app.active_mut()
                    .replace_selection(crate::selection::Selection::point(0));
                let event = if paste {
                    InputEvent::Text("Z".into())
                } else {
                    InputEvent::Key(KeyStroke::char('Z'))
                };
                app.handle_input(event).unwrap();
                assert_eq!(
                    app.active_buffer().text().to_string(),
                    expected,
                    "{policy} paste={paste}"
                );
            }
        }
    }

    #[test]
    fn synchronous_theme_refresh_preserves_remaining_program_steps() {
        let path =
            std::env::temp_dir().join(format!("runyte-mode-theme-{}.yaml", std::process::id()));
        let config: Config = serde_yaml::from_str(
            "editor: {scratch_markdown: false}\nkeys:\n  bind:\n    normal:\n      F6: [{command: theme, argument: dark}, move-right]\n",
        ).unwrap();
        let mut app = App::new(config, None).unwrap();
        app.note_loaded_config(&path);
        app.insert_text("abc");
        app.active_mut()
            .replace_selection(crate::selection::Selection::point(0));
        app.handle_input(InputEvent::Key(KeyStroke::plain(
            crate::input::KeyCode::Function(6),
        )))
        .unwrap();
        assert_eq!(app.active().selection.primary().head, 1, "{}", app.status);
        assert!(!app.status_error, "{}", app.status);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn reload_and_restore_missing_active_role_fall_back_to_normal() {
        let mut app = app();
        app.enter_named_binding_mode("insert");
        assert_eq!(app.mode, Mode::Insert);
        let config: Config =
            serde_yaml::from_str("buffer-types: {text: {normal: normal}}").unwrap();
        let compiled = crate::keymap::configured::compile_configuration(
            &config,
            crate::keymap::default_keymap(),
        );
        app.set_keymap(compiled.keymap);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.binding_mode_name().as_deref(), Some("normal"));
        app.set_editing_role(Mode::Command);
        app.restore_binding_mode(Mode::Insert, Some("insert"));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.binding_mode_name().as_deref(), Some("normal"));
    }

    #[test]
    fn keyboard_pane_focus_settles_a_missing_destination_role() {
        let path =
            std::env::temp_dir().join(format!("runyte-mode-focus-{}.md", std::process::id()));
        std::fs::write(&path, "markdown").unwrap();
        let config: Config = serde_yaml::from_str(
            "editor: {scratch_markdown: false}\nbuffer-types: {markdown: {normal: markdown}}",
        )
        .unwrap();
        let mut app = App::new(config, None).unwrap();
        let source = app.active_pane;
        app.split(crate::layout::Axis::Vertical, Some(path.clone()))
            .unwrap();
        let destination = app.active_pane;
        assert_eq!(app.key_binding_scope(), BindingScope::Markdown);
        app.activate_pane(source);
        app.enter_named_binding_mode("insert");
        for key in KeySequence::parse("Ctrl-w w").unwrap().as_slice() {
            app.handle_input(InputEvent::Key(*key)).unwrap();
        }
        assert_eq!(app.active_pane, destination);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.binding_mode_name().as_deref(), Some("markdown"));
        app.handle_input(InputEvent::Key(KeyStroke::char(':')))
            .unwrap();
        assert_eq!(app.mode, Mode::Command);
        std::fs::remove_file(path).unwrap();
    }
}
