// SPDX-License-Identifier: MPL-2.0

use super::*;

// `🤷‍♀️` is four code points: a base emoji, a zero-width joiner, a sign and a
// variation selector. None of the editor's carets may stop inside it.
const SHRUG: &str = "🤷\u{200D}♀\u{FE0F}";

fn app_with(text: &str) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, text);
    app
}

fn head(app: &App) -> usize {
    app.active().selection.primary().head
}

#[test]
fn horizontal_motion_crosses_whole_emoji() {
    let mut app = app_with(&format!("a{SHRUG}b😀👍🏽c\n"));
    let mut heads = vec![head(&app)];
    for _ in 0..5 {
        press(&mut app, 'l');
        heads.push(head(&app));
    }
    assert_eq!(heads, [0, 1, 5, 6, 7, 9]);
    for _ in 0..5 {
        press(&mut app, 'h');
    }
    assert_eq!(head(&app), 0);
}

#[test]
fn deleting_a_selected_emoji_removes_every_code_point() {
    let mut app = app_with(&format!("a{SHRUG}b\n"));
    press(&mut app, 'l');
    press(&mut app, 'd');
    assert_eq!(text(&app), "ab\n");
}

#[test]
fn insert_mode_backspace_and_delete_take_whole_emoji() {
    let mut app = app_with(&format!("x{SHRUG}{SHRUG}y\n"));
    press(&mut app, 'l');
    press(&mut app, 'l');
    press(&mut app, 'i');
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(text(&app), format!("x{SHRUG}y\n"));
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert_eq!(text(&app), "xy\n");
    key(&mut app, KeyCode::Char('z'), Modifiers::NONE);
    assert_eq!(text(&app), "xzy\n");
}

#[test]
fn line_end_and_vertical_motion_land_on_the_start_of_an_emoji() {
    let mut app = app_with(&format!("abcdefgh\n{SHRUG}{SHRUG}{SHRUG}\n"));
    app.motion(Motion::LineEnd);
    assert_eq!(head(&app), 7);
    for column in 0..8 {
        set_cursor(&mut app, 0, column);
        press(&mut app, 'j');
        let offset = head(&app) - app.active_buffer().line_to_offset(1);
        assert_eq!(offset % 4, 0, "column {column} landed inside an emoji");
    }
    app.motion(Motion::LineEnd);
    assert_eq!(head(&app) - app.active_buffer().line_to_offset(1), 8);
}

#[test]
fn a_caret_set_inside_an_emoji_clamps_to_its_start() {
    let app = app_with(&format!("a{SHRUG}\n"));
    let buffer = app.active_buffer();
    for offset in 1..5 {
        assert_eq!(buffer.clamp_offset(offset, false), 1);
        assert_eq!(buffer.clamp_offset(offset, true), 1);
    }
    assert_eq!(buffer.clamp_offset(5, true), 5);
}

#[test]
fn wrapping_and_screen_columns_measure_emoji_whole() {
    let line = format!("{SHRUG}{SHRUG}{SHRUG}");
    let segments = crate::wrap::segments(&line, 5, 4);
    assert_eq!(
        segments
            .iter()
            .map(|segment| (segment.start, segment.end))
            .collect::<Vec<_>>(),
        [(0, 8), (8, 12)]
    );
    assert_eq!(crate::wrap::display_column(&line, 4, 4), 2);
    assert_eq!(crate::wrap::display_column(&line, 8, 4), 4);
    // A click on either cell of an emoji resolves to its first code point.
    assert_eq!(crate::wrap::column_for_cell_from(&line, 0, 3, 4), 4);
    assert_eq!(crate::wrap::column_for_scrolled_cell(&line, 4, 1, 4), 4);
}

#[test]
fn prompt_editing_moves_and_deletes_whole_emoji() {
    let mut value = format!("a{SHRUG}b");
    let mut cursor = 5;
    assert_eq!(prompt_left(&value, cursor), 1);
    assert_eq!(prompt_right(&value, 1), 5);
    prompt_backspace(&mut value, &mut cursor);
    assert_eq!((value.as_str(), cursor), ("ab", 1));

    let mut value = format!("a{SHRUG}b");
    prompt_delete(&mut value, 1);
    assert_eq!(value, "ab");
}
