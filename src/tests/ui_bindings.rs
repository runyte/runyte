// SPDX-License-Identifier: MPL-2.0
use super::*;

fn configured(source: &str) -> (Bindings, Vec<String>) {
    let section = serde_yaml::from_str(source).unwrap();
    let mut errors = Vec::new();
    (
        compile(Some(&section), KeyStroke::char(' '), &mut errors),
        errors,
    )
}

#[test]
fn simultaneous_swaps_replace_all_default_aliases_and_match_exact_modifiers() {
    let (bindings, errors) = configured("completion: {next: [Up], previous: [Down], cancel: []}");
    assert!(errors.is_empty());
    assert_eq!(
        bindings.lookup(Owner::Completion, KeyStroke::plain(KeyCode::Up)),
        Some(Action::Next)
    );
    assert_eq!(
        bindings.lookup(Owner::Completion, KeyStroke::plain(KeyCode::Down)),
        Some(Action::Previous)
    );
    assert_eq!(
        bindings.lookup(Owner::Completion, KeyStroke::ctrl('n')),
        None
    );
    assert_eq!(
        bindings.lookup(Owner::Completion, KeyStroke::ctrl('c')),
        None
    );
    assert_eq!(
        bindings.lookup(
            Owner::Completion,
            KeyStroke::new(KeyCode::Up, Modifiers::ALT)
        ),
        None
    );
}

#[test]
fn overlaps_with_legacy_modifier_policies_roll_back_the_owner() {
    for source in [
        "completion: {next: [Ctrl-Tab]}",
        "completion: {next: [Ctrl-Alt-c]}",
        "completion: {next: [F9], previous: [F9]}",
        "completion: {next: [Ctrl-n, Ctrl-Shift-n]}",
        "completion: {next: null}",
        "completion: {next: ['F9 j']}",
        "completion: {unknown: []}",
        "unknown: {}",
        "[]",
    ] {
        let (bindings, errors) = configured(source);
        assert_eq!(errors.len(), 1, "{source}");
        assert_eq!(
            bindings.lookup(Owner::Completion, KeyStroke::ctrl('n')),
            Some(Action::Next)
        );
        assert_eq!(
            bindings.lookup(Owner::Completion, KeyStroke::plain(KeyCode::Function(9))),
            None
        );
    }
}

#[test]
fn ui_configuration_remains_independent_of_editor_defaults_and_bad_pointer_rules() {
    let source = serde_yaml::from_str("defaults: false\nbind: {normal: {':': open-command-palette}}\npointer: invalid\nui: {completion: {accept: [F9]}}\n").unwrap();
    let result = crate::keymap::configured::compile(&source, crate::keymap::default_keymap());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(
        result
            .keymap
            .ui()
            .lookup(Owner::Completion, KeyStroke::plain(KeyCode::Function(9))),
        Some(Action::Accept)
    );
    assert_eq!(
        result
            .keymap
            .ui()
            .lookup(Owner::Completion, KeyStroke::plain(KeyCode::Tab)),
        None
    );
}

#[test]
fn menu_defaults_preserve_every_legacy_modifier_combination() {
    use Action::*;
    let bindings = Bindings::default();
    for owner in [Owner::ContextActions, Owner::PathActions, Owner::Path] {
        for bits in 0..64 {
            let modifiers = Modifiers::from_bits(bits).unwrap();
            for (code, without_control, with_control) in [
                (KeyCode::Escape, Some(Cancel), Some(Cancel)),
                (KeyCode::Tab, Some(Cancel), Some(Cancel)),
                (KeyCode::BackTab, Some(Previous), Some(Previous)),
                (KeyCode::Down, Some(Next), None),
                (KeyCode::Up, Some(Previous), None),
                (KeyCode::Char('j'), Some(Next), None),
                (KeyCode::Char('k'), Some(Previous), None),
                (KeyCode::Enter, Some(Accept), None),
                (KeyCode::Char('c'), None, Some(Cancel)),
                (KeyCode::Char('n'), None, None),
            ] {
                let mut expected = if modifiers.contains(Modifiers::CONTROL) {
                    with_control
                } else {
                    without_control
                };
                if owner == Owner::Path {
                    expected = match code {
                        KeyCode::Tab => Some(OpenActions),
                        KeyCode::Escape | KeyCode::Char('c') => expected,
                        _ => None,
                    };
                }
                assert_eq!(
                    bindings.lookup(owner, KeyStroke::new(code, modifiers)),
                    expected,
                    "{owner:?} {code:?} {modifiers:?}"
                );
            }
        }
    }
}

#[test]
fn replacing_cancel_restores_keys_covered_by_the_inherited_leader() {
    for leader in [
        KeyStroke::char('j'),
        KeyStroke::char('r'),
        KeyStroke::plain(KeyCode::Tab),
    ] {
        let mut errors = Vec::new();
        let defaults = compile(None, leader, &mut errors);
        assert_eq!(
            defaults.lookup(Owner::PathActions, leader),
            Some(Action::Cancel)
        );
        let section = serde_yaml::from_str("path-actions: {cancel: []}").unwrap();
        let map = compile(Some(&section), leader, &mut errors);
        assert!(errors.is_empty());
        let expected = if leader == KeyStroke::char('j') {
            Some(Action::Next)
        } else {
            None
        };
        assert_eq!(map.lookup(Owner::PathActions, leader), expected);
        assert_eq!(
            map.lookup(Owner::Path, leader),
            Some(Action::Cancel),
            "other owner keeps its default"
        );
        assert_eq!(map.label(Owner::PathActions, Action::Cancel), "unbound");
    }
}

#[test]
fn structural_choices_and_owner_conflicts_roll_back_only_that_owner() {
    for invalid in [
        "context-actions: {next: [r]}",
        "context-actions: {next: [R]}",
        "path-actions: {next: [Ctrl-r]}",
        "path-actions: {next: [Alt-s]}",
        "context-actions: {next: [Space]}",
        "path-actions: {next: [Ctrl-Tab]}",
        "path: {next: [F7]}",
    ] {
        let (map, errors) = configured(&format!("{invalid}\ncompletion: {{accept: [F9]}}"));
        assert_eq!(errors.len(), 1, "{invalid}");
        assert_eq!(
            map.lookup(Owner::Completion, KeyStroke::plain(KeyCode::Function(9))),
            Some(Action::Accept)
        );
        assert_eq!(
            map.lookup(Owner::ContextActions, KeyStroke::plain(KeyCode::Down)),
            Some(Action::Next)
        );
    }
    let (map, errors) = configured(
        "context-actions: {next: [Ctrl-r]}\npath: {open-actions: [F8]}\npath-actions: {next: [F9]}",
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        map.lookup(Owner::ContextActions, KeyStroke::ctrl('r')),
        Some(Action::Next)
    );
    assert_eq!(
        map.lookup(Owner::Path, KeyStroke::plain(KeyCode::Function(8))),
        Some(Action::OpenActions)
    );
    assert_eq!(
        map.lookup(Owner::PathActions, KeyStroke::plain(KeyCode::Function(9))),
        Some(Action::Next)
    );
}

#[test]
fn leader_masking_a_default_spelling_still_teaches_a_surviving_modifier_alias() {
    let mut errors = Vec::new();
    let map = compile(None, KeyStroke::plain(KeyCode::Tab), &mut errors);
    let label = map.label(Owner::Path, Action::OpenActions);
    assert_ne!(label, "unbound");
    assert_ne!(label, "Tab");
    assert_eq!(
        map.lookup(Owner::Path, KeyStroke::parse(&label).unwrap()),
        Some(Action::OpenActions)
    );
    assert_eq!(
        map.lookup(Owner::Path, KeyStroke::plain(KeyCode::Tab)),
        Some(Action::Cancel)
    );
}

#[test]
fn backward_tab_hint_keeps_its_distinct_key_identity() {
    let map = Bindings::default();
    for owner in [Owner::ContextActions, Owner::PathActions] {
        assert!(
            map.label(owner, Action::Previous)
                .split('/')
                .any(|label| label == "BackTab")
        );
        assert_eq!(
            map.lookup(owner, KeyStroke::plain(KeyCode::BackTab)),
            Some(Action::Previous)
        );
        assert_eq!(
            map.lookup(owner, KeyStroke::new(KeyCode::Tab, Modifiers::SHIFT)),
            Some(Action::Cancel)
        );
    }
    let mut errors = Vec::new();
    let map = compile(None, KeyStroke::plain(KeyCode::BackTab), &mut errors);
    assert!(
        !map.label(Owner::PathActions, Action::Previous)
            .split('/')
            .any(|label| label == "BackTab")
    );
    assert_eq!(
        map.lookup(Owner::PathActions, KeyStroke::plain(KeyCode::BackTab)),
        Some(Action::Cancel)
    );
}

#[test]
fn program_menu_defaults_preserve_wildcard_modifiers_and_replacements_remove_aliases() {
    let bindings = Bindings::default();
    for bits in 0..64 {
        let modifiers = Modifiers::from_bits(bits).unwrap();
        for (code, expected) in [
            (KeyCode::Down, Some(Action::Next)),
            (KeyCode::Up, Some(Action::Previous)),
            (KeyCode::Enter, Some(Action::Accept)),
            (KeyCode::Tab, Some(Action::Cancel)),
            (KeyCode::Escape, Some(Action::Cancel)),
            (KeyCode::BackTab, Some(Action::Previous)),
            (
                KeyCode::Char('n'),
                modifiers
                    .contains(Modifiers::CONTROL)
                    .then_some(Action::Next),
            ),
            (
                KeyCode::Char('p'),
                modifiers
                    .contains(Modifiers::CONTROL)
                    .then_some(Action::Previous),
            ),
            (
                KeyCode::Char('c'),
                modifiers
                    .contains(Modifiers::CONTROL)
                    .then_some(Action::Cancel),
            ),
            (KeyCode::Char('j'), None),
        ] {
            assert_eq!(
                bindings.lookup(Owner::ProgramActions, KeyStroke::new(code, modifiers)),
                expected
            );
        }
    }
    let (bindings, errors) =
        configured("program-actions: {next: [Up], previous: [Down], accept: [], cancel: []}");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        bindings.lookup(Owner::ProgramActions, KeyStroke::plain(KeyCode::Up)),
        Some(Action::Next)
    );
    for key in [
        KeyStroke::ctrl('n'),
        KeyStroke::char(' '),
        KeyStroke::plain(KeyCode::Enter),
        KeyStroke::plain(KeyCode::BackTab),
    ] {
        assert_eq!(bindings.lookup(Owner::ProgramActions, key), None);
    }
    assert_eq!(
        bindings.lookup(Owner::Path, KeyStroke::char(' ')),
        Some(Action::Cancel)
    );
}
