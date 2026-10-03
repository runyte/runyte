// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn left_click_and_drag_use_viewport_relative_tabs_after_horizontal_scroll() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    config.editor.tab_width = 4;
    config.editor.soft_wrap = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "ab\tz-more-text");
    app.active_mut().replace_selection(Selection::point(2));
    app.active_mut().scroll_col = 2;
    app.active_mut().preserve_scroll = true;
    let geometry = FrameGeometry {
        screen: Rect {
            width: 12,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 12,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let pane = view.pane(0).unwrap();
    assert_eq!(pane.scroll_col, 2);
    let event = |kind, cell| PointerEvent {
        kind,
        column: pane.body.x + pane.gutter_width as u16 + cell,
        row: pane.body.y,
        modifiers: Modifiers::NONE,
    };
    app.handle_pointer(event(PointerEventKind::Down(PointerButton::Left), 3), &view)
        .unwrap();
    assert_eq!(
        app.active().head(),
        2,
        "all four visible tab cells name the tab"
    );
    app.handle_pointer(event(PointerEventKind::Drag(PointerButton::Left), 4), &view)
        .unwrap();
    assert_eq!(app.active().selection.primary(), Range::new(2, 3));
}
