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
    fs::write(&main, "# main\nkeys:\n  leader: Ctrl-x\n  bind: {normal: {F6: yank, F7: select-all}}\n  scopes: {directory: {normal: {F6: open-parent-directory}}}\n").unwrap();
    fs::create_dir(root.path().join("keybinds.d")).unwrap();
    let fragment = root.path().join("keybinds.d/last.yaml");
    fs::write(&fragment, "# later\nbind:\n  normal:\n    'F6': [select-all, yank]\n    F8: &action yank\n    F9: *action\n    F10: not-an-action\n").unwrap();
    let (config, _) = Config::load(Some(&main)).unwrap();
    for (path, file, line) in [
        (vec!["bind", "normal", "F6"], &fragment, 4),
        (vec!["bind", "normal", "F7"], &main, 4),
        (vec!["bind", "normal", "F9"], &fragment, 6),
        (vec!["scopes", "directory", "normal", "F6"], &main, 5),
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
            "bind: {normal: {F6: [select-all, yank]}}",
            BindingScope::Global,
            "F6",
        ),
        (
            "bind: {normal: {F6: {command: help, argument: key-actions}}}",
            BindingScope::Global,
            "F6",
        ),
        (
            "scopes: {directory: {normal: {F6: open-parent-directory}}}",
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
        let document: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let rebuilt = configured::compile(&document["keys"], default_keymap());
        assert!(rebuilt.errors.is_empty(), "{:?}", rebuilt.errors);
        let Lookup::Exact(copy) = rebuilt.keymap.lookup_in(Mode::Normal, scope, &sequence) else {
            panic!()
        };
        assert_eq!(binding.target, copy.target);
        assert_eq!(binding.actions, copy.actions);
        assert_eq!(binding.description, copy.description);
    }
}

#[test]
fn rejected_scoped_overrides_and_rebinds_never_claim_builtin_provenance() {
    let section =
        serde_yaml::from_str("rebind: {w: d}\nscopes: {directory: {normal: {F6: not-an-action}}}")
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
        "bind: {normal: {F6: [select-all, yank], F7: {command: help, argument: key-actions}}}",
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
            binding.actions = vec![
                Action::Editor(EditorCommand::SelectAll),
                Action::Editor(EditorCommand::Yank),
            ];
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
