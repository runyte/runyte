// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn shift_backspace_aliases_backspace_in_insert_and_replace_modes() {
    let mut insert = App::new(Config::default(), None).unwrap();
    seed(&mut insert, "ABC");
    insert.mode = Mode::Insert;
    set_cursor(&mut insert, 0, 3);

    key(&mut insert, KeyCode::Backspace, Modifiers::SHIFT);
    assert_eq!(text(&insert), "AB");

    let mut replace = App::new(Config::default(), None).unwrap();
    seed(&mut replace, "abc");
    press(&mut replace, 'R');
    replace
        .handle_input(InputEvent::Text("X".to_owned()))
        .unwrap();

    key(&mut replace, KeyCode::Backspace, Modifiers::SHIFT);
    assert_eq!(text(&replace), "abc");
    assert_eq!(replace.mode, Mode::Replace);
}

#[test]
fn replace_mode_overwrites_from_every_selection_head_and_restores_steps() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abc\nxy");
    app.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::new(2, 0), Range::new(4, 5)], 0);

    press(&mut app, 'R');
    assert_eq!(app.mode, Mode::Replace);
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::point(0), Range::point(5)]
    );

    app.handle_input(InputEvent::Text("λZ".to_owned())).unwrap();
    assert_eq!(text(&app), "λZc\nxλZ");

    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(text(&app), "λbc\nxλ");
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(text(&app), "abc\nxy");
    assert_eq!(app.mode, Mode::Replace);
}

#[test]
fn external_active_buffer_mutation_invalidates_the_replace_trail() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abc");

    press(&mut app, 'R');
    app.handle_input(InputEvent::Text("X".to_owned())).unwrap();
    assert_eq!(text(&app), "Xbc");
    assert_eq!(app.replace_session.as_ref().unwrap().steps.len(), 1);

    assert!(app.apply_to_buffer(
        app.active().buffer,
        &Transaction::new(vec![Change::new(0, 1, "Q")]),
    ));
    assert_eq!(text(&app), "Qbc");
    assert!(app.replace_session.is_none());

    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(text(&app), "Qbc");
    assert_eq!(app.mode, Mode::Replace);
}

#[test]
fn replace_word_restoration_uses_the_primary_carets_boundary() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "xxxxx-----yyyyyyyyyy");
    app.active_mut().selection = Selection::new(vec![Range::point(0), Range::point(10)], 1);

    press(&mut app, 'R');
    app.handle_input(InputEvent::Text("ab cd".to_owned()))
        .unwrap();
    key(&mut app, KeyCode::Backspace, Modifiers::ALT);

    assert_eq!(text(&app), "ab xx-----ab yyyyyyy");
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::point(3), Range::point(13)]
    );
}

#[test]
fn replace_line_restoration_uses_the_primary_carets_line() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "xxxxx\nYYYYYYYYYY");
    app.active_mut().selection = Selection::new(vec![Range::point(0), Range::point(6)], 1);

    press(&mut app, 'R');
    app.handle_input(InputEvent::Text("AB".to_owned())).unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.handle_input(InputEvent::Text("CD".to_owned())).unwrap();
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);

    assert_eq!(text(&app), "AB\nxxx\nAB\nYYYYYYYY");
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::point(3), Range::point(10)]
    );
}

/// Ctrl-k clears the rest of each caret's line without joining it to the next
/// one, and a caret already sitting at the line end has nothing to remove.
#[test]
fn delete_to_line_end_clears_each_carets_line_tail_without_joining_lines() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(
        &mut app,
        "keep this
second line
third",
    );
    app.active_mut().selection = Selection::new(
        vec![Range::point(4), Range::point(10 + 6), Range::point(22 + 5)],
        0,
    );

    press(&mut app, 'i');
    key(&mut app, KeyCode::Char('k'), Modifiers::CONTROL);

    assert_eq!(
        text(&app),
        "keep
second
third"
    );
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::point(4), Range::point(11), Range::point(17)],
        "each caret stays where the removed tail began"
    );
}

#[test]
fn no_op_pane_navigation_preserves_replace_mode_and_its_trail() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abc");

    press(&mut app, 'R');
    app.handle_input(InputEvent::Text("X".to_owned())).unwrap();
    app.focus_from_terminal_insert(1, 0);
    assert_eq!(app.mode, Mode::Replace);
    assert_eq!(app.replace_session.as_ref().unwrap().steps.len(), 1);

    app.next_window_from_terminal_insert();
    assert_eq!(app.mode, Mode::Replace);
    assert_eq!(app.replace_session.as_ref().unwrap().steps.len(), 1);

    app.toggle_maximized(MaximizedView::Fullscreen);
    app.focus_from_terminal_insert(-1, 0);
    assert_eq!(app.mode, Mode::Replace);
    assert_eq!(app.replace_session.as_ref().unwrap().steps.len(), 1);

    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(text(&app), "abc");
}

#[test]
fn replace_mode_appends_at_line_end_inserts_newlines_and_undoes_as_one_edit() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "ab");
    set_cursor(&mut app, 0, 1);

    press(&mut app, 'R');
    app.handle_input(InputEvent::Text("XYZ".to_owned()))
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.handle_input(InputEvent::Text("Q".to_owned())).unwrap();
    assert_eq!(text(&app), "aXYZ\nQ");
    assert_eq!(app.mode.label(), "REP");

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, 'u');
    assert_eq!(text(&app), "ab");
}

#[test]
fn lowercase_r_remains_a_single_character_normal_mode_command() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abc");

    press(&mut app, 'r');
    press(&mut app, 'X');

    assert_eq!(text(&app), "Xbc");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn count_prefixes_repeat_motions_and_address_lines() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "zero\none\ntwo\nthree\nfour\n");

    press(&mut app, '3');
    press(&mut app, 'j');
    assert_eq!(cursor(&app), Position::new(3, 0));

    press(&mut app, '2');
    press(&mut app, 'g');
    press(&mut app, 'g');
    assert_eq!(cursor(&app), Position::new(1, 0));

    press(&mut app, '4');
    press(&mut app, 'G');
    assert_eq!(cursor(&app), Position::new(3, 0));
    press(&mut app, 'G');
    assert_eq!(cursor(&app), Position::new(5, 0));
}

#[test]
fn soft_wrap_makes_vertical_motion_follow_visual_lines() {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "abcdef\nxy");
    app.active_mut().wrap_width = 3;
    set_cursor(&mut app, 0, 1);

    press(&mut app, 'j');
    assert_eq!(cursor(&app), Position::new(0, 4));
    press(&mut app, 'j');
    assert_eq!(cursor(&app), Position::new(1, 1));
    press(&mut app, 'k');
    assert_eq!(cursor(&app), Position::new(0, 4));
}

#[test]
fn soft_wrap_vertical_motion_stops_at_document_edges() {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "abcdef");
    app.active_mut().wrap_width = 3;

    set_cursor(&mut app, 0, 1);
    press(&mut app, 'k');
    assert_eq!(cursor(&app), Position::new(0, 1));
    press(&mut app, 'k');
    assert_eq!(cursor(&app), Position::new(0, 1));

    set_cursor(&mut app, 0, 4);
    press(&mut app, 'j');
    assert_eq!(cursor(&app), Position::new(0, 4));
    press(&mut app, 'j');
    assert_eq!(cursor(&app), Position::new(0, 4));
}

/// Page and window motions are the family measured in screen rows rather than
/// in document lines, so they are resolved against the projection the pane is
/// showing. Unwrapped, a screen row is a line; the window motions are then
/// answered from where the pane is scrolled to rather than from the document's
/// own top and bottom.
#[test]
fn page_and_window_motions_count_screen_rows_from_where_the_pane_is_scrolled() {
    let mut app = App::new(Config::default(), None).unwrap();
    let document = (0..60).fold(String::new(), |mut text, line| {
        text.push_str(&format!("line {line}\n"));
        text
    });
    seed(&mut app, &document);
    let viewport = app.viewport_height();
    assert_eq!(viewport, 20, "the default viewport this test counts in");

    key(&mut app, KeyCode::PageDown, Modifiers::NONE);
    assert_eq!(cursor(&app).row, viewport);
    key(&mut app, KeyCode::Char('d'), Modifiers::CONTROL);
    assert_eq!(cursor(&app).row, viewport + viewport / 2);
    key(&mut app, KeyCode::PageUp, Modifiers::NONE);
    assert_eq!(cursor(&app).row, viewport / 2);
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert_eq!(cursor(&app).row, 0);

    for (motion, row) in [('H', 0), ('M', (viewport - 1) / 2), ('L', viewport - 1)] {
        press(&mut app, motion);
        assert_eq!(cursor(&app).row, row, "{motion} at the document's top");
    }

    // Scrolled down, the same three motions name three different lines: the
    // window is what they are relative to, not the file.
    app.active_mut().scroll_row = 30;
    for (motion, row) in [
        ('H', 30),
        ('M', 30 + (viewport - 1) / 2),
        ('L', 30 + viewport - 1),
    ] {
        press(&mut app, motion);
        assert_eq!(cursor(&app).row, row, "{motion} after scrolling");
    }
}

/// Under soft wrap the same motions count wrapped segments, so a page ends on
/// a column part-way through a line rather than at the start of one.
#[test]
fn page_and_window_motions_count_wrapped_segments_under_soft_wrap() {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "aaaaaa\nbbbbbb\ncccccc");
    app.active_mut().wrap_width = 3;

    // Three lines of two segments each: six screen rows inside a viewport that
    // could hold twenty, so a page ends at the last of them.
    key(&mut app, KeyCode::PageDown, Modifiers::NONE);
    assert_eq!(cursor(&app), Position::new(2, 3));

    set_cursor(&mut app, 0, 0);
    press(&mut app, 'L');
    assert_eq!(cursor(&app), Position::new(2, 3));
    press(&mut app, 'M');
    assert_eq!(cursor(&app), Position::new(1, 0));
    press(&mut app, 'H');
    assert_eq!(cursor(&app), Position::new(0, 0));

    key(&mut app, KeyCode::Char('d'), Modifiers::CONTROL);
    assert_eq!(
        cursor(&app),
        Position::new(2, 3),
        "half a viewport is still more rows than the document has"
    );
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert_eq!(cursor(&app), Position::new(0, 0));
}

#[test]
fn wrapping_namespace_wraps_and_toggles_soft_wrap_and_whitespace_markers() {
    let mut config = Config::default();
    config.editor.hard_wrap_width = 10;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "alpha beta gamma");
    app.mode = Mode::Select;
    app.panes.get_mut(&0).unwrap().selection =
        Selection::single(Range::new(0, "alpha beta gamma".chars().count() - 1));

    for stroke in [' ', 'p', 'w'] {
        press(&mut app, stroke);
    }
    assert_eq!(text(&app), "alpha beta\ngamma");

    app.config.editor.hard_wrap_width = 6;
    app.panes.get_mut(&0).unwrap().selection =
        Selection::single(Range::new(0, text(&app).chars().count() - 1));
    for stroke in [' ', 'p', 'w'] {
        press(&mut app, stroke);
    }
    assert_eq!(text(&app), "alpha\nbeta\ngamma");
    press(&mut app, 'u');
    assert_eq!(text(&app), "alpha beta\ngamma");

    assert!(!app.config.editor.soft_wrap);
    for stroke in [' ', 'p', 's'] {
        press(&mut app, stroke);
    }
    assert!(app.config.editor.soft_wrap);

    assert!(!app.config.editor.render_whitespace);
    for stroke in [' ', 'p', '.'] {
        press(&mut app, stroke);
    }
    assert!(app.config.editor.render_whitespace);
    assert!(app.status.contains("whitespace markers enabled"));
    for stroke in [' ', 'p', '.'] {
        press(&mut app, stroke);
    }
    assert!(!app.config.editor.render_whitespace);
    assert!(app.status.contains("whitespace markers disabled"));
}

#[test]
fn wrapping_namespace_reflows_at_configured_width_as_one_edit() {
    let mut config = Config::default();
    config.editor.hard_wrap_width = 16;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "// alpha beta\n// gamma delta epsilon");
    app.mode = Mode::Select;
    app.panes.get_mut(&0).unwrap().selection =
        Selection::single(Range::new(0, text(&app).chars().count() - 1));

    for stroke in [' ', 'p', 'r'] {
        press(&mut app, stroke);
    }
    assert_eq!(text(&app), "// alpha beta\n// gamma delta\n// epsilon");

    app.config.editor.hard_wrap_width = 24;
    app.panes.get_mut(&0).unwrap().selection =
        Selection::single(Range::new(0, text(&app).chars().count() - 1));
    for stroke in [' ', 'p', 'r'] {
        press(&mut app, stroke);
    }
    assert_eq!(text(&app), "// alpha beta gamma\n// delta epsilon");
    press(&mut app, 'u');
    assert_eq!(text(&app), "// alpha beta\n// gamma delta\n// epsilon");
}

#[test]
fn named_registers_support_overwrite_append_and_paste() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abc");
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 1));

    press(&mut app, '"');
    press(&mut app, 'a');
    press(&mut app, 'y');
    assert_eq!(app.registers[&'a'].text, "ab");
    assert_eq!(app.registers[&'"'].text, "ab");

    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(2, 3));
    press(&mut app, '"');
    press(&mut app, 'A');
    press(&mut app, 'y');
    assert_eq!(app.registers[&'a'].text, "abc");

    app.panes.get_mut(&0).unwrap().selection = Selection::point(0);
    press(&mut app, '"');
    press(&mut app, 'a');
    press(&mut app, 'P');
    assert!(text(&app).starts_with("abc"));
}

#[test]
fn named_macros_record_stop_and_replay_through_the_macro_namespace() {
    let mut app = App::new(Config::default(), None).unwrap();
    for stroke in [' ', 'm', 'M', 'a', 'i', 'x'] {
        press(&mut app, stroke);
    }
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }

    assert_eq!(text(&app), "x");
    assert_eq!(app.macros[&'a'].len(), 3);
    assert!(app.recording_macro.is_none());
    press(&mut app, '2');
    for stroke in [' ', 'm', 'R', 'a'] {
        press(&mut app, stroke);
    }
    finish_macro_replay(&mut app);
    assert_eq!(text(&app), "xxx");
}

#[test]
fn the_default_macro_is_recorded_replayed_and_listed_from_one_namespace() {
    let mut app = App::new(Config::default(), None).unwrap();
    for stroke in [' ', 'm', 'm', 'i', 'x'] {
        press(&mut app, stroke);
    }
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.recording_macro, Some(DEFAULT_MACRO_REGISTER));
    // The keys that spell the stop belong to the gesture, not the macro.
    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }
    assert!(app.recording_macro.is_none());
    assert_eq!(app.macros[&DEFAULT_MACRO_REGISTER].len(), 3);
    assert!(app.macro_staging.is_empty());

    press(&mut app, '2');
    for stroke in [' ', 'm', 'r'] {
        press(&mut app, stroke);
    }
    finish_macro_replay(&mut app);
    assert_eq!(text(&app), "xxx");

    for stroke in [' ', 'm', 'l'] {
        press(&mut app, stroke);
    }
    let list = app.list.as_ref().expect("the macro list is open");
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].label, "@@");
    assert_eq!(list.items[0].detail, "default · 3 input(s)");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    finish_macro_replay(&mut app);
    assert!(app.list.is_none());
    assert_eq!(text(&app), "xxxx");
}

#[test]
fn the_old_macro_aliases_are_gone_and_a_second_recording_is_refused() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, 'Q');
    assert!(app.recording_macro.is_none());
    assert!(app.status.contains('Q'), "{}", app.status);

    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }
    for stroke in [' ', 'm', 'M', 'b'] {
        press(&mut app, stroke);
    }
    assert_eq!(app.recording_macro, Some(DEFAULT_MACRO_REGISTER));
    assert!(app.status_error);
    assert!(app.status.contains("Space m m"), "{}", app.status);
}

#[test]
fn the_macro_namespace_awaits_its_register_under_the_vim_grammar_too() {
    let mut config = Config::default();
    config.editor.grammar = GrammarKind::Vim;
    let mut app = App::new(config, None).unwrap();
    for stroke in [' ', 'm', 'M'] {
        press(&mut app, stroke);
    }
    assert!(app.recording_macro.is_none(), "the register is still owed");
    press(&mut app, 'a');
    assert_eq!(app.recording_macro, Some('a'));

    press(&mut app, 'i');
    press(&mut app, 'x');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    // Vim keeps `q` as its own stop, and the namespace stops it as well.
    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }
    assert!(app.recording_macro.is_none());
    assert_eq!(app.macros[&'a'].len(), 3);

    // Vim drops a count when it enters a Space namespace, so repetition
    // stays on its own `2@a`; the namespace replays once.
    for stroke in [' ', 'm', 'R', 'a'] {
        press(&mut app, stroke);
    }
    finish_macro_replay(&mut app);
    assert_eq!(text(&app), "xx");
    press(&mut app, '2');
    press(&mut app, '@');
    press(&mut app, 'a');
    finish_macro_replay(&mut app);
    assert_eq!(text(&app), "xxxx");
}

#[test]
fn an_empty_macro_list_says_so_instead_of_opening_an_empty_picker() {
    let mut app = App::new(Config::default(), None).unwrap();
    for stroke in [' ', 'm', 'l'] {
        press(&mut app, stroke);
    }
    assert!(app.list.is_none());
    assert_eq!(app.status, "no macros recorded");
}

#[test]
fn literal_text_is_one_insert_transaction_and_one_macro_event() {
    let mut app = App::new(Config::default(), None).unwrap();
    for stroke in [' ', 'm', 'M', 'a', 'i'] {
        press(&mut app, stroke);
    }
    app.handle_input(InputEvent::Text("α\nβ".to_owned()))
        .unwrap();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }

    assert_eq!(text(&app), "α\nβ");
    assert!(matches!(
        app.macros[&'a'].as_slice(),
        [
            InputEvent::Key(KeyStroke {
                code: KeyCode::Char('i'),
                modifiers: Modifiers::NONE,
            }),
            InputEvent::Text(pasted),
            InputEvent::Key(KeyStroke {
                code: KeyCode::Escape,
                modifiers: Modifiers::NONE,
            }),
        ] if pasted == "α\nβ"
    ));

    press(&mut app, 'u');
    assert_eq!(text(&app), "", "one undo removes the whole text event");
    for stroke in [' ', 'm', 'R', 'a'] {
        press(&mut app, stroke);
    }
    finish_macro_replay(&mut app);
    assert_eq!(text(&app), "α\nβ", "macro replay preserves text ordering");
}

fn replay_inputs(register: char) -> Vec<InputEvent> {
    [' ', 'm', 'R', register]
        .into_iter()
        .map(|character| InputEvent::Key(KeyStroke::char(character)))
        .collect()
}

#[test]
fn recursive_macro_replay_aborts_the_whole_root_before_trailing_inputs() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut inputs = replay_inputs('a');
    inputs.extend([
        InputEvent::Key(KeyStroke::char('i')),
        InputEvent::Text("unreachable".to_owned()),
        InputEvent::Key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE)),
    ]);
    app.macros.insert('a', inputs);

    app.replay_macro('a', 1).unwrap();
    finish_macro_replay(&mut app);

    assert_eq!(text(&app), "");
    assert!(app.status_error);
    assert_eq!(app.status, "recursive macro replay stopped: @a -> @a");
}

#[test]
fn mutual_macro_recursion_reports_the_active_register_chain() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.macros.insert('a', replay_inputs('b'));
    app.macros.insert('b', replay_inputs('a'));

    app.replay_macro('a', 1).unwrap();
    finish_macro_replay(&mut app);

    assert!(app.status_error);
    assert_eq!(app.status, "recursive macro replay stopped: @a -> @b -> @a");
}

#[test]
fn one_total_work_budget_bounds_large_counted_replay() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.macros
        .insert('a', vec![InputEvent::Key(KeyStroke::char('l'))]);

    app.replay_macro('a', 999_999).unwrap();
    finish_macro_replay(&mut app);

    assert!(app.status_error);
    assert_eq!(
        app.status,
        format!(
            "macro replay stopped after {MAX_MACRO_REPLAY_WORK} work unit(s); \
             {MAX_MACRO_REPLAY_WORK}-unit safety limit reached"
        )
    );
}

#[test]
fn a_recorded_maximal_command_count_is_expanded_cooperatively() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, &"x".repeat(MAX_MACRO_REPLAY_WORK + 100));
    app.macros.insert(
        'a',
        "999999l"
            .chars()
            .map(|character| InputEvent::Key(KeyStroke::char(character)))
            .collect(),
    );

    app.replay_macro('a', 1).unwrap();
    app.advance_macro_replay().unwrap();

    assert!(app.macro_replay_pending());
    assert!(app.active().head() < MAX_MACRO_REPLAY_WORK);
    finish_macro_replay(&mut app);
    assert!(app.status_error);
    assert_eq!(
        app.status,
        format!(
            "macro replay stopped after {MAX_MACRO_REPLAY_WORK} work unit(s); \
             {MAX_MACRO_REPLAY_WORK}-unit safety limit reached"
        )
    );
}

#[test]
fn grammar_level_counts_cannot_bypass_the_macro_work_budget() {
    for (mut app, recorded) in [
        (
            App::new(Config::default(), None).unwrap(),
            "999999x".to_owned(),
        ),
        (super::commands::vim_app("abcdef"), "999999l".to_owned()),
    ] {
        let selection = app.active().selection.clone();
        app.macros.insert(
            'a',
            recorded
                .chars()
                .map(|character| InputEvent::Key(KeyStroke::char(character)))
                .collect(),
        );

        app.replay_macro('a', 1).unwrap();
        finish_macro_replay(&mut app);

        assert_eq!(app.active().selection, selection);
        assert!(app.status_error);
        assert_eq!(
            app.status,
            format!(
                "macro replay stopped after 7 work unit(s); counted range exceeds the \
                 {MAX_MACRO_REPLAY_ATOMIC_REPETITIONS}-repetition per-action limit"
            )
        );
    }
}

#[test]
fn macro_replay_preserves_action_errors_across_progress_and_completion() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abc");
    let mut inputs = vec![InputEvent::Key(KeyStroke::char('l')); 126];
    inputs.extend([
        InputEvent::Key(KeyStroke::char('f')),
        InputEvent::Key(KeyStroke::char('z')),
        InputEvent::Key(KeyStroke::char('l')),
    ]);
    app.macros.insert('a', inputs);

    app.replay_macro('a', 1).unwrap();
    app.advance_macro_replay().unwrap();

    assert!(app.macro_replay_pending());
    assert!(app.status_error);
    assert_eq!(app.status, "character not found: z");

    finish_macro_replay(&mut app);
    assert!(!app.status_error);
    assert_eq!(
        app.status,
        format!("replayed macro @a; {} work unit(s)", 129)
    );

    app.macros.insert(
        'b',
        vec![
            InputEvent::Key(KeyStroke::char('f')),
            InputEvent::Key(KeyStroke::char('z')),
        ],
    );
    app.replay_macro('b', 1).unwrap();
    finish_macro_replay(&mut app);
    assert!(app.status_error);
    assert_eq!(app.status, "character not found: z");
}

#[test]
fn replay_finishing_on_a_batch_boundary_releases_input_immediately() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, &"x".repeat(MACRO_REPLAY_BATCH_INPUTS + 1));
    app.macros.insert(
        'a',
        vec![InputEvent::Key(KeyStroke::char('l')); MACRO_REPLAY_BATCH_INPUTS],
    );

    app.replay_macro('a', 1).unwrap();
    app.advance_macro_replay().unwrap();

    assert!(!app.macro_replay_pending());
    assert_eq!(app.active().head(), MACRO_REPLAY_BATCH_INPUTS);
    assert_eq!(
        app.status,
        format!("replayed macro @a; {MACRO_REPLAY_BATCH_INPUTS} work unit(s)")
    );
}

#[test]
fn semantic_range_work_counts_toward_the_current_replay_slice() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.macros.insert(
        'a',
        "128x128x"
            .chars()
            .map(|character| InputEvent::Key(KeyStroke::char(character)))
            .collect(),
    );

    app.replay_macro('a', 1).unwrap();
    app.advance_macro_replay().unwrap();

    assert!(app.macro_replay_pending());
    assert!(app.pending_sequence().is_empty());
    assert_eq!(
        app.status,
        "replaying macro @a; 131 work unit(s) · Esc/Ctrl-c cancels"
    );

    finish_macro_replay(&mut app);
    assert_eq!(app.status, "replayed macro @a; 262 work unit(s)");
}

#[test]
fn an_oversized_recorded_text_event_is_refused_before_it_edits() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.mode = Mode::Insert;
    app.macros.insert(
        'a',
        vec![InputEvent::Text("x".repeat(MAX_MACRO_REPLAY_WORK + 1))],
    );

    app.replay_macro('a', 1).unwrap();
    finish_macro_replay(&mut app);

    assert_eq!(text(&app), "");
    assert!(app.status_error);
    assert_eq!(
        app.status,
        format!(
            "macro replay stopped after 0 work unit(s); \
             {MAX_MACRO_REPLAY_WORK}-unit safety limit reached"
        )
    );
}

#[test]
fn an_oversized_raw_recording_is_refused_before_snapshotting() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.macros.insert(
        'a',
        vec![InputEvent::Key(KeyStroke::char('l')); MAX_MACRO_REPLAY_WORK + 1],
    );

    app.replay_macro('a', 1).unwrap();

    assert!(!app.macro_replay_pending());
    assert_eq!(app.active().head(), 0);
    assert!(app.status_error);
    assert_eq!(
        app.status,
        format!(
            "macro replay stopped after 0 work unit(s); \
             {MAX_MACRO_REPLAY_WORK}-unit safety limit reached"
        )
    );
}

#[test]
fn a_lifecycle_command_stops_trailing_macro_input() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.macros.insert(
        'a',
        vec![
            InputEvent::Key(KeyStroke::char(':')),
            InputEvent::Key(KeyStroke::char('q')),
            InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE)),
            InputEvent::Key(KeyStroke::char('i')),
            InputEvent::Text("unreachable".to_owned()),
        ],
    );

    app.replay_macro('a', 1).unwrap();
    finish_macro_replay(&mut app);

    assert!(app.should_quit);
    assert_eq!(text(&app), "");
    assert!(!app.macro_replay_pending());
}

#[test]
fn escape_and_ctrl_c_cancel_cooperative_macro_replay() {
    for cancel in [
        KeyStroke::new(KeyCode::Escape, Modifiers::NONE),
        KeyStroke::ctrl('c'),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.macros
            .insert('a', vec![InputEvent::Key(KeyStroke::char('l'))]);
        app.replay_macro('a', 999_999).unwrap();
        app.advance_macro_replay().unwrap();
        assert!(app.macro_replay_pending());

        app.handle_key(cancel).unwrap();

        assert!(!app.macro_replay_pending());
        assert!(app.pending_sequence().is_empty());
        assert_eq!(
            app.status,
            format!("macro replay @a cancelled after {MACRO_REPLAY_BATCH_INPUTS} work unit(s)")
        );
    }
}

#[test]
fn literal_text_edits_prompts_but_is_safe_in_modal_and_jump_contexts() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.handle_input(InputEvent::Text("ignored".to_owned()))
        .unwrap();
    assert_eq!(text(&app), "");

    press(&mut app, ':');
    app.handle_input(InputEvent::Text("open α.txt".to_owned()))
        .unwrap();
    assert_eq!(app.command, "open α.txt");
    assert_eq!(app.command_cursor, 10);

    app.close_prompt();
    app.jump = JumpLabels::new([0]);
    app.handle_input(InputEvent::Text("xy".to_owned())).unwrap();
    assert!(app.jump.is_none());
    assert_eq!(app.status, "jump cancelled");
}

/// A paste arrives as one input event rather than a run of keystrokes, so
/// every overlay that accepts typing has to accept it that way too. The
/// finder's query and a filterable list's filter each take the whole pasted
/// run, and the buffer waiting behind them takes none of it.
#[test]
fn pasted_text_reaches_an_open_picker_and_list_rather_than_the_buffer() {
    let root = temporary("paste-into-overlays");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("alpha.rs");
    fs::write(&path, "mod alpha {\n    fn beta() {}\n}\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(path).unwrap();
    let before = text(&app);

    app.open_project_picker().unwrap();
    app.handle_input(InputEvent::Text("alpha".to_owned()))
        .unwrap();
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.picker.is_none(), "the picker stayed open");

    app.execute_command("document-outline").unwrap();
    app.handle_input(InputEvent::Text("bet".to_owned()))
        .unwrap();
    assert_eq!(app.list.as_ref().unwrap().filter, "bet");
    assert_eq!(
        app.list.as_ref().unwrap().visible_indices().len(),
        1,
        "the pasted filter narrowed the outline"
    );

    assert_eq!(text(&app), before, "the buffer took the pasted text");
    fs::remove_dir_all(root).unwrap();
}

pub(super) struct MemoryClipboard(pub(super) Arc<Mutex<String>>);

impl SystemClipboard for MemoryClipboard {
    fn read(&mut self) -> Result<String> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn write(&mut self, text: &str) -> Result<()> {
        *self.0.lock().unwrap() = text.to_owned();
        Ok(())
    }
}

#[test]
fn app_delegates_interpretation_state_to_the_input_grammar() {
    let source = production_source();
    let fields = source
        .split_once("pub struct App {")
        .unwrap()
        .1
        .split_once("\n}")
        .unwrap()
        .0;

    assert!(
        fields
            .lines()
            .any(|line| line.trim() == "grammar: ActiveGrammar,")
    );
    for removed in [
        "pending: KeySequence,",
        "count: Option<usize>,",
        "awaiting_character: Option<(EditorCommand, usize)>,",
    ] {
        assert!(
            !fields.lines().any(|line| line.trim() == removed),
            "App regained grammar state {removed}"
        );
    }
    let direct_lookup = [".keymap", ".lookup_in("].concat();
    assert!(!source.contains(&direct_lookup));
}

#[test]
fn production_selection_replacements_are_revision_tracked() {
    let source = production_source();
    let production = source.split("\n#[cfg(test)]\nmod tests").next().unwrap();
    let assignments = production
        .lines()
        .map(str::trim)
        .filter(|line| line.contains(".selection ="))
        .collect::<Vec<_>>();
    assert_eq!(
        assignments,
        [
            "self.selection = selection;",
            "pane.selection = pane.selection.map(transaction);",
        ],
        "semantic selection producers must use Pane::replace_selection; only the setter and transaction coordinate mapping assign directly"
    );
}

#[test]
fn system_clipboard_bindings_use_the_clipboard_boundary() {
    let shared = Arc::new(Mutex::new(String::new()));
    let mut app = App::new(Config::default(), None).unwrap();
    app.set_system_clipboard(Box::new(MemoryClipboard(shared.clone())));
    seed(&mut app, "abc");
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 1));

    press(&mut app, ' ');
    press(&mut app, 'c');
    press(&mut app, 'y');
    assert_eq!(&*shared.lock().unwrap(), "ab");

    *shared.lock().unwrap() = "XY".to_owned();
    app.panes.get_mut(&0).unwrap().selection = Selection::point(0);
    press(&mut app, ' ');
    press(&mut app, 'c');
    press(&mut app, 'P');
    assert_eq!(text(&app), "XYabc");
}

#[test]
fn path_command_opens_a_popup_with_the_active_files_absolute_path() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary_directory().join(format!("runyte-path-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();

    app.open_file(path.clone()).unwrap();
    app.execute_command("path").unwrap();

    assert_eq!(
        app.path_popup.as_ref().unwrap().path,
        path.display().to_string()
    );
    assert!(app.path_action_menu.is_none());

    std::fs::remove_file(path).unwrap();
}

#[test]
fn path_command_on_a_directory_buffer_shows_its_root() {
    let mut app = App::new(Config::default(), None).unwrap();
    let directory = temporary_directory().join(format!(
        "runyte-path-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();

    app.open_explorer(Some(directory.clone())).unwrap();
    app.execute_command("path").unwrap();

    assert_eq!(
        app.path_popup.as_ref().unwrap().path,
        directory.display().to_string()
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn path_command_on_a_pathless_buffer_refuses() {
    let mut app = App::new(Config::default(), None).unwrap();
    assert!(app.active_buffer().path.is_none());

    app.execute_command("path").unwrap();

    assert!(app.path_popup.is_none());
    assert!(app.status_error);
}

#[test]
fn path_popup_tab_opens_a_mnemonic_action_menu_and_escape_unwinds_one_level_at_a_time() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary_directory().join(format!("runyte-path-tab-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();

    app.execute_command("path").unwrap();
    assert!(app.path_popup.is_some());
    assert!(app.path_action_menu.is_none());

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let menu = app.path_action_menu.as_ref().unwrap();
    assert_eq!(menu.selected, 0);
    assert_eq!(
        menu.actions,
        vec![PathClipboardTarget::System, PathClipboardTarget::Register]
    );

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(
        app.path_action_menu.is_none(),
        "Tab backs out of just the submenu"
    );
    assert!(app.path_popup.is_some(), "the popup itself stays open");

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(
        app.path_popup.is_none(),
        "Escape closes the bare popup fully"
    );

    std::fs::remove_file(path).unwrap();
}

#[test]
fn path_action_menu_cycles_with_up_and_down_and_wraps() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary_directory().join(format!("runyte-path-cycle-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();
    app.execute_command("path").unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(app.path_action_menu.as_ref().unwrap().selected, 1);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(
        app.path_action_menu.as_ref().unwrap().selected,
        0,
        "cycling wraps forward"
    );
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    assert_eq!(
        app.path_action_menu.as_ref().unwrap().selected,
        1,
        "cycling wraps backward"
    );
    key(&mut app, KeyCode::BackTab, Modifiers::NONE);
    assert_eq!(app.path_action_menu.as_ref().unwrap().selected, 0);

    std::fs::remove_file(path).unwrap();
}

#[test]
fn path_action_menu_mnemonic_s_copies_to_the_system_clipboard_and_closes() {
    let shared = Arc::new(Mutex::new(String::new()));
    let mut app = App::new(Config::default(), None).unwrap();
    app.set_system_clipboard(Box::new(MemoryClipboard(shared.clone())));
    let path = temporary_directory().join(format!("runyte-path-sys-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();

    app.execute_command("path").unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    press(&mut app, 's');

    assert_eq!(&*shared.lock().unwrap(), &path.display().to_string());
    assert!(app.path_action_menu.is_none());
    assert!(app.path_popup.is_none());

    std::fs::remove_file(path).unwrap();
}

#[test]
fn path_action_menu_mnemonic_r_copies_to_the_unnamed_register_and_closes() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary_directory().join(format!("runyte-path-reg-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();

    app.execute_command("path").unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    press(&mut app, 'r');

    assert_eq!(
        app.registers.get(&'"').unwrap().text,
        path.display().to_string()
    );
    assert!(app.path_action_menu.is_none());
    assert!(app.path_popup.is_none());

    std::fs::remove_file(path).unwrap();
}

#[test]
fn path_action_menu_down_then_enter_copies_to_the_register_target() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary_directory().join(format!("runyte-path-enter-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();

    app.execute_command("path").unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(
        app.registers.get(&'"').unwrap().text,
        path.display().to_string()
    );
    assert!(app.path_action_menu.is_none());
    assert!(app.path_popup.is_none());

    std::fs::remove_file(path).unwrap();
}

/// A clipboard whose helper is unavailable, so the editor has to report the
/// failure rather than claim the path was copied.
struct RefusingClipboard;

impl SystemClipboard for RefusingClipboard {
    fn read(&mut self) -> Result<String> {
        anyhow::bail!("no clipboard helper")
    }

    fn write(&mut self, _text: &str) -> Result<()> {
        anyhow::bail!("no clipboard helper")
    }
}

/// The popup and its action menu are one overlay at a time: the bare popup
/// offers the way into the actions, and the menu replaces it with the copy
/// targets and their mnemonics rather than stacking a second overlay on top.
#[test]
fn the_path_popup_and_its_action_menu_publish_one_overlay_each() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path =
        temporary_directory().join(format!("runyte-path-overlay-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();

    app.execute_command("path").unwrap();
    let overlays = app.overlay_snapshots();
    assert_eq!(overlays.len(), 1);
    let popup = &overlays[0];
    assert_eq!(popup.kind, crate::snapshot::OverlayKind::Path);
    assert_eq!(popup.title, "Path");
    assert_eq!(popup.message, Some(path.display().to_string()));
    assert!(popup.rows.is_empty(), "the bare popup lists no actions");
    assert_eq!(popup.selected, None);
    assert_eq!(
        popup
            .actions
            .iter()
            .map(|action| action.key_hint.as_str())
            .collect::<Vec<_>>(),
        ["Tab", "Esc"]
    );

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let overlays = app.overlay_snapshots();
    assert_eq!(overlays.len(), 1, "the menu replaces the popup overlay");
    let menu = &overlays[0];
    assert_eq!(menu.kind, crate::snapshot::OverlayKind::PathActions);
    assert_eq!(menu.selected, Some(0));
    assert_eq!(
        menu.rows
            .iter()
            .map(|row| (row.label.as_str(), row.detail.as_str()))
            .collect::<Vec<_>>(),
        [
            ("s", "copy to system clipboard"),
            ("r", "copy to Runyte register"),
        ]
    );
    assert_eq!(
        menu.message,
        Some(path.display().to_string()),
        "the menu still names the path it will copy"
    );

    std::fs::remove_file(path).unwrap();
}

/// Ctrl-c closes the popup the way Escape does, and a key that means nothing
/// here leaves it standing rather than dismissing it by accident.
#[test]
fn control_c_closes_the_path_popup_and_an_unbound_key_leaves_it_open() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary_directory().join(format!("runyte-path-ctrlc-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();

    app.execute_command("path").unwrap();
    press(&mut app, 'q');
    assert!(app.path_popup.is_some(), "an unbound key is ignored here");

    key(&mut app, KeyCode::Char('c'), Modifiers::CONTROL);
    assert!(app.path_popup.is_none());
    assert!(app.path_action_menu.is_none());

    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_refused_system_clipboard_reports_the_failure_instead_of_a_copy() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.set_system_clipboard(Box::new(RefusingClipboard));
    let path = temporary_directory().join(format!("runyte-path-refuse-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();

    app.execute_command("path").unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    press(&mut app, 's');

    assert!(app.status_error, "a refused copy is an error, not a status");
    assert!(
        app.status.contains("no clipboard helper"),
        "the helper's own reason reaches the person: {:?}",
        app.status
    );
    assert_eq!(
        app.notifications
            .entries()
            .last()
            .map(|notification| notification.title.as_str()),
        Some("Clipboard operation failed"),
        "the refusal is retained rather than only flashed"
    );
    assert!(app.path_popup.is_none(), "the popup still closes");

    std::fs::remove_file(path).unwrap();
}

/// A register selected before the popup opens is the one the path lands in,
/// and the confirmation names it so the person knows where to paste from.
#[test]
fn copying_the_path_to_a_selected_register_names_that_register() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary_directory().join(format!("runyte-path-named-{}.txt", std::process::id()));
    std::fs::write(&path, "contents\n").unwrap();
    app.open_file(path.clone()).unwrap();

    press(&mut app, '"');
    press(&mut app, 'a');
    app.execute_command("path").unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    press(&mut app, 'r');

    assert_eq!(
        app.registers.get(&'a').unwrap().text,
        path.display().to_string()
    );
    assert_eq!(app.status, "copied path to register a");
    assert!(!app.status_error);

    std::fs::remove_file(path).unwrap();
}

#[test]
fn buffer_picker_filters_and_switches_the_active_pane() {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary("buffer-picker-switch.txt");
    fs::write(&path, "durable notes").unwrap();
    app.buffers.push(Buffer::open(&path).unwrap());
    app.syntax.push(None);

    press(&mut app, ' ');
    press(&mut app, 'b');
    press(&mut app, 'b');
    assert_eq!(app.list.as_ref().unwrap().title, "Buffers");
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, 1);
    assert_eq!(app.active_buffer().to_string(), "durable notes");
    fs::remove_file(path).unwrap();
}

#[test]
fn buffer_picker_previews_authoritative_text_and_toggles_the_column() {
    let directory = temporary("buffer-picker-preview");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("notes.txt");
    fs::write(&path, "disk text\n").unwrap();
    let mut app = App::new(Config::default(), Some(path)).unwrap();
    app.buffers[0].apply(&Transaction::insert(0, "unsaved "));

    app.open_buffer_picker();

    let picker = app.list.as_ref().unwrap();
    assert!(picker.has_preview());
    assert_eq!(picker.preview_title(), Some("Contents"));
    assert!(
        picker
            .selected_preview()
            .is_some_and(|preview| preview.starts_with("unsaved disk text"))
    );
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::ResultList)
        .unwrap();
    assert_eq!(overlay.layout, crate::snapshot::OverlayLayout::Preview);
    assert!(matches!(
        overlay.preview,
        Some(crate::snapshot::OverlayPreview::MatchedText { ref lines, .. })
            if lines.first().is_some_and(|line| line == "unsaved disk text")
    ));

    key(&mut app, KeyCode::Char('t'), Modifiers::CONTROL);
    assert!(!app.list.as_ref().unwrap().show_preview);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn buffer_picker_uses_names_and_project_relative_or_absolute_paths() {
    let fixture = temporary("buffer-picker-path-columns");
    let project = fixture.join("project");
    let source = project.join("src");
    let inside = source.join("lorem_ipsum.md");
    let outside = fixture.join("outside.md");
    fs::create_dir_all(&source).unwrap();
    fs::write(&inside, "inside").unwrap();
    fs::write(&outside, "outside").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(inside), &project).unwrap();
    app.buffers[0].apply(&Transaction::insert(0, "changed "));
    app.open_file(outside.clone()).unwrap();

    app.open_buffer_picker();

    let items = &app.list.as_ref().unwrap().items;
    assert_eq!(items[0].label, "lorem_ipsum.md");
    assert_eq!(items[0].detail, "src/lorem_ipsum.md");
    assert_eq!(items[1].label, "*outside.md*");
    assert_eq!(items[1].detail, outside.display().to_string());
    fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn buffer_picker_uses_directory_names_and_paths() {
    let fixture = temporary("buffer-picker-directory-columns");
    let project = fixture.join("project");
    let inside = project.join("assets");
    let outside = fixture.join("external");
    fs::create_dir_all(&inside).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let project_buffer = Buffer::open_directory(&project, false).unwrap();
    let inside_buffer = Buffer::open_directory(&inside, false).unwrap();
    let outside_buffer = Buffer::open_directory(&outside, false).unwrap();

    assert_eq!(
        buffer_picker_columns(&project_buffer, &project, false),
        ("[explorer] project".to_owned(), ".".to_owned())
    );
    assert_eq!(
        buffer_picker_columns(&inside_buffer, &project, false),
        ("[explorer] assets".to_owned(), "assets".to_owned())
    );
    assert_eq!(
        buffer_picker_columns(&outside_buffer, &project, true),
        (
            "*[explorer] external*".to_owned(),
            outside.display().to_string()
        )
    );
    fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn buffer_picker_keeps_special_names_and_marks_read_only_types() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers
        .push(Buffer::virtual_text("[notes]", "durable notes"));
    app.syntax.push(None);
    app.active_mut().retarget(1);

    app.open_buffer_picker();

    let items = &app.list.as_ref().unwrap().items;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "*[notes]* [RO]");
    assert_eq!(items[0].detail, "");
}

#[test]
fn space_b_n_opens_a_fresh_scratch_buffer_in_the_current_pane() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "kept in the original scratch");
    let original = app.active().buffer;
    let pane = app.active_pane;

    press(&mut app, ' ');
    press(&mut app, 'b');
    press(&mut app, 'n');

    assert_eq!(app.active_pane, pane);
    assert_ne!(app.active().buffer, original);
    assert!(matches!(app.active_buffer().kind, BufferKind::Scratch));
    assert_eq!(text(&app), "");

    app.jump_in(true, true);
    assert_eq!(app.active().buffer, original);
    assert_eq!(text(&app), "kept in the original scratch");
}

#[test]
fn buffer_new_and_its_short_spelling_open_the_same_scratch_buffer_as_the_key() {
    for spelling in ["buffer-new", "new"] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, "kept in the original scratch");
        let original = app.active().buffer;
        let pane = app.active_pane;

        type_command(&mut app, spelling);

        assert_eq!(app.active_pane, pane, ":{spelling} changed pane");
        assert_ne!(app.active().buffer, original);
        assert!(matches!(app.active_buffer().kind, BufferKind::Scratch));
        assert_eq!(text(&app), "");

        app.jump_in(true, true);
        assert_eq!(app.active().buffer, original);
    }
}

#[test]
fn literal_text_does_not_mutate_the_picker_behind_a_buffer_action_menu() {
    let directory = temporary("buffer-actions-text-input");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("notes.txt");
    fs::write(&path, "before").unwrap();
    let mut app = App::new(Config::default(), Some(path)).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(6, " after"));

    press(&mut app, ' ');
    press(&mut app, 'b');
    press(&mut app, 'b');
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.buffer_action_menu.is_some());
    let filter = app.list.as_ref().unwrap().filter.clone();

    app.handle_input(InputEvent::Text("hidden".to_owned()))
        .unwrap();

    assert_eq!(app.list.as_ref().unwrap().filter, filter);
    assert!(app.buffer_action_menu.is_some());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn buffer_picker_discard_requires_confirmation_and_keeps_the_file_open() {
    let directory = temporary("buffer-actions-discard");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("notes.txt");
    fs::write(&path, "on disk").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let file = app.active().buffer;
    app.buffers[file].apply(&Transaction::insert(0, "changed "));

    app.open_buffer_picker();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.buffer_discard_confirmation, Some(file));
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Discard buffer changes");
    assert_eq!(overlay.actions[0].label, "discard changes");
    assert!(
        overlay
            .message
            .as_deref()
            .is_some_and(|message| message.contains("notes.txt"))
    );
    assert!(app.buffers[file].dirty);
    assert_eq!(app.buffers[file].to_string(), "changed on disk");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.buffer_discard_confirmation, None);
    assert!(!app.buffers[file].dirty);
    assert_eq!(app.buffers[file].to_string(), "on disk");
    assert!(!app.closed_buffers.contains(&file));
    assert!(app.list.is_some(), "discard returns to the buffer picker");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn explorer_rows_have_no_buffer_management_actions() {
    let directory = temporary("buffer-actions-explorer");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("keep.txt"), "keep").unwrap();
    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    let explorer = app.active().buffer;

    app.open_buffer_picker();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    assert!(app.buffer_action_menu.is_none());
    assert!(
        app.status
            .contains("explorer buffers have no management actions")
    );
    assert!(!app.closed_buffers.contains(&explorer));
    assert_eq!(
        fs::read_to_string(directory.join("keep.txt")).unwrap(),
        "keep"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn closing_a_shared_buffer_redirects_every_pane() {
    let directory = temporary("buffer-actions-shared-pane");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("notes.txt");
    fs::write(&path, "shared").unwrap();
    let mut app = App::new(Config::default(), Some(path)).unwrap();
    let file = app.active().buffer;
    app.split(Axis::Horizontal, None).unwrap();

    app.open_buffer_picker();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.closed_buffers.contains(&file));
    let fallback = app.active().buffer;
    assert!(app.buffers[fallback].is_empty_clean_scratch());
    assert!(app.panes.values().all(|pane| pane.buffer == fallback));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn closing_a_buffer_releases_its_text_and_history_payloads() {
    let mut app = App::new(Config::default(), None).unwrap();
    let file = app.active().buffer;
    let path = temporary("retired-large.txt");
    let contents = "x".repeat(256 * 1024);
    app.buffers[file].path = Some(path.clone());
    app.buffers[file].kind = crate::buffer::BufferKind::File;
    app.buffers[file].set_text(&contents);
    app.buffers[file].apply(&Transaction::delete(0, contents.len() / 2));
    app.buffers[file].mark_saved();
    assert!(app.buffers[file].len_chars() > 0);
    assert!(app.buffers[file].history_footprint() > 0);

    app.close_buffer(file);

    assert!(app.closed_buffers.contains(&file));
    assert_eq!(app.buffers[file].len_chars(), 0);
    assert_eq!(app.buffers[file].history_footprint(), 0);
    assert!(app.buffers[file].path.is_none());
    assert_eq!(app.status, format!("closed {}", path.display()));
}

#[test]
fn tab_still_navigates_non_buffer_result_pickers() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.list = Some(ListPicker::new(
        "Code actions",
        vec![
            PickerItem::new("first", "action", 0),
            PickerItem::new("second", "action", 1),
        ],
    ));
    app.list_actions = vec![ListAction::CodeAction(0), ListAction::CodeAction(1)];

    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    assert_eq!(
        app.list.as_ref().unwrap().selected_item().unwrap().label,
        "second"
    );
    assert!(app.buffer_action_menu.is_none());
}

#[test]
fn global_search_opens_a_reusable_result_buffer_and_jumps_to_a_match() {
    let directory = temporary_directory().join(format!(
        "runyte-global-search-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(directory.join("src")).unwrap();
    let path = directory.join("src/example.txt");
    fs::write(&path, "first\nα needle here\nlast\n").unwrap();
    fs::create_dir_all(directory.join(".git")).unwrap();
    fs::write(directory.join(".git/ignored"), "needle").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = directory.clone();
    type_text(&mut app, " //");
    type_text(&mut app, "needle");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.active_buffer().is_workspace_search());
    let result_row = app.active_buffer().offset_to_row(app.active().head());
    assert!(
        app.active_buffer()
            .workspace_search_target_at(result_row)
            .is_some()
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_buffer().path.as_deref(), Some(path.as_path()));
    assert_eq!(
        app.active_buffer()
            .position_of(app.active().selection.primary().from()),
        Position::new(1, 2)
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_search_returns_to_input_before_controlled_scan_completion() {
    let directory = temporary("workspace-search-background-input");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("example.txt"), "needle\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = directory.clone();
    seed(&mut app, "abc");
    let (service, requests) = WorkspaceSearchService::controlled();
    app.attach_workspace_search(service);

    type_text(&mut app, " /s");
    type_text(&mut app, "needle");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    let request = requests.try_recv().expect("the scan was queued");
    assert_eq!(app.status, "searching workspace in the background");
    assert!(!app.active_buffer().is_workspace_search());
    let action = app
        .long_running_action_snapshot()
        .expect("the pending search owns the progress status");
    assert_eq!(action.label, "Searching workspace");
    assert_eq!(action.detail, "needle");
    assert_eq!(action.cancel_hint, None);
    assert!(app.has_long_running_action());
    press(&mut app, 'l');
    assert_eq!(app.active().head(), 1, "input remains live while scanning");

    let id = request.id;
    let (matches, limited) = crate::workspace_search::perform(request, || false)
        .unwrap()
        .unwrap();
    app.apply_workspace_search_event(WorkspaceSearchEvent::Completed {
        id,
        matches,
        limited,
    });

    assert!(app.active_buffer().is_workspace_search());
    assert!(app.active_buffer().to_string().contains("example.txt:1:1"));
    assert!(!app.has_long_running_action());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_search_rejects_a_superseded_completion() {
    let directory = temporary("workspace-search-superseded");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("example.txt");
    fs::write(&path, "first second\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = directory.clone();
    let (service, requests) = WorkspaceSearchService::controlled();
    app.attach_workspace_search(service);

    app.open_global_search("first", SearchMode::Insensitive);
    let first = requests.recv().unwrap();
    app.open_global_search("second", SearchMode::Insensitive);
    let second = requests.recv().unwrap();
    let first_id = first.id;
    let (matches, limited) = crate::workspace_search::perform(first, || false)
        .unwrap()
        .unwrap();

    app.apply_workspace_search_event(WorkspaceSearchEvent::Completed {
        id: first_id,
        matches,
        limited,
    });
    assert!(!app.active_buffer().is_workspace_search());
    assert_eq!(app.status, "searching workspace in the background");

    let second_id = second.id;
    let (matches, limited) = crate::workspace_search::perform(second, || false)
        .unwrap()
        .unwrap();
    app.apply_workspace_search_event(WorkspaceSearchEvent::Completed {
        id: second_id,
        matches,
        limited,
    });
    assert!(
        app.active_buffer()
            .to_string()
            .starts_with("Query: second\n")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn background_workspace_search_uses_the_open_buffer_snapshot() {
    let directory = temporary("workspace-search-open-snapshot");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("example.txt");
    fs::write(&path, "needle on disk\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.project_root = directory.clone();
    let length = app.active_buffer().len_chars();
    assert!(app.apply_to_buffer(
        app.active().buffer,
        &Transaction::new(vec![Change::new(0, length, "needle live\n")]),
    ));
    let (service, requests) = WorkspaceSearchService::controlled();
    app.attach_workspace_search(service);

    app.open_global_search("needle", SearchMode::Insensitive);
    let request = requests.recv().unwrap();
    assert_eq!(request.open_buffers.len(), 1);
    assert_eq!(request.open_buffers[0].text.to_string(), "needle live\n");
    let id = request.id;
    let (matches, limited) = crate::workspace_search::perform(request, || false)
        .unwrap()
        .unwrap();
    app.apply_workspace_search_event(WorkspaceSearchEvent::Completed {
        id,
        matches,
        limited,
    });

    let rendered = app.active_buffer().to_string();
    assert!(rendered.contains("needle live"), "{rendered}");
    assert!(!rendered.contains("needle on disk"), "{rendered}");
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn workspace_search_escapes_line_breaks_without_losing_path_identity() {
    let directory = temporary("workspace-search-newline-path");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("line\nbreak.txt");
    fs::write(&path, "needle\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = directory.clone();

    app.open_global_search("needle", SearchMode::Insensitive);

    let rendered = app.active_buffer().to_string();
    assert!(rendered.contains("line\\nbreak.txt:1:1"), "{rendered:?}");
    let targets = (0..app.active_buffer().len_lines())
        .filter_map(|row| app.active_buffer().workspace_search_target_at(row))
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].path, path);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_search_retains_disk_scan_truncation_after_live_reconciliation() {
    let directory = temporary("workspace-search-retained-limit");
    fs::create_dir_all(&directory).unwrap();
    let omitted = directory.join("a-omitted.txt");
    let saturated = directory.join("z-saturated.txt");
    fs::write(&omitted, "needle omitted\n").unwrap();
    fs::write(
        &saturated,
        "needle\n".repeat(crate::workspace_search::GLOBAL_SEARCH_RESULT_LIMIT),
    )
    .unwrap();
    let mut app = App::new(Config::default(), Some(saturated.clone())).unwrap();
    app.project_root = directory.clone();
    let length = app.buffers[0].len_chars();
    assert!(app.apply_to_buffer(
        0,
        &Transaction::new(vec![Change::new(0, length, "needle live\n")]),
    ));

    app.open_global_search("needle", SearchMode::Insensitive);

    assert!(app.status.contains("limit reached"), "{}", app.status);
    assert!(
        app.active_buffer()
            .to_string()
            .contains("result limit reached")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_search_remains_jumpable_and_is_rebuilt_in_place() {
    let directory = temporary("workspace-search-view");
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("a.txt");
    let second = directory.join("b.txt");
    fs::write(&first, "needle one\n").unwrap();
    fs::write(&second, "needle two\n").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = directory.clone();

    app.open_global_search("needle", SearchMode::Insensitive);
    let search_buffer = app.active().buffer;
    assert!(app.active_buffer().is_workspace_search());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_buffer().path.as_deref(), Some(first.as_path()));
    assert!(!app.closed_buffers.contains(&search_buffer));

    key(&mut app, KeyCode::Char('o'), Modifiers::ALT);
    assert_eq!(app.active().buffer, search_buffer);

    app.open_global_search("two", SearchMode::Sensitive);
    assert_eq!(app.active().buffer, search_buffer);
    assert_eq!(
        app.buffers
            .iter()
            .enumerate()
            .filter(|(index, buffer)| {
                !app.closed_buffers.contains(index) && buffer.is_workspace_search()
            })
            .count(),
        1
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn workspace_search_offers_the_same_flavours_as_the_buffer() {
    let directory = temporary_directory().join(format!(
        "runyte-workspace-flavours-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("example.txt"),
        "Needle here\nneedle there\nnee(dle escaped\n",
    )
    .unwrap();

    let matches = |app: &App| {
        (0..app.active_buffer().len_lines())
            .filter(|row| {
                app.active_buffer()
                    .workspace_search_target_at(*row)
                    .is_some()
            })
            .count()
    };

    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = directory.clone();

    type_text(&mut app, " /s");
    type_text(&mut app, "needle");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(matches(&app), 2, "ignoring case finds both spellings");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    // Literal workspace search escapes the pattern; the regex flavour does
    // not, which is the whole difference between the two.
    type_text(&mut app, " /s");
    type_text(&mut app, "nee(dle");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(matches(&app), 1);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    type_text(&mut app, " //");
    type_text(&mut app, "nee(dle");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error, "an unclosed group is reported, not found");

    type_text(&mut app, " //");
    type_text(&mut app, "nee.dle");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(matches(&app), 1, "Space / / is the regex workspace search");

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn remaining_multi_selection_bindings_use_regex_and_rotate_contents() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "a1 a2 b");
    press(&mut app, '%');
    press(&mut app, '/');
    type_text(&mut app, r"a\d");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().selection.len(), 2);

    app.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::point(1), Range::point(4)], 0);
    key(&mut app, KeyCode::Char(')'), Modifiers::ALT);
    assert_eq!(text(&app), "a2 a1 b");
}

#[test]
fn syntax_newline_indentation_is_one_pre_edit_multi_caret_transaction() {
    let path = temporary("syntax-indent.rs");
    let original = "fn outer() {\n    if ready {\n    }\n}\n";
    fs::write(&path, original).unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.mode = Mode::Insert;
    let first = app.buffers[0].line_to_offset(0) + app.buffers[0].line_len(0);
    let second = app.buffers[0].line_to_offset(1) + app.buffers[0].line_len(1);
    app.replace_active_selection(Selection::new(
        vec![Range::point(first), Range::point(second)],
        0,
    ));

    app.edit_newline();

    assert_eq!(
        text(&app),
        "fn outer() {\n    \n    if ready {\n        \n    }\n}\n"
    );
    app.undo();
    assert_eq!(text(&app), original, "both carets must share one undo step");
    fs::remove_file(path).unwrap();
}

#[test]
fn smart_newline_preserves_crlf_and_uses_its_syntax_indent() {
    let path = temporary("syntax-indent-crlf.rs");
    let original = "fn outer() {\r\n    if ready {\r\n    }\r\n}\r\n";
    fs::write(&path, original).unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.mode = Mode::Insert;
    let caret = app.buffers[0].line_to_offset(1) + app.buffers[0].line_len(1);
    app.replace_active_selection(Selection::point(caret));

    app.edit_newline();

    assert_eq!(
        text(&app),
        "fn outer() {\r\n    if ready {\r\n        \r\n    }\r\n}\r\n"
    );
    app.undo();
    assert_eq!(text(&app), original);
    fs::remove_file(path).unwrap();
}

#[test]
fn smart_newline_uses_the_required_make_recipe_tab() {
    let path = temporary("syntax-indent.mk");
    let original = "all:\n";
    fs::write(&path, original).unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.mode = Mode::Insert;
    let caret = app.buffers[0].line_len(0);
    app.replace_active_selection(Selection::point(caret));

    app.edit_newline();

    assert_eq!(text(&app), "all:\n\t\n");
    app.undo();
    assert_eq!(text(&app), original);
    fs::remove_file(path).unwrap();
}

#[test]
fn syntax_newline_mid_line_and_unterminated_eof_degrade_without_losing_prefix() {
    let path = temporary("syntax-indent-positions.rs");
    fs::write(&path, "fn outer() {\n    let value = 1;\n    tail").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.mode = Mode::Insert;

    let middle = app.buffers[0].line_to_offset(1) + 7;
    app.replace_active_selection(Selection::point(middle));
    app.edit_newline();
    assert_eq!(
        text(&app),
        "fn outer() {\n    let\n     value = 1;\n    tail"
    );

    let eof = app.active_buffer().len_chars();
    app.replace_active_selection(Selection::point(eof));
    app.edit_newline();
    assert!(
        text(&app).ends_with("    tail\n    "),
        "an unterminated final row keeps its exact prefix as the safe fallback"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn syntax_newline_multiline_selection_uses_normalized_insertion_row_in_both_directions() {
    let path = temporary("syntax-indent-selection.rs");
    let original = "fn outer() {\n    one();\n        two();\n}\n";
    fs::write(&path, original).unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.mode = Mode::Insert;
    let from = app.buffers[0].line_to_offset(1) + 4;
    let to = app.buffers[0].line_to_offset(2) + 8;

    app.replace_active_selection(Selection::single(Range::new(from, to)));
    app.edit_newline();
    let forward = text(&app);
    app.undo();
    assert_eq!(text(&app), original);

    app.replace_active_selection(Selection::single(Range::new(to, from)));
    app.edit_newline();
    assert_eq!(text(&app), forward);
    assert_eq!(forward, "fn outer() {\n    \n        two();\n}\n");
    fs::remove_file(path).unwrap();
}

#[test]
fn newline_inside_leading_whitespace_preserves_the_tail_indentation() {
    for (column, expected) in [
        (0, "\n    foo\n    bar\n"),
        (2, "  \n    foo\n    bar\n"),
        (4, "    \n    foo\n    bar\n"),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, "    foo\n    bar\n");
        app.mode = Mode::Insert;
        app.replace_active_selection(Selection::point(column));

        app.edit_newline();

        assert_eq!(text(&app), expected, "caret column {column}");
    }
}

#[test]
fn smart_newline_aligns_list_continuations_under_their_content() {
    for (line, expected_indent) in [
        ("- bullet", "  "),
        ("* bullet", "  "),
        ("+ bullet", "  "),
        ("1. numbered", "   "),
        ("10285. numbered", "       "),
        ("a. lettered", "   "),
        ("A. lettered", "   "),
        ("I. roman", "   "),
        ("II. roman", "    "),
        ("MCMLXXXIV. roman", "           "),
        ("    - nested", "      "),
        ("\t1. nested after a tab", "\t   "),
        ("        + nested", "          "),
        ("            a. nested", "               "),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, line);
        app.mode = Mode::Insert;
        app.replace_active_selection(Selection::point(app.active_buffer().len_chars()));

        app.edit_newline();

        assert_eq!(text(&app), format!("{line}\n{expected_indent}"), "{line}");
    }
}

#[test]
fn smart_newline_keeps_following_continuation_lines_aligned() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "10285. numbered");
    app.mode = Mode::Insert;
    app.replace_active_selection(Selection::point(app.active_buffer().len_chars()));

    app.edit_newline();
    app.insert_text("continued");
    app.edit_newline();

    assert_eq!(text(&app), "10285. numbered\n       continued\n       ",);
}

#[test]
fn smart_newline_ignores_prose() {
    for prose in [
        "Hello. prose",
        "civil. rights",
        "mix. ingredients",
        "IIV. invalid",
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, prose);
        app.mode = Mode::Insert;
        app.replace_active_selection(Selection::point(app.active_buffer().len_chars()));
        app.edit_newline();
        assert_eq!(text(&app), format!("{prose}\n"), "{prose}");
    }
}

#[test]
fn disabled_smart_newline_keeps_leading_indent_without_list_alignment() {
    for (line, expected) in [
        ("1. First line", "1. First line\n"),
        ("   Second line", "   Second line\n   "),
        ("    - nested", "    - nested\n    "),
        ("\t1. nested after a tab", "\t1. nested after a tab\n\t"),
    ] {
        let mut config = Config::default();
        config.editor.smart_newline = false;
        let mut app = App::new(config, None).unwrap();
        seed(&mut app, line);
        app.mode = Mode::Insert;
        app.replace_active_selection(Selection::point(app.active_buffer().len_chars()));

        app.edit_newline();

        assert_eq!(text(&app), expected, "{line}");
    }

    let mut config = Config::default();
    config.editor.smart_newline = false;
    let mut crlf = App::new(config, None).unwrap();
    seed(&mut crlf, "   alpha\r\nnext");
    crlf.mode = Mode::Insert;
    let caret = crlf.active_buffer().line_len(0);
    crlf.replace_active_selection(Selection::point(caret));

    crlf.edit_newline();

    assert_eq!(text(&crlf), "   alpha\r\n   \r\nnext");
}

#[test]
fn disabled_smart_newline_does_not_add_syntax_indentation() {
    let path = temporary("disabled-smart-newline.rs");
    fs::write(&path, "fn outer() {\n    if ready {\n    }\n}\n").unwrap();
    let mut config = Config::default();
    config.editor.smart_newline = false;
    let mut app = App::new(config, Some(path.clone())).unwrap();
    app.mode = Mode::Insert;
    let caret = app.buffers[0].line_to_offset(1) + app.buffers[0].line_len(1);
    app.replace_active_selection(Selection::point(caret));

    app.edit_newline();

    assert_eq!(text(&app), "fn outer() {\n    if ready {\n    \n    }\n}\n");
    fs::remove_file(path).unwrap();
}

/// View alignment moves the pane rather than the caret: the same line ends up
/// at the top, the middle, or the bottom of the viewport without the document
/// position changing.
#[test]
fn view_alignment_places_the_cursor_line_at_the_top_middle_or_bottom() {
    let mut app = App::new(Config::default(), None).unwrap();
    let document = (0..60).fold(String::new(), |mut text, line| {
        text.push_str(&format!("line {line}\n"));
        text
    });
    seed(&mut app, &document);
    let viewport = app.viewport_height();
    assert_eq!(viewport, 20, "the default viewport this test counts in");
    set_cursor(&mut app, 30, 0);

    for (key, expected) in [
        ('t', 30),
        ('b', 30 - (viewport - 1)),
        ('z', 30 - viewport / 2),
    ] {
        press(&mut app, 'z');
        press(&mut app, key);
        assert_eq!(app.active().scroll_row, expected, "z{key}");
        assert_eq!(cursor(&app), Position::new(30, 0), "z{key} moved the caret");
    }
}

/// Aligning the middle is the horizontal half of the same idea, and is the one
/// alignment measured against the pane's width rather than its height.
#[test]
fn aligning_the_middle_centers_the_cursor_column_in_the_pane_width() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, &"x".repeat(400));
    set_cursor(&mut app, 0, 200);

    press(&mut app, 'z');
    press(&mut app, 'm');

    assert_eq!(app.active().scroll_col, 200 - 80 / 2);
    assert_eq!(cursor(&app), Position::new(0, 200));

    set_cursor(&mut app, 0, 10);
    press(&mut app, 'z');
    press(&mut app, 'm');
    assert_eq!(
        app.active().scroll_col,
        0,
        "a column nearer the start than half a pane cannot scroll behind itself"
    );
}

/// Under soft wrap the alignment is resolved in screen rows, so the pane
/// starts part-way through a line and remembers which of its segments that is.
#[test]
fn soft_wrapped_alignment_and_scrolling_are_measured_in_wrapped_segments() {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "aaaaaa\nbbbbbb\ncccccc");
    app.active_mut().wrap_width = 3;

    // Six screen rows in a twenty-row viewport, so the top alignment is the
    // only one with anywhere to go.
    set_cursor(&mut app, 2, 3);
    press(&mut app, 'z');
    press(&mut app, 't');
    assert_eq!(
        (app.active().scroll_row, app.active().scroll_wrap),
        (2, 1),
        "the pane starts at the second segment of the last line"
    );

    press(&mut app, 'z');
    press(&mut app, 'k');
    assert_eq!(
        (app.active().scroll_row, app.active().scroll_wrap),
        (2, 0),
        "one screen row back is the earlier segment of the same line"
    );

    press(&mut app, 'z');
    press(&mut app, 'k');
    assert_eq!(
        (app.active().scroll_row, app.active().scroll_wrap),
        (1, 1),
        "the row before it is entered at its last segment, not its first"
    );

    press(&mut app, 'z');
    press(&mut app, 'j');
    assert_eq!((app.active().scroll_row, app.active().scroll_wrap), (2, 0));
}

/// Without soft wrap a screen row is a line, so scrolling the view is a row at
/// a time in both directions and stops at the document's own edges.
#[test]
fn scrolling_the_view_without_soft_wrap_moves_one_line_and_stops_at_the_top() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "one\ntwo\nthree\nfour\n");
    app.active_mut().scroll_row = 2;

    press(&mut app, 'z');
    press(&mut app, 'k');
    assert_eq!(app.active().scroll_row, 1);

    press(&mut app, 'z');
    press(&mut app, 'k');
    assert_eq!(app.active().scroll_row, 0);

    press(&mut app, 'z');
    press(&mut app, 'k');
    assert_eq!(app.active().scroll_row, 0, "the first line is the top");

    press(&mut app, 'z');
    press(&mut app, 'j');
    assert_eq!(app.active().scroll_row, 1);
}
