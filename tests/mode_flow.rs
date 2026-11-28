// SPDX-License-Identifier: MPL-2.0
use runyte::{
    command::Mode,
    config::Config,
    keymap::{KeySequence, Lookup, configured, default_keymap},
};

fn compile(source: &str) -> configured::CompiledKeymap {
    let config: Config = serde_yaml::from_str(source).unwrap();
    configured::compile_configuration(&config, default_keymap())
}
fn accepts(program: &str) {
    let result = compile(&format!(
        "keys:\n  bind:\n    normal:\n      F6: {program}\n"
    ));
    assert!(result.errors.is_empty(), "{program}: {:?}", result.errors);
}
fn rejects(program: &str, detail: &str) {
    let result = compile(&format!(
        "keys:\n  bind:\n    normal:\n      F6: {program}\n      F7: yank\n"
    ));
    assert!(
        result.errors.iter().any(|e| e.contains(detail)),
        "{program}: {:?}",
        result.errors
    );
    assert!(matches!(
        result
            .keymap
            .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()),
        Lookup::NoMatch
    ));
    assert!(matches!(
        result
            .keymap
            .lookup(Mode::Normal, &KeySequence::parse("F7").unwrap()),
        Lookup::Exact(_)
    ));
}

#[test]
fn mode_entry_changes_admission_for_the_remaining_program() {
    accepts("[{action: mode-enter, arg: insert}, insert-newline]");
    accepts("[enter-insert-mode, insert-newline]");
    rejects(
        "[insert-newline, {action: mode-enter, arg: insert}]",
        "step 1",
    );
    rejects(
        "[{action: mode-enter, arg: normal}, insert-newline]",
        "step 2",
    );
}

#[test]
fn conditional_joins_validate_every_reachable_tail() {
    accepts(
        "[{if: recording-macro, then: [{action: mode-enter, arg: insert}], else: [enter-insert-mode]}, insert-newline]",
    );
    rejects(
        "[{if: recording-macro, then: [{action: mode-enter, arg: insert}], else: []}, insert-newline]",
        "step 2",
    );
    accepts("{if: {mode: insert}, then: [insert-newline], else: [yank]}");
    rejects(
        "{if: recording-macro, then: [insert-newline], else: [yank]}",
        "step 1.then.1",
    );
}

#[test]
fn history_restoration_requires_normalization_before_a_mode_specific_tail() {
    for restore in [
        "selection-undo",
        "selection-redo",
        "syntax-shrink-selection",
    ] {
        rejects(&format!("[{restore}, insert-newline]"), "step 2");
        accepts(&format!(
            "[{restore}, {{action: mode-enter, arg: insert}}, insert-newline]"
        ));
    }
}

#[test]
fn public_program_schema_is_strict_and_nonfatal() {
    for step in [
        "mode-enter",
        "{action: mode-enter, arg: list}",
        "{action: mode-enter, arg: missing}",
        "{if: {mode: missing}, then: [yank], else: []}",
        "{if: {mode: normal}, then: [], else: []}",
        "{action: yank, temporary: false}",
    ] {
        rejects(step, "F6");
    }
    accepts("{action: mode-enter, arg: normal, temporary: false}");
    accepts("{if: recording-macro, then: [yank], else: []}");
}

#[test]
fn role_conditions_include_inherited_named_submodes() {
    let result = compile(
        "modes:\n  custom-select: {inherits: select}\n  custom-normal: {inherits: normal}\nbuffer-types:\n  text: {normal: custom-normal, select: select, insert: insert, replace: replace}\nkeys:\n  bind:\n    custom-normal:\n      F6: [{action: mode-enter, arg: custom-select}, {if: {mode: select}, then: [yank], else: [insert-newline]}]\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let result = compile(
        "modes:\n  custom-select: {inherits: select}\n  custom-normal: {inherits: normal}\nbuffer-types:\n  text: {normal: custom-normal, select: select, insert: insert, replace: replace}\nkeys:\n  bind:\n    custom-normal:\n      F6: [{action: mode-enter, arg: select}, {if: {mode: custom-select}, then: [insert-newline], else: [yank]}]\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn history_keeps_the_saved_temporary_return_mode() {
    let result = compile(
        "keys:\n  bind:\n    insert:\n      F6: {action: mode-enter, arg: select, temporary: true}\n    select:\n      F7: select-all\n    normal:\n      F8: [selection-undo, {if: {mode: select}, then: [move-word-forward], else: []}]\n",
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.contains("F8") && e.contains("step 2.then.1")),
        "{:?}",
        result.errors
    );
}

#[test]
fn changing_buffer_type_rechecks_named_conditions() {
    let result = compile(
        "editor: {scratch_markdown: false}\nkeys:\n  bind:\n    directory:\n      F6: [new-buffer, {if: {mode: directory}, then: [], else: [insert-newline]}]\n      F7: [show-help, move-file-end]\n",
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.contains("F6") && e.contains("step 2.else.1")),
        "{:?}",
        result.errors
    );
    assert!(
        !result.errors.iter().any(|e| e.contains("F7")),
        "{:?}",
        result.errors
    );
}

#[test]
fn temporary_return_is_applied_before_action_admission() {
    accepts("[{action: mode-enter, arg: insert, temporary: true}, move-word-forward]");
    rejects(
        "[{action: mode-enter, arg: insert, temporary: true}, insert-newline]",
        "step 2",
    );
    accepts("[{action: mode-enter, arg: select, temporary: true}, yank]");
}

#[test]
fn completed_inheritance_tables_include_removals_before_flow_validation() {
    let good = compile(
        "modes:\n  parent: {}\n  child: {inherits: parent}\nbuffer-types:\n  text: {normal: child, select: select, insert: parent, replace: replace}\nkeys:\n  defaults: false\n  bind:\n    parent: {F6: insert-newline}\n    child: {F6: null, Enter: null, Tab: null, S-Tab: null}\n",
    );
    // This graph still inherits other Insert defaults, so the graph as a whole
    // may be restored, but the removed authored F6 must never cause an error.
    assert!(
        !good.errors.iter().any(|e| e.contains("F6")),
        "{:?}",
        good.errors
    );
    let bad = compile(
        "modes:\n  parent: {}\n  child: {inherits: parent}\nbuffer-types:\n  text: {normal: child, select: select, insert: parent, replace: replace}\nkeys:\n  defaults: false\n  bind:\n    parent: {F6: insert-newline}\n    normal: {F7: yank}\n",
    );
    assert!(
        bad.errors
            .iter()
            .any(|e| e.contains("F6") && e.contains("child") && e.contains("text")),
        "{:?}",
        bad.errors
    );
}

#[test]
fn tree_early_dispatch_preserves_the_temporary_gesture() {
    let result = compile(
        "keys:\n  bind:\n    directory-tree:\n      F6: [{action: mode-enter, arg: select, temporary: true}, directory-tree-down, {if: {mode: select}, then: [directory-tree-down], else: [insert-newline]}]\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn scratch_buffer_preserves_the_editing_role() {
    let result = compile(
        "keys:\n  bind:\n    normal:\n      F6: [{action: mode-enter, arg: select}, new-buffer, {if: {mode: select}, then: [yank], else: [insert-newline]}]\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn unreachable_named_entry_does_not_activate_its_table() {
    let result = compile(
        "modes:\n  unreachable: {}\nkeys:\n  bind:\n    normal:\n      F6: {if: {mode: insert}, then: [{action: mode-enter, arg: unreachable}], else: []}\n    unreachable:\n      F7: insert-newline\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn scope_conditions_are_checked_only_on_reachable_paths() {
    let result = compile(
        "keys:\n  bind:\n    normal:\n      F6: {if: {mode: directory-tree}, then: [{if: tree-entry-is-directory, then: [directory-tree-down], else: []}], else: [yank]}\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn context_only_actions_remain_configurable() {
    for action in [
        "override-setting-language",
        "override-setting-pattern",
        "remove-setting-override",
    ] {
        let result = compile(&format!(
            "keys:\n  bind:\n    settings:\n      F6: {action}\n"
        ));
        assert!(result.errors.is_empty(), "{action}: {:?}", result.errors);
    }
}

#[test]
fn scratch_markdown_destination_is_included_in_branch_analysis() {
    let result = compile(
        "keys:\n  bind:\n    directory:\n      F6: [new-buffer, {if: {mode: markdown}, then: [insert-newline], else: [yank]}]\n",
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.contains("F6") && e.contains("step 2.then.1")),
        "{:?}",
        result.errors
    );
}

#[test]
fn tree_focus_discards_a_cross_view_temporary_origin_for_the_next_key() {
    let result = compile(
        "modes:\n  custom-normal: {inherits: normal}\nbuffer-types:\n  text: custom-normal\nkeys:\n  bind:\n    custom-normal:\n      F6: [{action: mode-enter, arg: select, temporary: true}, directory-tree-focus]\n    directory-tree:\n      F7: {if: {mode: custom-normal}, then: [insert-newline], else: [directory-tree-down]}\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn operand_completion_keeps_the_temporary_mode_until_the_next_step() {
    accepts(
        "[{action: mode-enter, arg: select, temporary: true}, find-next-char, {if: {mode: select}, then: [yank], else: [insert-newline]}]",
    );
}

#[test]
fn counted_file_boundary_joins_counted_and_uncounted_restoration() {
    rejects(
        "[{action: mode-enter, arg: select, temporary: true}, move-file-start, {if: {mode: select}, then: [yank], else: [insert-newline]}]",
        "step 3.else.1",
    );
}

#[test]
fn same_view_reopening_does_not_recreate_a_consumed_gesture() {
    let result = compile(
        "modes:\n  custom-help: {inherits: help}\nkeys:\n  bind:\n    help:\n      F5: {action: mode-enter, arg: custom-help}\n    custom-help:\n      F6: [{action: mode-enter, arg: select, temporary: true}, show-help, move-right, {if: {mode: custom-help}, then: [insert-newline], else: [yank]}]\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn legacy_maps_keep_starting_mode_admission_without_a_graph() {
    let base = runyte::keymap::Keymap::new(Vec::new()).unwrap();
    for program in [
        "insert-newline",
        "{action: mode-enter, arg: normal}",
        "{if: {mode: normal}, then: [yank], else: []}",
    ] {
        let section = serde_yaml::from_str(&format!(
            "bind:\n  normal:\n    F6: {program}\n    F7: yank\n"
        ))
        .unwrap();
        let result = configured::compile(&section, &base);
        assert!(!result.errors.is_empty(), "{program}");
        assert!(matches!(
            result
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("F6").unwrap()),
            Lookup::NoMatch
        ));
        assert!(matches!(
            result
                .keymap
                .lookup(Mode::Normal, &KeySequence::parse("F7").unwrap()),
            Lookup::Exact(_)
        ));
    }
}

#[test]
fn standalone_program_does_not_inherit_removed_temporary_origins() {
    let result = compile(
        "keys:\n  defaults: false\n  bind:\n    normal:\n      F5: open-command-palette\n    select:\n      F6: [move-right, {if: {mode: select}, then: [yank], else: [insert-newline]}]\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn buffer_type_guards_narrow_reachable_conditions_and_actions() {
    accepts(
        "{if: {buffer-type: directory-tree}, then: [{if: tree-entry-expanded, then: [directory-tree-collapse], else: []}], else: [yank]}",
    );
    rejects(
        "{if: {buffer-type: text}, then: [{if: tree-entry-expanded, then: [yank], else: []}], else: [yank]}",
        "condition is unavailable",
    );
    for condition in [
        "{buffer-type: unknown}",
        "{buffer-type: 1}",
        "{buffer-type: null}",
        "{buffer-type: text, mode: normal}",
    ] {
        let result = compile(&format!(
            "keys: {{bind: {{normal: {{F6: {{if: {condition}, then: [yank], else: []}}}}}}}}"
        ));
        assert!(!result.errors.is_empty(), "{condition}");
    }
}

#[test]
fn selection_conditions_reject_non_text_owners_but_allow_generated_text() {
    for owner in ["media", "directory-tree", "session-manager", "list"] {
        for condition in ["selection-has-text", "multiple-selections"] {
            let result = compile(&format!(
                "keys: {{bind: {{{owner}: {{F6: {{if: {condition}, then: [yank], else: []}}}}}}}}"
            ));
            assert!(!result.errors.is_empty(), "{owner}: {condition}");
        }
    }
    for owner in [
        "normal",
        "markdown",
        "help",
        "settings",
        "terminal",
        "directory",
    ] {
        let result = compile(&format!(
            "keys: {{bind: {{{owner}: {{F6: {{if: selection-has-text, then: [yank], else: []}}}}}}}}"
        ));
        assert!(result.errors.is_empty(), "{owner}: {:?}", result.errors);
    }
}

#[test]
fn buffer_guard_follows_a_programs_focus_destination() {
    accepts(
        "[directory-tree-focus, {if: {buffer-type: directory-tree}, then: [{if: tree-entry-expanded, then: [directory-tree-collapse], else: [directory-tree-expand]}], else: [insert-newline]}]",
    );
}

#[test]
fn list_owners_reject_program_syntax_with_the_migration_diagnostic() {
    for owner in ["list", "session-manager"] {
        for value in [
            "[list-next]",
            "[list-next, list-previous]",
            "{steps: [list-next], description: next}",
            "{if: register-empty, then: [list-next], else: [list-previous]}",
            "{action: list-next, arg: x}",
            "{action: list-next, count: 2}",
            "{action: list-next, register: none}",
            "{action: list-next, extend: false}",
        ] {
            let result = compile(&format!("keys: {{bind: {{{owner}: {{F6: {value}}}}}}}"));
            assert!(
                result
                    .errors
                    .iter()
                    .any(|error| error.contains("list bindings take a single action in 0.5")),
                "{owner}: {value}: {:?}",
                result.errors
            );
        }
        for value in ["list-next", "{action: list-next}", "null"] {
            let result = compile(&format!("keys: {{bind: {{{owner}: {{F6: {value}}}}}}}"));
            assert!(
                result.errors.is_empty(),
                "{owner}: {value}: {:?}",
                result.errors
            );
        }
    }
}

#[test]
fn builtin_list_owners_only_contain_single_plain_actions() {
    use runyte::keymap::{BindingScope, program::Step};
    for &scope in BindingScope::ALL {
        for binding in default_keymap().bindings_for_scope(Mode::List, scope) {
            assert!(
                matches!(binding.program.steps(), [Step::Action(_)]),
                "{}",
                binding.sequence
            );
        }
    }
}

#[test]
fn shared_list_ancestors_cannot_bypass_single_action_validation() {
    let result = compile(
        "modes:\n  shared: {inherits: normal}\n  custom-normal: {inherits: shared}\n  list: {inherits: shared}\nbuffer-types:\n  text: {normal: custom-normal, select: select, insert: insert, replace: replace}\nkeys:\n  bind:\n    custom-normal:\n      Space p: open-command-palette\n    shared:\n      F6: {if: {mode: normal}, then: [yank], else: [list-next]}\n",
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("list bindings take a single action in 0.5")),
        "{:?}",
        result.errors
    );
}
