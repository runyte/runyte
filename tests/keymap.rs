// SPDX-License-Identifier: MPL-2.0

use std::collections::HashSet;

use runyte::{
    app::App,
    command::{
        ColonCommand, CommandId, CommandInvocation, EditorCommand, InvocationParameters, Mode,
    },
    config::Config,
    input::{KeyCode, KeyStroke, Modifiers},
    keymap::{
        BindingAvailability, BindingRole, BindingScope, BindingTarget, Key, KeySequence, Lookup,
        default_keymap, is_fast_pane_key, keymap_for,
    },
};

#[test]
fn every_mode_sequence_is_unique_and_described() {
    let keymap = default_keymap();
    let mut seen = HashSet::new();

    for binding in keymap.bindings() {
        assert!(!binding.description.is_empty());
        for mode in binding.modes {
            assert!(
                seen.insert((*mode, binding.scope, binding.sequence.clone())),
                "duplicate {} {:?} binding for {}",
                mode.label(),
                binding.scope,
                binding.sequence
            );
        }
    }
}

#[test]
fn git_namespace_keeps_only_navigation_and_refresh_commands() {
    let retained = [
        " gg", " gd", " gD", " gr", " gt", " g/", " gl", " gb", " gB", " gw",
    ];
    for keys in retained {
        assert!(
            matches!(
                default_keymap().lookup(Mode::Normal, &sequence(keys)),
                Lookup::Exact(_)
            ),
            "missing retained Git binding {keys:?}"
        );
    }

    for keys in [" ga", " gc", " gi", " gs", " gS", " gu"] {
        assert!(
            matches!(
                default_keymap().lookup(Mode::Normal, &sequence(keys)),
                Lookup::NoMatch
            ),
            "removed Git binding {keys:?} is still registered"
        );
    }
}

#[test]
fn worktree_removal_is_scoped_only_to_the_worktree_list() {
    let actions = default_keymap()
        .context_actions(BindingScope::GitWorktrees)
        .filter(|action| action.target == BindingTarget::Editor(EditorCommand::RemoveWorktree))
        .collect::<Vec<_>>();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].mnemonic, Key::char('D'));
    assert!(matches!(
        default_keymap().lookup_in(Mode::Normal, BindingScope::GitWorktrees, &sequence("D")),
        Lookup::NoMatch
    ));
}

#[test]
fn close_bindings_keep_panes_and_buffers_as_separate_decisions() {
    for mode in [Mode::Normal, Mode::Select] {
        assert!(matches!(
            default_keymap().lookup(mode, &sequence(" wc")),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::CloseWindow)
        ));
        assert!(matches!(
            default_keymap().lookup(mode, &sequence(" bc")),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Colon(ColonCommand::CloseBuffer)
        ));
        assert!(default_keymap().bindings().iter().all(|binding| {
            binding.target != BindingTarget::Colon(ColonCommand::ForceCloseBuffer)
        }));
    }
}

#[test]
fn equalizing_is_reached_from_the_window_namespace_alone() {
    for mode in [Mode::Normal, Mode::Select] {
        assert!(matches!(
            default_keymap().lookup(mode, &sequence(" w=")),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::EqualizeWindows)
        ));
    }
    assert_eq!(
        default_keymap()
            .bindings()
            .iter()
            .filter(
                |binding| binding.target == BindingTarget::Editor(EditorCommand::EqualizeWindows)
            )
            .count(),
        1
    );
}

#[test]
fn finder_and_workspace_search_are_global_in_every_buffer_scope() {
    let cases = [
        (" //", EditorCommand::OpenFilePicker),
        (" /s", EditorCommand::GlobalSearch),
        (" /S", EditorCommand::GlobalSearchRegex),
        (" /a", EditorCommand::OpenAllFilesPicker),
        (" /p", EditorCommand::OpenPathFilePicker),
    ];
    for scope in [
        BindingScope::Global,
        BindingScope::Directory,
        BindingScope::Settings,
        BindingScope::GitStatus,
        BindingScope::GitBranches,
        BindingScope::Help,
        BindingScope::CommitMessage,
        BindingScope::Diff,
    ] {
        for (keys, command) in cases {
            assert!(matches!(
                default_keymap().lookup_in(Mode::Normal, scope, &sequence(keys)),
                Lookup::Exact(binding)
                    if binding.target == BindingTarget::Editor(command)
            ));
        }
    }

    {
        let (canonical, alias, command) = (" //", "/", EditorCommand::OpenFilePicker);
        let Lookup::Exact(binding) = default_keymap().lookup(Mode::Normal, &sequence(canonical))
        else {
            panic!("missing canonical picker binding {canonical:?}");
        };
        assert_eq!(binding.alias.as_ref(), Some(&sequence(alias)));
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &sequence(alias)),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(command)
        ));
    }
}

#[test]
fn registry_inventory_uses_canonical_owned_strokes_without_changing_labels() {
    for binding in default_keymap().bindings() {
        let canonical = KeySequence::new(binding.sequence.as_slice().iter().copied());
        assert_eq!(canonical, binding.sequence, "{}", binding.target.name());
        assert_eq!(
            binding.sequence.to_string(),
            binding
                .sequence
                .as_slice()
                .iter()
                .map(|stroke| stroke.label())
                .collect::<Vec<_>>()
                .join(" "),
            "{}",
            binding.target.name()
        );
    }

    assert_eq!(
        KeySequence::from(KeyStroke::new(KeyCode::Char('G'), Modifiers::SHIFT)),
        KeySequence::from(Key::char('G'))
    );
    assert_eq!(
        KeySequence::from(KeyStroke::new(KeyCode::BackTab, Modifiers::SHIFT)).to_string(),
        "Shift-BackTab"
    );
}

#[test]
fn core_minor_modes_expose_registry_continuations() {
    let cases = [
        (
            KeySequence::from(Key::char('g')),
            EditorCommand::MoveFileStart,
        ),
        (
            KeySequence::from(Key::char(' ')),
            EditorCommand::OpenFilePicker,
        ),
        (
            KeySequence::from(Key::ctrl('w')),
            EditorCommand::FocusWindowLeft,
        ),
        (
            KeySequence::from([Key::char(' '), Key::char('w')]),
            EditorCommand::OnlyWindow,
        ),
        (
            KeySequence::from([Key::char(' '), Key::char('p')]),
            EditorCommand::HardWrap,
        ),
    ];

    for (prefix, expected) in cases {
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &prefix),
            Lookup::Prefix(bindings)
                if bindings.iter().any(|binding| binding.target == BindingTarget::Editor(expected))
        ));
    }
}

#[test]
fn space_r_reuses_the_reload_command_identity() {
    let sequence = KeySequence::from([Key::char(' '), Key::char('r')]);
    for mode in [Mode::Normal, Mode::Select] {
        assert!(matches!(
            default_keymap().lookup(mode, &sequence),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Colon(ColonCommand::Reload)
                    && binding.role == BindingRole::Primary
        ));
    }
}

#[test]
fn space_p_r_exposes_reflow_in_normal_and_select_modes() {
    let sequence = KeySequence::from([Key::char(' '), Key::char('p'), Key::char('r')]);
    for mode in [Mode::Normal, Mode::Select] {
        assert!(matches!(
            default_keymap().lookup(mode, &sequence),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::Reflow)
        ));
    }
}

#[test]
fn space_p_j_exposes_join_selections_in_normal_and_select_modes() {
    let sequence = KeySequence::from([Key::char(' '), Key::char('p'), Key::char('j')]);
    for mode in [Mode::Normal, Mode::Select] {
        assert!(matches!(
            default_keymap().lookup(mode, &sequence),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::JoinSelections)
        ));
    }

    // Helix spends `J` on joining; Runyte leaves the letter unbound because the
    // delimiter prompt is part of the command.
    let helix = KeySequence::from([Key::char('J')]);
    assert!(!matches!(
        default_keymap().lookup(Mode::Normal, &helix),
        Lookup::Exact(_)
    ));
}

#[test]
fn space_p_t_exposes_format_table_in_normal_and_select_modes() {
    let sequence = KeySequence::from([Key::char(' '), Key::char('p'), Key::char('t')]);
    for mode in [Mode::Normal, Mode::Select] {
        assert!(matches!(
            default_keymap().lookup(mode, &sequence),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::FormatTable)
        ));
    }
}

#[test]
fn space_p_w_exposes_hard_wrap_in_normal_and_select_modes() {
    let sequence = KeySequence::from([Key::char(' '), Key::char('p'), Key::char('w')]);
    for mode in [Mode::Normal, Mode::Select] {
        assert!(matches!(
            default_keymap().lookup(mode, &sequence),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::HardWrap)
        ));
    }

    let retired = KeySequence::from([Key::char(' '), Key::char('p'), Key::char('h')]);
    assert!(!matches!(
        default_keymap().lookup(Mode::Normal, &retired),
        Lookup::Exact(_)
    ));
}

#[test]
fn implemented_and_unsupported_bindings_are_explicit() {
    let implemented = KeySequence::from([Key::char('g'), Key::char('h')]);
    // V4 has no remaining Planned binding. Shell piping remains explicitly
    // unsupported because it is outside the milestone.
    let lsp = KeySequence::from([Key::char('g'), Key::char('d')]);
    let buffer_picker = KeySequence::from([Key::char(' '), Key::char('b'), Key::char('b')]);
    let new_buffer = KeySequence::from([Key::char(' '), Key::char('b'), Key::char('n')]);
    let unsupported = KeySequence::from(Key::char('|'));

    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &implemented),
        Lookup::Exact(binding)
            if binding.availability == BindingAvailability::Implemented
    ));
    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &lsp),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::GotoDefinition)
                && binding.availability == BindingAvailability::Implemented
    ));
    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &buffer_picker),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::OpenBufferPicker)
                && binding.availability == BindingAvailability::Implemented
    ));
    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &new_buffer),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::NewBuffer)
                && binding.availability == BindingAvailability::Implemented
    ));
    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &unsupported),
        Lookup::Exact(binding)
            if matches!(binding.availability, BindingAvailability::Unsupported(_))
    ));
}

#[test]
fn explorer_bindings_distinguish_active_and_working_directories() {
    for (key, command, description) in [
        (
            'e',
            EditorCommand::OpenExplorer,
            "Open file explorer in the active buffer's directory",
        ),
        (
            'E',
            EditorCommand::OpenWorkingDirectoryExplorer,
            "Open file explorer in the working directory",
        ),
    ] {
        let sequence = KeySequence::from([Key::char(' '), Key::char(key)]);
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &sequence),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(command)
                    && binding.description == description
        ));
    }
}

#[test]
fn syntax_shrink_binding_describes_the_action_as_shrinking() {
    let sequence = KeySequence::from([Key::char(' '), Key::char('x'), Key::char('s')]);
    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &sequence),
        Lookup::Exact(binding)
            if binding.target
                == BindingTarget::Editor(EditorCommand::ShrinkSyntaxSelection)
                && binding.description == "Shrink the syntax selection"
    ));
}

#[test]
fn v4_leaves_no_planned_key_binding() {
    assert!(
        default_keymap()
            .bindings()
            .iter()
            .all(|binding| !matches!(binding.availability, BindingAvailability::Planned(_)))
    );
}

#[test]
fn select_and_insert_modes_share_the_registry_without_duplicate_dispatch_tables() {
    let movement = KeySequence::from(Key::char('e'));
    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &movement),
        Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::MoveWordEnd)
    ));
    assert!(matches!(
        default_keymap().lookup(Mode::Select, &movement),
        Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::MoveWordEnd)
    ));

    let delete_word = KeySequence::from(Key::new(KeyCode::Backspace, Modifiers::ALT));
    assert!(matches!(
        default_keymap().lookup(Mode::Insert, &delete_word),
        Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::DeleteWordBackward)
    ));
    let delete_next_word = KeySequence::from(Key::new(KeyCode::Delete, Modifiers::ALT));
    assert!(matches!(
        default_keymap().lookup(Mode::Insert, &delete_next_word),
        Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::DeleteWordForward)
    ));
}

#[test]
fn app_executes_three_key_space_window_sequences() {
    let mut app = App::new(Config::default(), None).unwrap();
    for key in [
        KeyStroke::new(KeyCode::Char(' '), Modifiers::NONE),
        KeyStroke::new(KeyCode::Char('w'), Modifiers::NONE),
        KeyStroke::new(KeyCode::Char('v'), Modifiers::NONE),
    ] {
        app.handle_key(key).unwrap();
    }

    assert_eq!(app.panes.len(), 2);
    assert!(app.pending_sequence().is_empty());
}

fn sequence(keys: &str) -> KeySequence {
    KeySequence::new(keys.chars().map(Key::char))
}

#[test]
fn nested_space_tree_is_exact_primary_and_keeps_fast_compatibility_paths() {
    use BindingTarget::{Colon, Editor};

    let cases = [
        ("  ", Colon(ColonCommand::SessionList)),
        (" cy", Editor(EditorCommand::ClipboardYank)),
        (" cp", Editor(EditorCommand::ClipboardPasteAfter)),
        (" cP", Editor(EditorCommand::ClipboardPasteBefore)),
        (" lh", Editor(EditorCommand::ShowDocumentation)),
        (" ls", Editor(EditorCommand::DocumentSymbols)),
        (" lS", Editor(EditorCommand::WorkspaceSymbols)),
        (" ld", Editor(EditorCommand::Diagnostics)),
        (" lr", Editor(EditorCommand::RenameSymbol)),
        (" la", Editor(EditorCommand::CodeAction)),
        (" lf", Colon(ColonCommand::Format)),
        (" lR", Colon(ColonCommand::LspRestart)),
        (" l?", Colon(ColonCommand::LspStatus)),
        (" lgd", Editor(EditorCommand::GotoDefinition)),
        (" lgD", Editor(EditorCommand::GotoDeclaration)),
        (" lgy", Editor(EditorCommand::GotoTypeDefinition)),
        (" lgr", Editor(EditorCommand::GotoReferences)),
        (" lgi", Editor(EditorCommand::GotoImplementation)),
        (" //", Editor(EditorCommand::OpenFilePicker)),
        (" /s", Editor(EditorCommand::GlobalSearch)),
        (" /S", Editor(EditorCommand::GlobalSearchRegex)),
        (" /a", Editor(EditorCommand::OpenAllFilesPicker)),
        (" /p", Editor(EditorCommand::OpenPathFilePicker)),
        (" p.", Editor(EditorCommand::ToggleWhitespace)),
        (" g/", Colon(ColonCommand::GitSearchCommits)),
        (" gw", Colon(ColonCommand::GitWorktrees)),
        (" xe", Editor(EditorCommand::ExpandSyntaxSelection)),
        (" xs", Editor(EditorCommand::ShrinkSyntaxSelection)),
        (" xp", Editor(EditorCommand::SelectSyntaxParent)),
        (" xc", Editor(EditorCommand::SelectSyntaxChild)),
        (" xh", Editor(EditorCommand::SelectPreviousSyntaxSibling)),
        (" xl", Editor(EditorCommand::SelectNextSyntaxSibling)),
        (" xo", Editor(EditorCommand::DocumentOutline)),
        (" xaf", Editor(EditorCommand::SelectSyntaxFunction)),
        (" xac", Editor(EditorCommand::SelectSyntaxClass)),
        (" xap", Editor(EditorCommand::SelectSyntaxParameter)),
        (" xif", Editor(EditorCommand::SelectInsideSyntaxFunction)),
        (" xic", Editor(EditorCommand::SelectInsideSyntaxClass)),
        (" xip", Editor(EditorCommand::SelectInsideSyntaxParameter)),
        (" xa(", Editor(EditorCommand::SelectAroundParentheses)),
        (" xa)", Editor(EditorCommand::SelectAroundParentheses)),
        (" xi(", Editor(EditorCommand::SelectInsideParentheses)),
        (" xi)", Editor(EditorCommand::SelectInsideParentheses)),
        (" xa[", Editor(EditorCommand::SelectAroundSquareBrackets)),
        (" xi]", Editor(EditorCommand::SelectInsideSquareBrackets)),
        (" xa{", Editor(EditorCommand::SelectAroundBraces)),
        (" xi}", Editor(EditorCommand::SelectInsideBraces)),
        (" xa<", Editor(EditorCommand::SelectAroundAngleBrackets)),
        (" xi>", Editor(EditorCommand::SelectInsideAngleBrackets)),
        (" xa\"", Editor(EditorCommand::SelectAroundDoubleQuotes)),
        (" xi'", Editor(EditorCommand::SelectInsideSingleQuotes)),
        (" xa`", Editor(EditorCommand::SelectAroundBackticks)),
        (" xam", Editor(EditorCommand::SelectAroundClosestDelimiter)),
        (" xim", Editor(EditorCommand::SelectInsideClosestDelimiter)),
        (" x[f", Editor(EditorCommand::GotoPreviousSyntaxFunction)),
        (" x[c", Editor(EditorCommand::GotoPreviousSyntaxClass)),
        (" x[p", Editor(EditorCommand::GotoPreviousSyntaxParameter)),
        (" x]f", Editor(EditorCommand::GotoNextSyntaxFunction)),
        (" x]c", Editor(EditorCommand::GotoNextSyntaxClass)),
        (" x]p", Editor(EditorCommand::GotoNextSyntaxParameter)),
    ];

    for (keys, target) in cases {
        assert!(
            matches!(
                default_keymap().lookup(Mode::Normal, &sequence(keys)),
                Lookup::Exact(binding)
                    if binding.target == target
                        && binding.role == BindingRole::Primary
                        && binding.description == target.description()
            ),
            "missing primary binding for {keys:?}"
        );
    }

    // `Space b` became the Buffers namespace, so the picker it used to reach
    // directly now lives at `Space b b` as a Primary binding.
    for keys in [" e", " E"] {
        assert!(
            matches!(
                default_keymap().lookup(Mode::Normal, &sequence(keys)),
                Lookup::Exact(binding) if binding.role == BindingRole::Fast
            ),
            "existing fast binding changed for {keys:?}"
        );
    }
    let fast = default_keymap()
        .bindings()
        .iter()
        .filter(|binding| binding.role == BindingRole::Fast)
        .map(|binding| binding.sequence.to_string())
        .collect::<HashSet<_>>();
    assert_eq!(
        fast,
        ["Space e", "Space E"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    let space_window = KeySequence::from([Key::char(' '), Key::char('w')]);
    assert!(default_keymap().bindings().iter().all(|binding| {
        !binding.sequence.starts_with(&space_window) || binding.role == BindingRole::Primary
    }));
    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &KeySequence::from(Key::ctrl('w'))),
        Lookup::Prefix(bindings)
            if bindings.iter().all(|binding| binding.role == BindingRole::Compatibility)
    ));
    for keys in ["gd", "gD", "gy", "gr", "gi"] {
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &sequence(keys)),
            Lookup::Exact(binding) if binding.role == BindingRole::Primary
        ));
    }
    for (keys, command) in [
        ("gp", EditorCommand::GotoNextParagraph),
        ("gP", EditorCommand::GotoPreviousParagraph),
    ] {
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &sequence(keys)),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(command)
                    && binding.role == BindingRole::Primary
        ));
    }
}

#[test]
fn character_find_and_project_find_keep_distinct_direct_bindings() {
    let direct = sequence("f");
    for mode in [Mode::Normal, Mode::Select] {
        assert!(matches!(
            default_keymap().lookup(mode, &direct),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::FindNextChar)
        ));
        assert!(matches!(
            default_keymap().lookup(mode, &sequence(" /f")),
            Lookup::NoMatch
        ));
        assert!(matches!(
            default_keymap().lookup(mode, &sequence("/")),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::OpenFilePicker)
        ));
    }
}

#[test]
fn completion_has_a_discoverable_space_binding_and_text_entry_alias() {
    let canonical = sequence(" lc");
    let insert_alias = KeySequence::from(Key::ctrl('x'));

    for mode in [Mode::Normal, Mode::Select] {
        let Lookup::Exact(binding) = default_keymap().lookup(mode, &canonical) else {
            panic!("missing Space l c in {} mode", mode.label());
        };
        assert_eq!(
            binding.target,
            BindingTarget::Editor(EditorCommand::TriggerCompletion)
        );
        assert_eq!(binding.role, BindingRole::Primary);
        assert_eq!(binding.alias.as_ref(), Some(&insert_alias));
        assert_eq!(
            binding.alias_modes,
            Some(&[Mode::Insert, Mode::Replace][..])
        );
    }

    assert!(matches!(
        default_keymap().lookup(Mode::Insert, &insert_alias),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::TriggerCompletion)
    ));
    assert!(matches!(
        default_keymap().lookup(Mode::Replace, &insert_alias),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::TriggerCompletion)
    ));
}

#[test]
fn control_backslash_exits_insert_and_control_w_moves_between_panes() {
    for key in [Key::ctrl('\\'), Key::ctrl('4')] {
        assert!(matches!(
            default_keymap().lookup(Mode::Insert, &KeySequence::from(key)),
            Lookup::Exact(binding)
                if binding.target
                    == BindingTarget::Editor(EditorCommand::EnterNormalMode)
        ));
    }

    for key in [Key::ctrl('\\'), Key::ctrl('4')] {
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &KeySequence::from(key)),
            Lookup::Exact(binding)
                if binding.target
                    == BindingTarget::Editor(EditorCommand::EnterNormalMode)
        ));
    }
    assert!(matches!(
        default_keymap().lookup(Mode::Insert, &KeySequence::from(Key::ctrl('w'))),
        Lookup::Prefix(_)
    ));
    assert!(matches!(
        default_keymap().lookup(
            Mode::Insert,
            &KeySequence::from([Key::ctrl('w'), Key::char('h')])
        ),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::FocusWindowLeft)
    ));
    assert!(matches!(
        default_keymap().lookup(Mode::Normal, &KeySequence::from(Key::ctrl('w'))),
        Lookup::Prefix(_)
    ));
}

#[test]
fn ctrl_w_arrow_suffixes_stay_unbound_in_every_mode_and_terminal_insert() {
    let keymap = default_keymap();
    for arrow in [KeyCode::Left, KeyCode::Down, KeyCode::Up, KeyCode::Right] {
        let sequence = KeySequence::from([Key::ctrl('w'), Key::plain(arrow)]);
        for mode in [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace] {
            assert!(
                matches!(keymap.lookup(mode, &sequence), Lookup::NoMatch),
                "Ctrl-w {} must stay unbound in {}",
                Key::plain(arrow).label(),
                mode.label()
            );
        }
        assert!(
            matches!(
                keymap.lookup_in(Mode::Insert, BindingScope::Terminal, &sequence),
                Lookup::NoMatch
            ),
            "Ctrl-w {} must stay unbound in Terminal Insert",
            Key::plain(arrow).label()
        );
    }
}

#[test]
fn removed_duplicate_bindings_stay_unbound() {
    for keys in [
        " :", " Fe", " FE", " Ff", " Fb", " F/", " Fs", " Fr", " h", " S", " d", " a", " y", " P",
        " wq",
        // `?` was the backward search prompt. Search no longer has a direction
        // to choose at the prompt, and `Space ?` still opens help.
        "?", // Removing a selection is `Space s r`, so the key matches the word.
        " sj",
    ] {
        assert!(
            matches!(
                default_keymap().lookup(Mode::Normal, &sequence(keys)),
                Lookup::NoMatch
            ),
            "removed binding still resolves: {keys:?}"
        );
    }
    // Every selection command that had an Alt shortcut now lives under
    // `Space s`. `Alt-j`/`Alt-k` in particular were also the key-hint popup's
    // scroll keys, so the popup and the keymap were fighting over them.
    for removed in [Key::alt('s'), Key::alt('*'), Key::alt('k'), Key::alt('j')] {
        assert!(
            matches!(
                default_keymap().lookup(Mode::Normal, &KeySequence::from(removed)),
                Lookup::NoMatch
            ),
            "removed binding still resolves: {removed:?}"
        );
    }
    for sequence in [
        KeySequence::from([Key::ctrl('w'), Key::char('q')]),
        KeySequence::from([Key::ctrl('w'), Key::ctrl('q')]),
    ] {
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &sequence),
            Lookup::NoMatch
        ));
    }
    for sequence in [
        KeySequence::from(Key::ctrl('h')),
        KeySequence::from(Key::ctrl('d')),
    ] {
        assert!(matches!(
            default_keymap().lookup(Mode::Insert, &sequence),
            Lookup::NoMatch
        ));
    }
}

#[test]
fn namespace_rows_are_non_executable_prefixes_backed_by_real_bindings() {
    let keymap = default_keymap();
    assert!(!keymap.namespaces().is_empty());
    for namespace in keymap.namespaces() {
        assert!(!namespace.description.is_empty());
        assert!(matches!(
            keymap.lookup(Mode::Normal, &namespace.sequence),
            Lookup::Prefix(bindings) if !bindings.is_empty()
        ));
    }
    for binding in keymap.bindings() {
        assert_eq!(binding.description, binding.target.description());
    }

    for (index, binding) in keymap.bindings().iter().enumerate() {
        for candidate in &keymap.bindings()[index + 1..] {
            if binding.scope != candidate.scope
                || !binding
                    .modes
                    .iter()
                    .any(|mode| candidate.modes.contains(mode))
            {
                continue;
            }
            let binding_prefix = candidate.sequence.starts_with(&binding.sequence)
                && candidate.sequence != binding.sequence;
            let candidate_prefix = binding.sequence.starts_with(&candidate.sequence)
                && binding.sequence != candidate.sequence;
            assert!(
                !binding_prefix && !candidate_prefix,
                "executable binding prefix conflict: {} and {}",
                binding.sequence,
                candidate.sequence
            );
        }
    }
}

#[test]
fn colon_key_targets_construct_the_same_typed_identities_as_direct_calls() {
    let direct_format = CommandInvocation::from_parts(
        CommandId::Colon(ColonCommand::Format),
        InvocationParameters::None,
        Default::default(),
    )
    .unwrap();
    let cases = [
        (" lf", direct_format),
        (" lR", CommandInvocation::lsp_restart(None).unwrap()),
        (" l?", CommandInvocation::lsp_status()),
    ];
    for (keys, direct) in cases {
        let Lookup::Exact(binding) = default_keymap().lookup(Mode::Normal, &sequence(keys)) else {
            panic!("missing colon binding {keys:?}")
        };
        assert_eq!(binding.target.invocation().unwrap().id(), direct.id());
    }
}

/// Off by default, so the four keys keep whatever they already meant. That is
/// the whole reason the option exists: `Ctrl-j` and `Ctrl-k` are Insert-mode
/// editing keys, and a terminal's child wants all four.
#[test]
fn single_key_pane_moves_are_absent_until_configured_on() {
    let keymap = keymap_for(false);
    assert!(std::ptr::eq(keymap, default_keymap()));

    for mode in [Mode::Normal, Mode::Select] {
        for character in ['h', 'j', 'k', 'l'] {
            assert!(
                matches!(
                    keymap.lookup(mode, &KeySequence::from(Key::ctrl(character))),
                    Lookup::NoMatch
                ),
                "Ctrl-{character} must stay unbound in {}",
                mode.label()
            );
        }
    }
    assert!(matches!(
        keymap.lookup(Mode::Insert, &KeySequence::from(Key::ctrl('j'))),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::InsertNewline)
    ));
    assert!(matches!(
        keymap.lookup(Mode::Insert, &KeySequence::from(Key::ctrl('k'))),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::DeleteToLineEnd)
    ));
}

/// The same four moves the `Ctrl-w` prefix reaches, one keystroke shorter, in
/// every mode a cursor can be in.
#[test]
fn configured_on_the_pane_moves_answer_in_every_mode() {
    let keymap = keymap_for(true);
    let expected = [
        ('h', EditorCommand::FocusWindowLeft),
        ('j', EditorCommand::FocusWindowDown),
        ('k', EditorCommand::FocusWindowUp),
        ('l', EditorCommand::FocusWindowRight),
    ];

    for (character, command) in expected {
        assert!(is_fast_pane_key(Key::ctrl(character)));
        for mode in [Mode::Normal, Mode::Select, Mode::Insert] {
            let Lookup::Exact(binding) =
                keymap.lookup(mode, &KeySequence::from(Key::ctrl(character)))
            else {
                panic!("Ctrl-{character} must move a pane in {}", mode.label());
            };
            assert_eq!(binding.target, BindingTarget::Editor(command));
            assert_eq!(binding.role, BindingRole::Fast);
            // Discovery still points at the prefix spelling, which is the one
            // that works whether or not this option is on.
            assert_eq!(
                binding.alias,
                Some(KeySequence::from([Key::ctrl('w'), Key::char(character)]))
            );
        }
    }

    // Nothing about the prefix spelling changes; the short keys are an
    // addition, not a replacement.
    assert!(matches!(
        keymap.lookup(
            Mode::Normal,
            &KeySequence::from([Key::ctrl('w'), Key::char('h')])
        ),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::FocusWindowLeft)
    ));
}

/// One answer per key in the registry, so help and the hint popup cannot
/// describe a key the editor no longer honours.
#[test]
fn the_pane_moves_replace_the_insert_keys_they_collide_with() {
    let keymap = keymap_for(true);
    let mut seen = HashSet::new();
    for binding in keymap.bindings() {
        for mode in binding.modes {
            assert!(
                seen.insert((*mode, binding.scope, binding.sequence.clone())),
                "duplicate {} {:?} binding for {}",
                mode.label(),
                binding.scope,
                binding.sequence
            );
        }
    }

    for command in [EditorCommand::InsertNewline, EditorCommand::DeleteToLineEnd] {
        assert!(
            !keymap.bindings().iter().any(|binding| {
                binding.target == BindingTarget::Editor(command)
                    && binding.sequence.len() == 1
                    && binding.is_active_in(Mode::Insert)
                    && matches!(
                        binding.sequence.as_slice(),
                        [key] if key.modifiers.contains(Modifiers::CONTROL)
                            && matches!(key.code, KeyCode::Char('j' | 'k'))
                    )
            }),
            "the shadowed {command:?} binding must leave the registry, not just lose dispatch"
        );
    }

    // Everything the default keymap binds outside those four keys survives.
    for binding in default_keymap().bindings() {
        if binding.sequence.len() == 1 && is_fast_pane_key(binding.sequence.as_slice()[0]) {
            continue;
        }
        assert!(
            keymap.bindings().iter().any(|kept| {
                kept.sequence == binding.sequence
                    && kept.scope == binding.scope
                    && kept.target == binding.target
            }),
            "{} must survive turning fast pane keys on",
            binding.sequence
        );
    }
}
