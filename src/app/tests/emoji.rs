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

#[test]
fn word_motions_select_whole_characters_and_never_split_a_word_at_a_mark() {
    let mut app = app_with("ab 👍🏽 cd\n");
    press(&mut app, 'e');
    press(&mut app, 'e');
    let range = app.active().selection.primary();
    assert_eq!(
        (range.from(), range.to()),
        (2, 3),
        "ends on the emoji's start"
    );

    // A decomposed accent belongs to the letter before it.
    let mut app = app_with("cafe\u{301} x\n");
    press(&mut app, 'w');
    let range = app.active().selection.primary();
    assert_eq!((range.from(), range.to()), (0, 5));
    press(&mut app, 'w');
    assert_eq!(app.active().selection.primary().from(), 6);
}

#[test]
fn text_objects_and_word_ends_cover_whole_characters() {
    let mut app = app_with("cafe\u{301} x\n");
    press(&mut app, 'm');
    press(&mut app, 'i');
    press(&mut app, 'w');
    press(&mut app, 'd');
    assert_eq!(text(&app), " x\n");

    let document = crate::text::Text::from_str("cafe\u{301} x");
    let word =
        crate::text_object::word(&document, 4, false, crate::text_object::Part::Inside).unwrap();
    assert_eq!((word.from, word.to), (0, 5));

    let app = app_with("👍🏽 x\n");
    assert_eq!(
        super::super::movement::word_bounds(app.active_buffer(), 0),
        (0, 2),
        "`*` searches for the whole emoji"
    );
    let mut app = app_with("👍🏽 x\n");
    press(&mut app, 'i');
    app.delete_word_forward();
    assert_eq!(text(&app), " x\n");
}

#[test]
fn character_scans_step_over_whole_emoji() {
    let app = app_with(&format!("a{SHRUG}b👍🏽c\nd"));
    let buffer = app.active_buffer();
    assert_eq!(
        super::super::movement::offsets_after(buffer, 0).collect::<Vec<_>>(),
        [1, 5, 6, 8, 10]
    );
    assert_eq!(
        super::super::movement::offsets_before(buffer, 10).collect::<Vec<_>>(),
        [8, 6, 5, 1, 0]
    );
}

#[test]
fn word_motions_from_an_accented_letter_match_its_precomposed_form() {
    // `ê` written as `e` and a combining circumflex must move exactly as the
    // single code point does, mapped back through the extra code points.
    let composed = "ab x\u{EA}y \u{EA}tre, z\n";
    let decomposed = "ab xe\u{302}y e\u{302}tre, z\n";
    let to_composed = |offset: usize| {
        let prefix = decomposed.chars().take(offset).collect::<String>();
        prefix
            .chars()
            .filter(|character| *character != '\u{302}')
            .count()
    };
    for selecting in [true, false] {
        for keys in ["w", "e", "b", "ww", "bb", "eb", "be", "W", "E", "B"] {
            for start in [4, 7, 8] {
                let run = |text: &str, offset: usize| {
                    let mut config = Config::default();
                    config.editor.selecting_motions = selecting;
                    let mut app = App::new(config, None).unwrap();
                    seed(&mut app, text);
                    app.panes.get_mut(&0).unwrap().selection = Selection::point(offset);
                    for key in keys.chars() {
                        press(&mut app, key);
                    }
                    let range = app.active().selection.primary();
                    (range.anchor, range.head)
                };
                // The offset of the same letter in the decomposed text.
                let decomposed_start = (0..decomposed.chars().count())
                    .find(|offset| {
                        to_composed(*offset) == start
                            && decomposed.chars().nth(*offset) != Some('\u{302}')
                    })
                    .unwrap();
                let (anchor, head) = run(decomposed, decomposed_start);
                for end in [anchor, head] {
                    assert_ne!(
                        decomposed.chars().nth(end),
                        Some('\u{302}'),
                        "{keys:?} from {start} rests inside a character"
                    );
                }
                assert_eq!(
                    (to_composed(anchor), to_composed(head)),
                    run(composed, start),
                    "{keys:?} from {start}, selecting motions {selecting}"
                );
            }
        }
    }
}
