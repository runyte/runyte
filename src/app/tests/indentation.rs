// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{config::IndentStyle, indentation::Scope, test_support::TestRuntimeRoot};

fn editor(source: &str) -> (App, TestRuntimeRoot, PathBuf) {
    let root = TestRuntimeRoot::new("indent").unwrap();
    let path = root.path().join("config.yaml");
    fs::write(&path, source).unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new_in_project(config, None, root.path()).unwrap();
    app.note_loaded_config(&path);
    (app, root, path)
}

#[test]
fn backspace_aligns_leading_whitespace_and_preserves_ordinary_deletion() {
    for (before, column, expected, after) in [
        ("        text", 8, "    text", 4),
        ("      text", 6, "    text", 4),
        ("    text", 4, "text", 0),
        ("  text", 2, "text", 0),
        ("  \t  text", 5, "  \ttext", 3),
        (" \ttext", 2, "text", 0),
        ("a    text", 5, "a   text", 4),
        ("    ", 4, "", 0),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, before);
        app.mode = Mode::Insert;
        set_cursor(&mut app, 0, column);
        key(&mut app, KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(text(&app), expected, "{before:?}");
        assert_eq!(cursor(&app).col, after);
    }
}

#[test]
fn alt_backspace_clears_indentation_without_leaving_the_line() {
    for ending in ["\n", "\r\n"] {
        for (line, column, expected) in [
            ("        text", 8, "text"),
            ("\t\ttext", 2, "text"),
            (" \t  text", 4, "text"),
            ("        ", 8, ""),
            ("    text", 2, "  text"),
        ] {
            let mut app = App::new(Config::default(), None).unwrap();
            let before = format!("α previous{ending}{ending}{line}{ending}next");
            seed(&mut app, &before);
            app.mode = Mode::Insert;
            set_cursor(&mut app, 2, column);

            key(&mut app, KeyCode::Backspace, Modifiers::ALT);

            assert_eq!(
                text(&app),
                format!("α previous{ending}{ending}{expected}{ending}next")
            );
            assert_eq!(cursor(&app), Position::new(2, 0));
            key(&mut app, KeyCode::Escape, Modifiers::NONE);
            press(&mut app, 'u');
            assert_eq!(text(&app), before);
        }
    }
}

#[test]
fn alt_backspace_preserves_word_deletion_and_explicit_line_joining() {
    for (before, row, column, expected, after) in [
        ("    βeta!  ", 0, 11, "    βeta", Position::new(0, 8)),
        ("    βeta  ", 0, 10, "    ", Position::new(0, 4)),
        ("    text", 0, 4, "text", Position::new(0, 0)),
        ("α beta\ntext", 1, 0, "α text", Position::new(0, 2)),
        ("α beta\r\ntext", 1, 0, "α text", Position::new(0, 2)),
        ("text", 0, 0, "text", Position::new(0, 0)),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, before);
        app.mode = Mode::Insert;
        set_cursor(&mut app, row, column);
        key(&mut app, KeyCode::Backspace, Modifiers::ALT);
        assert_eq!(text(&app), expected, "{before:?}");
        assert_eq!(cursor(&app), after);
    }
}

#[test]
fn alt_backspace_merges_indentation_deletions_and_undo_restores_indentation() {
    let mut app = App::new(Config::default(), None).unwrap();
    let before = "prior\r\n      a\r\n\t\tb";
    seed(&mut app, before);
    app.mode = Mode::Insert;
    app.replace_active_selection(Selection::new(
        vec![Range::point(11), Range::point(13), Range::point(18)],
        0,
    ));

    key(&mut app, KeyCode::Backspace, Modifiers::ALT);

    assert_eq!(text(&app), "prior\r\na\r\nb");
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::point(7), Range::point(10)]
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'u');
    assert_eq!(text(&app), before);
}

#[test]
fn backspace_merges_overlapping_carets_and_undo_restores_indentation() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "      a\r\n    b");
    app.mode = Mode::Insert;
    app.replace_active_selection(Selection::new(
        vec![Range::point(5), Range::point(6), Range::point(13)],
        0,
    ));
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(text(&app), "    a\r\nb");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'u');
    assert_eq!(text(&app), "      a\r\n    b");
}

#[test]
fn language_and_path_widths_reach_tab_backspace_and_reload() {
    let source = "editor:\n  tab_width: 4\nindentation:\n  languages:\n    yaml:\n      tab_width: 2\n  files:\n    '*.special.yaml':\n      tab_width: 8\n";
    let (mut app, root, path) = editor(source);
    app.buffers[0].path = Some(root.path().join("a.yaml"));
    seed(&mut app, "   value");
    app.mode = Mode::Insert;
    set_cursor(&mut app, 0, 3);
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(text(&app), "  value");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(text(&app), "    value");
    app.buffers[0].path = Some(root.path().join("a.special.yaml"));
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(text(&app), "        value");
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(text(&app), "value");
    fs::write(&path, source.replace("tab_width: 8", "tab_width: 3")).unwrap();
    app.mode = Mode::Normal;
    app.execute_command("config-reload").unwrap();
    app.mode = Mode::Insert;
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(text(&app), "   value");
}

fn select_setting(app: &mut App, setting: SettingId, scoped: bool) {
    let row = (0..app.active_buffer().len_lines())
        .find(|row| {
            app.active_buffer().setting_at(*row) == Some(setting)
                && app.active_buffer().setting_override_at(*row).is_some() == scoped
        })
        .unwrap();
    set_cursor(app, row, 0);
}

fn choose_label(app: &mut App, label: &str) {
    if let Some(menu) = &app.context_action_menu {
        let mnemonic = menu
            .actions
            .iter()
            .find(|action| action.description == label)
            .unwrap()
            .mnemonic;
        app.handle_key(mnemonic).unwrap();
        return;
    }
    let picker = app.list.as_mut().expect("picker open");
    picker.selected = picker
        .items
        .iter()
        .position(|item| item.label == label)
        .unwrap();
    key(app, KeyCode::Enter, Modifiers::NONE);
}

#[test]
fn settings_keys_create_edit_and_remove_an_explicit_language_override() {
    let (mut app, _root, path) = editor("editor:\n  tab_width: 4\n");
    app.open_settings_buffer();
    select_setting(&mut app, SettingId::EditorTabWidth, false);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    choose_label(&mut app, "Override for language…");
    choose_label(&mut app, "python");
    assert_eq!(app.command, "4");
    assert!(app.overlay_snapshots().iter().any(|overlay| {
        overlay
            .message
            .as_deref()
            .is_some_and(|s| s.contains("Inherited: 4 from editor.tab_width"))
    }));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.config.indentation.languages["python"].tab_width,
        Some(4)
    );
    assert!(text(&app).contains("language: python"));
    select_setting(&mut app, SettingId::EditorTabWidth, true);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.command = "2".into();
    app.command_cursor = 1;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.config.indentation.languages["python"].tab_width,
        Some(2)
    );
    select_setting(&mut app, SettingId::EditorTabWidth, true);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    choose_label(&mut app, "Remove override");
    assert!(!text(&app).contains("language: python"));
    assert_eq!(
        Config::load(Some(&path)).unwrap().0.indentation.languages["python"].tab_width,
        None
    );
}

#[test]
fn file_pattern_flow_validates_cancels_and_saves_without_losing_target() {
    let (mut app, _root, path) = editor("# configuration\neditor:\n  tab_width: 4\n");
    app.open_settings_buffer();
    select_setting(&mut app, SettingId::EditorTabWidth, false);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    choose_label(&mut app, "Override for file pattern…");
    app.command = "../oops".into();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert!(matches!(app.prompt_kind, PromptKind::IndentationPattern(_)));
    app.command = "**/*.yaml".into();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.indentation_prompt_scope,
        Some(Scope::Files("**/*.yaml".into()))
    );
    app.command = "0".into();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    app.command = "2".into();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.config.indentation.files[0].values.tab_width, Some(2));
    select_setting(&mut app, SettingId::EditorTabWidth, true);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.command = "8".into();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.indentation_prompt_scope.is_none());
    assert_eq!(
        Config::load(Some(&path)).unwrap().0.indentation.files[0]
            .values
            .tab_width,
        Some(2)
    );
}

#[test]
fn scoped_style_preview_rolls_back_on_cancel_and_failed_save() {
    let (mut app, _root, path) = editor("editor:\n  indent: spaces\n");
    app.open_settings_buffer();
    let scope = Scope::Language("python".into());
    app.open_override_value(SettingId::EditorIndent, scope.clone());
    let index = app
        .list
        .as_ref()
        .unwrap()
        .items
        .iter()
        .position(|item| item.label == "tabs")
        .unwrap();
    app.list.as_mut().unwrap().selected = index;
    app.preview_selected_setting_value();
    assert_eq!(
        app.config.indentation.languages["python"].indent,
        Some(IndentStyle::Tabs)
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.config.indentation.languages.is_empty());
    app.open_override_value(SettingId::EditorIndent, scope.clone());
    app.list.as_mut().unwrap().selected = index;
    app.preview_selected_setting_value();
    fs::write(&path, "indentation: {languages: {}}\n").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.list.is_some());
    assert!(app.config.indentation.languages.is_empty());
    fs::write(&path, "# fixed\n").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.config.indentation.languages["python"].indent,
        Some(IndentStyle::Tabs)
    );
    assert!(app.list.is_none());
}

#[test]
fn pattern_prompt_uses_the_origin_documents_inherited_value_and_source() {
    let (mut app, root, _) = editor("indentation:\n  languages:\n    yaml:\n      tab_width: 2\n");
    app.buffers[0].path = Some(root.path().join("test.yaml"));
    app.open_settings_buffer();
    app.open_override_value(SettingId::EditorTabWidth, Scope::Files("*.yaml".into()));
    assert_eq!(app.command, "2");
    assert!(
        app.override_inherited_label(SettingId::EditorTabWidth, &Scope::Files("*.yaml".into()))
            .contains("2 from language: yaml")
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.open_override_value(SettingId::EditorTabWidth, Scope::Files("*.py".into()));
    assert_eq!(app.command, "4");
    assert!(
        app.override_inherited_label(SettingId::EditorTabWidth, &Scope::Files("*.py".into()))
            .contains("varies by file")
    );
}

#[test]
fn simultaneous_panes_render_and_hit_test_their_own_tab_widths() {
    let (mut app, root, path) = editor(
        "editor:\n  line_numbers: false\nindentation:\n  languages:\n    python:\n      tab_width: 4\n    yaml:\n      tab_width: 2\n",
    );
    seed(&mut app, "\tx");
    app.buffers[0].path = Some(root.path().join("a.py"));
    app.split(Axis::Horizontal, None).unwrap();
    let right = app.active_pane;
    let mut second = Buffer::scratch();
    second.apply(&Transaction::insert(0, "\tx"));
    second.path = Some(root.path().join("a.yaml"));
    let id = app.buffers.len();
    app.buffers.push(second);
    app.syntax.push(None);
    app.active_mut().retarget(id);
    let geometry = FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 90,
            height: 20,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 90,
            height: 18,
        },
        status: Rect {
            x: 0,
            y: 18,
            width: 90,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 19,
            width: 90,
            height: 1,
        },
    };
    let view = app.prepare_view(geometry);
    let snapshot = app.snapshot(&view);
    for (pane_id, width) in [(0, 4), (right, 2)] {
        let pane = view.pane(pane_id).unwrap();
        let x = pane.body.x
            + (pane.gutter_width + pane.content_indent + pane.row_prefix_width + width) as u16;
        assert_eq!(
            app.pointer_text_offset(&view, pane_id, x, pane.body.y),
            Some(1)
        );
        let crate::snapshot::SnapshotRow::Text(row) = &snapshot.pane(pane_id).unwrap().rows[0]
        else {
            panic!("text row");
        };
        let rendered: String = row.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(
            rendered.find('x'),
            Some(width),
            "pane {pane_id}: {rendered:?}"
        );
    }
    // Changing an inactive pane's width must invalidate its cached snapshot.
    let source = fs::read_to_string(&path)
        .unwrap()
        .replace("tab_width: 4", "tab_width: 6");
    fs::write(&path, source).unwrap();
    app.execute_command("config-reload").unwrap();
    let view = app.prepare_view(geometry);
    let snapshot = app.snapshot(&view);
    let crate::snapshot::SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
        panic!("text row");
    };
    assert_eq!(
        row.runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>()
            .find('x'),
        Some(6)
    );
}

#[test]
fn numeric_override_save_failure_keeps_input_and_can_retry() {
    let (mut app, _root, path) = editor("editor:\n  tab_width: 4\n");
    app.open_settings_buffer();
    app.open_override_value(SettingId::EditorTabWidth, Scope::Language("yaml".into()));
    app.command = "2".into();
    fs::write(&path, "indentation: {languages: {}}\n").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.command, "2");
    assert!(matches!(app.prompt_kind, PromptKind::IndentationValue(_)));
    assert!(app.config.indentation.languages.is_empty());
    fs::write(&path, "# repaired\n").unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.config.indentation.languages["yaml"].tab_width, Some(2));
}

#[test]
fn document_comparison_snapshots_inherit_source_widths_without_local_paths() {
    use crate::buffer::{GeneratedViewIdentity as Identity, ProviderDocument, ProviderIdentity};
    let (mut app, root, _) = editor(
        "indentation:\n  languages:\n    yaml:\n      tab_width: 2\n  files:\n    'special.yaml':\n      tab_width: 6\n",
    );
    app.buffers[0].path = Some(root.path().join("special.yaml"));
    seed(&mut app, "\tx");
    let provider = Buffer::provider_document(
        ProviderDocument {
            identity: ProviderIdentity {
                configured_plugin: "test".into(),
                provider: "files".into(),
                key: "special.yaml".into(),
            },
            label: "special.yaml".into(),
            syntax_hint: Some("yaml".into()),
            version: "v1".into(),
            generation: "g1".into(),
            available: true,
            baseline_epoch: 0,
            uncertain: None,
        },
        "\tx".into(),
    );
    let provider_id = app.buffers.len();
    app.buffers.push(provider);
    app.syntax.push(None);
    let revision_file = crate::git::RevisionFile {
        left: Some("special.yaml".into()),
        right: Some("ordinary.yaml".into()),
        left_object: String::new(),
        right_object: String::new(),
        left_mode: "100644".into(),
        right_mode: "100644".into(),
        stats: None,
    };
    let cases = [
        (
            Identity::DiskSnapshot {
                source_buffer: 0,
                revision: "disk".into(),
            },
            6,
        ),
        (
            Identity::ProviderSnapshot {
                source_buffer: provider_id,
                generation: "g1".into(),
                version: "v1".into(),
            },
            2,
        ),
        (
            Identity::GitDiffSide {
                path: root.path().join("special.yaml"),
                scope: "staged".into(),
                previous: true,
            },
            6,
        ),
        (
            Identity::GitRevisionFile {
                repository: root.path().into(),
                left: "a".into(),
                right: "b".into(),
                file: revision_file.clone(),
                side: Some(true),
            },
            6,
        ),
        (
            Identity::GitRevisionFile {
                repository: root.path().into(),
                left: "a".into(),
                right: "b".into(),
                file: revision_file,
                side: Some(false),
            },
            2,
        ),
    ];
    for (identity, width) in cases {
        let id = app.buffers.len();
        app.buffers.push(Buffer::virtual_text_identified(
            identity,
            "[snapshot]",
            "\tx",
        ));
        app.syntax.push(None);
        app.active_mut().retarget(id);
        assert!(app.active_buffer().path.is_none());
        assert!(app.active_buffer().is_read_only());
        assert_eq!(app.indentation_for(id).tab_width, width);
        let geometry = FrameGeometry {
            screen: Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 20,
            },
            editor: Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 18,
            },
            status: Rect {
                x: 0,
                y: 18,
                width: 80,
                height: 1,
            },
            message: Rect {
                x: 0,
                y: 19,
                width: 80,
                height: 1,
            },
        };
        let view = app.prepare_view(geometry);
        let snapshot = app.snapshot(&view);
        let crate::snapshot::SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("text row");
        };
        assert_eq!(
            row.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .find('x'),
            Some(width)
        );
    }
}

#[test]
fn failed_override_removal_retains_the_row_and_explains_the_retry_action() {
    let original = "indentation:\n  languages:\n    yaml:\n      tab_width: 2\n";
    let (mut app, _root, path) = editor(original);
    app.open_settings_buffer();
    select_setting(&mut app, SettingId::EditorTabWidth, true);
    fs::write(&path, "indentation: {languages: {yaml: {tab_width: 2}}}\n").unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    choose_label(&mut app, "Remove override");
    assert_eq!(app.config.indentation.languages["yaml"].tab_width, Some(2));
    assert!(app.status.contains("Tab → Remove override retries"));
    assert!(app.context_action_menu.is_none());
    assert!(text(&app).contains("language: yaml"));
    fs::write(&path, original).unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    choose_label(&mut app, "Remove override");
    assert!(!text(&app).contains("language: yaml"));
    assert_eq!(app.config.indentation.languages["yaml"].tab_width, None);
}
