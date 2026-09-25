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
