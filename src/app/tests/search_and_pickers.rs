// SPDX-License-Identifier: MPL-2.0

use super::*;

/// Drives the finder the way the event loop does, until nothing is left to do.
///
/// Terminal output only marks a session dirty; reading it back is a separate,
/// deliberately slow step, so a test that wants the finder settled has to take
/// both — and a refresh may itself queue a pass.
#[allow(dead_code)]
fn settle_finder(app: &mut App) {
    loop {
        while app.resource_finder_scan_pending() {
            app.advance_resource_finder_scan();
        }
        if !app.refresh_finder_terminals() {
            break;
        }
    }
}

/// Applies a background picker event the way an event loop does.
///
/// The loop's pacing wake-up follows a ranked answer within the list
/// interval, so a test that is not about pacing takes the answer straight
/// away rather than waiting a second of wall clock for it.
fn deliver(app: &mut App, event: FilePickerEvent) {
    app.apply_file_picker_event(event);
    app.publish_paced_picker_rows();
}

/// Runs the picker's own clocks the way the event loop does when
/// [`App::picker_pacing_delay`] comes round, without a test waiting out the
/// interval in wall clock.
fn pacing_tick(app: &mut App) {
    if let Some(due) = app.content_rescan_due {
        app.content_rescan_due = Some(due - PICKER_LIST_INTERVAL);
    }
    app.advance_picker_pacing();
}

fn content_hits(path: &str, lines: usize) -> crate::file_picker::FileHits {
    crate::file_picker::FileHits {
        path: PathBuf::from(path),
        lines: (0..lines)
            .map(|row| crate::file_picker::LineHit {
                row,
                column: 0,
                text: format!("match {row}"),
            })
            .collect(),
    }
}

#[test]
fn disk_hits_use_only_the_unified_content_budget_left_by_resources() {
    let mut entries = vec![
        content_hits("a.txt", 2),
        content_hits("b.txt", 2),
        content_hits("c.txt", 1),
    ];

    assert!(super::super::picker_workflows::truncate_content_hits(
        &mut entries,
        3
    ));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].len(), 2);
    assert_eq!(entries[1].len(), 1);
    assert_eq!(
        entries
            .iter()
            .map(crate::file_picker::FileHits::len)
            .sum::<usize>(),
        3
    );
}

#[test]
fn search_prompt_repeats_and_wraps_unicode_matches() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "α one\ntwo α");

    press(&mut app, '/');
    assert_eq!(app.prompt_kind, PromptKind::Search(SearchMode::Regex));
    press(&mut app, 'α');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    // Both matches are selected at once, and the caret sits on the one the
    // cursor was already at or before.
    assert_eq!(app.active().selection.len(), 2);
    assert_eq!(cursor(&app), Position::default());
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.status, "match 1/2 (all selected): α");

    press(&mut app, 'n');
    assert_eq!(app.active().selection.len(), 1);
    assert_eq!(cursor(&app), Position::new(1, 4));
    assert_eq!(app.status, "match 2/2: α");
    press(&mut app, 'N');
    assert_eq!(app.active().selection.len(), 1);
    assert_eq!(cursor(&app), Position::default());

    press(&mut app, 'n');
    assert_eq!(app.active().selection.len(), 1);
    assert_eq!(cursor(&app), Position::new(1, 4));

    press(&mut app, 'c');
    press(&mut app, 'z');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(text(&app), "α one\ntwo z");
}

#[test]
fn pane_swap_moves_pristine_search_presentation_with_its_content() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "cat dog cat");
    press(&mut app, 's');
    type_text(&mut app, "cat");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let searched = app.active_pane;
    assert!(app.pristine_search_selection(searched));

    app.split(Axis::Horizontal, None).unwrap();
    let clone = app.active_pane;
    app.panes
        .get_mut(&clone)
        .unwrap()
        .replace_selection(Selection::point(4));
    app.swap_window();

    assert_eq!(app.active_pane, searched);
    assert_eq!(app.search_selection.unwrap().pane, clone);
    assert!(app.pristine_search_selection(clone));
    assert!(!app.pristine_search_selection(searched));
}

#[test]
fn scalar_prompt_editing_supports_character_word_and_line_controls() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "search target");
    press(&mut app, 's');
    for character in "alpha beta".chars() {
        press(&mut app, character);
    }

    key(&mut app, KeyCode::Left, Modifiers::NONE);
    key(&mut app, KeyCode::Char('b'), Modifiers::ALT);
    assert_eq!(app.command_cursor, 6);
    key(&mut app, KeyCode::Char('f'), Modifiers::ALT);
    key(&mut app, KeyCode::Right, Modifiers::NONE);
    assert_eq!(app.command_cursor, app.command.chars().count());
    key(&mut app, KeyCode::Home, Modifiers::NONE);
    key(&mut app, KeyCode::Right, Modifiers::NONE);
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert_eq!(app.command, "apha beta");
    key(&mut app, KeyCode::End, Modifiers::NONE);
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(app.command, "apha bet");

    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('f'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('d'), Modifiers::CONTROL);
    assert_eq!(app.command, "aha bet");
    key(&mut app, KeyCode::Char('e'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('b'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('h'), Modifiers::CONTROL);
    assert_eq!(app.command, "aha bt");
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    assert_eq!(app.command, "aha t");
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert_eq!(app.command, "t");

    type_text(&mut app, "wo words");
    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('k'), Modifiers::CONTROL);
    assert!(app.command.is_empty());
    key(&mut app, KeyCode::Char('z'), Modifiers::CONTROL);
    assert_eq!(app.mode, Mode::Command);
    key(&mut app, KeyCode::Char('c'), Modifiers::CONTROL);
    assert_eq!(app.mode, Mode::Normal);
}

/// Runs one of the search prompts and submits `pattern`.
fn search_for(app: &mut App, opener: char, pattern: &str) {
    press(app, opener);
    type_text(app, pattern);
    key(app, KeyCode::Enter, Modifiers::NONE);
}

#[test]
fn search_flavours_fold_case_and_take_literals_literally() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "Foo foo FOO");

    search_for(&mut app, 's', "foo");
    assert_eq!(app.prompt_kind, PromptKind::Command, "the prompt closed");
    assert_eq!(app.active().selection.len(), 3, "`s` ignores case");

    app.active_mut().selection = Selection::point(0);
    search_for(&mut app, '/', "foo");
    assert_eq!(app.active().selection.len(), 1, "`/` respects case");

    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "a(b) and axb");
    search_for(&mut app, 's', "a(b");
    assert!(
        !app.status_error,
        "a literal pattern is not a regex: {}",
        app.status
    );
    assert_eq!(app.active().selection.len(), 1);

    // The same text through `/` is a regular expression, and an invalid one.
    app.active_mut().selection = Selection::point(0);
    search_for(&mut app, '/', "a(b");
    assert!(app.status_error);
}

#[test]
fn every_match_is_selected_with_the_caret_on_its_last_character() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "let foo = foo_bar(foo);");

    search_for(&mut app, 's', "foo");
    let ranges = app.active().selection.ranges().to_vec();
    assert_eq!(ranges.len(), 3);
    for range in &ranges {
        // Forward, so the whole match is selected and the caret sits on its
        // last character, where an append or a motion continues from.
        assert_eq!(range.head, range.to());
        assert_eq!(
            operative_span(app.active_buffer(), range).1 - range.from(),
            3
        );
    }
    assert_eq!(
        ranges.iter().map(|range| range.from()).collect::<Vec<_>>(),
        vec![4, 10, 18]
    );
}

#[test]
fn a_selection_scopes_the_search_and_confines_cycling_to_it() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "foo\nfoo\nfoo\nfoo");

    // Two selected lines, which is what `x x` leaves behind.
    press(&mut app, 'x');
    press(&mut app, 'x');
    assert_eq!(app.active().selection.len(), 1);

    search_for(&mut app, 's', "foo");
    assert_eq!(app.active().selection.len(), 2, "only the selected lines");

    // Cycling wraps inside the region rather than escaping into rows 2-3.
    let rows = |app: &App| cursor(app).row;
    press(&mut app, 'n');
    assert_eq!(app.active().selection.len(), 1);
    assert_eq!(rows(&app), 1);
    press(&mut app, 'n');
    assert_eq!(rows(&app), 0, "wrapped back to the top of the region");
    press(&mut app, 'N');
    assert_eq!(rows(&app), 1);
}

#[test]
fn a_bare_caret_does_not_scope_a_search() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "foo\nfoo\nfoo");
    // A caret is a one-character range in this grammar, so it must not be
    // mistaken for a selection.
    assert!(app.active().selection.primary().is_empty());

    search_for(&mut app, 's', "foo");
    assert_eq!(app.active().selection.len(), 3);
}

#[test]
fn successive_searches_narrow_into_the_previous_matches() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha beta\nalpha gamma\nalpha delta");

    press(&mut app, 'x');
    press(&mut app, 'x');
    search_for(&mut app, 's', "alpha");
    assert_eq!(app.active().selection.len(), 2, "the two selected lines");

    // The matches are themselves a selection, so the next search narrows
    // again: four `a`s inside two `alpha`s, not every `a` in the buffer.
    search_for(&mut app, '/', "a");
    assert_eq!(app.active().selection.len(), 4);
    assert!(
        app.active().selection.ranges().iter().all(|range| app
            .active_buffer()
            .position_of(range.from())
            .row
            < 2)
    );
}

#[test]
fn star_selects_every_occurrence_of_the_word_under_the_caret() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha beta Alpha alpha");

    press(&mut app, '*');
    assert_eq!(
        app.active().selection.len(),
        2,
        "case-sensitive, so `Alpha` is a different word"
    );
}

#[test]
fn filtering_selections_prompts_for_its_own_pattern() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "keep one\ndrop two\nkeep three");

    // No search has run, so the old shared-pattern coupling would have
    // refused this outright.
    search_for(&mut app, '/', "(?m)^.+$");
    assert_eq!(app.active().selection.len(), 3);

    type_text(&mut app, " sk");
    assert_eq!(app.prompt_kind, PromptKind::FilterSelections { keep: true });
    type_text(&mut app, "keep");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().selection.len(), 2);

    type_text(&mut app, " sr");
    assert_eq!(
        app.prompt_kind,
        PromptKind::FilterSelections { keep: false }
    );
    type_text(&mut app, "one");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().selection.len(), 1);
}

#[test]
fn a_failed_search_keeps_the_previous_one_working() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "foo\nbar\nfoo");

    search_for(&mut app, 's', "foo");
    assert_eq!(app.active().selection.len(), 2);

    search_for(&mut app, 's', "absent");
    // A search that ran cleanly and found nothing is informational, not a
    // failure: it does not take the interaction line's error styling.
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.status, "pattern not found: absent");
    assert_eq!(
        app.displayed_status_message(),
        "s (pattern not found: absent)"
    );
    assert!(!app.displayed_status_message_is_error());
    assert_eq!(app.unread_notification_counts().infos, 1);
    assert_eq!(app.unread_notification_counts().warnings, 0);
    // `n` still walks the search that did work.
    press(&mut app, 'n');
    assert_eq!(app.active().selection.len(), 1);
    assert!(!app.status_error, "{}", app.status);
}

#[test]
fn an_invalid_regex_from_the_search_prompt_echoes_as_an_error() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "a(b) and axb");

    search_for(&mut app, '/', "a(b");
    assert!(app.status_error);
    assert!(app.displayed_status_message_is_error());
    assert!(
        app.displayed_status_message()
            .contains("invalid regular expression"),
        "{}",
        app.displayed_status_message()
    );
}

#[test]
fn a_vim_search_prompt_with_no_match_echoes_as_information() {
    let mut app = vim_app("foo bar");

    search_for(&mut app, '/', "absent");
    assert!(!app.status_error, "{}", app.status);
    assert!(!app.displayed_status_message_is_error());
    assert_eq!(
        app.displayed_status_message(),
        "/ (pattern not found: absent)"
    );
}

#[test]
fn view_and_window_modes_update_presentation_state() {
    let mut app = App::new(Config::default(), None).unwrap();
    let content = (0..100)
        .map(|row| format!("line {row}"))
        .collect::<Vec<_>>()
        .join("\n");
    seed(&mut app, &content);
    set_cursor(&mut app, 50, 3);
    app.areas.insert(
        0,
        Rect {
            width: 40,
            height: 12,
            ..Rect::default()
        },
    );

    press(&mut app, 'z');
    press(&mut app, 'z');
    assert_eq!(app.active().scroll_row, 45);

    press(&mut app, 'Z');
    press(&mut app, 'j');
    assert_eq!(app.active().scroll_row, 46);
    assert_eq!(app.pending_sequence().to_string(), "Z");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.pending_sequence().is_empty());

    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, 's');
    assert_eq!(app.panes.len(), 2);
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    press(&mut app, 'o');
    assert_eq!(app.panes.len(), 1);
}

#[test]
fn insert_mode_word_and_line_deletion_bindings_edit_without_literal_input() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, 'i');
    type_text(&mut app, "hello world");
    key(&mut app, KeyCode::Backspace, Modifiers::ALT);
    assert_eq!(text(&app), "hello ");
    app.active_mut().selection = Selection::point(0);
    key(&mut app, KeyCode::Delete, Modifiers::ALT);
    assert_eq!(text(&app), " ");
    app.active_mut().selection = Selection::point(1);
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert_eq!(text(&app), "");
    key(&mut app, KeyCode::Char('j'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(text(&app), "\n    ");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn insert_backspace_and_delete_treat_crlf_as_one_line_break() {
    let mut backspace = App::new(Config::default(), None).unwrap();
    seed(&mut backspace, "alpha\r\nbravo");
    backspace.mode = Mode::Insert;
    backspace.active_mut().selection =
        Selection::point(backspace.active_buffer().line_to_offset(1));

    key(&mut backspace, KeyCode::Backspace, Modifiers::NONE);

    assert_eq!(text(&backspace), "alphabravo");

    let mut delete = App::new(Config::default(), None).unwrap();
    seed(&mut delete, "alpha\r\nbravo");
    delete.mode = Mode::Insert;
    delete.active_mut().selection = Selection::point(5);

    key(&mut delete, KeyCode::Delete, Modifiers::NONE);

    assert_eq!(text(&delete), "alphabravo");

    let mut selected_cr = App::new(Config::default(), None).unwrap();
    seed(&mut selected_cr, "alpha\r\nbravo");
    selected_cr.mode = Mode::Insert;
    selected_cr.active_mut().selection = Selection::single(Range::new(5, 6));

    key(&mut selected_cr, KeyCode::Backspace, Modifiers::NONE);

    assert_eq!(text(&selected_cr), "alphabravo");

    let mut selected_lf = App::new(Config::default(), None).unwrap();
    seed(&mut selected_lf, "alpha\r\nbravo");
    selected_lf.mode = Mode::Insert;
    selected_lf.active_mut().selection = Selection::single(Range::new(6, 7));

    key(&mut selected_lf, KeyCode::Delete, Modifiers::NONE);

    assert_eq!(text(&selected_lf), "alphabravo");
}

#[test]
fn alt_delete_deletes_forward_by_word_class_across_unicode_and_lines() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "one \nβeta!");
    app.mode = Mode::Insert;
    app.active_mut().selection = Selection::point(3);

    key(&mut app, KeyCode::Delete, Modifiers::ALT);

    assert_eq!(text(&app), "one!");
    assert_eq!(app.active().selection, Selection::point(3));
}

#[test]
fn insert_exact_prefix_fallback_executes_then_reprocesses_literal_key() {
    let keymap = Box::leak(Box::new(
        Keymap::new(vec![
            Binding::implemented(
                &[Mode::Insert],
                KeyStroke::ctrl('w'),
                EditorCommand::DeleteWordBackward,
            ),
            Binding::implemented(
                &[Mode::Insert],
                [KeyStroke::ctrl('w'), KeyStroke::char('h')],
                EditorCommand::FocusWindowLeft,
            ),
        ])
        .unwrap(),
    ));
    let mut app = App::new(Config::default(), None).unwrap();
    app.set_keymap(keymap);
    app.mode = Mode::Insert;
    type_text(&mut app, "hello");

    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    assert_eq!(app.pending_sequence().to_string(), "Ctrl-w");
    press(&mut app, 'x');

    assert_eq!(text(&app), "x");
    assert!(app.pending_sequence().is_empty());
    assert_eq!(app.mode, Mode::Insert);
}

#[test]
fn yank_paste_and_prompt_editing_use_unicode_character_positions() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "αβc");

    press(&mut app, 'v');
    press(&mut app, 'l');
    press(&mut app, 'y');
    press(&mut app, 'p');
    // The yank leaves a caret on the last character it copied, so `p` pastes
    // past that character rather than over the two-byte one under it.
    assert_eq!(text(&app), "αβαβc");
    assert_eq!(app.mode, Mode::Normal, "buffer paste stays in Normal mode");

    press(&mut app, '/');
    type_text(&mut app, "α beta");
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    assert_eq!(app.command, "α ");
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert!(app.command.is_empty());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn visual_yank_includes_the_character_under_the_cursor() {
    for mut app in [App::new(Config::default(), None).unwrap(), vim_app("")] {
        seed(&mut app, "Test Test");

        for motion in ['e', 'w'] {
            app.active_mut().replace_selection(Selection::point(0));
            app.enter_normal_mode();
            press(&mut app, 'v');
            press(&mut app, motion);
            press(&mut app, 'y');

            let expected = if motion == 'e' { "Test" } else { "Test T" };
            assert_eq!(
                app.read_selected_register().text,
                expected,
                "after v{motion}y"
            );
        }
    }
}

#[test]
fn picker_control_bindings_page_and_open_in_a_split() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        temporary_directory().join(format!("runyte-picker-{}-{unique}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    for index in 0..15 {
        fs::write(
            directory.join(format!("{index:02}.txt")),
            format!("{index}"),
        )
        .unwrap();
    }

    let mut app = App::new(Config::default(), None).unwrap();
    let mut picker = FilePicker::new(
        1,
        directory.clone(),
        crate::file_picker::ScanScope::ignoring(&directory),
    );
    picker.add_paths(
        (0..15)
            .map(|index| ScanEntry::file(directory.join(format!("{index:02}.txt"))))
            .collect(),
    );
    picker.finish(0, false);
    app.picker = Some(picker);

    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
    key(&mut app, KeyCode::Char('n'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().selected, 2);
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
    key(&mut app, KeyCode::Char('p'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::PageDown, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 10);
    key(&mut app, KeyCode::Char('d'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().selected, 14);
    key(&mut app, KeyCode::PageUp, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 4);
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::End, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 14);
    key(&mut app, KeyCode::Home, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
    key(&mut app, KeyCode::BackTab, Modifiers::SHIFT);
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);

    type_text(&mut app, "12 tail");
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 7);
    key(&mut app, KeyCode::Left, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 6);
    key(&mut app, KeyCode::Char('b'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 5);
    key(&mut app, KeyCode::Right, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 6);
    key(&mut app, KeyCode::Char('f'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 7);
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query, "12 tai");
    key(&mut app, KeyCode::Char('h'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query, "12 ta");
    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 0);
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query, "2 ta");
    key(&mut app, KeyCode::Char('e'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 4);
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query, "2 ");
    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 0);
    key(&mut app, KeyCode::Char('k'), Modifiers::CONTROL);
    assert!(app.picker.as_ref().unwrap().query.is_empty());
    key(&mut app, KeyCode::Char('e'), Modifiers::CONTROL);

    let preview = app.picker.as_ref().unwrap().show_preview;
    key(&mut app, KeyCode::Char('t'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().show_preview, !preview);
    key(&mut app, KeyCode::Char('v'), Modifiers::CONTROL);

    assert!(app.picker.is_none());
    assert_eq!(app.panes.len(), 2);
    assert_eq!(
        app.active_buffer()
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .unwrap(),
        "00.txt"
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn background_picker_query_edits_remain_safe_while_the_scanner_owns_ranking() {
    let directory = temporary("background-picker-keyboard");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("alpha.txt"), "alpha").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_picker_at(
        directory.clone(),
        crate::file_picker::ScanScope::ignoring(&directory),
        FilePickerKind::Files,
    )
    .unwrap();
    assert!(app.file_scanner.is_some());

    type_text(&mut app, "alpha tail");
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha tail");
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 10);
    key(&mut app, KeyCode::Left, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 9);
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha tal");
    key(&mut app, KeyCode::Char('h'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha tl");
    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 0);
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query, "lpha tl");
    key(&mut app, KeyCode::Char('e'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 7);
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query, "lpha ");
    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query_cursor, 0);
    key(&mut app, KeyCode::Char('k'), Modifiers::CONTROL);
    assert!(app.picker.as_ref().unwrap().query.is_empty());

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.picker.is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn space_closes_a_new_picker_but_remains_a_query_separator_after_text() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.picker = Some(FilePicker::new(
        1,
        PathBuf::from("/project"),
        crate::file_picker::ScanScope::ignoring("/project"),
    ));

    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    assert!(app.picker.is_none(), "initial Space dismisses the picker");

    let mut picker = FilePicker::new(
        2,
        PathBuf::from("/project"),
        crate::file_picker::ScanScope::ignoring("/project"),
    );
    picker.insert_query_text("src");
    app.picker = Some(picker);
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query, "src ");
}

#[test]
fn space_closes_a_new_result_list_but_remains_a_filter_separator_after_text() {
    let items = || {
        vec![PickerItem::searchable(
            "abcdef123456 Refresh workspace Git state",
            "",
            "Refresh workspace Git state Ada 2026-08-16 abcdef123456",
            0,
        )]
    };
    let mut app = App::new(Config::default(), None).unwrap();
    app.list = Some(ListPicker::fuzzy("Git commits", items()));

    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    assert!(app.list.is_none(), "initial Space dismisses the list");

    app.list = Some(ListPicker::fuzzy("Git commits", items()));
    for character in "workspace".chars() {
        press(&mut app, character);
    }
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    press(&mut app, 'g');
    assert_eq!(
        app.list.as_ref().map(|list| list.filter.as_str()),
        Some("workspace g"),
        "a space narrows a commit search the way it narrows the finder"
    );
    assert_eq!(app.list.as_ref().unwrap().visible_indices(), vec![0]);

    // A report has no filter for a space to belong to.
    app.list = Some(ListPicker::new("Service health", items()).as_report());
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    assert!(app.list.is_none(), "Space still closes a report");
}

#[test]
fn project_finder_switches_name_and_content_modes_without_losing_its_query() {
    let root = temporary("project-finder-modes");
    fs::create_dir_all(&root).unwrap();
    let alpha = root.join("alpha.txt");
    let beta = root.join("beta.txt");
    fs::write(&alpha, "alpha\n").unwrap();
    fs::write(&beta, "beta\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(alpha.clone()).unwrap();
    app.open_file(beta).unwrap();

    app.open_project_picker().unwrap();
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Names);
    type_text(&mut app, "alpha");
    assert_eq!(app.finder.as_ref().unwrap().matches.len(), 1);
    assert!(matches!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(FinderTarget::Resource(ResourceTarget::Buffer(_)))
    ));

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let finder = app.finder.as_ref().unwrap();
    assert_eq!(finder.mode, FinderMode::Contents);
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha");
    assert_eq!(app.picker.as_ref().unwrap().kind, FilePickerKind::Contents);
    assert!(!finder.matches.is_empty());
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
        .unwrap();
    assert!(
        overlay.title.starts_with("Finder · Contents · ") && overlay.title.ends_with(" matched"),
        "the header names the mode and says what its counts count: {}",
        overlay.title
    );
    assert!(
        overlay
            .message
            .as_deref()
            .is_none_or(|message| !message.contains("Scanning")),
        "an attached client's header does not report whether a scan is running: {:?}",
        overlay.message
    );
    assert_eq!(overlay.layout, crate::snapshot::OverlayLayout::Preview);
    assert_eq!(overlay.preview_title.as_deref(), Some("Contents"));
    // A live buffer's content match previews the same snippet a file on disk
    // does, so the matched text is highlighted wherever the row came from.
    let Some(crate::snapshot::OverlayPreview::Snippet {
        lines,
        focus_row,
        emphasis,
        ..
    }) = &overlay.preview
    else {
        panic!("a content match previews a snippet: {:?}", overlay.preview);
    };
    assert_eq!(*focus_row, 0);
    assert_eq!(
        emphasis
            .iter()
            .map(|position| lines[0].chars().nth(*position).unwrap())
            .collect::<String>(),
        "alpha"
    );
    assert!(
        overlay
            .actions
            .iter()
            .any(|action| { action.key_hint == "Tab" && action.label == "names" })
    );
    assert!(
        overlay
            .actions
            .iter()
            .any(|action| { action.key_hint == "Ctrl-t" && action.label == "toggle preview" })
    );
    assert!(
        overlay
            .actions
            .iter()
            .all(|action| action.key_hint != "Ctrl-s" && action.key_hint != "Ctrl-v"),
        "live buffer results must not advertise file-only split actions"
    );

    key(&mut app, KeyCode::Char('t'), Modifiers::CONTROL);
    assert!(!app.picker.as_ref().unwrap().show_preview);

    key(&mut app, KeyCode::BackTab, Modifiers::SHIFT);
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Contents);
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha");
    assert!(!app.picker.as_ref().unwrap().show_preview);

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Names);
    assert_eq!(app.picker.as_ref().unwrap().kind, FilePickerKind::Files);
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha");
    assert!(!app.picker.as_ref().unwrap().show_preview);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.picker.is_none());
    assert!(app.finder.is_none());
    assert_eq!(app.active_buffer().path.as_deref(), Some(alpha.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_finder_keeps_file_split_activation() {
    let root = temporary("project-finder-file-split");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("split-target.txt");
    fs::write(&target, "split target\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();

    app.open_project_picker().unwrap();
    type_text(&mut app, "split-target");
    assert!(matches!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(FinderTarget::File(_))
    ));
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
        .unwrap();
    assert!(
        overlay
            .actions
            .iter()
            .any(|action| action.key_hint == "Ctrl-s")
    );
    assert!(
        overlay
            .actions
            .iter()
            .any(|action| action.key_hint == "Ctrl-v")
    );
    key(&mut app, KeyCode::Char('s'), Modifiers::CONTROL);

    assert!(app.picker.is_none());
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.active_buffer().path.as_deref(), Some(target.as_path()));

    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_project_picker().unwrap();
    type_text(&mut app, "split-target");
    key(&mut app, KeyCode::Char('v'), Modifiers::CONTROL);
    assert!(app.picker.is_none());
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.active_buffer().path.as_deref(), Some(target.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_finder_snapshot_reports_filesystem_scan_failure() {
    let root = temporary("project-finder-scan-failure");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_project_picker().unwrap();
    let scan_id = app.picker.as_ref().unwrap().scan_id;
    deliver(
        &mut app,
        FilePickerEvent::Failed {
            scan_id,
            message: "discovery refused".to_owned(),
        },
    );

    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
        .unwrap();
    assert_eq!(
        overlay.message.as_deref(),
        Some("Scan failed: discovery refused")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_picker_keeps_tab_navigation_and_has_no_resource_mode() {
    let root = temporary("directory-picker-tab");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_picker_at(
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
        FilePickerKind::Files,
    )
    .unwrap();

    assert!(app.finder.is_none());
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_finder_keyboard_editing_and_navigation_are_symmetric() {
    let root = temporary("project-finder-keyboard");
    fs::create_dir_all(&root).unwrap();
    for index in 0..25 {
        fs::write(
            root.join(format!("entry-{index:02}.txt")),
            format!("row {index}\n"),
        )
        .unwrap();
    }
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_project_picker().unwrap();
    let total = app.finder.as_ref().unwrap().matches.len();
    assert!(total >= 25);

    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Char('n'), Modifiers::CONTROL);
    assert_eq!(app.finder.as_ref().unwrap().selected, 2);
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    key(&mut app, KeyCode::Char('p'), Modifiers::CONTROL);
    assert_eq!(app.finder.as_ref().unwrap().selected, 0);

    key(&mut app, KeyCode::PageDown, Modifiers::NONE);
    key(&mut app, KeyCode::Char('d'), Modifiers::CONTROL);
    assert_eq!(app.finder.as_ref().unwrap().selected, 20);
    key(&mut app, KeyCode::PageUp, Modifiers::NONE);
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert_eq!(app.finder.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::End, Modifiers::NONE);
    assert_eq!(app.finder.as_ref().unwrap().selected, total - 1);
    key(&mut app, KeyCode::Home, Modifiers::NONE);
    assert_eq!(app.finder.as_ref().unwrap().selected, 0);

    type_text(&mut app, "entry-12 tail");
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.query_cursor, picker.query.chars().count());
    key(&mut app, KeyCode::Left, Modifiers::NONE);
    key(&mut app, KeyCode::Char('b'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Right, Modifiers::NONE);
    key(&mut app, KeyCode::Char('f'), Modifiers::CONTROL);
    assert_eq!(
        app.picker.as_ref().unwrap().query_cursor,
        app.picker.as_ref().unwrap().query.chars().count()
    );
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    key(&mut app, KeyCode::Char('h'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query, "entry-12 ta");
    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query, "ntry-12 ta");
    key(&mut app, KeyCode::Char('e'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().query, "ntry-12 ");
    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('k'), Modifiers::CONTROL);
    assert!(app.picker.as_ref().unwrap().query.is_empty());
    key(&mut app, KeyCode::Char('e'), Modifiers::CONTROL);

    let preview = app.picker.as_ref().unwrap().show_preview;
    key(&mut app, KeyCode::Char('t'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().show_preview, !preview);
    key(&mut app, KeyCode::Null, Modifiers::NONE);
    assert!(
        app.picker.is_some(),
        "an unrelated key leaves the finder open"
    );

    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_finder_content_reaches_and_activates_a_pathless_buffer() {
    let root = temporary("project-finder-pathless-buffer");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("buffer-new").unwrap();
    let scratch = app.active().buffer;
    app.buffers[scratch].apply(&Transaction::insert(0, "first\npathless needle\nlast"));
    app.open_project_picker().unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    type_text(&mut app, "needle");
    let selected = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap());
    assert!(
        matches!(
            selected,
            Some(FinderTarget::Resource(ResourceTarget::BufferLocation {
                buffer,
                row: 1,
                ..
            })) if buffer == scratch
        ),
        "selected {selected:?}; items: {:?}",
        app.finder
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|item| (&item.label, &item.detail))
            .collect::<Vec<_>>()
    );
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
        .unwrap();
    let row = overlay.rows.get(overlay.selected.unwrap()).unwrap();
    let emphasized = row
        .detail_emphasis
        .iter()
        .map(|position| row.detail.chars().nth(*position).unwrap())
        .collect::<String>();
    assert_eq!(emphasized, "needle");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().buffer, scratch);
    assert_eq!(cursor(&app).row, 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pathless_buffer_content_is_scanned_in_bounded_slices() {
    let root = temporary("project-finder-bounded-buffer-scan");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("buffer-new").unwrap();
    let scratch = app.active().buffer;
    let text = (0..400)
        .map(|row| {
            if row == 350 {
                "Zunique live-buffer match".to_owned()
            } else {
                format!("ordinary row {row}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    app.buffers[scratch].apply(&Transaction::insert(0, text));

    app.open_project_picker().unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    type_text(&mut app, "Zunique");
    assert!(app.resource_finder_scan_pending());
    // A pass over a new query starts from nothing, so every row it finds is
    // progress and the loop is free to show it arriving.
    assert!(!app.finder_scan_refills());
    assert!(app.finder.as_ref().unwrap().matches.is_empty());
    settle_finder(&mut app);

    assert!(matches!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(FinderTarget::Resource(ResourceTarget::BufferLocation {
            buffer,
            row: 350,
            ..
        })) if buffer == scratch
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn finder_path_fields_include_project_home_and_basename_spellings() {
    let home = Path::new("/home/person");
    let root = home.join("code/runyte-dev");
    let path = root.join("src/main.rs");
    let fields = resource_path_fields(&path, &root, Some(home));
    assert!(fields.contains(&path.display().to_string()));
    assert!(fields.contains(&"src/main.rs".to_owned()));
    assert!(fields.contains(&"~/code/runyte-dev/src/main.rs".to_owned()));
    assert!(fields.contains(&"main.rs".to_owned()));
}

#[cfg(unix)]
#[test]
fn project_finder_indexes_terminal_names_and_content_and_reveals_the_matching_row() {
    let root = temporary("project-finder-terminal-directory");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let note = root.join("note.txt");
    fs::write(&note, "kept behind the terminal").unwrap();
    app.open_file(note).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let id = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id,
        bytes: b"combined finder terminal preview\r\n".to_vec(),
    });
    app.terminals
        .get_mut(id)
        .unwrap()
        .rename(Some("my_terminal_name".to_owned()))
        .unwrap();
    app.apply_terminal_output(TerminalOutput::Exited { id, code: Some(0) });
    assert!(
        app.terminals.get(id).is_some(),
        "exited output stays searchable"
    );

    app.open_project_picker().unwrap();
    type_text(
        &mut app,
        "terminal my_terminal_name project-finder-terminal cat",
    );
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(FinderTarget::Resource(ResourceTarget::Terminal(id)))
    );
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_preview()
            .is_some_and(|preview| preview
                .lines()
                .join("\n")
                .contains("combined finder terminal preview"))
    );
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
        .unwrap();
    assert_eq!(overlay.preview_title.as_deref(), Some("Output"));
    assert!(matches!(
        overlay.preview,
        Some(crate::snapshot::OverlayPreview::Text(ref lines))
            if lines.iter().any(|line| line.contains("combined finder terminal preview"))
    ));

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_terminal(), Some(id));
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.terminals.get(id).unwrap().reviewing());

    app.mode = Mode::Normal;
    app.open_project_grep().unwrap();
    type_text(&mut app, "preview");
    let selected = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap());
    assert!(
        matches!(
            selected,
            Some(FinderTarget::Resource(ResourceTarget::TerminalLocation {
                terminal,
                ..
            })) if terminal == id
        ),
        "selected {selected:?}; items: {:?}",
        app.finder
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|item| (&item.label, &item.detail))
            .collect::<Vec<_>>()
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_terminal(), Some(id));
    assert!(app.terminals.get(id).unwrap().reviewing());
    assert_eq!(app.mode, Mode::Normal);
    app.close_terminal_id(id);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn busy_terminal_updates_only_its_name_finder_item_and_selected_preview() {
    let root = temporary("project-finder-busy-terminal-name-mode");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("buffer-new").unwrap();
    let scratch = app.active().buffer;
    app.buffers[scratch].apply(&Transaction::insert(0, "kept scratch"));
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.open_project_picker().unwrap();
    let item_count = app.finder.as_ref().unwrap().items.len();
    let buffer_match = app
        .finder
        .as_ref()
        .unwrap()
        .matches
        .iter()
        .position(|found| {
            matches!(
                found.source,
                FinderMatchSource::Resource(item)
                    if app.finder.as_ref().unwrap().items[item].target
                        == ResourceTarget::Buffer(scratch)
            )
        })
        .unwrap();
    app.finder.as_mut().unwrap().first();
    for _ in 0..buffer_match {
        app.finder.as_mut().unwrap().down();
    }
    let claimed = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap());

    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b]2;hot-title\x07first visible row\r\n".to_vec(),
    });
    for row in 0..256 {
        app.apply_terminal_output(TerminalOutput::Bytes {
            id: terminal,
            bytes: format!("busy row {row}\r\n").into_bytes(),
        });
    }
    assert_eq!(app.finder.as_ref().unwrap().items.len(), item_count);
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        claimed
    );

    // 257 chunks are worth one refresh, not 257: the whole burst leaves one
    // dirty session behind for the tick to read.
    assert!(app.refresh_finder_terminals());
    assert!(!app.finder_terminals_dirty());
    assert_eq!(app.finder.as_ref().unwrap().items.len(), item_count);
    type_text(&mut app, "hot-title");
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(FinderTarget::Resource(ResourceTarget::Terminal(terminal)))
    );
    let preview = app.finder.as_ref().unwrap().selected_preview().unwrap();
    assert!(preview.lines().join("\n").contains("busy row 255"));
    assert!(preview.lines().len() <= 200);
    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn terminal_output_queues_a_bounded_incremental_finder_scan_for_the_refresh_tick() {
    let root = temporary("project-finder-live-terminal-output");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let id = app.active_terminal().unwrap();

    app.open_project_picker().unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    type_text(&mut app, "arrived-later");
    settle_finder(&mut app);
    assert!(app.finder.as_ref().unwrap().matches.is_empty());

    app.apply_terminal_output(TerminalOutput::Bytes {
        id,
        bytes: b"arrived-later\r\n".to_vec(),
    });
    assert!(
        app.finder_terminals_dirty(),
        "output marks the terminal for the refresh tick"
    );
    assert!(
        !app.resource_finder_scan_pending(),
        "output alone must not start a pass"
    );
    assert!(app.refresh_finder_terminals());
    assert!(app.resource_finder_scan_pending());
    assert!(app.finder.as_ref().unwrap().matches.is_empty());
    settle_finder(&mut app);

    assert!(matches!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(FinderTarget::Resource(ResourceTarget::TerminalLocation {
            terminal,
            ..
        })) if terminal == id
    ));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_busy_terminal_leaves_content_rows_standing_until_the_refresh_tick() {
    let root = temporary("project-finder-terminal-refresh-debounce");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"settled needle\r\n".to_vec(),
    });

    app.open_project_grep().unwrap();
    type_text(&mut app, "settled needle");
    settle_finder(&mut app);
    let settled = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap());
    assert!(matches!(
        settled,
        Some(FinderTarget::Resource(
            ResourceTarget::TerminalLocation { .. }
        ))
    ));
    let rows = app.finder.as_ref().unwrap().matches.len();

    for chunk in 0..200 {
        app.apply_terminal_output(TerminalOutput::Bytes {
            id: terminal,
            bytes: format!("unrelated chunk {chunk}\r\n").into_bytes(),
        });
        assert!(
            !app.resource_finder_scan_pending(),
            "a write must not start a pass of its own"
        );
        assert_eq!(
            app.finder.as_ref().unwrap().matches.len(),
            rows,
            "the list must hold still while the child writes"
        );
        assert_eq!(
            app.finder
                .as_ref()
                .unwrap()
                .selected_target(app.picker.as_ref().unwrap()),
            settled
        );
    }

    assert!(app.finder_terminals_dirty());
    settle_finder(&mut app);
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        settled,
        "the refresh keeps the row the reader had settled on"
    );

    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn repeated_terminal_output_does_not_starve_later_buffer_content() {
    let root = temporary("project-finder-terminal-output-coalescing");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("buffer-new").unwrap();
    let scratch = app.active().buffer;
    let text = (0..400)
        .map(|row| {
            if row == 350 {
                "late-buffer-needle".to_owned()
            } else {
                format!("ordinary row {row}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    app.buffers[scratch].apply(&Transaction::insert(0, text));
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();

    app.open_project_picker().unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    type_text(&mut app, "late-buffer-needle");
    for tick in 0..12 {
        app.apply_terminal_output(TerminalOutput::Bytes {
            id: terminal,
            bytes: format!("tick {tick}\r\n").into_bytes(),
        });
        // The refresh tick may fire between chunks. It must not abandon the
        // pass in flight, or the later buffer source is never reached.
        app.refresh_finder_terminals();
        app.advance_resource_finder_scan();
        if app.finder.as_ref().unwrap().items.iter().any(|item| {
            matches!(
                item.target,
                ResourceTarget::BufferLocation {
                    buffer,
                    row: 350,
                    ..
                } if buffer == scratch
            )
        }) {
            fs::remove_dir_all(root).unwrap();
            return;
        }
    }
    panic!("continuous terminal output starved the later buffer source");
}

#[cfg(unix)]
#[test]
fn terminal_output_after_a_complete_scan_refreshes_only_that_terminal() {
    let root = temporary("project-finder-idle-terminal-refresh");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("buffer-new").unwrap();
    let scratch = app.active().buffer;
    app.buffers[scratch].apply(&Transaction::insert(0, "preserved-needle in buffer"));
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"preserved-needle in terminal\r\n".to_vec(),
    });

    app.open_project_picker().unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    type_text(&mut app, "preserved-needle");
    settle_finder(&mut app);
    let buffer_match = app
        .finder
        .as_ref()
        .unwrap()
        .matches
        .iter()
        .position(|found| {
            matches!(
                found.source,
                FinderMatchSource::Resource(item)
                    if matches!(
                        app.finder.as_ref().unwrap().items[item].target,
                        ResourceTarget::BufferLocation { buffer, .. } if buffer == scratch
                    )
            )
        })
        .unwrap();
    app.finder.as_mut().unwrap().first();
    for _ in 0..buffer_match {
        app.finder.as_mut().unwrap().down();
    }
    let claimed = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap());

    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"new terminal row\r\n".to_vec(),
    });
    assert!(app.refresh_finder_terminals());
    assert!(app.resource_finder_scan_pending());
    assert!(app.finder.as_ref().unwrap().items.iter().any(|item| {
        matches!(
            item.target,
            ResourceTarget::BufferLocation { buffer, .. } if buffer == scratch
        )
    }));
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        claimed
    );
    settle_finder(&mut app);
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        claimed
    );
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_refilling_pass_is_marked_as_a_state_not_worth_showing() {
    let root = temporary("project-finder-terminal-refill-frames");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"refill needle\r\n".to_vec(),
    });

    app.open_project_grep().unwrap();
    type_text(&mut app, "refill needle");
    settle_finder(&mut app);
    let settled = app.finder.as_ref().unwrap().matches.len();
    assert!(settled > 0);

    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"unrelated row\r\n".to_vec(),
    });
    assert!(app.refresh_finder_terminals());
    assert!(
        app.finder_scan_refills(),
        "a refresh must declare itself a refill"
    );
    assert!(
        app.finder.as_ref().unwrap().matches.len() < settled,
        "the refresh drops the rows it is about to read back, so what stands \
         between it and the end of the pass is a list with a hole in it"
    );

    settle_finder(&mut app);
    assert!(!app.finder_scan_refills());
    assert_eq!(app.finder.as_ref().unwrap().matches.len(), settled);

    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn narrowing_a_terminal_reads_back_the_rows_it_truncated() {
    let root = temporary("project-finder-terminal-narrowed");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    // Nothing but the needle itself can spell the needle, so a truncated line
    // cannot go on matching by accident.
    let padding = "x".repeat(50);
    // Deep enough in history that the rows a narrowing retires cannot reach
    // it: only the width it was read at says this row has changed.
    let output = std::iter::once(format!("{padding}needle-past-the-fold\r\n"))
        .chain((0..40).map(|row| format!("ordinary row {row}\r\n")))
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: output.into_bytes(),
    });

    app.open_project_grep().unwrap();
    type_text(&mut app, "needle-past-the-fold");
    settle_finder(&mut app);
    assert!(
        app.finder.as_ref().unwrap().items.iter().any(|item| {
            matches!(item.target, ResourceTarget::TerminalLocation { .. })
                && item.detail.contains("needle-past-the-fold")
        }),
        "the row matches at its full width"
    );

    // Narrowing truncates every retained line in place and leaves its identity
    // alone, so identity alone cannot tell the finder that the row changed.
    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
    assert!(
        app.finder_terminals_dirty(),
        "a resize is a change the finder has to be told about"
    );
    settle_finder(&mut app);
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .items
            .iter()
            .all(|item| { !matches!(item.target, ResourceTarget::TerminalLocation { .. }) }),
        "the row no longer holds the text it was found by"
    );

    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_terminal_refresh_reads_only_what_the_child_added() {
    let root = temporary("project-finder-terminal-incremental-refresh");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    let keeper = crate::terminal::grid::SCROLLBACK_LIMIT;
    let initial = (0..crate::terminal::grid::SCROLLBACK_LIMIT + 64)
        .map(|row| {
            if row == keeper {
                "keeper needle\r\n".to_owned()
            } else {
                format!("ordinary row {row}\r\n")
            }
        })
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: initial.into_bytes(),
    });

    app.open_project_grep().unwrap();
    type_text(&mut app, "keeper needle");
    settle_finder(&mut app);
    let (line_id, label, row) = app
        .finder
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find_map(|item| match item.target {
            ResourceTarget::TerminalLocation { line_id, .. } => Some((
                line_id,
                item.label.clone(),
                app.terminals
                    .get(terminal)
                    .unwrap()
                    .retained_line_row(line_id)
                    .unwrap(),
            )),
            _ => None,
        })
        .expect("the keeper row is a content result");

    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"one more row\r\n".to_vec(),
    });
    assert!(app.refresh_finder_terminals());
    let mut passes = 0;
    while app.resource_finder_scan_pending() {
        app.advance_resource_finder_scan();
        passes += 1;
    }
    // Re-reading the whole session would take a pass for every 128 of its
    // 5000 retained rows. Only the row the child added, and the screen it
    // may have rewritten, are worth revisiting.
    assert!(
        passes <= 2,
        "a refresh read {passes} slices, so it re-read rows it already had"
    );

    let session = app.terminals.get(terminal).unwrap();
    let shifted = session.retained_line_row(line_id).unwrap();
    assert_ne!(shifted, row, "bounded history moved the kept row");
    let number = session.output_line_number(shifted);
    let item = app
        .finder
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|item| {
            matches!(
                item.target,
                ResourceTarget::TerminalLocation { line_id: found, .. } if found == line_id
            )
        })
        .expect("the kept row survives the refresh");
    assert_eq!(item.label, label, "a kept row keeps the name it was given");
    assert!(
        item.label.ends_with(&format!(":{number}")),
        "and that name still numbers the line it points at: {}",
        item.label
    );
    let FilePreview::Snippet(snippet) = app
        .finder
        .as_ref()
        .unwrap()
        .selected_preview()
        .expect("the selected terminal result has a preview")
    else {
        panic!("a terminal content result previews a snippet");
    };
    assert_eq!(
        snippet.focus_row + 1,
        usize::try_from(number).unwrap(),
        "the preview focus uses the same stable output number as its result"
    );
    let retained_start = FilePreview::snippet_rows(shifted).start;
    assert_eq!(
        snippet.start_row + 1,
        usize::try_from(session.output_line_number(retained_start)).unwrap(),
        "the snippet context also uses stable output numbering"
    );

    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn terminal_content_selection_follows_stable_line_identity_through_eviction() {
    let root = temporary("project-finder-terminal-stable-lines");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    let initial = (0..crate::terminal::grid::SCROLLBACK_LIMIT + 64)
        .map(|row| format!("stable-repeat {row}\r\n"))
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: initial.into_bytes(),
    });
    let target_row = 64;
    let (target_line_id, target_text) = app
        .terminals
        .get(terminal)
        .unwrap()
        .plain_line_with_id(target_row)
        .unwrap();

    app.open_project_grep().unwrap();
    type_text(&mut app, "stable-repeat");
    settle_finder(&mut app);
    let target_match = app
        .finder
        .as_ref()
        .unwrap()
        .matches
        .iter()
        .position(|found| {
            matches!(
                found.source,
                FinderMatchSource::Resource(item)
                    if app.finder.as_ref().unwrap().items[item].target
                        == ResourceTarget::TerminalLocation {
                            terminal,
                            line_id: target_line_id,
                            column: 0,
                        }
            )
        })
        .unwrap();
    app.finder.as_mut().unwrap().first();
    for _ in 0..target_match {
        app.finder.as_mut().unwrap().down();
    }
    let claimed = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap());

    let shifted = (0..10)
        .map(|row| format!("unrelated shift {row}\r\n"))
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: shifted.into_bytes(),
    });
    settle_finder(&mut app);
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        claimed,
        "the selected retained line must survive a row-index shift"
    );
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_preview()
            .is_some_and(|preview| preview.lines().join("\n").contains(&target_text))
    );

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let session = app.terminals.get_mut(terminal).unwrap();
    session.select_review_line(true, false);
    assert_eq!(session.review_selection_text(), target_text);
    app.leave_terminal();

    app.open_project_grep().unwrap();
    type_text(&mut app, "stable-repeat");
    settle_finder(&mut app);
    assert!(app.finder.as_ref().unwrap().items.iter().any(|item| {
        matches!(
            item.target,
            ResourceTarget::TerminalLocation { line_id, .. } if line_id == target_line_id
        )
    }));

    let evicting = (0..100)
        .map(|row| format!("eviction row {row}\r\n"))
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: evicting.into_bytes(),
    });
    settle_finder(&mut app);
    assert!(app.finder.as_ref().unwrap().items.iter().all(|item| {
        !matches!(
            item.target,
            ResourceTarget::TerminalLocation { line_id, .. } if line_id == target_line_id
        )
    }));

    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn terminal_content_selection_does_not_cross_primary_and_alternate_screens() {
    let root = temporary("project-finder-terminal-screen-identities");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"screen-identity primary".to_vec(),
    });

    app.open_project_grep().unwrap();
    type_text(&mut app, "screen-identity");
    settle_finder(&mut app);
    let primary = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap())
        .unwrap();
    app.finder.as_mut().unwrap().first();

    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b[?1049hscreen-identity alternate".to_vec(),
    });
    settle_finder(&mut app);
    let alternate = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap())
        .unwrap();
    assert_ne!(alternate, primary);
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_preview()
            .is_some_and(|preview| {
                let text = preview.lines().join("\n");
                text.contains("screen-identity alternate")
                    && !text.contains("screen-identity primary")
            })
    );
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .items
            .iter()
            .all(|item| { FinderTarget::Resource(item.target) != primary })
    );

    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b[?1049l".to_vec(),
    });
    settle_finder(&mut app);
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(primary),
        "the claimed primary-screen identity returns only with its own grid"
    );

    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn terminal_screen_clear_preserves_scrollback_match_identity() {
    let root = temporary("project-finder-terminal-clear-history");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    let output = std::iter::once("history-clear-match\r\n".to_owned())
        .chain((0..30).map(|row| format!("ordinary history {row}\r\n")))
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: output.into_bytes(),
    });

    app.open_project_grep().unwrap();
    type_text(&mut app, "history-clear-match");
    settle_finder(&mut app);
    app.finder.as_mut().unwrap().first();
    let claimed = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap())
        .unwrap();
    let session = app.terminals.get(terminal).unwrap();
    let screen_row = session.plain_line_count() - 1;
    let screen_id_before = session.plain_line_with_id(screen_row).unwrap().0;

    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b[2Jscreen-cleared".to_vec(),
    });
    let screen_id_after = app
        .terminals
        .get(terminal)
        .unwrap()
        .plain_line_with_id(screen_row)
        .unwrap()
        .0;
    assert_ne!(screen_id_after, screen_id_before);
    settle_finder(&mut app);

    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(claimed)
    );
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_preview()
            .is_some_and(|preview| preview.lines().join("\n").contains("history-clear-match"))
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let session = app.terminals.get_mut(terminal).unwrap();
    session.select_review_line(true, false);
    assert_eq!(session.review_selection_text(), "history-clear-match");

    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn terminal_content_activation_captures_before_a_shorter_pane_resize() {
    let root = temporary("project-finder-terminal-activation-resize");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b[24;1Hresize-identity-match\x1b[1;1H".to_vec(),
    });
    app.leave_terminal();
    app.areas.insert(
        app.active_pane,
        Rect {
            width: 82,
            height: 10,
            ..Rect::default()
        },
    );

    app.open_project_grep().unwrap();
    type_text(&mut app, "resize-identity-match");
    settle_finder(&mut app);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.active_terminal(), Some(terminal));
    assert_eq!(app.mode, Mode::Normal);
    let session = app.terminals.get_mut(terminal).unwrap();
    assert!(session.reviewing());
    session.select_review_line(true, false);
    assert_eq!(session.review_selection_text(), "resize-identity-match");

    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn terminal_content_activation_enforces_the_review_memory_budget_immediately() {
    let root = temporary("project-finder-terminal-activation-budget");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"budget-identity-match".to_vec(),
    });
    app.leave_terminal();

    app.open_project_grep().unwrap();
    type_text(&mut app, "budget-identity-match");
    settle_finder(&mut app);
    app.terminals.set_memory_budget_for_test(0);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.active_terminal(), None);
    assert!(!app.terminals.get(terminal).unwrap().reviewing());
    assert_eq!(
        app.status,
        "that terminal line exceeds the retained review budget"
    );

    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn dirty_terminal_rows_are_invalidated_when_another_source_reaches_the_limit() {
    let root = temporary("project-finder-dirty-terminal-at-limit");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.terminals.apply(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"fresh needle\r\n".to_vec(),
    });
    let terminal_line_id = (0..app.terminals.get(terminal).unwrap().plain_line_count())
        .find_map(|row| {
            app.terminals
                .get(terminal)
                .unwrap()
                .plain_line_with_id(row)
                .filter(|(_, line)| line.contains("fresh needle"))
                .map(|(line_id, _)| line_id)
        })
        .unwrap();

    let mut picker = FilePicker::grep(
        91,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.insert_query_text("needle");
    picker.add_content(vec![content_hits("disk.txt", CONTENT_ENTRY_LIMIT - 1)]);
    picker.finish(0, false);
    let mut finder = ResourceFinder::new(FinderMode::Contents);
    finder.begin_content_scan(&picker, "needle", std::iter::empty());
    finder.append_items(
        [ResourceItem::content(
            "terminal:1",
            "stale needle",
            ResourceTarget::TerminalLocation {
                terminal,
                line_id: terminal_line_id,
                column: 6,
            },
            ResourceKind::Terminal,
        )],
        &picker,
        "needle",
    );
    finder.finish_content_scan(false);
    app.picker = Some(picker);
    app.finder = Some(finder);
    app.finder_content_scan = Some(FinderContentScan {
        query: "needle".to_owned(),
        sources: vec![FinderContentSource::Buffer {
            buffer: 0,
            label: "scratch".to_owned(),
            path: None,
        }]
        .into(),
        source: 0,
        row: 0,
        column: 0,
        retirements: Vec::new(),
        retirement: 0,
        limited: false,
        refilling: false,
        drop_observer: None,
    });
    app.finder_dirty_terminals.insert(terminal);

    // A pass in flight owns its cursor, so the dirty terminal waits for it.
    assert!(!app.refresh_finder_terminals());
    app.advance_resource_finder_scan();
    assert!(!app.resource_finder_scan_pending());
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|item| item.detail == "stale needle")
    );

    // The refresh drops what it is about to re-read before reading it, so a
    // corpus already at its ceiling cannot leave the stale rows standing.
    assert!(app.refresh_finder_terminals());
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .items
            .iter()
            .all(|item| item.detail != "stale needle")
    );
    settle_finder(&mut app);
    let finder = app.finder.as_ref().unwrap();
    assert!(finder.limited);
    assert!(
        finder
            .items
            .iter()
            .any(|item| item.detail == "fresh needle")
    );
    assert_eq!(
        finder.items.len() + app.picker.as_ref().unwrap().entries.len(),
        CONTENT_ENTRY_LIMIT
    );
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fuzzy_picker_preview_prefers_unsaved_text_and_ignores_stale_scan_events() {
    let directory = temporary("picker-preview");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.txt");
    fs::write(&path, "disk text\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.buffers[0].apply(&Transaction::insert(0, "unsaved "));
    let mut picker = FilePicker::new(
        9,
        directory.clone(),
        crate::file_picker::ScanScope::ignoring(&directory),
    );
    picker.add_paths(vec![ScanEntry::file(path.clone())]);
    picker.finish(0, false);
    app.picker = Some(picker);
    app.refresh_file_picker_preview();

    let FilePreview::Text(lines) = app.picker.as_ref().unwrap().preview.as_ref().unwrap() else {
        panic!("text preview expected");
    };
    assert_eq!(lines[0], "unsaved disk text");

    deliver(
        &mut app,
        FilePickerEvent::Files {
            scan_id: 8,
            paths: vec![ScanEntry::file(directory.join("stale.txt"))],
        },
    );
    assert_eq!(app.picker.as_ref().unwrap().entries.len(), 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn background_picker_query_is_visible_before_the_ranker_answers() {
    let root = temporary("background-picker-query");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_picker().unwrap();

    press(&mut app, 'n');

    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.query, "n");
    assert_eq!(picker.query_revision, 1);
    assert!(picker.ranking);
    assert!(
        app.overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
            .unwrap()
            .actions
            .iter()
            .all(|action| action.key_hint != "Enter")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn finished_background_scan_stays_pending_until_the_final_rank_arrives() {
    let root = temporary("background-picker-final-rank-pending");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("only.rs"), "one candidate\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, mut events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_picker_at(
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
        FilePickerKind::Files,
    )
    .unwrap();
    let scan_id = app.picker.as_ref().unwrap().scan_id;

    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let event = events.recv().await.unwrap();
                    let finished = matches!(
                        event,
                        FilePickerEvent::Finished {
                            scan_id: event_scan,
                            ..
                        } if event_scan == scan_id
                    );
                    deliver(&mut app, event);
                    if finished {
                        break;
                    }
                }

                let picker = app.picker.as_ref().unwrap();
                assert!(!picker.loading);
                assert!(
                    picker.ranking,
                    "the final sub-batch rank must keep the picker pending"
                );
                assert!(picker.selected_target().is_none(), "Enter stays disabled");

                loop {
                    deliver(&mut app, events.recv().await.unwrap());
                    if !app.picker.as_ref().unwrap().ranking {
                        break;
                    }
                }
            })
            .await
            .expect("the final rank should arrive");
        });

    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.entries.len(), 1);
    assert_eq!(picker.matches.len(), 1);
    assert!(picker.selected_target().is_some());
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_rank_already_in_flight_when_a_scan_finishes_leaves_the_rows_pending() {
    let root = temporary("background-picker-in-flight-rank");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("only.rs");
    fs::write(&path, "one candidate\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    let mut picker = FilePicker::new(
        9,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.add_paths(vec![ScanEntry::file(path)]);
    let complete = picker.matches.clone();
    app.picker = Some(picker);

    deliver(
        &mut app,
        FilePickerEvent::Finished {
            scan_id: 9,
            skipped: 0,
            limited: false,
        },
    );
    assert!(app.picker.as_ref().unwrap().ranking);

    // A publish the ranker had already sent when the scan finished answers
    // only the candidates it held then, so it cannot release the rows.
    deliver(
        &mut app,
        FilePickerEvent::Ranked {
            scan_id: 9,
            query_revision: 0,
            matches: Vec::new(),
            match_positions: vec![None],
            finder_matches: None,
            finder_revision: None,
            finder_positions: HashMap::new(),
            flushed: false,
        },
    );
    let picker = app.picker.as_ref().unwrap();
    assert!(
        picker.ranking,
        "a rank published before the flush must keep the picker pending"
    );
    assert!(picker.selected_target().is_none(), "Enter stays disabled");

    deliver(
        &mut app,
        FilePickerEvent::Ranked {
            scan_id: 9,
            query_revision: 0,
            matches: complete.clone(),
            match_positions: vec![Some(0)],
            finder_matches: None,
            finder_revision: None,
            finder_positions: HashMap::new(),
            flushed: true,
        },
    );
    let picker = app.picker.as_ref().unwrap();
    assert!(!picker.ranking);
    assert_eq!(picker.matches, complete);
    assert!(picker.selected_target().is_some());
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn background_picker_rejects_a_stale_query_revision() {
    let root = temporary("background-picker-stale-rank");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("alpha.rs");
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    let mut picker = FilePicker::new(
        9,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.add_paths(vec![ScanEntry::file(path)]);
    picker.insert_query_unranked('a');
    let visible = picker.matches.clone();
    app.picker = Some(picker);

    deliver(
        &mut app,
        FilePickerEvent::Ranked {
            scan_id: 9,
            query_revision: 0,
            matches: Vec::new(),
            match_positions: vec![None],
            finder_matches: None,
            finder_revision: None,
            finder_positions: HashMap::new(),
            flushed: false,
        },
    );

    assert_eq!(app.picker.as_ref().unwrap().matches, visible);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn background_picker_rejects_a_stale_resource_revision() {
    let root = temporary("background-picker-stale-resource");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    let mut picker = FilePicker::new(
        4,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.finish(0, false);
    picker.ranking = true;
    app.picker = Some(picker);
    let mut finder = ResourceFinder::new(FinderMode::Names);
    let item = |label: &str| {
        ResourceItem::new(
            label,
            "",
            ResourceTarget::Buffer(0),
            ResourceKind::Buffer,
            Vec::<String>::new(),
        )
    };
    finder.replace_items_unmerged(vec![item("old")], "");
    let stale_revision = finder.file_rank_revision();
    let stale = crate::finder::FinderMatch {
        source: FinderMatchSource::Resource(0),
        emphasis: Vec::new(),
        detail_emphasis: Vec::new(),
        score: 0,
        type_boost: false,
    };
    finder.matches = vec![stale.clone()];
    finder.append_items_unmerged([item("current")], "");
    app.finder = Some(finder);

    deliver(
        &mut app,
        FilePickerEvent::Ranked {
            scan_id: 4,
            query_revision: 0,
            matches: Vec::new(),
            match_positions: Vec::new(),
            finder_matches: Some(vec![stale.clone()]),
            finder_revision: Some(stale_revision),
            finder_positions: [(stale.source, 0)].into_iter().collect(),
            flushed: false,
        },
    );

    assert_eq!(app.finder.as_ref().unwrap().matches, vec![stale]);
    assert!(app.picker.as_ref().unwrap().ranking);
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap())
            .is_none(),
        "old rows must remain inert until both halves have the current revision"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn background_file_scan_rank_and_preview_converge() {
    let root = temporary("background-picker-converges");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("alpha.rs"), "alpha preview\n").unwrap();
    fs::write(root.join("beta.rs"), "beta preview\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, mut events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_picker().unwrap();

    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    while app.resource_finder_scan_pending() {
                        app.advance_resource_finder_scan();
                    }
                    let event = events.recv().await.unwrap();
                    deliver(&mut app, event);
                    let settled = app.picker.as_ref().is_some_and(|picker| {
                        !picker.loading
                            && !picker.ranking
                            && picker.entries.len() == 2
                            && picker.matches.len() == 2
                            && picker.preview.is_some()
                    });
                    if settled {
                        break;
                    }
                }
            })
            .await
            .expect("background finder work should settle");
        });

    let finder = app.finder.as_ref().unwrap();
    assert_eq!(finder.matches.len(), 3);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn background_content_query_converges_after_rapid_typing() {
    let root = temporary("background-content-converges");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("alpha.rs"), "ordinary\nthe needle is here\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, mut events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_grep().unwrap();
    for character in "needle".chars() {
        press(&mut app, character);
    }
    assert_eq!(app.picker.as_ref().unwrap().query, "needle");

    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    while app.resource_finder_scan_pending() {
                        app.advance_resource_finder_scan();
                    }
                    let event = events.recv().await.unwrap();
                    deliver(&mut app, event);
                    let settled = app.picker.as_ref().is_some_and(|picker| {
                        !picker.loading
                            && !picker.ranking
                            && picker.query == "needle"
                            && picker.matches.len() == 1
                            && picker.preview.is_some()
                    });
                    if settled {
                        break;
                    }
                }
            })
            .await
            .expect("the latest content query should settle");
        });

    assert_eq!(
        app.picker.as_ref().unwrap().selected_entry().unwrap().text,
        Some("the needle is here")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_content_scan_reaches_text_after_very_long_indentation() {
    let root = temporary("finder-long-indentation");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    seed(&mut app, &format!("{}needle\n", " ".repeat(4_096)));
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_grep().unwrap();
    type_text(&mut app, "needle");
    settle_finder(&mut app);

    let match_item = app
        .finder
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|item| item.detail == "needle")
        .expect("authoritative live text after long indentation must match");
    assert!(matches!(
        match_item.target,
        ResourceTarget::BufferLocation { column: 4_096, .. }
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn attached_terminal_refill_makes_remapped_rows_inert_before_rank_response() {
    let root = temporary("finder-terminal-refill-readiness");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.terminals.apply(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"first needle\r\nsecond needle\r\n".to_vec(),
    });
    let rows = (0..app.terminals.get(terminal).unwrap().plain_line_count())
        .filter_map(|row| {
            let (line_id, line) = app
                .terminals
                .get(terminal)
                .unwrap()
                .plain_line_with_id(row)?;
            line.contains("needle").then_some((line_id, row, line))
        })
        .collect::<Vec<_>>();
    assert!(rows.len() >= 2);

    let mut picker = FilePicker::grep(
        77,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.insert_query_text("needle");
    picker.finish(0, false);
    let mut finder = ResourceFinder::new(FinderMode::Contents);
    finder.begin_content_scan(&picker, "needle", std::iter::empty());
    finder.append_items(
        rows.iter()
            .enumerate()
            .map(|(index, (line_id, row, line))| {
                ResourceItem::content(
                    format!("terminal:{}", row + 1),
                    line.clone(),
                    ResourceTarget::TerminalLocation {
                        terminal,
                        line_id: *line_id,
                        column: 0,
                    },
                    ResourceKind::Terminal,
                )
                .with_path(root.join(format!("terminal-{index}")))
            }),
        &picker,
        "needle",
    );
    finder.finish_content_scan(false);
    finder.down();
    assert!(finder.selected_target(&picker).is_some());
    app.picker = Some(picker);
    app.finder = Some(finder);
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);

    app.start_terminal_content_refresh([terminal].into_iter().collect());

    assert!(app.picker.as_ref().unwrap().ranking);
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap())
            .is_none(),
        "Enter must not resolve a stale resource index while refill remaps items"
    );
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn attached_terminal_refill_moves_its_large_index_and_advances_it_in_slices() {
    let root = temporary("finder-terminal-refill-sliced");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.terminals.apply(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"needle\r\n".to_vec(),
    });
    let line_id = app
        .terminals
        .get(terminal)
        .unwrap()
        .plain_line_with_id(0)
        .unwrap()
        .0;
    let mut picker = FilePicker::grep(
        78,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.insert_query_text("needle");
    picker.finish(0, false);
    let mut finder = ResourceFinder::new(FinderMode::Contents);
    finder.begin_content_scan_unmerged("needle", Arc::new(HashSet::new()));
    finder.append_content_items_unmerged(
        (0..CONTENT_ENTRY_LIMIT).map(|row| {
            ResourceItem::content(
                format!("terminal:{row}"),
                "needle",
                ResourceTarget::TerminalLocation {
                    terminal,
                    line_id,
                    column: 0,
                },
                ResourceKind::Terminal,
            )
        }),
        "needle",
    );
    let storage = finder.terminal_content_index_storage(terminal);
    assert_eq!(storage.1, CONTENT_ENTRY_LIMIT);
    app.picker = Some(picker);
    app.finder = Some(finder);
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);

    app.start_terminal_content_refresh([terminal].into_iter().collect());

    let scan = app.finder_content_scan.as_ref().unwrap();
    assert_eq!(scan.retirements[0].items.as_ptr() as usize, storage.0);
    assert_eq!(scan.retirements[0].items.len(), CONTENT_ENTRY_LIMIT);
    app.advance_resource_finder_scan();
    assert_eq!(
        app.finder_content_scan.as_ref().unwrap().retirements[0].item,
        128
    );
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn attached_terminal_shift_during_retirement_forces_a_full_repair_pass() {
    let root = temporary("finder-terminal-refill-shift");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    let initial = (0..5_100)
        .map(|row| format!("stable-needle-{row}\r\n"))
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: initial.into_bytes(),
    });
    let (scanner, mut events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_grep().unwrap();
    type_text(&mut app, "stable-needle");
    while app.resource_finder_scan_pending() {
        app.advance_resource_finder_scan();
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while app.picker.as_ref().unwrap().ranking {
                deliver(&mut app, events.recv().await.unwrap());
            }
        })
        .await
        .expect("the initial terminal ranking should settle");
    });
    let target_line_id = app
        .terminals
        .get(terminal)
        .unwrap()
        .plain_line_with_id(130)
        .unwrap()
        .0;
    assert!(app.finder.as_ref().unwrap().items.iter().any(|item| {
        matches!(
            item.target,
            ResourceTarget::TerminalLocation { line_id, .. } if line_id == target_line_id
        )
    }));
    let selected = app
        .finder
        .as_ref()
        .unwrap()
        .matches
        .iter()
        .position(|found| match found.source {
            FinderMatchSource::Resource(item) => matches!(
                app.finder.as_ref().unwrap().items[item].target,
                ResourceTarget::TerminalLocation { line_id, .. } if line_id == target_line_id
            ),
            FinderMatchSource::File(_) => false,
        })
        .unwrap();
    app.finder.as_mut().unwrap().first();
    for _ in 0..selected {
        app.finder.as_mut().unwrap().down();
    }
    let claimed = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap())
        .unwrap();

    app.start_terminal_content_refresh([terminal].into_iter().collect());
    app.advance_resource_finder_scan();
    let shifted = (0..12)
        .map(|row| format!("stable-needle-new-{row}\r\n"))
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: shifted.into_bytes(),
    });
    let mut saw_full_repair = false;
    while app.resource_finder_scan_pending() {
        app.advance_resource_finder_scan();
        saw_full_repair |= app.finder_content_scan.as_ref().is_some_and(|scan| {
            scan.refilling
                && scan
                    .sources
                    .first()
                    .is_some_and(|source| source.first_row() == 0)
        });
        assert!(
            app.finder_content_scan.is_none() || app.finder_scan_refills(),
            "the intermediate false drop must not become a drawable frame"
        );
    }
    assert!(saw_full_repair);
    assert!(!app.finder_dirty_terminals.contains(&terminal));
    assert!(app.finder_terminal_marks.contains_key(&terminal));

    assert!(
        app.finder.as_ref().unwrap().items.iter().any(|item| {
            matches!(
                item.target,
                ResourceTarget::TerminalLocation { line_id, .. } if line_id == target_line_id
            )
        }),
        "the still-retained match must be restored by the full repair pass"
    );
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while app.picker.as_ref().unwrap().ranking {
                deliver(&mut app, events.recv().await.unwrap());
            }
        })
        .await
        .expect("the repaired terminal ranking should settle");
    });
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(claimed)
    );

    let evicted_line_id = app
        .terminals
        .get(terminal)
        .unwrap()
        .plain_line_with_id(130)
        .unwrap()
        .0;
    let selected = app
        .finder
        .as_ref()
        .unwrap()
        .matches
        .iter()
        .position(|found| match found.source {
            FinderMatchSource::Resource(item) => matches!(
                app.finder.as_ref().unwrap().items[item].target,
                ResourceTarget::TerminalLocation { line_id, .. } if line_id == evicted_line_id
            ),
            FinderMatchSource::File(_) => false,
        })
        .unwrap();
    app.finder.as_mut().unwrap().first();
    for _ in 0..selected {
        app.finder.as_mut().unwrap().down();
    }
    let evicted_claim = app
        .finder
        .as_ref()
        .unwrap()
        .selected_target(app.picker.as_ref().unwrap())
        .unwrap();
    app.start_terminal_content_refresh([terminal].into_iter().collect());
    app.advance_resource_finder_scan();
    let replacement = (0..5_100)
        .map(|row| format!("stable-needle-replacement-{row}\r\n"))
        .collect::<String>();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: replacement.into_bytes(),
    });
    while app.resource_finder_scan_pending() {
        app.advance_resource_finder_scan();
    }
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while app.picker.as_ref().unwrap().ranking {
                deliver(&mut app, events.recv().await.unwrap());
            }
        })
        .await
        .expect("the eviction repair ranking should settle");
    });
    let finder = app.finder.as_ref().unwrap();
    assert!(!finder.selection_is_user_owned());
    assert_ne!(
        finder.selected_target(app.picker.as_ref().unwrap()),
        Some(evicted_claim),
        "a recycled resource slot must not inherit the evicted stable-line claim"
    );
    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacing_a_large_live_content_cursor_drops_it_on_the_rank_worker() {
    let root = temporary("finder-content-cursor-cancel");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.terminals.apply(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"needle\r\n".to_vec(),
    });
    let line_id = app
        .terminals
        .get(terminal)
        .unwrap()
        .plain_line_with_id(0)
        .unwrap()
        .0;
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_grep().unwrap();
    let scan = app.finder_content_scan.as_mut().unwrap();
    scan.retirements.push(TerminalContentRetirement {
        terminal,
        items: vec![(line_id, 0); CONTENT_ENTRY_LIMIT],
        item: 0,
        retained_row: 0,
        retained_until: 1,
    });
    let editor_thread = std::thread::current().id();
    let (dropped, observed) = std::sync::mpsc::channel();
    scan.drop_observer = Some(dropped);

    press(&mut app, 'n');

    let worker_thread = observed
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the replaced scan should be destroyed promptly");
    assert_ne!(worker_thread, editor_thread);
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn switching_a_large_content_finder_to_names_retires_ownership_on_the_rank_worker() {
    let root = temporary("finder-mode-switch-retirement");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_grep().unwrap();
    app.finder.as_mut().unwrap().append_content_items_unmerged(
        (0..CONTENT_ENTRY_LIMIT).map(|row| {
            ResourceItem::content(
                format!("scratch:{row}"),
                "content",
                ResourceTarget::BufferLocation {
                    buffer: 0,
                    row,
                    column: 0,
                },
                ResourceKind::Buffer,
            )
        }),
        "",
    );
    let scan = app.finder_content_scan.as_mut().unwrap();
    scan.retirements = (0..CONTENT_ENTRY_LIMIT)
        .map(|_| TerminalContentRetirement {
            terminal: TerminalId::from_raw(99),
            items: Vec::new(),
            item: 0,
            retained_row: 0,
            retained_until: 0,
        })
        .collect();
    let editor_thread = std::thread::current().id();
    let (dropped, observed) = std::sync::mpsc::channel();
    scan.drop_observer = Some(dropped);

    app.toggle_finder_mode();

    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Names);
    assert!(app.finder.as_ref().unwrap().items.len() < CONTENT_ENTRY_LIMIT);
    let worker_thread = observed
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("mode replacement should retire the content cursor promptly");
    assert_ne!(worker_thread, editor_thread);
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reopening_over_a_large_finder_retires_ownership_on_the_rank_worker() {
    let root = temporary("finder-reopen-retirement");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_grep().unwrap();
    app.finder.as_mut().unwrap().append_content_items_unmerged(
        (0..CONTENT_ENTRY_LIMIT).map(|row| {
            ResourceItem::content(
                format!("scratch:{row}"),
                "content",
                ResourceTarget::BufferLocation {
                    buffer: 0,
                    row,
                    column: 0,
                },
                ResourceKind::Buffer,
            )
        }),
        "",
    );
    let scan = app.finder_content_scan.as_mut().unwrap();
    scan.retirements = (0..CONTENT_ENTRY_LIMIT)
        .map(|_| TerminalContentRetirement {
            terminal: TerminalId::from_raw(100),
            items: Vec::new(),
            item: 0,
            retained_row: 0,
            retained_until: 0,
        })
        .collect();
    let editor_thread = std::thread::current().id();
    let (dropped, observed) = std::sync::mpsc::channel();
    scan.drop_observer = Some(dropped);

    app.open_project_picker().unwrap();

    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Names);
    let worker_thread = observed
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("picker replacement should retire the old finder promptly");
    assert_ne!(worker_thread, editor_thread);
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn attached_finder_switches_from_content_back_to_names_after_ranker_reset() {
    let root = temporary("background-finder-mode-switch");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("alpha.rs"), "alpha needle\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, mut events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_picker().unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let settle = |app: &mut App,
                  events: &mut tokio::sync::mpsc::Receiver<FilePickerEvent>,
                  expected: FinderMode| {
        runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    while app.resource_finder_scan_pending() {
                        app.advance_resource_finder_scan();
                    }
                    if app
                        .finder
                        .as_ref()
                        .is_some_and(|finder| finder.mode == expected && !finder.loading)
                        && app.picker.as_ref().is_some_and(|picker| {
                            !picker.loading && !picker.ranking && !picker.matches.is_empty()
                        })
                    {
                        break;
                    }
                    let event = events.recv().await.unwrap();
                    deliver(app, event);
                }
            })
            .await
            .expect("finder mode should settle");
        });
    };

    settle(&mut app, &mut events, FinderMode::Names);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    settle(&mut app, &mut events, FinderMode::Contents);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    settle(&mut app, &mut events, FinderMode::Names);
    assert_eq!(
        app.picker
            .as_ref()
            .unwrap()
            .selected_entry()
            .unwrap()
            .relative,
        "alpha.rs"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn attached_finder_snapshot_materializes_only_its_selected_window() {
    let mut app = App::new(Config::default(), None).unwrap();
    let root = PathBuf::from("/project");
    let mut picker = FilePicker::new(
        1,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.add_paths(
        (0..1_500)
            .map(|index| ScanEntry::file(root.join(format!("file-{index:04}.rs"))))
            .collect(),
    );
    picker.finish(0, false);
    let mut finder = ResourceFinder::new(FinderMode::Names);
    finder.replace_items(Vec::new(), &picker, "");
    finder.last();
    app.picker = Some(picker);
    app.finder = Some(finder);

    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
        .unwrap();
    assert_eq!(snapshot.rows.len(), 512);
    assert_eq!(snapshot.total_rows, 1_500);
    assert_eq!(snapshot.omitted_rows, 988);
    assert_eq!(snapshot.row_offset, 988);
    assert_eq!(snapshot.selected, Some(511));
    assert_eq!(snapshot.scroll_anchor, Some(1_499));
}

#[test]
fn fuzzy_grep_searches_contents_at_both_roots_and_enter_jumps_to_the_match() {
    let root = temporary("fuzzy-grep");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(root.join("outside.txt"), "project needle\n").unwrap();
    let active = nested.join("active.txt");
    fs::write(&active, "first\nlocal needle\nlast\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(active.clone()).unwrap();
    let active_buffer = app.active().buffer;
    app.buffers[active_buffer].apply(&Transaction::insert(6, "unsaved "));

    app.execute_command("fuzzy-grep-directory").unwrap();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, FilePickerKind::Contents);
    assert_eq!(picker.root, nested.canonicalize().unwrap());
    assert!(picker.views().all(|entry| entry.path == active));
    assert!(
        picker
            .views()
            .any(|entry| entry.text == Some("unsaved local needle"))
    );
    assert!(
        !picker
            .views()
            .any(|entry| entry.text == Some("local needle")),
        "open buffer text must replace stale disk candidates"
    );
    type_text(&mut app, "usvd");
    assert_eq!(app.picker.as_ref().unwrap().matches.len(), 1);
    let FilePreview::Snippet(snippet) = app.picker.as_ref().unwrap().preview.as_ref().unwrap()
    else {
        panic!("fuzzy grep should preview the selected matching line");
    };
    assert_eq!(snippet.focus_row, 1);
    assert_eq!(snippet.lines[1], "unsaved local needle");
    assert!(!snippet.emphasis.is_empty());
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
        .unwrap();
    assert!(matches!(
        overlay.preview,
        Some(crate::snapshot::OverlayPreview::Snippet {
            start_row: 0,
            focus_row: 1,
            ref emphasis,
            ..
        }) if !emphasis.is_empty()
    ));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.picker.is_none());
    assert_eq!(cursor(&app), Position { row: 1, col: 0 });

    app.execute_command("fuzzy-grep").unwrap();
    assert_eq!(
        app.picker.as_ref().unwrap().root,
        root.canonicalize().unwrap()
    );
    type_text(&mut app, "pndl");
    assert_eq!(
        app.picker.as_ref().unwrap().selected_path(),
        Some(root.join("outside.txt").as_path())
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fuzzy_grep_reaches_a_match_the_candidate_limit_used_to_hide() {
    // Filler on both sides of the needle puts more than
    // `CONTENT_ENTRY_LIMIT` lines ahead of it whichever way the walk goes,
    // so the unfiltered scan behind an empty query cannot reach it. Typing
    // is what makes it reachable: the scan restarts under the query and
    // spends its budget on matches rather than on the first lines it read.
    let root = temporary("fuzzy-grep-past-the-limit");
    fs::create_dir_all(&root).unwrap();
    // Each side is sized from the budget, so raising the budget keeps the
    // needle on the far side of a truncated walk rather than quietly
    // turning this into a test that would pass without the fix.
    let filler = (0..CONTENT_ENTRY_LIMIT + 1_000)
        .map(|line| format!("let value_{line} = compute(input);\n"))
        .collect::<String>();
    fs::write(root.join("a_filler.rs"), &filler).unwrap();
    fs::write(root.join("m_needle.rs"), "call_the_marked_thing();\n").unwrap();
    fs::write(root.join("z_filler.rs"), &filler).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();

    app.execute_command("fuzzy-grep").unwrap();
    let picker = app.picker.as_ref().unwrap();
    assert!(
        picker.limited,
        "an unfiltered scan of twice the budget fills the candidate budget"
    );
    assert!(
        !picker
            .views()
            .any(|entry| entry.text == Some("call_the_marked_thing();")),
        "the truncated scan is expected to stop short of the needle"
    );

    type_text(&mut app, "markedthing");
    // The re-scan a truncated corpus owes this query waits for the query to
    // settle, which here is the next turn of the event loop.
    pacing_tick(&mut app);
    let picker = app.picker.as_ref().unwrap();
    assert!(
        !picker.limited,
        "one match for the query is nowhere near the candidate budget"
    );
    assert_eq!(
        picker.selected_path(),
        Some(root.canonicalize().unwrap().join("m_needle.rs").as_path())
    );

    fs::remove_dir_all(root).unwrap();
}

/// The emphasized text of a content preview, which is what the finder shows
/// highlighted in its preview column.
fn previewed_match(preview: &crate::file_picker::FilePreview) -> String {
    let crate::file_picker::FilePreview::Snippet(snippet) = preview else {
        panic!("a content match previews a snippet, not {preview:?}");
    };
    let focused = snippet
        .lines
        .get(snippet.focus_row - snippet.start_row)
        .expect("the focused row is inside the snippet");
    snippet
        .emphasis
        .iter()
        .map(|position| focused.chars().nth(*position).unwrap())
        .collect()
}

#[test]
fn buffer_content_preview_highlights_the_matched_text() {
    let root = temporary("project-finder-buffer-preview-emphasis");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("buffer-new").unwrap();
    let scratch = app.active().buffer;
    let matching_line = format!("{}{}needle", " ".repeat(20), "x".repeat(500));
    let contents = format!("first\n{matching_line}\nlast");
    app.buffers[scratch].apply(&Transaction::insert(0, &contents));
    app.open_project_grep().unwrap();
    type_text(&mut app, "needle");
    settle_finder(&mut app);
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(FinderTarget::Resource(ResourceTarget::BufferLocation {
            buffer: scratch,
            row: 1,
            column: 20,
        }))
    );
    let preview = app.finder.as_ref().unwrap().selected_preview().unwrap();
    assert_eq!(previewed_match(preview), "needle");
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn terminal_content_preview_highlights_the_matched_text() {
    let root = temporary("project-finder-terminal-preview-emphasis");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"quiet row\r\n   indented needle here\r\nquiet row\r\n".to_vec(),
    });
    app.open_project_grep().unwrap();
    type_text(&mut app, "needle");
    settle_finder(&mut app);
    assert!(
        matches!(
            app.finder
                .as_ref()
                .unwrap()
                .selected_target(app.picker.as_ref().unwrap()),
            Some(FinderTarget::Resource(ResourceTarget::TerminalLocation {
                terminal: selected,
                column: 3,
                ..
            })) if selected == terminal
        ),
        "the indented terminal row is the selected match"
    );
    let preview = app.finder.as_ref().unwrap().selected_preview().unwrap();
    assert_eq!(previewed_match(preview), "needle");
    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn truncated_content_rescan_reranks_under_the_query_it_restarts_for() {
    let root = temporary("background-content-rescan-reranks");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("alpha.rs"), "one\nneedle here\nthree\n").unwrap();
    fs::write(root.join("beta.rs"), "needle again\n").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, mut events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_grep().unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let settle =
        |app: &mut App, events: &mut tokio::sync::mpsc::Receiver<FilePickerEvent>, what: &str| {
            runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        while app.resource_finder_scan_pending() {
                            app.advance_resource_finder_scan();
                        }
                        let settled = app.finder.as_ref().zip(app.picker.as_ref()).is_some_and(
                            |(finder, picker)| {
                                !finder.loading
                                    && !picker.loading
                                    && !picker.ranking
                                    && !picker.content_rescan_needed()
                                    && !picker.matches.is_empty()
                                    && picker.preview.is_some()
                            },
                        );
                        if settled {
                            break;
                        }
                        pacing_tick(app);
                        deliver(app, events.recv().await.unwrap());
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("{what}"));
            });
        };

    settle(
        &mut app,
        &mut events,
        "the initial content scan should settle",
    );
    type_text(&mut app, "needle");
    settle(&mut app, &mut events, "the typed query should settle");

    // The scan that collected these entries ran under the empty query and
    // stopped at the entry ceiling, so the entries on hand cannot answer
    // "needle" and the picker restarts the scan the moment it learns that.
    let scan_id = app.picker.as_ref().unwrap().scan_id;
    deliver(
        &mut app,
        FilePickerEvent::Finished {
            scan_id,
            skipped: 0,
            limited: true,
        },
    );
    pacing_tick(&mut app);
    assert_ne!(
        app.picker.as_ref().unwrap().scan_id,
        scan_id,
        "a truncated scan restarts under the current query"
    );
    settle(&mut app, &mut events, "the restarted scan should settle");

    let picker = app.picker.as_ref().unwrap();
    let finder = app.finder.as_ref().unwrap();
    assert!(
        !picker.matches.is_empty(),
        "the restarted scan must be ranked for the query it restarted under"
    );
    assert!(
        finder.matches.iter().all(|found| match found.source {
            FinderMatchSource::File(entry) => picker.view(entry).is_some(),
            FinderMatchSource::Resource(item) => finder.items.get(item).is_some(),
        }),
        "every finder row must resolve against the rebuilt entry table"
    );
    assert!(
        picker.preview.is_some(),
        "the selected file match must still be previewable"
    );
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_retained_content_row_keeps_its_preview_while_the_new_scan_ranks() {
    let root = temporary("retained-content-preview");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("note.txt");
    fs::write(&path, "before\nneedle preview\nafter\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let mut picker = FilePicker::grep(
        9,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.add_content(vec![crate::file_picker::FileHits {
        path: path.clone(),
        lines: vec![crate::file_picker::LineHit {
            row: 1,
            column: 0,
            text: "needle preview".to_owned(),
        }],
    }]);
    picker.insert_query_text("needle");
    let mut finder = ResourceFinder::new(FinderMode::Contents);
    finder.merge_files(&picker, "needle");

    // A new content walk keeps the old corpus for exactly the rows still on
    // screen. The picker has no matches in its new table yet, but the finder
    // row remains fully resolvable through its recorded scan identity.
    let _discarded = picker.restart_content_scan(10);
    assert!(picker.matches.is_empty());
    assert!(finder.file_entry(&picker, 0).is_some());
    app.picker = Some(picker);
    app.finder = Some(finder);

    app.refresh_finder_preview();

    let preview = app
        .picker
        .as_ref()
        .unwrap()
        .preview
        .as_ref()
        .expect("the retained row should keep a preview while its replacement ranks");
    assert_eq!(previewed_match(preview), "needle");
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rapid_finder_mode_switches_and_retyped_query_restore_the_file_preview() {
    let root = temporary("rapid-finder-mode-preview");
    fs::create_dir_all(&root).unwrap();
    for index in 0..512 {
        fs::write(
            root.join(format!("note-{index:04}.txt")),
            format!("heading {index}\nneedle preview {index}\ntrailing\n"),
        )
        .unwrap();
    }
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, mut events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_project_picker().unwrap();

    type_text(&mut app, "needle");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    for _ in 0.."needle".len() {
        key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    }
    type_text(&mut app, "needle");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    while app.resource_finder_scan_pending() {
                        app.advance_resource_finder_scan();
                    }
                    pacing_tick(&mut app);
                    let settled = app.finder.as_ref().zip(app.picker.as_ref()).is_some_and(
                        |(finder, picker)| {
                            finder.mode == FinderMode::Contents
                                && !finder.loading
                                && !picker.loading
                                && !picker.ranking
                                && !picker.content_rescan_needed()
                                && !finder.matches.is_empty()
                                && picker.preview.is_some()
                        },
                    );
                    if settled {
                        break;
                    }
                    deliver(&mut app, events.recv().await.unwrap());
                }
            })
            .await
            .expect("the final content mode and its preview should settle");
        });

    assert!(app.picker.as_ref().unwrap().preview.is_some());
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn output_that_leaves_a_terminal_item_unchanged_does_not_move_the_name_list() {
    let root = temporary("project-finder-quiet-terminal-name-mode");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_terminal_at(Some("/bin/cat".to_owned()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.open_project_picker().unwrap();
    let before = app.finder.as_ref().unwrap().matches.clone();

    // Output is what marks a terminal dirty, but a name-mode item describes
    // the session rather than its output. A child that writes without saying
    // anything new about itself leaves the list exactly as it was.
    for row in 0..64 {
        app.apply_terminal_output(TerminalOutput::Bytes {
            id: terminal,
            bytes: format!("busy row {row}\r\n").into_bytes(),
        });
    }
    assert!(app.finder_terminals_dirty());
    assert!(
        !app.refresh_finder_terminals(),
        "a repeat of the item already held is not a change worth a frame"
    );
    assert!(!app.finder_terminals_dirty());
    assert!(!app.picker.as_ref().unwrap().ranking);
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .matches
            .iter()
            .map(|found| found.source)
            .collect::<Vec<_>>(),
        before.iter().map(|found| found.source).collect::<Vec<_>>(),
        "no row moved"
    );

    // A title the reader can search for is a change, and is taken.
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b]2;hot-title\x07".to_vec(),
    });
    assert!(app.refresh_finder_terminals());
    type_text(&mut app, "hot-title");
    assert_eq!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_target(app.picker.as_ref().unwrap()),
        Some(FinderTarget::Resource(ResourceTarget::Terminal(terminal)))
    );
    app.close_file_picker();
    app.close_terminal_id(terminal);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_header_count_changes_once_a_second_and_catches_up_after_the_work_stops() {
    let root = temporary("picker-header-count-pacing");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let geometry = FrameGeometry {
        screen: Rect {
            width: 80,
            height: 24,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let candidates = |app: &App| {
        app.picker.as_ref().unwrap().entries.len() + app.finder.as_ref().unwrap().items.len()
    };
    app.open_project_picker().unwrap();
    let scan_id = app.picker.as_ref().unwrap().scan_id;
    app.prepare_view(geometry);
    let held = app.picker_progress_counts();
    assert_eq!(held.1, candidates(&app));

    // A scan the header is watching delivers far more states than a header
    // can be read at, so the rest of the second the last one was shown in is
    // not shown at all.
    app.picker.as_mut().unwrap().loading = true;
    deliver(
        &mut app,
        FilePickerEvent::Files {
            scan_id,
            paths: vec![ScanEntry::file(root.join("alpha.rs"))],
        },
    );
    app.prepare_view(geometry);
    assert_ne!(candidates(&app), held.1, "the corpus did move");
    assert_eq!(
        app.picker_progress_counts(),
        held,
        "a header does not follow the scanner within the same second"
    );

    // A second on, it catches up.
    app.picker_progress.as_mut().unwrap().published -= Duration::from_secs(1);
    app.prepare_view(geometry);
    let caught_up = app.picker_progress_counts();
    assert_ne!(caught_up, held);
    assert_eq!(caught_up.1, candidates(&app));

    // Work that stops inside the interval does not release it: the reader
    // asked for a header that holds still, and one that jumps the moment a
    // scan happens to end is the flicker they asked to be rid of.
    deliver(
        &mut app,
        FilePickerEvent::Files {
            scan_id,
            paths: vec![ScanEntry::file(root.join("beta.rs"))],
        },
    );
    deliver(
        &mut app,
        FilePickerEvent::Finished {
            scan_id,
            skipped: 0,
            limited: false,
        },
    );
    app.prepare_view(geometry);
    assert_eq!(
        app.picker_progress_counts(),
        caught_up,
        "still inside the second the catch-up started"
    );

    // Nothing else would come back for those last counts, so the loop is
    // told how long it has to wait for them.
    let owed = app
        .picker_pacing_delay(Instant::now())
        .expect("a header behind the work owes the reader a catch-up");
    assert!(owed <= PICKER_PROGRESS_INTERVAL);
    app.picker_progress.as_mut().unwrap().published -= Duration::from_secs(1);
    app.prepare_view(geometry);
    assert_eq!(
        app.picker_progress_counts().1,
        candidates(&app),
        "the counts the scan finished on are exact once the interval is out"
    );
    assert_eq!(
        app.picker_pacing_delay(Instant::now()),
        None,
        "a header that has caught up leaves the loop nothing to wake for"
    );
    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_ranked_answer_waits_for_the_rows_under_the_reader_and_a_list_key_takes_it() {
    let root = temporary("picker-row-pacing");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    let mut picker = FilePicker::new(
        9,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.add_paths(vec![
        ScanEntry::file(root.join("alpha.rs")),
        ScanEntry::file(root.join("beta.rs")),
    ]);
    let both = picker.matches.clone();
    app.picker = Some(picker);
    let answer = |matches: Vec<crate::file_picker::FuzzyMatch>,
                  match_positions: Vec<Option<usize>>| {
        FilePickerEvent::Ranked {
            scan_id: 9,
            query_revision: 0,
            matches,
            match_positions,
            finder_matches: None,
            finder_revision: None,
            finder_positions: HashMap::new(),
            flushed: false,
        }
    };

    // An empty list has no rows to hold still, so the first answer is shown
    // as it lands and starts the interval.
    app.apply_file_picker_event(answer(both.clone(), vec![Some(0), Some(1)]));
    assert_eq!(app.picker.as_ref().unwrap().matches.len(), 2);

    // The next answer inside that interval waits. Rows are what the reader
    // is choosing from, so they are not turned over underneath them.
    app.apply_file_picker_event(answer(vec![both[0].clone()], vec![Some(0), None]));
    assert_eq!(
        app.picker.as_ref().unwrap().matches.len(),
        2,
        "the rows the reader is looking at hold still"
    );
    let owed = app
        .picker_pacing_delay(Instant::now())
        .expect("a held answer owes the reader a frame");
    assert!(owed <= PICKER_LIST_INTERVAL);

    // A key that reads the list reads the newest answer, not the paced one.
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().matches.len(), 1);
    assert!(app.held_rank.is_none());

    // Typing is the one picker key that does not read the list, so it leaves
    // a held answer where it is — and an answer to a query the reader has
    // already moved on from is discarded rather than shown.
    app.apply_file_picker_event(answer(both.clone(), vec![Some(0), Some(1)]));
    assert!(app.held_rank.is_some());
    press(&mut app, 'a');
    assert!(app.held_rank.is_some(), "typing does not read the list");
    app.publish_paced_picker_rows();
    assert_eq!(
        app.picker.as_ref().unwrap().matches.len(),
        1,
        "a stale answer is discarded on publication, not shown"
    );

    app.close_file_picker();
    assert!(app.held_rank.is_none(), "a closed picker holds nothing");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_held_answer_offers_enter_only_when_publishing_it_would_release_the_rows() {
    let root = temporary("held-answer-enter-hint");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    let mut picker = FilePicker::new(
        9,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.enable_unified_finder();
    picker.add_paths(vec![ScanEntry::file(root.join("alpha.rs"))]);
    picker.ranking = true;
    let mut finder = ResourceFinder::new(FinderMode::Names);
    finder.merge_files(&picker, "");
    assert!(!finder.matches.is_empty());
    // The finder's own half is still ranking, so the answer below covers
    // only part of the list.
    finder.loading = true;
    let revision = finder.file_rank_revision();
    let held = finder.matches.clone();
    app.picker = Some(picker);
    app.finder = Some(finder);
    // Rows this fresh are what pacing holds an answer back from.
    app.picker_rows_published = Some(Instant::now());
    app.apply_file_picker_event(FilePickerEvent::Ranked {
        scan_id: 9,
        query_revision: 0,
        matches: Vec::new(),
        match_positions: vec![Some(0)],
        finder_matches: Some(held),
        finder_revision: Some(revision),
        finder_positions: HashMap::new(),
        flushed: false,
    });
    assert!(app.held_rank.is_some(), "the answer is being held");

    let offers_enter = |app: &App| {
        app.overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
            .expect("the picker is open")
            .actions
            .iter()
            .any(|action| action.key_hint == "Enter")
    };
    assert!(
        !offers_enter(&app),
        "a key that published this answer would still find the rows inert"
    );

    app.finder.as_mut().unwrap().loading = false;
    assert!(
        offers_enter(&app),
        "an answer that completes the rank is worth offering Enter for"
    );

    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_content_rescan_waits_for_the_query_to_settle() {
    let root = temporary("content-rescan-settles");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("alpha.rs"),
        "alpha and beta
",
    )
    .unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("fuzzy-grep").unwrap();
    let scanned = app.picker.as_ref().unwrap().scan_id;
    // A walk that stopped at the entry ceiling left matches in the project it
    // never reached, so no longer query can be answered from the entries on
    // hand. That is the case that re-walks.
    app.picker.as_mut().unwrap().limited = true;

    // The walk does not run on the keystroke: it replaces the corpus the rows
    // on screen are read from, and one per character is what emptied and
    // refilled the list for every letter typed.
    press(&mut app, 'a');
    assert!(app.picker.as_ref().unwrap().content_rescan_needed());
    assert_eq!(
        app.picker.as_ref().unwrap().scan_id,
        scanned,
        "the keystroke itself starts no walk"
    );
    let owed = app
        .picker_pacing_delay(Instant::now())
        .expect("the loop is owed a re-scan");
    assert!(owed <= PICKER_LIST_INTERVAL);

    // Typing on pushes it out rather than adding a second walk.
    press(&mut app, 'p');
    assert_eq!(app.picker.as_ref().unwrap().scan_id, scanned);
    pacing_tick(&mut app);
    assert_ne!(
        app.picker.as_ref().unwrap().scan_id,
        scanned,
        "one walk, taken once the query stopped moving"
    );
    assert!(
        app.content_rescan_due.is_none(),
        "and no second walk left owed"
    );

    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_content_walk_with_nothing_yet_keeps_the_rows_it_is_replacing() {
    let root = temporary("content-walk-not-yet");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (scanner, _events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    let mut picker = FilePicker::grep(
        9,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.add_content(vec![content_hits("/project/alpha.rs", 1)]);
    let mut finder = ResourceFinder::new(FinderMode::Contents);
    finder.merge_files(&picker, "");
    assert_eq!(finder.matches.len(), 1);
    let revision = finder.file_rank_revision();
    app.picker = Some(picker);
    app.finder = Some(finder);
    let answer = |flushed| FilePickerEvent::Ranked {
        scan_id: 9,
        query_revision: 0,
        matches: Vec::new(),
        match_positions: vec![None],
        finder_matches: Some(Vec::new()),
        finder_revision: Some(revision),
        finder_positions: HashMap::new(),
        flushed,
    };

    // The walk is still running, so finding nothing yet is not an answer.
    app.apply_file_picker_event(answer(false));
    assert_eq!(
        app.finder.as_ref().unwrap().matches.len(),
        1,
        "rows the reader is choosing from are not emptied by a walk in progress"
    );

    // The flush a finished scan asks for is the answer, empty or not.
    app.apply_file_picker_event(answer(true));
    assert!(
        app.finder.as_ref().unwrap().matches.is_empty(),
        "a finished walk that found nothing says so"
    );

    app.close_file_picker();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn settled_content_header_excludes_retired_item_slots() {
    let root = temporary("content-header-active-candidates");
    fs::create_dir_all(&root).unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let mut picker = FilePicker::grep(
        19,
        root.clone(),
        crate::file_picker::ScanScope::ignoring(&root),
    );
    picker.finish(0, false);
    let mut finder = ResourceFinder::new(FinderMode::Contents);
    finder.begin_content_scan_unmerged("needle", Arc::new(HashSet::new()));
    finder.append_content_items_unmerged(
        [0, 1].map(|row| {
            ResourceItem::content(
                format!("scratch:{}", row + 1),
                "needle",
                ResourceTarget::BufferLocation {
                    buffer: 0,
                    row,
                    column: 0,
                },
                ResourceKind::Buffer,
            )
        }),
        "needle",
    );
    finder.retire_content_item(0, false).unwrap();
    finder.finish_content_scan_unmerged(false);
    assert_eq!(finder.items.len(), 2, "the retired slot remains allocated");
    assert_eq!(finder.content_item_count(), 1);
    app.picker = Some(picker);
    app.finder = Some(finder);

    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 80,
            height: 24,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });

    assert_eq!(
        app.picker_progress_counts().1,
        1,
        "the settled denominator counts only live content candidates"
    );
    fs::remove_dir_all(root).unwrap();
}

/// Every relative path the finder is currently offering as a file row.
fn finder_file_rows(app: &App) -> Vec<String> {
    let picker = app.picker.as_ref().unwrap();
    let finder = app.finder.as_ref().unwrap();
    finder
        .matches
        .iter()
        .filter_map(|found| match found.source {
            FinderMatchSource::File(entry) => finder
                .file_entry(picker, entry)
                .map(|view| view.relative.to_owned()),
            FinderMatchSource::Resource(_) => None,
        })
        .collect()
}

/// A project whose `.gitignore` hides one file the tests then go looking for.
fn ignored_file_project(name: &str) -> PathBuf {
    let root = language::temporary(name);
    fs::create_dir_all(root.join("build")).unwrap();
    fs::write(root.join(".gitignore"), "build/\n").unwrap();
    fs::write(root.join("tracked.rs"), "tracked marker\n").unwrap();
    fs::write(root.join("build/out.rs"), "generated marker\n").unwrap();
    root
}

fn isolated_app(root: &Path) -> App {
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    App::new_in_isolated_project(root, ports).unwrap()
}

#[test]
fn the_all_files_finder_lists_what_the_project_ignore_files_hide() {
    let root = ignored_file_project("all-files-finder");
    let mut app = isolated_app(&root);

    app.open_project_picker().unwrap();
    let tracked = finder_file_rows(&app);
    assert!(tracked.iter().any(|path| path == "tracked.rs"));
    assert!(
        !tracked.iter().any(|path| path == "build/out.rs"),
        "the ordinary finder still obeys .gitignore: {tracked:?}"
    );

    app.open_all_files_picker().unwrap();
    let everything = finder_file_rows(&app);
    assert!(
        everything.iter().any(|path| path == "build/out.rs"),
        "the all-files finder offers the ignored file: {everything:?}"
    );
    assert!(everything.iter().any(|path| path == "tracked.rs"));
    assert!(
        app.finder.is_some(),
        "it is the unified finder, not a bare picker"
    );
    fs::remove_dir_all(root).unwrap();
}

/// Tab restarts the walk, so the scope has to outlive the scan that opened it.
#[test]
fn switching_the_all_files_finder_to_content_mode_keeps_its_scope() {
    let root = ignored_file_project("all-files-finder-tab");
    let mut app = isolated_app(&root);
    app.open_all_files_picker().unwrap();

    app.toggle_finder_mode();
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Contents);
    let lines = finder_file_rows(&app);
    assert!(
        lines.iter().any(|path| path.starts_with("build/out.rs")),
        "an ignored file's lines survive the mode switch: {lines:?}"
    );

    app.toggle_finder_mode();
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Names);
    let names = finder_file_rows(&app);
    assert!(
        names.iter().any(|path| path == "build/out.rs"),
        "and switching back does not narrow it again: {names:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_path_finder_roots_itself_outside_the_workspace() {
    let root = ignored_file_project("path-finder-project");
    let outside = language::temporary("path-finder-elsewhere");
    fs::create_dir_all(outside.join("nested")).unwrap();
    fs::write(outside.join(".gitignore"), "nested/\n").unwrap();
    fs::write(outside.join("nested/buried.rs"), "buried marker\n").unwrap();
    let mut app = isolated_app(&root);

    app.open_finder_path(&outside).unwrap();
    assert_eq!(
        app.picker.as_ref().unwrap().root,
        outside.canonicalize().unwrap()
    );
    let rows = finder_file_rows(&app);
    assert!(
        rows.iter().any(|path| path == "nested/buried.rs"),
        "the path finder is unfiltered too: {rows:?}"
    );
    assert!(
        !rows.iter().any(|path| path == "tracked.rs"),
        "and it does not carry the workspace's files with it: {rows:?}"
    );
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn a_path_finder_target_that_cannot_be_scanned_is_reported() {
    let root = ignored_file_project("path-finder-refusals");
    let mut app = isolated_app(&root);

    let error = app
        .open_finder_path(&root.join("tracked.rs"))
        .expect_err("a file is not a finder root");
    assert!(error.to_string().contains("not a directory"), "{error}");
    assert!(app.picker.is_none(), "nothing opens over a refused root");

    // Reserved state is refused by the walk rather than by the open, so the
    // overlay does appear and carries the failure.
    fs::create_dir_all(root.join(".git/objects")).unwrap();
    app.open_finder_path(&root.join(".git/objects")).unwrap();
    let failure = app
        .picker
        .as_ref()
        .unwrap()
        .error
        .as_deref()
        .expect("a reserved root fails its scan");
    assert!(failure.contains("reserved"), "{failure}");
    fs::remove_dir_all(root).unwrap();
}

/// A relative path and `~` mean what they mean at every other path prompt.
#[test]
fn the_path_finder_prompt_resolves_a_relative_path_against_the_working_directory() {
    let root = ignored_file_project("path-finder-relative");
    let mut app = isolated_app(&root);
    app.working_directory = root.clone();

    app.open_finder_path(Path::new("build")).unwrap();
    assert_eq!(
        app.picker.as_ref().unwrap().root,
        root.join("build").canonicalize().unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

/// The scope has to reach the background scanner too, which is the seam a
/// real editor uses and the synchronous tests above never touch.
#[test]
fn the_background_scanner_honors_an_unfiltered_scope() {
    let root = ignored_file_project("all-files-background");
    let mut app = isolated_app(&root);
    let (scanner, mut events) = crate::file_picker::scanner();
    app.attach_file_scanner(scanner);
    app.open_all_files_picker().unwrap();

    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    while app.resource_finder_scan_pending() {
                        app.advance_resource_finder_scan();
                    }
                    let event = events.recv().await.unwrap();
                    deliver(&mut app, event);
                    let settled = app
                        .picker
                        .as_ref()
                        .is_some_and(|picker| !picker.loading && !picker.ranking);
                    if settled {
                        break;
                    }
                }
            })
            .await
            .expect("the background scan should settle");
        });

    let rows = finder_file_rows(&app);
    assert!(
        rows.iter().any(|path| path == "build/out.rs"),
        "the scope reached the scanner thread: {rows:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

/// The drawn title and the snapshot an attached client renders read the same
/// scope, because they read it from the same place.
#[test]
fn every_surface_names_the_finder_scope_in_front_of_the_reader() {
    let root = ignored_file_project("finder-scope-title");
    let outside = language::temporary("finder-scope-elsewhere");
    fs::create_dir_all(&outside).unwrap();
    let mut app = isolated_app(&root);

    let scope_of = |app: &App| {
        app.overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
            .expect("the finder is open")
            .title
    };

    app.open_project_picker().unwrap();
    let project = scope_of(&app);
    assert!(
        project.starts_with("Finder · Names · ") && !project.contains("all files"),
        "the ordinary project finder says nothing about its scope: {project}"
    );

    app.open_all_files_picker().unwrap();
    assert!(
        scope_of(&app).starts_with("Finder · Names · all files · "),
        "{}",
        scope_of(&app)
    );

    app.open_finder_path(&outside).unwrap();
    let named = scope_of(&app);
    let expected = outside.canonicalize().unwrap();
    assert!(
        named.contains(&expected.display().to_string()),
        "a finder rooted elsewhere names that root: {named}"
    );

    // The picker owns the label, so the drawn title cannot drift from it.
    assert_eq!(
        app.picker
            .as_ref()
            .unwrap()
            .scope_label(&app.project_root)
            .as_deref(),
        Some(expected.display().to_string().as_str())
    );
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

/// The prompt itself, driven by its keys rather than by the method behind it:
/// opening, completing, selecting, and accepting.
#[test]
fn the_finder_path_prompt_opens_completes_and_accepts_from_its_keys() {
    let root = ignored_file_project("finder-path-keys");
    let mut app = isolated_app(&root);
    app.working_directory = root.clone();

    press(&mut app, ' ');
    press(&mut app, '/');
    press(&mut app, 'p');
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.prompt_kind, PromptKind::FinderPath);

    // Both directories are offered; Down moves within them and Tab accepts
    // the selected row as the whole prompt.
    let offered = app.finder_path_hints().expect("the prompt owns the rows");
    let names = offered
        .iter()
        .map(|hint| hint.value.clone())
        .collect::<Vec<_>>();
    assert!(
        names.iter().any(|value| value.starts_with("build")),
        "an ignored directory is still a legal root: {names:?}"
    );
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    let selected = app.command_selection;
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    assert_eq!(app.command_selection + 1, selected, "Up undoes Down");

    app.command_selection = names
        .iter()
        .position(|value| value.starts_with("build"))
        .expect("build is offered");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.command,
        format!("build{}", std::path::MAIN_SEPARATOR),
        "Tab takes the row as the whole path, unquoted"
    );

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal, "the prompt closes on accept");
    assert_eq!(
        app.picker.as_ref().unwrap().root,
        root.join("build").canonicalize().unwrap()
    );
    assert!(app.finder.is_some(), "it opens the unified finder");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_empty_or_unusable_finder_path_is_refused_at_the_prompt() {
    let root = ignored_file_project("finder-path-refused");
    let mut app = isolated_app(&root);
    app.working_directory = root.clone();

    press(&mut app, ' ');
    press(&mut app, '/');
    press(&mut app, 'p');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.picker.is_none(), "an empty path opens nothing");

    press(&mut app, ' ');
    press(&mut app, '/');
    press(&mut app, 'p');
    for character in "tracked.rs".chars() {
        press(&mut app, character);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.picker.is_none(), "a file is not a finder root");
    fs::remove_dir_all(root).unwrap();
}

/// The colon spelling reaches the same finder, and takes its path in quotes
/// the way every other path argument does.
#[test]
fn the_colon_path_finder_accepts_a_quoted_argument_and_a_bare_call() {
    let root = ignored_file_project("finder-path-colon");
    fs::create_dir_all(root.join("a directory")).unwrap();
    let mut app = isolated_app(&root);
    app.working_directory = root.clone();

    app.execute_command("file-picker-path \"a directory\"")
        .unwrap();
    assert_eq!(
        app.picker.as_ref().unwrap().root,
        root.join("a directory").canonicalize().unwrap()
    );

    app.close_file_picker();
    app.execute_command("file-picker-path").unwrap();
    assert_eq!(
        app.prompt_kind,
        PromptKind::FinderPath,
        "a bare call asks for the path rather than refusing"
    );
    fs::remove_dir_all(root).unwrap();
}
