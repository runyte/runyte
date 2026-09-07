// SPDX-License-Identifier: MPL-2.0

use super::*;

fn geometry() -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            width: 24,
            height: 10,
            ..Rect::default()
        },
        editor: Rect {
            width: 24,
            height: 8,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    }
}

fn document(text: &str, wrap: bool) -> App {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    config.editor.soft_wrap = wrap;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, text);
    app
}

fn pointer(app: &mut App, kind: PointerEventKind, row: u16) {
    let view = app.prepare_view(geometry());
    let column = view.pane(app.active_pane).unwrap().body.x + 2;
    app.handle_pointer(
        PointerEvent {
            kind,
            column,
            row,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
}

fn tick(app: &mut App) -> bool {
    let now = Instant::now();
    let due = now
        + app
            .pointer_autoscroll_delay(now)
            .expect("edge drag has a deadline");
    app.advance_pointer_autoscroll(due, geometry())
}

#[test]
fn mouse_autoscroll_keeps_extending_without_more_motion_and_reverses() {
    let text = (0..40).map(|n| format!("line {n}\n")).collect::<String>();
    let mut app = document(&text, false);
    assert!(app.pointer_autoscroll_delay(Instant::now()).is_none());
    pointer(&mut app, PointerEventKind::Down(PointerButton::Left), 3);
    let anchor = app.active().selection.primary().anchor;
    pointer(&mut app, PointerEventKind::Drag(PointerButton::Left), 6);
    let start = app.active().scroll_row;
    let head = app.active().selection.primary().head;
    assert!(tick(&mut app));
    assert!(tick(&mut app));
    assert_eq!(app.active().scroll_row, start + 2);
    assert_eq!(app.active().selection.primary().anchor, anchor);
    assert!(app.active().selection.primary().head > head);
    assert_eq!(app.mode, Mode::Select);

    // Crossing the top border retains the original anchor and selects upward.
    pointer(&mut app, PointerEventKind::Drag(PointerButton::Left), 0);
    assert!(tick(&mut app));
    assert!(tick(&mut app));
    assert_eq!(app.active().scroll_row, 0);
    assert_eq!(cursor(&app).row, 0);
    assert_eq!(app.active().selection.primary().anchor, anchor);
    assert!(!tick(&mut app));
    assert!(app.pointer_autoscroll_delay(Instant::now()).is_none());
}

#[test]
fn mouse_autoscroll_stops_on_interior_release_keyboard_and_overlay() {
    let mut app = document(&"abcdef\n".repeat(40), false);
    for ending in 0..4 {
        pointer(&mut app, PointerEventKind::Down(PointerButton::Left), 3);
        pointer(&mut app, PointerEventKind::Drag(PointerButton::Left), 7);
        assert!(app.pointer_autoscroll_delay(Instant::now()).is_some());
        match ending {
            0 => pointer(&mut app, PointerEventKind::Drag(PointerButton::Left), 3),
            1 => pointer(&mut app, PointerEventKind::Up(PointerButton::Left), 9),
            2 => {
                app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
                    .unwrap();
            }
            _ => app.mode = Mode::Command,
        }
        let before = (app.active().scroll_row, app.active().selection.clone());
        assert!(app.pointer_autoscroll_delay(Instant::now()).is_none());
        assert!(
            !app.advance_pointer_autoscroll(Instant::now() + Duration::from_secs(1), geometry())
        );
        assert_eq!(
            (app.active().scroll_row, app.active().selection.clone()),
            before
        );
    }
}

#[test]
fn mouse_autoscroll_uses_wrapped_rows_and_stops_at_document_end() {
    let mut app = document(&"界abcdef ".repeat(50), true);
    pointer(&mut app, PointerEventKind::Down(PointerButton::Left), 2);
    pointer(&mut app, PointerEventKind::Drag(PointerButton::Left), 8);
    let head = app.active().selection.primary().head;
    assert!(tick(&mut app));
    assert_eq!(app.active().scroll_row, 0);
    assert_eq!(app.active().scroll_wrap, 1);
    assert!(app.active().selection.primary().head > head);
    for _ in 0..100 {
        if !tick(&mut app) {
            break;
        }
    }
    assert!(app.pointer_autoscroll_delay(Instant::now()).is_none());
    assert_eq!(cursor(&app).row, 0);
    // Blank rows below EOF still extend to the last projected text row.
    assert!(app.active().selection.primary().head > head);
}

#[test]
fn mouse_autoscroll_is_paced_and_cannot_retarget_a_replaced_buffer() {
    let mut app = document(&"abcdef\n".repeat(40), false);
    pointer(&mut app, PointerEventKind::Down(PointerButton::Left), 3);
    pointer(&mut app, PointerEventKind::Drag(PointerButton::Left), 7);
    let now = Instant::now();
    let due = now + app.pointer_autoscroll_delay(now).unwrap();
    assert!(!app.advance_pointer_autoscroll(due - Duration::from_millis(1), geometry()));
    assert!(app.advance_pointer_autoscroll(due + Duration::from_secs(5), geometry()));
    assert_eq!(
        app.active().scroll_row,
        1,
        "late ticks advance only one row"
    );
    let buffer = app.buffers.len();
    app.buffers.push(Buffer::scratch());
    app.active_mut().buffer = buffer;
    assert!(app.pointer_autoscroll_delay(Instant::now()).is_none());
    assert!(!app.advance_pointer_autoscroll(due + Duration::from_secs(6), geometry()));
    assert_eq!(app.buffers[buffer].to_string(), "");
}

#[test]
fn mouse_autoscroll_rechecks_resized_geometry_and_stops_when_the_pane_is_hidden() {
    for height in [2, 20] {
        let mut app = document(&"abcdef\n".repeat(40), false);
        pointer(&mut app, PointerEventKind::Down(PointerButton::Left), 3);
        pointer(&mut app, PointerEventKind::Drag(PointerButton::Left), 6);
        let now = Instant::now();
        let due = now + app.pointer_autoscroll_delay(now).unwrap();
        let mut resized = geometry();
        resized.screen.height = height + 2;
        resized.editor.height = height;
        assert!(!app.advance_pointer_autoscroll(due, resized));
        assert!(app.pointer_autoscroll_delay(due).is_none());
        assert_eq!(app.active().scroll_row, 0);
    }
}

#[test]
fn mouse_autoscroll_skips_folds_and_keeps_other_panes_still() {
    let path = temporary("autoscroll-fold.rs");
    fs::write(
        &path,
        format!(
            "fn folded() {{\n    let one = 1;\n    let two = 2;\n}}\n{}",
            "after\n".repeat(30)
        ),
    )
    .unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.fold_all_syntax();
    app.panes.insert(1, Pane::new(0));
    app.layout = Layout::Split {
        axis: Axis::Horizontal,
        ratio: u16::MAX / 2,
        first: Box::new(Layout::Pane(0)),
        second: Box::new(Layout::Pane(1)),
    };
    pointer(&mut app, PointerEventKind::Down(PointerButton::Left), 1);
    pointer(&mut app, PointerEventKind::Drag(PointerButton::Left), 7);
    assert!(tick(&mut app));
    assert_eq!(app.panes[&0].scroll_row, 3);
    assert_eq!(app.panes[&1].scroll_row, 0);
    assert_eq!(app.active_pane, 0);
    fs::remove_file(path).unwrap();
}
