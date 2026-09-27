// SPDX-License-Identifier: MPL-2.0

use super::*;

fn document() -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha βeta gamma\nsecond line\nthird line\n");
    app
}

fn undo_selection(app: &mut App) {
    key(app, KeyCode::Char('u'), Modifiers::ALT);
}

fn redo_selection(app: &mut App) {
    key(app, KeyCode::Char('U'), Modifiers::ALT | Modifiers::SHIFT);
}

#[test]
fn selection_history_recovers_ranges_direction_primary_and_mode() {
    let mut app = document();
    let selection = Selection::new(vec![Range::new(5, 0), Range::new(6, 10)], 1);
    app.active_mut().replace_selection(selection.clone());
    app.mode = Mode::Select;
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    let collapsed = app.active().selection.clone();
    undo_selection(&mut app);
    assert_eq!(app.active().selection, selection);
    assert_eq!(app.mode, Mode::Select);
    redo_selection(&mut app);
    assert_eq!(app.active().selection, collapsed);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(text(&app), "alpha βeta gamma\nsecond line\nthird line\n");
}

#[test]
fn selection_history_groups_counts_and_preserves_redo_on_noop() {
    let mut app = document();
    press(&mut app, '3');
    press(&mut app, 'l');
    assert_eq!(app.active().head(), 3);
    undo_selection(&mut app);
    assert_eq!(app.active().head(), 0);
    press(&mut app, 'h'); // no movement, redo survives
    redo_selection(&mut app);
    assert_eq!(app.active().head(), 3);
    undo_selection(&mut app);
    press(&mut app, 'w');
    let changed = app.active().selection.clone();
    redo_selection(&mut app);
    assert_eq!(app.active().selection, changed);
    assert!(app.status.contains("no later selection"));
}

#[test]
fn selection_history_restores_whole_line_delete_behavior() {
    let mut app = document();
    press(&mut app, 'x');
    let lines = app.active().selection.clone();
    press(&mut app, 'j');
    undo_selection(&mut app);
    assert_eq!(app.active().selection, lines);
    assert!(app.line_select.is_some());
    press(&mut app, 'd');
    assert_eq!(text(&app), "second line\nthird line\n");
    let after = app.active().selection.clone();
    undo_selection(&mut app);
    assert_eq!(app.active().selection, after);
}

#[test]
fn selection_history_aliases_and_semantic_execution_share_history() {
    let mut app = document();
    app.execute(CommandInvocation::editor(EditorCommand::MoveRight, Default::default()).unwrap())
        .unwrap();
    for ch in [' ', 's', 'u'] {
        press(&mut app, ch);
    }
    assert_eq!(app.active().head(), 0);
    for ch in [' ', 's', 'U'] {
        press(&mut app, ch);
    }
    assert_eq!(app.active().head(), 1);
    app.execute(
        CommandInvocation::editor(EditorCommand::SelectionUndo, Default::default()).unwrap(),
    )
    .unwrap();
    assert_eq!(app.active().head(), 0);
    app.execute_command("selection-redo").unwrap();
    assert_eq!(app.active().head(), 1);
    app.execute_command("selection-undo").unwrap();
    assert_eq!(app.active().head(), 0);
}

#[test]
fn selection_history_search_acceptance_is_one_step_and_cancel_is_noop() {
    let mut app = document();
    let original = app.active().selection.clone();
    press(&mut app, 's');
    for ch in "line".chars() {
        press(&mut app, ch);
        app.refresh_search_preview();
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let matches = app.active().selection.clone();
    assert_eq!(matches.len(), 2);
    undo_selection(&mut app);
    assert_eq!(app.active().selection, original);
    redo_selection(&mut app);
    assert_eq!(app.active().selection, matches);
    press(&mut app, 's');
    press(&mut app, 'z');
    app.refresh_search_preview();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    undo_selection(&mut app);
    assert_eq!(app.active().selection, original);
}

#[test]
fn selection_history_is_independent_in_splits_and_invalidated_by_shared_edits() {
    let mut app = document();
    press(&mut app, 'l');
    let first = app.active_pane;
    app.execute_editor_command(EditorCommand::SplitVertical)
        .unwrap();
    let second = app.active_pane;
    assert_ne!(first, second);
    assert!(app.active().selection_history.is_empty());
    // Splits normally start an explorer. Explicitly show the same document.
    app.active_mut().retarget(0);
    app.active_mut().replace_selection(Selection::point(0));
    press(&mut app, 'w');
    undo_selection(&mut app);
    assert_eq!(app.active().head(), 0);
    app.active_pane = first;
    assert_eq!(app.active().head(), 1);
    undo_selection(&mut app);
    assert_eq!(app.active().head(), 0);
    redo_selection(&mut app);
    app.active_pane = second;
    press(&mut app, 'i');
    press(&mut app, 'Z');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.active_pane = first;
    let after_edit = app.active().selection.clone();
    undo_selection(&mut app);
    assert_eq!(app.active().selection, after_edit);
    assert!(app.status.contains("no earlier selection"));
}

#[test]
fn selection_history_survives_buffer_visits_but_not_text_undo() {
    let mut app = document();
    press(&mut app, 'l');
    let other = app.buffers.len();
    app.buffers.push(Buffer::scratch());
    app.syntax.push(None);
    app.active_mut().retarget(other);
    app.active_mut().retarget(0);
    undo_selection(&mut app);
    assert_eq!(app.active().head(), 0);
    press(&mut app, 'l');
    press(&mut app, 'u');
    let after = app.active().selection.clone();
    undo_selection(&mut app);
    assert_eq!(app.active().selection, after);
    assert!(app.status.contains("no earlier selection"));
}

#[test]
fn selection_history_is_bounded_and_works_in_read_only_buffers() {
    let mut app = document();
    app.open_virtual_page(
        GeneratedViewIdentity::Named("history-test".into()),
        "[history]".into(),
        &"a".repeat(300),
        ContentAlignment::default(),
    );
    for _ in 0..200 {
        press(&mut app, 'l');
    }
    for _ in 0..200 {
        undo_selection(&mut app);
    }
    assert_eq!(app.active().head(), 72);
    assert!(app.status.contains("no earlier selection"));
}

#[test]
fn selection_history_groups_mouse_drag_and_keeps_pointer_semantics() {
    let mut app = document();
    let original = app.active().selection.clone();
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
    for (kind, offset) in [
        (PointerEventKind::Down(PointerButton::Left), 1),
        (PointerEventKind::Drag(PointerButton::Left), 3),
        (PointerEventKind::Drag(PointerButton::Left), 8),
        (PointerEventKind::Up(PointerButton::Left), 8),
    ] {
        let view = app.prepare_view(geometry);
        let body = view.pane(app.active_pane).unwrap().body;
        app.handle_pointer(
            PointerEvent {
                kind,
                column: body.x + offset,
                row: body.y,
                modifiers: Modifiers::NONE,
            },
            &view,
        )
        .unwrap();
    }
    let dragged = app.active().selection.clone();
    let semantics = app.active().selection_semantics;
    assert_ne!(dragged, original);
    undo_selection(&mut app);
    assert_eq!(app.active().selection, original);
    redo_selection(&mut app);
    assert_eq!(app.active().selection, dragged);
    assert_eq!(app.active().selection_semantics, semantics);
    undo_selection(&mut app);
    undo_selection(&mut app);
    assert!(app.status.contains("no earlier selection"));
}

#[test]
fn selection_history_bounds_range_storage_and_breaks_on_oversized_selections() {
    let mut app = document();
    app.buffers[0].apply(&Transaction::insert(0, "a".repeat(10000)));
    let many = Selection::new((0..2048).map(|i| Range::point(i * 2)).collect(), 0);
    app.active_mut().replace_selection(many.clone());
    for _ in 0..4 {
        press(&mut app, ')');
    }
    for _ in 0..3 {
        undo_selection(&mut app);
    }
    assert!(app.status.contains("no earlier selection"));
    assert_eq!(app.active().selection.len(), 2048);
    let oversized = Selection::new((0..4097).map(|i| Range::point(i * 2)).collect(), 0);
    app.active_mut().replace_selection(oversized);
    press(&mut app, ',');
    let collapsed = app.active().selection.clone();
    undo_selection(&mut app);
    assert_eq!(app.active().selection, collapsed);
    assert!(app.status.contains("no earlier selection"));
}

#[test]
fn selection_history_bounds_retained_buffers() {
    let mut app = document();
    for _ in 0..40 {
        let id = app.buffers.len();
        let mut buffer = Buffer::scratch();
        buffer.apply(&Transaction::insert(0, "abc"));
        app.buffers.push(buffer);
        app.syntax.push(None);
        app.active_mut().retarget(id);
        app.active_mut().replace_selection(Selection::point(0));
        press(&mut app, 'l');
    }
    assert_eq!(app.active().selection_history.len(), 32);
    app.active_mut().retarget(1);
    undo_selection(&mut app);
    assert!(app.status.contains("no earlier selection"));
    assert_eq!(app.active().selection_history.len(), 32);
}

#[test]
fn selection_history_replayed_keys_and_counted_history_commands() {
    let mut app = document();
    for _ in 0..3 {
        app.handle_replayed_input(InputEvent::Key(KeyStroke::new(
            KeyCode::Char('l'),
            Modifiers::NONE,
        )))
        .unwrap();
    }
    assert_eq!(app.active().head(), 3);
    press(&mut app, '3');
    undo_selection(&mut app);
    assert_eq!(app.active().head(), 0);
    press(&mut app, '3');
    redo_selection(&mut app);
    assert_eq!(app.active().head(), 3);
}

#[test]
fn selection_history_does_not_reenter_insert_or_restore_across_replacement() {
    let mut app = document();
    press(&mut app, 'v');
    press(&mut app, 'w');
    let selected = app.active().selection.clone();
    press(&mut app, 'i');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    undo_selection(&mut app);
    assert_eq!(app.active().selection, selected);
    assert_eq!(app.mode, Mode::Select);
    app.buffers[0] = Buffer::scratch();
    app.active_mut().replace_selection(Selection::point(0));
    undo_selection(&mut app);
    assert_eq!(app.active().selection, Selection::point(0));
    assert!(app.status.contains("no earlier selection"));
}

fn typed_command(app: &mut App, command: &str) {
    press(app, ':');
    for ch in command.chars() {
        press(app, ch);
    }
    key(app, KeyCode::Enter, Modifiers::NONE);
}

#[test]
fn selection_history_typed_colon_commands_preserve_line_and_select_modes() {
    let mut app = document();
    press(&mut app, 'x');
    let line = app.active().selection.clone();
    typed_command(&mut app, "selection-undo");
    assert_eq!(app.active().selection, Selection::point(0));
    redo_selection(&mut app);
    assert_eq!(app.active().selection, line);
    assert!(app.line_select.is_some());
    assert_eq!(app.mode, Mode::Select);

    let mut app = document();
    for ch in ['v', 'w', 'w'] {
        press(&mut app, ch);
    }
    let selected = app.active().selection.clone();
    typed_command(&mut app, "selection-undo");
    assert_ne!(app.active().selection, selected);
    redo_selection(&mut app);
    assert_eq!(app.active().selection, selected);
    assert_eq!(app.mode, Mode::Select);
}

#[test]
fn selection_history_cancelled_prompts_after_line_selection_preserve_redo() {
    for prompt in ['s', '/', ':'] {
        let mut app = document();
        press(&mut app, 'x');
        let line = app.active().selection.clone();
        press(&mut app, 'j');
        let moved = app.active().selection.clone();
        undo_selection(&mut app);
        assert_eq!(app.active().selection, line);
        press(&mut app, prompt);
        press(&mut app, 'z');
        key(&mut app, KeyCode::Escape, Modifiers::NONE);
        redo_selection(&mut app);
        assert_eq!(app.active().selection, moved, "cancel {prompt}");
        undo_selection(&mut app);
        undo_selection(&mut app);
        assert_eq!(app.active().selection, Selection::point(0));
    }
}

#[test]
fn selection_history_search_from_lines_is_one_step_and_restores_line_behavior() {
    let mut app = document();
    press(&mut app, 'x');
    let line = app.active().selection.clone();
    press(&mut app, 's');
    for ch in "alpha".chars() {
        press(&mut app, ch);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.line_select.is_none());
    undo_selection(&mut app);
    assert_eq!(app.active().selection, line);
    assert!(app.line_select.is_some());
    undo_selection(&mut app);
    assert_eq!(app.active().selection, Selection::point(0));
}
