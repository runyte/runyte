// SPDX-License-Identifier: MPL-2.0
use runyte::{
    command::{self, Mode, REMOVED_ACTION_PROGRAMS},
    keymap::{BindingScope as S, KeySequence, Lookup, configured, default_keymap},
};

#[test]
fn every_removed_program_hint_parses_and_loads_in_its_former_contexts() {
    let expected = [
        "enter-normal-mode",
        "enter-insert-mode",
        "enter-replace-mode",
        "enter-select-mode",
        "yank-collapse-normal-mode",
        "yank-line-collapse-normal-mode",
        "directory-tree-left",
        "directory-tree-right",
        "yank-delete-selection",
        "yank-change-selection",
        "change-selection",
        "search-selection",
    ];
    assert_eq!(REMOVED_ACTION_PROGRAMS.len(), expected.len());
    let map = default_keymap();
    let mut assignments = std::collections::BTreeMap::<
        String,
        std::collections::BTreeMap<String, serde_yaml::Value>,
    >::new();
    let mut cases = Vec::new();
    for (index, name) in expected.into_iter().enumerate() {
        let key = format!("F6 {}", char::from(b'a' + index as u8));
        assert!(command::resolve_command(name).is_none());
        let hint = command::removed_action_replacement(name).unwrap();
        assert!(!hint.contains("preset-only"));
        let program = hint.strip_prefix("use ").unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(program).unwrap();
        assert!(value.is_sequence());
        let mut count = 0;
        for &scope in S::ALL {
            if matches!(
                scope,
                S::SessionManager | S::GitMergeReview | S::GitMergeDetail | S::GitMergeInput
            ) {
                continue;
            }
            let tree = name.starts_with("directory-tree-");
            if tree != (scope == S::DirectoryTree) {
                continue;
            }
            if scope == S::Media && !matches!(name, "enter-normal-mode" | "enter-select-mode") {
                continue;
            }
            if scope == S::Terminal
                && !matches!(
                    name,
                    "enter-normal-mode"
                        | "enter-select-mode"
                        | "yank-collapse-normal-mode"
                        | "yank-line-collapse-normal-mode"
                )
            {
                continue;
            }
            for mode in [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace] {
                if mode != Mode::Normal
                    && (tree || name != "enter-normal-mode" && mode != Mode::Select)
                {
                    continue;
                }
                let Some(id) = map.named_mode_for(mode, scope) else {
                    continue;
                };
                let mode_name = map.mode_graph().unwrap().get(id).unwrap().name();
                assignments
                    .entry(mode_name.into())
                    .or_default()
                    .insert(key.clone(), value.clone());
                cases.push((name, scope, mode, key.clone()));
                count += 1;
            }
        }
        assert!(count > 0, "untested hint {name}");
    }
    let source =
        serde_yaml::to_value(std::collections::BTreeMap::from([("bind", assignments)])).unwrap();
    let result = configured::compile(&source, map);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    for (name, scope, mode, key) in cases {
        assert!(
            matches!(
                result
                    .keymap
                    .lookup_in(mode, scope, &KeySequence::parse(&key).unwrap()),
                Lookup::Exact(_)
            ),
            "{name} {scope:?} {mode:?}"
        );
    }
}

#[test]
fn removed_mode_actions_are_rejected_without_installing_any_prefix() {
    for name in [
        "enter-normal-mode",
        "enter-insert-mode",
        "enter-replace-mode",
        "enter-select-mode",
    ] {
        let diagnostic = command::parse_named_command(name, None)
            .unwrap_err()
            .to_string();
        assert!(diagnostic.contains("was removed in 0.5; use ["));
        for binding in [
            name.to_owned(),
            format!("[move-right, {name}]"),
            format!("{{action: {name}}}"),
            format!("{{action: {name}, arg: normal}}"),
            format!("{{if: read-only, then: [{name}], else: [yank]}}"),
        ] {
            let result = configured::compile(
                &serde_yaml::from_str(&format!("bind: {{normal: {{F6: {binding}}}}}")).unwrap(),
                default_keymap(),
            );
            assert!(
                result.errors.iter().any(|e| e.contains(&diagnostic)),
                "{:?}",
                result.errors
            );
            assert!(matches!(
                result
                    .keymap
                    .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()),
                Lookup::NoMatch
            ));
        }
    }
}
