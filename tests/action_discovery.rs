// SPDX-License-Identifier: MPL-2.0
use runyte::{
    command::{Mode, discovery},
    config::Config,
    keymap::{BindingScope, KeySequence, Lookup, configured, default_keymap},
    test_support::TestRuntimeRoot,
};
use std::fs;

#[test]
fn provenance_tracks_flow_quoted_keys_fragments_aliases_and_rollback() {
    let root = TestRuntimeRoot::new("discovery-sources").unwrap();
    let main = root.path().join("config.yaml");
    fs::write(&main, "keys:\n  leader: Ctrl-x\n  bind:\n    normal: {F6: yank, F7: select-all}\n    directory: {F6: directory-open-parent}\n").unwrap();
    fs::create_dir(root.path().join("keybinds.d")).unwrap();
    let fragment = root.path().join("keybinds.d/last.yaml");
    fs::write(&fragment, "# later\nbind:\n  normal:\n    'F6': [select-all, yank]\n    F8: &action yank\n    F9: *action\n    F10: not-an-action\n").unwrap();
    let (config, _) = Config::load(Some(&main)).unwrap();
    for (path, file, line) in [
        (vec!["bind", "normal", "F6"], &fragment, 4),
        (vec!["bind", "normal", "F7"], &main, 4),
        (vec!["bind", "normal", "F9"], &fragment, 6),
        (vec!["bind", "directory", "F6"], &main, 5),
    ] {
        let path = path.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let location = config.key_sources.location(&path).unwrap();
        assert_eq!(&location.file, file);
        assert_eq!(location.line, Some(line), "{path:?}");
    }
    let compiled = configured::compile(config.keys.as_ref().unwrap(), default_keymap());
    assert!(compiled.errors.iter().any(|e| e.contains("F10")));
    let Lookup::Exact(binding) = compiled
        .keymap
        .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap())
    else {
        panic!()
    };
    assert_eq!(binding.source_paths, vec![vec!["bind", "normal", "F6"]]);
    assert!(matches!(
        compiled
            .keymap
            .lookup(Mode::Normal, &KeySequence::parse("F10").unwrap()),
        Lookup::NoMatch
    ));
    let Lookup::Exact(moved) = compiled
        .keymap
        .lookup(Mode::Normal, &KeySequence::parse("Ctrl-x f").unwrap())
    else {
        panic!()
    };
    assert!(moved.source_paths.contains(&vec!["leader".into()]));
}

#[test]
fn copied_yaml_preserves_order_arguments_descriptions_and_scope() {
    for (source, scope, key) in [
        (
            "bind: {normal: {F6: {steps: [{action: select-all}, {action: help, arg: key-actions}], description: Read help}}}",
            BindingScope::Global,
            "F6",
        ),
        (
            "bind: {normal: {F6: [select-all, yank]}}",
            BindingScope::Global,
            "F6",
        ),
        (
            "bind: {normal: {F6: {action: help, arg: key-actions}}}",
            BindingScope::Global,
            "F6",
        ),
        (
            "bind:\n  directory:\n    F6: directory-open-parent\n",
            BindingScope::Directory,
            "F6",
        ),
    ] {
        let section = serde_yaml::from_str(source).unwrap();
        let compiled = configured::compile(&section, default_keymap());
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        let sequence = KeySequence::parse(key).unwrap();
        let Lookup::Exact(binding) = compiled.keymap.lookup_in(Mode::Normal, scope, &sequence)
        else {
            panic!()
        };
        let yaml = discovery::binding_yaml(binding, Mode::Normal, Some(&section), &compiled.keymap)
            .unwrap();
        assert!(!yaml.contains("command:"), "{yaml}");
        assert!(!yaml.contains("argument:"), "{yaml}");
        let document: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let rebuilt = configured::compile(&document["keys"], default_keymap());
        assert!(rebuilt.errors.is_empty(), "{:?}", rebuilt.errors);
        let Lookup::Exact(copy) = rebuilt.keymap.lookup_in(Mode::Normal, scope, &sequence) else {
            panic!()
        };
        assert_eq!(binding.target, copy.target);
        assert_eq!(binding.program, copy.program);
        assert_eq!(binding.description, copy.description);
    }
}

#[test]
fn rejected_scoped_overrides_and_rebinds_never_claim_builtin_provenance() {
    let section =
        serde_yaml::from_str("rebind:\n  w: d\nbind:\n  directory:\n    F6: not-an-action\n")
            .unwrap();
    let compiled = configured::compile(&section, default_keymap());
    assert!(!compiled.errors.is_empty());
    let Lookup::Exact(binding) = compiled
        .keymap
        .lookup(Mode::Normal, &KeySequence::parse("w").unwrap())
    else {
        panic!()
    };
    assert!(binding.source_paths.is_empty());
}

#[test]
fn generated_guide_contracts_match_the_shared_renderer() {
    let guide = include_str!("../docs/user-guide/custom-key-bindings.md").replace("\r\n", "\n");
    let block = guide
        .split_once("<!-- action-reference:start -->\n")
        .unwrap()
        .1
        .split_once("<!-- action-reference:end -->")
        .unwrap()
        .0;
    assert_eq!(block, discovery::guide_reference());
}

#[test]
fn composed_keys_are_listed_for_every_member_without_claiming_plain_binding() {
    use runyte::command::{CommandId, EditorCommand};
    let section = serde_yaml::from_str(
        "bind: {normal: {F6: [select-all, yank], F7: {action: help, arg: key-actions}}}",
    )
    .unwrap();
    let compiled = configured::compile(&section, default_keymap());
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    for command in [EditorCommand::SelectAll, EditorCommand::Yank] {
        let id = CommandId::Editor(command);
        assert!(
            !discovery::keys(id, &compiled.keymap, Mode::Normal, BindingScope::Global)
                .contains(&"F6".into())
        );
        let details = discovery::details(id, &compiled.keymap, Mode::Normal, BindingScope::Global);
        assert!(
            details.contains("Composed/argument binding: F6"),
            "{details}"
        );
    }
}

#[test]
fn browser_projection_bounds_many_plain_and_composed_bindings() {
    use runyte::{
        command::{CommandId, EditorCommand},
        keymap::{Binding, Keymap, actions::Action},
    };
    let mut bindings = Vec::new();
    for i in 1..=80 {
        let mut binding = Binding::implemented(
            &[Mode::Normal],
            KeySequence::parse(&format!("F1 F{}", i)).unwrap(),
            EditorCommand::SelectAll,
        );
        if i > 40 {
            binding.program = runyte::keymap::program::Program::new(vec![
                Action::Editor(EditorCommand::SelectAll),
                Action::Editor(EditorCommand::Yank),
            ])
            .unwrap();
            binding.configured_actions = true;
        }
        bindings.push(binding);
    }
    let map = Keymap::new(bindings).unwrap();
    let details = discovery::details(
        CommandId::Editor(EditorCommand::SelectAll),
        &map,
        Mode::Normal,
        BindingScope::Global,
    );
    assert!(details.contains("8 more"), "{details}");
    assert!(
        details.contains("36 more composed/argument bindings"),
        "{details}"
    );
    assert_eq!(details.matches("Composed/argument binding:").count(), 4);
    assert!(details.len() < 8000);
}

#[test]
fn discovery_descriptors_include_the_result_page_mode_transition() {
    use runyte::command::{ColonCommand, descriptor::ActionEffect};
    for command in [ColonCommand::DescribeKey, ColonCommand::ActionBrowser] {
        assert!(
            command
                .action_policy()
                .effects
                .contains(&ActionEffect::Mode)
        );
    }
}

#[test]
fn discovery_defaults_follow_the_help_prefix_and_leader_remapping() {
    use runyte::command::{ColonCommand, CommandId, EditorCommand};
    for mode in [Mode::Normal, Mode::Select] {
        for (sequence, id) in [
            ("Space h k", CommandId::Colon(ColonCommand::DescribeKey)),
            ("Space h a", CommandId::Colon(ColonCommand::ActionBrowser)),
            ("Space ?", CommandId::Editor(EditorCommand::ShowHelp)),
        ] {
            let Lookup::Exact(binding) =
                default_keymap().lookup(mode, &KeySequence::parse(sequence).unwrap())
            else {
                panic!("missing {sequence}");
            };
            assert_eq!(binding.target.id(), id);
        }
    }
    assert_eq!(
        ColonCommand::ActionBrowser.descriptor().metadata.name,
        "browse-actions"
    );
    let help = KeySequence::parse("Space h").unwrap();
    assert!(
        default_keymap()
            .namespaces()
            .iter()
            .any(|ns| ns.sequence == help && ns.description == "Help")
    );
    let section = serde_yaml::from_str("leader: Ctrl-x").unwrap();
    let compiled = configured::compile(&section, default_keymap());
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    assert!(matches!(
        compiled
            .keymap
            .lookup(Mode::Normal, &KeySequence::parse("Ctrl-x h k").unwrap()),
        Lookup::Exact(_)
    ));
}

#[test]
fn discovery_uses_shared_readable_contracts_and_grouped_reference() {
    use discovery::display;
    use runyte::{
        command::{CommandId, EditorCommand},
        keymap::BindingAvailability,
    };
    let details = discovery::details(
        CommandId::Editor(EditorCommand::MoveLeft),
        default_keymap(),
        Mode::Normal,
        BindingScope::Global,
    );
    assert!(details.contains("Count: repeats"), "{details}");
    assert!(details.contains("Character operands: none"), "{details}");
    assert!(
        details.contains("Sequence position: can be followed by another action"),
        "{details}"
    );
    assert_eq!(
        display::binding_availability(BindingAvailability::Planned("later")),
        "planned: later"
    );
    assert_eq!(
        display::binding_availability(BindingAvailability::Unsupported("platform")),
        "unsupported: platform"
    );
    let reference = runyte::keymap::actions::reference();
    for heading in [
        "## Editing actions",
        "## Movement actions",
        "## Selection actions",
    ] {
        assert!(reference.contains(heading));
    }
    for internal in [
        "[Selection",
        "| Repeat |",
        "| Reject |",
        "| Zero |",
        "| One |",
        "| Two |",
        "ReplacesPrompt",
        "Planned(",
    ] {
        assert!(!reference.contains(internal), "{internal}");
    }
    assert!(reference.contains("| repeats | none |"));
}

#[test]
fn discovery_explains_effect_and_argument_continuation() {
    use runyte::command::{CommandId, EditorCommand as C};
    for (command, expected) in [
        (C::FindNextChar, "can be followed by another action"),
        (C::EnterInsertMode, "can be followed by another action"),
        (
            C::RenameTerminal,
            "last unless an argument supplies the prompt input",
        ),
        (C::ReplayDefaultMacro, "must be the last action"),
    ] {
        let details = discovery::details(
            CommandId::Editor(command),
            default_keymap(),
            Mode::Normal,
            BindingScope::Global,
        );
        assert!(details.contains(expected), "{details}");
    }
}
