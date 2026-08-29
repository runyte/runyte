// SPDX-License-Identifier: MPL-2.0

use super::*;

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
    search_for(&mut app, 'S', "foo");
    assert_eq!(app.active().selection.len(), 1, "`S` respects case");

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
    press(&mut app, 'P');
    // Yank keeps the selection, so paste-before lands at its start rather
    // than at the caret. The V1 model cleared the anchor on yank.
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
    let mut picker = FilePicker::new(1, directory.clone());
    picker.add_paths(
        (0..15)
            .map(|index| ScanEntry::file(directory.join(format!("{index:02}.txt"))))
            .collect(),
    );
    picker.finish(0, false);
    app.picker = Some(picker);
    key(&mut app, KeyCode::Char('d'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().selected, 10);
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
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
fn space_closes_a_new_picker_but_remains_a_query_separator_after_text() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.picker = Some(FilePicker::new(1, PathBuf::from("/project")));

    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    assert!(app.picker.is_none(), "initial Space dismisses the picker");

    let mut picker = FilePicker::new(2, PathBuf::from("/project"));
    picker.insert_query_text("src");
    app.picker = Some(picker);
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().query, "src ");
}

#[test]
fn project_finder_switches_modes_without_losing_its_query_or_file_scan() {
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
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Files);
    type_text(&mut app, "alpha");
    let file_matches = app.picker.as_ref().unwrap().matches.len();
    assert_eq!(file_matches, 1);

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let finder = app.finder.as_ref().unwrap();
    assert_eq!(finder.mode, FinderMode::Resources);
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha");
    assert!(
        finder
            .matches
            .iter()
            .any(|found| { finder.items[found.item].target == ResourceTarget::Buffer(1) })
    );
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilePicker)
        .unwrap();
    assert_eq!(overlay.title, "Find · Buffers + terminals");
    assert_eq!(overlay.layout, crate::snapshot::OverlayLayout::Preview);
    assert_eq!(overlay.preview_title.as_deref(), Some("Contents"));
    assert!(matches!(
        overlay.preview,
        Some(crate::snapshot::OverlayPreview::Text(ref lines))
            if lines.first().is_some_and(|line| line == "alpha")
    ));
    assert!(
        overlay
            .actions
            .iter()
            .any(|action| { action.key_hint == "Tab" && action.label == "files" })
    );
    assert!(
        overlay
            .actions
            .iter()
            .any(|action| { action.key_hint == "Ctrl-t" && action.label == "toggle preview" })
    );

    key(&mut app, KeyCode::Char('t'), Modifiers::CONTROL);
    assert!(!app.picker.as_ref().unwrap().show_preview);

    key(&mut app, KeyCode::BackTab, Modifiers::SHIFT);
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Resources);
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha");
    assert!(!app.picker.as_ref().unwrap().show_preview);

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Files);
    assert_eq!(app.picker.as_ref().unwrap().matches.len(), file_matches);
    assert_eq!(app.picker.as_ref().unwrap().query, "alpha");
    assert!(!app.picker.as_ref().unwrap().show_preview);

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(!app.picker.as_ref().unwrap().show_preview);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.picker.is_none());
    assert!(app.finder.is_none());
    assert_eq!(app.active_buffer().path.as_deref(), Some(alpha.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_picker_keeps_tab_navigation_and_has_no_resource_mode() {
    let root = temporary("directory-picker-tab");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_picker_at(root.clone(), FilePickerKind::Files)
        .unwrap();

    assert!(app.finder.is_none());
    assert_eq!(app.picker.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_finder_remembers_each_modes_selection() {
    let root = temporary("project-finder-selections");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    let ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.open_file(root.join("a.txt")).unwrap();
    app.open_file(root.join("b.txt")).unwrap();
    app.open_project_picker().unwrap();

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let first_resource = app.finder.as_ref().unwrap().selected_target();
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    let resource = app.finder.as_ref().unwrap().selected_target();
    assert_ne!(resource, first_resource);
    key(&mut app, KeyCode::BackTab, Modifiers::SHIFT);
    assert_eq!(app.finder.as_ref().unwrap().mode, FinderMode::Resources);
    assert_eq!(
        app.finder.as_ref().unwrap().selected_target(),
        first_resource
    );
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(app.finder.as_ref().unwrap().selected_target(), resource);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
    for (code, modifiers) in [
        (KeyCode::Backspace, Modifiers::NONE),
        (KeyCode::Delete, Modifiers::NONE),
        (KeyCode::Char('w'), Modifiers::CONTROL),
        (KeyCode::Char('k'), Modifiers::CONTROL),
    ] {
        key(&mut app, code, modifiers);
    }
    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    assert_eq!(app.finder.as_ref().unwrap().selected_target(), resource);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.picker.as_ref().unwrap().selected, 1);
    app.close_file_picker();
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
fn project_finder_indexes_and_activates_terminal_metadata() {
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

    app.open_project_picker().unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    type_text(
        &mut app,
        "terminal my_terminal_name project-finder-terminal cat",
    );
    assert_eq!(
        app.finder.as_ref().unwrap().selected_target(),
        Some(ResourceTarget::Terminal(id))
    );
    assert!(
        app.finder
            .as_ref()
            .unwrap()
            .selected_preview()
            .is_some_and(|preview| preview.contains("combined finder terminal preview"))
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
    assert_eq!(app.mode, Mode::Insert);

    app.mode = Mode::Normal;
    app.open_project_picker().unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    type_text(&mut app, "buffer note.txt");
    assert!(matches!(
        app.finder.as_ref().unwrap().selected_target(),
        Some(ResourceTarget::Buffer(_))
    ));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(
        app.active_terminal().is_none(),
        "selecting the terminal pane's underlying buffer must reveal it"
    );
    assert_eq!(app.mode, Mode::Normal);
    app.close_terminal_id(id);
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
    let mut picker = FilePicker::new(9, directory.clone());
    picker.add_paths(vec![ScanEntry::file(path.clone())]);
    picker.finish(0, false);
    app.picker = Some(picker);
    app.refresh_file_picker_preview();

    let FilePreview::Text(lines) = app.picker.as_ref().unwrap().preview.as_ref().unwrap() else {
        panic!("text preview expected");
    };
    assert_eq!(lines[0], "unsaved disk text");

    app.apply_file_picker_event(FilePickerEvent::Files {
        scan_id: 8,
        paths: vec![ScanEntry::file(directory.join("stale.txt"))],
    });
    assert_eq!(app.picker.as_ref().unwrap().entries.len(), 1);
    fs::remove_dir_all(directory).unwrap();
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
