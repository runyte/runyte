// SPDX-License-Identifier: MPL-2.0
use super::*;

fn configured(source: &str) -> App {
    let config = Config {
        keys: Some(serde_yaml::from_str(source).unwrap()),
        ..Config::default()
    };
    App::new(config, None).unwrap()
}

#[test]
fn action_binding_sequences_execute_and_group_selection_history() {
    let mut app = configured(
        "bind:\n  normal:\n    F6: [move-right, move-right]\n    F7: [select-all, yank]\n    F8: [select-all, enter-insert-mode]\n",
    );
    seed(&mut app, "abc def");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 2);
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 0);
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert_eq!(app.registers[&'"'].text, "abc def");
    // `yank` leaves the selection and mode as `select-all` set them.
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary(), Range::new(0, 6));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
}

#[test]
fn action_binding_counts_character_arguments_and_runtime_errors() {
    let mut app = configured(
        "bind:\n  normal:\n    F1: move-right\n    F2: [move-right, move-right]\n    F3: [move-right, find-next-char]\n    F4: [remove-primary-selection, move-right]\n",
    );
    seed(&mut app, "abc def");
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 3);
    press(&mut app, '2');
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 3);
    assert!(app.status_error);
    assert!(app.status.contains("multiple actions"));
    key(&mut app, KeyCode::Function(4), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 3);
    assert!(app.status_error);
    set_cursor(&mut app, 0, 0);
    key(&mut app, KeyCode::Function(3), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 1);
    press(&mut app, 'e');
    assert_eq!(cursor(&app).col, 5);
}

#[test]
fn action_bindings_preserve_insert_replace_modes_and_render_full_help() {
    let mut app = configured(
        "bind:\n  normal:\n    F1: [select-all, yank]\n    F2: {command: help, argument: key-actions}\n  insert:\n    F3: insert-newline\n  replace:\n    F3: enter-normal-mode\n",
    );
    seed(&mut app, "abc");
    press(&mut app, 'i');
    key(&mut app, KeyCode::Function(3), Modifiers::NONE);
    assert_eq!(app.active_buffer().text().to_string(), "\nabc");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'R');
    key(&mut app, KeyCode::Function(3), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .contains("select-line-up")
    );
    app.execute_editor_command(EditorCommand::ShowHelp).unwrap();
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .contains("select-all → yank")
    );
}

#[test]
fn action_binding_overwrites_do_not_advertise_old_keys_in_teaching_text() {
    let mut app = configured(
        "bind:\n  normal:\n    Space e: yank\n    F1: open-explorer\n  select:\n    Space e: yank\n    F1: open-explorer\n",
    );
    app.execute_command("about").unwrap();
    let body = app.active_buffer().text().to_string();
    assert!(body.contains("F1"));
    assert!(!body.contains("Space e"));
}

#[test]
fn action_bindings_replay_once_per_recorded_key_and_keep_prompts_owned() {
    let mut app = configured(
        "bind:\n  normal:\n    F1: [move-right, move-right]\n    F2: [move-right, search]\n",
    );
    seed(&mut app, "abcdef");
    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }
    assert_eq!(app.macros[&DEFAULT_MACRO_REGISTER].len(), 1);
    for stroke in [' ', 'm', 'r'] {
        press(&mut app, stroke);
    }
    finish_macro_replay(&mut app);
    assert_eq!(cursor(&app).col, 4);
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Command);
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(cursor(&app).col, 5);
}

/// Without the built-in keys, `Esc` still belongs to the program inside a
/// terminal, so `Ctrl-\\` has to stay the way out of its input.
#[cfg(unix)]
#[test]
fn without_defaults_a_terminal_can_still_be_left() {
    let root = super::language::temporary("standalone-terminal-escape");
    std::fs::create_dir_all(&root).unwrap();
    let mut app = configured(
        "defaults: false\nbind:\n  normal:\n    ':': open-command-palette\n  select:\n    Esc: enter-normal-mode\n  insert:\n    Esc: enter-normal-mode\n  replace:\n    Esc: enter-normal-mode\n",
    );
    assert!(!app.status_error, "{}", app.status);
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    assert!(app.active_terminal().is_some());
    assert_eq!(app.mode, Mode::Insert);

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert, "Esc went to the program");
    key(&mut app, KeyCode::Char('\\'), Modifiers::CONTROL);
    assert_eq!(app.mode, Mode::Normal);
    std::fs::remove_dir_all(root).unwrap();
}

fn register(app: &mut App, text: &str, linewise: bool) {
    app.registers.insert(
        '"',
        Register {
            text: text.into(),
            linewise,
            directory: None,
        },
    );
}

fn select(app: &mut App, range: Range) {
    app.mode = Mode::Select;
    app.panes.get_mut(&0).unwrap().selection = Selection::single(range);
}

#[test]
fn paste_after_pastes_past_a_range_without_replacing_it_or_leaving_select_mode() {
    let mut app = configured("bind:\n  select:\n    F5: paste-after\n");
    seed(&mut app, "abc def\nghi\njkl");
    register(&mut app, "XY", false);
    select(&mut app, Range::new(0, 2));
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(text(&app), "abcXY def\nghi\njkl");
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary(), Range::new(0, 2));

    // Whole lines go after the last row the range touches, not the head's.
    let mut app = configured("bind:\n  select:\n    F5: paste-after\n");
    seed(&mut app, "abc def\nghi\njkl");
    register(&mut app, "L\n", true);
    select(&mut app, Range::new(9, 0));
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(text(&app), "abc def\nghi\nL\njkl");
    assert_eq!(app.active().selection.primary(), Range::new(9, 0));

    // `p` still replaces the same range and ends in Normal mode.
    let mut app = configured("");
    seed(&mut app, "abc def\nghi\njkl");
    register(&mut app, "XY", false);
    select(&mut app, Range::new(0, 2));
    press(&mut app, 'p');
    assert_eq!(text(&app), "XY def\nghi\njkl");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn paste_after_stays_on_an_empty_last_row_and_pastes_after_every_range() {
    // A range ending on an empty row pastes on that row, as a caret there
    // would, rather than at the start of the next row.
    let mut app = configured("bind:\n  select:\n    F5: paste-after\n");
    seed(&mut app, "abc\n\ndef");
    register(&mut app, "XY", false);
    select(&mut app, Range::new(0, 4));
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(text(&app), "abc\nXY\ndef");

    // Every range gets its own paste, adjacent ranges included.
    let mut app = configured("bind:\n  select:\n    F5: paste-after\n");
    seed(&mut app, "ab cd");
    register(&mut app, "X", false);
    app.mode = Mode::Select;
    app.panes.get_mut(&0).unwrap().selection = Selection::new(
        vec![Range::new(0, 0), Range::new(1, 1), Range::new(3, 4)],
        0,
    );
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(text(&app), "aXbX cdX");
    // Each range stays on its own characters, the pasted text beside it.
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::new(0, 0), Range::new(2, 2), Range::new(5, 6)]
    );

    // A count repeats the paste after the same range.
    let mut app = configured("bind:\n  select:\n    F5: paste-after\n");
    seed(&mut app, "abc def");
    register(&mut app, "XY", false);
    select(&mut app, Range::new(0, 2));
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(text(&app), "abcXYXYXY def");
    assert_eq!(app.active().selection.primary(), Range::new(0, 2));
}

#[test]
fn replace_with_register_replaces_whole_lines_and_an_empty_rows_break() {
    let mut app = configured(
        "bind:\n  select:\n    F6: replace-with-register\n  normal:\n    F6: replace-with-register\n",
    );
    seed(&mut app, "abc\ndef\n");
    register(&mut app, "L\n", true);
    press(&mut app, 'x');
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(text(&app), "L\ndef\n");

    // On a caret on an empty row the span is the row's break, as for `d`.
    let mut app = configured("bind:\n  normal:\n    F6: replace-with-register\n");
    seed(&mut app, "abc\n\ndef");
    register(&mut app, "Z", false);
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::point(4));
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(text(&app), "abc\nZdef");
}

#[test]
fn a_search_step_that_finds_nothing_ends_its_sequence() {
    let mut app = configured(
        "bind:\n  normal:\n    F8: [search-next, delete-selection]\n    F9: [search-previous, delete-selection]\n",
    );
    seed(&mut app, "abc def");
    app.search.pattern = "zzz".into();
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.status.contains("pattern not found"), "{}", app.status);
    assert_eq!(text(&app), "abc def");
    key(&mut app, KeyCode::Function(9), Modifiers::NONE);
    assert_eq!(text(&app), "abc def");

    // A step that finds its match lets the sequence go on.
    app.search.pattern = "def".into();
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert_eq!(text(&app), "abc ");
}

#[test]
fn paste_after_a_half_open_range_keeps_the_range_off_the_pasted_text() {
    let mut app = configured("bind:\n  select:\n    F5: paste-after\n");
    seed(&mut app, "abc def");
    register(&mut app, "XY", false);
    select(&mut app, Range::new(0, 3));
    app.active_mut()
        .mark_selection_semantics(SelectionSemantics::HalfOpen);
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(text(&app), "abcXY def");
    assert_eq!(app.active().selection.primary(), Range::new(0, 3));
}

#[test]
fn replace_with_register_replaces_a_caret_or_range_and_keeps_the_mode() {
    let mut app = configured(
        "bind:\n  normal:\n    F6: replace-with-register\n  select:\n    F6: replace-with-register\n",
    );
    seed(&mut app, "abc def");
    register(&mut app, "Z", false);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    // A bare caret stands for the character under it, as it does for `d`;
    // `p` there would have pasted after it instead.
    assert_eq!(text(&app), "Zbc def");
    assert_eq!(app.mode, Mode::Normal);

    // It takes no count: repeating a replacement would only replace the copy,
    // so a count would look accepted while doing nothing.
    assert!(!crate::command::EditorCommand::ReplaceWithRegister.accepts_count());
    assert!(crate::command::EditorCommand::PasteAfter.accepts_count());

    select(&mut app, Range::new(4, 6));
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(text(&app), "Zbc Z");
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary(), Range::new(4, 4));
    // The register is left alone, so it can replace one range after another.
    assert_eq!(app.registers[&'"'].text, "Z");
}

#[test]
fn set_search_from_selection_sets_the_pattern_and_search_next_continues_from_it() {
    let mut app = configured(
        "bind:\n  normal:\n    F7: set-search-from-selection\n    F8: [set-search-from-selection, search-next]\n    F9: [set-search-from-selection, search-previous, yank]\n",
    );
    seed(&mut app, "foo bar foo baz bar");
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::point(4));
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert_eq!(app.search.pattern, "bar");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.active().selection.primary(), Range::point(4));
    press(&mut app, 'n');
    assert_eq!(app.active().selection.primary(), Range::new(16, 18));

    app.mode = Mode::Normal;
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::point(0));
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert_eq!(app.search.pattern, "foo");
    assert_eq!(app.active().selection.primary(), Range::new(8, 10));
    assert_eq!(app.mode, Mode::Select);

    // `search-previous` continues too: the yank after it copies the match.
    app.mode = Mode::Normal;
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::point(16));
    key(&mut app, KeyCode::Function(9), Modifiers::NONE);
    assert_eq!(app.active().selection.primary(), Range::new(4, 6));
    assert_eq!(app.registers[&'"'].text, "bar");
}

#[test]
fn yank_and_yank_line_keep_the_selection_and_mode_while_y_and_y_line_collapse() {
    let mut app =
        configured("bind:\n  select:\n    F5: yank\n    F6: yank-line\n    F7: [yank, indent]\n");
    seed(&mut app, "abc def\nghi");
    app.mode = Mode::Select;
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 2));
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(app.registers[&'"'].text, "abc");
    assert!(!app.registers[&'"'].linewise);
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary(), Range::new(0, 2));

    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.registers[&'"'].text, "abc def\n");
    assert!(app.registers[&'"'].linewise);
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary(), Range::new(0, 2));

    // A register chosen with `"` is the one the bare yank writes.
    press(&mut app, '"');
    press(&mut app, 'a');
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(app.registers[&'a'].text, "abc");

    // The range survives the yank, so the next action in a sequence acts on it.
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .starts_with(char::is_whitespace)
    );
    assert_eq!(app.mode, Mode::Select);

    // `y` and `Y` still end the gesture on a caret in Normal mode.
    seed(&mut app, "abc def\nghi");
    app.mode = Mode::Select;
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 2));
    press(&mut app, 'y');
    assert_eq!(app.registers[&'"'].text, "abc");
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.active().selection.primary().is_empty());

    app.mode = Mode::Select;
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 2));
    press(&mut app, 'Y');
    assert_eq!(app.registers[&'"'].text, "abc def\n");
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.active().selection.primary().is_empty());
}

#[test]
fn delete_and_change_selection_leave_registers_alone_while_d_and_c_yank() {
    let mut app = configured(
        "bind:\n  normal:\n    F5: delete-selection\n    F6: change-selection\n    F7: [select-all, delete-selection]\n",
    );
    seed(&mut app, "abc def");
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 2));
    press(&mut app, 'd');
    assert_eq!(app.registers[&'"'].text, "abc");
    assert_eq!(app.active_buffer().text().to_string(), " def");

    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(1, 3));
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(app.active_buffer().text().to_string(), " ");
    assert_eq!(app.registers[&'"'].text, "abc");

    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 0));
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.active_buffer().text().to_string(), "");
    assert_eq!(app.registers[&'"'].text, "abc");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    // A register chosen for a delete that writes nothing must not linger
    // and capture the next yank instead.
    seed(&mut app, "ghi jkl");
    press(&mut app, '"');
    press(&mut app, 'a');
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert!(!app.registers.contains_key(&'a'));
    press(&mut app, 'y');
    assert!(!app.registers.contains_key(&'a'));
    assert_eq!(app.registers[&'"'].text, "h");

    // `delete-selection` continues a sequence, so it can follow a selection.
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert_eq!(app.active_buffer().text().to_string(), "");
    assert_eq!(app.registers[&'"'].text, "h");

    press(&mut app, 'u');
    assert_eq!(app.active_buffer().text().to_string(), "hi jkl");
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 1));
    press(&mut app, 'c');
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.registers[&'"'].text, "hi");
}
