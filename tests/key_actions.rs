// SPDX-License-Identifier: MPL-2.0

use runyte::{
    command::{EditorCommand as C, Mode},
    keymap::{BindingTarget, KeySequence, Lookup, actions, configured, default_keymap, validate},
};

fn compile(source: &str) -> configured::CompiledKeymap {
    configured::compile(&serde_yaml::from_str(source).unwrap(), default_keymap())
}

#[test]
fn action_bindings_override_after_remapping_and_keep_modes_independent() {
    let compiled = compile(
        "leader: Ctrl-x\nrebind:\n  Space g: Leader G\nbind:\n  normal:\n    X: select-line-up\n    Leader G z: [select-all, yank]\n    F7: { command: pipe, argument: sort }\n  insert:\n    F8: insert-newline\n  replace:\n    F8: insert-other-indent\n",
    );
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    validate::assert_valid(&compiled.keymap);
    for (mode, spelling, target) in [
        (Mode::Normal, "X", C::SelectLineUp),
        (Mode::Select, "X", C::ExtendLineAbove),
        (Mode::Insert, "F8", C::InsertNewline),
        (Mode::Replace, "F8", C::InsertLiteralTab),
    ] {
        let Lookup::Exact(binding) = compiled
            .keymap
            .lookup(mode, &KeySequence::parse(spelling).unwrap())
        else {
            panic!("missing {spelling}")
        };
        assert_eq!(binding.target, BindingTarget::Editor(target));
    }
    let Lookup::Exact(binding) = compiled
        .keymap
        .lookup(Mode::Normal, &KeySequence::parse("Ctrl-x G z").unwrap())
    else {
        panic!("missing sequence")
    };
    assert_eq!(binding.actions.len(), 2);
    assert_eq!(binding.description, "select-all → yank");
}

#[test]
fn action_binding_conflicts_roll_back_without_losing_valid_remaps() {
    let compiled = compile(
        "leader: Ctrl-x\nbind:\n  normal:\n    F1: yank\n    F1 x: select-all\n    F2: [search, yank]\n    F3: not-an-action\n    F4: insert-newline\n    F5: commit-undo-checkpoint\n    F6: {command: pipe}\n    Leader g: yank\n    F8: yank\n",
    );
    assert!(compiled.errors.len() >= 7, "{:?}", compiled.errors);
    validate::assert_valid(&compiled.keymap);
    assert_eq!(compiled.keymap.leader().label(), "Ctrl-x");
    assert!(matches!(
        compiled
            .keymap
            .lookup(Mode::Normal, &KeySequence::parse("F8").unwrap()),
        Lookup::Exact(_)
    ));
    assert!(matches!(
        compiled
            .keymap
            .lookup(Mode::Normal, &KeySequence::parse("F1").unwrap()),
        Lookup::NoMatch
    ));
}

#[test]
fn action_unbinding_and_replacement_remove_stale_aliases() {
    let compiled = compile(
        "bind:\n  normal:\n    ',': null\n    Space s c: [select-all, yank]\n  select:\n    ',': select-all\n",
    );
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    validate::assert_valid(&compiled.keymap);
    assert!(matches!(
        compiled
            .keymap
            .lookup(Mode::Normal, &KeySequence::parse(",").unwrap()),
        Lookup::NoMatch
    ));
    assert!(
        compiled
            .keymap
            .bindings()
            .iter()
            .filter(|binding| binding.target == BindingTarget::Editor(C::KeepPrimarySelection))
            .all(|binding| binding.alias.is_none())
    );
}

#[test]
fn action_configuration_rejects_malformed_and_oversized_values_nonfatally() {
    for source in [
        "bind: nope",
        "bind: {terminal: {F1: yank}}",
        "bind: {normal: false}",
        "bind: {normal: {1: yank}}",
        "bind: {normal: {F1: []}}",
        "bind: {normal: {F1: {command: yank, surprise: true}}}",
        "bind: {normal: {'1': yank}}",
        "bind: {normal: {'F1 Esc': yank}}",
        "bind: {normal: {'F1 a b c d e f g h': yank}}",
        "bind: {normal: {F1: {command: pipe, argument: 2}}}",
    ] {
        let compiled = compile(source);
        assert!(!compiled.errors.is_empty(), "{source}");
        validate::assert_valid(&compiled.keymap);
    }
    let actions = std::iter::repeat_n("yank", 17)
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        !compile(&format!("bind: {{normal: {{F1: [{actions}]}}}}"))
            .errors
            .is_empty()
    );
    let rules = (0..257)
        .map(|i| format!("    F1 a{i}: yank\n"))
        .collect::<String>();
    assert!(
        compile(&format!("bind:\n  normal:\n{rules}"))
            .errors
            .iter()
            .any(|error| error.contains("256"))
    );
}

#[test]
fn action_configuration_is_order_independent_and_reference_uses_eligibility() {
    let a = compile("bind: {normal: {F1: yank, F2: select-all, X: null}}");
    let b = compile("bind: {normal: {X: null, F2: select-all, F1: yank}}");
    assert_eq!(a.keymap.bindings(), b.keymap.bindings());
    let reference = actions::reference();
    assert!(reference.contains("`extend-line-above`"));
    assert!(reference.contains("`select-line-up`"));
    assert!(reference.contains("`pipe`"));
    assert!(!reference.contains("`commit-undo-checkpoint`"));
    assert!(actions::editor_modes(C::ListNext).is_empty());
}

#[test]
fn action_binding_hints_keep_sequences_and_teaching_tracks_each_mode() {
    use runyte::key_hints::{KeyHintState, key_hint_description};
    use runyte::keymap::Key;
    let compiled = compile(
        "bind:\n  normal:\n    F1 a: [select-all, yank]\n    Space e: yank\n    F2: open-explorer\n",
    );
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    let mut hints = KeyHintState::default();
    hints.observe(Key::parse("F1").unwrap(), Mode::Normal, &compiled.keymap);
    let rows = hints.rows(&compiled.keymap, Mode::Normal);
    assert_eq!(rows.len(), 1);
    assert!(key_hint_description(&rows[0]).contains("select-all → yank"));
    let prose = runyte::key_spelling::resolve("{binding:Space e}", &compiled.keymap).unwrap();
    assert!(prose.text.contains("F2 (NOR)"), "{}", prose.text);
    assert!(prose.text.contains("Space e (SEL)"), "{}", prose.text);
    let unbound = compile("bind:\n  normal:\n    Space e: null\n  select:\n    Space e: null\n");
    let prose = runyte::key_spelling::resolve("{binding:Space e}", &unbound.keymap).unwrap();
    assert_eq!(prose.text, "[unbound: open-explorer]");
}

#[test]
fn action_bindings_preserve_terminal_input_and_its_window_commands() {
    use runyte::keymap::BindingScope;
    let compiled = compile(
        "window: Ctrl-a\nbind:\n  insert:\n    Window h: insert-newline\n    Window F1: insert-newline\n    Window l: null\n    F2: insert-indent\n",
    );
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    validate::assert_valid(&compiled.keymap);
    for (sequence, expected) in [
        ("Ctrl-a h", C::FocusWindowLeft),
        ("Ctrl-a l", C::FocusWindowRight),
    ] {
        let sequence = KeySequence::parse(sequence).unwrap();
        let Lookup::Exact(binding) =
            compiled
                .keymap
                .lookup_in(Mode::Insert, BindingScope::Terminal, &sequence)
        else {
            panic!("missing terminal command")
        };
        assert_eq!(binding.target, BindingTarget::Editor(expected));
        assert!(binding.actions.is_empty());
    }
    for sequence in ["Ctrl-a F1", "F2"] {
        let sequence = KeySequence::parse(sequence).unwrap();
        assert!(matches!(
            compiled
                .keymap
                .lookup_in(Mode::Insert, BindingScope::Terminal, &sequence),
            Lookup::NoMatch
        ));
        assert!(matches!(
            compiled.keymap.lookup(Mode::Insert, &sequence),
            Lookup::Exact(_)
        ));
        assert!(
            !compiled
                .keymap
                .bindings_for_scope(Mode::Insert, BindingScope::Terminal)
                .any(|binding| binding.sequence == sequence)
        );
    }
}
