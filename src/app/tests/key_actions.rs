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
    // Both actions preserve the binding's starting mode.
    assert_eq!(app.mode, Mode::Normal);
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
    assert_eq!(cursor(&app).col, 6, "count reaches only the first move");
    assert!(!app.status_error);
    key(&mut app, KeyCode::Function(4), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 6);
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
        "bind:\n  normal:\n    F7: search-set-from-selection\n    F8: [search-set-from-selection, search-next]\n    F9: [search-set-from-selection, search-previous, yank]\n",
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
        "bind:\n  normal:\n    F5: delete-selection\n    F6: [delete-selection-for-change, enter-insert-mode]\n    F7: [select-all, delete-selection]\n",
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

#[test]
fn program_counts_skip_nonaccepting_steps_and_drop_unused_counts() {
    let mut app = configured(
        "bind: {normal: {F1: [yank, move-right, move-right], F2: [select-all, yank], F3: [yank, move-file-start]}}",
    );
    seed(&mut app, "abc def\nsecond\nthird\nfourth");
    press(&mut app, '2');
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 3);
    assert_eq!(app.registers[&'"'].text, "a");
    assert!(!app.status_error);
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 0, "one selection-history action");
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert!(!app.status_error);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.registers[&'"'].text,
        text(&app),
        "the entire program ran"
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(3), Modifiers::NONE);
    assert_eq!(
        cursor(&app).row,
        2,
        "an interpreted count is a line address"
    );
}

#[test]
fn singleton_bindings_drop_unused_counts_for_operands_and_invocations() {
    let mut app = configured(
        "bind: {normal: {F1: find-next-char, F2: {command: help, argument: key-actions}}}",
    );
    seed(&mut app, "axxb");
    press(&mut app, '2');
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    press(&mut app, 'x');
    assert_eq!(cursor(&app).col, 1);
    press(&mut app, '2');
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert!(!app.status_error);
    assert!(
        text(&app).contains("Bindable actions"),
        "counted parameterized singleton opened help"
    );
}

#[test]
fn counted_program_replay_keeps_steps_ordered_and_rejects_oversized_work() {
    for (program, count, accepted) in [
        ("[move-right, yank]", "3", true),
        ("[yank, move-right, yank]", "129", false),
        ("[yank, select-line, yank]", "129", false),
    ] {
        let mut app = configured(&format!("bind: {{normal: {{F1: {program}}}}}"));
        seed(&mut app, "abcd");
        register(&mut app, "sentinel", false);
        if accepted {
            for c in count.chars() {
                press(&mut app, c);
            }
            key(&mut app, KeyCode::Function(1), Modifiers::NONE);
            assert_eq!(app.registers[&'"'].text, "d");
            set_cursor(&mut app, 0, 0);
            register(&mut app, "sentinel", false);
        }
        let inputs = count
            .chars()
            .map(|c| InputEvent::Key(KeyStroke::new(KeyCode::Char(c), Modifiers::NONE)))
            .chain(std::iter::once(InputEvent::Key(KeyStroke::new(
                KeyCode::Function(1),
                Modifiers::NONE,
            ))))
            .collect();
        app.macros.insert(DEFAULT_MACRO_REGISTER, inputs);
        for c in [' ', 'm', 'r'] {
            press(&mut app, c);
        }
        finish_macro_replay(&mut app);
        if accepted {
            assert_eq!(
                app.registers[&'"'].text, "d",
                "tail follows all counted moves"
            );
            assert_eq!(cursor(&app).col, 3);
        } else {
            assert!(app.status_error);
            assert!(app.status.contains("per-action limit"));
            assert_eq!(
                app.registers[&'"'].text, "sentinel",
                "neither prefix nor tail ran"
            );
            assert_eq!(cursor(&app).col, 0);
        }
        assert!(!app.atomic_program_replay);
    }
}

#[test]
fn counted_program_replay_reserves_remaining_budget_before_prefix_effects() {
    let mut app = configured("bind: {normal: {F1: [yank, move-right, yank]}}");
    seed(&mut app, "abcd");
    register(&mut app, "sentinel", false);
    app.macros.insert(
        DEFAULT_MACRO_REGISTER,
        vec![
            InputEvent::Key(KeyStroke::new(KeyCode::Char('3'), Modifiers::NONE)),
            InputEvent::Key(KeyStroke::new(KeyCode::Function(1), Modifiers::NONE)),
        ],
    );
    app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
    app.macro_replay.as_mut().unwrap().remaining_work = 3;
    finish_macro_replay(&mut app);
    assert!(app.status_error);
    assert!(app.status.contains("safety limit"));
    assert_eq!(app.registers[&'"'].text, "sentinel");
    assert_eq!(cursor(&app).col, 0);
}

/// Construct internal programs to exercise execution independently of compilation.
pub(super) fn program_app(commands: &[EditorCommand]) -> App {
    use crate::keymap::{Binding, Key, actions::Action, program::Program};
    let mut app = configured("{}");
    let mut binding = Binding::implemented(
        &[Mode::Normal, Mode::Select],
        Key::plain(KeyCode::Function(6)),
        commands[0],
    );
    binding.program = Program::new(commands.iter().copied().map(Action::Editor).collect()).unwrap();
    binding.description = "resumable test program".into();
    let mut bindings = app.keymap.bindings().to_vec();
    bindings.push(binding);
    app.keymap = app.keymap.with_test_bindings(bindings).into();
    app
}

#[test]
fn program_resumes_after_each_operand_and_cancels_before_the_tail() {
    use EditorCommand as C;
    for cancel in [false, true] {
        let mut app = configured(
            "bind: {normal: {F6: [move-right, find-next-char, move-right, find-next-char, yank]}}",
        );
        seed(&mut app, "abxcdxef");
        key(&mut app, KeyCode::Function(6), Modifiers::NONE);
        assert_eq!(cursor(&app).col, 1);
        assert_eq!(app.awaiting_character_command(), Some(C::FindNextChar));
        press(&mut app, 'x');
        assert_eq!(cursor(&app).col, 3);
        assert_eq!(app.awaiting_character_command(), Some(C::FindNextChar));
        if cancel {
            key(&mut app, KeyCode::Escape, Modifiers::NONE);
            assert!(!app.registers.contains_key(&'"'));
            assert_eq!(cursor(&app).col, 3);
        } else {
            press(&mut app, 'e');
            assert_eq!(cursor(&app).col, 6);
            assert_eq!(app.registers[&'"'].text, "cdxe");
        }
        assert_eq!(app.awaiting_character_command(), None);
    }
}

#[test]
fn program_does_not_arm_later_operands_after_failure() {
    use EditorCommand as C;
    let mut app = program_app(&[C::RemovePrimarySelection, C::FindNextChar, C::Yank]);
    seed(&mut app, "abc");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.awaiting_character_command(), None);
    let mut app = program_app(&[C::FindNextChar, C::Yank]);
    seed(&mut app, "abc");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    press(&mut app, 'z');
    assert!(app.status_error);
    assert!(!app.registers.contains_key(&'"'));
}

#[test]
fn paused_program_refuses_a_changed_document_or_selection() {
    use EditorCommand as C;
    for edit in [false, true] {
        let mut app = program_app(&[C::ReplaceChar, C::Yank]);
        seed(&mut app, "abc");
        key(&mut app, KeyCode::Function(6), Modifiers::NONE);
        if edit {
            seed(&mut app, "xyz");
        } else {
            app.execute_editor_command(C::MoveRight).unwrap();
        }
        let before = app.active_buffer().text().to_string();
        press(&mut app, 'q');
        assert_eq!(app.active_buffer().text().to_string(), before);
        assert_eq!(app.awaiting_character_command(), None);
        assert!(!app.registers.contains_key(&'"'));
    }
}

#[test]
fn program_waits_for_both_surround_operands_and_preserves_its_tail() {
    for cancel_at in [0, 1, 2] {
        let mut app = configured("bind: {normal: {F6: [surround-replace, yank]}}");
        seed(&mut app, "(abc)");
        set_cursor(&mut app, 0, 2);
        key(&mut app, KeyCode::Function(6), Modifiers::NONE);
        if cancel_at > 0 {
            press(&mut app, '(');
        }
        if cancel_at < 2 {
            key(&mut app, KeyCode::Escape, Modifiers::NONE);
            assert_eq!(text(&app), "(abc)");
            assert!(!app.registers.contains_key(&'"'));
        } else {
            press(&mut app, '[');
            assert_eq!(text(&app), "[abc]");
            assert_eq!(app.registers[&'"'].text, "b");
        }
        assert_eq!(app.awaiting_character_command(), None);
    }
}

#[test]
fn program_counts_and_selection_history_span_operand_waits() {
    use EditorCommand as C;
    let mut app = program_app(&[C::FindNextChar, C::MoveRight, C::Yank]);
    seed(&mut app, "axbcdef");
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    press(&mut app, 'x');
    assert_eq!(cursor(&app).col, 4);
    assert_eq!(app.registers[&'"'].text, "d");
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 0);
}

#[test]
fn a_paused_program_never_resumes_in_another_pane() {
    use EditorCommand as C;
    let mut app = program_app(&[C::FindNextChar, C::Yank]);
    seed(&mut app, "axbxc");
    let first = app.active_pane;
    app.execute_editor_command(C::SplitVertical).unwrap();
    let second = app.active_pane;
    app.activate_pane(first);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    app.activate_pane(second);
    assert_eq!(app.awaiting_character_command(), None);
    assert!(app.status.contains("binding cancelled"), "{}", app.status);
    press(&mut app, 'l');
    assert_eq!(cursor(&app).col, 1);
    assert!(!app.registers.contains_key(&'"'));
    app.activate_pane(first);
    assert_eq!(app.awaiting_character_command(), None);
    press(&mut app, 'l');
    assert_eq!(cursor(&app).col, 1);
    assert!(!app.registers.contains_key(&'"'));
}

#[test]
fn replayed_operands_finish_counted_steps_before_the_program_tail() {
    use EditorCommand as C;
    let mut app = program_app(&[C::FindNextChar, C::MoveRight, C::Yank]);
    seed(&mut app, "axbcdef");
    app.macros.insert(
        DEFAULT_MACRO_REGISTER,
        vec![
            InputEvent::Key(KeyStroke::plain(KeyCode::Char('3'))),
            InputEvent::Key(KeyStroke::plain(KeyCode::Function(6))),
            InputEvent::Key(KeyStroke::plain(KeyCode::Char('x'))),
        ],
    );
    for c in [' ', 'm', 'r'] {
        press(&mut app, c);
    }
    finish_macro_replay(&mut app);
    assert_eq!(cursor(&app).col, 4);
    assert_eq!(app.registers[&'"'].text, "d");
    assert!(!app.atomic_program_replay);
}

#[test]
fn a_typed_program_resuming_inside_replay_must_reserve_its_work() {
    use EditorCommand as C;
    let mut app = program_app(&[C::FindNextChar, C::MoveRight, C::Yank]);
    seed(&mut app, "axbcdef");
    register(&mut app, "sentinel", false);
    for c in "129".chars() {
        press(&mut app, c);
    }
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    app.macros.insert(
        DEFAULT_MACRO_REGISTER,
        vec![InputEvent::Key(KeyStroke::plain(KeyCode::Char('x')))],
    );
    app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
    finish_macro_replay(&mut app);
    assert!(app.status_error);
    assert!(app.status.contains("per-action limit"));
    assert_eq!(cursor(&app).col, 0);
    assert_eq!(app.registers[&'"'].text, "sentinel");
}

#[test]
fn text_input_cannot_edit_behind_an_operand_wait_in_insert_mode() {
    use EditorCommand as C;
    let mut app = program_app(&[C::EnterInsertMode, C::ReplaceChar, C::Yank]);
    seed(&mut app, "abc");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.awaiting_character_command(), Some(C::ReplaceChar));
    app.handle_input(InputEvent::Text("bulk text".into()))
        .unwrap();
    assert_eq!(text(&app), "abc");
    assert_eq!(app.awaiting_character_command(), None);
    assert!(!app.registers.contains_key(&'"'));
}

#[test]
fn semantic_focus_cancels_the_source_panes_paused_program() {
    use EditorCommand as C;
    let mut app = program_app(&[C::MoveRight, C::FindNextChar, C::MoveRight]);
    seed(&mut app, "axbxc");
    app.execute_editor_command(C::SplitVertical).unwrap();
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
    app.execute_editor_command(C::FocusWindowLeft).unwrap();
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    app.execute_editor_command(C::FocusWindowRight).unwrap();
    assert_eq!(app.awaiting_character_command(), None);
    assert!(app.status.contains("binding cancelled"), "{}", app.status);
    app.execute_editor_command(C::FocusWindowLeft).unwrap();
    assert_eq!(app.awaiting_character_command(), None);
    press(&mut app, 'l');
    assert_eq!(cursor(&app).col, 2);
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 1);
}

#[test]
fn pointer_focus_cancels_the_source_panes_paused_program() {
    use EditorCommand as C;
    let mut app = program_app(&[C::MoveRight, C::FindNextChar, C::Yank]);
    seed(&mut app, "axbxc");
    app.execute_editor_command(C::SplitVertical).unwrap();
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
    let second = app.active_pane;
    app.execute_editor_command(C::FocusWindowLeft).unwrap();
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    let view = app.prepare_view(FrameGeometry {
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
    let area = view
        .panes
        .iter()
        .find(|p| p.pane_id == second)
        .unwrap()
        .body;
    for kind in [
        PointerEventKind::Down(PointerButton::Left),
        PointerEventKind::Up(PointerButton::Left),
    ] {
        app.handle_pointer(
            PointerEvent {
                kind,
                column: area.x,
                row: area.y,
                modifiers: Modifiers::NONE,
            },
            &view,
        )
        .unwrap();
    }
    assert_eq!(app.active_pane, second);
    assert_eq!(app.awaiting_character_command(), None);
    assert!(app.status.contains("binding cancelled"), "{}", app.status);
    app.execute_editor_command(C::FocusWindowLeft).unwrap();
    assert_eq!(app.awaiting_character_command(), None);
    press(&mut app, 'l');
    assert_eq!(cursor(&app).col, 2);
    assert!(!app.registers.contains_key(&'"'));
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 1);
}

#[test]
fn terminal_review_changes_invalidate_a_paused_program() {
    use EditorCommand as C;
    let mut app = program_app(&[C::FindNextChar, C::Yank]);
    let terminal = app.terminals.insert_test_session(80, 24);
    app.active_mut().terminal = Some(terminal);
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"abcdef".to_vec(),
    });
    app.terminals.get_mut(terminal).unwrap().begin_review();
    app.mode = Mode::Normal;
    app.panes.insert(1, Pane::new(0));
    app.layout.split(0, 1, Axis::Horizontal);
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
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.awaiting_character_command(), Some(C::FindNextChar));
    app.execute_editor_command(C::FocusWindowRight).unwrap();
    assert_eq!(app.active_pane, 1);
    app.execute_editor_command(C::FocusWindowLeft).unwrap();
    assert_eq!(app.awaiting_character_command(), None);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    app.execute_editor_command(C::MoveFileStart).unwrap();
    press(&mut app, 'x');
    assert_eq!(app.awaiting_character_command(), None);
    assert!(app.status.contains("binding cancelled"), "{}", app.status);
}

#[test]
fn replay_resumption_does_not_charge_a_counted_step_that_already_completed() {
    use EditorCommand as C;
    let mut app = program_app(&[C::MoveRight, C::FindNextChar, C::Yank]);
    seed(&mut app, &format!("{}z", "a".repeat(140)));
    for c in "129".chars() {
        press(&mut app, c);
    }
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 129);
    app.macros.insert(
        DEFAULT_MACRO_REGISTER,
        vec![InputEvent::Key(KeyStroke::plain(KeyCode::Char('z')))],
    );
    app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
    finish_macro_replay(&mut app);
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(cursor(&app).col, 140);
    assert_eq!(app.awaiting_character_command(), None);
}

#[test]
fn a_resumed_program_ending_in_another_pane_leaves_no_stale_history_origin() {
    use EditorCommand as C;
    let mut app = program_app(&[C::MoveRight, C::FindNextChar, C::FocusWindowRight]);
    seed(&mut app, "axbxc");
    app.panes.insert(1, Pane::new(0));
    app.layout.split(0, 1, Axis::Horizontal);
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
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    press(&mut app, 'b');
    assert_eq!(app.active_pane, 1);
    app.execute_editor_command(C::FocusWindowLeft).unwrap();
    assert_eq!(cursor(&app).col, 2);
    press(&mut app, 'l');
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 2);
}

#[test]
fn splitting_a_paused_pane_does_not_copy_its_program_history() {
    use EditorCommand as C;
    let mut app = program_app(&[C::MoveRight, C::FindNextChar, C::Yank]);
    seed(&mut app, "axbxc");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    app.execute_editor_command(C::SplitVertical).unwrap();
    assert_eq!(app.awaiting_character_command(), None);
    assert!(app.status.contains("binding cancelled"), "{}", app.status);
    press(&mut app, 'l');
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 1);
}

#[test]
fn a_prompt_that_splits_does_not_leak_its_selection_origin() {
    let mut app = configured("{}");
    seed(&mut app, "abcde");
    for c in ":vsplit".chars() {
        press(&mut app, c);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_ne!(app.active_pane, 0);
    press(&mut app, 'l');
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 0);
}

#[test]
fn swapping_a_paused_pane_cancels_before_moving_its_history() {
    use EditorCommand as C;
    let mut app = program_app(&[C::MoveRight, C::FindNextChar, C::Yank]);
    seed(&mut app, "axbxc");
    app.execute_editor_command(C::SplitVertical).unwrap();
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    app.execute_editor_command(C::SwapWindow).unwrap();
    assert_eq!(app.awaiting_character_command(), None);
    assert!(app.status.contains("binding cancelled"), "{}", app.status);
    press(&mut app, 'l');
    assert_eq!(cursor(&app).col, 2);
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 1);
    assert!(!app.registers.contains_key(&'"'));
}
#[test]
fn native_link_focus_cancels_the_source_panes_paused_program() {
    use EditorCommand as C;
    let mut app = program_app(&[C::MoveRight, C::FindNextChar, C::Yank]);
    seed(&mut app, "axbxc");
    app.execute_editor_command(C::SplitVertical).unwrap();
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
    app.native_media = true;
    let second = app.active_pane;
    app.execute_editor_command(C::FocusWindowLeft).unwrap();
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    let view = app.prepare_view(FrameGeometry {
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
    let area = view
        .panes
        .iter()
        .find(|p| p.pane_id == second)
        .unwrap()
        .body;
    for kind in [
        PointerEventKind::Down(PointerButton::Left),
        PointerEventKind::Up(PointerButton::Left),
    ] {
        app.handle_pointer(
            PointerEvent {
                kind,
                column: area.x + view.pane(second).unwrap().gutter_width as u16,
                row: area.y,
                modifiers: if cfg!(target_os = "macos") {
                    Modifiers::SUPER
                } else {
                    Modifiers::CONTROL
                },
            },
            &view,
        )
        .unwrap();
    }
    assert_eq!(app.active_pane, second);
    assert_eq!(app.awaiting_character_command(), None);
    assert!(app.status.contains("binding cancelled"), "{}", app.status);
    app.execute_editor_command(C::FocusWindowLeft).unwrap();
    assert_eq!(app.awaiting_character_command(), None);
    press(&mut app, 'l');
    assert_eq!(cursor(&app).col, 2);
    assert!(!app.registers.contains_key(&'"'));
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 1);
}

#[test]
fn configured_programs_continue_after_mode_operand_and_view_effects() {
    let mut app = configured(
        "bind: {normal: {F6: [enter-insert-mode, move-right, enter-normal-mode], F7: [find-next-char, move-right], F8: [show-help, move-file-end]}}",
    );
    seed(&mut app, "abxcd");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(cursor(&app).col, 1);
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert_eq!(
        app.awaiting_character_command(),
        Some(EditorCommand::FindNextChar)
    );
    press(&mut app, 'x');
    assert_eq!(cursor(&app).col, 3);
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(cursor(&app).row > 0);
}

#[test]
fn internal_programs_cannot_continue_after_deferred_macro_replay() {
    let mut app = program_app(&[
        EditorCommand::ReplayDefaultMacro,
        EditorCommand::DeleteSelection,
    ]);
    seed(&mut app, "abc");
    app.macros.insert(
        DEFAULT_MACRO_REGISTER,
        vec![InputEvent::Key(KeyStroke::plain(KeyCode::Char('l')))],
    );
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    finish_macro_replay(&mut app);
    assert_eq!(text(&app), "abc");
    assert_eq!(cursor(&app).col, 1);
}

#[test]
fn configured_grammar_and_tree_mode_changes_preserve_the_tail() {
    let mut app = configured(
        "bind: {normal: {F6: [{command: grammar, argument: runyte}, move-right], F7: [focus-directory-tree, close-window]}}",
    );
    seed(&mut app, "abc");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 1);
    // Tree focus changes the live scope; the supported close action still runs.
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert!(!app.directory_tree.visible);
}

#[test]
fn terminal_mode_changes_and_supplied_name_continue_the_program() {
    let mut app = configured(
        "bind: {normal: {F6: [enter-insert-mode, enter-normal-mode, leave-terminal], F7: [{command: rename-terminal, argument: build}, leave-terminal]}}",
    );
    let terminal = app.terminals.insert_test_session(80, 24);
    app.active_mut().terminal = Some(terminal);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert!(app.active_terminal().is_none());
    app.active_mut().terminal = Some(terminal);
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert!(app.active_terminal().is_none());
}

#[test]
fn newly_admitted_colon_program_still_stops_on_runtime_refusal() {
    let mut app = configured("bind: {normal: {F6: [close, move-right]}}");
    seed(&mut app, "abc");
    app.insert_text("x");
    let before = app.active().selection.clone();
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.active().selection, before);
}

#[test]
fn view_overlays_cannot_leave_an_orphaned_program_operand() {
    let root = crate::test_support::OwnedTempDir::new().unwrap();
    let path = root.path().join("sample.txt");
    fs::write(&path, "alpha beta").unwrap();
    for opener in ["open-buffer-picker", "path"] {
        for escape in [false, true] {
            let mut app = configured(&format!(
                "bind: {{normal: {{F6: [{opener}, find-next-char, delete-selection]}}}}"
            ));
            app.open_file(path.clone()).unwrap();
            key(&mut app, KeyCode::Function(6), Modifiers::NONE);
            assert!(app.has_input_overlay(), "{opener}");
            assert!(app.status_error, "{opener}");
            assert!(app.awaiting_character_command().is_none());
            if escape {
                key(&mut app, KeyCode::Escape, Modifiers::NONE);
            } else {
                press(&mut app, 'b');
            }
            assert!(app.awaiting_character_command().is_none());
            assert_eq!(text(&app), "alpha beta");
        }
    }
}

#[test]
fn jump_label_input_never_runs_an_internal_program_tail() {
    for cancel in [false, true] {
        let mut app = program_app(&[EditorCommand::GotoWord, EditorCommand::DeleteSelection]);
        seed(&mut app, "alpha beta");
        app.active_mut().wrap_width = 80;
        key(&mut app, KeyCode::Function(6), Modifiers::NONE);
        assert!(app.jump.is_some());
        assert_eq!(text(&app), "alpha beta");
        if cancel {
            key(&mut app, KeyCode::Escape, Modifiers::NONE);
        } else {
            press(&mut app, 's');
            assert_eq!(cursor(&app).col, 6);
        }
        assert!(app.jump.is_none());
        assert_eq!(text(&app), "alpha beta");
    }
}

#[test]
fn atomic_split_defaults_preserve_pasted_ranges_and_linewise_cut_distinctions() {
    for direction in ['p', 'P'] {
        let mut app = configured("");
        seed(&mut app, "abc");
        register(&mut app, "XY", false);
        select(&mut app, Range::new(0, 1));
        press(&mut app, direction);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(
            app.active().selection.primary(),
            if direction == 'p' {
                Range::new(0, 1)
            } else {
                Range::new(2, 3)
            }
        );
        assert_eq!(text(&app), if direction == 'p' { "XYc" } else { "XYabc" });
    }
    for newline in ["\n", "\r\n"] {
        for (action, expected_mode, linewise) in
            [('d', Mode::Normal, true), ('c', Mode::Insert, false)]
        {
            let mut app = configured("");
            seed(&mut app, &format!("abc{newline}def{newline}"));
            press(&mut app, 'x');
            press(&mut app, action);
            assert_eq!(app.mode, expected_mode);
            let register = &app.registers[&'"'];
            assert_eq!(register.linewise, linewise);
            assert_eq!(
                register.text,
                if linewise {
                    format!("abc{newline}")
                } else {
                    "abc".into()
                }
            );
            assert_eq!(
                text(&app),
                if linewise {
                    format!("def{newline}")
                } else {
                    format!("{newline}def{newline}")
                }
            );
        }
    }
}

#[test]
fn explicit_mode_entry_preserves_ranges_and_replaces_transient_line_mode() {
    let mut app = configured("");
    seed(&mut app, "abc\ndef");
    press(&mut app, 'x');
    let range = app.active().selection.clone();
    app.execute(CommandInvocation::mode_entry(Mode::Select))
        .unwrap();
    assert!(app.line_select.is_none());
    assert_eq!(app.active().selection, range);
    press(&mut app, 'l');
    assert_eq!(app.mode, Mode::Select);
    let range = app.active().selection.clone();
    app.execute(CommandInvocation::mode_entry(Mode::Normal))
        .unwrap();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.active().selection, range);
}

#[test]
fn atomic_selection_and_character_actions_leave_mode_to_the_program() {
    let mut app = configured(
        "bind:\n  normal:\n    F1: select-all\n  select:\n    F2: collapse-selection\n    F3: toggle-case\n    F4: replace-char\n",
    );
    seed(&mut app, "abc");
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, '%');
    assert_eq!(app.mode, Mode::Select);
    key(&mut app, KeyCode::Function(3), Modifiers::NONE);
    assert_eq!(text(&app), "ABC");
    assert_eq!(app.mode, Mode::Select);
    key(&mut app, KeyCode::Function(4), Modifiers::NONE);
    press(&mut app, 'z');
    assert_eq!(text(&app), "zzz");
    assert_eq!(app.mode, Mode::Select);
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Select);
    assert!(app.active().selection.primary().is_empty());
    press(&mut app, ';');
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, '%');
    press(&mut app, '~');
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, '%');
    press(&mut app, 'r');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(
        app.mode,
        Mode::Select,
        "cancelled operand must not enter Normal"
    );
    press(&mut app, 'r');
    press(&mut app, 'a');
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn mode_entry_rejects_missing_unknown_and_read_only_editing_modes() {
    use crate::command::parse_named_command;
    assert!(parse_named_command("mode-enter", None).is_err());
    assert!(parse_named_command("mode-enter", Some("typo")).is_err());
    let mut app = configured("bind:\n  normal:\n    F1: mode-enter\n");
    assert!(app.status_error);
    app.execute_editor_command(EditorCommand::ShowAbout)
        .unwrap();
    let original = text(&app);
    for mode in [Mode::Insert, Mode::Replace] {
        let result = app.execute(CommandInvocation::mode_entry(mode)).unwrap();
        assert!(matches!(result, CommandOutcome::UserError(_)));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(text(&app), original);
    }
}

#[test]
fn explicit_terminal_modes_own_review_without_editing_the_backing_buffer() {
    let mut app = configured("");
    seed(&mut app, "backing text");
    let terminal = app.terminals.insert_test_session(80, 24);
    app.active_mut().terminal = Some(terminal);
    app.execute(CommandInvocation::mode_entry(Mode::Select))
        .unwrap();
    assert_eq!(app.mode, Mode::Select);
    let result = app
        .execute(CommandInvocation::mode_entry(Mode::Replace))
        .unwrap();
    assert!(matches!(result, CommandOutcome::UserError(_)));
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(text(&app), "backing text");
    app.execute(CommandInvocation::mode_entry(Mode::Insert))
        .unwrap();
    assert_eq!(app.mode, Mode::Insert);
    app.execute(CommandInvocation::mode_entry(Mode::Normal))
        .unwrap();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(text(&app), "backing text");
}

#[test]
fn selection_fit_is_separate_from_mode_entry_and_keeps_append_escape_behavior() {
    let mut app = configured("bind: {insert: {F1: fit-selection-for-normal-mode}}");
    seed(&mut app, "abc");
    press(&mut app, 'a');
    press(&mut app, 'X');
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(cursor(&app).col, 1);
    app.execute(CommandInvocation::mode_entry(Mode::Normal))
        .unwrap();
    assert_eq!(cursor(&app).col, 1);
    assert_eq!(text(&app), "aXbc");
    press(&mut app, 'u');
    assert_eq!(text(&app), "abc");

    set_cursor(&mut app, 0, 0);
    press(&mut app, 'a');
    press(&mut app, 'Y');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(cursor(&app).col, 1);
}

#[test]
fn atomic_actions_finish_line_selection_without_restoring_its_previous_mode() {
    for action in [
        "select-all",
        "collapse-selection",
        "toggle-case",
        "replace-char",
        "fit-selection-for-normal-mode",
    ] {
        let mut app = configured(&format!("bind: {{select: {{F1: {action}}}}}"));
        seed(&mut app, "abc\ndef");
        press(&mut app, 'x');
        key(&mut app, KeyCode::Function(1), Modifiers::NONE);
        if action == "replace-char" {
            press(&mut app, 'z');
        }
        assert_eq!(app.mode, Mode::Select, "{action}");
        assert!(app.line_select.is_none(), "{action}");
        press(&mut app, 'l');
        assert_eq!(app.mode, Mode::Select, "{action} then motion");
    }
}

#[test]
fn search_match_primitives_preserve_mode_and_stop_when_query_has_no_matches() {
    let mut app = configured(
        "bind:\n  normal:\n    F6: [search-set-from-selection, search-select-all-matches]\n    F7: [search-select-all-matches, move-right]\n",
    );
    seed(&mut app, "foo bar foo");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::new(0, 2), Range::new(8, 10)]
    );
    app.search.pattern = "missing".into();
    let before = app.active().selection.clone();
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert_eq!(app.active().selection, before);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.status.contains("not found"));
}

#[test]
fn collapse_half_open_ranges_keeps_the_caret_on_the_selected_grapheme() {
    for (range, expected) in [
        (Range::new(0, 3), 1),
        (Range::new(3, 0), 0),
        (Range::new(0, 4), 3),
    ] {
        let mut app = configured("");
        seed(&mut app, "ae\u{301}\nb");
        app.mode = Mode::Select;
        app.active_mut().replace_selection(Selection::single(range));
        app.active_mut()
            .mark_selection_semantics(SelectionSemantics::HalfOpen);
        app.execute_editor_command(EditorCommand::CollapseSelection)
            .unwrap();
        assert_eq!(app.mode, Mode::Select);
        assert_eq!(app.active().selection.primary(), Range::point(expected));
    }
}

#[test]
fn counted_paste_inserts_all_copies_once_and_preserves_bundled_mode() {
    for (value, expected, end) in [("x", "xxxbc", 2), ("XY", "XYXYXYbc", 5)] {
        for replay in [false, true] {
            let mut app = configured("");
            seed(&mut app, "abc");
            register(&mut app, value, false);
            app.mode = Mode::Select;
            if replay {
                app.macros.insert(
                    DEFAULT_MACRO_REGISTER,
                    "3p".chars()
                        .map(|c| InputEvent::Key(KeyStroke::plain(KeyCode::Char(c))))
                        .collect(),
                );
                app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
                finish_macro_replay(&mut app);
            } else {
                press(&mut app, '3');
                press(&mut app, 'p');
            }
            assert_eq!(text(&app), expected);
            assert_eq!(app.mode, Mode::Normal);
            assert_eq!(app.active().selection.primary(), Range::new(0, end));
            press(&mut app, 'u');
            assert_eq!(text(&app), "abc");
        }
    }
    let mut app = configured("bind: {select: {F6: paste-after-or-replace}}");
    seed(&mut app, "abc");
    register(&mut app, "x", false);
    app.mode = Mode::Select;
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(text(&app), "xxxbc");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn clipboard_paste_defaults_count_once_and_empty_sources_do_not_change_mode() {
    for key_char in ['p', 'P'] {
        let mut app = configured("");
        seed(&mut app, "abc");
        app.mode = Mode::Select;
        app.set_system_clipboard(Box::new(MemoryClipboard(Arc::new(Mutex::new("x".into())))));
        for c in ['3', ' ', 'c', key_char] {
            press(&mut app, c);
        }
        assert_eq!(text(&app), if key_char == 'p' { "xxxbc" } else { "xxxabc" });
        assert_eq!(app.mode, Mode::Normal);
        press(&mut app, 'u');
        assert_eq!(text(&app), "abc");
    }
    for clipboard in [false, true] {
        let mut app = configured("");
        seed(&mut app, "abc");
        app.mode = Mode::Select;
        if clipboard {
            app.set_system_clipboard(Box::new(MemoryClipboard(Arc::new(Mutex::new(
                String::new(),
            )))));
            press(&mut app, ' ');
            press(&mut app, 'c');
        }
        press(&mut app, 'p');
        assert_eq!(app.mode, Mode::Select);
        assert_eq!(text(&app), "abc");
    }
}

#[test]
fn cut_for_change_keeps_end_of_line_caret_and_does_not_leak_undo_group() {
    let mut app = configured("");
    seed(&mut app, "abc");
    select(&mut app, Range::point(2));
    app.execute_editor_command(EditorCommand::CutSelectionForChange)
        .unwrap();
    assert_eq!(text(&app), "ab");
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary(), Range::point(2));
    app.edit(Transaction::insert(0, "X"));
    app.undo();
    assert_eq!(text(&app), "ab");
    app.undo();
    assert_eq!(text(&app), "abc");
}

#[test]
fn search_extension_adds_primary_wraps_and_converts_half_open_ranges() {
    let mut app = configured("");
    seed(&mut app, "ab cd ab");
    app.search = SearchQuery {
        pattern: "ab".into(),
        mode: SearchMode::Sensitive,
        region: None,
    };
    app.mode = Mode::Select;
    app.active_mut()
        .replace_selection(Selection::single(Range::new(2, 0)));
    app.active_mut()
        .mark_selection_semantics(SelectionSemantics::HalfOpen);
    app.execute_editor_command(EditorCommand::SearchExtendNext)
        .unwrap();
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::new(1, 0), Range::new(6, 7)]
    );
    assert_eq!(app.active().selection.primary(), Range::new(6, 7));
    app.execute_editor_command(EditorCommand::Yank).unwrap();
    assert_eq!(app.registers[&'"'].text, "ab\nab");
    app.execute_editor_command(EditorCommand::SearchExtendNext)
        .unwrap();
    assert_eq!(app.active().selection.ranges().len(), 2);
    assert_eq!(app.active().selection.primary(), Range::new(0, 1));
    app.execute_editor_command(EditorCommand::SearchExtendPrevious)
        .unwrap();
    assert_eq!(app.active().selection.primary(), Range::new(6, 7));
}

#[test]
fn oversized_counted_paste_fails_before_editing_and_transient_yank_retains_linewise_data() {
    let mut app = configured("bind: {select: {F6: yank}}");
    seed(&mut app, "abc\ndef");
    press(&mut app, 'x');
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.registers[&'"'].linewise);
    assert_eq!(app.registers[&'"'].text, "abc\n");
    assert!(app.line_select.is_none());
    let invocation = CommandInvocation::editor(
        EditorCommand::PasteBefore,
        CommandExecutionContext::resolved(std::num::NonZeroUsize::new(usize::MAX).unwrap(), None),
    )
    .unwrap();
    app.execute(invocation).unwrap();
    assert_eq!(text(&app), "abc\ndef");
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.status.contains("64 MiB"));
}

#[test]
fn standalone_clipboard_input_normalizes_after_leaving_insert_or_replace() {
    for mode in [Mode::Insert, Mode::Replace] {
        for event in [
            InputEvent::ClipboardPaste,
            InputEvent::Key(KeyStroke::ctrl('v')),
        ] {
            for original in ["ab", "ab\ncd"] {
                let mut app = configured("");
                seed(&mut app, original);
                app.mode = mode;
                app.active_mut().replace_selection(Selection::point(2));
                app.set_system_clipboard(Box::new(MemoryClipboard(Arc::new(Mutex::new(
                    "x".into(),
                )))));
                // Prior failed programs must not suppress standalone success.
                app.sequence_halted = true;
                app.handle_input(event.clone()).unwrap();
                assert_eq!(text(&app), if original == "ab" { "abx" } else { "abx\ncd" });
                assert_eq!(app.mode, Mode::Normal);
                assert_eq!(app.active().selection.primary(), Range::point(2));
            }
        }
    }
}

#[test]
fn counted_paste_handles_multiple_ranges_before_and_linewise_crlf() {
    let mut app = configured("");
    seed(&mut app, "abc def");
    register(&mut app, "XY", false);
    app.mode = Mode::Select;
    app.active_mut()
        .replace_selection(Selection::new(vec![Range::point(0), Range::new(4, 6)], 1));
    for c in ['2', 'p'] {
        press(&mut app, c);
    }
    assert_eq!(text(&app), "XYXYbc XYXY");
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::new(0, 3), Range::new(7, 10)]
    );
    assert_eq!(app.active().selection.primary_index(), 1);
    for ending in ["\n", "\r\n"] {
        let mut app = configured("");
        seed(&mut app, &format!("abc{ending}def{ending}"));
        register(&mut app, &format!("x{ending}"), true);
        for c in ['x', '2', 'p'] {
            press(&mut app, c);
        }
        assert_eq!(text(&app), format!("x{ending}x{ending}def{ending}"));
        assert_eq!(app.mode, Mode::Normal);
    }
    let mut app = configured("");
    seed(&mut app, "abc");
    register(&mut app, "x", false);
    select(&mut app, Range::new(0, 1));
    for c in ['3', 'P'] {
        press(&mut app, c);
    }
    assert_eq!(text(&app), "xxxabc");
    assert_eq!(app.active().selection.primary(), Range::new(3, 4));
    assert_eq!(app.mode, Mode::Normal);
}

fn conditional_app(program: crate::keymap::program::Program) -> App {
    use crate::keymap::{Binding, Key};
    let mut app = configured("{}");
    let mut binding = Binding::implemented(
        &[Mode::Normal, Mode::Select],
        Key::plain(KeyCode::Function(6)),
        EditorCommand::Yank,
    );
    binding.program = program;
    binding.description = "conditional test program".into();
    let mut bindings = app.keymap.bindings().to_vec();
    bindings.push(binding);
    app.keymap = app.keymap.with_test_bindings(bindings).into();
    app
}

#[test]
fn conditional_programs_observe_live_mode_and_route_counts_only_into_taken_branches() {
    use crate::keymap::{
        actions::Action as A,
        program::{Condition as C, Program as P, Step as S},
    };
    let program = P::from_steps(vec![
        S::Action(A::Invocation(CommandInvocation::mode_entry(Mode::Select))),
        S::If {
            condition: C::Mode(Mode::Select),
            then: Box::new(P::single(EditorCommand::MoveRight.into())),
            otherwise: Box::new(P::single(EditorCommand::FindNextChar.into())),
        },
        S::Action(A::Editor(EditorCommand::MoveRight)),
    ])
    .unwrap();
    let mut app = conditional_app(program);
    seed(&mut app, "abcdef");
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary().head, 4);
    assert!(app.grammar.awaiting_character().is_none());
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(app.active().selection.primary().head, 0);

    let program = P::from_steps(vec![
        S::If {
            condition: C::Mode(Mode::Select),
            then: Box::new(P::single(EditorCommand::MoveLeft.into())),
            otherwise: Box::new(P::from_steps(vec![]).unwrap()),
        },
        S::Action(A::Editor(EditorCommand::MoveRight)),
    ])
    .unwrap();
    let mut app = conditional_app(program);
    seed(&mut app, "abcdef");
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.active().selection.primary().head, 3);
}

#[test]
fn conditional_operands_resume_outer_tails_and_cancel_the_whole_program() {
    use crate::keymap::{
        actions::Action as A,
        program::{Condition as C, Program as P, Step as S},
    };
    for cancel in [false, true] {
        let program = P::from_steps(vec![
            S::If {
                condition: C::Mode(Mode::Normal),
                then: Box::new(P::single(EditorCommand::FindNextChar.into())),
                otherwise: Box::new(P::from_steps(vec![]).unwrap()),
            },
            S::Action(A::Editor(EditorCommand::MoveRight)),
        ])
        .unwrap();
        let mut app = conditional_app(program);
        seed(&mut app, "abc def");
        key(&mut app, KeyCode::Function(6), Modifiers::NONE);
        assert_eq!(
            app.grammar.awaiting_character(),
            Some(EditorCommand::FindNextChar)
        );
        if cancel {
            key(&mut app, KeyCode::Escape, Modifiers::NONE);
        } else {
            press(&mut app, 'd');
        }
        assert_eq!(
            app.active().selection.primary().head,
            if cancel { 0 } else { 5 }
        );
        assert!(app.grammar.awaiting_character().is_none());
    }
}

#[test]
fn conditional_edits_share_undo_and_failure_stops_the_outer_tail() {
    use crate::keymap::{
        actions::Action as A,
        program::{Condition as C, Program as P, Step as S},
    };
    let program = P::from_steps(vec![
        S::Action(A::Editor(EditorCommand::ToggleCase)),
        S::If {
            condition: C::Mode(Mode::Normal),
            then: Box::new(
                P::new(vec![
                    A::Editor(EditorCommand::MoveRight),
                    A::Editor(EditorCommand::ToggleCase),
                ])
                .unwrap(),
            ),
            otherwise: Box::new(P::from_steps(vec![]).unwrap()),
        },
    ])
    .unwrap();
    let mut app = conditional_app(program);
    seed(&mut app, "abc");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(text(&app), "ABc");
    press(&mut app, 'u');
    assert_eq!(text(&app), "abc");

    let program = P::from_steps(vec![
        S::If {
            condition: C::Mode(Mode::Normal),
            then: Box::new(P::single(EditorCommand::SearchNext.into())),
            otherwise: Box::new(P::from_steps(vec![]).unwrap()),
        },
        S::Action(A::Editor(EditorCommand::ToggleCase)),
    ])
    .unwrap();
    let mut app = conditional_app(program);
    seed(&mut app, "abc");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(text(&app), "abc");
}

#[test]
fn select_toggle_settles_line_gestures_and_converts_half_open_endpoints() {
    for line_key in ['x', 'X'] {
        let mut app = configured("{}");
        seed(&mut app, "abc\ndef\n");
        press(&mut app, line_key);
        let before = app.active().selection.clone();
        press(&mut app, 'v');
        assert_eq!(app.mode, Mode::Select);
        assert_eq!(app.active().selection, before);
        assert!(app.line_select.is_none());
        press(&mut app, 'v');
        assert_eq!(app.mode, Mode::Normal);
    }
    let mut app = configured("{}");
    seed(&mut app, "abc\ndef");
    app.mode = Mode::Select;
    app.active_mut()
        .replace_selection(Selection::single(Range::new(0, 4)));
    app.active_mut()
        .mark_selection_semantics(SelectionSemantics::HalfOpen);
    press(&mut app, 'v');
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.active().selection.primary(), Range::point(3));
}

#[test]
fn conditional_replay_checks_selected_command_and_range_work_before_effects() {
    use crate::keymap::program::{Condition, Program};
    for command in [EditorCommand::MoveRight, EditorCommand::SelectLine] {
        for (count, budget, expected) in [
            ("129", None, "per-action limit"),
            ("3", Some(3), "safety limit"),
        ] {
            let mut app = conditional_app(Program::conditional(
                Condition::Mode(Mode::Normal),
                Program::single(command.into()),
                Program::from_steps(vec![]).unwrap(),
            ));
            seed(&mut app, "abc\ndef\nghi\n");
            let selection = app.active().selection.clone();
            let inputs = count
                .chars()
                .map(|c| InputEvent::Key(KeyStroke::new(KeyCode::Char(c), Modifiers::NONE)))
                .chain(std::iter::once(InputEvent::Key(KeyStroke::new(
                    KeyCode::Function(6),
                    Modifiers::NONE,
                ))))
                .collect();
            app.macros.insert(DEFAULT_MACRO_REGISTER, inputs);
            app.replay_macro(DEFAULT_MACRO_REGISTER, 1).unwrap();
            if let Some(budget) = budget {
                app.macro_replay.as_mut().unwrap().remaining_work = budget;
            }
            finish_macro_replay(&mut app);
            assert!(app.status_error);
            assert!(app.status.contains(expected), "{}", app.status);
            assert_eq!(app.active().selection, selection);
            assert!(!app.atomic_program_replay);
        }
    }
}

#[test]
fn recording_default_is_start_only_but_default_binding_stops_any_recording() {
    let mut app = configured("{}");
    app.start_macro_recording('a');
    app.record_default_macro();
    assert!(app.status_error);
    assert_eq!(app.recording_macro, Some('a'));
    for c in [' ', 'm', 'm'] {
        press(&mut app, c);
    }
    assert!(app.recording_macro.is_none());
    assert!(app.macro_staging.is_empty());
    assert!(app.macros[&'a'].is_empty());
}

#[test]
fn conditional_macro_calls_keep_frame_scheduling_beyond_atomic_repeat_limit() {
    use crate::keymap::program::{Condition, Program};
    let mut app = conditional_app(Program::conditional(
        Condition::Mode(Mode::Normal),
        Program::single(EditorCommand::ReplayDefaultMacro.into()),
        Program::from_steps(vec![]).unwrap(),
    ));
    seed(&mut app, &"a".repeat(160));
    app.macros.insert(
        DEFAULT_MACRO_REGISTER,
        vec![InputEvent::Key(KeyStroke::new(
            KeyCode::Char('l'),
            Modifiers::NONE,
        ))],
    );
    app.macros.insert(
        'a',
        "129"
            .chars()
            .map(|c| InputEvent::Key(KeyStroke::new(KeyCode::Char(c), Modifiers::NONE)))
            .chain(std::iter::once(InputEvent::Key(KeyStroke::new(
                KeyCode::Function(6),
                Modifiers::NONE,
            ))))
            .collect(),
    );
    app.replay_macro('a', 1).unwrap();
    finish_macro_replay(&mut app);
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.active().selection.primary().head, 129);
}

#[test]
fn replace_with_register_preserves_mode_on_refusal_and_consumes_line_selection() {
    let mut app = configured(
        "bind: {normal: {F6: replace-with-register}, select: {F6: replace-with-register}}",
    );
    seed(&mut app, "abc\ndef");
    register(&mut app, "XY\n", true);
    press(&mut app, 'x');
    let mode = app.mode;
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(text(&app), "XY\ndef");
    assert_eq!(app.mode, mode);
    assert!(app.line_select.is_none());
    press(&mut app, 'l');
    assert_eq!(app.mode, mode);

    app.open_help();
    press(&mut app, 'x');
    let mode = app.mode;
    assert!(app.line_select.is_some());
    let original = text(&app);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.mode, mode);
    assert!(app.line_select.is_none());
    assert_eq!(text(&app), original);
    press(&mut app, 'l');
    assert_eq!(app.mode, mode);

    let terminal = app.terminals.insert_test_session(80, 24);
    app.active_mut().terminal = Some(terminal);
    app.mode = Mode::Normal;
    press(&mut app, 'x');
    let mode = app.mode;
    assert!(app.line_select.is_some());
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.mode, mode);
    assert!(app.line_select.is_none());
}

#[test]
fn no_yank_delete_actions_preserve_mode_and_distinct_caret_seams() {
    use crate::keymap::program::Program;
    for mode in [Mode::Normal, Mode::Select] {
        for (command, head) in [
            (EditorCommand::DeleteSelection, 0),
            (EditorCommand::DeleteSelectionForChange, 1),
        ] {
            let mut app = conditional_app(Program::single(command.into()));
            seed(&mut app, "ab");
            app.mode = mode;
            app.active_mut().selection = Selection::single(Range::new(1, 1));
            register(&mut app, "saved", false);
            app.selected_register = 'a';
            key(&mut app, KeyCode::Function(6), Modifiers::NONE);
            assert_eq!(text(&app), "a");
            assert_eq!(app.mode, mode);
            assert_eq!(app.active().selection.primary().head, head);
            assert_eq!(app.registers[&'"'].text, "saved");
            assert!(!app.registers.contains_key(&'a'));
            assert_eq!(app.selected_register, '"');
        }
    }
}

#[test]
fn no_yank_delete_actions_distinguish_whole_rows_and_retire_the_gesture() {
    use crate::keymap::program::Program;
    for gesture in ['x', 'X'] {
        for (command, expected) in [
            (EditorCommand::DeleteSelection, "ab"),
            (EditorCommand::DeleteSelectionForChange, "ab\n"),
        ] {
            let mut app = conditional_app(Program::single(command.into()));
            seed(&mut app, "ab\ncd");
            app.active_mut().selection = Selection::single(Range::new(3, 3));
            press(&mut app, gesture);
            let mode = app.mode;
            assert!(app.line_select.is_some());
            key(&mut app, KeyCode::Function(6), Modifiers::NONE);
            assert_eq!(text(&app), expected, "{command:?}, {gesture}");
            assert_eq!(app.mode, mode);
            assert!(app.line_select.is_none());
            press(&mut app, 'h');
            assert_eq!(app.mode, mode);
        }
    }
}

#[test]
fn no_yank_change_program_preserves_the_seam_and_groups_typing_with_delete() {
    use crate::keymap::{
        actions::Action,
        program::{Program, Step},
    };
    let program = Program::from_steps(vec![
        Step::Action(Action::Editor(EditorCommand::DeleteSelectionForChange)),
        Step::Action(Action::Invocation(CommandInvocation::mode_entry(
            Mode::Insert,
        ))),
    ])
    .unwrap();
    let mut app = conditional_app(program);
    seed(&mut app, "ab");
    app.active_mut().selection = Selection::single(Range::new(1, 1));
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
    press(&mut app, 'z');
    assert_eq!(text(&app), "az");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'u');
    assert_eq!(text(&app), "ab");
}

#[test]
fn no_yank_delete_refusals_preserve_mode_and_consume_line_gestures() {
    use crate::keymap::program::Program;
    for command in [
        EditorCommand::DeleteSelection,
        EditorCommand::DeleteSelectionForChange,
    ] {
        for terminal in [false, true] {
            let mut app = conditional_app(Program::single(command.into()));
            if terminal {
                let id = app.terminals.insert_test_session(80, 24);
                app.active_mut().terminal = Some(id);
            } else {
                app.open_help();
            }
            press(&mut app, 'x');
            let mode = app.mode;
            let original = text(&app);
            assert!(app.line_select.is_some());
            key(&mut app, KeyCode::Function(6), Modifiers::NONE);
            assert!(app.status_error);
            assert_eq!(app.mode, mode);
            assert_eq!(text(&app), original);
            assert!(app.line_select.is_none());
        }
    }
}

#[test]
fn no_yank_delete_program_enters_normal_only_after_success() {
    use crate::keymap::{
        actions::Action,
        program::{Program, Step},
    };
    let program = Program::from_steps(vec![
        Step::Action(Action::Editor(EditorCommand::DeleteSelection)),
        Step::Action(Action::Invocation(CommandInvocation::mode_entry(
            Mode::Normal,
        ))),
    ])
    .unwrap();
    let mut app = conditional_app(program);
    seed(&mut app, "ab");
    app.mode = Mode::Select;
    app.active_mut().selection = Selection::single(Range::new(1, 1));
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(text(&app), "a");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.active().selection.primary().head, 0);

    app.open_help();
    press(&mut app, 'v');
    assert_eq!(app.mode, Mode::Select);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.mode, Mode::Select, "refusal must stop the mode step");
}
