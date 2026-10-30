// SPDX-License-Identifier: MPL-2.0
use super::App;
use crate::input_grammar::InputGrammar;
use crate::{
    command::Mode,
    input::{KeyCode, KeyStroke, Modifiers},
    keymap::{native_window, ui::Owner},
};

impl App {
    /// Preserve factory window precedence while keeping newly authored global
    /// chords from consuming an active owner's structural character input.
    pub(crate) fn native_window_bindings(&self) -> native_window::Bindings {
        use native_window::Bindings as Window;
        let base = self.keymap.ui().native_window();
        // Authored window characters require Control, Alt or Super. Among
        // these admitted chords, structural owners accept all characters,
        // Super text, or the path menu's exact r/s choices.
        let guard = if self.native_window_structural_key(KeyStroke::ctrl('x')) {
            Window::GUARD_CHARACTER
        } else if self
            .native_window_structural_key(KeyStroke::new(KeyCode::Char('x'), Modifiers::SUPER))
        {
            Window::GUARD_TEXT
        } else if self.native_window_structural_key(KeyStroke::ctrl('r')) {
            Window::GUARD_PATH_CHOICE
        } else {
            Window::GUARD_NONE
        };
        base.clone()
            .with_policy(base.authored(), guard)
            .expect("window input policy")
    }

    fn native_window_structural_key(&self, key: KeyStroke) -> bool {
        if !matches!(key.code, KeyCode::Char(_)) {
            return false;
        }
        if self.context_overlay_active() {
            return Owner::ContextReview.structural(key);
        }
        if self.macro_replay.is_some() || self.provider_ui_owner().is_some() {
            return false;
        }
        if let Some(owner) = self.plugin_input_ui_owner() {
            return owner.structural(key);
        }
        if let Some(owner) = self.session_navigation_ui_owner() {
            return owner.structural(key);
        }
        if let Some(review) = &self.merge_ui.review {
            return review.input_focused() && Owner::MergeAcknowledgment.structural(key);
        }
        if let Some(owner) = self.confirmation_ui_owner() {
            let typed = if self.git_branch_switch.is_some() {
                true
            } else if let Some(value) = &self.git_branch_deletion {
                value.typed()
            } else {
                self.git_worktree_removal
                    .as_ref()
                    .is_some_and(|value| value.typed())
            };
            return (owner != Owner::TextConfirmation || typed) && owner.structural(key);
        }
        if self.context_action_menu.is_some() {
            return Owner::ContextActions.structural(key);
        }
        if self.program_action_menu.is_some() {
            return Owner::ProgramActions.structural(key);
        }
        if self.path_action_menu.is_some() {
            return Owner::PathActions.structural(key);
        }
        if self.path_popup.is_some() {
            return Owner::Path.structural(key);
        }
        if self.picker.is_some() {
            return self.picker_ui_owner().structural(key);
        }
        if let Some(list) = &self.list {
            if let Some(owner) = self.list_menu_ui_owner() {
                return owner.structural(key);
            }
            return list.accepts_filter_input()
                && self.list_binding_for(key).is_none()
                && !key
                    .modifiers
                    .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER);
        }
        if self.jump.is_some() {
            return !key
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::ALT);
        }
        if self.mode == Mode::Command {
            return self.prompt_ui_owner().structural(key);
        }
        matches!(self.mode, Mode::Normal | Mode::Select)
            && self.grammar.awaiting_character().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_guards_cover_admitted_structural_characters() {
        let mut app = App::new(crate::config::Config::default(), None).unwrap();
        for state in 0..4 {
            match state {
                0 => (),
                1 => {
                    app.handle_key(KeyStroke::char('f')).unwrap();
                }
                2 => {
                    app.handle_key(KeyStroke::char('x')).unwrap();
                    app.handle_key(KeyStroke::char(':')).unwrap();
                }
                3 => {
                    app.handle_key(KeyStroke::plain(KeyCode::Escape)).unwrap();
                    app.path_popup = Some(super::super::PathPopup {
                        path: "example.txt".into(),
                    });
                    app.handle_key(KeyStroke::plain(KeyCode::Tab)).unwrap();
                }
                _ => unreachable!(),
            }
            let policy = app.native_window_bindings();
            for bits in 0..64 {
                let modifiers = Modifiers::from_bits(bits).unwrap();
                if !modifiers.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER) {
                    continue;
                }
                for c in ['x', 'r', 's', 'X', 'R', 'S', '1', '9', 'é'] {
                    let key = KeyStroke::new(KeyCode::Char(c), modifiers).canonical_for_binding();
                    let map = native_window::Bindings::from_entries(vec![(
                        key,
                        native_window::Action::FontSize(1),
                    )])
                    .unwrap()
                    .with_policy(1, policy.guard())
                    .unwrap();
                    assert_eq!(
                        map.lookup(key).is_none(),
                        app.native_window_structural_key(key),
                        "state {state}, {key:?}"
                    );
                }
            }
        }
    }
}
