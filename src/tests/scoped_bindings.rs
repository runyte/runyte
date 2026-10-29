// SPDX-License-Identifier: MPL-2.0
use crate::{
    command::{EditorCommand, Mode},
    keymap::{self, Binding, BindingScope as S, BindingTarget, KeySequence, Keymap, Lookup},
};

fn configured(source: &str) -> std::sync::Arc<Keymap> {
    let result = keymap::configured::compile(
        &serde_yaml::from_str(source).unwrap(),
        keymap::default_keymap(),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result.keymap
}
fn target(map: &Keymap, mode: Mode, scope: S, keys: &str) -> Option<BindingTarget> {
    match map.lookup_in(mode, scope, &KeySequence::parse(keys).unwrap()) {
        Lookup::Exact(binding) => Some(binding.target),
        Lookup::NoMatch => None,
        other => panic!("unexpected lookup {other:?}"),
    }
}

#[test]
fn exact_removals_and_overrides_are_local_and_survive_plugin_rebuilds() {
    let map = configured("scopes: {markdown: {normal: {j: move-up, k: null}}}");
    assert_eq!(
        target(&map, Mode::Normal, S::Markdown, "j"),
        Some(EditorCommand::MoveUp.into())
    );
    assert_eq!(target(&map, Mode::Normal, S::Markdown, "k"), None);
    assert_eq!(
        target(&map, Mode::Normal, S::Global, "j"),
        Some(EditorCommand::MoveDown.into())
    );
    assert!(target(&map, Mode::Select, S::Markdown, "k").is_some());
    let rebuilt = map
        .with_plugin_bindings(vec![Binding::implemented(
            &[Mode::Normal],
            KeySequence::parse("F12").unwrap(),
            BindingTarget::Plugin(1),
        )])
        .unwrap();
    assert_eq!(target(&rebuilt, Mode::Normal, S::Markdown, "k"), None);
    assert_eq!(
        target(&rebuilt, Mode::Normal, S::Markdown, "F12"),
        Some(BindingTarget::Plugin(1))
    );
    assert!(
        !map.bindings_for_scope(Mode::Normal, S::Markdown)
            .any(|binding| binding.sequence.to_string() == "k")
    );
}

#[test]
fn tombstone_only_scope_does_not_fall_back_to_global_index() {
    let map = configured("scopes: {diff: {insert: {Backspace: null}}}");
    assert!(
        target(
            keymap::default_keymap(),
            Mode::Insert,
            S::Global,
            "Backspace"
        )
        .is_some()
    );
    assert_eq!(target(&map, Mode::Insert, S::Diff, "Backspace"), None);
}

#[test]
fn child_removal_and_assignment_resolve_conflicting_parents() {
    for decision in ["null", "move-right"] {
        let map = configured(&format!(
            "scopes:\n  markdown: {{normal: {{F9: move-up}}}}\n  git-conflict-file: {{normal: {{F9: null}}}}\n  git-conflict-markdown: {{normal: {{F9: {decision}}}}}"
        ));
        assert_eq!(
            target(&map, Mode::Normal, S::GitConflictMarkdown, "F9"),
            (decision != "null").then_some(EditorCommand::MoveRight.into())
        );
        assert_eq!(
            target(&map, Mode::Normal, S::Markdown, "F9"),
            Some(EditorCommand::MoveUp.into())
        );
    }
}

#[test]
fn invalid_graph_rolls_back_scopes_but_keeps_other_sections() {
    for scopes in [
        "{markdown: {normal: {F9: move-up}}, git-conflict-file: {normal: {F9: null}}}",
        "{markdown: {normal: {g: move-up}}}",
        "{markdown: {normal: {'1': move-up}}}",
        "{markdown: {normal: {'F9 Esc': move-up}}}",
        "{markdown: {normal: {F9: commit-undo-checkpoint}}}",
        "{markdown: {normal: {F9: list-next}}}",
        "{session-manager: {list: {'F9 j': list-next}}}",
        "{unknown: {normal: {F9: move-up}}}",
        "{markdown: {normal: {Ctrl-j: move-up, Ctrl-Shift-j: move-down}}}",
        "{global: {normal: {j: move-up}}}",
        "[]",
    ] {
        let result = keymap::configured::compile(&serde_yaml::from_str(&format!("scopes: {scopes}\nbind: {{normal: {{F8: move-up}}}}\nui: {{completion: {{accept: [F7]}}}}")).unwrap(), keymap::default_keymap());
        assert_eq!(result.errors.len(), 1, "{scopes}: {:?}", result.errors);
        assert!(result.errors[0].contains("all scoped overrides rolled back"));
        assert_eq!(
            target(&result.keymap, Mode::Normal, S::Markdown, "F9"),
            None
        );
        assert_eq!(
            target(&result.keymap, Mode::Normal, S::Global, "F8"),
            Some(EditorCommand::MoveUp.into())
        );
        assert_eq!(
            result
                .keymap
                .ui()
                .label(keymap::ui::Owner::Completion, keymap::ui::Action::Accept),
            "F7"
        );
    }
}

#[test]
fn list_bindings_can_swap_remove_and_override_inherited_keys() {
    let map = configured(
        "scopes: {global: {list: {Ctrl-n: list-previous, Ctrl-p: list-next}}, session-manager: {list: {Ctrl-n: null, F9: list-next}}}",
    );
    assert_eq!(
        target(&map, Mode::List, S::Global, "Ctrl-n"),
        Some(EditorCommand::ListPrevious.into())
    );
    assert_eq!(target(&map, Mode::List, S::SessionManager, "Ctrl-n"), None);
    assert_eq!(
        target(&map, Mode::List, S::SessionManager, "F9"),
        Some(EditorCommand::ListNext.into())
    );
    keymap::validate::assert_valid(&map);
}

#[test]
fn suppressing_an_alias_hides_it_only_in_affected_hints() {
    let default = keymap::default_keymap();
    let binding = default
        .bindings_for_scope(Mode::Normal, S::Markdown)
        .find(|binding| {
            binding.alias.is_some() && binding.alias_modes.is_none() && binding.sequence.len() > 1
        })
        .unwrap();
    let alias = binding.alias.as_ref().unwrap();
    let map = configured(&format!(
        "scopes:\n  markdown:\n    normal:\n      '{alias}': null\n"
    ));
    let prefix = KeySequence::new(
        binding.sequence.as_slice()[..binding.sequence.len() - 1]
            .iter()
            .copied(),
    );
    let mut hints = crate::key_hints::KeyHintState::default();
    // Use the public observation path so discovery sees the same owner as dispatch.
    for key in prefix.as_slice() {
        hints.observe_in(*key, Mode::Normal, S::Markdown, &map);
    }
    let row = hints
        .rows_in(&map, Mode::Normal, S::Markdown)
        .into_iter()
        .find(|row| row.sequence == binding.sequence)
        .unwrap();
    assert!(row.alias.is_none());
    assert!(
        hints
            .rows_in(&map, Mode::Normal, S::Global)
            .iter()
            .any(|row| row.sequence == binding.sequence && row.alias.is_some())
    );
}

#[test]
fn structural_and_unmigrated_handlers_cannot_be_shadowed_or_silently_unbound() {
    for (scope, mode, key) in [
        ("global", "list", "1"),
        ("session-manager", "list", "Meta-2"),
        ("global", "list", "Enter"),
        ("global", "list", "Ctrl-Tab"),
        ("global", "list", "Backspace"),
        ("global", "list", "Space"),
        ("markdown", "normal", "Tab"),
        ("git-merge-input", "normal", "Left"),
        ("git-merge-input", "normal", "a"),
    ] {
        let section = serde_yaml::from_str(&format!(
            "scopes: {{{scope}: {{{mode}: {{'{key}': null}}}}}}"
        ))
        .unwrap();
        let result = keymap::configured::compile(&section, keymap::default_keymap());
        assert_eq!(result.errors.len(), 1, "{scope} {mode} {key}");
    }
}

#[test]
fn removing_last_namespace_descendants_hides_only_that_scopes_namespace() {
    let default = keymap::default_keymap();
    let namespace = default
        .namespaces_for_scope(Mode::Normal, S::Markdown)
        .find(|namespace| namespace.sequence.to_string() == "g")
        .unwrap();
    let removals = default
        .bindings_for_scope(Mode::Normal, S::Markdown)
        .filter(|binding| binding.sequence.starts_with(&namespace.sequence))
        .map(|binding| format!("      '{}': null\n", binding.sequence))
        .collect::<String>();
    let map = configured(&format!("scopes:\n  markdown:\n    normal:\n{removals}"));
    assert!(
        !map.namespaces_for_scope(Mode::Normal, S::Markdown)
            .any(|n| n.sequence == namespace.sequence)
    );
    assert!(
        map.namespaces_for_scope(Mode::Normal, S::Global)
            .any(|n| n.sequence == namespace.sequence)
    );
}

#[test]
fn scoped_fragments_export_and_reload_preserve_exact_removals() {
    let mut config = crate::config::Config::default();
    config.merge_keys_source("scopes:\n  markdown:\n    normal: {j: move-up, k: move-down}\n  global:\n    list: {F9: list-next}\n").unwrap();
    config
        .merge_keys_source(
            "scopes:\n  markdown:\n    normal: {k: null}\n    select: {F8: move-up}\n",
        )
        .unwrap();
    let files = config
        .to_files(std::path::Path::new("config.yaml"))
        .unwrap();
    let mut written = crate::config::Config::default();
    for (path, text) in files {
        if path.starts_with(crate::config::KEYBINDS_DIRECTORY) {
            written.merge_keys_source(&text).unwrap();
        }
    }
    let result =
        keymap::configured::compile(written.keys.as_ref().unwrap(), keymap::default_keymap());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        target(&result.keymap, Mode::Normal, S::Markdown, "j"),
        Some(EditorCommand::MoveUp.into())
    );
    assert_eq!(target(&result.keymap, Mode::Normal, S::Markdown, "k"), None);
    assert_eq!(
        target(&result.keymap, Mode::Select, S::Markdown, "F8"),
        Some(EditorCommand::MoveUp.into())
    );
    assert_eq!(
        target(&result.keymap, Mode::List, S::Global, "F9"),
        Some(EditorCommand::ListNext.into())
    );
}

#[test]
fn cross_mode_aliases_and_scope_action_reference_follow_admission() {
    let map = configured(
        "scopes: {markdown: {insert: {Ctrl-x: null}, normal: {F9: extend-line-above}}, directory-tree: {normal: {Tab: null}}}",
    );
    let mut hints = crate::key_hints::KeyHintState::default();
    for key in KeySequence::parse("Space l").unwrap().as_slice() {
        hints.observe_in(*key, Mode::Normal, S::Markdown, &map);
    }
    let row = |scope| {
        hints
            .rows_in(&map, Mode::Normal, scope)
            .into_iter()
            .find(|row| row.target == Some(EditorCommand::TriggerCompletion.into()))
            .unwrap()
    };
    assert!(row(S::Markdown).alias.is_none());
    assert!(row(S::Global).alias.is_some());
    assert_eq!(
        target(&map, Mode::Normal, S::Markdown, "F9"),
        Some(EditorCommand::ExtendLineAbove.into())
    );
    let reference = keymap::actions::reference();
    assert!(reference.contains("### global\n"));
    assert!(reference.contains("`list-next`: list"));
    assert!(reference.contains("### directory-tree\n"));
    assert!(!reference.contains("`commit-undo-checkpoint`"));
}
