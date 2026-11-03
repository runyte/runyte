// SPDX-License-Identifier: MPL-2.0
use super::{Action, Bindings, Owner};
use crate::{
    input::{KeyCode, KeyStroke, Modifiers},
    keymap::BindingScope,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Input {
    Literal(char),
    Edit(Action),
    Scoped { key: KeyStroke, reset: bool },
}

impl Bindings {
    pub(crate) fn merge_input(
        &self,
        scope: BindingScope,
        key: KeyStroke,
        leader: Option<KeyStroke>,
    ) -> Input {
        let physical_key = key;
        let key = if leader.is_some_and(|leader| key.canonical_for_binding() == leader) {
            KeyStroke::plain(KeyCode::Escape)
        } else {
            key
        };
        if scope == BindingScope::GitMergeInput {
            if let KeyCode::Char(c) = key.code
                && (key.modifiers.is_empty() || key.modifiers == Modifiers::SHIFT)
            {
                return Input::Literal(c);
            }
            if let Some(action) = self.lookup(Owner::MergeAcknowledgment, key) {
                return Input::Edit(action);
            }
        }
        Input::Scoped {
            key,
            reset: self.lookup(Owner::MergePrefix, physical_key) == Some(Action::ResetAndContinue)
                || self.lookup(Owner::MergePrefix, key) == Some(Action::ResetAndContinue),
        }
    }
    pub(crate) fn merge_reset_label(
        &self,
        scope: BindingScope,
        leader: Option<KeyStroke>,
    ) -> String {
        self.reachable_label(Owner::MergePrefix, Action::ResetAndContinue, None, |key| {
            matches!(
                self.merge_input(scope, key, leader),
                Input::Scoped { reset: true, .. }
            )
        })
    }
}

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    if owner == Owner::MergePrefix {
        return (key.code == KeyCode::Escape).then_some(Action::ResetAndContinue);
    }
    match key.code {
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Delete => Some(Action::Delete),
        KeyCode::Left => Some(Action::MoveLeft),
        KeyCode::Right => Some(Action::MoveRight),
        _ => None,
    }
}
pub(super) fn labels(action: Action) -> &'static [&'static str] {
    match action {
        Action::Backspace => &["Backspace"],
        Action::Delete => &["Delete"],
        Action::MoveLeft => &["Left"],
        Action::MoveRight => &["Right"],
        Action::ResetAndContinue => &["Esc"],
        _ => &[],
    }
}

impl crate::keymap::Keymap {
    pub(crate) fn merge_binding_reachable(
        &self,
        scope: BindingScope,
        binding: &crate::keymap::Binding,
        leader: Option<KeyStroke>,
    ) -> bool {
        use crate::{
            command::Mode,
            keymap::{KeySequence, Lookup},
        };
        if !scope.is_merge_review() {
            return true;
        }
        let mut pending = KeySequence::default();
        for (index, key) in binding.sequence.as_slice().iter().enumerate() {
            let Input::Scoped { key, reset } = self.ui().merge_input(scope, *key, leader) else {
                return false;
            };
            if reset && index > 0 {
                return false;
            }
            pending.push(key);
            match self.lookup_in(Mode::Normal, scope, &pending) {
                Lookup::Exact(found) | Lookup::ExactAndPrefix { exact: found, .. } => {
                    return index + 1 == binding.sequence.len()
                        && found.scope == scope
                        && found.target == binding.target;
                }
                Lookup::Prefix(_) => {}
                Lookup::NoMatch => return false,
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_ui_defaults_preserve_all_named_key_modifier_aliases() {
        let bindings = Bindings::default();
        for bits in 0..64 {
            let modifiers = Modifiers::from_bits(bits).unwrap();
            for code in [
                KeyCode::Escape,
                KeyCode::Backspace,
                KeyCode::Delete,
                KeyCode::Left,
                KeyCode::Right,
            ] {
                let key = KeyStroke::new(code, modifiers);
                for owner in [Owner::MergeAcknowledgment, Owner::MergePrefix] {
                    assert_eq!(bindings.lookup(owner, key), legacy_action(owner, key));
                }
            }
        }
    }

    #[test]
    fn scope_validation_uses_effective_ui_ownership_and_owner_rollback() {
        let base = crate::keymap::default_keymap();
        let invalid = crate::keymap::configured::compile(&serde_yaml::from_str("ui: {merge-acknowledgment: {move-left: [a]}}\nscopes: {git-merge-input: {normal: {Left: git-review-next}}}").unwrap(), base);
        assert_eq!(invalid.errors.len(), 2, "{:?}", invalid.errors);
        assert_eq!(
            invalid
                .keymap
                .ui()
                .lookup(Owner::MergeAcknowledgment, KeyStroke::plain(KeyCode::Left)),
            Some(Action::MoveLeft)
        );
        let valid = crate::keymap::configured::compile(&serde_yaml::from_str("ui: {merge-acknowledgment: {move-left: []}, merge-prefix: {reset-and-continue: []}}\nscopes: {git-merge-input: {normal: {Left: git-review-next, 'F6 Esc': git-review-cancel}}}").unwrap(), base);
        assert!(valid.errors.is_empty(), "{:?}", valid.errors);
        assert_eq!(
            valid
                .keymap
                .ui()
                .lookup(Owner::MergeAcknowledgment, KeyStroke::plain(KeyCode::Left)),
            None
        );
        let invalid_suffix = crate::keymap::configured::compile(&serde_yaml::from_str("ui: {merge-prefix: {reset-and-continue: [F8]}}\nscopes: {git-merge-review: {normal: {'F6 F8': git-review-cancel}}}").unwrap(), base);
        assert_eq!(invalid_suffix.errors.len(), 1);
    }
    #[test]
    fn merge_reset_discovery_follows_editing_literal_and_leader_layers() {
        let base = crate::keymap::default_keymap();
        let compile = |source: &str| {
            let result =
                crate::keymap::configured::compile(&serde_yaml::from_str(source).unwrap(), base);
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            result.keymap
        };
        let map = compile("ui: {merge-acknowledgment: {move-left: [Esc]}}");
        assert_ne!(
            map.ui()
                .merge_reset_label(BindingScope::GitMergeInput, None),
            "Esc"
        );
        let map = compile("ui: {merge-prefix: {reset-and-continue: [a]}}");
        assert_eq!(
            map.ui()
                .merge_reset_label(BindingScope::GitMergeInput, None),
            "unbound"
        );
        assert_eq!(
            map.ui()
                .merge_reset_label(BindingScope::GitMergeReview, Some(map.leader())),
            "a"
        );
        let map = compile("leader: F8\nui: {merge-prefix: {reset-and-continue: [F8]}}");
        assert_eq!(
            map.ui()
                .merge_reset_label(BindingScope::GitMergeReview, Some(map.leader())),
            "F8"
        );
        assert_eq!(
            map.ui().merge_input(
                BindingScope::GitMergeReview,
                KeyStroke::plain(KeyCode::Function(8)),
                Some(map.leader())
            ),
            Input::Scoped {
                key: KeyStroke::plain(KeyCode::Escape),
                reset: true
            }
        );
        let result = crate::keymap::configured::compile(&serde_yaml::from_str("leader: Left\nui: {merge-acknowledgment: {move-left: []}}\nscopes: {git-merge-input: {normal: {Left: git-review-next}}}").unwrap(), &crate::keymap::Keymap::new(vec![]).unwrap());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(matches!(
            result.keymap.ui().merge_input(
                BindingScope::GitMergeInput,
                KeyStroke::plain(KeyCode::Left),
                None
            ),
            Input::Scoped { .. }
        ));
    }
}
