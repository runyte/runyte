// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn launch_targets_open_once_keep_first_active_and_apply_positions_on_first_reveal() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = temporary_directory().join(format!(
        "runyte-launch-targets-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("first file.txt");
    let unicode = directory.join("unicode.txt");
    let clamp = directory.join("clamp.txt");
    fs::write(&first, "zero\nalpha\n").unwrap();
    fs::write(&unicode, "top\n😀λz\n").unwrap();
    fs::write(&clamp, "short\nlast").unwrap();

    let mut app = App::new_with_targets(
        Config::default(),
        vec![
            LaunchTarget::at(&first, launch_position(2, Some(3))),
            LaunchTarget::at(&unicode, launch_position(2, Some(2))),
            LaunchTarget::new(directory.join("./unicode.txt")),
            LaunchTarget::at(&clamp, launch_position(99, Some(99))),
        ],
    )
    .unwrap();

    assert_eq!(app.buffers.len(), 3, "duplicate paths share one buffer");
    assert_eq!(app.active_buffer().path.as_deref(), Some(first.as_path()));
    assert_eq!(cursor(&app), Position::new(1, 2));

    // Closing the first launch buffer reveals the next one directly,
    // without going through switch_buffer. Its pending position must be
    // consumed on that first reveal, not on a later switch away and back.
    app.close_buffer(0);
    assert_eq!(app.active_buffer().path.as_deref(), Some(unicode.as_path()));
    assert_eq!(cursor(&app), Position::new(1, 1));
    assert_eq!(app.active_buffer().char_at(app.active().head()), Some('λ'));

    app.switch_buffer(2);
    assert_eq!(app.active_buffer().path.as_deref(), Some(clamp.as_path()));
    assert_eq!(cursor(&app), Position::new(1, 3));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn multiple_binary_launch_targets_fail_before_one_can_be_silently_dropped() {
    let directory = temporary("launch-binary-targets");
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("first.bin");
    let second = directory.join("second.bin");
    fs::write(&first, [0, 1]).unwrap();
    fs::write(&second, [0, 2]).unwrap();

    let error = App::new_with_targets(
        Config::default(),
        vec![LaunchTarget::new(first), LaunchTarget::new(second)],
    )
    .err()
    .expect("two binary targets must be rejected");
    assert!(
        error
            .to_string()
            .contains("open binary files one at a time")
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_late_binary_startup_target_reaches_the_external_program_prompt() {
    let directory = temporary("launch-late-binary");
    fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("late-invalid.bin");
    let mut contents = vec![b'a'; 8192];
    contents.push(0xff);
    fs::write(&binary, contents).unwrap();

    let app = App::new_with_targets(Config::default(), vec![LaunchTarget::new(&binary)]).unwrap();

    assert_eq!(app.prompt_kind, PromptKind::ExternalProgram);
    assert_eq!(app.external_target.as_deref(), Some(binary.as_path()));
    assert!(
        app.buffers.iter().all(|buffer| buffer.path.is_none()),
        "the binary startup target became an editable buffer"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn a_file_open_rejects_a_symlink_identity_changed_after_preflight() {
    use std::os::unix::fs::symlink;

    let directory = temporary("open-identity-race");
    fs::create_dir_all(&directory).unwrap();
    let first = directory.join("first.txt");
    let second = directory.join("second.txt");
    let alias = directory.join("alias.txt");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();
    symlink(&first, &alias).unwrap();
    let expected = crate::path_safety::path_identity(&alias).unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(&second, &alias).unwrap();

    let error = open_or_new_at_identity(&alias, &expected, false).unwrap_err();

    assert!(error.to_string().contains("changed its resolved identity"));
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn unresolved_launch_paths_preserve_parent_components_across_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = temporary("launch-symlink-parent");
    let real = directory.join("real");
    fs::create_dir_all(&real).unwrap();
    symlink(&real, directory.join("link")).unwrap();

    let unresolved = resolve_launch_path(PathBuf::from("link/../new"), &directory);
    assert_eq!(unresolved, directory.join("link/../new"));
    assert_ne!(unresolved, directory.join("new"));

    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn missing_launch_targets_below_a_symlinked_parent_share_one_buffer() {
    use std::os::unix::fs::symlink;

    let directory = temporary("launch-missing-symlink-parent");
    let real = directory.join("real");
    let alias = directory.join("alias");
    fs::create_dir_all(&real).unwrap();
    symlink("real", &alias).unwrap();
    let target = real.join("new.txt");
    let alias_target = alias.join("new.txt");

    let app = App::new_with_targets(
        Config::default(),
        vec![LaunchTarget::new(&alias_target), LaunchTarget::new(&target)],
    )
    .unwrap();

    assert_eq!(app.buffers.len(), 1);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(alias_target.as_path())
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn grammar_colon_is_runyte_only_and_rejects_removed_vim() {
    let mut app = App::new(Config::default(), None).unwrap();
    assert!(matches!(
        app.execute_command("grammar simple").unwrap(),
        CommandOutcome::UserError(message) if message.contains("expected runyte")
    ));
    assert_eq!(app.grammar_kind(), GrammarKind::Runyte);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.config.editor.grammar, GrammarKind::Runyte);

    press(&mut app, ':');
    assert_eq!(app.mode, Mode::Command);
    app.command = "grammar ru".to_owned();
    app.command_cursor = app.command.chars().count();
    app.complete_selected_command();
    assert_eq!(app.command, "grammar runyte");
    app.close_prompt();

    assert!(matches!(
        app.execute_command("grammar vim").unwrap(),
        CommandOutcome::UserError(message) if message.contains("expected runyte")
    ));
    assert_eq!(app.grammar_kind(), GrammarKind::Runyte);
    assert_eq!(app.mode, Mode::Normal);
    app.execute_command("grammar helix").unwrap();
    assert_eq!(app.grammar_kind(), GrammarKind::Runyte);
    assert_eq!(app.mode, Mode::Normal);
}

pub(super) fn vim_app(text: &str) -> App {
    let mut config = Config::default();
    config.editor.grammar = GrammarKind::Vim;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, text);
    app.active_mut()
        .mark_selection_semantics(SelectionSemantics::HalfOpen);
    app
}

#[test]
fn vim_addresses_horizontal_clamping_and_insert_escape_are_conventional() {
    let mut app = vim_app("zero\none\ntwo");
    press(&mut app, 'G');
    assert_eq!(cursor(&app).row, 2);
    press(&mut app, '1');
    press(&mut app, 'G');
    assert_eq!(cursor(&app).row, 0);
    press(&mut app, '2');
    press(&mut app, 'G');
    assert_eq!(cursor(&app).row, 1);
    press(&mut app, 'g');
    press(&mut app, 'g');
    assert_eq!(cursor(&app).row, 0);
    press(&mut app, '2');
    press(&mut app, 'g');
    press(&mut app, 'g');
    assert_eq!(cursor(&app).row, 1);

    press(&mut app, '$');
    let end = cursor(&app);
    press(&mut app, 'l');
    assert_eq!(cursor(&app), end, "Vim l does not wrap to the next row");
    press(&mut app, '0');
    press(&mut app, 'h');
    assert_eq!(cursor(&app).col, 0, "Vim h does not wrap to the prior row");

    press(&mut app, 'i');
    app.handle_input(InputEvent::Text("λ".to_owned())).unwrap();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(
        cursor(&app).col,
        0,
        "Esc returns onto the inserted Unicode character"
    );
    assert_eq!(app.mode, Mode::Normal);

    let mut counted_line_end = vim_app("a\nbcd\nef");
    press(&mut counted_line_end, '2');
    press(&mut counted_line_end, '$');
    assert_eq!(cursor(&counted_line_end), Position::new(1, 2));

    let mut counted_till = vim_app("a,b,c,d");
    press(&mut counted_till, '2');
    press(&mut counted_till, 't');
    press(&mut counted_till, ',');
    assert_eq!(cursor(&counted_till).col, 2);
}

#[test]
fn vim_character_edits_visual_exit_and_insert_first_nonblank_are_exact() {
    let mut replace = vim_app("abc");
    press(&mut replace, 'r');
    press(&mut replace, 'X');
    assert_eq!(text(&replace), "Xbc");
    set_cursor(&mut replace, 0, 2);
    press(&mut replace, 'r');
    press(&mut replace, 'Y');
    assert_eq!(text(&replace), "XbY");

    let mut counted_x = vim_app("abc");
    press(&mut counted_x, '3');
    press(&mut counted_x, 'x');
    assert_eq!(text(&counted_x), "");

    let mut line_end = vim_app("abc");
    set_cursor(&mut line_end, 0, 2);
    press(&mut line_end, 'd');
    press(&mut line_end, '$');
    assert_eq!(text(&line_end), "ab");

    let mut visual = vim_app("abcd");
    press(&mut visual, 'v');
    press(&mut visual, 'l');
    key(&mut visual, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(cursor(&visual).col, 1);

    let mut first_nonblank = vim_app("   abc");
    press(&mut first_nonblank, 'I');
    assert_eq!(first_nonblank.mode, Mode::Insert);
    assert_eq!(cursor(&first_nonblank).col, 3);
}

#[test]
fn vim_shifted_uppercase_visual_lines_and_undo_redo_work_end_to_end() {
    let mut file = vim_app("one\ntwo\nthree");
    key(&mut file, KeyCode::Char('G'), Modifiers::SHIFT);
    assert_eq!(cursor(&file).row, 2);
    key(&mut file, KeyCode::Char('V'), Modifiers::SHIFT);
    assert_eq!(file.mode, Mode::Select);
    assert_eq!(
        file.active().selection_semantics(),
        SelectionSemantics::VimLinewise
    );
    press(&mut file, 'k');
    press(&mut file, 'y');
    assert_eq!(file.registers[&'"'].text, "two\nthree\n");
    assert!(file.registers[&'"'].linewise);
    assert_eq!(file.mode, Mode::Normal);

    press(&mut file, 'i');
    press(&mut file, 'X');
    key(&mut file, KeyCode::Escape, Modifiers::NONE);
    let edited = text(&file);
    press(&mut file, 'u');
    assert_ne!(text(&file), edited);
    key(&mut file, KeyCode::Char('r'), Modifiers::CONTROL);
    assert_eq!(text(&file), edited);

    let mut config = Config::default();
    config.editor.grammar = GrammarKind::Vim;
    let mut empty = App::new(config, None).unwrap();
    press(&mut empty, 'u');
    assert_eq!(empty.status, "nothing to undo");
    key(&mut empty, KeyCode::Char('r'), Modifiers::CONTROL);
    assert_eq!(empty.status, "nothing to redo");
}

#[test]
fn vim_visual_lines_edit_and_undo_an_explorer_as_buffer_rows() {
    let directory = temporary("runyte-vim-visual-line-explorer");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("alpha.txt"), "alpha").unwrap();
    fs::write(directory.join("beta.txt"), "beta").unwrap();
    let mut config = Config::default();
    config.editor.grammar = GrammarKind::Vim;
    let mut explorer = App::new(config, Some(directory.clone())).unwrap();
    let before = text(&explorer);

    key(&mut explorer, KeyCode::Char('V'), Modifiers::SHIFT);
    press(&mut explorer, 'd');
    assert_ne!(text(&explorer), before);
    press(&mut explorer, 'u');
    assert_eq!(text(&explorer), before);
    key(&mut explorer, KeyCode::Char('r'), Modifiers::CONTROL);
    assert_ne!(text(&explorer), before);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn vim_added_word_find_search_jump_and_operator_motions_execute_in_files() {
    let mut words = vim_app("one two three");
    set_cursor(&mut words, 0, 10);
    press(&mut words, 'g');
    press(&mut words, 'e');
    assert_eq!(cursor(&words).col, 6);

    let mut last_nonblank = vim_app("abc   \nnext");
    press(&mut last_nonblank, 'g');
    press(&mut last_nonblank, '_');
    assert_eq!(cursor(&last_nonblank).col, 2);

    let mut find = vim_app("a:x:x");
    press(&mut find, 'f');
    press(&mut find, 'x');
    assert_eq!(cursor(&find).col, 2);
    press(&mut find, ';');
    assert_eq!(cursor(&find).col, 4);
    press(&mut find, ',');
    assert_eq!(cursor(&find).col, 2);

    let mut word_search = vim_app("alpha beta alpha");
    press(&mut word_search, '*');
    assert_eq!(cursor(&word_search).col, 11);
    press(&mut word_search, '#');
    assert_eq!(cursor(&word_search).col, 0);

    let mut find_delete = vim_app("abc:def");
    press(&mut find_delete, 'd');
    press(&mut find_delete, 'f');
    press(&mut find_delete, ':');
    assert_eq!(text(&find_delete), "def");

    let mut unavailable_bracket = vim_app("(abc) tail");
    press(&mut unavailable_bracket, 'd');
    press(&mut unavailable_bracket, '%');
    assert_eq!(text(&unavailable_bracket), "(abc) tail");

    let bracket_path = temporary("runyte-vim-bracket-motion.rs");
    fs::write(&bracket_path, "fn main() { call(); }\n").unwrap();
    let mut config = Config::default();
    config.editor.grammar = GrammarKind::Vim;
    let mut bracket_delete = App::new(config, Some(bracket_path.clone())).unwrap();
    set_cursor(&mut bracket_delete, 0, 10);
    press(&mut bracket_delete, 'd');
    press(&mut bracket_delete, '%');
    assert_eq!(text(&bracket_delete), "fn main() \n");
    fs::remove_file(bracket_path).unwrap();

    let mut to_end = vim_app("one\ntwo\nthree");
    press(&mut to_end, 'd');
    key(&mut to_end, KeyCode::Char('G'), Modifiers::SHIFT);
    assert_eq!(text(&to_end), "");
}

#[test]
fn vim_operator_counts_linewise_registers_change_and_cw_are_shared_edits() {
    let mut multiplied = vim_app("one two three four five six seven");
    for key_name in ['2', 'd', '3', 'w'] {
        press(&mut multiplied, key_name);
    }
    let mut direct = vim_app("one two three four five six seven");
    for key_name in ['d', '6', 'w'] {
        press(&mut direct, key_name);
    }
    assert_eq!(text(&multiplied), text(&direct));

    let mut lines = vim_app("one\ntwo\nthree\n");
    press(&mut lines, 'y');
    press(&mut lines, 'y');
    assert_eq!(lines.registers[&'"'].text, "one\n");
    assert!(lines.registers[&'"'].linewise);
    press(&mut lines, 'p');
    assert_eq!(text(&lines), "one\none\ntwo\nthree\n");

    let mut delete_lines = vim_app("one\ntwo\nthree\n");
    press(&mut delete_lines, 'd');
    press(&mut delete_lines, 'j');
    assert_eq!(text(&delete_lines), "three\n");

    let mut change_line = vim_app("one\ntwo\n");
    press(&mut change_line, 'c');
    press(&mut change_line, 'c');
    assert_eq!(text(&change_line), "\ntwo\n");
    assert_eq!(change_line.mode, Mode::Insert);
    press(&mut change_line, 'X');
    assert_eq!(text(&change_line), "X\ntwo\n");

    let mut change_word = vim_app("one two");
    press(&mut change_word, 'c');
    press(&mut change_word, 'w');
    assert_eq!(text(&change_word), " two");
    assert_eq!(change_word.mode, Mode::Insert);

    let mut whitespace_cw = vim_app("one two");
    set_cursor(&mut whitespace_cw, 0, 3);
    press(&mut whitespace_cw, 'c');
    press(&mut whitespace_cw, 'w');
    assert_eq!(text(&whitespace_cw), "onetwo");
}

#[test]
fn vim_visual_line_change_preserves_crlf_registers_and_undo_grouping() {
    for (source, row, changed, register, inserted) in [
        (
            "one\r\ntwo\r\n",
            0,
            "\r\ntwo\r\n",
            "one\r\n",
            "X\r\ntwo\r\n",
        ),
        ("one\r\ntwo", 1, "one\r\n", "two\r\n", "one\r\nX"),
    ] {
        let mut app = vim_app(source);
        set_cursor(&mut app, row, 0);

        press(&mut app, 'V');
        press(&mut app, 'c');

        assert_eq!(text(&app), changed, "change row {row}");
        assert_eq!(app.registers[&'"'].text, register, "register row {row}");
        assert!(app.registers[&'"'].linewise);
        assert_eq!(app.mode, Mode::Insert);

        press(&mut app, 'X');
        assert_eq!(text(&app), inserted, "insert row {row}");
        key(&mut app, KeyCode::Escape, Modifiers::NONE);
        press(&mut app, 'u');
        assert_eq!(text(&app), source, "undo row {row}");
    }
}

#[test]
fn vim_visual_syntax_objects_are_half_open_and_failed_objects_are_atomic() {
    let path = temporary("vim-syntax-objects.rs");
    fs::write(&path, "fn café(x: i32) { x; }\n").unwrap();
    let mut config = Config::default();
    config.editor.grammar = GrammarKind::Vim;
    let mut app = App::new(config, Some(path.clone())).unwrap();

    press(&mut app, 'v');
    press(&mut app, 'a');
    press(&mut app, 'f');
    assert_eq!(
        app.active().selection_semantics(),
        SelectionSemantics::HalfOpen
    );
    press(&mut app, 'y');
    assert_eq!(app.registers[&'"'].text, "fn café(x: i32) { x; }");

    let before = text(&app);
    set_cursor(&mut app, 0, 0);
    press(&mut app, 'd');
    press(&mut app, 'i');
    press(&mut app, 'p');
    assert_eq!(
        text(&app),
        before,
        "missing parameter object must not delete"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn syntax_namespace_text_objects_end_on_the_last_included_character() {
    let path = temporary("inclusive-syntax-objects.rs");
    let source = "fn demo() { call(2 + 3); }\n";
    fs::write(&path, source).unwrap();

    for grammar in [GrammarKind::Runyte, GrammarKind::Vim] {
        let mut config = Config::default();
        config.editor.grammar = grammar;
        let mut app = App::new(config, Some(path.clone())).unwrap();

        for (part, expected, last) in [('i', "2 + 3", '3'), ('a', "(2 + 3)", ')')] {
            app.active_mut()
                .replace_selection(Selection::point(source.find('2').unwrap()));
            app.enter_normal_mode();
            for key in [' ', 'x', part, '('] {
                press(&mut app, key);
            }

            assert_eq!(
                app.active().selection_semantics(),
                SelectionSemantics::Runyte
            );
            assert_eq!(app.active_buffer().char_at(app.active().head()), Some(last));
            press(&mut app, 'y');
            assert_eq!(app.read_selected_register().text, expected);
        }
    }

    fs::remove_file(path).unwrap();
}

#[test]
fn vim_registers_are_deferred_validated_and_macros_stop_post_dispatch() {
    let mut app = vim_app("abc");
    press(&mut app, '"');
    press(&mut app, 'a');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'x');
    assert!(!app.registers.contains_key(&'a'));

    press(&mut app, '"');
    press(&mut app, '_');
    press(&mut app, 'x');
    assert_eq!(text(&app), "c");
    press(&mut app, '"');
    press(&mut app, '1');
    assert!(app.status_error);

    let mut invalid_operator = vim_app("abc");
    press(&mut invalid_operator, '"');
    press(&mut invalid_operator, 'a');
    press(&mut invalid_operator, 'd');
    press(&mut invalid_operator, 'z');
    press(&mut invalid_operator, 'x');
    assert!(!invalid_operator.registers.contains_key(&'a'));

    let mut macros = vim_app("");
    press(&mut macros, 'q');
    press(&mut macros, 'a');
    press(&mut macros, 'i');
    press(&mut macros, 'x');
    key(&mut macros, KeyCode::Escape, Modifiers::NONE);
    press(&mut macros, 'q');
    assert_eq!(macros.macros[&'a'].len(), 3, "stop q is not recorded");
    press(&mut macros, '2');
    press(&mut macros, '@');
    press(&mut macros, 'a');
    finish_macro_replay(&mut macros);
    assert_eq!(text(&macros), "xxx");
}

pub(super) fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, character);
    }
}

/// Runs a command the way a person does, through `:` and Enter, so the
/// palette's own mode handling is part of what is under test.
pub(super) fn type_command(app: &mut App, command: &str) {
    press(app, ':');
    type_text(app, command);
    key(app, KeyCode::Enter, Modifiers::NONE);
}

#[test]
fn startup_status_reports_only_lazy_configs_that_were_actually_used() {
    let registry = Registry::new_with_broken_config_for_test("rust", false);
    assert_eq!(
        startup_status(&registry.errors(), ":? or Space+? for help"),
        ":? or Space+? for help"
    );

    let rust = registry.language_for_name("rust").unwrap();
    assert!(DocumentSyntax::new(&Text::from_str("fn main() {}\n"), rust, &registry).is_none());
    let status = startup_status(&registry.errors(), ":? or Space+? for help");
    assert!(status.contains("1 grammar(s) unavailable"));
    assert!(status.contains("rust: query failed to compile"));
}

fn app_with_broken_rust_registry() -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.registry = Arc::new(Registry::new_with_broken_config_for_test("rust", false));
    app.reported_registry_errors.clear();
    // These tests drive server edits against files under `temporary`.
    app.project_root = temporary_directory();
    app
}

#[test]
fn opening_a_lazily_broken_language_reports_once_and_keeps_the_buffer_editable() {
    let directory = temporary("lazy-registry-open");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("broken.rs");
    fs::write(&path, "fn main() {}\n").unwrap();
    let mut app = app_with_broken_rust_registry();

    app.open_file(path.clone()).unwrap();

    assert!(app.status_error);
    assert!(app.status.contains("1 grammar(s) unavailable"));
    assert!(app.status.contains("rust: query failed to compile"));
    assert!(!app.active_buffer().is_read_only());
    assert!(app.syntax[app.active().buffer].is_none());
    assert!(app.edit(Transaction::insert(0, "// still editable\n")));

    fs::write(&path, "fn reloaded() {}\n").unwrap();
    app.reload_file().unwrap();
    assert!(app.file_reload_confirmation.is_some());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.status, format!("reloaded {}", path.display()));
    assert!(!app.status_error);
    assert_eq!(app.reported_registry_errors.len(), 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn saving_a_scratch_buffer_into_a_lazily_broken_language_reports_the_failure() {
    let directory = temporary("lazy-registry-save-as");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("saved.rs");
    let mut app = app_with_broken_rust_registry();
    seed(&mut app, "fn saved() {}\n");

    app.save(Some(path.clone()), false).unwrap();

    assert!(path.is_file());
    assert!(app.status_error);
    assert!(app.status.contains("rust: query failed to compile"));
    assert!(!app.active_buffer().is_read_only());
    assert!(app.syntax[app.active().buffer].is_none());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn lsp_edit_of_an_offscreen_broken_language_merges_failure_with_outer_success() {
    let directory = temporary("lazy-registry-lsp-edit");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("offscreen.rs");
    fs::write(&path, "fn target() {}\n").unwrap();
    let mut app = app_with_broken_rust_registry();
    app.apply_lsp_event(LspEvent::Ready {
        language: "rust".into(),
        generation: 1,
        name: "mock-rust-server".into(),
        encoding: Encoding::Utf8,
        sync: DocumentSync::default(),
        capabilities: Capabilities::everything_for_test(),
    });

    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(7),
        edits: vec![DocumentEdit {
            path: path.clone(),
            version: None,
            edits: vec![crate::lsp::TextEdit {
                range: LspRange {
                    start: LspPosition {
                        line: 0,
                        character: 0,
                    },
                    end: LspPosition {
                        line: 0,
                        character: 0,
                    },
                },
                new_text: "// applied offscreen\n".into(),
            }],
        }],
        skipped: 0,
    });

    assert!(app.status_error);
    assert!(app.status.contains("applied"));
    assert!(app.status.contains("rust: query failed to compile"));
    let buffer = app
        .buffers
        .iter()
        .position(|buffer| buffer.path.as_deref() == Some(path.as_path()))
        .unwrap();
    assert!(
        app.buffers[buffer]
            .to_string()
            .starts_with("// applied offscreen")
    );
    assert!(app.syntax[buffer].is_none());
    assert_eq!(app.reported_registry_errors.len(), 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn incremental_injected_failure_is_presented_after_the_edit() {
    let directory = temporary("lazy-registry-injected-edit");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("outer.md");
    fs::write(&path, "# Outer\n").unwrap();
    let mut app = app_with_broken_rust_registry();
    app.open_file(path).unwrap();
    assert!(!app.status_error);
    assert!(app.syntax[app.active().buffer].is_some());

    let end = app.active_buffer().len_chars();
    assert!(app.edit(Transaction::insert(
        end,
        "\n```rust\nfn injected() {}\n```\n"
    )));

    assert!(app.status_error);
    assert!(app.status.contains("rust: query failed to compile"));
    assert!(app.syntax[app.active().buffer].is_some());
    assert_eq!(app.reported_registry_errors.len(), 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn document_outline_direct_colon_and_key_paths_share_identity_and_jump_unicode() {
    let directory = temporary("document-outline-identity");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("outline.rs");
    fs::write(&path, "mod café {\n    fn βeta() {}\n}\n").unwrap();
    let mut app = App::new(Config::default(), Some(path)).unwrap();
    assert!(!app.ports.has_lsp());

    let direct = CommandInvocation::editor(
        EditorCommand::DocumentOutline,
        CommandExecutionContext::default(),
    )
    .unwrap();
    assert_eq!(
        direct.id(),
        CommandId::Editor(EditorCommand::DocumentOutline)
    );
    assert_eq!(app.execute(direct).unwrap(), CommandOutcome::Completed);
    let picker = app.list.as_ref().unwrap();
    assert_eq!(picker.title, "Document outline");
    assert_eq!(picker.items[0].label, "café");
    assert_eq!(picker.items[0].detail, "module");
    assert_eq!(picker.items[1].label, "βeta");
    assert_eq!(picker.items[1].detail, "function · café");

    app.list = None;
    app.list_actions.clear();
    for spelling in ["outline", "document-outline"] {
        let parsed = parse_colon_command(spelling).unwrap();
        assert_eq!(
            parsed.id(),
            CommandId::Editor(EditorCommand::DocumentOutline)
        );
        assert_eq!(app.execute(parsed).unwrap(), CommandOutcome::Completed);
        assert!(app.list.is_some());
        app.list = None;
        app.list_actions.clear();
    }

    press(&mut app, ' ');
    press(&mut app, 'x');
    press(&mut app, 'o');
    assert!(app.list.is_some());
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let range = app.active().selection.primary();
    assert_eq!(
        app.active_buffer().slice(range.from(), range.to() + 1),
        "βeta"
    );
    app.execute(
        CommandInvocation::editor(
            EditorCommand::JumpBackward,
            CommandExecutionContext::default(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(app.active().selection, Selection::point(0));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn document_outline_breadcrumbs_are_strictly_bounded_for_deep_many_items() {
    let directory = temporary("bounded-document-outline-breadcrumbs");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("outline.rs");
    let mut source = String::new();
    for depth in 0..63 {
        source.push_str(&format!("mod m{depth}_{} {{\n", "界".repeat(80)));
    }
    for index in 0..4_000 {
        source.push_str(&format!("fn item_{index}() {{}}\n"));
    }
    for _ in 0..63 {
        source.push_str("}\n");
    }
    fs::write(&path, source).unwrap();
    let mut app = App::new(Config::default(), Some(path)).unwrap();

    app.execute(
        CommandInvocation::editor(
            EditorCommand::DocumentOutline,
            CommandExecutionContext::default(),
        )
        .unwrap(),
    )
    .unwrap();

    let picker = app.list.as_ref().unwrap();
    assert!(picker.items.len() >= 4_000);
    let detail_bytes = picker
        .items
        .iter()
        .map(|item| {
            assert!(item.detail.len() <= OUTLINE_DETAIL_MAX_BYTES);
            assert!(display_cells(&item.detail) <= OUTLINE_DETAIL_MAX_CELLS);
            item.detail.len()
        })
        .sum::<usize>();
    // PickerItem also owns a search projection containing the detail, so
    // bounding every rendered detail bounds both retained copies.
    assert!(detail_bytes <= picker.items.len() * OUTLINE_DETAIL_MAX_BYTES);
    let deep_item = picker
        .items
        .iter()
        .find(|item| item.label == "item_3999")
        .unwrap();
    assert!(deep_item.detail.contains('…'));
    assert!(deep_item.detail.contains("m62_"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_document_outline_picker_is_rejected_after_an_edit() {
    let directory = temporary("stale-document-outline");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("outline.rs");
    fs::write(&path, "fn before() {}\n").unwrap();
    let mut app = App::new(Config::default(), Some(path)).unwrap();
    app.execute(
        CommandInvocation::editor(
            EditorCommand::DocumentOutline,
            CommandExecutionContext::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let buffer = app.active().buffer;
    assert!(app.apply_to_buffer(buffer, &Transaction::insert(0, "// edit\n")));

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.list.is_none());
    assert_eq!(app.status, "document outline is stale; reopen it");
    assert_eq!(app.unavailable_revision, 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn document_outline_reports_unsupported_empty_degraded_and_truncated_results() {
    let directory = temporary("document-outline-statuses");
    fs::create_dir_all(&directory).unwrap();

    let json = directory.join("unsupported.json");
    fs::write(&json, "{}\n").unwrap();
    let mut unsupported = App::new(Config::default(), Some(json)).unwrap();
    assert!(matches!(
        unsupported.execute(CommandInvocation::editor(
            EditorCommand::DocumentOutline,
            CommandExecutionContext::default(),
        ).unwrap()).unwrap(),
        CommandOutcome::Unavailable(message) if message.contains("does not support document outlines")
    ));

    let empty_path = directory.join("empty.rs");
    fs::write(&empty_path, "// no declarations\n").unwrap();
    let mut empty = App::new(Config::default(), Some(empty_path)).unwrap();
    assert_eq!(
        empty
            .execute(
                CommandInvocation::editor(
                    EditorCommand::DocumentOutline,
                    CommandExecutionContext::default(),
                )
                .unwrap()
            )
            .unwrap(),
        CommandOutcome::Status("document outline is empty".to_owned())
    );

    let markdown = directory.join("degraded.md");
    fs::write(&markdown, "# Outer\n\n```json\n{}\n```\n").unwrap();
    let mut degraded = App::new(Config::default(), Some(markdown)).unwrap();
    assert!(matches!(
        degraded.execute(CommandInvocation::editor(
            EditorCommand::DocumentOutline,
            CommandExecutionContext::default(),
        ).unwrap()).unwrap(),
        CommandOutcome::Status(message) if message.contains("degraded")
    ));
    assert!(degraded.list.is_some());

    let truncated_path = directory.join("truncated.rs");
    let mut source = String::new();
    for index in 0..4_100 {
        source.push_str(&format!("fn item_{index}() {{}}\n"));
    }
    fs::write(&truncated_path, source).unwrap();
    let mut truncated = App::new(Config::default(), Some(truncated_path)).unwrap();
    assert_eq!(
        truncated
            .execute(
                CommandInvocation::editor(
                    EditorCommand::DocumentOutline,
                    CommandExecutionContext::default(),
                )
                .unwrap()
            )
            .unwrap(),
        CommandOutcome::Status("document outline is truncated".to_owned())
    );
    assert_eq!(truncated.list.as_ref().unwrap().items.len(), 4_096);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loaded_config_status_keeps_an_initial_lazy_grammar_failure() {
    let mut app = app_with_broken_rust_registry();
    let rust = app.registry.language_for_name("rust").unwrap();
    assert!(DocumentSyntax::new(&Text::from_str("fn main() {}\n"), rust, &app.registry).is_none());
    app.status = startup_status(&app.registry.errors(), ":? or Space+? for help");
    app.status_error = true;

    app.note_loaded_config(Path::new("/tmp/runyte-config.yaml"));

    assert!(app.status.contains("config: /tmp/runyte-config.yaml"));
    assert!(app.status.contains("rust: query failed to compile"));
    assert!(app.status_error);
}

#[test]
fn split_and_close_maintains_valid_focus() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    assert_eq!(app.panes.len(), 3);
    app.close_pane();
    assert_eq!(app.panes.len(), 2);
    assert!(app.panes.contains_key(&app.active_pane));
}

#[test]
fn only_window_drops_views_without_retiring_their_buffers() {
    let directory = temporary("only-window-content-ownership");
    fs::create_dir_all(&directory).unwrap();
    let paths = ["first.txt", "second.txt", "third.txt"].map(|name| directory.join(name));
    for (index, path) in paths.iter().enumerate() {
        fs::write(path, format!("content {index}")).unwrap();
    }

    let mut app = App::new(Config::default(), Some(paths[0].clone())).unwrap();
    let first_pane = app.active_pane;
    let first_buffer = app.active().buffer;
    app.split(Axis::Horizontal, Some(paths[1].clone())).unwrap();
    let second_pane = app.active_pane;
    let second_buffer = app.active().buffer;
    app.split(Axis::Vertical, Some(paths[2].clone())).unwrap();
    let kept_pane = app.active_pane;
    let kept_buffer = app.active().buffer;

    app.only_window();

    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.active_pane, kept_pane);
    assert!(matches!(app.layout, Layout::Pane(pane) if pane == kept_pane));
    assert!(!app.panes.contains_key(&first_pane));
    assert!(!app.panes.contains_key(&second_pane));
    assert_eq!(app.active().buffer, kept_buffer);
    for (buffer, path) in [first_buffer, second_buffer, kept_buffer]
        .into_iter()
        .zip(paths.iter())
    {
        assert!(!app.closed_buffers.contains(&buffer));
        assert_eq!(app.buffers[buffer].path.as_deref(), Some(path.as_path()));
    }

    app.switch_buffer(first_buffer);
    assert_eq!(app.active_pane, kept_pane);
    assert_eq!(app.active().buffer, first_buffer);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn directional_focus_follows_shared_edges_in_a_nested_pane_grid() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.panes.insert(1, Pane::new(0));
    app.panes.insert(2, Pane::new(0));
    //     ┌───────── 0 ─────────┐
    //     ├──── 1 ────┬──── 2 ───┤
    //     └───────────┴──────────┘
    app.layout = Layout::Split {
        axis: Axis::Vertical,
        ratio: u16::MAX / 2 + 1,
        first: Box::new(Layout::Pane(0)),
        second: Box::new(Layout::Split {
            axis: Axis::Horizontal,
            ratio: u16::MAX / 2 + 1,
            first: Box::new(Layout::Pane(1)),
            second: Box::new(Layout::Pane(2)),
        }),
    };
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

    app.active_pane = 1;
    app.focus(1, 0);
    assert_eq!(
        app.active_pane, 2,
        "right follows the shared edge, not pane 0 above"
    );

    app.focus(-1, 0);
    assert_eq!(app.active_pane, 1);

    app.focus(0, -1);
    assert_eq!(app.active_pane, 0, "pane 0 is directly above pane 1");

    app.active_pane = 2;
    app.focus(0, -1);
    assert_eq!(app.active_pane, 0, "pane 0 is directly above pane 2");

    app.focus(0, 1);
    assert_eq!(
        app.active_pane, 2,
        "down returns to the lower pane that was active most recently"
    );

    app.focus(0, -1);
    app.active_pane = 1;
    app.focus(0, -1);
    app.focus(0, 1);
    assert_eq!(
        app.active_pane, 1,
        "the preferred ambiguous target changes with activation history"
    );
}

#[test]
fn resize_commands_move_each_named_boundary_in_cells() {
    let geometry = FrameGeometry {
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
    };

    let mut horizontal = App::new(Config::default(), None).unwrap();
    horizontal.split(Axis::Horizontal, None).unwrap();
    horizontal.active_pane = 0;
    let before = horizontal
        .prepare_view(geometry)
        .pane(0)
        .unwrap()
        .area
        .width;
    horizontal
        .execute(parse_colon_command("resize-right + 4").unwrap())
        .unwrap();
    assert_eq!(
        horizontal
            .prepare_view(geometry)
            .pane(0)
            .unwrap()
            .area
            .width,
        before + 4
    );
    horizontal.active_pane = 1;
    let before = horizontal
        .prepare_view(geometry)
        .pane(1)
        .unwrap()
        .area
        .width;
    horizontal
        .execute(parse_colon_command("resize-left - 2").unwrap())
        .unwrap();
    assert_eq!(
        horizontal
            .prepare_view(geometry)
            .pane(1)
            .unwrap()
            .area
            .width,
        before - 2
    );

    let mut vertical = App::new(Config::default(), None).unwrap();
    vertical.split(Axis::Vertical, None).unwrap();
    vertical.active_pane = 0;
    let before = vertical.prepare_view(geometry).pane(0).unwrap().area.height;
    vertical
        .execute(parse_colon_command("resize-bottom +3").unwrap())
        .unwrap();
    assert_eq!(
        vertical.prepare_view(geometry).pane(0).unwrap().area.height,
        before + 3
    );
    vertical.active_pane = 1;
    let before = vertical.prepare_view(geometry).pane(1).unwrap().area.height;
    vertical
        .execute(parse_colon_command("resize-top + 2").unwrap())
        .unwrap();
    assert_eq!(
        vertical.prepare_view(geometry).pane(1).unwrap().area.height,
        before + 2
    );
}

#[test]
fn equalizing_levels_pane_widths_and_then_each_column_of_heights() {
    let geometry = FrameGeometry {
        screen: Rect {
            width: 90,
            height: 32,
            ..Rect::default()
        },
        editor: Rect {
            width: 90,
            height: 30,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };

    // Three columns, with the middle one stacking two panes.
    let mut app = App::new(Config::default(), None).unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    app.active_pane = 1;
    app.split(Axis::Vertical, None).unwrap();

    app.active_pane = 0;
    app.execute(parse_colon_command("resize-right + 12").unwrap())
        .unwrap();
    app.active_pane = 1;
    app.execute(parse_colon_command("resize-bottom + 6").unwrap())
        .unwrap();

    for ch in [' ', 'w', '='] {
        app.handle_key(KeyStroke::new(KeyCode::Char(ch), Modifiers::NONE))
            .unwrap();
    }
    assert!(app.pending_sequence().is_empty());

    let view = app.prepare_view(geometry);
    let areas = [0, 1, 2, 3].map(|pane| view.pane(pane).expect("every pane is drawn").area);
    for (pane, area) in areas.iter().enumerate() {
        assert_eq!(area.width, 30, "pane {pane} width");
    }
    assert_eq!(areas[0].height, 30, "a column of one keeps the full height");
    assert_eq!(areas[2].height, 30, "a column of one keeps the full height");
    assert_eq!(areas[1].height, 15, "a column of two splits its height");
    assert_eq!(areas[3].height, 15, "a column of two splits its height");
    assert_eq!(app.status, "equalized panes");
}

#[test]
fn ambiguous_directional_focus_falls_back_to_most_recently_opened_pane() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.panes.insert(1, Pane::new(0));
    app.panes.insert(2, Pane::new(0));
    app.record_pane_opened(1);
    app.record_pane_opened(2);
    app.pane_activated_at.clear();
    app.layout = Layout::Split {
        axis: Axis::Vertical,
        ratio: u16::MAX / 2 + 1,
        first: Box::new(Layout::Pane(0)),
        second: Box::new(Layout::Split {
            axis: Axis::Horizontal,
            ratio: u16::MAX / 2 + 1,
            first: Box::new(Layout::Pane(1)),
            second: Box::new(Layout::Pane(2)),
        }),
    };
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

    app.active_pane = 0;
    app.focus(0, 1);

    assert_eq!(app.active_pane, 2);
}

#[test]
fn pane_swap_refuses_when_the_immediately_previous_pane_was_closed() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    let middle = app.active_pane;
    app.split(Axis::Horizontal, None).unwrap();
    let closed_previous = app.active_pane;
    app.activate_pane(middle);
    assert_eq!(app.previously_focused_pane, Some(closed_previous));
    assert!(app.remove_pane(closed_previous));
    assert_eq!(app.panes.len(), 2, "an older pane still survives");

    app.swap_window();

    assert_eq!(app.active_pane, middle);
    assert_eq!(app.previously_focused_pane, None);
    assert_eq!(app.status, "no previously focused pane to swap with");
    assert!(app.status_error);
}

#[test]
fn closing_the_last_pane_explains_how_to_quit() {
    let mut app = App::new(Config::default(), None).unwrap();
    let pane = app.active_pane;

    app.close_pane();

    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.active_pane, pane);
    assert_eq!(
        app.status,
        "Cannot close the last pane. To quit runyte type :quit"
    );
}

#[test]
fn normal_insert_round_trip() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Char('i'), Modifiers::NONE))
        .unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Char('x'), Modifiers::NONE))
        .unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();
    assert_eq!(text(&app), "x");
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn typed_colon_paths_preserve_spaces_and_remove_balanced_quotes() {
    let directory = temporary("typed-command-paths");
    fs::create_dir_all(&directory).unwrap();
    let quoted = directory.join("katalog z plikiem.txt");
    let unquoted = directory.join("dwa  odstępy.txt");
    fs::write(&quoted, "quoted").unwrap();
    fs::write(&unquoted, "unquoted").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.working_directory = directory.clone();
    app.execute_command(r#"open "katalog z plikiem.txt""#)
        .unwrap();
    assert_eq!(app.active_buffer().path.as_deref(), Some(quoted.as_path()));
    app.execute_command("open dwa  odstępy.txt").unwrap();
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(unquoted.as_path())
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn command_inventory_classifies_every_command_and_current_binding() {
    let bindings = crate::keymap::default_keymap().bindings();
    assert_eq!(bindings.len(), 340, "current binding inventory changed");

    let mut rows = HashSet::new();
    for binding in bindings {
        assert!(!binding.modes.is_empty());
        assert!(!binding.sequence.is_empty());
        assert!(!binding.description.is_empty());
        for mode in binding.modes {
            assert!(
                rows.insert((
                    binding.target,
                    *mode,
                    binding.scope,
                    binding.sequence.clone(),
                )),
                "duplicate inventory row for {:?} {:?} {}",
                mode,
                binding.scope,
                binding.sequence
            );
        }
    }
    assert_eq!(rows.len(), 680, "mode-expanded binding inventory changed");

    let shared_colon = COMMANDS
        .iter()
        .filter_map(|spec| match spec.id {
            CommandId::Editor(command) => Some(command),
            CommandId::Colon(_) => None,
        })
        .collect::<HashSet<_>>();
    let mut exposures = Vec::new();
    for command in EditorCommand::ALL {
        let exposure = if INTERNAL_EDITOR_COMMANDS.contains(command) {
            CommandExposure::Internal
        } else if GRAMMAR_ONLY_EDITOR_COMMANDS.contains(command) {
            CommandExposure::GrammarOnly
        } else if bindings.iter().any(|binding| {
            binding.target == BindingTarget::Editor(*command)
                && matches!(binding.availability, BindingAvailability::Unsupported(_))
        }) {
            CommandExposure::UnsupportedBinding
        } else if shared_colon.contains(command) {
            CommandExposure::SharedColon
        } else if bindings.iter().any(|binding| {
            binding.target == BindingTarget::Editor(*command)
                && binding.availability == BindingAvailability::Implemented
        }) || crate::keymap::default_keymap()
            .all_context_actions()
            .iter()
            .any(|action| action.target == BindingTarget::Editor(*command))
        {
            CommandExposure::Bound
        } else {
            panic!("unclassified editor command: {command:?}");
        };
        let _category = command.category();
        exposures.push(exposure);
    }

    assert_eq!(
        exposures
            .iter()
            .filter(|exposure| **exposure == CommandExposure::Internal)
            .count(),
        INTERNAL_EDITOR_COMMANDS.len()
    );
    assert_eq!(
        exposures
            .iter()
            .filter(|exposure| **exposure == CommandExposure::UnsupportedBinding)
            .count(),
        1
    );
    assert_eq!(
        exposures
            .iter()
            .filter(|exposure| **exposure == CommandExposure::GrammarOnly)
            .count(),
        GRAMMAR_ONLY_EDITOR_COMMANDS.len()
    );
    assert_eq!(
        exposures
            .iter()
            .filter(|exposure| **exposure == CommandExposure::SharedColon)
            .count(),
        31
    );
    assert_eq!(
        exposures
            .iter()
            .filter(|exposure| **exposure == CommandExposure::Bound)
            .count(),
        EditorCommand::ALL.len()
            - INTERNAL_EDITOR_COMMANDS.len()
            - GRAMMAR_ONLY_EDITOR_COMMANDS.len()
            - 32
    );

    for spec in COMMANDS {
        let _category = spec.id.category();
        for name in spec.names() {
            let resolved = resolve_command(name).unwrap();
            assert_eq!(resolved.id, spec.id, "identity drift for :{name}");
            assert_eq!(resolved.name, spec.name, "canonical drift for :{name}");
        }
    }
}

fn split_observation(app: &App) -> (usize, Axis, &str) {
    let Layout::Split { axis, .. } = &app.layout else {
        panic!("command did not create a split");
    };
    (app.panes.len(), *axis, app.status.as_str())
}

#[test]
fn vertical_split_key_colon_and_direct_paths_are_equivalent() {
    let mut key_path = App::new(Config::default(), None).unwrap();
    for character in [' ', 'w', 'v'] {
        press(&mut key_path, character);
    }

    let mut colon = App::new(Config::default(), None).unwrap();
    type_command(&mut colon, "vsplit");

    let mut direct = App::new(Config::default(), None).unwrap();
    direct
        .execute(CommandInvocation::split_vertical(None))
        .unwrap();

    let expected = split_observation(&key_path);
    assert_eq!(split_observation(&colon), expected);
    assert_eq!(split_observation(&direct), expected);
}

#[test]
fn horizontal_split_key_colon_and_direct_paths_are_equivalent() {
    let mut key_path = App::new(Config::default(), None).unwrap();
    for character in [' ', 'w', 's'] {
        press(&mut key_path, character);
    }

    let mut colon = App::new(Config::default(), None).unwrap();
    type_command(&mut colon, "hsplit");

    let mut direct = App::new(Config::default(), None).unwrap();
    direct
        .execute(CommandInvocation::split_horizontal(None))
        .unwrap();

    let expected = split_observation(&key_path);
    assert_eq!(split_observation(&colon), expected);
    assert_eq!(split_observation(&direct), expected);
}

#[test]
fn help_key_is_contextual_while_colon_help_opens_the_manual() {
    let mut key_path = App::new(Config::default(), None).unwrap();
    press(&mut key_path, ' ');
    press(&mut key_path, '?');

    let mut colon = App::new(Config::default(), None).unwrap();
    type_command(&mut colon, "?");

    let mut direct = App::new(Config::default(), None).unwrap();
    direct
        .execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();

    let rendered = |app: &App| app.active_buffer().to_string();
    assert!(key_path.active_buffer().is_help());
    assert!(rendered(&key_path).starts_with("Help · RUNYTE · TEXT"));
    assert!(colon.active_buffer().is_manual());
    assert!(rendered(&colon).starts_with("Help · RUNYTE\n"));
    assert_ne!(rendered(&colon), rendered(&key_path));
    assert_eq!(rendered(&direct), rendered(&key_path));
}

#[test]
fn working_directory_explorer_key_colon_and_direct_paths_are_equivalent() {
    let directory = temporary("command-explorer-equivalence");
    fs::create_dir_all(&directory).unwrap();

    let run = |path: &str| {
        let mut app = App::new(Config::default(), None).unwrap();
        app.working_directory = directory.clone();
        match path {
            "key" => {
                press(&mut app, ' ');
                press(&mut app, 'E');
            }
            "colon" => type_command(&mut app, "explorer"),
            "direct" => {
                app.execute(CommandInvocation::open_explorer(None)).unwrap();
            }
            _ => unreachable!(),
        }
        (
            app.active_buffer().path.clone(),
            app.active_buffer().is_directory(),
            app.mode,
        )
    };

    let expected = run("key");
    assert_eq!(run("colon"), expected);
    assert_eq!(run("direct"), expected);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn terminal_entry_points_report_invalid_contexts_without_starting_a_child() {
    let mut app = App::new(Config::default(), None).unwrap();

    app.open_terminal(Some("'unterminated".to_owned()));
    assert!(app.status_error && app.status.contains("cannot read"));

    app.open_terminal_file_directory(None);
    assert!(app.status_error && app.status.contains("no file directory"));
    app.open_terminal_directory_root(None);
    assert!(app.status_error && app.status.contains("not a directory buffer"));
    app.open_terminal_selected_directory(None);
    assert_eq!(app.status, "buffer is not a directory");
    app.open_terminal_session_directory("missing");
    assert!(
        app.status_error && app.status.contains("missing"),
        "{}",
        app.status
    );
    assert!(
        app.terminals.is_empty(),
        "a refused entry point must not leave a terminal session behind"
    );
}

#[test]
fn space_r_reloads_files_and_refreshes_directories() {
    let directory = temporary("space-reload");
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("note.txt");
    fs::write(&file, "disk").unwrap();

    let mut file_app = App::new(Config::default(), Some(file.clone())).unwrap();
    file_app.buffers[0].apply(&Transaction::insert(0, "draft "));
    press(&mut file_app, ' ');
    press(&mut file_app, 'r');
    assert!(file_app.file_reload_confirmation.is_some());
    key(&mut file_app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(text(&file_app), "disk");
    assert_eq!(file_app.status, format!("reloaded {}", file.display()));

    let mut directory_app = App::new(Config::default(), Some(directory.clone())).unwrap();
    fs::write(directory.join("second.txt"), "new").unwrap();
    press(&mut directory_app, ' ');
    press(&mut directory_app, 'r');
    assert!(
        directory_app
            .active_buffer()
            .to_string()
            .contains("second.txt")
    );
    assert_eq!(directory_app.status, "directory refreshed");

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reload_dispatch_matches_the_file_explorer_and_git_list_contract() {
    use super::super::file_workflows::{ReloadDispatch, reload_dispatch};

    assert_eq!(
        reload_dispatch(&BufferKind::Directory),
        ReloadDispatch::Directory
    );
    for (kind, expected) in [
        (BufferKind::GitStatus, ReloadDispatch::GitStatus),
        (BufferKind::GitBranches, ReloadDispatch::GitBranches),
        (BufferKind::GitWorktrees, ReloadDispatch::GitWorktrees),
        (BufferKind::GitLog, ReloadDispatch::GitLog),
        (BufferKind::GitStash, ReloadDispatch::GitStash),
    ] {
        assert_eq!(reload_dispatch(&kind), expected);
    }

    assert_eq!(
        reload_dispatch(&BufferKind::GitBlame),
        ReloadDispatch::File,
        "Git blame is a generated attribution view, not a refreshable Git list"
    );
    assert_eq!(reload_dispatch(&BufferKind::File), ReloadDispatch::File);
}

#[test]
fn save_key_colon_and_direct_paths_are_equivalent() {
    let directory = temporary("command-save-equivalence");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.txt");

    let run = |surface: &str| {
        fs::write(&path, "base").unwrap();
        let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
        assert!(app.buffers[0].apply(&Transaction::insert(0, "draft ")));
        match surface {
            "key" => key(&mut app, KeyCode::Char('s'), Modifiers::CONTROL),
            "colon" => type_command(&mut app, "write"),
            "direct" => {
                app.execute(CommandInvocation::save(None)).unwrap();
            }
            _ => unreachable!(),
        }
        (
            text(&app),
            app.active_buffer().dirty,
            app.status.clone(),
            app.status_error,
            fs::read_to_string(&path).unwrap(),
        )
    };

    let expected = run("key");
    assert_eq!(run("colon"), expected);
    assert_eq!(run("direct"), expected);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn saving_trims_trailing_spaces_and_tabs_by_default_and_can_be_disabled() {
    let directory = temporary("trim-trailing-whitespace");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.txt");
    fs::write(&path, "one  \r\ntwo\t \r\nlast\t").unwrap();

    let mut trimmed = App::new(Config::default(), Some(path.clone())).unwrap();
    key(&mut trimmed, KeyCode::Char('s'), Modifiers::CONTROL);
    assert_eq!(text(&trimmed), "one\r\ntwo\r\nlast");
    assert_eq!(fs::read_to_string(&path).unwrap(), "one\r\ntwo\r\nlast");
    assert!(!trimmed.active_buffer().dirty);
    press(&mut trimmed, 'u');
    assert_eq!(text(&trimmed), "one  \r\ntwo\t \r\nlast\t");
    assert!(trimmed.active_buffer().dirty);

    fs::write(&path, "keep  \n").unwrap();
    let mut config = Config::default();
    config.editor.trim_trailing_whitespace = false;
    let mut preserved = App::new(config, Some(path.clone())).unwrap();
    key(&mut preserved, KeyCode::Char('s'), Modifiers::CONTROL);
    assert_eq!(text(&preserved), "keep  \n");
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep  \n");

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn semantic_outcomes_cover_success_errors_prompts_confirmations_and_unavailable_services() {
    let move_right =
        CommandInvocation::editor(EditorCommand::MoveRight, CommandExecutionContext::default())
            .unwrap();
    let mut completed = App::new(Config::default(), None).unwrap();
    assert_eq!(
        completed.execute(move_right).unwrap(),
        CommandOutcome::Completed
    );

    let mut prompt = App::new(Config::default(), None).unwrap();
    let open_prompt = CommandInvocation::editor(
        EditorCommand::OpenCommandPalette,
        CommandExecutionContext::default(),
    )
    .unwrap();
    assert_eq!(
        prompt.execute(open_prompt.clone()).unwrap(),
        CommandOutcome::Prompt(PromptKind::Command)
    );
    assert_eq!(
        prompt.execute(open_prompt).unwrap(),
        CommandOutcome::Prompt(PromptKind::Command)
    );

    let mut unavailable = App::new(Config::default(), None).unwrap();
    let first = unavailable
        .execute(CommandInvocation::lsp_status())
        .unwrap();
    let second = unavailable
        .execute(CommandInvocation::lsp_status())
        .unwrap();
    assert!(matches!(first, CommandOutcome::Unavailable(_)));
    assert!(matches!(second, CommandOutcome::Unavailable(_)));

    let mut user_error = App::new(Config::default(), None).unwrap();
    seed(&mut user_error, "dirty");
    let quit = parse_colon_command("quit").unwrap();
    assert!(matches!(
        user_error.execute(quit).unwrap(),
        CommandOutcome::UserError(_)
    ));

    let save_directory = temporary("command-outcome-status");
    fs::create_dir_all(&save_directory).unwrap();
    let save_path = save_directory.join("note.txt");
    fs::write(&save_path, "base").unwrap();
    let mut status = App::new(Config::default(), Some(save_path)).unwrap();
    status.buffers[0].apply(&Transaction::insert(0, "draft "));
    assert!(matches!(
        status.execute(CommandInvocation::save(None)).unwrap(),
        CommandOutcome::Status(_)
    ));
    fs::remove_dir_all(save_directory).unwrap();

    let directory = temporary("command-outcome-confirmation");
    fs::create_dir_all(&directory).unwrap();
    let mut confirmation = App::new(Config::default(), Some(directory.clone())).unwrap();
    confirmation.buffers[0].apply(&Transaction::insert(0, "draft\n"));
    let refresh = CommandInvocation::editor(
        EditorCommand::RefreshDirectory,
        CommandExecutionContext::default(),
    )
    .unwrap();
    assert!(matches!(
        confirmation.execute(refresh.clone()).unwrap(),
        CommandOutcome::Confirmation(_)
    ));
    assert!(matches!(
        confirmation.execute(refresh).unwrap(),
        CommandOutcome::Confirmation(_)
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn lsp_colon_and_direct_registry_routes_converge() {
    let run = |direct: bool| {
        let mut app = App::new(Config::default(), None).unwrap();
        let outcome = if direct {
            app.execute(CommandInvocation::lsp_status()).unwrap()
        } else {
            app.execute(parse_colon_command("lsp-status").unwrap())
                .unwrap()
        };
        (outcome, app.status, app.status_error)
    };

    assert_eq!(run(false), run(true));
}

#[test]
fn nested_space_keys_and_direct_colon_invocations_converge() {
    let format = CommandInvocation::from_parts(
        CommandId::Colon(ColonCommand::Format),
        InvocationParameters::None,
        CommandExecutionContext::default(),
    )
    .unwrap();
    let cases = [
        (&[' ', 'l', 'f'][..], format),
        (
            &[' ', 'l', 'R'][..],
            CommandInvocation::lsp_restart(None).unwrap(),
        ),
        (&[' ', 'l', '?'][..], CommandInvocation::lsp_status()),
    ];

    for (keys, direct) in cases {
        let mut keyed = App::new(Config::default(), None).unwrap();
        for key in keys {
            press(&mut keyed, *key);
        }
        let mut invoked = App::new(Config::default(), None).unwrap();
        invoked.execute(direct).unwrap();

        assert_eq!(keyed.status, invoked.status, "key route {keys:?}");
        assert_eq!(
            keyed.status_error, invoked.status_error,
            "key route {keys:?}"
        );
        assert_eq!(keyed.mode, invoked.mode, "key route {keys:?}");
    }
}

#[test]
fn rename_and_rejected_or_stopped_lsp_work_are_unavailable() {
    let rename = CommandInvocation::editor(
        EditorCommand::RenameSymbol,
        CommandExecutionContext::default(),
    )
    .unwrap();
    let mut no_manager = App::new(Config::default(), None).unwrap();
    assert!(matches!(
        no_manager.execute(rename.clone()).unwrap(),
        CommandOutcome::Unavailable(_)
    ));
    assert_ne!(no_manager.prompt_kind, PromptKind::Rename);

    let (mut stopped, _, mut stopped_queue) = rust_app("let value = 1;\n");
    ready(&mut stopped, Encoding::Utf8);
    drain(&mut stopped_queue);
    stopped.apply_lsp_event(LspEvent::Stopped {
        language: "rust".to_owned(),
        message: "rust server stopped".to_owned(),
    });
    assert!(matches!(
        stopped.execute(rename).unwrap(),
        CommandOutcome::Unavailable(_)
    ));
    assert_ne!(stopped.prompt_kind, PromptKind::Rename);

    let (mut rejected, _, mut rejected_queue) = rust_app("fn main() {}\n");
    ready(&mut rejected, Encoding::Utf8);
    drain(&mut rejected_queue);
    drop(rejected_queue);
    assert!(matches!(
        rejected.execute(CommandInvocation::lsp_status()).unwrap(),
        CommandOutcome::Unavailable(_)
    ));
    let goto_definition = CommandInvocation::editor(
        EditorCommand::GotoDefinition,
        CommandExecutionContext::default(),
    )
    .unwrap();
    assert!(matches!(
        rejected.execute(goto_definition).unwrap(),
        CommandOutcome::Unavailable(_)
    ));
    assert!(
        rejected.lsp_requests.is_empty(),
        "a rejected queue must not leave a request that can never answer"
    );
}

#[test]
fn key_and_direct_execution_clear_stale_error_styling() {
    let mut key_path = App::new(Config::default(), None).unwrap();
    seed(&mut key_path, "abc");
    key_path.action_failed("stale error");
    press(&mut key_path, 'l');
    assert!(!key_path.status_error);

    let mut direct = App::new(Config::default(), None).unwrap();
    seed(&mut direct, "abc");
    direct.action_failed("stale error");
    let invocation =
        CommandInvocation::editor(EditorCommand::MoveRight, CommandExecutionContext::default())
            .unwrap();
    assert_eq!(
        direct.execute(invocation).unwrap(),
        CommandOutcome::Completed
    );
    assert!(!direct.status_error);
    assert_eq!(direct.cursor_position(), key_path.cursor_position());
}

#[test]
fn unsupported_key_and_direct_routes_share_the_semantic_unavailable_boundary() {
    const REASON: &str = "shell pipes are not available";
    let mut key_path = App::new(Config::default(), None).unwrap();
    press(&mut key_path, '|');

    let mut direct = App::new(Config::default(), None).unwrap();
    let invocation = CommandInvocation::unavailable_editor(
        EditorCommand::ShellPipe,
        CommandUnavailable::Unsupported(REASON),
    );
    let outcome = direct.execute(invocation).unwrap();

    assert_eq!(outcome, CommandOutcome::Unavailable(direct.status.clone()));
    assert_eq!(direct.status, key_path.status);
    assert_eq!(direct.status_error, key_path.status_error);
    assert_eq!(
        direct.status,
        "Pipe the selection through a shell command is unsupported: shell pipes are not available"
    );
}

#[test]
fn counted_find_keeps_baseline_key_behavior_and_is_rejected_directly() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "axxb");
    for character in ['2', 'f', 'x'] {
        press(&mut app, character);
    }
    assert_eq!(
        app.active().head(),
        1,
        "the Runyte grammar ignores the find count"
    );

    let two = std::num::NonZeroUsize::new(2).unwrap();
    assert_eq!(
        CommandInvocation::editor(
            EditorCommand::FindNextChar,
            CommandExecutionContext::resolved(two, Some('x')),
        ),
        Err(crate::command::CommandInvocationError::CountNotSupported(
            EditorCommand::FindNextChar,
        ))
    );
}

#[test]
fn command_prompt_filters_and_completes_commands() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.handle_key(KeyStroke::new(KeyCode::Char(':'), Modifiers::NONE))
        .unwrap();
    assert_eq!(app.matching_commands().len(), COMMANDS.len());
    assert!(app.matching_commands().iter().any(|matched| {
        matched.name == "path" && matched.spec.id == CommandId::Colon(ColonCommand::Path)
    }));

    for ch in "the".chars() {
        app.handle_key(KeyStroke::new(KeyCode::Char(ch), Modifiers::NONE))
            .unwrap();
    }
    let matches = app.matching_commands();
    assert_eq!(matches.first().map(|matched| matched.name), Some("theme"));
    assert!(
        matches
            .iter()
            .any(|matched| { matched.spec.description.to_lowercase().contains("the") })
    );

    app.handle_key(KeyStroke::new(KeyCode::Tab, Modifiers::NONE))
        .unwrap();
    assert_eq!(app.command, "theme ");
    assert_eq!(app.mode, Mode::Command);

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, ':');
    type_text(&mut app, "the");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.command, "theme ");
    assert_eq!(app.mode, Mode::Command);

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, ':');
    type_text(&mut app, "open");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.command, "open ");
    assert_eq!(app.mode, Mode::Command);

    type_text(&mut app, "\"unterminated");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert!(app.status.contains("unbalanced quoted path"));
    assert_eq!(app.mode, Mode::Command);

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, ':');
    type_text(&mut app, "quit extra");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.status, "quit does not take arguments");
    assert_eq!(app.command, "quit extra");
    assert_eq!(app.mode, Mode::Command);
}

#[test]
fn path_commands_hint_files_and_open_selected_directories_as_explorers() {
    let root = temporary("command-path-hints");
    let directory = root.join("alpha");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("inside.txt"), "inside\n").unwrap();
    fs::write(root.join("alpine.txt"), "file\n").unwrap();
    fs::write(root.join(".hidden"), "hidden\n").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.working_directory = root.clone();
    press(&mut app, ':');
    type_text(&mut app, "open al");

    let hints = app
        .matching_path_hints()
        .expect("a path argument owns hints");
    assert_eq!(
        hints
            .iter()
            .map(|hint| (hint.value.as_str(), hint.is_directory))
            .collect::<Vec<_>>(),
        [("alpha/", true), ("alpine.txt", false)]
    );
    assert!(hints.iter().all(|hint| hint.value != ".hidden"));

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.command, "open alpha/");
    assert!(
        app.matching_path_hints()
            .unwrap()
            .iter()
            .any(|hint| hint.value == "alpha/inside.txt")
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.active_buffer().is_directory());
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(directory.as_path())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn open_expands_home_paths_for_files_and_directory_explorers() {
    let root = temporary("open-home-path");
    let home = root.join("home");
    let project = home.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(home.join(".bashrc"), "export TEST=1\n").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.working_directory = root.clone();
    app.home_directory = Some(home.clone());

    press(&mut app, ':');
    type_text(&mut app, "open ~/.b");
    let hints = app.matching_path_hints().unwrap();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].value, "~/.bashrc");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(home.join(".bashrc").as_path())
    );

    type_command(&mut app, "open ~/project");
    assert!(app.active_buffer().is_directory());
    assert_eq!(app.active_buffer().path.as_deref(), Some(project.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn path_hint_quotes_spaces_and_keeps_the_cursor_inside_directory_quotes() {
    let root = temporary("quoted-command-path-hint");
    let directory = root.join("space dir");
    let child = directory.join("child");
    fs::create_dir_all(&child).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.working_directory = root.clone();
    press(&mut app, ':');
    type_text(&mut app, "open spa");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.command, "open \"space dir/\"");
    assert_eq!(app.command_cursor, app.command.chars().count() - 1);
    type_text(&mut app, "ch");
    assert_eq!(
        app.matching_path_hints().unwrap()[0].value,
        "space dir/child/"
    );
    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.active_buffer().is_directory());
    assert_eq!(app.active_buffer().path.as_deref(), Some(child.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn command_palette_searches_categories_descriptions_and_owned_availability() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, ':');
    type_text(&mut app, "Configuration");
    let matches = app.matching_commands();
    assert!(!matches.is_empty());
    assert!(
        matches
            .iter()
            .all(|matched| matched.category == CommandCategory::Configuration)
    );

    app.command = "optional service".to_owned();
    let matches = app.matching_commands();
    assert_eq!(
        matches
            .iter()
            .map(|matched| matched.name)
            .collect::<Vec<_>>(),
        ["service-health"]
    );

    app.command = "outline".to_owned();
    let outline = app.matching_commands();
    assert_eq!(outline.len(), 1);
    assert_eq!(
        outline[0].availability.reason(),
        Some("syntax is unavailable for this buffer")
    );
}

#[test]
fn session_commands_stay_in_the_palette_and_share_one_availability() {
    let app = App::new(Config::default(), None).unwrap();
    let mut capabilities = app.command_capabilities();
    capabilities.persistent_session = persistent_session_availability(true, false);
    let matches = app.matching_commands_with_capabilities(&capabilities);

    assert_eq!(matches.len(), COMMANDS.len());
    for name in [
        "session-attach",
        "session-list",
        "session-stop",
        "session-rename",
    ] {
        let matched = matches
            .iter()
            .find(|matched| matched.name == name)
            .unwrap_or_else(|| panic!("{name} must remain in the command inventory"));
        assert_eq!(
            matched.availability.reason(),
            Some(crate::service_health::PERSISTENT_SESSION_STANDALONE_REASON)
        );
    }
    assert!(
        matches
            .iter()
            .find(|matched| matched.name == "quit")
            .unwrap()
            .availability
            .is_available()
    );

    // On Unix the namespace becomes available as soon as the workspace is in
    // persistent mode; nothing else gates it.
    #[cfg(unix)]
    {
        let mut app = app;
        app.enable_persistent_session();
        for name in [
            "session-attach",
            "session-list",
            "session-stop",
            "session-rename",
        ] {
            let spec = resolve_command(name).unwrap();
            assert!(
                app.command_capabilities()
                    .command_availability(spec)
                    .is_available(),
                "persistent-mode availability changed for {name}"
            );
        }
    }
}

#[test]
fn session_execution_reports_the_shared_unsupported_platform_reason_first() {
    let mut app = App::new(Config::default(), None).unwrap();
    let invocations = [
        (
            ColonCommand::SessionAttach,
            InvocationParameters::Path(PathBuf::from("attach")),
        ),
        (ColonCommand::SessionList, InvocationParameters::None),
        (
            ColonCommand::SessionStop,
            InvocationParameters::OptionalPath(Some(PathBuf::from("stop"))),
        ),
        (
            ColonCommand::SessionRename,
            InvocationParameters::SessionRename {
                workspace: PathBuf::from("rename"),
                name: "new name".to_owned(),
            },
        ),
    ];

    for (command, parameters) in invocations {
        app.status.clear();
        app.status_error = false;
        app.execute_colon_invocation_for_workspace_platform(command, parameters, false)
            .unwrap();
        assert_eq!(
            app.status,
            crate::service_health::PERSISTENT_SESSION_UNSUPPORTED_REASON
        );
        assert!(app.status_error);
        assert!(app.workspace_switch.is_none());
        assert!(app.list.is_none());
    }
}

#[test]
fn unavailable_palette_activation_stays_typed_and_does_not_mutate_editor_state() {
    let mut app = App::new(Config::default(), None).unwrap();
    let text_before = text(&app);
    press(&mut app, ':');
    type_text(&mut app, "outline");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.command, "outline");
    assert_eq!(text(&app), text_before);
    assert!(app.status.contains("unavailable"));
    assert!(app.list.is_none());
}

#[test]
fn service_health_colon_and_space_routes_open_the_same_read_only_report() {
    let run = |space: bool| {
        let mut app = App::new(Config::default(), None).unwrap();
        if space {
            for character in [' ', 'o', 's'] {
                press(&mut app, character);
            }
        } else {
            app.execute(CommandInvocation::service_health()).unwrap();
        }
        (
            app.list.as_ref().map(|picker| picker.title.clone()),
            app.list.as_ref().map(|picker| picker.items.len()),
            app.list_actions.len(),
        )
    };

    assert_eq!(run(true), run(false));
    assert_eq!(run(false).0.as_deref(), Some("Service health"));
    assert_eq!(run(false).2, 0);
}

#[test]
fn report_navigation_advances_the_snapshot_on_the_first_key() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.list = Some(
        ListPicker::new(
            "Report",
            (0..20)
                .map(|index| PickerItem::new(format!("row {index}"), "", index))
                .collect(),
        )
        .as_report(),
    );

    key(&mut app, KeyCode::Down, Modifiers::NONE);

    assert_eq!(app.list.as_ref().unwrap().report_offset, 1);
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.purpose == crate::snapshot::OverlayPurpose::Report)
        .unwrap();
    assert_eq!(snapshot.row_offset, 1);
    assert_eq!(snapshot.rows[0].label, "row 1");
}

#[test]
fn reload_command_discards_edits_and_clamps_every_shared_view() {
    let path = temporary("reload.txt");
    fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    assert!(app.apply_to_buffer(0, &Transaction::insert(0, "draft ")));
    for pane in app.panes.values_mut() {
        pane.selection = Selection::point(100);
    }
    fs::write(&path, "new\n").unwrap();

    app.execute_command("reload").unwrap();

    assert_eq!(text(&app), "draft one\ntwo\nthree\n");
    assert!(app.file_reload_confirmation.is_some());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(text(&app), "new\n");
    assert!(!app.buffers[0].dirty);
    assert_eq!(app.buffers[0].history_len(), 0);
    assert!(
        app.panes
            .values()
            .all(|pane| pane.selection.primary().head == 4),
        "every pane sharing the reloaded buffer must have a valid caret"
    );
    assert_eq!(app.status, format!("reloaded {}", path.display()));
    fs::remove_file(path).unwrap();
}

#[test]
fn failed_reload_keeps_the_edited_buffer_and_undo_history() {
    let path = temporary("failed-reload.txt");
    fs::write(&path, "saved\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    assert!(app.apply_to_buffer(0, &Transaction::insert(0, "draft ")));
    let before = text(&app);
    let history = app.buffers[0].history_len();
    fs::remove_file(&path).unwrap();

    let outcome = app.execute_command("reload").unwrap();

    assert!(
        matches!(outcome, CommandOutcome::UserError(message) if message.contains("deleted on disk"))
    );
    assert_eq!(text(&app), before);
    assert!(app.buffers[0].dirty);
    assert_eq!(app.buffers[0].history_len(), history);
}

/// Points settings writes at a temporary file. No test may write to the
/// person's real configuration directory.
fn private_theme_config(app: &mut App, root: &Path) -> PathBuf {
    let path = root.join("config.yaml");
    app.note_loaded_config(&path);
    path
}

#[test]
fn theme_names_activate_the_matching_theme() {
    let directory = temporary("theme-switch");
    fs::create_dir_all(&directory).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    private_theme_config(&mut app, &directory);
    assert_eq!(app.theme_name, "default-dark");
    assert_eq!(
        app.terminals.default_colors(),
        DefaultColors::new(Some((0xb9, 0xb9, 0xbe)), Some((0x28, 0x2a, 0x2f)))
    );
    let default_accent = app.theme.accent;

    type_command(&mut app, "theme dark");
    assert_eq!(app.theme_name, "dark");
    assert_eq!(
        app.terminals.default_colors(),
        DefaultColors::new(Some((0xd6, 0xda, 0xe0)), Some((0x16, 0x18, 0x1d)))
    );
    let dark = app.theme.background;
    assert_ne!(app.theme.accent, default_accent);

    type_command(&mut app, "theme base16");
    assert_eq!(app.theme_name, "base16");
    assert_ne!(app.theme.background, dark);

    type_command(&mut app, "theme nonesuch");
    assert!(app.status_error, "an unknown theme must be reported");
    assert_eq!(app.theme_name, "base16", "and must not change the theme");

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bare_theme_opens_the_theme_setting_choices() {
    let mut app = App::new(Config::default(), None).unwrap();

    type_command(&mut app, "theme");

    assert!(matches!(
        app.settings_view,
        Some(SettingsView::Values(ref preview)) if preview.setting == SettingId::Theme
    ));
    assert!(app.list.is_some());
}

/// The theme list is the one setting long enough to be worth reading half
/// at a time, so Tab narrows it to the dark themes, then the light ones,
/// then back to every theme. Other settings keep Tab as a downward move.
#[test]
fn tab_narrows_the_theme_choices_to_one_appearance_at_a_time() {
    let directory = temporary("theme-tab-groups");
    fs::create_dir_all(&directory).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    private_theme_config(&mut app, &directory);
    type_command(&mut app, "theme");

    let names = |app: &App| {
        let picker = app.list.as_ref().unwrap();
        picker
            .visible_indices()
            .into_iter()
            .map(|index| picker.items[index].label.clone())
            .collect::<Vec<_>>()
    };
    let every = names(&app);
    assert!(every.contains(&"gruvbox".to_owned()));
    assert!(every.contains(&"paper".to_owned()));
    assert_eq!(app.list.as_ref().unwrap().tag_label(), "all");

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.list.as_ref().unwrap().tag_label(), "dark");
    let dark = names(&app);
    assert!(dark.contains(&"gruvbox".to_owned()));
    assert!(!dark.contains(&"paper".to_owned()));

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.list.as_ref().unwrap().tag_label(), "light");
    let light = names(&app);
    assert!(light.contains(&"paper".to_owned()));
    assert!(!light.contains(&"gruvbox".to_owned()));
    assert_eq!(dark.len() + light.len(), every.len());

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.list.as_ref().unwrap().tag_label(), "all");
    assert_eq!(names(&app), every);

    // The narrowed row is previewed like any other selection, so Enter
    // still saves what the list is showing.
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let selected = app
        .list
        .as_ref()
        .unwrap()
        .selected_item()
        .unwrap()
        .label
        .clone();
    assert_eq!(app.theme_name, selected);

    fs::remove_dir_all(directory).unwrap();
}

/// A setting whose choices have no such axis keeps Tab as plain downward
/// navigation, and says nothing about a narrowing it does not offer.
#[test]
fn other_setting_choices_keep_tab_as_a_downward_move() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_setting_values(SettingId::EditorShowHiddenFiles);

    assert!(!app.list.as_ref().unwrap().has_tags());
    let first = app.list.as_ref().unwrap().selected;
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_ne!(app.list.as_ref().unwrap().selected, first);

    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::ResultList)
        .unwrap();
    assert!(
        overlay
            .actions
            .iter()
            .all(|action| action.key_hint != "Tab")
    );
}

#[test]
fn the_theme_choice_overlay_names_the_group_tab_moves_to() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_setting_values(SettingId::Theme);

    let hints = |app: &App| {
        app.overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::ResultList)
            .map(|overlay| {
                overlay
                    .actions
                    .iter()
                    .map(|action| format!("{} {}", action.key_hint, action.label))
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    assert_eq!(
        hints(&app),
        ["Tab all", "Enter to save", "Esc cancel"],
        "the theme list names its narrowing and the shortened save hint"
    );

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(hints(&app)[0], "Tab dark");
}

#[test]
fn the_last_selected_theme_is_written_to_the_configuration() {
    let directory = temporary("theme-memory");
    fs::create_dir_all(&directory).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    let path = private_theme_config(&mut app, &directory);

    type_command(&mut app, "theme gruvbox");
    assert_eq!(app.theme_name, "gruvbox");
    assert_eq!(
        Config::load(Some(&path)).unwrap().0.theme.as_deref(),
        Some("gruvbox")
    );

    let config = Config::load(Some(&path)).unwrap().0;
    let (name, _) = config.startup_theme().unwrap();
    assert_eq!(name, "gruvbox");

    type_command(&mut app, "theme nonesuch");
    assert_eq!(
        Config::load(Some(&path)).unwrap().0.theme.as_deref(),
        Some("gruvbox")
    );

    fs::remove_dir_all(directory).unwrap();
}

/// Opening a binary file must not produce a buffer: a screenful of
/// replacement characters cannot be saved back without destroying it.
#[test]
fn opening_a_binary_file_asks_for_a_program_instead_of_a_buffer() {
    let directory = temporary("binary-open");
    fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("image.png");
    let text = directory.join("notes.txt");
    fs::write(&binary, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x00]).unwrap();
    fs::write(&text, "plain\n").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    // The cache is a person's, not a test's: no test may write to a real
    // home directory.
    app.programs = ProgramCache::load(Some(directory.join("cache")));
    let buffers = app.buffers.len();

    app.open_file(binary.clone()).unwrap();
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.prompt_kind, PromptKind::ExternalProgram);
    assert_eq!(app.external_target.as_deref(), Some(binary.as_path()));
    let choices = app.matching_program_choices();
    assert!(app.command.is_empty());
    assert!(choices[0].system && choices[0].is_default);
    assert_eq!(app.buffers.len(), buffers, "no buffer may be created");

    // A second binary file must not replace the question already asked.
    let other = directory.join("other.png");
    fs::write(&other, [0x00, 0x01]).unwrap();
    type_text(&mut app, "fe");
    app.open_file(other).unwrap();
    assert_eq!(app.external_target.as_deref(), Some(binary.as_path()));
    assert!(app.command.ends_with("fe"), "typed input is kept");
    assert!(app.status_error);

    // Esc abandons the file with the prompt.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.external_target, None);
    assert_eq!(app.buffers.len(), buffers);

    // A text file in the same directory still opens normally.
    app.open_file(text).unwrap();
    assert_eq!(app.buffers.len(), buffers + 1);
    assert_eq!(app.active_buffer().to_string(), "plain\n");

    fs::remove_dir_all(directory).unwrap();
}

/// The bounded probe is only an optimization. The bytes accepted by the
/// final read still decide whether the file can safely become editable text.
#[test]
fn binary_bytes_beyond_the_probe_still_use_the_external_program_prompt() {
    let directory = temporary("binary-beyond-probe");
    fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("late-invalid.bin");
    let mut contents = vec![b'a'; 8192];
    contents.push(0xff);
    fs::write(&binary, contents).unwrap();
    assert!(!external_open::looks_binary(&binary));

    let mut app = App::new(Config::default(), None).unwrap();
    app.programs = ProgramCache::load(Some(directory.join("cache")));
    let buffers = app.buffers.len();

    app.open_file(binary.clone()).unwrap();

    assert_eq!(app.prompt_kind, PromptKind::ExternalProgram);
    assert_eq!(app.external_target.as_deref(), Some(binary.as_path()));
    assert_eq!(app.buffers.len(), buffers);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_chosen_program_is_remembered_and_offered_back_as_a_hint() {
    let directory = temporary("binary-program-cache");
    fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("image.png");
    fs::write(&binary, [0x00, 0x01, 0x02, 0x03]).unwrap();
    // A program that exists everywhere and does nothing, so the test
    // spawns something real without depending on a viewer being installed.
    let program = "true";

    let mut app = App::new(Config::default(), None).unwrap();
    app.programs = ProgramCache::load(Some(directory.join("cache")));

    app.open_file(binary.clone()).unwrap();
    assert!(app.matching_programs().is_empty(), "nothing remembered yet");
    type_text(&mut app, program);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert!(!app.status_error, "{}", app.status);
    assert_eq!(app.external_target, None);
    assert_eq!(app.programs.programs(), [program]);

    // The next binary file selects the system opener and offers the
    // remembered choice after it.
    app.open_file(binary.clone()).unwrap();
    assert!(app.command.is_empty());
    assert_eq!(app.matching_programs(), [program]);
    let choices = app.matching_program_choices();
    assert!(choices[0].system && choices[0].is_default);
    assert_eq!(choices[1].program, program);

    // Tab manages the selected remembered program. Making it the default
    // moves it to the initial selection without opening the file.
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.program_action_menu.as_ref().unwrap().actions,
        [ProgramAction::Delete, ProgramAction::SetDefault]
    );
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.programs.default_program(), Some(program));
    assert_eq!(app.matching_program_choices()[0].program, program);

    // Both the remembered choice and its default status survive a restart.
    let reloaded = ProgramCache::load(Some(directory.join("cache")));
    assert_eq!(reloaded.programs(), [program]);
    assert_eq!(reloaded.default_program(), Some(program));

    // Enter opens the selected default row. On the next prompt, Tab can
    // delete it; deleting the custom default restores the system opener.
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    assert!(!app.status_error, "{}", app.status);
    app.open_file(binary).unwrap();
    assert_eq!(app.matching_program_choices()[0].program, program);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.program_action_menu.as_ref().unwrap().actions,
        [ProgramAction::Delete]
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.programs.programs().is_empty());
    assert_eq!(app.programs.default_program(), None);
    assert!(app.matching_program_choices()[0].system);

    let reloaded = ProgramCache::load(Some(directory.join("cache")));
    assert!(reloaded.programs().is_empty());
    assert_eq!(reloaded.default_program(), None);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_program_that_cannot_run_is_reported_and_not_remembered() {
    let directory = temporary("binary-bad-program");
    fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("image.png");
    fs::write(&binary, [0x00, 0x01]).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.programs = ProgramCache::load(Some(directory.join("cache")));

    app.open_file(binary).unwrap();
    app.command.clear();
    app.command_cursor = 0;
    type_text(&mut app, "runyte-no-such-program");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert!(app.programs.programs().is_empty());

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn command_hints_list_the_alias_that_matched() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, ':');
    type_text(&mut app, "sp");

    // `split` is an alias of `hsplit`, and it is what was typed, so it is
    // what the hint offers.
    let matches = app.matching_commands();
    assert_eq!(matches.first().map(|matched| matched.name), Some("split"));
    assert!(
        matches
            .iter()
            .any(|matched| matched.name == "service-health")
    );
    assert_eq!(matches[0].usage(), "split [path]");
    assert_eq!(matches[0].other_names(), ["hsplit"]);

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(app.command, "split ");

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.panes.len(), 2);
}

#[test]
fn goto_word_labels_visible_words_and_one_key_jumps_to_a_nearby_target() {
    use crate::jump_labels::LabelPart;

    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha beta\nx gamma");
    app.active_mut().wrap_width = 80;
    set_cursor(&mut app, 0, 0);

    press(&mut app, 'g');
    press(&mut app, 'w');
    let labels = app.jump.as_ref().unwrap();
    // `alpha`, `beta`, and `gamma` earn labels; the one-character `x` has
    // nowhere to put one.
    assert_eq!(labels.len(), 3);
    assert_eq!(labels.label_at(0), Some(('a', LabelPart::Immediate)));
    assert_eq!(labels.label_at(6), Some(('s', LabelPart::Immediate)));
    assert_eq!(labels.label_at(13), Some(('d', LabelPart::Immediate)));

    press(&mut app, 'd');
    assert!(app.jump.is_none());
    assert_eq!(cursor(&app), Position::new(1, 2));

    // A jump this long is worth remembering.
    key(&mut app, KeyCode::Char('o'), Modifiers::CONTROL);
    assert_eq!(cursor(&app), Position::new(0, 0));
}

#[test]
fn goto_word_gives_distant_targets_prefix_free_two_key_labels() {
    use crate::jump_labels::LabelPart;

    let mut app = App::new(Config::default(), None).unwrap();
    let line = std::iter::repeat_n("aa", 27).collect::<Vec<_>>().join(" ");
    seed(&mut app, &line);
    app.active_mut().wrap_width = 100;
    set_cursor(&mut app, 0, 39);

    press(&mut app, 'g');
    press(&mut app, 'w');
    let labels = app.jump.as_ref().unwrap();
    assert_eq!(labels.len(), 27);
    assert_eq!(labels.label_at(39), Some(('a', LabelPart::Immediate)));
    assert_eq!(labels.label_at(0), Some(('m', LabelPart::Prefix)));
    assert_eq!(labels.label_at(78), Some(('m', LabelPart::Prefix)));

    press(&mut app, 'm');
    let labels = app.jump.as_ref().unwrap();
    assert_eq!(labels.label_at(0), Some(('a', LabelPart::Immediate)));
    assert_eq!(labels.label_at(1), None);
    assert_eq!(labels.label_at(78), Some(('s', LabelPart::Immediate)));
    press(&mut app, 's');
    assert_eq!(cursor(&app), Position::new(0, 78));
}

#[test]
fn goto_word_keeps_a_fitting_one_key_target_at_the_right_edge() {
    use crate::jump_labels::LabelPart;

    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "xxxxx aa");
    app.active_mut().wrap_width = 7;

    press(&mut app, 'g');
    press(&mut app, 'w');
    let labels = app.jump.as_ref().unwrap();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels.label_at(6), Some(('s', LabelPart::Immediate)));
    assert_eq!(labels.label_at(7), None);
}

#[test]
fn goto_word_drops_a_two_key_target_that_crosses_the_right_edge() {
    let mut app = App::new(Config::default(), None).unwrap();
    let line = format!("{}aa", "aa ".repeat(26));
    seed(&mut app, &line);
    app.active_mut().wrap_width = 79;

    press(&mut app, 'g');
    press(&mut app, 'w');
    let labels = app.jump.as_ref().unwrap();
    assert_eq!(labels.len(), 26);
    assert_eq!(labels.label_at(78), None);
}

#[test]
fn goto_word_right_edge_is_measured_in_terminal_cells() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "界界 aa bb");
    app.active_mut().wrap_width = 7;

    press(&mut app, 'g');
    press(&mut app, 'w');
    let labels = app.jump.as_ref().unwrap();
    assert_eq!(labels.len(), 1);
    assert!(labels.label_at(3).is_some());
    assert_eq!(labels.label_at(6), None);
}

#[test]
fn goto_word_right_edge_accounts_for_tabs() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "\t\t aa bb");
    app.active_mut().wrap_width = 11;

    press(&mut app, 'g');
    press(&mut app, 'w');
    let labels = app.jump.as_ref().unwrap();
    assert_eq!(labels.len(), 1);
    assert!(labels.label_at(3).is_some());
    assert_eq!(labels.label_at(6), None);
}

#[test]
fn goto_word_excludes_words_past_a_horizontally_scrolled_view() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "zz zz aa bb cc");
    app.active_mut().scroll_col = 6;
    app.active_mut().wrap_width = 5;

    press(&mut app, 'g');
    press(&mut app, 'w');
    let labels = app.jump.as_ref().unwrap();
    assert_eq!(labels.len(), 2);
    assert!(labels.label_at(6).is_some());
    assert!(labels.label_at(9).is_some());
    assert_eq!(labels.label_at(12), None);
}

#[test]
fn goto_word_labels_are_spent_by_anything_that_is_not_a_label() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha beta");
    app.active_mut().wrap_width = 80;
    set_cursor(&mut app, 0, 0);

    press(&mut app, 'g');
    press(&mut app, 'w');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.jump.is_none());
    assert_eq!(cursor(&app), Position::default());

    press(&mut app, 'g');
    press(&mut app, 'w');
    // Only `a` and `s` are labels here, so `z` names nothing.
    press(&mut app, 'z');
    assert!(app.jump.is_none());
    assert!(app.status_error);
    assert_eq!(cursor(&app), Position::default());
}

#[test]
fn goto_word_extends_the_selection_in_select_mode() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha beta");
    app.active_mut().wrap_width = 80;
    set_cursor(&mut app, 0, 0);

    press(&mut app, 'v');
    press(&mut app, 'g');
    press(&mut app, 'w');
    press(&mut app, 's');
    let range = app.active().selection.primary();
    assert_eq!((range.anchor, range.head), (0, 6));
    assert_eq!(app.mode, Mode::Select);
}

#[test]
fn goto_word_leaves_double_width_words_unlabelled() {
    use crate::jump_labels::LabelPart;

    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "你好 hello");
    app.active_mut().wrap_width = 80;

    press(&mut app, 'g');
    press(&mut app, 'w');
    let labels = app.jump.as_ref().unwrap();
    // A label drawn over two double-width characters would occupy half the
    // cells they do and pull the rest of the row leftwards, so that word
    // goes unlabelled and `hello` takes the only label.
    assert_eq!(labels.len(), 1);
    assert_eq!(labels.label_at(3), Some(('a', LabelPart::Immediate)));
}

#[test]
fn goto_word_reports_when_there_is_nothing_to_label() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "a b c");
    app.active_mut().wrap_width = 80;

    press(&mut app, 'g');
    press(&mut app, 'w');
    assert!(app.jump.is_none());
    assert!(app.status_error);
}

#[test]
fn window_close_has_a_typed_spelling_after_close_becomes_buffer_local() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    assert_eq!(app.panes.len(), 2);

    press(&mut app, ':');
    type_text(&mut app, "wc");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.panes.len(), 1);
}

#[test]
fn leaving_the_editor_is_typed_rather_than_bound() {
    let mut app = App::new(Config::default(), None).unwrap();

    // Both V0 quit shortcuts are gone, so a stray keypress cannot end the
    // session.
    press(&mut app, ' ');
    press(&mut app, 'q');
    assert!(!app.should_quit);
    key(&mut app, KeyCode::Char('q'), Modifiers::CONTROL);
    assert!(!app.should_quit);

    press(&mut app, ':');
    type_text(&mut app, "q");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.should_quit);
}

#[test]
fn quit_closes_one_pane_while_quit_all_leaves_the_editor() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute_command("vsplit").unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved"));

    app.execute_command("q").unwrap();
    assert_eq!(app.panes.len(), 1);
    assert!(!app.should_quit);
    assert!(app.buffers[buffer].dirty, "closing a view lost its buffer");

    app.execute_command("qa").unwrap();
    assert!(!app.should_quit);
    assert!(app.status.contains(":qa!"), "{}", app.status);
    app.execute_command("qa!").unwrap();
    assert!(app.should_quit);
}

#[test]
fn detach_is_persistent_only_and_preserves_dirty_editor_state() {
    let mut standalone = App::new(Config::default(), None).unwrap();
    standalone.execute_command("detach").unwrap();
    assert!(!standalone.should_quit);
    assert_eq!(
        standalone.status,
        ":detach is available only in persistent mode"
    );

    let mut persistent = App::new(Config::default(), None).unwrap();
    persistent.execute_command("vsplit").unwrap();
    let buffer = persistent.active().buffer;
    persistent.buffers[buffer].apply(&Transaction::insert(0, "unsaved"));
    persistent.enable_persistent_session();

    persistent.execute_command("detach").unwrap();

    assert!(persistent.should_quit);
    assert_eq!(persistent.panes.len(), 2);
    assert!(persistent.buffers[buffer].dirty);
    assert_eq!(persistent.buffers[buffer].text().to_string(), "unsaved");
    assert_eq!(
        persistent.take_persistent_exit_request(),
        Some(PersistentExitRequest::Detach)
    );
}

#[test]
fn persistent_quit_requests_host_shutdown_and_retains_force_intent() {
    let mut safe = App::new(Config::default(), None).unwrap();
    safe.enable_persistent_session();
    safe.execute_command("q").unwrap();
    assert_eq!(
        safe.take_persistent_exit_request(),
        Some(PersistentExitRequest::Quit { force: false })
    );

    let mut forced = App::new(Config::default(), None).unwrap();
    forced.enable_persistent_session();
    let buffer = forced.active().buffer;
    forced.buffers[buffer].apply(&Transaction::insert(0, "unsaved"));
    forced.execute_command("q!").unwrap();
    assert_eq!(
        forced.take_persistent_exit_request(),
        Some(PersistentExitRequest::Quit { force: true })
    );
}

#[test]
fn control_q_does_nothing_inside_the_command_prompt() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, ':');
    type_text(&mut app, "open note");

    key(&mut app, KeyCode::Char('q'), Modifiers::CONTROL);

    assert!(!app.should_quit);
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.command, "open note");
}

#[test]
fn quit_here_uses_the_active_file_directory_and_preserves_quit_safety() {
    let root = temporary("quit-here-file");
    let file_directory = root.join("files");
    let working = root.join("working");
    fs::create_dir_all(&file_directory).unwrap();
    fs::create_dir_all(&working).unwrap();
    let file = file_directory.join("note.txt");
    fs::write(&file, "saved").unwrap();

    let mut app = App::new(Config::default(), Some(file)).unwrap();
    app.enable_quit_directory_handoff();
    app.working_directory = working.clone();
    press(&mut app, 'i');
    press(&mut app, 'x');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    type_command(&mut app, "qh");
    assert!(!app.should_quit);
    assert_eq!(app.working_directory, working);
    assert!(app.quit_directory().is_none());
    assert!(app.status.contains(":qh!"));

    type_command(&mut app, "qh!");
    assert!(app.should_quit);
    assert_eq!(app.working_directory, file_directory);
    assert_eq!(app.quit_directory(), Some(file_directory.as_path()));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_here_uses_the_last_directory_shown_by_the_active_explorer() {
    let root = temporary("quit-here-explorer");
    let visited = root.join("visited");
    fs::create_dir_all(&visited).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.enable_quit_directory_handoff();
    let scratch = app.active().buffer;
    app.open_file(root.clone()).unwrap();
    app.open_file(visited.clone()).unwrap();
    app.active_mut().buffer = scratch;
    type_command(&mut app, "quit-here");

    assert!(app.should_quit);
    assert_eq!(app.working_directory, visited);
    assert_eq!(app.quit_directory(), Some(visited.as_path()));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quit_here_refuses_to_degrade_to_plain_quit_without_a_shell_handoff() {
    let mut app = App::new(Config::default(), None).unwrap();

    type_command(&mut app, "qh");

    assert!(!app.should_quit);
    assert!(app.quit_directory().is_none());
    assert!(app.status_error);
    assert!(app.status.contains("runyte()"));
    assert!(app.status.contains("README.md"));
    assert!(!app.status.contains("--cwd-file"));
}

#[test]
fn quit_here_refuses_to_exit_when_the_destination_no_longer_exists() {
    let root = temporary("quit-here-missing");
    let file = root.join("note.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&file, "saved").unwrap();
    let mut app = App::new(Config::default(), Some(file)).unwrap();
    app.enable_quit_directory_handoff();
    fs::remove_dir_all(&root).unwrap();

    type_command(&mut app, "qh");

    assert!(!app.should_quit);
    assert!(app.quit_directory().is_none());
    assert!(app.status_error);
    assert!(app.status.contains("cannot quit here"));
}

#[test]
fn command_hints_keep_canonical_names_when_they_match() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, ':');
    type_text(&mut app, "hsp");

    let matches = app.matching_commands();
    assert_eq!(
        matches
            .iter()
            .map(|matched| matched.name)
            .collect::<Vec<_>>(),
        ["hsplit"]
    );
    assert_eq!(matches[0].usage(), "hsplit [path]");
    assert_eq!(matches[0].other_names(), ["split"]);
}

#[test]
fn applied_explorer_moves_retarget_files_and_refresh_other_explorers() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = temporary_directory().join(format!(
        "runyte-explorer-reconcile-{}-{unique}",
        std::process::id()
    ));
    let child = directory.join("child");
    fs::create_dir_all(&child).unwrap();
    let old = child.join("old.txt");
    let new = child.join("new.txt");
    fs::write(&old, "contents\n").unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = directory.clone();
    app.open_file(old.clone()).unwrap();
    let file_buffer = app.active().buffer;
    app.open_file(child.clone()).unwrap();
    let child_buffer = app.active().buffer;
    app.split(Axis::Vertical, Some(directory.clone())).unwrap();
    let root_buffer = app.active().buffer;
    assert_ne!(
        child_buffer, root_buffer,
        "each pane must own the explorer whose reconciliation is being tested"
    );

    fs::rename(&old, &new).unwrap();
    let report = ApplyReport {
        applied: vec![FsOperation::Rename {
            from: PathBuf::from("child/old.txt"),
            to: PathBuf::from("child/new.txt"),
            kind: EntryKind::File,
        }],
    };
    assert_eq!(
        app.reconcile_applied_filesystem(&directory, root_buffer, &report, true),
        None
    );
    assert_eq!(
        app.buffers[file_buffer].path.as_deref(),
        Some(new.as_path())
    );
    assert!(app.buffers[child_buffer].to_string().contains("new.txt"));
    assert!(!app.buffers[child_buffer].to_string().contains("old.txt"));
    assert!(!app.buffers[child_buffer].dirty);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn registry_dispatches_arbitrary_sequences_and_reports_invalid_keys() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "first\nsecond");
    set_cursor(&mut app, 1, 4);

    press(&mut app, 'g');
    assert_eq!(app.pending_sequence().to_string(), "g");
    press(&mut app, 'h');
    assert_eq!(cursor(&app), Position::new(1, 0));
    assert!(app.pending_sequence().is_empty());

    press(&mut app, ' ');
    press(&mut app, 'w');
    assert_eq!(app.pending_sequence().to_string(), "Space w");
    press(&mut app, 'v');
    assert_eq!(app.panes.len(), 2);

    press(&mut app, 'g');
    press(&mut app, 'z');
    assert!(app.status_error);
    assert_eq!(app.status, "No binding: g z");
    assert!(app.pending_sequence().is_empty());
    // Already visible above through `status`/`status_error`, and in the
    // key hints, which read the grammar notice directly. A burst of
    // mistyping must not also grow the notification count.
    assert_eq!(
        app.unread_notification_counts(),
        NotificationCounts::default()
    );
}

#[test]
fn completed_key_bindings_report_the_typed_sequence_and_action() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "first line\n");

    press(&mut app, 'g');
    assert_eq!(app.status, "g …");
    press(&mut app, 'l');
    assert_eq!(app.displayed_status_message(), "g l (Move to line end)");

    set_cursor(&mut app, 0, 0);
    press(&mut app, 'f');
    press(&mut app, 'l');
    assert_eq!(app.displayed_status_message(), "f l (Find next character)");

    set_cursor(&mut app, 0, 0);
    press(&mut app, '3');
    press(&mut app, 'l');
    assert_eq!(app.displayed_status_message(), "3 l (Move right)");
}

#[test]
fn completed_actions_keep_specific_success_details() {
    let mut app = App::new(Config::default(), None).unwrap();

    press(&mut app, ' ');
    press(&mut app, 'e');
    assert!(
        app.displayed_status_message()
            .starts_with("Space e (opened "),
        "{}",
        app.displayed_status_message()
    );
    assert!(app.displayed_status_message().ends_with(')'));

    let closed_name = app.active_buffer().display_name();
    type_command(&mut app, "bc");
    assert_eq!(
        app.displayed_status_message(),
        format!(":bc (closed {closed_name})")
    );
}

#[test]
fn counted_colon_binding_echoes_failure_and_retains_its_info_notification() {
    let mut app = App::new(Config::default(), None).unwrap();

    press(&mut app, '2');
    press(&mut app, ' ');
    press(&mut app, 'r');

    assert!(app.status_error);
    assert_eq!(
        app.status,
        "Reload the active view does not support a count"
    );
    assert_eq!(
        app.displayed_status_message(),
        "2 Space r (Reload the active view · failed: Reload the active view does not \
             support a count)"
    );
    assert!(app.displayed_status_message_is_error());
    assert_eq!(app.unread_notification_counts().infos, 1);
}

#[test]
fn failed_action_echoes_its_message_inline_in_full() {
    // Composition never truncates: src/ui.rs::clip_interaction_line
    // truncates against the render frame's actual width instead, since
    // the composed text here has no width to measure against.
    let mut app = App::new(Config::default(), None).unwrap();
    app.report_completed_action(
        "p",
        "Paste after the selection",
        CommandOutcome::UserError("Cannot paste into a read-only buffer".to_owned()),
    );
    assert_eq!(
        app.displayed_status_message(),
        "p (Paste after the selection · failed: Cannot paste into a read-only buffer)"
    );
    assert!(app.displayed_status_message_is_error());
}

#[test]
fn unavailable_action_echoes_its_message_inline_and_is_not_styled_as_an_error() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.report_completed_action(
        "Space l s",
        "Report language server state",
        CommandOutcome::Unavailable("language servers are not running".to_owned()),
    );
    assert_eq!(
        app.displayed_status_message(),
        "Space l s (Report language server state · unavailable: language servers are not \
             running)"
    );
    assert!(!app.displayed_status_message_is_error());
}

#[test]
fn unsupported_key_binding_echoes_its_message_inline() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, '|');
    assert_eq!(
        app.displayed_status_message(),
        "| (Pipe the selection through a shell command · unavailable: Pipe the selection \
             through a shell command is unsupported: shell pipes are not available)"
    );
    assert!(!app.displayed_status_message_is_error());
}

#[test]
fn unavailable_colon_command_stays_typed_and_leaves_the_prior_echo_alone() {
    // command_capabilities().command_availability() marks `:lsp-status`
    // unavailable before it ever reaches execute()'s hint-based check,
    // since no language-server manager is attached in a fresh App. This
    // is the one deliberate exception documented in the resolved issue
    // file: the palette owns the interaction line until it closes, so
    // reporting here would mean closing it out from under whoever is
    // still typing. The reason stays retained-only in `:not` instead,
    // and the prior echo (here, none yet) is left exactly as it was.
    let mut app = App::new(Config::default(), None).unwrap();
    type_command(&mut app, "lsp-status");
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.command, "lsp-status");
    assert_eq!(app.displayed_status_message(), "");
    assert!(app.status.contains("unavailable"));
    assert_eq!(app.unread_notification_counts().infos, 1);
}
