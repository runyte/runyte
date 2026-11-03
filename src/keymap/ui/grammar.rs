// SPDX-License-Identifier: MPL-2.0
use super::{Action, Bindings, Owner};
use crate::input::{KeyCode, KeyStroke};

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    if owner == Owner::GrammarPrefix {
        return match key.code {
            KeyCode::Escape => Some(Action::Cancel),
            KeyCode::Backspace => Some(Action::Back),
            _ => None,
        };
    }
    if key == KeyStroke::ctrl('n') || key == KeyStroke::plain(KeyCode::Down) {
        Some(Action::Next)
    } else if key == KeyStroke::ctrl('p') || key == KeyStroke::plain(KeyCode::Up) {
        Some(Action::Previous)
    } else {
        None
    }
}

pub(super) fn labels(owner: Owner, action: Action) -> &'static [&'static str] {
    match (owner, action) {
        (Owner::GrammarPrefix, Action::Cancel) => &["Esc"],
        (Owner::GrammarPrefix, Action::Back) => &["Backspace"],
        (Owner::KeyHints, Action::Next) => &["Ctrl-n", "Down"],
        (Owner::KeyHints, Action::Previous) => &["Ctrl-p", "Up"],
        _ => &[],
    }
}

impl Bindings {
    /// Factory arrows yield to registered continuations. Authored aliases,
    /// including authored arrows, own input whenever hint discovery is pending.
    pub(crate) fn unconditional_hint_action(&self, key: KeyStroke) -> Option<Action> {
        let action = self.lookup(Owner::KeyHints, key)?;
        if !self.explicitly_configured(Owner::KeyHints, action)
            && matches!(key.code, KeyCode::Up | KeyCode::Down)
        {
            None
        } else {
            Some(action)
        }
    }

    pub(crate) fn input_policy_configured(&self) -> bool {
        [Owner::GrammarPrefix, Owner::KeyHints]
            .iter()
            .any(|owner| !self.owners[owner].configured.is_empty())
    }

    pub(crate) fn input_policy_conflict(&self) -> Option<String> {
        self.owners[&Owner::GrammarPrefix]
            .keys
            .keys()
            .filter_map(|key| {
                self.lookup(Owner::KeyHints, *key).map(|_| {
                    format!(
                        "{} is owned by both hint scrolling and prefix control",
                        key.label()
                    )
                })
            })
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::{EditorCommand, Mode},
        input::{InputEvent, Modifiers},
        input_grammar::{GrammarContext, InputGrammar, RunyteGrammar},
        key_hints::{HintEventResult, KeyHintState},
        keymap::{Binding, BindingScope, KeySequence, Keymap, Lookup},
    };

    fn compile(source: &str) -> std::sync::Arc<Keymap> {
        let compiled = crate::keymap::configured::compile(
            &serde_yaml::from_str(source).unwrap(),
            crate::keymap::default_keymap(),
        );
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        crate::keymap::validate::assert_valid(&compiled.keymap);
        compiled.keymap
    }

    #[test]
    fn prefix_and_hint_defaults_preserve_raw_modifier_masks() {
        let bindings = Bindings::default();
        for bits in 0..64 {
            for code in [
                KeyCode::Escape,
                KeyCode::Backspace,
                KeyCode::Up,
                KeyCode::Down,
                KeyCode::Char('n'),
                KeyCode::Char('p'),
            ] {
                let key = KeyStroke::new(code, Modifiers::from_bits(bits).unwrap());
                for owner in [Owner::GrammarPrefix, Owner::KeyHints] {
                    assert_eq!(
                        bindings.lookup(owner, key),
                        legacy_action(owner, key),
                        "{owner:?} {key:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn prefix_controls_and_hints_keep_counts_and_scroll_ownership() {
        let keymap = compile(
            "ui: {grammar-prefix: {cancel: [F8], back: [F7]}, key-hints: {next: [F6], previous: [F5]}}",
        );
        for mode in [Mode::Normal, Mode::Select] {
            let mut grammar = RunyteGrammar::default();
            let mut hints = KeyHintState::default();
            for key in KeySequence::parse("2 Space g").unwrap().as_slice() {
                assert_eq!(
                    hints.observe_in(*key, mode, BindingScope::Global, &keymap),
                    HintEventResult::Forward
                );
                grammar
                    .translate(
                        InputEvent::Key(*key),
                        GrammarContext::new(mode, BindingScope::Global, &keymap),
                    )
                    .unwrap();
            }
            for (key, offset) in [("F6", 1), ("F5", 0)] {
                assert_eq!(
                    hints.observe_in(
                        KeyStroke::parse(key).unwrap(),
                        mode,
                        BindingScope::Global,
                        &keymap
                    ),
                    HintEventResult::Consumed
                );
                assert_eq!(hints.scroll_offset(), offset);
            }
            assert_eq!(
                hints.scroll_key_label(mode, BindingScope::Global, &keymap),
                "F6 / F5"
            );
            for (key, length, count) in [("F7", 1, Some(2)), ("F8", 0, None)] {
                let key = KeyStroke::parse(key).unwrap();
                assert_eq!(
                    hints.observe_in(key, mode, BindingScope::Global, &keymap),
                    HintEventResult::Forward
                );
                grammar
                    .translate(
                        InputEvent::Key(key),
                        GrammarContext::new(mode, BindingScope::Global, &keymap),
                    )
                    .unwrap();
                assert_eq!(grammar.pending_sequence().len(), length);
                assert_eq!(hints.pending().len(), length);
                assert_eq!(grammar.pending_count(), count);
            }
        }
    }

    #[test]
    fn released_controls_can_be_continuations_and_plugin_bindings() {
        let keymap = compile(
            "ui: {grammar-prefix: {cancel: [F8], back: []}, key-hints: {next: [], previous: []}}\nbind: {normal: {'F9 Esc': move-left, 'F9 Backspace': move-right, 'F9 Ctrl-n': move-up, 'F9 Ctrl-p': move-down, 'F9 Down': move-down}}",
        );
        for suffix in ["Esc", "Backspace", "Ctrl-n", "Ctrl-p", "Down"] {
            let sequence = KeySequence::parse(&format!("F9 {suffix}")).unwrap();
            let mut grammar = RunyteGrammar::default();
            let mut hints = KeyHintState::default();
            for key in sequence.as_slice() {
                assert_eq!(
                    hints.observe_in(*key, Mode::Normal, BindingScope::Global, &keymap),
                    HintEventResult::Forward
                );
                grammar
                    .translate(
                        InputEvent::Key(*key),
                        GrammarContext::new(Mode::Normal, BindingScope::Global, &keymap),
                    )
                    .unwrap();
            }
            assert!(grammar.pending_sequence().is_empty());
        }
        let binding = Binding::implemented(
            &[Mode::Normal],
            KeySequence::parse("F10 Esc").unwrap(),
            EditorCommand::MoveLeft,
        );
        assert!(keymap.with_plugin_bindings(vec![binding.clone()]).is_ok());
        assert!(
            crate::keymap::default_keymap()
                .with_plugin_bindings(vec![binding])
                .is_err()
        );
        let binding = Binding::implemented(
            &[Mode::Normal],
            KeySequence::parse("F10 F8").unwrap(),
            EditorCommand::MoveLeft,
        );
        assert!(keymap.with_plugin_bindings(vec![binding]).is_err());
    }

    #[test]
    fn input_policy_rollback_revalidates_dependent_edits_and_keeps_other_owners() {
        let base = Keymap::new(vec![Binding::implemented(
            &[Mode::Normal],
            KeySequence::parse("F9 x").unwrap(),
            EditorCommand::MoveLeft,
        )])
        .unwrap();
        let source = "ui: {grammar-prefix: {cancel: [x]}, hover: {accept: [F8]}}\nbind: {normal: {'F10 Esc': move-right}}";
        let compiled =
            crate::keymap::configured::compile(&serde_yaml::from_str(source).unwrap(), &base);
        assert_eq!(compiled.errors.len(), 2, "{:?}", compiled.errors);
        assert_eq!(
            compiled
                .keymap
                .ui()
                .label(Owner::GrammarPrefix, Action::Cancel),
            "Esc"
        );
        assert_eq!(
            compiled.keymap.ui().label(Owner::Hover, Action::Accept),
            "F8"
        );
        assert!(matches!(
            compiled
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("F10 Esc").unwrap()),
            Lookup::NoMatch
        ));
        crate::keymap::validate::assert_valid(&compiled.keymap);

        let source = "ui: {grammar-prefix: {cancel: [x]}}\nbind: {normal: {'F9 x': null, 'F10 Esc': move-right}}";
        let compiled =
            crate::keymap::configured::compile(&serde_yaml::from_str(source).unwrap(), &base);
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        crate::keymap::validate::assert_valid(&compiled.keymap);
        assert_eq!(
            compiled
                .keymap
                .ui()
                .label(Owner::GrammarPrefix, Action::Cancel),
            "x"
        );
    }

    #[test]
    fn overlapping_hint_and_prefix_controls_roll_back_together() {
        let source = "ui: {grammar-prefix: {cancel: [Ctrl-n]}}";
        let compiled = crate::keymap::configured::compile(
            &serde_yaml::from_str(source).unwrap(),
            crate::keymap::default_keymap(),
        );
        assert_eq!(compiled.errors.len(), 1);
        assert_eq!(
            compiled
                .keymap
                .ui()
                .label(Owner::GrammarPrefix, Action::Cancel),
            "Esc"
        );
        compile("ui: {grammar-prefix: {cancel: [Ctrl-n]}, key-hints: {next: []}}");
    }
    #[test]
    fn root_prefix_control_aliases_start_discovery_before_they_cancel() {
        let base = Keymap::new(vec![Binding::implemented(
            &[Mode::Normal],
            KeySequence::parse("F9 x").unwrap(),
            EditorCommand::MoveLeft,
        )])
        .unwrap();
        let compiled = crate::keymap::configured::compile(
            &serde_yaml::from_str("ui: {grammar-prefix: {cancel: [F9]}}").unwrap(),
            &base,
        );
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        let mut hints = KeyHintState::default();
        let mut grammar = RunyteGrammar::default();
        for length in [1, 0] {
            let key = KeyStroke::plain(KeyCode::Function(9));
            assert_eq!(
                hints.observe_in(key, Mode::Normal, BindingScope::Global, &compiled.keymap),
                HintEventResult::Forward
            );
            grammar
                .translate(
                    InputEvent::Key(key),
                    GrammarContext::new(Mode::Normal, BindingScope::Global, &compiled.keymap),
                )
                .unwrap();
            assert_eq!(hints.pending().len(), length);
            assert_eq!(grammar.pending_sequence().len(), length);
        }
    }

    #[test]
    fn conditional_arrows_cannot_mask_prefix_control_and_merge_has_its_own_policy() {
        let base = Keymap::new(vec![]).unwrap();
        let compiled = crate::keymap::configured::compile(
            &serde_yaml::from_str("ui: {grammar-prefix: {cancel: [Up]}}").unwrap(),
            &base,
        );
        assert_eq!(compiled.errors.len(), 1);
        compile(
            "ui: {key-hints: {next: [F8]}}\nscopes: {git-merge-review: {normal: {'F9 F8': git-review-next}}}",
        );
    }
    #[test]
    fn scoped_removals_release_policy_collisions_before_final_validation() {
        let base = Keymap::new(vec![Binding::implemented_in(
            &[Mode::Normal],
            BindingScope::Media,
            KeySequence::parse("F9 x").unwrap(),
            EditorCommand::MoveLeft,
        )])
        .unwrap();
        let source = "ui: {grammar-prefix: {cancel: [x]}}\nscopes: {media: {normal: {'F9 x': null, 'F10 Esc': move-right}}}";
        let compiled =
            crate::keymap::configured::compile(&serde_yaml::from_str(source).unwrap(), &base);
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        crate::keymap::validate::assert_valid(&compiled.keymap);
        assert_eq!(
            compiled
                .keymap
                .ui()
                .label(Owner::GrammarPrefix, Action::Cancel),
            "x"
        );
        assert!(matches!(
            compiled.keymap.lookup_in(
                Mode::Normal,
                BindingScope::Media,
                &KeySequence::parse("F10 Esc").unwrap()
            ),
            Lookup::Exact(_)
        ));
        compile("ui: {grammar-prefix: {cancel: []}}\nrebind: {'Space e': 'F9 Esc'}");
    }

    #[test]
    fn factory_hint_arrows_yield_but_authored_aliases_require_reachable_bindings() {
        let source = "bind: {normal: {'F9 Down': move-down}}";
        let keymap = compile(source);
        let mut hints = KeyHintState::default();
        hints.observe_in(
            KeyStroke::plain(KeyCode::Function(9)),
            Mode::Normal,
            BindingScope::Global,
            &keymap,
        );
        assert_eq!(
            hints.observe_in(
                KeyStroke::plain(KeyCode::Down),
                Mode::Normal,
                BindingScope::Global,
                &keymap
            ),
            HintEventResult::Forward
        );
        let source = "ui: {key-hints: {next: [Down]}}\nbind: {normal: {'F9 Down': move-down}}";
        let compiled = crate::keymap::configured::compile(
            &serde_yaml::from_str(source).unwrap(),
            crate::keymap::default_keymap(),
        );
        assert_eq!(compiled.errors.len(), 1, "{:?}", compiled.errors);
        assert!(
            !compiled
                .keymap
                .ui()
                .explicitly_configured(Owner::KeyHints, Action::Next)
        );
        crate::keymap::validate::assert_valid(&compiled.keymap);
    }
}
