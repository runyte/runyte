// SPDX-License-Identifier: MPL-2.0

use super::*;

fn type_keys(app: &mut App, keys: &str) {
    for character in keys.chars() {
        press(app, character);
    }
}

/// The text a Runyte selection covers, its head's character included.
fn inclusive_text(app: &App) -> String {
    let range = app.active().selection.primary();
    let buffer = app.active_buffer();
    buffer.slice(range.from(), (range.to() + 1).min(buffer.len_chars()))
}

fn plain_app(source: &str) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, source);
    app
}

#[test]
fn m_s_surrounds_the_selection_and_selects_the_pair_it_added() {
    let mut app = plain_app("let name = value;");
    set_cursor(&mut app, 0, 5);
    type_keys(&mut app, "miw");
    type_keys(&mut app, "ms\"");
    assert_eq!(text(&app), "let \"name\" = value;");
    assert_eq!(inclusive_text(&app), "\"name\"");
    assert_eq!(app.mode, Mode::Normal);

    press(&mut app, 'u');
    assert_eq!(text(&app), "let name = value;", "one undo step");

    // Either bracket names the pair; anything else is used on both sides.
    set_cursor(&mut app, 0, 11);
    type_keys(&mut app, "miwms)");
    assert_eq!(text(&app), "let name = (value);");
    set_cursor(&mut app, 0, 4);
    type_keys(&mut app, "ms*");
    assert_eq!(
        text(&app),
        "let *n*ame = (value);",
        "a bare caret wraps its character"
    );
    assert_eq!(inclusive_text(&app), "*n*");
}

#[test]
fn m_s_wraps_every_selection_including_adjacent_ones() {
    let mut app = plain_app("abcd");
    app.active_mut()
        .replace_selection(Selection::new(vec![Range::new(0, 1), Range::new(3, 2)], 1));
    type_keys(&mut app, "ms[");
    assert_eq!(text(&app), "[ab][cd]");
    assert_eq!(
        app.active().selection.ranges(),
        &[Range::new(0, 3), Range::new(7, 4)],
        "each range covers its own pair and keeps its direction"
    );
    assert_eq!(app.active().selection.primary_index(), 1);
}

#[test]
fn surround_add_preserves_the_primary_when_inclusive_spans_merge() {
    let mut app = plain_app("abcdef");
    app.active_mut().replace_selection(Selection::new(
        vec![Range::new(0, 1), Range::new(2, 1), Range::new(4, 5)],
        1,
    ));
    type_keys(&mut app, "ms[");
    assert_eq!(text(&app), "[abc]d[ef]");
    assert_eq!(app.active().selection.primary_index(), 0);
    assert_eq!(app.active().selection.primary(), Range::new(4, 0));
}

#[test]
fn m_r_replaces_the_named_or_closest_pair_around_the_cursor() {
    let mut app = plain_app("f(a, [b])");
    set_cursor(&mut app, 0, 6);
    type_keys(&mut app, "mr(");
    assert_eq!(app.status, "replace ( with …");
    press(&mut app, '{');
    assert_eq!(text(&app), "f{a, [b]}");
    assert_eq!(cursor(&app), Position::new(0, 6), "the cursor stays on b");

    type_keys(&mut app, "mrm\"");
    assert_eq!(text(&app), "f{a, \"b\"}", "m names the closest pair");

    press(&mut app, 'u');
    assert_eq!(text(&app), "f{a, [b]}");
}

#[test]
fn m_d_deletes_the_pair_around_the_cursor_or_the_pair_selected() {
    let mut app = plain_app("say(\"hi\")");
    set_cursor(&mut app, 0, 5);
    type_keys(&mut app, "md\"");
    assert_eq!(text(&app), "say(hi)");
    assert_eq!(cursor(&app), Position::new(0, 4));

    // `m a (` selects the pair; `m d (` deletes it rather than looking past
    // it for one further out.
    let mut nested = plain_app("((x))");
    set_cursor(&mut nested, 0, 2);
    type_keys(&mut nested, "ma(");
    assert_eq!(inclusive_text(&nested), "(x)");
    type_keys(&mut nested, "md(");
    assert_eq!(text(&nested), "(x)");

    let mut closest = plain_app("[x, {y}]");
    set_cursor(&mut closest, 0, 5);
    type_keys(&mut closest, "mdm");
    assert_eq!(text(&closest), "[x, y]");
}

#[test]
fn surround_edits_change_nothing_unless_every_cursor_has_a_pair() {
    let mut app = plain_app("(a) b");
    app.active_mut()
        .replace_selection(Selection::new(vec![Range::point(1), Range::point(4)], 0));
    type_keys(&mut app, "md(");
    assert_eq!(text(&app), "(a) b");
    assert!(app.status_error);
    assert_eq!(app.status, "no surrounding parentheses");

    type_keys(&mut app, "mdx");
    assert_eq!(text(&app), "(a) b");
    assert!(app.status.starts_with("no surround pair for x"));

    // Two cursors inside one pair edit it once.
    let mut shared = plain_app("(ab)");
    shared
        .active_mut()
        .replace_selection(Selection::new(vec![Range::point(1), Range::point(2)], 0));
    type_keys(&mut shared, "mr([");
    assert_eq!(text(&shared), "[ab]");
    type_keys(&mut shared, "md[");
    assert_eq!(text(&shared), "ab");
}

#[test]
fn escape_between_m_r_operands_cancels_the_replacement() {
    let mut app = plain_app("(x)");
    set_cursor(&mut app, 0, 1);
    type_keys(&mut app, "mr(");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.awaiting_character_command(), None);
    press(&mut app, 'l');
    assert_eq!(text(&app), "(x)");
    assert_eq!(
        cursor(&app),
        Position::new(0, 2),
        "l moved instead of replacing"
    );
}

#[test]
fn surround_pairs_resolve_through_the_syntax_tree() {
    let path = language::temporary("surround-syntax.rs");
    fs::write(&path, "fn f() { g(\")\", x); }\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let x = text(&app).find('x').unwrap();
    app.active_mut().replace_selection(Selection::point(x));

    // A balanced scan would pair the `(` after `g` with the `)` in the
    // string; the tree knows that one is text.
    type_keys(&mut app, "mr([");
    assert_eq!(text(&app), "fn f() { g[\")\", x]; }\n");
    fs::remove_file(path).unwrap();
}

#[test]
fn surround_edits_are_refused_in_a_read_only_buffer() {
    let mut app = plain_app("(x)");
    app.open_help();
    assert!(app.active_buffer().is_read_only());
    let help = text(&app);
    for keys in ["md(", "mr([", "ms*"] {
        app.mode = Mode::Normal;
        type_keys(&mut app, keys);
        assert_eq!(text(&app), help, "{keys}");
        assert!(app.status_error, "{keys}");
        assert_eq!(app.status, "help is read-only", "{keys}");
    }
}

fn atomic_surround_app(source: &str) -> App {
    let config = Config {
        keys: Some(serde_yaml::from_str(
            "bind: {normal: {F1: surround-add, F2: surround-replace, F3: surround-delete}, select: {F1: surround-add, F2: surround-replace, F3: surround-delete}}",
        ).unwrap()),
        ..Config::default()
    };
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, source);
    app
}

#[test]
fn surround_actions_preserve_mode_while_defaults_add_normal_entry() {
    for mode in [Mode::Normal, Mode::Select] {
        for (function, prefix, operands, expected) in [
            (1, "ms", "[", "([x])"),
            (2, "mr", "([", "[x]"),
            (3, "md", "(", "x"),
        ] {
            let mut atomic = atomic_surround_app("(x)");
            let mut default = plain_app("(x)");
            for app in [&mut atomic, &mut default] {
                set_cursor(app, 0, 1);
                app.mode = mode;
            }
            key(&mut atomic, KeyCode::Function(function), Modifiers::NONE);
            type_keys(&mut default, prefix);
            assert_eq!(atomic.mode, mode);
            assert_eq!(default.mode, mode, "operand wait cannot run the mode tail");
            type_keys(&mut atomic, operands);
            type_keys(&mut default, operands);
            assert_eq!(text(&atomic), expected);
            assert_eq!(text(&default), expected);
            assert_eq!(atomic.mode, mode);
            assert_eq!(default.mode, Mode::Normal);
            assert_eq!(atomic.active().selection, default.active().selection);
            press(&mut default, 'u');
            assert_eq!(text(&default), "(x)", "default is one undo unit");
        }
    }
}

#[test]
fn surround_failed_or_cancelled_operands_do_not_run_the_mode_tail() {
    for prefix in ["ms", "mr", "mr(", "md"] {
        let mut app = plain_app("(x)");
        set_cursor(&mut app, 0, 1);
        app.mode = Mode::Select;
        type_keys(&mut app, prefix);
        key(&mut app, KeyCode::Escape, Modifiers::NONE);
        assert_eq!(app.mode, Mode::Select, "{prefix}");
        assert_eq!(text(&app), "(x)");
        press(&mut app, 'l');
        assert_eq!(text(&app), "(x)", "no resumed operand after cancellation");
    }
    for keys in ["md(", "mr([", "mdz"] {
        let mut app = plain_app("x");
        app.mode = Mode::Select;
        type_keys(&mut app, keys);
        assert!(app.status_error, "{keys}");
        assert_eq!(app.mode, Mode::Select, "{keys}");
        assert_eq!(text(&app), "x");
    }
    for keys in ["ms[", "mr([", "md("] {
        let mut app = plain_app("(x)");
        app.open_help();
        app.mode = Mode::Select;
        let original = text(&app);
        type_keys(&mut app, keys);
        assert!(app.status_error, "{keys}");
        assert_eq!(app.mode, Mode::Select, "{keys}");
        assert_eq!(text(&app), original);
    }
}

#[test]
fn atomic_surround_retires_transient_line_mode_before_later_motion() {
    let mut app = atomic_surround_app("abc\ndef");
    press(&mut app, 'x');
    let mode = app.mode;
    assert!(app.line_select.is_some());
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    press(&mut app, '[');
    assert_eq!(app.mode, mode);
    assert!(app.line_select.is_none());
    press(&mut app, 'l');
    assert_eq!(
        app.mode, mode,
        "a later motion must not restore a stale mode"
    );
}
