// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::config::PaneStyle;

fn geometry(width: u16, height: u16) -> FrameGeometry {
    crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, width, height))
}

fn app() -> App {
    let mut config = Config::default();
    config.editor.pane_style = PaneStyle::Compact;
    config.editor.line_numbers = false;
    App::new(config, None).unwrap()
}

#[test]
fn compact_panes_reclaim_edges_and_update_cached_inactive_snapshots() {
    let mut app = app();
    seed(&mut app, "abcdefghijklmnopqrstuvwxyz\nlast row\n");
    let view = app.prepare_view(geometry(40, 10));
    let pane = &view.panes[0];
    assert_eq!(
        pane.body,
        Rect {
            x: 0,
            y: 1,
            width: 40,
            height: 7
        }
    );
    assert_eq!(app.pane_cells(pane.pane_id), (40, 7));
    app.split(Axis::Horizontal, None).unwrap();
    let view = app.prepare_view(geometry(40, 10));
    let snapshot = app.snapshot(&view);
    let mut panes = view.panes.iter().collect::<Vec<_>>();
    panes.sort_by_key(|p| p.area.x);
    assert_eq!(panes[0].body.width + panes[1].body.width, 39);
    assert!(!panes[0].left_separator);
    assert!(panes[1].left_separator);
    assert!(snapshot.panes.iter().all(|p| p.compact));
    // Rebuilding a prepared pane invalidates the inactive snapshot cache, too.
    app.config.editor.pane_style = PaneStyle::Framed;
    let view = app.prepare_view(geometry(40, 10));
    let snapshot = app.snapshot(&view);
    assert!(
        snapshot
            .panes
            .iter()
            .all(|p| !p.compact && !p.left_separator)
    );
    assert_eq!(view.panes.iter().map(|p| p.body.width).sum::<u16>(), 36);
    assert!(view.panes.iter().all(|p| p.body.height == 6));
}

#[test]
fn compact_resize_only_claims_separator_and_lower_title_not_reclaimed_content() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let mut app = app();
        app.config.editor.soft_wrap = false;
        seed(
            &mut app,
            &std::iter::repeat_n("abcdefghijklmnopqrstuvwxyz0123456789", 12)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        app.split(axis, None).unwrap();
        let view = app.prepare_view(geometry(40, 14));
        let mut panes = view.panes.iter().collect::<Vec<_>>();
        panes.sort_by_key(|p| (p.area.y, p.area.x));
        let (first, second) = (panes[0], panes[1]);
        let (x, y, content_x, content_y) = match axis {
            Axis::Horizontal => (second.area.x, 2, second.area.x - 1, 2),
            Axis::Vertical => (2, second.area.y, 2, second.area.y - 1),
        };
        assert_eq!(
            pointer_resize_pair(&view, x, y),
            Some((first.pane_id, second.pane_id, axis))
        );
        assert_eq!(pointer_resize_pair(&view, content_x, content_y), None);
        app.handle_pointer(
            PointerEvent {
                kind: PointerEventKind::Down(PointerButton::Left),
                column: content_x,
                row: content_y,
                modifiers: Modifiers::NONE,
            },
            &view,
        )
        .unwrap();
        assert!(!matches!(
            app.pointer_drag,
            Some(PointerDrag::Resize { .. })
        ));
        assert_eq!(app.active_pane, first.pane_id);
        let position = app.cursor_position();
        assert_eq!(position.row, usize::from(content_y - first.body.y));
        assert_eq!(position.col, usize::from(content_x - first.body.x));
    }
}

#[test]
fn compact_nested_splits_sidebar_and_maximization_have_no_duplicate_edges() {
    let mut app = app();
    app.split(Axis::Horizontal, None).unwrap();
    app.split(Axis::Vertical, None).unwrap();
    let view = app.prepare_view(geometry(100, 24));
    assert_eq!(view.panes.len(), 3);
    assert_eq!(view.panes.iter().filter(|p| p.left_separator).count(), 2);
    app.directory_tree.visible = true;
    let view = app.prepare_view(geometry(100, 24));
    let tree = view.tree_area.unwrap();
    let first = view.panes.iter().min_by_key(|p| p.area.x).unwrap();
    assert_eq!(first.body.x, tree.x + tree.width);
    assert!(!first.left_separator);
    app.execute_command("fullscreen").unwrap();
    let view = app.prepare_view(geometry(100, 24));
    assert_eq!(view.panes.len(), 1);
    assert_eq!(view.panes[0].body.width, 100);
    assert!(!view.panes[0].left_separator);
    app.config.editor.zen_width = 20;
    app.execute_command("zen").unwrap();
    let view = app.prepare_view(geometry(100, 24));
    assert_eq!(app.pane_cells(view.panes[0].pane_id), (20, 21));
    assert_eq!(view.panes[0].text_width, 20);
}

#[test]
fn compact_tiny_view_still_has_a_title_and_one_content_cell() {
    let mut app = app();
    let view = app.prepare_view(geometry(1, 4));
    assert!(view.panes[0].drawable);
    assert_eq!(
        view.panes[0].body,
        Rect {
            x: 0,
            y: 1,
            width: 1,
            height: 1
        }
    );
}

#[test]
fn compact_alignment_uses_reclaimed_bottom_row() {
    let mut app = app();
    seed(
        &mut app,
        &(0..40).map(|n| format!("line {n}\n")).collect::<String>(),
    );
    let target = app.buffers[0].offset_of(Position::new(20, 0));
    app.panes.get_mut(&0).unwrap().selection = Selection::point(target);
    app.prepare_view(geometry(40, 10));
    app.align_view(ViewAlignment::Bottom);
    let view = app.prepare_view(geometry(40, 10));
    assert_eq!(app.viewport_height(), 7);
    assert_eq!(app.snapshot(&view).panes[0].cursor_screen_row, Some(6));
}

#[test]
fn compact_config_roundtrip_and_invalid_style() {
    assert_eq!(Config::default().editor.pane_style, PaneStyle::Framed);
    let config: Config = serde_yaml::from_str("editor:\n  pane_style: compact\n").unwrap();
    assert_eq!(config.editor.pane_style, PaneStyle::Compact);
    let encoded = serde_yaml::to_string(&config).unwrap();
    let decoded: Config = serde_yaml::from_str(&encoded).unwrap();
    assert_eq!(decoded.editor.pane_style, PaneStyle::Compact);
    assert!(serde_yaml::from_str::<Config>("editor:\n  pane_style: none\n").is_err());
}

#[test]
fn compact_terminal_initial_size_and_live_resize_match_snapshot_body() {
    let root = crate::test_support::TestRuntimeRoot::new("compact-terminal").unwrap();
    let mut config = Config::default();
    config.editor.pane_style = PaneStyle::Compact;
    let mut app = App::new_in_project(config, None, &*root).unwrap();
    app.prepare_view(geometry(40, 10));
    app.open_terminal_at(Some(terminal_fixture_command()), root.to_path_buf());
    let id = app.active_terminal().unwrap();
    let cleanup = terminal_cleanup(&app, id);
    assert_eq!(app.terminals.get(id).unwrap().columns(), 40);
    for (style, columns, rows) in [(PaneStyle::Compact, 40, 7), (PaneStyle::Framed, 38, 6)] {
        app.config.editor.pane_style = style;
        let view = app.prepare_view(geometry(40, 10));
        let snapshot = app.snapshot(&view);
        let pane = &snapshot.panes[0];
        let terminal = pane.terminal.as_ref().unwrap();
        assert_eq!(terminal.columns, columns);
        assert_eq!(terminal.rows.len(), rows);
        assert_eq!(usize::from(pane.body.width), columns);
        assert_eq!(usize::from(pane.body.height), rows);
    }
    app.config.editor.pane_style = PaneStyle::Compact;
    app.config.editor.zen_width = 20;
    app.execute_command("zen").unwrap();
    let view = app.prepare_view(geometry(40, 10));
    let snapshot = app.snapshot(&view);
    let pane = &snapshot.panes[0];
    assert_eq!(
        pane.body,
        Rect {
            x: 10,
            y: 1,
            width: 20,
            height: 7
        }
    );
    assert_eq!(pane.terminal.as_ref().unwrap().columns, 20);
    assert_eq!(pane.terminal.as_ref().unwrap().rows.len(), 7);
    app.execute_command("zen").unwrap();
    app.prepare_view(geometry(40, 10));
    assert_eq!(app.terminals.get(id).unwrap().columns(), 40);
    drop(app);
    cleanup();
}

#[test]
fn compact_render_and_attached_transport_preserve_title_and_edge_content() {
    use ratatui::{Terminal, backend::TestBackend, style::Modifier};
    let mut app = app();
    app.config.editor.soft_wrap = false;
    app.config.editor.scroll_offset = 0;
    // Fill every body cell, including the newly reclaimed final row and columns.
    seed(
        &mut app,
        &std::iter::repeat_n("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", 7)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.active_mut().selection = Selection::point(0);
    let prepared = app.prepare_view(geometry(40, 10));
    let editor = app.snapshot(&prepared);
    let host = crate::workspace::HostFrame {
        id: crate::workspace::FrameId::from_raw(1),
        active_buffer: crate::workspace::BufferId::from_raw(1),
        active_revision: crate::workspace::BufferRevision::from_raw(1),
        editor: editor.clone(),
        overlays: app.overlay_snapshots(),
    };
    let wire = crate::protocol::HostFrame::from(host);
    let encoded = serde_json::to_vec(&wire).unwrap();
    let decoded: crate::protocol::HostFrame = serde_json::from_slice(&encoded).unwrap();
    let attached = crate::workspace::HostFrame::try_from(decoded).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    terminal
        .draw(|frame| {
            crate::ui::render_exact_colors_for_test(
                frame,
                &app,
                &editor,
                &crate::key_hints::KeyHintState::default(),
            )
        })
        .unwrap();
    let direct = terminal.backend().buffer().clone();
    assert_eq!(direct[(39, 7)].symbol(), "x");
    assert_eq!(direct[(0, 7)].symbol(), "x");
    assert_ne!(direct[(0, 0)].bg, direct[(0, 7)].bg);
    assert!(direct[(1, 0)].modifier.contains(Modifier::BOLD));
    terminal
        .draw(|frame| crate::ui::render_host_frame_exact_colors_for_test(frame, &attached))
        .unwrap();
    // Global status distinguishes standalone from persistent; pane cells agree.
    assert_eq!(
        &direct.content[..320],
        &terminal.backend().buffer().content[..320]
    );

    app.split(Axis::Horizontal, None).unwrap();
    let prepared = app.prepare_view(geometry(40, 10));
    let editor = app.snapshot(&prepared);
    let right = editor.panes.iter().max_by_key(|p| p.area.x).unwrap();
    let boundary = right.area.x;
    terminal
        .draw(|frame| {
            crate::ui::render_exact_colors_for_test(
                frame,
                &app,
                &editor,
                &crate::key_hints::KeyHintState::default(),
            )
        })
        .unwrap();
    let cells = terminal.backend().buffer();
    for row in 1..8 {
        assert_eq!(cells[(boundary, row)].symbol(), "│");
        assert_eq!(cells[(boundary - 1, row)].symbol(), "x");
        assert_eq!(cells[(boundary + 1, row)].symbol(), "x");
    }
    assert_eq!(cells[(boundary, 0)].symbol(), "│");
    assert!(!cells[(1, 0)].modifier.contains(Modifier::BOLD));
    assert!(cells[(boundary + 2, 0)].modifier.contains(Modifier::BOLD));
}

#[test]
fn compact_setting_preview_cancel_and_save_update_geometry_and_preserve_yaml() {
    let root = crate::test_support::TestRuntimeRoot::new("compact-settings").unwrap();
    let path = root.join("config.yaml");
    let original = "# keep\nunknown: kept\neditor:\n  pane_style: framed\n";
    fs::write(&path, original).unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.note_loaded_config(&path);
    for save in [false, true] {
        app.open_setting_values(SettingId::EditorPaneStyle);
        let compact = app
            .list_actions
            .iter()
            .position(|action| {
                matches!(
                    action,
                    ListAction::SettingValue {
                        value: SettingValue::PaneStyle(PaneStyle::Compact),
                        ..
                    }
                )
            })
            .unwrap();
        app.list.as_mut().unwrap().selected = compact;
        app.preview_selected_setting_value();
        assert_eq!(app.prepare_view(geometry(40, 10)).panes[0].body.width, 40);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        key(
            &mut app,
            if save {
                KeyCode::Enter
            } else {
                KeyCode::Escape
            },
            Modifiers::NONE,
        );
        assert_eq!(
            app.prepare_view(geometry(40, 10)).panes[0].body.width,
            if save { 40 } else { 38 }
        );
    }
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("# keep\nunknown: kept"));
    let (config, _) = Config::load(Some(&path)).unwrap();
    assert_eq!(config.editor.pane_style, PaneStyle::Compact);
}
