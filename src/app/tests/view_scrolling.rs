// SPDX-License-Identifier: MPL-2.0
use super::*;

fn geometry() -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 16,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 14,
        },
        status: Rect {
            x: 0,
            y: 14,
            width: 40,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 15,
            width: 40,
            height: 1,
        },
    }
}

#[test]
fn view_scrolling_keeps_focus_visible_and_retains_position_after_exit() {
    for prefix in ['z', 'Z'] {
        for exit in [KeyCode::Escape, KeyCode::Enter] {
            let mut config = Config::default();
            config.editor.soft_wrap = false;
            let mut app = App::new(config, None).unwrap();
            seed(&mut app, &"abcdef\n".repeat(100));
            app.config.focus.enabled = true;
            set_cursor(&mut app, 20, 3);
            app.prepare_view(geometry());
            app.align_view(ViewAlignment::Center);
            let initial = app.active().head();
            press(&mut app, prefix);
            press(&mut app, 'j');
            assert_eq!(
                app.active().head(),
                initial,
                "caret inside margins stays put"
            );
            for direction in ['j', 'k'] {
                for _ in 0..25 {
                    if prefix == 'z' {
                        press(&mut app, prefix);
                    }
                    press(&mut app, direction);
                    let prepared = app.prepare_view(geometry());
                    let snapshot = app.snapshot(&prepared);
                    assert!(snapshot.panes[0].cursor_screen_row.is_some());
                }
            }
            let before = (app.active().scroll_row, app.active().head());
            key(&mut app, exit, Modifiers::NONE);
            app.prepare_view(geometry());
            assert_eq!((app.active().scroll_row, app.active().head()), before);
        }
    }
}

#[test]
fn view_scrolling_tracks_wrapped_rows_and_preserves_select_anchor() {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, &"abcdef ".repeat(250));
    app.prepare_view(geometry());
    app.mode = Mode::Select;
    press(&mut app, 'Z');
    for _ in 0..15 {
        press(&mut app, 'j');
        let prepared = app.prepare_view(geometry());
        assert!(app.snapshot(&prepared).panes[0].cursor_screen_row.is_some());
        assert_eq!(app.active().selection.primary().anchor, 0);
    }
    assert!(app.active().scroll_wrap > 0);
    assert!(app.active().head() > 0);
    let before = (app.active().scroll_wrap, app.active().head());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.prepare_view(geometry());
    assert_eq!((app.active().scroll_wrap, app.active().head()), before);
}

#[test]
fn view_scrolling_preserves_secondary_selections_and_wheel_scrolling() {
    let mut config = Config::default();
    config.editor.soft_wrap = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, &"abcdef\n".repeat(100));
    let secondary = Range::point(app.active_buffer().line_to_offset(80));
    app.replace_active_selection(Selection::new(vec![Range::point(0), secondary], 0));
    app.prepare_view(geometry());
    press(&mut app, 'z');
    press(&mut app, 'j');
    assert!(app.active().head() > 0);
    assert!(app.active().selection.ranges().contains(&secondary));
    let before = app.active().selection.clone();
    app.scroll_pane(app.active_pane, 1);
    assert_eq!(app.active().selection, before);
}

#[test]
fn view_scrolling_preserves_column_on_the_bottom_margin() {
    let mut config = Config::default();
    config.editor.soft_wrap = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, &"abcdef\n".repeat(100));
    app.prepare_view(geometry());
    let height = app.viewport_height();
    let margin = app.config.editor.scroll_offset;
    app.active_mut().scroll_row = 10;
    set_cursor(&mut app, 9 + height - margin - 1, 4);
    let before = app.active().head();
    press(&mut app, 'Z');
    press(&mut app, 'k');
    assert_eq!(app.active().scroll_row, 9);
    assert_eq!(app.active().head(), before);
}

#[test]
fn view_scrolling_skips_folded_lines_and_stops_at_document_edges() {
    let path = temporary("view-scroll-fold.rs");
    fs::write(
        &path,
        format!(
            "fn folded() {{\n{}\n}}\n{}",
            "    work();\n".repeat(20),
            "after();\n".repeat(30)
        ),
    )
    .unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.fold_all_syntax();
    app.prepare_view(geometry());
    assert!(!app.resolved_folds(0).is_empty());
    press(&mut app, 'Z');
    for direction in ['j', 'k'] {
        for _ in 0..70 {
            press(&mut app, direction);
            let prepared = app.prepare_view(geometry());
            assert!(app.snapshot(&prepared).panes[0].cursor_screen_row.is_some());
            assert!(fold_hiding_row(&app.resolved_folds(0), app.cursor_position().row).is_none());
        }
    }
    assert_eq!(app.active().scroll_row, 0);
    fs::remove_file(path).unwrap();
}

#[test]
fn view_scrolling_keeps_cursor_visible_in_tiny_panes() {
    for body_height in [1, 2, 4] {
        for exit in [KeyCode::Escape, KeyCode::Enter] {
            let mut config = Config::default();
            config.editor.soft_wrap = false;
            config.editor.scroll_offset = 10;
            let mut app = App::new(config, None).unwrap();
            seed(&mut app, &"abcdef\n".repeat(100));
            let mut geometry = geometry();
            geometry.editor.height = body_height + 2;
            app.prepare_view(geometry);
            press(&mut app, 'Z');
            for _ in 0..15 {
                press(&mut app, 'j');
                let prepared = app.prepare_view(geometry);
                assert!(app.snapshot(&prepared).panes[0].cursor_screen_row.is_some());
            }
            let before = (app.active().scroll_row, app.active().head());
            key(&mut app, exit, Modifiers::NONE);
            app.prepare_view(geometry);
            assert_eq!((app.active().scroll_row, app.active().head()), before);
        }
    }
}
