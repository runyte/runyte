// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::snapshot::ResizeSnapshot;

fn geometry() -> FrameGeometry {
    crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 100, 32))
}

fn split_app() -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "preserve this text\n");
    app.split(Axis::Horizontal, None).unwrap();
    app.active_pane = 0;
    app.prepare_view(geometry());
    app
}

fn begin(app: &mut App) {
    let outcome = app.execute_command("resize").unwrap();
    assert!(app.pane_resize.is_some(), "{outcome:?}: {}", app.status);
}

fn preview(app: &mut App) -> ResizeSnapshot {
    let view = app.prepare_view(geometry());
    app.snapshot(&view).resize.expect("resize preview")
}

fn active_area(app: &mut App) -> Rect {
    preview(app)
        .panes
        .into_iter()
        .find(|p| p.active)
        .unwrap()
        .area
}

#[test]
fn resize_preview_moves_in_cells_and_only_enter_changes_the_live_layout() {
    for (axis, active, motion, growth) in [
        (Axis::Horizontal, 0, KeyCode::Right, true),
        (Axis::Horizontal, 1, KeyCode::Right, false),
        (Axis::Vertical, 0, KeyCode::Down, true),
        (Axis::Vertical, 1, KeyCode::Down, false),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.split(axis, None).unwrap();
        app.active_pane = active;
        let old = app.prepare_view(geometry()).pane(active).unwrap().area;
        let original = app.layout.clone();
        begin(&mut app);
        for _ in 0..4 {
            key(&mut app, motion, Modifiers::NONE);
        }
        assert_eq!(app.layout, original);
        let proposed = active_area(&mut app);
        let extent = |r: Rect| {
            if axis == Axis::Horizontal {
                r.width
            } else {
                r.height
            }
        };
        assert_eq!(
            i32::from(extent(proposed)) - i32::from(extent(old)),
            if growth { 4 } else { -4 }
        );
        assert_eq!(app.prepare_view(geometry()).pane(active).unwrap().area, old);
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        assert!(app.pane_resize.is_none());
        assert_eq!(
            app.prepare_view(geometry()).pane(active).unwrap().area,
            proposed
        );
    }
}

#[test]
fn resize_preview_cycles_edges_and_ignores_perpendicular_motion() {
    let mut app = split_app();
    app.split(Axis::Vertical, None).unwrap();
    app.active_pane = 0;
    app.prepare_view(geometry());
    begin(&mut app);
    let initial = active_area(&mut app);
    press(&mut app, 'j');
    assert_eq!(active_area(&mut app), initial);
    assert!(preview(&mut app).detail.contains("left/right"));
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    press(&mut app, 'j');
    assert_eq!(active_area(&mut app).height, initial.height + 1);
    key(&mut app, KeyCode::BackTab, Modifiers::SHIFT);
    press(&mut app, 'l');
    assert_eq!(active_area(&mut app).width, initial.width + 1);
    press(&mut app, 'u');
    assert_eq!(active_area(&mut app).width, initial.width);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.prepare_view(geometry()).pane(0).unwrap().area, initial);
}

#[test]
fn resize_preview_extension_is_undoable_and_preserves_hidden_buffers_and_terminals() {
    let mut app = split_app();
    let terminal = app.terminals.insert_test_session(48, 28);
    app.panes.get_mut(&1).unwrap().terminal = Some(terminal);
    let original = app.layout.clone();
    let buffers = app.buffers.len();
    begin(&mut app);
    press(&mut app, 'L');
    assert_eq!(preview(&mut app).panes.len(), 1);
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.layout, original);
    assert!(preview(&mut app).detail.contains("1 pane views removed"));
    press(&mut app, 'u');
    assert_eq!(preview(&mut app).panes.len(), 2);
    key(&mut app, KeyCode::Right, Modifiers::SHIFT);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.layout, original);
    begin(&mut app);
    key(&mut app, KeyCode::Right, Modifiers::SHIFT);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.buffers.len(), buffers);
    assert!(app.terminals.get(terminal).is_some());
}

#[test]
fn resize_preview_shows_fragmented_neighbour_removal_and_all_survivor_geometry() {
    let mut app = split_app();
    // AAA / BCD: C extends up, closes fragmented A, B and D fill the gaps.
    for id in 2..4 {
        app.panes.insert(id, Pane::new(0));
    }
    app.layout = Layout::Pane(0);
    app.layout.split(0, 1, Axis::Vertical);
    app.layout.split(1, 2, Axis::Horizontal);
    app.layout.split(2, 3, Axis::Horizontal);
    app.active_pane = 2;
    app.prepare_view(geometry());
    begin(&mut app);
    press(&mut app, 'K');
    let proposed = preview(&mut app);
    assert_eq!(proposed.panes.len(), 3);
    assert!(
        proposed
            .panes
            .iter()
            .all(|p| p.area.height == geometry().editor.height)
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let actual = app.prepare_view(geometry());
    assert_eq!(actual.panes.len(), 3);
    for pane in actual.panes {
        assert!(proposed.panes.iter().any(|p| p.area == pane.area));
    }
}

#[test]
fn resize_preview_does_not_resize_terminals_or_deliver_text_or_pointer_input() {
    let mut app = split_app();
    let terminal = app.terminals.insert_test_session(48, 28);
    app.panes.get_mut(&0).unwrap().terminal = Some(terminal);
    app.prepare_view(geometry());
    let columns = app.terminals.get(terminal).unwrap().columns();
    begin(&mut app);
    press(&mut app, 'l');
    let proposed = active_area(&mut app);
    assert_eq!(app.terminals.get(terminal).unwrap().columns(), columns);
    app.handle_input(InputEvent::Text("paste must not edit".into()))
        .unwrap();
    app.handle_input(InputEvent::ClipboardPaste).unwrap();
    let view = app.prepare_view(geometry());
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: 70,
            row: 5,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active_pane, 0);
    assert_eq!(active_area(&mut app), proposed);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.prepare_view(geometry());
    assert_eq!(app.terminals.get(terminal).unwrap().columns(), columns + 1);
    assert_eq!(app.buffers[0].text().to_string(), "preserve this text\n");
}

#[test]
fn resize_preview_cancels_on_geometry_layout_semantic_command_and_attachment_changes() {
    for change in 0..4 {
        let mut app = split_app();
        begin(&mut app);
        press(&mut app, 'L');
        match change {
            0 => {
                let mut geometry = geometry();
                geometry.editor.width -= 1;
                app.prepare_view(geometry);
            }
            1 => {
                app.layout.equalize();
                app.active_pane = 1;
                app.prepare_view(geometry());
            }
            2 => {
                app.execute_command("vsplit").unwrap();
            }
            _ => {
                app.note_plugin_frontend(true);
                app.note_plugin_frontend(false);
            }
        }
        assert!(app.pane_resize.is_none());
        assert!(app.panes.contains_key(&1));
    }
}

#[test]
fn resize_preview_binding_remaps_drive_dispatch_and_legend() {
    let mut app = split_app();
    let config = serde_yaml::from_str(
        "ui: {pane-resize: {move-right: [r], accept: [a], extend-right: [e]}}",
    )
    .unwrap();
    let compiled = crate::keymap::configured::compile(&config, crate::keymap::default_keymap());
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    app.set_keymap(compiled.keymap);
    begin(&mut app);
    let original = active_area(&mut app);
    press(&mut app, 'l');
    assert_eq!(active_area(&mut app), original);
    press(&mut app, 'r');
    assert_eq!(active_area(&mut app).width, original.width + 1);
    let snapshot = preview(&mut app);
    assert!(snapshot.controls.contains('r'));
    assert!(snapshot.decisions.contains("a apply"));
    press(&mut app, 'e');
    assert_eq!(preview(&mut app).panes.len(), 1);
    press(&mut app, 'a');
    assert!(app.pane_resize.is_none());
    assert_eq!(app.panes.len(), 1);
}

#[test]
fn resize_preview_clamps_minimum_and_undo_restores_last_actual_change() {
    let mut app = split_app();
    begin(&mut app);
    for _ in 0..110 {
        press(&mut app, 'h');
    }
    assert_eq!(active_area(&mut app).width, 3);
    press(&mut app, 'u');
    assert_eq!(active_area(&mut app).width, 4);
    press(&mut app, 'H');
    assert!(preview(&mut app).detail.contains("No valid extension"));
}

#[test]
fn resize_preview_fade_finishes_without_an_idle_timer() {
    let mut app = split_app();
    begin(&mut app);
    assert!(app.has_presentation_animation());
    app.advance_resize_fade(Instant::now() + std::time::Duration::from_secs(1));
    assert!(!app.has_presentation_animation());
    assert_eq!(preview(&mut app).fade, 255);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    begin(&mut app);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(!app.has_presentation_animation());
}

#[test]
fn resize_preview_keeps_focus_when_absorbing_a_temporary_comparison_side() {
    use crate::git::{MemoryGitProvider, Repository};
    let root = crate::test_support::TestRuntimeRoot::new("resize-comparison").unwrap();
    let path = root.join("source.txt");
    fs::write(&path, "working\n").unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&*root))
            .with_staged("source.txt", "staged\n")
            .with_working("source.txt", "working\n"),
    ));
    let mut app = App::new_in_isolated_project(&*root, ports).unwrap();
    app.open_file(path).unwrap();
    let source = app.active().buffer;
    for character in [' ', 'g', 'D'] {
        press(&mut app, character);
    }
    let left = app.diffs[0].side(Side::Left).pane;
    let right = app.diffs[0].side(Side::Right).pane;
    app.activate_pane(right);
    app.split(Axis::Horizontal, None).unwrap();
    let extending = app.active_pane;
    app.prepare_view(geometry());
    begin(&mut app);
    press(&mut app, 'H');
    let proposed = active_area(&mut app);
    assert_eq!(preview(&mut app).panes.len(), 2);
    assert_eq!(app.diffs.len(), 1);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_pane, extending);
    assert!(!app.panes.contains_key(&right));
    assert!(app.diffs.is_empty());
    assert_eq!(app.panes[&left].buffer, source);
    assert_eq!(
        app.prepare_view(geometry()).pane(extending).unwrap().area,
        proposed
    );
}
