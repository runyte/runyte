// SPDX-License-Identifier: MPL-2.0
use super::key_actions::{program_actions, program_app};
use super::*;
use crate::keymap::actions::Action;
use EditorCommand as C;

fn run(app: &mut App) {
    key(app, KeyCode::Function(6), Modifiers::NONE);
}
fn undo(app: &mut App) {
    app.execute_editor_command(C::Undo).unwrap();
}
fn redo(app: &mut App) {
    app.execute_editor_command(C::Redo).unwrap();
}

#[test]
fn successful_and_failed_program_edits_form_one_checkpoint() {
    for failed in [false, true] {
        let commands = if failed {
            vec![
                C::DeleteSelection,
                C::DeleteSelection,
                C::FindNextChar,
                C::DeleteSelection,
            ]
        } else {
            vec![C::DeleteSelection, C::DeleteSelection]
        };
        let mut app = program_app(&commands);
        seed(&mut app, "abcde");
        let before = app.active_buffer().history_len();
        run(&mut app);
        if failed {
            press(&mut app, 'z');
        }
        assert_eq!(text(&app), "cde");
        assert_eq!(app.active_buffer().history_len(), before + 1);
        undo(&mut app);
        assert_eq!(text(&app), "abcde");
        redo(&mut app);
        assert_eq!(text(&app), "cde");
    }
}

#[test]
fn insert_handoff_includes_deletion_and_typing_until_escape() {
    let mut app = program_actions(vec![
        Action::Editor(C::DeleteSelection),
        Action::Invocation(CommandInvocation::mode_entry(Mode::Insert)),
    ]);
    seed(&mut app, "abc");
    run(&mut app);
    assert_eq!(app.mode, Mode::Insert);
    for c in "xy".chars() {
        press(&mut app, c);
    }
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(text(&app), "xybc");
    undo(&mut app);
    assert_eq!(text(&app), "abc");
    redo(&mut app);
    assert_eq!(text(&app), "xybc");
}

#[test]
fn intermediate_mode_changes_do_not_split_the_program_checkpoint() {
    let mut app = program_actions(vec![
        Action::Editor(C::DeleteSelection),
        Action::Invocation(CommandInvocation::mode_entry(Mode::Insert)),
        Action::Editor(C::InsertNewline),
        Action::Invocation(CommandInvocation::mode_entry(Mode::Normal)),
        Action::Editor(C::DeleteSelection),
    ]);
    seed(&mut app, "abc");
    run(&mut app);
    let after = text(&app);
    assert_ne!(after, "abc");
    undo(&mut app);
    assert_eq!(text(&app), "abc");
    redo(&mut app);
    assert_eq!(text(&app), after);
}

#[test]
fn operand_resumption_and_both_surround_cancellation_points_close_one_group() {
    for cancel_at in [0, 1, 2] {
        let mut app = program_app(&[C::DeleteSelection, C::SurroundReplace, C::DeleteSelection]);
        seed(&mut app, "x(abc)");
        run(&mut app);
        if cancel_at > 0 {
            press(&mut app, '(');
        }
        if cancel_at < 2 {
            key(&mut app, KeyCode::Escape, Modifiers::NONE);
        } else {
            press(&mut app, '[');
        }
        let after = text(&app);
        undo(&mut app);
        assert_eq!(text(&app), "x(abc)");
        redo(&mut app);
        assert_eq!(text(&app), after);
        assert!(app.program_undo.is_none());
    }
}

#[test]
fn external_edit_during_a_wait_is_a_separate_checkpoint() {
    let mut app = program_app(&[C::DeleteSelection, C::FindNextChar, C::DeleteSelection]);
    seed(&mut app, "abcde");
    run(&mut app);
    let buffer = app.active().buffer;
    app.apply_to_buffer(buffer, &Transaction::insert(0, "x"));
    assert_eq!(app.awaiting_character_command(), None);
    assert_eq!(text(&app), "xbcde");
    undo(&mut app);
    assert_eq!(text(&app), "bcde");
    undo(&mut app);
    assert_eq!(text(&app), "abcde");
}

#[test]
fn focus_cancellation_commits_the_source_group_before_other_pane_edits() {
    let mut app = program_app(&[C::DeleteSelection, C::FindNextChar, C::DeleteSelection]);
    seed(&mut app, "abcde");
    let source = app.active_pane;
    app.execute_editor_command(C::SplitVertical).unwrap();
    let destination = app.active_pane;
    app.activate_pane(source);
    run(&mut app);
    app.activate_pane(destination);
    app.execute_editor_command(C::DeleteSelection).unwrap();
    undo(&mut app);
    assert_eq!(text(&app), "bcde");
    undo(&mut app);
    assert_eq!(text(&app), "abcde");
}

#[test]
fn explicit_undo_and_checkpoint_keep_their_history_meaning() {
    let mut app = program_app(&[C::DeleteSelection, C::Undo, C::DeleteSelection]);
    seed(&mut app, "abc");
    run(&mut app);
    assert_eq!(text(&app), "ac");
    undo(&mut app);
    assert_eq!(text(&app), "abc");

    let mut app = program_actions(vec![
        Action::Editor(C::DeleteSelection),
        Action::Invocation(CommandInvocation::mode_entry(Mode::Insert)),
        Action::Editor(C::CommitUndoCheckpoint),
    ]);
    seed(&mut app, "abc");
    run(&mut app);
    press(&mut app, 'x');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    undo(&mut app);
    assert_eq!(text(&app), "bc");
    undo(&mut app);
    assert_eq!(text(&app), "abc");
}

#[test]
fn selection_only_program_has_one_selection_step_and_no_text_checkpoint() {
    let mut app = program_app(&[C::MoveRight, C::FindNextChar, C::MoveRight]);
    seed(&mut app, "abcde");
    let before = app.active_buffer().history_len();
    run(&mut app);
    press(&mut app, 'c');
    assert_eq!(cursor(&app).col, 3);
    assert_eq!(app.active_buffer().history_len(), before);
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 0);
}

#[test]
fn counted_singleton_paste_is_one_checkpoint_when_typed_or_replayed() {
    for replay in [false, true] {
        let mut app = program_app(&[C::PasteAfter]);
        seed(&mut app, "a");
        app.registers.insert(
            '"',
            Register {
                text: "x".into(),
                linewise: false,
                directory: None,
            },
        );
        let inputs = vec![
            InputEvent::Key(KeyStroke::plain(KeyCode::Char('3'))),
            InputEvent::Key(KeyStroke::plain(KeyCode::Function(6))),
        ];
        if replay {
            app.macros.insert(DEFAULT_MACRO_REGISTER, inputs);
            app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
            finish_macro_replay(&mut app);
        } else {
            for input in inputs {
                app.handle_input(input).unwrap();
            }
        }
        assert_eq!(text(&app), "axxx");
        undo(&mut app);
        assert_eq!(text(&app), "a");
        redo(&mut app);
        assert_eq!(text(&app), "axxx");
    }
}

#[test]
fn separate_replayed_bindings_remain_separate_undo_steps() {
    let mut app = program_app(&[C::DeleteSelection, C::DeleteSelection]);
    seed(&mut app, "abcde");
    app.macros.insert(
        DEFAULT_MACRO_REGISTER,
        vec![InputEvent::Key(KeyStroke::plain(KeyCode::Function(6))); 2],
    );
    app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
    finish_macro_replay(&mut app);
    assert_eq!(text(&app), "e");
    undo(&mut app);
    assert_eq!(text(&app), "cde");
    undo(&mut app);
    assert_eq!(text(&app), "abcde");
}

#[test]
fn save_hooks_share_the_program_checkpoint() {
    let root = crate::test_support::OwnedTempDir::new().unwrap();
    let path = root.path().join("save.txt");
    fs::write(&path, "abc   ").unwrap();
    let mut app = program_app(&[C::DeleteSelection, C::Save]);
    app.open_file(path.clone()).unwrap();
    run(&mut app);
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(text(&app), "bc");
    assert_eq!(fs::read_to_string(&path).unwrap(), "bc");
    undo(&mut app);
    assert_eq!(text(&app), "abc   ");
    root.cleanup().unwrap();
}

#[test]
fn completed_edits_in_each_buffer_have_one_checkpoint() {
    let root = crate::test_support::OwnedTempDir::new().unwrap();
    let mut app = program_app(&[
        C::DeleteSelection,
        C::DeleteSelection,
        C::FocusWindowRight,
        C::DeleteSelection,
        C::DeleteSelection,
    ]);
    seed(&mut app, "abcde");
    let first = app.active_pane;
    let path = root.path().join("other.txt");
    fs::write(&path, "vwxyz").unwrap();
    app.split(Axis::Horizontal, Some(path)).unwrap();
    let second = app.active_pane;
    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 80,
            height: 32,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 30,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
    app.activate_pane(first);
    run(&mut app);
    assert_eq!(app.active_pane, second);
    assert_eq!(text(&app), "xyz");
    undo(&mut app);
    assert_eq!(text(&app), "vwxyz");
    app.activate_pane(first);
    assert_eq!(text(&app), "cde");
    undo(&mut app);
    assert_eq!(text(&app), "abcde");
    root.cleanup().unwrap();
}

#[test]
fn deferred_motion_cancellation_and_limit_keep_completed_selection_undo() {
    for cancel in [false, true] {
        let mut app = program_app(&[C::MoveRight]);
        seed(&mut app, &"x".repeat(2000));
        let inputs = "1000"
            .chars()
            .map(|c| InputEvent::Key(KeyStroke::plain(KeyCode::Char(c))))
            .chain(std::iter::once(InputEvent::Key(KeyStroke::plain(
                KeyCode::Function(6),
            ))))
            .collect();
        app.macros.insert(DEFAULT_MACRO_REGISTER, inputs);
        app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
        if !cancel {
            app.macro_replay.as_mut().unwrap().remaining_work = 150;
        }
        app.advance_macro_replay().unwrap();
        assert!(cursor(&app).col > 0);
        if cancel {
            key(&mut app, KeyCode::Escape, Modifiers::NONE);
        } else {
            finish_macro_replay(&mut app);
        }
        key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
        assert_eq!(cursor(&app).col, 0);
    }
}

#[test]
fn deferred_edits_cancel_as_one_checkpoint_without_absorbing_the_next_edit() {
    let mut app = program_app(&[C::PasteAfter]);
    seed(&mut app, "a");
    app.registers.insert(
        '"',
        Register {
            text: "x".into(),
            linewise: false,
            directory: None,
        },
    );
    let inputs = "1000"
        .chars()
        .map(|c| InputEvent::Key(KeyStroke::plain(KeyCode::Char(c))))
        .chain(std::iter::once(InputEvent::Key(KeyStroke::plain(
            KeyCode::Function(6),
        ))))
        .collect();
    app.macros.insert(DEFAULT_MACRO_REGISTER, inputs);
    app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
    app.advance_macro_replay().unwrap();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    let partial = text(&app);
    app.execute_editor_command(C::DeleteSelection).unwrap();
    undo(&mut app);
    assert_eq!(text(&app), partial);
    undo(&mut app);
    assert_eq!(text(&app), "a");
}

#[test]
fn unavailable_completion_does_not_split_default_insert_undo() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, 'i');
    for c in "foo".chars() {
        press(&mut app, c);
    }
    key(&mut app, KeyCode::Char('x'), Modifiers::CONTROL);
    for c in "bar".chars() {
        press(&mut app, c);
    }
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(text(&app), "foobar");
    undo(&mut app);
    assert_eq!(text(&app), "");
}
