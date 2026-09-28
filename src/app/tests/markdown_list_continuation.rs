// SPDX-License-Identifier: MPL-2.0

use super::*;

fn markdown(text_value: &str) -> App {
    let mut config = Config::default();
    config.editor.smart_newline = true;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, text_value);
    app.mode = Mode::Insert;
    app.replace_active_selection(Selection::point(app.active_buffer().len_chars()));
    app
}

#[test]
fn markdown_enter_continues_bullets_numbers_letters_roman_and_tasks() {
    for (before, after) in [
        ("- item", "- item\n- "),
        ("* item", "* item\n* "),
        ("+ item", "+ item\n+ "),
        ("  -\titem", "  -\titem\n  -\t"),
        ("9. item", "9. item\n10. "),
        ("a. item", "a. item\nb. "),
        ("A. item", "A. item\nB. "),
        ("II. item", "II. item\nIII. "),
        ("IV. item", "IV. item\nV. "),
        ("- [ ] todo", "- [ ] todo\n- [ ] "),
        ("- [x] done", "- [x] done\n- [ ] "),
    ] {
        let mut app = markdown(before);
        app.edit_newline();
        assert_eq!(text(&app), after, "{before}");
        assert_eq!(app.active().selection.primary().head, after.chars().count());
        app.undo();
        assert_eq!(text(&app), before, "one undo step for {before}");
    }
}

#[test]
fn markdown_single_roman_letters_follow_a_preceding_roman_sibling() {
    for (before, after) in [
        ("I. first", "I. first\nJ. "),
        ("V. fifth", "V. fifth\nW. "),
        ("IV. fourth\nV. fifth", "IV. fourth\nV. fifth\nVI. "),
        ("IX. ninth\nX. tenth", "IX. ninth\nX. tenth\nXI. "),
        (
            "IV. fourth\n    continuation\nV. fifth",
            "IV. fourth\n    continuation\nV. fifth\nVI. ",
        ),
        (
            "IV. fourth\n  - nested\nV. fifth",
            "IV. fourth\n  - nested\nV. fifth\nVI. ",
        ),
        ("II. second\nV. fifth", "II. second\nV. fifth\nVI. "),
        (
            "II. second\nV. fifth\nX. tenth",
            "II. second\nV. fifth\nX. tenth\nXI. ",
        ),
        (
            "II. second\nA. first\nX. tenth",
            "II. second\nA. first\nX. tenth\nY. ",
        ),
        ("II.\nV. fifth", "II.\nV. fifth\nVI. "),
    ] {
        let mut app = markdown(before);
        app.edit_newline();
        assert_eq!(text(&app), after, "{before}");
    }
}

#[test]
fn markdown_empty_item_enter_ends_the_list_regardless_of_how_marker_was_typed() {
    for before in ["- ", "  * ", "10. ", "a. ", "II. ", "- [ ] "] {
        let mut app = markdown(before);
        app.edit_newline();
        assert_eq!(text(&app), "", "{before}");
        assert_eq!(app.active().selection.primary().head, 0);
        app.undo();
        assert_eq!(text(&app), before);
    }

    let mut app = markdown("1. First");
    app.edit_newline();
    assert_eq!(text(&app), "1. First\n2. ");
    app.edit_newline();
    assert_eq!(text(&app), "1. First\n");
}

#[test]
fn markdown_backspace_changes_empty_item_to_continuation_then_removes_alignment() {
    for (before, continuation, after) in [
        ("- ", "  ", ""),
        ("- first\n- ", "- first\n  ", "- first\n"),
        ("1. first\n10. ", "1. first\n    ", "1. first\n"),
        ("  9. first\n  10. ", "  9. first\n      ", "  9. first\n  "),
        ("\t-\tfirst\n\t-\t", "\t-\tfirst\n\t \t", "\t-\tfirst\n\t"),
    ] {
        let mut app = markdown(before);
        let original_revision = app.active_buffer().revision();
        app.edit_backspace();
        assert_eq!(text(&app), continuation, "{before}");
        let continuation_revision = app.active_buffer().revision();
        assert!(continuation_revision > original_revision);
        app.edit_backspace();
        assert_eq!(text(&app), after, "{before}");
        assert!(app.active_buffer().revision() > continuation_revision);
        app.undo();
        assert_eq!(text(&app), before, "Insert mode groups adjacent Backspaces");
    }
}

#[test]
fn markdown_task_backspace_keeps_tab_separator_and_visual_content_column() {
    let mut app = markdown("- [x]\ttext");
    app.edit_newline();
    assert_eq!(text(&app), "- [x]\ttext\n- [ ]\t");
    app.edit_backspace();
    assert_eq!(text(&app), "- [x]\ttext\n     \t");
    app.edit_backspace();
    assert_eq!(text(&app), "- [x]\ttext\n");

    let mut ordinary = markdown("  ");
    ordinary.edit_backspace();
    assert_eq!(
        text(&ordinary),
        " ",
        "unowned spaces keep character Backspace"
    );
}

#[test]
fn markdown_multicaret_backspace_uses_each_pre_edit_marker_and_alignment() {
    let original = "- first\n- \n1. second\n2. ";
    let mut app = markdown(original);
    app.replace_active_selection(Selection::new(
        vec![
            Range::point("- first\n- ".chars().count()),
            Range::point(original.chars().count()),
        ],
        0,
    ));
    app.edit_backspace();
    assert_eq!(text(&app), "- first\n  \n1. second\n   ");
    app.edit_backspace();
    assert_eq!(text(&app), "- first\n\n1. second\n");
    app.undo();
    assert_eq!(text(&app), original);
}

#[test]
fn markdown_enter_mid_item_moves_tail_to_next_item_but_marker_position_does_not() {
    let mut app = markdown("1. First");
    app.replace_active_selection(Selection::point(5));
    app.edit_newline();
    assert_eq!(text(&app), "1. Fi\n2. rst");

    for caret in [0, 1, 2] {
        let mut app = markdown("1. First");
        app.replace_active_selection(Selection::point(caret));
        app.edit_newline();
        assert!(!text(&app).contains("2. "), "caret {caret}");
    }
}

#[test]
fn markdown_letter_overflow_keeps_existing_hanging_indent() {
    for before in ["z. last", "Z. last"] {
        let mut app = markdown(before);
        app.edit_newline();
        assert_eq!(text(&app), format!("{before}\n   "));
    }
}

#[test]
fn list_keys_keep_existing_behavior_outside_markdown_or_without_smart_newline() {
    for smart in [false, true] {
        let mut config = Config::default();
        config.editor.smart_newline = smart;
        config.editor.scratch_markdown = false;
        let mut app = App::new(config, None).unwrap();
        seed(&mut app, "1. First");
        app.mode = Mode::Insert;
        app.replace_active_selection(Selection::point(app.active_buffer().len_chars()));
        app.edit_newline();
        assert_eq!(
            text(&app),
            if smart { "1. First\n   " } else { "1. First\n" }
        );
    }

    let mut non_markdown = Config::default();
    non_markdown.editor.smart_newline = true;
    non_markdown.editor.scratch_markdown = false;
    let mut app = App::new(non_markdown, None).unwrap();
    seed(&mut app, "- [x] task");
    app.mode = Mode::Insert;
    app.replace_active_selection(Selection::point(app.active_buffer().len_chars()));
    app.edit_newline();
    assert_eq!(text(&app), "- [x] task\n  ");

    let mut config = Config::default();
    config.editor.smart_newline = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "- ");
    app.mode = Mode::Insert;
    app.replace_active_selection(Selection::point(2));
    app.edit_newline();
    assert_eq!(text(&app), "- \n");
    app.undo();
    app.edit_backspace();
    assert_eq!(text(&app), "-");
}

#[test]
fn markdown_multicaret_newline_uses_each_pre_edit_item_and_one_undo_step() {
    let original = "9. first\n- [x] second";
    let mut app = markdown(original);
    let first = "9. first".chars().count();
    let second = original.chars().count();
    app.replace_active_selection(Selection::new(
        vec![Range::point(first), Range::point(second)],
        0,
    ));
    app.edit_newline();
    assert_eq!(text(&app), "9. first\n10. \n- [x] second\n- [ ] ");
    app.undo();
    assert_eq!(text(&app), original);
}

#[test]
fn markdown_file_continues_lists_even_when_scratch_markdown_is_disabled() {
    let root = crate::test_support::TestRuntimeRoot::new("markdown-list-file").unwrap();
    let path = root.path().join("notes.md");
    fs::write(&path, "- item").unwrap();
    let mut config = Config::default();
    config.editor.smart_newline = true;
    config.editor.scratch_markdown = false;
    let mut app = App::new(config, Some(path)).unwrap();
    app.mode = Mode::Insert;
    app.replace_active_selection(Selection::point(app.active_buffer().len_chars()));
    app.edit_newline();
    assert_eq!(text(&app), "- item\n- ");
}

#[test]
fn markdown_backspace_after_a_marker_unwinds_the_same_way_before_text() {
    let original = "1. First point\n2. Second point a bit longer to show the problem";
    let mut app = markdown(original);
    let split = "1. First point\n2. Second point a bit longer "
        .chars()
        .count();
    app.replace_active_selection(Selection::point(split));
    app.edit_newline();
    assert_eq!(
        text(&app),
        "1. First point\n2. Second point a bit longer \n3. to show the problem"
    );
    let content = split + "\n3. ".chars().count();
    assert_eq!(app.active().selection.primary().head, content);

    app.edit_backspace();
    assert_eq!(
        text(&app),
        "1. First point\n2. Second point a bit longer \n   to show the problem"
    );
    assert_eq!(app.active().selection.primary().head, content);

    app.edit_backspace();
    assert_eq!(
        text(&app),
        "1. First point\n2. Second point a bit longer \nto show the problem"
    );
    assert_eq!(app.active().selection.primary().head, split + 1);

    app.edit_backspace();
    assert_eq!(text(&app), original);
}

#[test]
fn markdown_backspace_at_an_existing_item_content_start_uses_character_columns() {
    for (before, caret, after) in [
        ("- żółw", 2, "  żółw"),
        ("\t- [x]\tżółw", 7, "\t     \tżółw"),
        ("ż\n  10. item", 8, "ż\n      item"),
    ] {
        let mut app = markdown(before);
        app.replace_active_selection(Selection::point(caret));
        app.edit_backspace();
        assert_eq!(text(&app), after, "{before}");
        assert_eq!(app.active().selection.primary().head, caret, "{before}");
    }

    for (before, caret, after) in [
        ("- żółw", 3, "- ółw"),
        ("1.  item", 3, "1. item"),
        ("  not a list", 2, " not a list"),
    ] {
        let mut app = markdown(before);
        app.replace_active_selection(Selection::point(caret));
        app.edit_backspace();
        assert_eq!(text(&app), after, "ordinary Backspace for {before}");
    }
}

fn newline_at(before: &str, carets: &[&str]) -> App {
    let mut app = markdown(before);
    let ranges = carets
        .iter()
        .map(|prefix| {
            assert!(before.starts_with(prefix), "{prefix:?} is a prefix");
            Range::point(prefix.chars().count())
        })
        .collect();
    app.replace_active_selection(Selection::new(ranges, 0));
    app
}

#[test]
fn markdown_enter_renumbers_following_items_and_backspace_restores_them() {
    let original = "1. First point\n2. Second point a bit longer to show the problem\n3. Some later point\n4. Last";
    let mut app = newline_at(original, &["1. First point\n2. Second point a bit longer "]);
    app.edit_newline();
    assert_eq!(
        text(&app),
        "1. First point\n2. Second point a bit longer \n3. to show the problem\n4. Some later point\n5. Last"
    );
    app.undo();
    assert_eq!(text(&app), original, "renumbering is part of one undo step");
    app.redo();

    app.edit_backspace();
    assert_eq!(
        text(&app),
        "1. First point\n2. Second point a bit longer \n   to show the problem\n3. Some later point\n4. Last"
    );
    app.edit_backspace();
    app.edit_backspace();
    assert_eq!(text(&app), original);
}

#[test]
fn markdown_enter_at_the_end_of_an_item_renumbers_the_same_way() {
    let mut app = newline_at("1. a\n2. b\n3. c", &["1. a"]);
    app.edit_newline();
    assert_eq!(text(&app), "1. a\n2. \n3. b\n4. c");
    app.edit_backspace();
    assert_eq!(text(&app), "1. a\n   \n2. b\n3. c");
    app.edit_backspace();
    assert_eq!(text(&app), "1. a\n\n2. b\n3. c");
}

#[test]
fn markdown_renumbering_follows_every_ordered_marker_style() {
    for (before, caret, after) in [
        ("a. x\nb. y\nc. z", "a. x", "a. x\nb. \nc. y\nd. z"),
        ("IV. x\nV. y\nVI. z", "IV. x", "IV. x\nV. \nVI. y\nVII. z"),
        (
            "III. x\nIV. y\nV. z",
            "III. x\nIV. y",
            "III. x\nIV. y\nV. \nVI. z",
        ),
        ("9. x\n10. y", "9. x", "9. x\n10. \n11. y"),
        (
            "1. x\r\n2. y\r\n3. z",
            "1. x",
            "1. x\r\n2. \r\n3. y\r\n4. z",
        ),
        (
            "- [x] 1. x\n- [ ] 2. y",
            "- [x] 1. x",
            "- [x] 1. x\n- [ ] \n- [ ] 2. y",
        ),
    ] {
        let mut app = newline_at(before, &[caret]);
        app.edit_newline();
        assert_eq!(text(&app), after, "{before}");
    }
}

#[test]
fn markdown_renumbering_keeps_numbers_that_were_not_in_sequence() {
    for (before, caret, after) in [
        ("1. a\n1. b\n1. c", "1. a", "1. a\n2. \n1. b\n1. c"),
        ("1. a\n2. b\n5. c", "1. a", "1. a\n2. \n3. b\n5. c"),
        (
            "1. a\n2. b\n- c\n3. d",
            "1. a",
            "1. a\n2. \n3. b\n- c\n3. d",
        ),
        (
            "1. a\n2. b\n\nparagraph\n\n3. c",
            "1. a",
            "1. a\n2. \n3. b\n\nparagraph\n\n3. c",
        ),
    ] {
        let mut app = newline_at(before, &[caret]);
        app.edit_newline();
        assert_eq!(text(&app), after, "{before}");
    }
}

#[test]
fn markdown_renumbering_passes_over_nested_items_continuations_and_blank_lines() {
    let before = "1. a\n2. b\n   continued\n   1. nested\n   2. nested\n\n3. c\n  4. deeper";
    let mut app = newline_at(before, &["1. a"]);
    app.edit_newline();
    assert_eq!(
        text(&app),
        "1. a\n2. \n3. b\n   continued\n   1. nested\n   2. nested\n\n4. c\n  4. deeper"
    );

    let before = "1. a\n   1. x\n   2. y\n2. b";
    let mut app = newline_at(before, &["1. a\n   1. x"]);
    app.edit_newline();
    assert_eq!(text(&app), "1. a\n   1. x\n   2. \n   3. y\n2. b");
}

#[test]
fn markdown_multicaret_enter_and_backspace_renumber_one_list_consistently() {
    let original = "1. a\n2. b\n3. c";
    let mut app = newline_at(original, &["1. a", "1. a\n2. b"]);
    app.edit_newline();
    assert_eq!(text(&app), "1. a\n2. \n3. b\n4. \n5. c");
    app.edit_backspace();
    assert_eq!(text(&app), "1. a\n   \n2. b\n   \n3. c");
    app.edit_backspace();
    assert_eq!(text(&app), "1. a\n\n2. b\n\n3. c");
    app.undo();
    assert_eq!(
        text(&app),
        original,
        "Insert mode groups the whole sequence"
    );

    let mut app = newline_at("8. a\n9. b\n10. c", &["8. a", "8. a\n9. b"]);
    app.edit_newline();
    let after = "8. a\n9. \n10. b\n11. \n12. c";
    assert_eq!(text(&app), after);
    let heads = app
        .active()
        .selection
        .ranges()
        .iter()
        .map(|range| range.head)
        .collect::<Vec<_>>();
    assert_eq!(
        heads,
        [
            "8. a\n9. ".chars().count(),
            "8. a\n9. \n10. b\n11. ".chars().count()
        ]
    );

    let original = "1. a\r\n2. \r\n3. \r\n4. d";
    let mut app = newline_at(original, &["1. a\r\n2. ", "1. a\r\n2. \r\n3. "]);
    app.edit_backspace();
    assert_eq!(text(&app), "1. a\r\n   \r\n   \r\n2. d");
}

#[test]
fn markdown_renumbering_continues_past_other_carets_unless_they_edit_a_marker() {
    // Carets elsewhere on a row leave its marker to be renumbered.
    let mut app = newline_at(
        "1. a\n2. b\n   cont\n3. c",
        &["1. a", "1. a\n2. b\n   cont"],
    );
    app.edit_newline();
    assert_eq!(text(&app), "1. a\n2. \n3. b\n   cont\n   \n4. c");

    let mut app = newline_at("1. a\n2. b\n3. c", &["1. a", "1. a\n2. b\n"]);
    app.edit_newline();
    assert_eq!(text(&app), "1. a\n2. \n3. b\n\n4. c");

    let mut app = newline_at("1. a\n2. b\n3. c\n4. d", &["1. a\n2. ", "1. a\n2. b\n3. c"]);
    app.edit_backspace();
    assert_eq!(text(&app), "1. a\n   b\n2. \n3. d");

    // A caret inside a marker stops the run above it rather than rewrite the
    // marker that caret is splitting.
    let mut app = newline_at("8. a\n9. b\n10. c", &["8. a", "8. a\n9. b\n1"]);
    app.edit_newline();
    assert_eq!(text(&app), "8. a\n9. \n10. b\n1\n0. c");
}

#[test]
fn markdown_enter_on_an_empty_item_renumbers_the_items_after_it() {
    let mut app = newline_at("1. a\n2. b\n3. c", &["1. a"]);
    app.edit_newline();
    assert_eq!(text(&app), "1. a\n2. \n3. b\n4. c");
    app.edit_newline();
    assert_eq!(text(&app), "1. a\n\n2. b\n3. c");
}

#[test]
fn markdown_backspace_renumbering_follows_letters_roman_numerals_and_tabs() {
    for (before, caret, after) in [
        ("a. x\nb. \nc. y", "a. x\nb. ", "a. x\n   \nb. y"),
        ("IV. x\nV. \nVI. y", "IV. x\nV. ", "IV. x\n   \nV. y"),
        (
            "\t1. x\n\t2. \n\t3. y",
            "\t1. x\n\t2. ",
            "\t1. x\n\t   \n\t2. y",
        ),
    ] {
        let mut app = newline_at(before, &[caret]);
        app.edit_backspace();
        assert_eq!(text(&app), after, "{before}");
    }

    for (before, caret, after) in [
        ("\t1. a\n\t2. b", "\t1. a", "\t1. a\n\t2. \n\t3. b"),
        ("x. a\ny. b\nz. c", "x. a", "x. a\ny. \nz. b\nz. c"),
    ] {
        let mut app = newline_at(before, &[caret]);
        app.edit_newline();
        assert_eq!(text(&app), after, "{before}");
    }
}

#[test]
fn markdown_renumbering_hands_its_roman_style_to_a_later_caret() {
    let original = "VIII. a\n\nIX. b\n\nX. c\n\nXI. d";
    let mut app = newline_at(original, &["VIII. a", "VIII. a\n\nIX. b\n\nX. c"]);
    app.edit_newline();
    assert_eq!(
        text(&app),
        "VIII. a\nIX. \n\nX. b\n\nXI. c\nXII. \n\nXIII. d"
    );

    let original = "VIII. a\nIX. \n\nX. \n\nXI. d";
    let mut app = newline_at(original, &["VIII. a\nIX. ", "VIII. a\nIX. \n\nX. "]);
    app.edit_backspace();
    assert_eq!(text(&app), "VIII. a\n    \n\n   \n\nIX. d");
}

#[test]
fn markdown_renumbering_leaves_a_marker_joined_onto_the_line_above() {
    for (original, joined) in [
        ("1. a\n2. b\n3. c\n4. d", "1. a\n   b\n2. c4. d"),
        ("1. a\r\n2. b\r\n3. c\r\n4. d", "1. a\r\n   b\r\n2. c4. d"),
    ] {
        let rows = original.split_inclusive('\n').collect::<Vec<_>>();
        let mut app = newline_at(
            original,
            &[&(rows[0].to_owned() + "2. "), &rows[..3].concat()],
        );
        app.edit_backspace();
        assert_eq!(text(&app), joined, "{original:?}");
    }
}
