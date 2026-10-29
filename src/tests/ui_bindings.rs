// SPDX-License-Identifier: MPL-2.0
use super::*;

fn configured(source: &str) -> (Bindings, Vec<String>) {
    let section = serde_yaml::from_str(source).unwrap();
    let mut errors = Vec::new();
    (compile(Some(&section), &mut errors), errors)
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
