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

#[test]
fn prompt_defaults_preserve_modifier_precedence_and_owner_specific_keys() {
    use Action::*;
    let bindings = Bindings::default();
    for owner in [
        Owner::CommandPrompt,
        Owner::TextPrompt,
        Owner::PathPrompt,
        Owner::ProgramPrompt,
    ] {
        for bits in 0..64 {
            let m = Modifiers::from_bits(bits).unwrap();
            for (code, ordinary, control, alt) in [
                (KeyCode::Enter, Some(Accept), None, None),
                (KeyCode::Escape, Some(Cancel), None, None),
                (KeyCode::Char('b'), None, Some(MoveLeft), Some(WordLeft)),
                (KeyCode::Char('f'), None, Some(MoveRight), Some(WordRight)),
                (KeyCode::Char('h'), None, Some(DeleteBackward), None),
                (KeyCode::Char('d'), None, Some(DeleteForward), None),
                (KeyCode::Char('n'), None, None, None),
                (KeyCode::Char('p'), None, None, None),
                (KeyCode::Char('s'), None, Some(Save), None),
                (KeyCode::Char('c'), None, Some(Cancel), None),
                (KeyCode::Backspace, Some(Backspace), None, None),
                (
                    KeyCode::Delete,
                    (owner != Owner::CommandPrompt).then_some(Delete),
                    None,
                    None,
                ),
                (
                    KeyCode::Home,
                    Some(if owner == Owner::CommandPrompt {
                        First
                    } else {
                        MoveStart
                    }),
                    None,
                    None,
                ),
                (
                    KeyCode::End,
                    Some(if owner == Owner::CommandPrompt {
                        Last
                    } else {
                        MoveEnd
                    }),
                    None,
                    None,
                ),
                (
                    KeyCode::Down,
                    (owner != Owner::TextPrompt).then_some(Next),
                    None,
                    None,
                ),
                (
                    KeyCode::Up,
                    (owner != Owner::TextPrompt).then_some(Previous),
                    None,
                    None,
                ),
                (
                    KeyCode::Tab,
                    match owner {
                        Owner::TextPrompt => None,
                        Owner::ProgramPrompt => Some(OpenActions),
                        _ => Some(Complete),
                    },
                    None,
                    None,
                ),
                (KeyCode::Char(' '), None, None, None),
            ] {
                let expected = if m.contains(Modifiers::CONTROL) {
                    control
                } else if m.contains(Modifiers::ALT) {
                    alt
                } else {
                    ordinary
                };
                assert_eq!(
                    bindings.lookup(owner, KeyStroke::new(code, m)),
                    expected,
                    "{owner:?} {code:?} {m:?}"
                );
            }
        }
    }
}

#[test]
fn prompt_literal_input_is_reserved_and_failed_owner_does_not_grant_accept_authority() {
    for key in ["a", "Space", "Super-a", "Ctrl-\u{1}"] {
        let mut keys = serde_yaml::Mapping::new();
        keys.insert(
            serde_yaml::Value::String("accept".into()),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::String(key.into())]),
        );
        let mut section = serde_yaml::Mapping::new();
        section.insert(
            serde_yaml::Value::String("command-prompt".into()),
            serde_yaml::Value::Mapping(keys),
        );
        let mut errors = vec![];
        let bindings = compile(
            Some(&serde_yaml::Value::Mapping(section)),
            KeyStroke::char(' '),
            &mut errors,
        );
        assert_eq!(errors.len(), 1, "{key:?}");
        assert!(!bindings.explicitly_configured(Owner::CommandPrompt, Action::Accept));
    }
    let (bindings, errors) = configured("command-prompt: {accept: [F9], cancel: []}");
    assert!(errors.is_empty());
    assert!(bindings.explicitly_configured(Owner::CommandPrompt, Action::Accept));
    assert!(!bindings.explicitly_configured(Owner::TextPrompt, Action::Accept));
    for key in ["Enter", "Shift-Enter", "Esc", "Ctrl-c"] {
        assert_eq!(
            bindings.lookup(Owner::CommandPrompt, KeyStroke::parse(key).unwrap()),
            None
        );
    }
}

#[test]
fn picker_default_named_keys_and_control_aliases_preserve_modifier_rules() {
    use Action::*;
    let map = Bindings::default();
    for owner in [Owner::FilePicker, Owner::Finder] {
        for bits in 0..64 {
            let m = Modifiers::from_bits(bits).unwrap();
            for (code, expected) in [
                (KeyCode::Enter, Some(Accept)),
                (KeyCode::Escape, Some(Cancel)),
                (KeyCode::Down, Some(Next)),
                (KeyCode::Up, Some(Previous)),
                (KeyCode::BackTab, Some(Previous)),
                (KeyCode::PageDown, Some(PageDown)),
                (KeyCode::PageUp, Some(PageUp)),
                (KeyCode::Backspace, Some(Backspace)),
                (KeyCode::Delete, Some(Delete)),
                (KeyCode::Left, Some(MoveLeft)),
                (
                    KeyCode::Char('h'),
                    m.contains(Modifiers::CONTROL).then_some(DeleteBackward),
                ),
                (
                    KeyCode::Char('s'),
                    m.contains(Modifiers::CONTROL).then_some(SplitHorizontal),
                ),
                (
                    KeyCode::Char('v'),
                    m.contains(Modifiers::CONTROL).then_some(SplitVertical),
                ),
                (
                    KeyCode::Tab,
                    if owner == Owner::FilePicker {
                        Some(Next)
                    } else {
                        m.is_empty().then_some(ToggleMode)
                    },
                ),
            ] {
                assert_eq!(
                    map.lookup(owner, KeyStroke::new(code, m)),
                    expected,
                    "{owner:?} {code:?} {m:?}"
                );
            }
        }
    }
}

#[test]
fn conditional_picker_leader_restores_the_configured_map_and_its_hints() {
    let source = serde_yaml::from_str("file-picker: {next: [F6]}").unwrap();
    let mut errors = vec![];
    let bindings = compile(Some(&source), KeyStroke::ctrl('n'), &mut errors);
    assert!(errors.is_empty());
    assert_eq!(
        bindings.lookup_with_leader(Owner::FilePicker, KeyStroke::ctrl('n'), true),
        Some(Action::Cancel)
    );
    assert_eq!(
        bindings.lookup_with_leader(Owner::FilePicker, KeyStroke::ctrl('n'), false),
        None,
        "removed next must not resurrect"
    );
    assert_eq!(
        bindings.lookup_with_leader(Owner::FilePicker, KeyStroke::parse("F6").unwrap(), false),
        Some(Action::Next)
    );
    assert_eq!(
        bindings.label_with_leader(Owner::FilePicker, Action::Cancel, false),
        "Esc/Ctrl-c"
    );
    assert_eq!(
        bindings.label_with_leader(Owner::FilePicker, Action::Next, false),
        "F6"
    );
    let bindings = compile(None, KeyStroke::plain(KeyCode::Down), &mut errors);
    assert_eq!(
        bindings.lookup_with_leader(Owner::Finder, KeyStroke::plain(KeyCode::Down), false),
        Some(Action::Next)
    );
    assert!(
        bindings
            .label_with_leader(Owner::Finder, Action::Next, false)
            .contains("Down")
    );
    assert!(
        !bindings
            .label_with_leader(Owner::Finder, Action::Next, true)
            .split('/')
            .any(|s| s == "Down")
    );
}

#[test]
fn list_menu_defaults_keep_back_cancel_and_all_modifier_aliases_distinct() {
    use Action::*;
    let defaults = Bindings::default();
    for owner in [
        Owner::BufferActions,
        Owner::TerminalActions,
        Owner::SessionActions,
    ] {
        for bits in 0..64 {
            let m = Modifiers::from_bits(bits).unwrap();
            for (code, action) in [
                (KeyCode::Tab, Some(Back)),
                (KeyCode::Escape, Some(Cancel)),
                (KeyCode::Enter, Some(Accept)),
                (KeyCode::Down, Some(Next)),
                (KeyCode::Up, Some(Previous)),
                (KeyCode::BackTab, Some(Previous)),
                (
                    KeyCode::Char('n'),
                    m.contains(Modifiers::CONTROL).then_some(Next),
                ),
                (
                    KeyCode::Char('p'),
                    m.contains(Modifiers::CONTROL).then_some(Previous),
                ),
                (
                    KeyCode::Char('c'),
                    m.contains(Modifiers::CONTROL).then_some(Cancel),
                ),
                (KeyCode::Char('j'), None),
            ] {
                assert_eq!(
                    defaults.lookup(owner, KeyStroke::new(code, m)),
                    action,
                    "{owner:?} {code:?} {m:?}"
                );
            }
        }
        let (map, errors) = configured(&format!(
            "{}: {{accept: [Ctrl-F8], back: [F9], cancel: []}}",
            owner.name()
        ));
        assert!(errors.is_empty(), "{errors:?}");
        for key in [
            "Enter",
            "Shift-Enter",
            "Tab",
            "Esc",
            "Ctrl-c",
            "Space",
            "F8",
        ] {
            assert_eq!(
                map.lookup(owner, KeyStroke::parse(key).unwrap()),
                None,
                "{owner:?} {key}"
            );
        }
        assert_eq!(
            map.lookup(owner, KeyStroke::parse("Ctrl-F8").unwrap()),
            Some(Accept)
        );
        assert_eq!(
            map.lookup(owner, KeyStroke::parse("F9").unwrap()),
            Some(Back)
        );
        let inherited = Bindings::with_leader(KeyStroke::plain(KeyCode::Tab));
        assert_eq!(
            inherited.lookup(owner, KeyStroke::plain(KeyCode::Tab)),
            Some(Cancel)
        );
    }
}

#[test]
fn session_menu_rejects_intercepted_control_text_and_dismissal_uses_surviving_actions() {
    let (map, errors) = configured("session-actions: {accept: [\"\\u0001\"]}");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        map.lookup(Owner::SessionActions, KeyStroke::plain(KeyCode::Enter)),
        Some(Action::Accept)
    );
    for owner in [
        Owner::BufferActions,
        Owner::TerminalActions,
        Owner::SessionActions,
    ] {
        for (source, expected) in [
            ("cancel: []", "Tab"),
            ("back: [], cancel: [F10]", "F10"),
            ("back: [], cancel: []", "unbound"),
        ] {
            let (map, errors) = configured(&format!("{}: {{{source}}}", owner.name()));
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(map.dismissal_label(owner), expected);
        }
    }
}

#[test]
fn plugin_manager_return_defaults_and_filter_reservations_match_transparent_ownership() {
    let map = Bindings::default();
    for bits in 0..64 {
        let modifiers = Modifiers::from_bits(bits).unwrap();
        for (code, action) in [
            (KeyCode::Escape, Some(Action::Cancel)),
            (
                KeyCode::Char('c'),
                modifiers
                    .contains(Modifiers::CONTROL)
                    .then_some(Action::Cancel),
            ),
            (KeyCode::Enter, None),
            (KeyCode::Tab, None),
        ] {
            assert_eq!(
                map.lookup(Owner::PluginManagerActions, KeyStroke::new(code, modifiers)),
                action
            );
        }
    }
    assert_eq!(
        map.lookup_with_leader(Owner::PluginManagerActions, KeyStroke::char(' '), true),
        Some(Action::Cancel)
    );
    assert_eq!(
        map.lookup_with_leader(Owner::PluginManagerActions, KeyStroke::char(' '), false),
        None
    );
    for spelling in ["a", "Meta-a", "\\u0001"] {
        let (_, errors) = configured(&format!(
            "plugin-manager-actions: {{cancel: [\"{spelling}\"]}}"
        ));
        assert_eq!(errors.len(), 1, "{spelling}");
    }
}

#[test]
fn session_navigation_defaults_preserve_modifiers_and_do_not_inherit_leader() {
    use Action::*;
    let map = Bindings::with_leader(KeyStroke::plain(KeyCode::Enter));
    for owner in [Owner::SessionDirectory, Owner::SessionInventory] {
        for bits in 0..64 {
            let m = Modifiers::from_bits(bits).unwrap();
            for (code, action) in [
                (KeyCode::Enter, Some(Accept)),
                (KeyCode::Escape, Some(Cancel)),
                (
                    KeyCode::Tab,
                    (owner == Owner::SessionDirectory).then_some(EnterDirectory),
                ),
                (KeyCode::Backspace, Some(Backspace)),
                (KeyCode::Down, Some(Next)),
                (KeyCode::Up, Some(Previous)),
                (KeyCode::BackTab, Some(Previous)),
                (KeyCode::PageUp, Some(PageUp)),
                (KeyCode::PageDown, Some(PageDown)),
                (KeyCode::Home, Some(First)),
                (KeyCode::End, Some(Last)),
                (
                    KeyCode::Char('c'),
                    m.contains(Modifiers::CONTROL).then_some(Cancel),
                ),
                (
                    KeyCode::Char('n'),
                    m.contains(Modifiers::CONTROL).then_some(Next),
                ),
                (
                    KeyCode::Char('p'),
                    m.contains(Modifiers::CONTROL).then_some(Previous),
                ),
                (
                    KeyCode::Char('u'),
                    m.contains(Modifiers::CONTROL).then_some(PageUp),
                ),
                (
                    KeyCode::Char('d'),
                    m.contains(Modifiers::CONTROL).then_some(PageDown),
                ),
            ] {
                assert_eq!(
                    map.lookup(owner, KeyStroke::new(code, m)),
                    action,
                    "{owner:?} {code:?} {m:?}"
                );
            }
        }
        for spelling in ["a", "Super-a", "Meta-a", "\\u0001"] {
            let (_, errors) =
                configured(&format!("{}: {{accept: [\"{spelling}\"]}}", owner.name()));
            assert_eq!(errors.len(), 1, "{owner:?} {spelling}");
        }
        let (map, errors) = configured(&format!(
            "{}: {{accept: [F8], cancel: [], next: [Ctrl-F6]}}",
            owner.name()
        ));
        assert!(errors.is_empty());
        for spelling in [
            "Enter",
            "Alt-Enter",
            "Esc",
            "Ctrl-c",
            "Space",
            "Down",
            "Ctrl-n",
            "F6",
        ] {
            assert_eq!(
                map.lookup(owner, KeyStroke::parse(spelling).unwrap()),
                None,
                "{owner:?} {spelling}"
            );
        }
        assert_eq!(
            map.lookup(owner, KeyStroke::parse("F8").unwrap()),
            Some(Accept)
        );
        assert_eq!(
            map.lookup(owner, KeyStroke::parse("Ctrl-F6").unwrap()),
            Some(Next)
        );
    }
}

#[test]
fn compact_ui_labels_use_effective_authored_aliases_and_preserve_unbound_state() {
    let (map, errors) = configured(
        "program-prompt: {previous: [F5, Ctrl-p], next: [], accept: [F8, F9]}\nprogram-actions: {cancel: [F10, F11, F12]}",
    );
    assert!(errors.is_empty());
    assert_eq!(
        map.compact_label(Owner::ProgramPrompt, Action::Previous, 1),
        "F5"
    );
    assert_eq!(
        map.compact_label(Owner::ProgramPrompt, Action::Next, 1),
        "unbound"
    );
    assert_eq!(
        map.compact_label(Owner::ProgramPrompt, Action::Accept, 1),
        "F8"
    );
    assert_eq!(
        map.compact_label(Owner::ProgramActions, Action::Cancel, 2),
        "F10/F11"
    );
    assert_eq!(
        map.label(Owner::ProgramActions, Action::Cancel),
        "F10/F11/F12"
    );
}

#[test]
fn simple_confirmation_defaults_preserve_every_modifier_and_stash_ctrl_c_exception() {
    let bindings = Bindings::default();
    for owner in [Owner::Confirmation, Owner::StashConfirmation] {
        for bits in 0..64 {
            let modifiers = Modifiers::from_bits(bits).unwrap();
            for (code, expected) in [
                (KeyCode::Enter, Some(Action::Accept)),
                (KeyCode::Escape, Some(Action::Cancel)),
                (
                    KeyCode::Char('c'),
                    (owner == Owner::Confirmation && modifiers.contains(Modifiers::CONTROL))
                        .then_some(Action::Cancel),
                ),
                (KeyCode::Tab, None),
            ] {
                assert_eq!(
                    bindings.lookup(owner, KeyStroke::new(code, modifiers)),
                    expected
                );
            }
        }
        let (bindings, errors) = configured(&format!(
            "{}: {{accept: [Esc], cancel: [F10]}}",
            owner.name()
        ));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            bindings.lookup(owner, KeyStroke::plain(KeyCode::Enter)),
            None
        );
        assert_eq!(
            bindings.lookup(owner, KeyStroke::plain(KeyCode::Escape)),
            Some(Action::Accept)
        );
        assert_eq!(bindings.lookup(owner, KeyStroke::char(' ')), None);
        assert_eq!(
            bindings.lookup(owner, KeyStroke::new(KeyCode::Escape, Modifiers::ALT)),
            None
        );
        let (bindings, errors) =
            configured(&format!("{}: {{accept: [], cancel: []}}", owner.name()));
        assert!(errors.is_empty());
        assert_eq!(
            bindings.lookup(owner, KeyStroke::plain(KeyCode::Escape)),
            None
        );
        assert_eq!(
            bindings.lookup(owner, KeyStroke::plain(KeyCode::Enter)),
            None
        );
        assert_eq!(bindings.lookup(owner, KeyStroke::char(' ')), None);
        assert!(owner.structural(KeyStroke::char('\n')));
    }
}
