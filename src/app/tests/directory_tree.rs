// SPDX-License-Identifier: MPL-2.0

use super::*;

fn isolated(label: &str) -> (App, PathBuf) {
    let root = temporary(label);
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    (app, root)
}

fn geometry(width: u16, height: u16) -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        status: Rect::default(),
        message: Rect::default(),
    }
}

fn chord(app: &mut App, suffix: char) {
    for character in [' ', 'd', suffix] {
        key(app, KeyCode::Char(character), Modifiers::NONE);
    }
}

#[test]
fn toggle_reserves_sidebar_without_creating_a_pane_or_changing_split_ratio() {
    let (mut app, root) = isolated("directory-tree-layout");
    app.execute(CommandInvocation::split_vertical(None))
        .unwrap();
    let layout = app.layout.clone();
    let panes = app.panes.keys().copied().collect::<HashSet<_>>();
    let before = app.prepare_view(geometry(90, 20));
    chord(&mut app, 't');
    let after = app.prepare_view(geometry(90, 20));
    assert_eq!(after.tree_area.unwrap().width, 33);
    assert_eq!(format!("{:#?}", app.layout), format!("{layout:#?}"));
    assert_eq!(app.panes.keys().copied().collect::<HashSet<_>>(), panes);
    assert_eq!(after.panes.len(), before.panes.len());
    assert!(after.panes.iter().all(|pane| pane.area.x >= 33));
    let snapshot = app.snapshot(&after);
    let wire: crate::protocol::EditorSnapshot = snapshot.clone().into();
    assert_eq!(
        crate::snapshot::EditorSnapshot::try_from(wire).unwrap(),
        snapshot
    );
    chord(&mut app, 't');
    let restored = app.prepare_view(geometry(90, 20));
    assert_eq!(restored.tree_area, None);
    assert_eq!(
        restored
            .panes
            .iter()
            .map(|pane| pane.area)
            .collect::<Vec<_>>(),
        before
            .panes
            .iter()
            .map(|pane| pane.area)
            .collect::<Vec<_>>()
    );
    chord(&mut app, 't');
    assert!(app.prepare_view(geometry(0, 0)).tree_area.is_none());
    assert!(app.prepare_view(geometry(35, 10)).tree_area.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reveal_open_and_escape_keep_the_last_ordinary_pane() {
    let (mut app, root) = isolated("directory-tree-open");
    let path = root.join("file.txt");
    fs::write(&path, "hello\n").unwrap();
    app.open_file(path.clone()).unwrap();
    app.execute(CommandInvocation::split_vertical(None))
        .unwrap();
    let destination = app.active_pane;
    let original_buffers = app.buffers.len();
    chord(&mut app, 'd');
    assert!(app.directory_tree.focused);
    assert_eq!(app.directory_tree.selected, path);
    assert_eq!(app.active_pane, destination);
    assert_eq!(app.buffers.len(), original_buffers);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(!app.directory_tree.focused);
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.directory_tree_destination.is_some());
    let number = app
        .directory_tree_panes()
        .iter()
        .position(|id| *id == destination)
        .unwrap()
        + 1;
    key(
        &mut app,
        KeyCode::Char(char::from_digit(number as u32, 10).unwrap()),
        Modifiers::NONE,
    );
    assert!(!app.directory_tree.focused);
    assert_eq!(app.active_pane, destination);
    assert_eq!(app.active_buffer().path.as_deref(), Some(path.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_actions_apply_and_tab_toggles_the_legend() {
    let (mut app, root) = isolated("directory-tree-actions");
    chord(&mut app, 'd');
    assert!(app.directory_tree.legend_visible);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(!app.directory_tree.legend_visible);
    assert!(app.context_action_menu.is_none());
    key(&mut app, KeyCode::Char('n'), Modifiers::NONE);
    app.handle_input(InputEvent::Text("new.txt".into()))
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(root.join("new.txt").is_file());
    assert!(app.fs_confirmation.is_none());
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.directory_tree.legend_visible);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_tree_operations_do_not_protect_a_clean_workspace_from_quit() {
    let (mut app, root) = isolated("directory-tree-quit");
    chord(&mut app, 'd');
    tree_prompt(&mut app, 'n', "created.txt");
    app.execute_command("quit-all").unwrap();
    assert!(app.should_quit);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn character_taking_editor_commands_cannot_edit_covered_file() {
    let (mut app, root) = isolated("directory-tree-character-command");
    let path = root.join("file.txt");
    fs::write(&path, "before\n").unwrap();
    app.open_file(path.clone()).unwrap();
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Char('r'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('X'), Modifiers::NONE);
    assert!(app.directory_tree.focused);
    assert_eq!(app.active_buffer().text().to_string(), "before\n");
    assert_eq!(fs::read_to_string(path).unwrap(), "before\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_pointer_frame_uses_the_rows_that_were_drawn() {
    let (mut app, root) = isolated("directory-tree-pointer-frame");
    chord(&mut app, 'd');
    fs::write(root.join("a"), "a").unwrap();
    fs::write(root.join("b"), "b").unwrap();
    app.directory_tree.refresh(root.clone());
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.directory_tree.rows().len() < 3 {
        assert!(Instant::now() < deadline);
        app.directory_tree.poll();
        std::thread::yield_now();
    }
    let view = app.prepare_view(geometry(80, 20));
    assert_eq!(view.tree_rows.get(1), Some(&root.join("a")));
    app.directory_tree.scroll = 1;
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: 1,
            row: 2,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.directory_tree.selected, root.join("a"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_help_returns_focus_to_the_ordinary_pane() {
    let (mut app, root) = isolated("directory-tree-help");
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    key(&mut app, KeyCode::Char('?'), Modifiers::NONE);
    assert!(!app.directory_tree.focused);
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .contains("DIRECTORY TREE")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_help_from_an_insert_pane_opens_read_only_help_in_normal_mode() {
    let (mut app, root) = isolated("directory-tree-help-insert");
    app.mode = Mode::Insert;
    app.handle_directory_tree_command(EditorCommand::FocusDirectoryTree)
        .unwrap();
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    key(&mut app, KeyCode::Char('?'), Modifiers::NONE);
    assert!(!app.directory_tree.focused);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.active_buffer().is_read_only());
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .contains("DIRECTORY TREE")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fullscreen_from_tree_focus_maximizes_the_ordinary_pane() {
    let (mut app, root) = isolated("directory-tree-maximize");
    app.execute(CommandInvocation::split_vertical(None))
        .unwrap();
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    key(&mut app, KeyCode::Char('w'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('f'), Modifiers::NONE);
    assert!(!app.directory_tree.focused);
    assert!(app.maximized.is_some());
    assert!(app.prepare_view(geometry(80, 20)).tree_area.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn narrow_view_returns_the_mode_saved_before_tree_focus() {
    let (mut app, root) = isolated("directory-tree-narrow-mode");
    app.mode = Mode::Insert;
    app.handle_directory_tree_command(EditorCommand::FocusDirectoryTree)
        .unwrap();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.directory_tree.focused);
    app.prepare_view(geometry(35, 20));
    assert!(!app.directory_tree.focused);
    assert_eq!(app.mode, Mode::Insert);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn narrowing_during_a_tree_prompt_keeps_the_prompt_usable() {
    let (mut app, root) = isolated("directory-tree-prompt-resize");
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('n'), Modifiers::NONE);
    assert_eq!(
        app.prompt_kind,
        PromptKind::DirectoryTreeAction(TreePromptAction::New)
    );
    app.prepare_view(geometry(35, 20));
    assert_eq!(app.mode, Mode::Command);
    assert!(!app.directory_tree.focused);
    app.handle_input(InputEvent::Text("new.txt".into()))
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(root.join("new.txt").exists());
    fs::remove_dir_all(root).unwrap();
}

fn tree_prompt(app: &mut App, action: char, value: &str) {
    key(app, KeyCode::Char(action), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Command);
    key(app, KeyCode::Char('u'), Modifiers::CONTROL);
    app.handle_input(InputEvent::Text(value.into())).unwrap();
    key(app, KeyCode::Enter, Modifiers::NONE);
}

#[test]
fn tree_creation_rename_move_and_collision_use_immediate_checked_operations() {
    let (mut app, root) = isolated("tree-immediate");
    chord(&mut app, 'd');
    tree_prompt(&mut app, 'n', "folder/");
    assert!(root.join("folder").is_dir());
    tree_prompt(&mut app, 'n', "nested.txt");
    assert!(root.join("folder/nested.txt").is_file());
    app.directory_tree.selected = root.clone();
    tree_prompt(&mut app, 'n', "file.txt");
    let file = root.join("file.txt");
    fs::write(&file, "contents").unwrap();
    app.open_file(file.clone()).unwrap();
    chord(&mut app, 'd');
    tree_prompt(&mut app, 'r', "renamed.txt");
    assert!(!file.exists());
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(root.join("renamed.txt").as_path())
    );
    assert_eq!(
        fs::read_to_string(root.join("renamed.txt")).unwrap(),
        "contents"
    );
    tree_prompt(&mut app, 'm', "folder/moved.txt");
    assert!(!root.join("renamed.txt").exists());
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(root.join("folder/moved.txt").as_path())
    );
    assert_eq!(
        fs::read_to_string(root.join("folder/moved.txt")).unwrap(),
        "contents"
    );
    fs::write(root.join("folder/existing.txt"), "keep").unwrap();
    tree_prompt(&mut app, 'r', "existing.txt");
    assert_eq!(
        fs::read_to_string(root.join("folder/existing.txt")).unwrap(),
        "keep"
    );
    assert!(root.join("folder/moved.txt").exists());
    assert!(app.fs_confirmation.is_none());
    fs::remove_dir_all(root).unwrap();
}

struct TreeTestTrash(PathBuf);
impl crate::fs_plan::TrashBackend for TreeTestTrash {
    fn delete(&self, path: &Path) -> Result<()> {
        fs::rename(path, &self.0)?;
        Ok(())
    }
}

#[test]
fn tree_delete_uses_interaction_line_defaults_to_no_and_rechecks_captured_plan() {
    let (mut app, root) = isolated("tree-delete");
    let file = root.join("file.txt");
    fs::write(&file, "original").unwrap();
    app.set_trash_backend(Box::new(TreeTestTrash(root.join("trashed.txt"))));
    app.open_file(file.clone()).unwrap();
    chord(&mut app, 'd');
    for cancel in [KeyCode::Enter, KeyCode::Escape, KeyCode::Char('n')] {
        key(&mut app, KeyCode::Char('d'), Modifiers::NONE);
        let view = app.prepare_view(geometry(100, 20));
        let snapshot = app.snapshot(&view);
        assert_eq!(snapshot.status.interaction_line, "Delete file.txt? [y/N]");
        assert!(app.fs_confirmation.is_none());
        assert!(app.has_input_overlay());
        assert!(file.exists());
        key(&mut app, cancel, Modifiers::NONE);
        assert!(app.directory_tree_delete.is_none());
        assert!(file.exists());
    }
    key(&mut app, KeyCode::Char('d'), Modifiers::NONE);
    app.handle_input(InputEvent::Text("y".into())).unwrap();
    assert!(file.exists(), "pasted input must not accept deletion");
    fs::write(&file, "changed while confirmation was open").unwrap();
    key(&mut app, KeyCode::Char('y'), Modifiers::NONE);
    assert!(
        file.exists(),
        "a changed source invalidates the captured plan"
    );
    assert!(!root.join("trashed.txt").exists());
    key(&mut app, KeyCode::Char('d'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('y'), Modifiers::NONE);
    assert!(!file.exists());
    assert_eq!(
        fs::read_to_string(root.join("trashed.txt")).unwrap(),
        "changed while confirmation was open"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_focus_and_resize_share_the_sidebar_boundary() {
    let (mut app, root) = isolated("tree-focus-resize");
    app.config.editor.fast_pane_keys = true;
    app.sync_keymap();
    app.split(Axis::Horizontal, None).unwrap();
    let right = app.active_pane;
    chord(&mut app, 't');
    app.prepare_view(geometry(120, 24));
    key(&mut app, KeyCode::Char('h'), Modifiers::CONTROL);
    let left = app.active_pane;
    assert_ne!(left, right);
    key(&mut app, KeyCode::Char('h'), Modifiers::CONTROL);
    assert!(app.directory_tree.focused);
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('l'), Modifiers::NONE);
    assert!(!app.directory_tree.focused);
    assert_eq!(app.active_pane, left);
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('h'), Modifiers::NONE);
    assert!(app.directory_tree.focused);
    // Exercise the actual colon prompt while the tree owns focus.
    key(&mut app, KeyCode::Char(':'), Modifiers::NONE);
    app.handle_input(InputEvent::Text("resize-right + 5".into()))
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let view = app.prepare_view(geometry(120, 24));
    assert_eq!(view.tree_area.unwrap().width, 38);
    key(&mut app, KeyCode::Char('l'), Modifiers::CONTROL);
    app.execute_command("resize-left + 3").unwrap();
    let view = app.prepare_view(geometry(120, 24));
    assert_eq!(view.tree_area.unwrap().width, 35);
    for (kind, column) in [
        (PointerEventKind::Down(PointerButton::Left), 34),
        (PointerEventKind::Drag(PointerButton::Left), 44),
        (PointerEventKind::Up(PointerButton::Left), 44),
    ] {
        app.handle_pointer(
            PointerEvent {
                kind,
                column,
                row: 5,
                modifiers: Modifiers::NONE,
            },
            &view,
        )
        .unwrap();
    }
    assert_eq!(
        app.prepare_view(geometry(120, 24)).tree_area.unwrap().width,
        45
    );
    chord(&mut app, 't');
    chord(&mut app, 't');
    assert_eq!(
        app.prepare_view(geometry(120, 24)).tree_area.unwrap().width,
        45
    );
    assert_eq!(
        app.prepare_view(geometry(40, 24)).tree_area.unwrap().width,
        16
    );
    assert_eq!(
        app.prepare_view(geometry(120, 24)).tree_area.unwrap().width,
        45
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_open_and_splits_choose_numbered_panes_and_restore_titles() {
    for action in [KeyCode::Enter, KeyCode::Char('v'), KeyCode::Char('s')] {
        let (mut app, root) = isolated("tree-destinations");
        let file = root.join("selected.txt");
        fs::write(&file, "selected").unwrap();
        app.split(Axis::Horizontal, None).unwrap();
        let untouched = app.active_pane;
        let untouched_buffer = app.active().buffer;
        chord(&mut app, 'd');
        app.directory_tree.selected = file.clone();
        let view = app.prepare_view(geometry(120, 24));
        let original = app.snapshot(&view);
        key(&mut app, action, Modifiers::NONE);
        assert_eq!(app.panes.len(), 2);
        let choosing = app.snapshot(&view);
        assert_eq!(
            choosing
                .panes
                .iter()
                .map(|p| p.title.name.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
        let wire: crate::protocol::EditorSnapshot = choosing.clone().into();
        assert_eq!(
            crate::snapshot::EditorSnapshot::try_from(wire).unwrap(),
            choosing
        );
        key(&mut app, KeyCode::Escape, Modifiers::NONE);
        assert_eq!(app.snapshot(&view).panes, original.panes);
        key(&mut app, action, Modifiers::NONE);
        key(&mut app, KeyCode::Char('1'), Modifiers::NONE);
        assert!(!app.directory_tree.focused);
        assert!(app.directory_tree_destination.is_none());
        assert_eq!(app.active_buffer().path.as_deref(), Some(file.as_path()));
        assert_eq!(app.panes[&untouched].buffer, untouched_buffer);
        assert_eq!(
            app.panes.len(),
            if action == KeyCode::Enter { 2 } else { 3 }
        );
        let view = app.prepare_view(geometry(120, 24));
        let opened = view.pane(app.active_pane).unwrap().area;
        let target = view.pane(0).unwrap().area;
        if action == KeyCode::Char('v') {
            assert!(opened.x > target.x);
            assert_eq!(opened.y, target.y);
        }
        if action == KeyCode::Char('s') {
            assert!(opened.y > target.y);
            assert_eq!(opened.x, target.x);
        }
        assert!(
            app.snapshot(&view)
                .panes
                .iter()
                .all(|p| p.title.name != "1")
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn direct_tree_digits_use_visual_order_after_pane_ids_have_gaps() {
    let (mut app, root) = isolated("tree-direct-digits");
    let file = root.join("selected.txt");
    fs::write(&file, "selected").unwrap();
    app.split(Axis::Horizontal, None).unwrap();
    let removed = app.active_pane;
    app.split(Axis::Horizontal, None).unwrap();
    let destination = app.active_pane;
    app.remove_pane(removed);
    chord(&mut app, 'd');
    app.directory_tree.selected = file.clone();
    app.prepare_view(geometry(120, 24));
    key(&mut app, KeyCode::Char('9'), Modifiers::NONE);
    assert!(app.directory_tree.focused);
    key(&mut app, KeyCode::Char('2'), Modifiers::NONE);
    assert!(!app.directory_tree.focused);
    assert_eq!(app.active_pane, destination);
    assert_eq!(app.active_buffer().path.as_deref(), Some(file.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_width_is_editable_and_persisted_from_the_settings_buffer() {
    let (mut app, root) = isolated("tree-width-setting");
    let config_path = root.join("config.yaml");
    fs::write(&config_path, "editor:\n  directory_tree_width: 33\n").unwrap();
    app.note_loaded_config(&config_path);
    chord(&mut app, 't');
    app.open_settings_buffer();
    let row = (0..app.active_buffer().len_lines())
        .find(|row| {
            app.active_buffer().setting_at(*row) == Some(SettingId::EditorDirectoryTreeWidth)
        })
        .unwrap();
    let offset = app.active_buffer().line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    app.handle_input(InputEvent::Text("41".into())).unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.config.editor.directory_tree_width, 41);
    assert_eq!(
        app.prepare_view(geometry(120, 24)).tree_area.unwrap().width,
        41
    );
    let (saved, _) = Config::load(Some(&config_path)).unwrap();
    assert_eq!(saved.editor.directory_tree_width, 41);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_splits_act_immediately_with_one_pane_and_large_choosers_accept_two_digits() {
    let (mut app, root) = isolated("tree-large-chooser");
    let file = root.join("selected.txt");
    fs::write(&file, "selected").unwrap();
    app.open_file(file.clone()).unwrap();
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Char('v'), Modifiers::NONE);
    assert_eq!(app.panes.len(), 2);
    assert!(app.directory_tree_destination.is_none());
    assert_eq!(app.active_buffer().path.as_deref(), Some(file.as_path()));
    for _ in 2..10 {
        app.split(Axis::Horizontal, None).unwrap();
    }
    chord(&mut app, 'd');
    app.prepare_view(geometry(160, 80));
    let expected = app.directory_tree_panes()[9];
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    key(&mut app, KeyCode::Char('1'), Modifiers::NONE);
    assert!(app.directory_tree_destination.is_some());
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    key(&mut app, KeyCode::Char('1'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('0'), Modifiers::NONE);
    assert!(app.directory_tree_destination.is_none());
    assert_eq!(app.active_pane, expected);
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    key(&mut app, KeyCode::Char('1'), Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_pane, app.directory_tree_panes()[0]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_footer_is_bounded_separate_from_rows_and_carried_to_attached_clients() {
    let (mut app, root) = isolated("tree-footer");
    chord(&mut app, 'd');
    let view = app.prepare_view(geometry(120, 20));
    let snapshot = app.snapshot(&view);
    let tree = snapshot.directory_tree.as_ref().unwrap();
    assert_eq!(tree.title, "[dir tree]");
    let legend = tree.legend.join(" ");
    for label in [
        "n: new",
        "d: delete",
        "m: move",
        "r: rename",
        "v: open in v-split",
        "s: open in h-split",
        "Tab: legend",
    ] {
        assert!(legend.contains(label), "{label}: {legend}");
    }
    assert!(tree.legend.iter().all(|line| line.len() <= 31));
    assert_eq!(app.directory_tree.viewport_rows + tree.legend.len() + 3, 20);
    let wire: crate::protocol::EditorSnapshot = snapshot.clone().into();
    assert_eq!(
        crate::snapshot::EditorSnapshot::try_from(wire).unwrap(),
        snapshot
    );
    let selected = app.directory_tree.selected.clone();
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: 1,
            row: 18,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.directory_tree.selected, selected);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let view = app.prepare_view(geometry(120, 20));
    assert!(
        app.snapshot(&view)
            .directory_tree
            .unwrap()
            .legend
            .is_empty()
    );
    assert_eq!(app.directory_tree.viewport_rows, 18);
    for height in 0..6 {
        app.prepare_view(geometry(120, height));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_focus_into_read_only_pane_leaves_insert_and_replace_modes() {
    for mode in [Mode::Insert, Mode::Replace] {
        let (mut app, root) = isolated("tree-readonly-focus");
        app.open_help();
        assert!(app.active_buffer().is_read_only());
        let destination = app.active_pane;
        app.split(Axis::Horizontal, None).unwrap();
        app.open_file(root.join("editable.txt")).unwrap();
        app.mode = mode;
        app.handle_directory_tree_command(EditorCommand::FocusDirectoryTree)
            .unwrap();
        app.prepare_view(geometry(120, 30));
        key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
        key(&mut app, KeyCode::Char('l'), Modifiers::NONE);
        assert_eq!(app.active_pane, destination);
        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.directory_tree.focused);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn tree_focus_from_terminal_input_into_document_enters_normal_mode() {
    let (mut app, root) = isolated("tree-terminal-focus");
    app.config.editor.fast_pane_keys = true;
    app.sync_keymap();
    let destination = app.active_pane;
    app.split(Axis::Horizontal, None).unwrap();
    app.open_terminal_at(Some(terminal_fixture_command()), root.clone());
    assert!(app.active_terminal().is_some());
    app.mode = Mode::Insert;
    app.handle_directory_tree_command(EditorCommand::FocusDirectoryTree)
        .unwrap();
    app.prepare_view(geometry(120, 30));
    key(&mut app, KeyCode::Char('l'), Modifiers::CONTROL);
    assert_eq!(app.active_pane, destination);
    assert_eq!(app.mode, Mode::Normal);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_busy_submission_retains_name_and_does_not_consume_another_confirmation() {
    let (mut app, root) = isolated("tree-confirmation-owner");
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Char('n'), Modifiers::NONE);
    app.handle_input(InputEvent::Text("created.txt".into()))
        .unwrap();
    app.fs_confirmation = Some(FsConfirmation {
        origin: FsConfirmationOrigin::Plugin {
            buffer: app.active().buffer,
        },
        plan: app
            .directory_tree
            .prepare_create(&root, "plugin.txt", false)
            .unwrap(),
        selected: 0,
    });
    // Exercise submission directly: ordinary input gives the existing owner priority.
    app.accept_directory_tree_prompt(TreePromptAction::New, root.clone(), "created.txt");
    assert!(app.fs_confirmation.is_some());
    assert!(!root.join("created.txt").exists());
    assert_eq!(app.mode, Mode::Command);
    app.fs_confirmation = None;
    app.plugins.filesystem_applying = true;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Command);
    assert!(!root.join("created.txt").exists());
    app.plugins.filesystem_applying = false;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(root.join("created.txt").exists());
    assert!(!root.join("plugin.txt").exists());
    app.set_trash_backend(Box::new(TreeTestTrash(root.join("trashed.txt"))));
    key(&mut app, KeyCode::Char('d'), Modifiers::NONE);
    app.plugins.filesystem_applying = true;
    key(&mut app, KeyCode::Char('y'), Modifiers::NONE);
    assert!(app.directory_tree_delete.is_some());
    assert!(root.join("created.txt").exists());
    app.plugins.filesystem_applying = false;
    key(&mut app, KeyCode::Char('y'), Modifiers::NONE);
    assert!(app.directory_tree_delete.is_none());
    assert!(!root.join("created.txt").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_cached_legend_and_help_follow_remapped_keys_and_resize() {
    let (mut app, root) = isolated("tree-remapped-legend");
    chord(&mut app, 'd');
    let area = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 20,
    };
    assert!(app.tree_legend(area).join(" ").contains("n: new"));
    // The public config currently remaps namespaces only. An injected
    // registry exercises direct-key spelling and cache invalidation too.
    let defaults = crate::keymap::default_keymap();
    let mut bindings = defaults.bindings().to_vec();
    let mut spellings = HashMap::new();
    for binding in &bindings {
        spellings.insert(binding.sequence.clone(), binding.sequence.clone());
        if let Some(alias) = &binding.alias {
            spellings.insert(alias.clone(), alias.clone());
        }
    }
    for (old, new) in [
        ("n", "F6"),
        ("v", "F7"),
        ("Tab", "F8"),
        (".", "F9"),
        ("/", "F10"),
        ("g g", "F11"),
    ] {
        let old = crate::keymap::KeySequence::parse(old).unwrap();
        let new = crate::keymap::KeySequence::parse(new).unwrap();
        for binding in &mut bindings {
            if binding.sequence == old {
                binding.sequence = new.clone();
            }
        }
        spellings.insert(old, new);
    }
    let remapped = Keymap::new(bindings).unwrap().with_spelling_metadata(
        KeyStroke::char(' '),
        KeyStroke::ctrl('w'),
        spellings,
    );
    app.set_keymap(Arc::new(remapped));
    let legend = app.tree_legend(area).join(" ");
    assert!(legend.contains("F6: new"));
    assert!(legend.contains("F7: open in v-split"));
    assert!(legend.contains("F8: legend"));
    assert!(legend.contains("F9: hidden files"));
    assert!(legend.contains("F10: search"));
    let narrow = app.tree_legend(Rect { width: 25, ..area });
    assert!(narrow.len() > app.tree_legend(area).len());
    assert_eq!(narrow.join(" "), legend);
    let help = crate::help::render(
        crate::help::HelpTopic::DirectoryTree,
        GrammarKind::Runyte,
        crate::keymap::BindingScope::DirectoryTree,
        app.keymap(),
        false,
    );
    assert!(help.contains("F6 creates"), "{help}");
    assert!(help.contains("F7 opens in a vertical split"));
    assert!(help.contains("F8 toggles"));
    assert!(help.contains("F9 toggles dotfiles"));
    assert!(help.contains("F10 searches visible names"));
    assert!(help.contains("F11 selects the root"));
    assert!(!help.contains("{binding:"));
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(!app.directory_tree.legend_visible);
    key(&mut app, KeyCode::Function(10), Modifiers::NONE);
    assert_eq!(app.prompt_kind, PromptKind::DirectoryTreeSearch);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Command);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_width_config_load_checks_both_bounds() {
    let (mut app, root) = isolated("tree-width-validation");
    let path = root.join("config.yaml");
    for width in [0, 5, 11, 241, 100000] {
        fs::write(&path, format!("editor:\n  directory_tree_width: {width}\n")).unwrap();
        let error = format!("{:#}", Config::load(Some(&path)).unwrap_err());
        assert!(error.contains("editor.directory_tree_width must be between 12 and 240"));
    }
    for width in [12, 33, 240] {
        fs::write(&path, format!("editor:\n  directory_tree_width: {width}\n")).unwrap();
        assert_eq!(
            Config::load(Some(&path))
                .unwrap()
                .0
                .editor
                .directory_tree_width,
            width
        );
    }
    app.note_loaded_config(&path);
    fs::write(&path, "editor:\n  directory_tree_width: 5\n").unwrap();
    let _ = app.execute_command("config-reload");
    assert_eq!(app.config.editor.directory_tree_width, 33);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_move_into_uncached_directory_keeps_its_existing_siblings() {
    let (mut app, root) = isolated("tree-uncached-destination");
    fs::create_dir(root.join("destination")).unwrap();
    fs::write(root.join("destination/sibling"), "keep").unwrap();
    fs::write(root.join("source"), "move").unwrap();
    app.open_file(root.join("source")).unwrap();
    chord(&mut app, 'd');
    tree_prompt(&mut app, 'm', "destination/");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !app
        .directory_tree
        .rows()
        .iter()
        .any(|row| row.path == root.join("destination/sibling"))
    {
        assert!(
            Instant::now() < deadline,
            "destination siblings did not load"
        );
        app.directory_tree.poll();
        std::thread::yield_now();
    }
    assert_eq!(app.directory_tree.selected, root.join("destination/source"));
    fs::remove_dir_all(root).unwrap();
}

fn wait_tree(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.directory_tree.poll();
        if !app.directory_tree.rows().iter().any(|row| row.loading) {
            break;
        }
        assert!(Instant::now() < deadline, "tree listing did not finish");
        std::thread::yield_now();
    }
}

#[test]
fn tree_modal_motions_use_tree_rows_and_viewport() {
    let (mut app, root) = isolated("tree-modal-motions");
    for number in 0..30 {
        fs::write(root.join(format!("file-{number:02}")), "unchanged").unwrap();
    }
    chord(&mut app, 'd');
    wait_tree(&mut app);
    let rows = app.directory_tree.rows();
    app.directory_tree.viewport_rows = 10;
    for (keys, expected) in [("G", 30), ("gg", 0), ("ge", 30), ("gg", 0)] {
        for character in keys.chars() {
            key(&mut app, KeyCode::Char(character), Modifiers::NONE);
        }
        assert_eq!(app.directory_tree.selected, rows[expected].path, "{keys}");
    }
    for (character, expected) in [('f', 10), ('d', 15), ('u', 10), ('b', 0), ('b', 0)] {
        key(&mut app, KeyCode::Char(character), Modifiers::CONTROL);
        assert_eq!(app.directory_tree.selected, rows[expected].path);
    }
    app.directory_tree.scroll = 10;
    for (keys, expected) in [
        ("H", 10),
        ("M", 15),
        ("L", 19),
        ("gt", 10),
        ("gc", 15),
        ("gb", 19),
    ] {
        for character in keys.chars() {
            key(&mut app, KeyCode::Char(character), Modifiers::NONE);
        }
        assert_eq!(app.directory_tree.selected, rows[expected].path, "{keys}");
    }
    assert!(app.directory_tree.focused);
    assert!(!app.active_buffer().dirty);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_dot_toggle_handles_pending_and_cached_listings_and_hidden_ancestors() {
    let (mut app, root) = isolated("tree-dot-toggle");
    app.config.editor.show_hidden_files = false;
    fs::create_dir(root.join(".private")).unwrap();
    fs::write(root.join(".private/inside"), "").unwrap();
    fs::create_dir(root.join("visible")).unwrap();
    fs::write(root.join("visible/.nested"), "").unwrap();
    chord(&mut app, 'd');
    // Toggle before publishing the first listing.
    key(&mut app, KeyCode::Char('.'), Modifiers::NONE);
    wait_tree(&mut app);
    assert!(
        app.directory_tree
            .rows()
            .iter()
            .any(|row| row.path == root.join(".private"))
    );
    app.directory_tree.selected = root.join(".private");
    key(&mut app, KeyCode::Char('l'), Modifiers::NONE);
    wait_tree(&mut app);
    key(&mut app, KeyCode::Char('l'), Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, root.join(".private/inside"));
    key(&mut app, KeyCode::Char('.'), Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, root);
    assert!(
        !app.directory_tree
            .rows()
            .iter()
            .any(|row| row.path.starts_with(root.join(".private")))
    );
    app.directory_tree.selected = root.join("visible");
    key(&mut app, KeyCode::Char('l'), Modifiers::NONE);
    wait_tree(&mut app);
    key(&mut app, KeyCode::Char('h'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('.'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('l'), Modifiers::NONE);
    assert!(
        app.directory_tree
            .rows()
            .iter()
            .any(|row| row.path == root.join("visible/.nested"))
    );
    chord(&mut app, 't');
    chord(&mut app, 'd');
    assert!(
        app.directory_tree
            .rows()
            .iter()
            .any(|row| row.path == root.join("visible/.nested"))
    );
    assert!(!app.config.editor.show_hidden_files);
    key(&mut app, KeyCode::Char('.'), Modifiers::NONE);
    app.directory_tree
        .reveal(&root.join("visible/.nested"), false)
        .unwrap();
    wait_tree(&mut app);
    assert_eq!(app.directory_tree.selected, root.join("visible/.nested"));
    key(&mut app, KeyCode::Char('.'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('.'), Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, root.join("visible"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_search_wraps_visible_names_without_searching_or_editing_the_pane() {
    let (mut app, root) = isolated("tree-search");
    for name in ["Alpha.txt", "beta.txt", "λ-alpha.txt", ".alpha"] {
        fs::write(root.join(name), "alpha in document").unwrap();
    }
    fs::create_dir(root.join("collapsed")).unwrap();
    fs::write(root.join("collapsed/alpha.txt"), "").unwrap();
    app.config.editor.show_hidden_files = false;
    app.open_file(root.join("beta.txt")).unwrap();
    let selection = app.active().selection.clone();
    chord(&mut app, 'd');
    wait_tree(&mut app);
    key(&mut app, KeyCode::Char('/'), Modifiers::NONE);
    assert_eq!(app.prompt_kind, PromptKind::DirectoryTreeSearch);
    app.handle_input(InputEvent::Text("alpha.*txt$".into()))
        .unwrap();
    let view = app.prepare_view(geometry(100, 30));
    let snapshot = app.snapshot(&view);
    let wire: crate::protocol::EditorSnapshot = snapshot.clone().into();
    assert_eq!(
        crate::snapshot::EditorSnapshot::try_from(wire).unwrap(),
        snapshot
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, root.join("λ-alpha.txt"));
    key(&mut app, KeyCode::Char('n'), Modifiers::CONTROL);
    assert_eq!(app.directory_tree.selected, root.join("Alpha.txt"));
    key(&mut app, KeyCode::Char('p'), Modifiers::CONTROL);
    assert_eq!(app.directory_tree.selected, root.join("λ-alpha.txt"));
    key(&mut app, KeyCode::Char('/'), Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, root.join("Alpha.txt"));
    for pattern in ["[", "not-present"] {
        tree_prompt(&mut app, '/', pattern);
        assert_eq!(app.directory_tree.selected, root.join("Alpha.txt"));
        assert!(app.directory_tree.focused);
    }
    assert!(app.status.contains("no matching tree entry"));
    key(&mut app, KeyCode::Char('/'), Modifiers::NONE);
    app.handle_input(InputEvent::Text("beta".into())).unwrap();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, root.join("Alpha.txt"));
    assert_eq!(app.active().selection, selection);
    assert_eq!(app.active_buffer().to_string(), "alpha in document");
    assert!(app.search.pattern.is_empty());
    key(&mut app, KeyCode::Char('n'), Modifiers::NONE);
    assert_eq!(
        app.prompt_kind,
        PromptKind::DirectoryTreeAction(TreePromptAction::New)
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_empty_search_and_invalid_regex_preserve_the_last_query() {
    let (mut app, root) = isolated("tree-search-errors");
    fs::write(root.join("one"), "").unwrap();
    fs::write(root.join("two"), "").unwrap();
    chord(&mut app, 'd');
    wait_tree(&mut app);
    tree_prompt(&mut app, '/', "");
    assert!(app.status.contains("tree search pattern is empty"));
    tree_prompt(&mut app, '/', "one|two");
    assert_eq!(app.directory_tree.selected, root.join("one"));
    tree_prompt(&mut app, '/', "[");
    assert!(app.status.contains("regex parse error"));
    key(&mut app, KeyCode::Char('n'), Modifiers::CONTROL);
    assert_eq!(app.directory_tree.selected, root.join("two"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_close_commands_hide_only_the_focused_tree() {
    for command in ["q", "quit", "q!", "wc", "window-close", "close", "close!"] {
        for split in [false, true] {
            let (mut app, root) = isolated("tree-close-commands");
            let file = root.join("file.txt");
            fs::write(&file, "original").unwrap();
            app.open_file(file.clone()).unwrap();
            if split {
                app.execute(CommandInvocation::split_vertical(None))
                    .unwrap();
            }
            key(&mut app, KeyCode::Char('i'), Modifiers::NONE);
            app.handle_input(InputEvent::Text("unsaved".into()))
                .unwrap();
            let pane = app.active_pane;
            let buffers = app.buffers.len();
            let panes = app.panes.len();
            let text = app.active_buffer().to_string();
            app.handle_directory_tree_command(EditorCommand::FocusDirectoryTree)
                .unwrap();
            key(&mut app, KeyCode::Char(':'), Modifiers::NONE);
            app.handle_input(InputEvent::Text(command.into())).unwrap();
            key(&mut app, KeyCode::Enter, Modifiers::NONE);
            assert!(!app.directory_tree.visible, "{command}");
            assert!(!app.directory_tree.focused, "{command}");
            assert!(!app.should_quit, "{command}");
            assert_eq!(app.panes.len(), panes, "{command}");
            assert_eq!(app.buffers.len(), buffers, "{command}");
            assert_eq!(app.active_pane, pane);
            assert_eq!(app.mode, Mode::Insert);
            assert_eq!(app.active_buffer().to_string(), text);
            assert_eq!(fs::read_to_string(file).unwrap(), "original");
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn tree_close_preserves_the_backing_terminal_and_unfocused_close_targets_the_pane() {
    let (mut app, root) = isolated("tree-close-terminal");
    app.open_terminal_at(Some(terminal_fixture_command()), root.clone());
    let terminal = app.active_terminal().unwrap();
    for command in ["q", "wc", "close"] {
        app.handle_directory_tree_command(EditorCommand::FocusDirectoryTree)
            .unwrap();
        app.execute_command(command).unwrap();
        assert!(!app.directory_tree.visible);
        assert_eq!(app.active_terminal(), Some(terminal));
        assert_eq!(app.terminals.len(), 1);
        assert_eq!(app.mode, Mode::Insert);
        assert!(!app.should_quit);
    }
    app.mode = Mode::Normal;
    app.split(Axis::Horizontal, None).unwrap();
    chord(&mut app, 't');
    assert!(!app.directory_tree.focused);
    app.execute_command("wc").unwrap();
    assert!(app.directory_tree.visible);
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.terminals.len(), 1);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn goto_word_selects_tree_paths_without_touching_the_covered_buffer() {
    let (mut app, root) = isolated("tree-jump");
    for name in ["alpha", "beta", "目录"] {
        fs::write(root.join(name), "unchanged").unwrap();
    }
    app.open_file(root.join("alpha")).unwrap();
    let selection = app.active().selection.clone();
    chord(&mut app, 'd');
    wait_tree(&mut app);
    let view = app.prepare_view(geometry(90, 20));
    key(&mut app, KeyCode::Char('g'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('w'), Modifiers::NONE);
    let snapshot = app.snapshot(&view);
    let tree = snapshot.directory_tree.as_ref().unwrap();
    assert!(tree.jump_active);
    assert!(snapshot.panes.iter().all(|pane| !pane.jump_active));
    let label = tree
        .rows
        .iter()
        .find(|row| row.path == root.join("目录"))
        .unwrap()
        .jump_label[0]
        .unwrap()
        .0;
    let wire: crate::protocol::EditorSnapshot = snapshot.clone().into();
    assert_eq!(
        crate::snapshot::EditorSnapshot::try_from(wire).unwrap(),
        snapshot
    );
    key(&mut app, KeyCode::Char(label), Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, root.join("目录"));
    assert!(app.directory_tree.focused);
    assert!(app.jump.is_none());
    assert_eq!(app.active_buffer().path.as_ref(), Some(&root.join("alpha")));
    assert_eq!(app.active().selection, selection);
    assert!(!app.active_buffer().dirty);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_jump_narrows_two_keys_and_cancels_without_running_tree_actions() {
    let (mut app, root) = isolated("tree-jump-many");
    for index in 0..60 {
        fs::write(root.join(format!("file-{index:02}")), "").unwrap();
    }
    chord(&mut app, 'd');
    wait_tree(&mut app);
    let view = app.prepare_view(geometry(90, 50));
    key(&mut app, KeyCode::Char('g'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('w'), Modifiers::NONE);
    let tree = app.snapshot(&view).directory_tree.unwrap();
    assert_eq!(app.jump.as_ref().unwrap().len(), tree.rows.len());
    assert!(tree.rows.len() < 61);
    let target = tree
        .rows
        .iter()
        .find(|row| row.jump_label[1].is_some())
        .unwrap();
    let path = target.path.clone();
    let first = target.jump_label[0].unwrap().0;
    let second = target.jump_label[1].unwrap().0;
    key(&mut app, KeyCode::Char(first), Modifiers::NONE);
    let tree = app.snapshot(&view).directory_tree.unwrap();
    let target = tree.rows.iter().find(|row| row.path == path).unwrap();
    assert_eq!(
        target.jump_label,
        [
            Some((second, crate::jump_labels::LabelPart::Immediate)),
            None
        ]
    );
    key(&mut app, KeyCode::Char(second), Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, path);
    for cancel in [KeyCode::Escape, KeyCode::Char('!'), KeyCode::Enter] {
        key(&mut app, KeyCode::Char('g'), Modifiers::NONE);
        key(&mut app, KeyCode::Char('w'), Modifiers::NONE);
        key(&mut app, cancel, Modifiers::NONE);
        assert!(app.jump.is_none());
        assert!(app.directory_tree.focused);
        assert_eq!(app.directory_tree.selected, path);
        assert!(app.directory_tree_destination.is_none());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_jump_tracks_paths_when_a_listing_inserts_rows() {
    let (mut app, root) = isolated("tree-jump-refresh");
    fs::write(root.join("beta"), "").unwrap();
    chord(&mut app, 'd');
    wait_tree(&mut app);
    let view = app.prepare_view(geometry(90, 20));
    key(&mut app, KeyCode::Char('g'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('w'), Modifiers::NONE);
    let tree = app.snapshot(&view).directory_tree.unwrap();
    let label = tree
        .rows
        .iter()
        .find(|row| row.path == root.join("beta"))
        .unwrap()
        .jump_label[0]
        .unwrap()
        .0;
    fs::write(root.join("alpha"), "").unwrap();
    app.directory_tree.refresh(root.clone());
    wait_tree(&mut app);
    key(&mut app, KeyCode::Char(label), Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, root.join("beta"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_jump_uses_scrolled_viewport_and_ignores_removed_targets() {
    let (mut app, root) = isolated("tree-jump-scroll");
    for index in 0..30 {
        fs::write(root.join(format!("file-{index:02}")), "").unwrap();
    }
    chord(&mut app, 'd');
    wait_tree(&mut app);
    key(&mut app, KeyCode::Char('g'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('w'), Modifiers::NONE);
    assert!(app.jump.is_none()); // No rendered geometry yet.
    app.directory_tree.select_last();
    let view = app.prepare_view(geometry(90, 15));
    assert!(app.directory_tree.scroll > 0);
    key(&mut app, KeyCode::Char('g'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('w'), Modifiers::NONE);
    let tree = app.snapshot(&view).directory_tree.unwrap();
    assert_eq!(app.jump.as_ref().unwrap().len(), tree.rows.len());
    assert!(tree.rows.iter().all(|row| row.jump_label[0].is_some()));
    assert!(!app.directory_tree_jump_paths.contains(&root));
    let target = &tree.rows[0];
    let label = target.jump_label[0].unwrap().0;
    let selected = app.directory_tree.selected.clone();
    fs::remove_file(&target.path).unwrap();
    app.directory_tree.refresh(root.clone());
    wait_tree(&mut app);
    key(&mut app, KeyCode::Char(label), Modifiers::NONE);
    assert_eq!(app.directory_tree.selected, selected);
    assert!(app.jump.is_none());
    fs::remove_dir_all(root).unwrap();
}
