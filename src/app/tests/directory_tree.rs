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
    assert_eq!(after.tree_area.unwrap().width, 28);
    assert_eq!(format!("{:#?}", app.layout), format!("{layout:#?}"));
    assert_eq!(app.panes.keys().copied().collect::<HashSet<_>>(), panes);
    assert_eq!(after.panes.len(), before.panes.len());
    assert!(after.panes.iter().all(|pane| pane.area.x >= 28));
    let snapshot = app.snapshot(&after);
    assert_eq!(snapshot.directory_tree.as_ref().unwrap().pending_count, 0);
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
    assert!(!app.directory_tree.focused);
    assert_eq!(app.active_pane, destination);
    assert_eq!(app.active_buffer().path.as_deref(), Some(path.as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_actions_stage_review_undo_and_clear_without_disk_mutation() {
    let (mut app, root) = isolated("directory-tree-actions");
    chord(&mut app, 'd');
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.context_action_menu.is_some());
    key(&mut app, KeyCode::Char('n'), Modifiers::NONE);
    assert_eq!(
        app.prompt_kind,
        PromptKind::DirectoryTreeAction(TreePromptAction::New)
    );
    app.handle_input(InputEvent::Text("new.txt".into()))
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.directory_tree.pending_count(), 1);
    assert!(!root.join("new.txt").exists());
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('p'), Modifiers::NONE);
    assert_eq!(
        app.fs_confirmation
            .as_ref()
            .unwrap()
            .plan
            .operations()
            .len(),
        1
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.directory_tree.pending_count(), 1);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('u'), Modifiers::NONE);
    assert_eq!(app.directory_tree.pending_count(), 0);
    app.directory_tree
        .stage_create(&root, "new.txt", true)
        .unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('c'), Modifiers::NONE);
    assert!(app.directory_tree_discard_confirmation);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.directory_tree.pending_count(), 1);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('c'), Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.directory_tree.pending_count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_pending_plan_blocks_normal_quit() {
    let (mut app, root) = isolated("directory-tree-quit");
    app.directory_tree
        .stage_create(&root, "pending.txt", true)
        .unwrap();
    app.execute_command("quit-all").unwrap();
    assert!(!app.should_quit);
    assert!(app.status.contains("unsaved changes"));
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
    app.directory_tree.stage_create(&root, "a", true).unwrap();
    app.directory_tree.stage_create(&root, "b", true).unwrap();
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
    assert_eq!(app.directory_tree.pending_count(), 1);
    fs::remove_dir_all(root).unwrap();
}
