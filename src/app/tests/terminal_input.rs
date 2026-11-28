// SPDX-License-Identifier: MPL-2.0
use super::*;

fn terminal_app(source: &str) -> (App, TerminalId) {
    let mut config = Config::default();
    config.merge_keys_source(source).unwrap();
    let mut app = App::new(config, None).unwrap();
    let terminal = app.terminals.insert_test_session(80, 24);
    app.panes.get_mut(&app.active_pane).unwrap().terminal = Some(terminal);
    app.mode = Mode::Insert;
    (app, terminal)
}

fn send(app: &mut App, spelling: &str) {
    app.handle_key(KeyStroke::parse(spelling).unwrap()).unwrap();
}

#[test]
fn scoped_escape_removal_forwards_old_keys_and_remapped_escape_changes_mode() {
    let (mut app, terminal) = terminal_app(
        "bind:\n  terminal-insert:\n    F6: enter-normal-mode\n    Ctrl-4: null\n    Ctrl-\\: null\n",
    );
    for spelling in ["Ctrl-4", "Ctrl-\\", "Esc", "a", "Shift-F6"] {
        let before = app.terminals.get(terminal).unwrap().input_generation();
        send(&mut app, spelling);
        assert_eq!(app.mode, Mode::Insert, "{spelling}");
        assert_eq!(
            app.terminals.get(terminal).unwrap().input_generation(),
            before + 1,
            "{spelling}"
        );
    }
    let before = app.terminals.get(terminal).unwrap().input_generation();
    send(&mut app, "F6");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before
    );
}

#[test]
fn scoped_terminal_prefix_executes_and_cancels_without_leaking_suffixes() {
    let (mut app, terminal) = terminal_app("bind:\n  terminal-insert:\n    F7 z: toggle-zen\n");
    let before = app.terminals.get(terminal).unwrap().input_generation();
    send(&mut app, "F7");
    assert!(!app.grammar.pending_sequence().is_empty());
    send(&mut app, "Esc");
    assert!(app.grammar.pending_sequence().is_empty());
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before
    );
    assert!(app.maximized.is_none());
    send(&mut app, "F7");
    send(&mut app, "z");
    assert!(app.maximized.is_some());
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before
    );
    send(&mut app, "z");
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before + 1
    );
}

#[test]
#[cfg(any(unix, windows))]
fn explicit_session_action_in_standalone_reports_unavailable_without_child_input() {
    let (mut app, terminal) =
        terminal_app("bind:\n  terminal-insert:\n    F8: session-next-running\n");
    let before = app.terminals.get(terminal).unwrap().input_generation();
    send(&mut app, "F8");
    assert_eq!(app.status, "session navigation needs mux mode");
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        before
    );
}

/// What help leaves out of the terminal page is exactly what review refuses
/// with "needs a buffer"; every other command is either consumed by review or
/// passed on to ordinary dispatch, never refused.
#[test]
fn terminal_review_refuses_exactly_what_help_leaves_out() {
    use crate::keymap::admission::admits_command;
    let mut mismatches = Vec::new();
    let mut refused = 0;
    let (mut app, terminal) = terminal_app("{}");
    for &command in EditorCommand::ALL {
        // Only the terminal handler runs, so one app serves every command.
        app.mode = Mode::Normal;
        app.status.clear();
        app.panes.get_mut(&app.active_pane).unwrap().terminal = Some(terminal);
        assert_eq!(app.key_binding_scope(), BindingScope::Terminal);
        let consumed = app.execute_terminal_command_count(terminal, command, false, 1);
        let refusal = app.status.contains("needs a buffer");
        if admits_command(BindingScope::Terminal, command) {
            if refusal {
                mismatches.push(format!("{command:?} admitted but refused: {}", app.status));
            }
        } else {
            refused += 1;
            if !(consumed && refusal) {
                mismatches.push(format!("{command:?} not refused: {}", app.status));
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
    assert!(refused > 100, "only {refused} refused");
}

/// The same agreement through real keys. Some commands are resolved before
/// the terminal handler runs — a counted `g g`, a replayed macro, anything
/// that takes a character — so a key help leaves out must still be refused
/// when it is pressed rather than called. A refusal changes nothing, so one
/// app serves every key.
#[test]
fn terminal_keys_help_leaves_out_are_refused_when_pressed() {
    use crate::keymap::admission::admits;
    let (mut app, terminal) = terminal_app("{}");
    let bindings = app
        .keymap()
        .bindings_for_scope(Mode::Normal, BindingScope::Terminal)
        .filter(|binding| !admits(BindingScope::Terminal, binding.target))
        .map(|binding| (binding.sequence.clone(), binding.target))
        .collect::<Vec<_>>();
    let mut mismatches = Vec::new();
    for (sequence, target) in &bindings {
        // The context launcher owns its key before command dispatch.
        if app.keymap().ui().lookup(
            crate::keymap::ui::Owner::ContextLauncher,
            sequence.as_slice()[0],
        ) == Some(crate::keymap::ui::Action::OpenActions)
        {
            continue;
        }
        app.mode = Mode::Normal;
        app.status.clear();
        app.grammar.reset();
        app.panes.get_mut(&app.active_pane).unwrap().terminal = Some(terminal);
        assert_eq!(app.key_binding_scope(), BindingScope::Terminal);
        for &key in sequence.as_slice() {
            app.handle_key(key).unwrap();
        }
        for _ in 0..2 {
            if app.grammar.awaiting_character().is_some() {
                send(&mut app, "x");
            }
        }
        // A command missing its syntax tree or language server is refused
        // before the terminal is asked.
        let refused = app.status.contains("needs a buffer")
            || app.status.contains("is unavailable for this buffer")
            || app.status.contains("is unsupported");
        if !refused {
            mismatches.push(format!(
                "{sequence} ({target:?}) not refused: {}",
                app.status
            ));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
    assert!(bindings.len() > 50, "only {} refused keys", bindings.len());
}

/// A macro recorded in a terminal is stopped, replayed and listed there too.
#[test]
fn the_default_macro_records_and_replays_in_a_terminal() {
    let (mut app, _) = terminal_app("{}");
    app.mode = Mode::Normal;
    for spelling in ["Space", "m", "m"] {
        send(&mut app, spelling);
    }
    assert!(app.recording_macro.is_some(), "{}", app.status);
    for spelling in ["Space", "m", "m"] {
        send(&mut app, spelling);
    }
    assert!(app.recording_macro.is_none(), "{}", app.status);
    for spelling in ["Space", "m", "r"] {
        send(&mut app, spelling);
    }
    assert!(!app.status.contains("needs a buffer"), "{}", app.status);
}

#[test]
fn review_movement_selection_and_copy_survive_multiple_busy_terminals() {
    let (mut app, terminal) = terminal_app("{}");
    app.terminals.set_scrollback_lines(2);
    let other = app.terminals.insert_test_session(80, 24);
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"first\r\nsecond\r\nthird".to_vec(),
    });
    send(&mut app, "Ctrl-\\");
    assert_eq!(app.mode, Mode::Normal);
    send(&mut app, "k");
    send(&mut app, "x");
    assert_eq!(
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .review_selection_text(),
        "second"
    );
    let frozen = app.terminals.get(terminal).unwrap().view(24).rows;
    for id in [terminal, other] {
        app.apply_terminal_output(TerminalOutput::Bytes {
            id,
            bytes: b"\r\nnew output".repeat(100),
        });
        assert_eq!(app.terminals.get(id).unwrap().scrollback_rows(), 2);
    }
    app.terminals.set_scrollback_lines(0);
    assert_eq!(app.terminals.get(terminal).unwrap().view(24).rows, frozen);
    send(&mut app, "y");
    assert_eq!(app.registers.get(&'"').unwrap().text, "second\n");
    assert!(app.terminals.get(terminal).unwrap().reviewing());
    send(&mut app, "i");
    assert_eq!(app.mode, Mode::Insert);
    assert!(!app.terminals.get(terminal).unwrap().reviewing());
}

/// Selection undo works on a terminal's review just as it does in a buffer,
/// and a fresh review snapshot starts a fresh history rather than applying
/// offsets recorded against different text.
#[test]
fn selection_history_undoes_and_redoes_terminal_review_selections() {
    let (mut app, terminal) = terminal_app("{}");
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"first\r\nsecond\r\nthird".to_vec(),
    });
    let selected = |app: &mut App| {
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .review_selection_text()
    };
    send(&mut app, "Ctrl-\\");
    // Live output has no selection to go back from, and is not refused.
    send(&mut app, "Alt-u");
    assert_eq!(app.status, "no earlier selection");
    send(&mut app, "k");
    let caret = selected(&mut app);
    assert_eq!(caret.chars().count(), 1, "{caret:?}");
    send(&mut app, "x");
    send(&mut app, "x");
    assert_eq!(selected(&mut app), "second\nthird");
    assert_eq!(app.mode, Mode::Select);

    send(&mut app, "Alt-u");
    assert_eq!(app.status, "selection undo");
    assert_eq!(selected(&mut app), "second");
    assert_eq!(app.mode, Mode::Select);
    send(&mut app, "Alt-u");
    assert_eq!(selected(&mut app), caret);
    assert_eq!(app.mode, Mode::Normal);
    send(&mut app, "Alt-u");
    assert_eq!(app.status, "no earlier selection");

    send(&mut app, "Alt-U");
    assert_eq!(app.status, "selection redo");
    assert_eq!(selected(&mut app), "second");
    // A restored line selection still extends by line, as in a buffer.
    send(&mut app, "x");
    assert_eq!(selected(&mut app), "second\nthird");
    send(&mut app, "Alt-u");
    assert_eq!(selected(&mut app), "second");

    // Returning to live output discards the snapshot the offsets addressed.
    send(&mut app, "i");
    send(&mut app, "Ctrl-\\");
    send(&mut app, "k");
    send(&mut app, "Alt-u");
    assert_eq!(app.status, "no earlier selection");
    assert!(app.terminals.get(terminal).unwrap().reviewing());
}

/// A terminal's selection history belongs to the terminal, so a buffer shown
/// in the same pane keeps its own and neither restores into the other.
#[test]
fn terminal_and_buffer_selection_histories_stay_separate() {
    let (mut app, terminal) = terminal_app("{}");
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"first\r\nsecond".to_vec(),
    });
    send(&mut app, "Ctrl-\\");
    send(&mut app, "k");
    send(&mut app, "x");
    app.panes.get_mut(&app.active_pane).unwrap().terminal = None;
    app.mode = Mode::Normal;
    send(&mut app, "Alt-u");
    assert_eq!(app.status, "no earlier selection");
    app.panes.get_mut(&app.active_pane).unwrap().terminal = Some(terminal);
    send(&mut app, "Alt-u");
    assert_eq!(app.status, "selection undo");
    assert_eq!(
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .review_selection_text()
            .chars()
            .count(),
        1
    );
}

/// A pointer drag across terminal review is one selection-history step, as a
/// drag across a document is.
#[test]
fn a_terminal_review_drag_is_one_selection_history_step() {
    let (mut app, terminal) = terminal_app("{}");
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"first\r\nsecond".to_vec(),
    });
    send(&mut app, "Ctrl-\\");
    send(&mut app, "k");
    let caret = app
        .terminals
        .get_mut(terminal)
        .unwrap()
        .review_selection_text();
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let body = view.pane(app.active_pane).unwrap().body;
    for (kind, column, row) in [
        (PointerEventKind::Down(PointerButton::Left), 1, 0),
        (PointerEventKind::Drag(PointerButton::Left), 3, 0),
        (PointerEventKind::Drag(PointerButton::Left), 2, 1),
        (PointerEventKind::Up(PointerButton::Left), 2, 1),
    ] {
        app.handle_pointer(
            PointerEvent {
                kind,
                column: body.x + column,
                row: body.y + row,
                modifiers: Modifiers::NONE,
            },
            &view,
        )
        .unwrap();
    }
    assert_eq!(
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .review_selection_text(),
        "irst\nsec"
    );
    send(&mut app, "Alt-u");
    assert_eq!(app.status, "selection undo");
    assert_eq!(
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .review_selection_text(),
        caret
    );
    send(&mut app, "Alt-u");
    assert_eq!(app.status, "no earlier selection");
}

/// Restoring a selection keeps search navigation in step with it: a restored
/// match is the active one again, and from any other selection `n` / `N`
/// step to the nearest match past the caret.
#[test]
fn selection_history_keeps_terminal_search_steps_in_step() {
    let (mut app, terminal) = terminal_app("{}");
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"foo one\r\nfoo two\r\nfoo three".to_vec(),
    });
    let anchor = |app: &App| {
        app.terminals
            .get(terminal)
            .unwrap()
            .review_selection_anchor()
            .unwrap()
    };
    for spelling in ["Ctrl-\\", "g", "g", "l"] {
        send(&mut app, spelling);
    }
    assert_eq!(anchor(&app), 1);
    for spelling in ["/", "f", "o", "o", "Enter"] {
        send(&mut app, spelling);
    }
    assert_eq!(anchor(&app), 0, "{}", app.status);
    send(&mut app, "n");
    assert_eq!(anchor(&app), 8);

    send(&mut app, "Alt-u");
    assert_eq!(anchor(&app), 0);
    send(&mut app, "n");
    assert_eq!(anchor(&app), 8);

    // Back past the search to the caret, which is not a match.
    send(&mut app, "Alt-u");
    send(&mut app, "Alt-u");
    assert_eq!(anchor(&app), 1);
    send(&mut app, "n");
    assert_eq!(anchor(&app), 8);
    send(&mut app, "Alt-u");
    assert_eq!(anchor(&app), 1);
    send(&mut app, "N");
    assert_eq!(anchor(&app), 0);
}

#[test]
fn terminal_yank_and_failed_counted_paste_preserve_review_and_modes() {
    let (mut app, terminal) = terminal_app("{}");
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"foo bar foo".to_vec(),
    });
    app.mode = Mode::Select;
    app.terminals
        .get_mut(terminal)
        .unwrap()
        .search_review("foo", false)
        .unwrap();
    let selected = app
        .terminals
        .get_mut(terminal)
        .unwrap()
        .review_selection_text();
    send(&mut app, "y");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .review_selection_text(),
        selected
    );
    for key in ["n", "N"] {
        send(&mut app, key);
        assert_eq!(app.mode, Mode::Normal);
    }
    app.registers.insert(
        '"',
        Register {
            text: "x".into(),
            linewise: false,
            directory: None,
        },
    );
    for key in ["p", "P"] {
        app.mode = Mode::Select;
        send(&mut app, "3");
        send(&mut app, key);
        assert_eq!(app.mode, Mode::Select);
        assert!(app.status.contains("terminal input queue"));
    }
    app.set_system_clipboard(Box::new(MemoryClipboard(Arc::new(Mutex::new("x".into())))));
    for key in ["p", "P"] {
        app.mode = Mode::Select;
        for key in ["3", "Space", "c", key] {
            send(&mut app, key);
        }
        assert_eq!(app.mode, Mode::Select);
    }
}

#[test]
fn configured_modal_terminal_binding_replaces_the_default() {
    for mode in ["normal", "select"] {
        let owner = if mode == "normal" {
            "terminal"
        } else {
            "terminal-select"
        };
        let (mut app, terminal) = terminal_app(&format!("bind: {{{owner}: {{p: yank}}}}"));
        app.apply_terminal_output(TerminalOutput::Bytes {
            id: terminal,
            bytes: b"foo".to_vec(),
        });
        app.mode = if mode == "normal" {
            Mode::Normal
        } else {
            Mode::Select
        };
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .search_review("foo", false)
            .unwrap();
        let before = app.terminals.get(terminal).unwrap().input_generation();
        send(&mut app, "p");
        assert_eq!(
            app.terminals.get(terminal).unwrap().input_generation(),
            before
        );
        assert_eq!(app.registers[&'"'].text, "foo");
        assert_eq!(
            app.mode,
            if mode == "normal" {
                Mode::Normal
            } else {
                Mode::Select
            }
        );
    }
}

#[test]
fn terminal_actions_do_not_fall_through_to_the_hidden_document() {
    let mut app = App::new(Config::default(), None).unwrap();
    let before = app.active().selection.clone();
    for command in [
        EditorCommand::TerminalUndoPaste,
        EditorCommand::TerminalEnterInput,
        EditorCommand::TerminalPaste,
        EditorCommand::TerminalClipboardPasteText,
        EditorCommand::TerminalSearch,
        EditorCommand::TerminalSearchRegex,
        EditorCommand::TerminalSearchNext,
        EditorCommand::TerminalSearchPrevious,
    ] {
        app.execute_editor_command(command).unwrap();
        assert!(app.status_error, "{command:?}: {}", app.status);
        assert_eq!(app.status, "this action needs a terminal");
        assert_eq!(app.active().selection, before);
        assert_eq!(app.mode, Mode::Normal);
    }
}

#[test]
fn terminal_review_search_actions_preserve_mode_and_document_query() {
    let (mut app, terminal) = terminal_app("{}");
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"foo bar foo".to_vec(),
    });
    app.search.pattern = "document query".into();
    app.terminals
        .get_mut(terminal)
        .unwrap()
        .search_review("foo", false)
        .unwrap();
    for mode in [Mode::Normal, Mode::Select] {
        app.set_editing_role(mode);
        for command in [
            EditorCommand::TerminalSearchNext,
            EditorCommand::TerminalSearchPrevious,
        ] {
            app.execute_editor_command(command).unwrap();
            assert!(!app.status_error, "{}", app.status);
            assert_eq!(app.mode, mode);
            assert_eq!(app.search.pattern, "document query");
        }
    }
}

/// Independent dispatch inventory: a new terminal branch needs a meaning audit.
#[test]
fn every_terminal_dispatch_branch_has_a_reviewed_meaning() {
    use EditorCommand as C;
    use std::collections::BTreeSet;
    let terminal = [
        C::TerminalUndoPaste,
        C::TerminalEnterInput,
        C::TerminalClipboardPasteText,
        C::TerminalPaste,
        C::TerminalSearch,
        C::TerminalSearchNext,
        C::TerminalSearchPrevious,
        C::TerminalSearchRegex,
    ];
    let shared = [
        C::EnterNormalMode,
        C::EnterSelectMode,
        C::ClipboardYank,
        C::CollapseSelection,
        C::CopySelectionDown,
        C::CopySelectionUp,
        C::ExtendLineAbove,
        C::ExtendLineBelow,
        C::FitSelectionForNormalMode,
        C::GotoFile,
        C::GotoNextParagraph,
        C::GotoPreviousParagraph,
        C::GotoWindowBottom,
        C::GotoWindowCenter,
        C::GotoWindowTop,
        C::GotoWord,
        C::HalfPageDown,
        C::HalfPageUp,
        C::JumpBackward,
        C::JumpBackwardBuffer,
        C::JumpForward,
        C::JumpForwardBuffer,
        C::KeepPrimarySelection,
        C::MoveDown,
        C::MoveFileEnd,
        C::MoveFileStart,
        C::MoveFirstNonWhitespace,
        C::MoveLeft,
        C::MoveLineEnd,
        C::MoveLineStart,
        C::MoveLongWordBackward,
        C::MoveLongWordEnd,
        C::MoveLongWordForward,
        C::MoveRight,
        C::MoveUp,
        C::MoveWordBackward,
        C::MoveWordEnd,
        C::MoveWordForward,
        C::PageDown,
        C::PageUp,
        C::ScrollViewDown,
        C::ScrollViewUp,
        C::SelectAll,
        C::SelectLine,
        C::SelectLineUp,
        C::SelectionRedo,
        C::SelectionUndo,
        C::Yank,
        C::YankLine,
    ];
    let pending_4c2: [C; 0] = [];
    let expected = terminal
        .iter()
        .chain(&shared)
        .chain(&pending_4c2)
        .map(|command| format!("{command:?}"))
        .collect::<BTreeSet<_>>();
    let source = include_str!("../terminal_workflows.rs").replace("\r\n", "\n");
    for source in [source.clone(), source.replace("\n", "\r\n")] {
        let source = source.replace("\r\n", "\n");
        let source = source
            .split_once("pub(super) fn execute_terminal_command_count(")
            .expect("terminal workflow source must contain the counted command dispatcher")
            .1
            .split_once("\n    }\n")
            .expect("terminal command dispatcher must end with an indented closing brace")
            .0;
        let actual = source
            .lines()
            .filter_map(|line| {
                let line = line.trim().strip_prefix("| ").unwrap_or(line.trim());
                line.starts_with("Command::")
                    .then_some(line.split("=>").next().unwrap())
            })
            .flat_map(|pattern| {
                pattern.split("Command::").skip(1).map(|command| {
                    command
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect::<String>()
                })
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual, expected,
            "audit new terminal meanings before adding dispatch branches"
        );
    }
    for command in shared {
        assert!(
            command
                .descriptor()
                .view_overrides
                .iter()
                .any(|view| view.scope == BindingScope::Terminal),
            "{command:?} needs terminal view prose"
        );
    }
    for command in terminal {
        assert!(command.metadata().name.starts_with("terminal-"));
    }
}

#[test]
fn terminal_yank_line_preserves_review_until_default_y_collapses_it() {
    let (mut app, terminal) = terminal_app("{}");
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"alpha beta\r\ngamma".to_vec(),
    });
    app.terminals
        .get_mut(terminal)
        .unwrap()
        .search_review("beta", false)
        .unwrap();
    app.set_editing_role(Mode::Select);
    let before = app
        .terminals
        .get_mut(terminal)
        .unwrap()
        .review_selection_text();
    app.execute_editor_command(EditorCommand::YankLine).unwrap();
    assert_eq!(app.registers[&'"'].text, "alpha beta\n");
    assert!(app.registers[&'"'].linewise);
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .review_selection_text(),
        before
    );
    send(&mut app, "Y");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.registers[&'"'].text, "alpha beta\n");
    assert!(
        app.terminals
            .get(terminal)
            .unwrap()
            .review_selection_state()
            .unwrap()
            .1
            .primary()
            .is_empty()
    );
}

#[test]
fn terminal_yank_line_deduplicates_rows_touched_by_disjoint_ranges() {
    let (mut app, terminal) = terminal_app("{}");
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"alpha beta\r\ngamma delta".to_vec(),
    });
    let session = app.terminals.get_mut(terminal).unwrap();
    session.begin_review();
    let generation = session.review_selection_state().unwrap().0;
    let selection = Selection::new(
        vec![Range::point(0), Range::new(6, 13), Range::point(18)],
        1,
    );
    assert!(session.restore_review_selection(generation, selection.clone()));
    assert_eq!(
        session.review_selected_lines_text(),
        "alpha beta\ngamma delta\n"
    );
    assert_eq!(session.review_selection_state().unwrap().1, &selection);
}

#[test]
fn terminal_input_keys_resume_the_child_without_editing_the_hidden_document() {
    for spelling in ["i", "a", "I", "A", "o", "O"] {
        let (mut app, terminal) = terminal_app("{}");
        app.apply_terminal_output(TerminalOutput::Bytes {
            id: terminal,
            bytes: b"review output".to_vec(),
        });
        app.terminals.get_mut(terminal).unwrap().select_all_review();
        app.set_editing_role(Mode::Select);
        let text = app.active_buffer().text().to_string();
        let selection = app.active().selection.clone();
        let generation = app.terminals.get(terminal).unwrap().input_generation();
        send(&mut app, spelling);
        assert!(!app.status_error, "{spelling}: {}", app.status);
        assert_eq!(app.mode, Mode::Insert, "{spelling}");
        assert!(!app.terminals.get(terminal).unwrap().reviewing());
        assert_eq!(
            app.terminals.get(terminal).unwrap().input_generation(),
            generation
        );
        assert_eq!(app.active_buffer().text().to_string(), text);
        assert_eq!(app.active().selection, selection);
    }
}

#[test]
fn document_editing_actions_are_not_terminal_input_aliases() {
    let (mut app, terminal) = terminal_app("{}");
    for command in [
        EditorCommand::Undo,
        EditorCommand::EnterInsertMode,
        EditorCommand::AppendAfter,
        EditorCommand::InsertLineStart,
        EditorCommand::InsertLineEnd,
        EditorCommand::OpenLineAbove,
        EditorCommand::OpenLineBelow,
        EditorCommand::EnterReplaceMode,
    ] {
        app.terminals.get_mut(terminal).unwrap().begin_review();
        app.set_editing_role(Mode::Normal);
        let generation = app.terminals.get(terminal).unwrap().input_generation();
        app.execute_editor_command(command).unwrap();
        assert!(app.status_error, "{command:?}: {}", app.status);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.terminals.get(terminal).unwrap().reviewing());
        assert_eq!(
            app.terminals.get(terminal).unwrap().input_generation(),
            generation
        );
    }
}

#[cfg(unix)]
#[test]
fn terminal_undo_paste_is_separate_from_document_undo_and_preserves_mode() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".into()));
    let terminal = app.active_terminal().unwrap();
    assert!(app.terminals.get_mut(terminal).unwrap().send_text("paste"));
    app.set_editing_role(Mode::Normal);
    send(&mut app, "u");
    assert!(!app.status_error, "{}", app.status);
    assert!(app.status.contains("erase 5 pasted characters"));
    assert_eq!(app.mode, Mode::Normal);
    app.execute_editor_command(EditorCommand::TerminalUndoPaste)
        .unwrap();
    assert!(!app.status_error);
    assert!(app.status.contains("nothing Runyte sent"));
    assert!(
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .send_text("already run\n")
    );
    app.execute_editor_command(EditorCommand::TerminalUndoPaste)
        .unwrap();
    assert!(app.status_error);
    assert!(app.status.contains("already run"));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn whole_row_yank_handles_trimmed_blank_review_rows() {
    let (mut app, terminal) = terminal_app("{}");
    let session = app.terminals.get_mut(terminal).unwrap();
    session.begin_review();
    session.move_review(crate::terminal::ReviewMotion::HalfPageDown, false);
    assert_eq!(
        session.review_selection_state().unwrap().1.primary().head,
        12
    );
    assert_eq!(session.review_selected_lines_text(), "\n");
}

#[cfg(unix)]
#[test]
fn counted_terminal_undo_consumes_line_gesture_without_restoring_mode() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".into()));
    let terminal = app.active_terminal().unwrap();
    assert!(app.terminals.get_mut(terminal).unwrap().send_text("one"));
    app.set_editing_role(Mode::Normal);
    send(&mut app, "x");
    assert_eq!(app.mode, Mode::Select);
    send(&mut app, "2");
    send(&mut app, "u");
    assert_eq!(app.mode, Mode::Select);
    assert!(!app.status_error, "{}", app.status);
    assert!(app.status.contains("nothing Runyte sent"));
    send(&mut app, "x");
    app.execute_editor_command(EditorCommand::TerminalUndoPaste)
        .unwrap();
    assert_eq!(app.mode, Mode::Select);
    assert!(!app.status_error);
}

#[test]
fn terminal_input_refuses_missing_insert_role_before_discarding_review() {
    let config: Config = serde_yaml::from_str(
        "buffer-types:\n  terminal: {normal: terminal}\nkeys:\n  defaults: false\n  bind:\n    normal: {F6: open-command-palette}\n    select: {Esc: enter-normal-mode}\n    insert: {Esc: enter-normal-mode}\n    replace: {Esc: enter-normal-mode}\n    terminal: {p: null, P: null, i: null, a: null, I: null, A: null, o: null, O: null}\n",
    ).unwrap();
    let compiled =
        crate::keymap::configured::compile_configuration(&config, crate::keymap::default_keymap());
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    let mut app = App::new(config, None).unwrap();
    let terminal = app.terminals.insert_test_session(80, 24);
    app.panes.get_mut(&app.active_pane).unwrap().terminal = Some(terminal);
    app.set_editing_role(Mode::Normal);
    app.terminals.get_mut(terminal).unwrap().select_all_review();
    let before = app
        .terminals
        .get(terminal)
        .unwrap()
        .review_selection_state()
        .map(|(generation, selection)| (generation, selection.clone()));
    app.execute_editor_command(EditorCommand::TerminalEnterInput)
        .unwrap();
    assert!(app.status_error);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.terminals.get(terminal).unwrap().reviewing());
    assert_eq!(
        app.terminals
            .get(terminal)
            .unwrap()
            .review_selection_state()
            .map(|(generation, selection)| (generation, selection.clone())),
        before
    );
}

#[cfg(unix)]
#[test]
fn terminal_undo_paste_replays_against_only_the_latest_child_input() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".into()));
    let terminal = app.active_terminal().unwrap();
    app.set_editing_role(Mode::Normal);
    assert!(app.terminals.get_mut(terminal).unwrap().send_text("first"));
    for spelling in ["Space", "m", "m", "u", "Space", "m", "m"] {
        send(&mut app, spelling);
    }
    assert!(app.recording_macro.is_none());
    assert!(app.terminals.get_mut(terminal).unwrap().send_text("second"));
    for spelling in ["Space", "m", "r"] {
        send(&mut app, spelling);
    }
    finish_macro_replay(&mut app);
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.mode, Mode::Normal);
    assert!(matches!(
        app.terminals.get_mut(terminal).unwrap().undo_sent_text(),
        SentTextUndo::NothingSent
    ));
}

#[test]
fn terminal_configuration_rejects_document_editing_identities() {
    for name in [
        "undo",
        "enter-insert-mode",
        "append-after",
        "insert-line-start",
        "insert-line-end",
        "open-line-above",
        "open-line-below",
        "enter-replace-mode",
    ] {
        let mut config = Config::default();
        config
            .merge_keys_source(&format!("bind:\n  terminal:\n    F6: {name}\n"))
            .unwrap();
        let result = crate::keymap::configured::compile_configuration(
            &config,
            crate::keymap::default_keymap(),
        );
        assert!(!result.errors.is_empty(), "{name}");
    }
}

#[cfg(unix)]
#[test]
fn terminal_undo_refusal_stops_its_program_before_leaving_the_terminal() {
    let mut config = Config::default();
    config
        .merge_keys_source("bind: {terminal: {F6: [terminal-undo-paste, terminal-leave]}}")
        .unwrap();
    let mut app = App::new(config, None).unwrap();
    app.open_terminal(Some("/bin/cat".into()));
    let terminal = app.active_terminal().unwrap();
    assert!(
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .send_text("submitted\n")
    );
    app.set_editing_role(Mode::Normal);
    send(&mut app, "F6");
    assert!(app.status_error);
    assert_eq!(app.active_terminal(), Some(terminal));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn switching_terminal_sessions_discards_a_temporary_mode() {
    let (mut app, first) = terminal_app("{}");
    let second = app.terminals.insert_test_session(80, 24);
    app.set_editing_role(Mode::Normal);
    send(&mut app, "x");
    assert!(app.line_select.is_some());
    app.show_terminal(second);
    assert_ne!(app.active_terminal(), Some(first));
    assert!(app.line_select.is_none());
}
