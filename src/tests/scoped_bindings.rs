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

#[test]
fn terminal_insert_actions_claim_only_effective_scoped_keys_and_prefixes() {
    use crate::keymap::{Key, terminal_input::Route::*};
    let map = configured(
        "scopes: {terminal: {insert: {F6: enter-normal-mode, 'F7 h': focus-window-left, 'Ctrl-4': null, 'Ctrl-\\': null, 'Ctrl-w': null}}}",
    );
    let route = |s| map.terminal_input_route(Key::parse(s).unwrap(), false, false);
    assert_eq!(route("F6"), EditorGrammar);
    assert_eq!(route("F7"), EditorGrammar);
    assert_eq!(route("Shift-F6"), Child);
    assert_eq!(route("Ctrl-4"), Child);
    assert_eq!(route("Ctrl-\\"), Child);
    assert_eq!(route("Alt-Ctrl-4"), EditorGrammar);
    // Exact removal does not remove the surviving window namespace.
    assert_eq!(route("Ctrl-w"), EditorGrammar);
    assert_eq!(route("Esc"), Child);
    assert_eq!(route("Enter"), Child);
    assert_eq!(route("a"), Child);
    let rebuilt = map.with_plugin_bindings(vec![]).unwrap();
    assert_eq!(
        rebuilt.terminal_input_route(Key::parse("F6").unwrap(), false, false),
        EditorGrammar
    );
    assert_eq!(
        rebuilt.terminal_input_route(Key::ctrl('4'), false, false),
        Child
    );
    let map = configured("scopes: {terminal: {insert: {'F8': next-running-session}}}");
    assert_eq!(
        map.terminal_input_route(Key::parse("F8").unwrap(), false, false),
        EditorGrammar
    );
}

#[test]
fn terminal_insert_rejects_text_actions_and_replace_mode_atomically() {
    for source in [
        "scopes: {terminal: {insert: {F6: insert-newline}}}",
        "scopes: {terminal: {insert: {F6: move-down}}}",
        "scopes: {terminal: {replace: {F6: null}}}",
    ] {
        let result = keymap::configured::compile(
            &serde_yaml::from_str(source).unwrap(),
            keymap::default_keymap(),
        );
        assert_eq!(result.errors.len(), 1, "{source}");
    }
}

#[test]
fn terminal_first_key_routing_preserves_all_default_modifier_policies() {
    use crate::{
        input::{KeyCode, KeyStroke, Modifiers},
        keymap::terminal_input::Route,
    };
    for fast in [false, true] {
        let map = keymap::keymap_for(fast);
        for persistent in [false, true] {
            for code in (0x20..=0x7e)
                .map(|c| KeyCode::Char(char::from_u32(c).unwrap()))
                .chain([
                    KeyCode::Escape,
                    KeyCode::Enter,
                    KeyCode::Left,
                    KeyCode::Right,
                    KeyCode::BackTab,
                    KeyCode::Tab,
                ])
            {
                for bits in 0..64 {
                    let key = KeyStroke::new(code, Modifiers::from_bits(bits).unwrap());
                    let navigation = |binding: &Binding| {
                        matches!(
                            binding.target,
                            BindingTarget::Editor(
                                EditorCommand::NextRunningSession
                                    | EditorCommand::PreviousRunningSession
                                    | EditorCommand::PreviousSession
                            )
                        )
                    };
                    let navigation =
                        match map.lookup_in(Mode::Insert, S::Terminal, &KeySequence::from(key)) {
                            Lookup::Exact(b) => navigation(b),
                            Lookup::Prefix(bs) => bs.iter().any(|b| navigation(b)),
                            Lookup::ExactAndPrefix {
                                exact,
                                continuations,
                            } => navigation(exact) || continuations.iter().any(|b| navigation(b)),
                            Lookup::NoMatch => false,
                        };
                    let old = keymap::is_terminal_normal_key(key)
                        || key.canonical_for_binding() == map.window_prefix()
                        || (fast && keymap::is_fast_pane_key(key))
                        || (persistent && navigation)
                        || map.terminal_clipboard_key(key);
                    assert_eq!(
                        map.terminal_input_route(key, persistent, fast) == Route::EditorGrammar,
                        old,
                        "{key:?}, fast={fast}, persistent={persistent}"
                    );
                }
            }
        }
    }
}

#[test]
fn terminal_window_tombstone_suppresses_capture_only_after_last_descendant_removed() {
    use crate::keymap::{Key, terminal_input::Route};
    let removals = keymap::default_keymap()
        .bindings_for_scope(Mode::Insert, S::Terminal)
        .filter(|b| b.sequence.as_slice().first() == Some(&Key::ctrl('w')))
        .map(|b| format!("      '{}': null\n", b.sequence))
        .collect::<String>();
    let source = format!("scopes:\n  terminal:\n    insert:\n{removals}");
    let map = configured(&source);
    assert_eq!(
        map.terminal_input_route(Key::ctrl('w'), false, false),
        Route::EditorGrammar
    );
    let map = configured(&format!("{source}      Ctrl-w: null\n"));
    assert_eq!(
        map.terminal_input_route(Key::ctrl('w'), false, false),
        Route::Child
    );
    let map = configured(&format!(
        "{source}      Ctrl-w: null\n      'Ctrl-w q': toggle-zen\n"
    ));
    assert_eq!(
        map.terminal_input_route(Key::ctrl('w'), false, false),
        Route::EditorGrammar
    );
}

#[test]
fn terminal_help_and_routing_follow_scoped_remaps_and_removals() {
    use crate::{
        command::GrammarKind,
        help::{self, HelpTopic},
    };
    let map = configured(
        "window: F10\nscopes: {terminal: {insert: {F6: enter-normal-mode, 'Ctrl-4': null, 'Ctrl-\\': null, 'F7 z': toggle-zen}}}",
    );
    let document = help::render(
        HelpTopic::Terminal,
        GrammarKind::Runyte,
        S::Terminal,
        &map,
        true,
    );
    let rows = document
        .split_once("Terminal Insert\n")
        .unwrap()
        .1
        .split_once("\nReplace\n")
        .unwrap()
        .0;
    for key in ["F6 ", "F7 z ", "F10 h "] {
        assert!(
            rows.lines().any(|line| line.trim_start().starts_with(key)),
            "{key}: {rows}"
        );
    }
    for key in ["Ctrl-4 ", "Ctrl-\\ ", "Ctrl-w h "] {
        assert!(
            !rows.lines().any(|line| line.trim_start().starts_with(key)),
            "{key}: {rows}"
        );
    }
    assert_eq!(
        map.terminal_input_route(keymap::Key::parse("F10").unwrap(), false, false),
        keymap::terminal_input::Route::EditorGrammar
    );
    assert_eq!(
        map.terminal_input_route(keymap::Key::ctrl('w'), false, false),
        keymap::terminal_input::Route::Child
    );
}
