// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn word_and_character_motions_handle_unicode_and_lines() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "αβ, γδ\n\nx end");

    press(&mut app, 'e');
    assert_eq!(cursor(&app).col, 1);
    press(&mut app, 'W');
    assert_eq!(cursor(&app), Position::new(0, 4));
    press(&mut app, 'f');
    press(&mut app, 'x');
    assert_eq!(cursor(&app), Position::new(2, 0));
    press(&mut app, 'F');
    press(&mut app, 'α');
    assert_eq!(cursor(&app), Position::default());

    press(&mut app, 'h');
    assert_eq!(cursor(&app), Position::default());
    key(&mut app, KeyCode::End, Modifiers::NONE);
    assert_eq!(cursor(&app).col, 5);
}

#[test]
fn runyte_word_motions_require_explicit_select_mode() {
    for motion in ['w', 'b', 'e', 'W', 'B', 'E'] {
        let mut normal = App::new(Config::default(), None).unwrap();
        seed(&mut normal, "alpha, βeta gamma\nnext row");
        set_cursor(&mut normal, 0, 8);
        press(&mut normal, motion);

        assert_eq!(normal.mode, Mode::Normal, "plain {motion}");
        assert!(
            normal.active().selection.primary().is_empty(),
            "plain {motion} must move the caret without selecting"
        );

        let mut selecting = App::new(Config::default(), None).unwrap();
        seed(&mut selecting, "alpha, βeta gamma\nnext row");
        set_cursor(&mut selecting, 0, 8);
        press(&mut selecting, 'v');
        press(&mut selecting, motion);

        assert_eq!(selecting.mode, Mode::Select, "v {motion}");
        assert!(
            !selecting.active().selection.primary().is_empty(),
            "v {motion} must extend the selection"
        );
    }
}

#[test]
fn paragraph_motions_cross_blank_line_runs_and_stop_at_document_edges() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\ncontinued\n\n\nbravo\n\ncharlie");
    set_cursor(&mut app, 0, 2);

    press(&mut app, 'g');
    press(&mut app, 'p');
    assert_eq!(cursor(&app), Position::new(4, 0));
    press(&mut app, 'g');
    press(&mut app, 'p');
    assert_eq!(cursor(&app), Position::new(6, 0));
    press(&mut app, 'g');
    press(&mut app, 'p');
    assert_eq!(cursor(&app), Position::new(6, 6));

    press(&mut app, 'g');
    press(&mut app, 'P');
    assert_eq!(cursor(&app), Position::new(6, 0));
    press(&mut app, 'g');
    press(&mut app, 'P');
    assert_eq!(cursor(&app), Position::new(4, 0));
    press(&mut app, 'g');
    press(&mut app, 'P');
    assert_eq!(cursor(&app), Position::new(0, 0));
    press(&mut app, 'g');
    press(&mut app, 'P');
    assert_eq!(cursor(&app), Position::new(0, 0));
}

#[test]
fn paragraph_motions_accept_counts_and_extend_in_select_mode() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "one\n\ntwo\n\nthree");

    press(&mut app, '2');
    press(&mut app, 'g');
    press(&mut app, 'p');
    assert_eq!(cursor(&app), Position::new(4, 0));

    set_cursor(&mut app, 0, 0);
    press(&mut app, 'v');
    press(&mut app, 'g');
    press(&mut app, 'p');
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.active().selection.primary(), Range::new(0, 5));
}

/// A line break ends a word. Without that, the last word of a row and the
/// first word of the next row scan as one word and `e` steps past the row
/// the cursor started on.
#[test]
fn word_end_stops_at_the_end_of_the_row_it_started_on() {
    let cases = [
        ("alpha\n\nbravo", Position::new(0, 4)),
        ("alpha\nbravo", Position::new(0, 4)),
        ("alpha\n\n\n  bravo", Position::new(0, 4)),
    ];

    for (seeded, expected) in cases {
        for motion in ['e', 'E'] {
            let mut app = App::new(Config::default(), None).unwrap();
            seed(&mut app, seeded);

            press(&mut app, motion);

            assert_eq!(cursor(&app), expected, "{motion} in {seeded:?}");
        }
    }
}

/// From the end of a word the next `e` crosses the blank rows and lands on
/// the end of the following word, not its start.
#[test]
fn word_end_from_a_word_end_crosses_blank_rows_to_the_next_word_end() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\n\nbravo charlie");
    set_cursor(&mut app, 0, 4);

    press(&mut app, 'e');
    assert_eq!(cursor(&app), Position::new(2, 4));
    press(&mut app, 'e');
    assert_eq!(cursor(&app), Position::new(2, 12));
}

/// The same boundary governs `w`: the word after the one at the end of a
/// row is the first word of the next non-empty row.
#[test]
fn word_forward_from_the_last_word_of_a_row_lands_on_the_next_rows_word() {
    let cases = [
        ("alpha\nbravo", Position::new(1, 0)),
        ("alpha\n\nbravo", Position::new(2, 0)),
        ("alpha\n\n  bravo", Position::new(2, 2)),
    ];

    for (seeded, expected) in cases {
        for motion in ['w', 'W'] {
            let mut app = App::new(Config::default(), None).unwrap();
            seed(&mut app, seeded);

            press(&mut app, motion);

            assert_eq!(cursor(&app), expected, "{motion} in {seeded:?}");
        }
    }
}

#[test]
fn word_back_from_an_empty_final_row_stops_on_the_previous_line() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "one\ntwo\nthree\nfour\n");
    set_cursor(&mut app, 4, 0);

    press(&mut app, 'b');

    assert_eq!(cursor(&app), Position::new(3, 0));
}

#[test]
fn select_mode_extends_backwards_and_editing_orders_the_range() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abcd");
    set_cursor(&mut app, 0, 2);

    press(&mut app, 'v');
    press(&mut app, 'h');
    let range = app.active().selection.primary();
    assert_eq!((range.anchor, range.head), (2, 1));
    key(&mut app, KeyCode::Char(';'), Modifiers::ALT);
    let range = app.active().selection.primary();
    assert_eq!((range.anchor, range.head), (1, 2));
    press(&mut app, 'd');

    assert_eq!(text(&app), "ad");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(cursor(&app), Position::new(0, 1));
}

#[test]
fn delete_on_an_empty_row_removes_its_line_ending() {
    let cases = [
        ("\nalpha", 0, "alpha", Position::new(0, 0)),
        ("alpha\n\nbravo", 1, "alpha\nbravo", Position::new(1, 0)),
        ("alpha\nbravo\n", 2, "alpha\nbravo", Position::new(1, 4)),
    ];

    for (before, row, after, position) in cases {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, before);
        set_cursor(&mut app, row, 0);

        press(&mut app, 'd');

        assert_eq!(text(&app), after, "deleting row {row} in {before:?}");
        assert_eq!(cursor(&app), position);
        assert_eq!(app.read_selected_register().text, "\n");

        press(&mut app, 'u');
        assert_eq!(text(&app), before, "the deletion is one undo step");
    }
}

#[test]
fn delete_in_a_truly_empty_buffer_remains_a_no_op() {
    let mut app = App::new(Config::default(), None).unwrap();

    press(&mut app, 'd');

    assert_eq!(text(&app), "");
    assert_eq!(app.active_buffer().history_len(), 0);
}

/// Rows the primary selection covers, inclusive of both ends.
fn selected_rows(app: &App) -> (usize, usize) {
    let buffer = app.active_buffer();
    let range = app.active().selection.primary();
    (
        buffer.offset_to_row(range.from()),
        buffer.offset_to_row(range.to()),
    )
}

#[test]
fn line_selection_walks_one_edge_in_both_directions() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo\ncharlie\ndelta");
    set_cursor(&mut app, 1, 3);

    press(&mut app, 'x');
    assert_eq!(selected_rows(&app), (1, 1));
    press(&mut app, 'x');
    assert_eq!(selected_rows(&app), (1, 2));

    // `X` retraces the edge `x` walked before it starts consuming rows
    // above the line the walk began on.
    press(&mut app, 'X');
    assert_eq!(selected_rows(&app), (1, 1));
    press(&mut app, 'X');
    assert_eq!(selected_rows(&app), (0, 1));
    press(&mut app, 'x');
    assert_eq!(selected_rows(&app), (1, 1));
}

#[test]
fn line_selection_down_up_down_preserves_exact_direction() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "aa\nbbb\ncccc\ndd");
    set_cursor(&mut app, 1, 1);

    for (key, expected) in [
        ('x', Range::new(3, 5)),
        ('x', Range::new(3, 10)),
        ('X', Range::new(3, 5)),
        ('x', Range::new(3, 10)),
    ] {
        press(&mut app, key);
        assert_eq!(app.active().selection.primary(), expected, "after {key}");
    }
}

#[test]
fn line_selection_up_down_up_preserves_exact_direction() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "aa\nbbb\ncccc\ndd");
    set_cursor(&mut app, 1, 1);

    for (key, expected) in [
        ('X', Range::new(3, 5)),
        ('X', Range::new(5, 0)),
        ('x', Range::new(3, 5)),
        ('X', Range::new(5, 0)),
    ] {
        press(&mut app, key);
        assert_eq!(app.active().selection.primary(), expected, "after {key}");
    }
}

#[test]
fn line_selection_extends_from_an_empty_line() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\n\nbravo");
    set_cursor(&mut app, 1, 0);

    // The empty line has no character to highlight, but it still anchors
    // the walk, so the next press reaches the line below.
    press(&mut app, 'x');
    assert_eq!(selected_rows(&app), (1, 1));
    press(&mut app, 'x');
    assert_eq!(selected_rows(&app), (1, 2));
    assert_eq!(app.selection_text(), "\nbravo");
}

#[test]
fn line_selection_ends_at_the_first_other_command() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo\ncharlie");
    set_cursor(&mut app, 0, 0);

    press(&mut app, 'x');
    assert_eq!(app.mode, Mode::Select);
    press(&mut app, 'j');
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.active().selection.primary().is_empty());
    assert_eq!(cursor(&app).row, 1);

    press(&mut app, 'x');
    press(&mut app, 'k');
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.active().selection.primary().is_empty());
    assert_eq!(cursor(&app).row, 0);
}

#[test]
fn line_selection_hands_select_mode_back_to_v() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo\ncharlie");
    set_cursor(&mut app, 0, 0);

    press(&mut app, 'v');
    press(&mut app, 'x');
    assert_eq!(selected_rows(&app), (0, 0));
    // `v` was explicit, so it outlives the line selection and `j` keeps
    // extending.
    press(&mut app, 'j');
    assert_eq!(app.mode, Mode::Select);
    assert!(!app.active().selection.primary().is_empty());
}

#[test]
fn space_p_j_joins_a_line_selection_with_the_typed_delimiter() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo\ncharlie\ndelta");
    set_cursor(&mut app, 0, 0);
    for _ in 0..3 {
        press(&mut app, 'x');
    }

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 'j');
    assert_eq!(app.prompt_kind, PromptKind::JoinDelimiter);
    // The line selection survives the prompt: the delimiter is typed into
    // the prompt, not into the buffer.
    press(&mut app, ',');
    press(&mut app, ' ');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    // The unselected line below stays where it was.
    assert_eq!(text(&app), "alpha, bravo, charlie\ndelta");
    assert_eq!(app.prompt_kind, PromptKind::Command);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn space_p_j_defaults_to_an_empty_delimiter_and_drops_joined_indentation() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(
        &mut app,
        "    let first = 1;\n        let second = 2;\ntail",
    );
    set_cursor(&mut app, 0, 0);
    press(&mut app, 'x');
    press(&mut app, 'x');

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 'j');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    // The first line keeps its own indentation; only the whitespace against
    // the removed break goes.
    assert_eq!(text(&app), "    let first = 1;let second = 2;\ntail");
    press(&mut app, 'u');
    assert_eq!(
        text(&app),
        "    let first = 1;\n        let second = 2;\ntail",
        "the join is one undo step"
    );
}

#[test]
fn space_p_j_joins_a_line_selection_ending_on_an_empty_row() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\n\nbeta");
    set_cursor(&mut app, 0, 0);
    press(&mut app, 'x');
    press(&mut app, 'x');
    // The empty row contributes no characters, so the span stops at the
    // break before it. That break is still one the selection covers.
    assert_eq!(app.operative_spans(), vec![(0, 6)]);

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 'j');
    press(&mut app, '-');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(text(&app), "alpha-\nbeta");
}

#[test]
fn space_p_j_holds_back_the_terminator_of_a_half_open_selection() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo\ncharlie");
    // What a pointer drag from the first row to the start of the third
    // leaves behind: the span ends at a row none of which is selected.
    let buffer = app.active_buffer();
    let end = buffer.offset_of(Position::new(2, 0));
    let pane = app.panes.get_mut(&0).unwrap();
    pane.replace_selection(Selection::single(Range::new(0, end)));
    pane.mark_selection_semantics(SelectionSemantics::HalfOpen);

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 'j');
    press(&mut app, '-');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(text(&app), "alpha-bravo\ncharlie");
}

#[test]
fn space_p_j_leaves_a_bare_caret_on_an_empty_row_alone() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\n\nbeta");
    set_cursor(&mut app, 1, 0);
    // `operative_span` widens a caret on an empty row over that row's
    // terminator so `d` can delete the row; nothing there is selected to
    // join.
    assert_eq!(app.operative_spans(), vec![(6, 7)]);

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 'j');
    press(&mut app, '-');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(text(&app), "alpha\n\nbeta");
    assert_eq!(app.status, "selection holds no line break to join");
}

#[test]
fn space_p_j_leaves_a_selection_holding_no_line_break_alone() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo");
    set_cursor(&mut app, 0, 0);
    press(&mut app, 'x');

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 'j');
    press(&mut app, ' ');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(text(&app), "alpha\nbravo");
    assert_eq!(app.status, "selection holds no line break to join");
}

#[test]
fn space_p_j_joins_every_selection_as_one_transaction() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo\n\ncharlie\ndelta");
    let buffer = app.active_buffer();
    let first = Range::new(0, buffer.offset_of(Position::new(1, 4)));
    let second = Range::new(
        buffer.offset_of(Position::new(3, 0)),
        buffer.offset_of(Position::new(4, 4)),
    );
    app.panes.get_mut(&0).unwrap().selection = Selection::new(vec![first, second], 0);

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 'j');
    press(&mut app, '-');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(text(&app), "alpha-bravo\n\ncharlie-delta");
    press(&mut app, 'u');
    assert_eq!(text(&app), "alpha\nbravo\n\ncharlie\ndelta");
}

/// The table from the issue that asked for the command, formatted by
/// selecting its rows and pressing the keys.
#[test]
fn space_p_t_aligns_the_columns_of_the_selected_table() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(
        &mut app,
        "| Column 1 | Column 2 |\n\
             |---|---|\n\
             | Value | abc |\n\
             | Longer text | Very very long text |",
    );
    set_cursor(&mut app, 0, 0);
    for _ in 0..4 {
        press(&mut app, 'x');
    }

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    assert_eq!(
        text(&app),
        "| Column 1    | Column 2            |\n\
             |-------------|---------------------|\n\
             | Value       | abc                 |\n\
             | Longer text | Very very long text |"
    );
    assert_eq!(app.status, "aligned the table columns");
    press(&mut app, 'u');
    assert_eq!(
        text(&app),
        "| Column 1 | Column 2 |\n|---|---|\n| Value | abc |\n| Longer text | Very very long text |",
        "the formatting is one undo step"
    );
}

#[test]
fn space_p_t_keeps_a_separator_drawn_with_plus_signs() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(
        &mut app,
        "| Column 1 | Column 2 |\n+---+---+\n| Value | abc |",
    );
    set_cursor(&mut app, 0, 0);
    for _ in 0..3 {
        press(&mut app, 'x');
    }

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    assert_eq!(
        text(&app),
        "| Column 1 | Column 2 |\n+----------+----------+\n| Value    | abc      |"
    );
}

#[test]
fn space_p_t_widens_a_selection_that_starts_and_ends_mid_row() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "| a | bb |\n|-|-|\n| ccc | d |\ntail");
    let buffer = app.active_buffer();
    // What `v` from inside the first row to inside the third leaves behind.
    let selection = Selection::single(Range::new(
        buffer.offset_of(Position::new(0, 3)),
        buffer.offset_of(Position::new(2, 4)),
    ));
    app.panes.get_mut(&0).unwrap().replace_selection(selection);

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    // Both partial rows are formatted in full, and the row below the
    // selection is left where it was.
    assert_eq!(text(&app), "| a   | bb |\n|-----|----|\n| ccc | d  |\ntail");
}

#[test]
fn space_p_t_allows_blank_lines_inside_the_selection() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "| a | bb |\n|-|-|\n   \n");
    set_cursor(&mut app, 0, 0);
    for _ in 0..3 {
        press(&mut app, 'x');
    }

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    assert_eq!(text(&app), "| a | bb |\n|---|----|\n   \n");
}

#[test]
fn space_p_t_refuses_a_selection_that_holds_no_table() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "| a | bb |\n|-|-|\nprose\n");
    set_cursor(&mut app, 0, 0);
    for _ in 0..3 {
        press(&mut app, 'x');
    }

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    assert_eq!(text(&app), "| a | bb |\n|-|-|\nprose\n");
    assert_eq!(app.status, "no table detected in the selected lines");
    assert!(app.status_error);
}

#[test]
fn space_p_t_leaves_an_already_aligned_table_alone() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "| a   | bb |\n|-----|----|\n| ccc | d  |");
    set_cursor(&mut app, 0, 0);
    for _ in 0..3 {
        press(&mut app, 'x');
    }

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    assert_eq!(text(&app), "| a   | bb |\n|-----|----|\n| ccc | d  |");
    assert_eq!(app.status, "the table columns are already aligned");
}

#[test]
fn space_p_t_formats_every_selection_as_one_transaction() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "| a | bb |\n|-|-|\n\n| ccc | d |\n|-|-|");
    let buffer = app.active_buffer();
    let first = Range::new(0, buffer.offset_of(Position::new(1, 4)));
    let second = Range::new(
        buffer.offset_of(Position::new(3, 0)),
        buffer.offset_of(Position::new(4, 4)),
    );
    app.panes.get_mut(&0).unwrap().selection = Selection::new(vec![first, second], 0);

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    // Each selection is its own table, so the two do not share a width.
    assert_eq!(
        text(&app),
        "| a | bb |\n|---|----|\n\n| ccc | d |\n|-----|---|"
    );
    press(&mut app, 'u');
    assert_eq!(text(&app), "| a | bb |\n|-|-|\n\n| ccc | d |\n|-|-|");
}

#[test]
fn space_p_t_folds_selections_that_widen_onto_the_same_rows() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "| a | bb |\n|-|-|\n| cccc | d |");
    let buffer = app.active_buffer();
    // Two ranges that do not overlap until they are widened: the first ends
    // on the separator row, the second starts further along it.
    let first = Range::new(0, buffer.offset_of(Position::new(1, 1)));
    let second = Range::new(
        buffer.offset_of(Position::new(1, 3)),
        buffer.offset_of(Position::new(2, 4)),
    );
    app.panes.get_mut(&0).unwrap().selection = Selection::new(vec![first, second], 0);

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    // One table across all three rows. Formatted as two, the overlapping
    // change would have been dropped and a row left unformatted under a
    // success message.
    assert_eq!(text(&app), "| a    | bb |\n|------|----|\n| cccc | d  |");
    assert_eq!(app.status, "aligned the table columns");
}

#[test]
fn space_p_t_folds_selections_lying_on_consecutive_rows() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "| a | bb |\n|-|-|\n| cccc | d |");
    let buffer = app.active_buffer();
    // A row each, with nothing between them but the line terminators.
    let ranges = (0..3)
        .map(|row| {
            Range::new(
                buffer.offset_of(Position::new(row, 0)),
                buffer.offset_of(Position::new(row, 1)),
            )
        })
        .collect();
    app.panes.get_mut(&0).unwrap().selection = Selection::new(ranges, 0);

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    // One table sharing one set of widths, not three single-row ones — which
    // would each have been refused for holding no separator.
    assert_eq!(text(&app), "| a    | bb |\n|------|----|\n| cccc | d  |");
}

#[test]
fn space_p_t_refuses_pipe_rows_with_no_separator_among_them() {
    let mut app = App::new(Config::default(), None).unwrap();
    // What a Rust closure looks like to a formatter that only counts pipes.
    seed(&mut app, "|value| value + 1\n|item| item * 2");
    set_cursor(&mut app, 0, 0);
    press(&mut app, 'x');
    press(&mut app, 'x');

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    assert_eq!(text(&app), "|value| value + 1\n|item| item * 2");
    assert_eq!(app.status, "no table detected in the selected lines");
    assert!(app.status_error);
}

#[test]
fn space_p_t_expands_a_tab_in_a_cell_to_the_configured_tab_width() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.config.editor.tab_width = 2;
    seed(&mut app, "| ab\tc | b |\n|-|-|\n| dddd | c |");
    set_cursor(&mut app, 0, 0);
    for _ in 0..3 {
        press(&mut app, 'x');
    }

    press(&mut app, ' ');
    press(&mut app, 'p');
    press(&mut app, 't');

    // `ab` reaches column 2, so the tab runs to the stop at 4 and the cell is
    // five wide. Left as a tab it would have measured three and drawn wrong.
    assert_eq!(text(&app), "| ab  c | b |\n|-------|---|\n| dddd  | c |");
}

#[test]
fn x_and_x_yanks_paste_whole_lines_but_v_yanks_remain_characterwise() {
    let mut single = App::new(Config::default(), None).unwrap();
    seed(&mut single, "alpha\nbravo\ncharlie");
    press(&mut single, 'x');
    press(&mut single, 'y');
    assert_eq!(single.registers[&'"'].text, "alpha\n");
    assert!(single.registers[&'"'].linewise);
    set_cursor(&mut single, 1, 0);
    press(&mut single, 'p');
    assert_eq!(text(&single), "alpha\nbravo\nalpha\ncharlie");
    assert_eq!(
        single.mode,
        Mode::Normal,
        "buffer paste stays in Normal mode"
    );

    let mut multiple = App::new(Config::default(), None).unwrap();
    seed(&mut multiple, "alpha\nbravo\ncharlie");
    set_cursor(&mut multiple, 2, 0);
    press(&mut multiple, 'X');
    press(&mut multiple, 'X');
    press(&mut multiple, 'y');
    assert_eq!(multiple.registers[&'"'].text, "bravo\ncharlie\n");
    assert!(multiple.registers[&'"'].linewise);

    let mut visual = App::new(Config::default(), None).unwrap();
    seed(&mut visual, "alpha\nbravo");
    press(&mut visual, 'v');
    press(&mut visual, 'g');
    press(&mut visual, 'l');
    press(&mut visual, 'y');
    assert_eq!(visual.registers[&'"'].text, "alpha");
    assert!(!visual.registers[&'"'].linewise);
    set_cursor(&mut visual, 1, 0);
    press(&mut visual, 'p');
    assert_eq!(text(&visual), "alpha\nbalpharavo");
}

#[test]
fn p_replaces_a_selection_while_capital_p_still_pastes_beside_it() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha bravo");
    press(&mut app, 'v');
    for _ in 0..4 {
        press(&mut app, 'l');
    }
    press(&mut app, 'y');
    assert_eq!(app.registers[&'"'].text, "alpha");

    set_cursor(&mut app, 0, 6);
    press(&mut app, 'v');
    for _ in 0..4 {
        press(&mut app, 'l');
    }
    press(&mut app, 'p');
    assert_eq!(text(&app), "alpha alpha");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.active().selection.primary(),
        Range::new(6, 10),
        "the replacement is left selected, so it can be replaced again"
    );
    assert_eq!(
        app.registers[&'"'].text, "alpha",
        "replacing does not consume the register"
    );

    // `P` is the way text still reaches a selection without consuming it.
    press(&mut app, 'P');
    assert_eq!(text(&app), "alpha alphaalpha");
}

#[test]
fn a_linewise_paste_over_a_line_selection_replaces_whole_lines() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo\ncharlie");
    press(&mut app, 'x');
    press(&mut app, 'y');
    assert!(app.registers[&'"'].linewise);

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'x');
    press(&mut app, 'p');
    assert_eq!(text(&app), "alpha\nalpha\ncharlie");
    assert_eq!(
        app.active().selection.primary(),
        Range::new(6, 10),
        "the pasted line is selected without its terminator"
    );

    set_cursor(&mut app, 2, 0);
    press(&mut app, 'x');
    press(&mut app, 'p');
    assert_eq!(
        text(&app),
        "alpha\nalpha\nalpha",
        "the final line gains no terminator it never had"
    );
}

#[test]
fn a_paste_replaces_every_range_that_holds_text_and_inserts_beside_the_rest() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "one two one");
    app.registers.insert(
        '"',
        Register {
            text: "X".to_owned(),
            linewise: false,
            directory: None,
        },
    );
    app.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::new(0, 2), Range::new(8, 10)], 0);
    press(&mut app, 'p');
    assert_eq!(text(&app), "X two X");
    assert_eq!(
        app.active().selection.ranges(),
        [Range::point(0), Range::point(6)]
    );

    let mut mixed = App::new(Config::default(), None).unwrap();
    seed(&mut mixed, "one two");
    mixed.registers.insert(
        '"',
        Register {
            text: "X".to_owned(),
            linewise: false,
            directory: None,
        },
    );
    mixed.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::new(0, 2), Range::point(4)], 0);
    press(&mut mixed, 'p');
    assert_eq!(
        text(&mixed),
        "X tXwo",
        "the range gives up its text; the bare caret keeps its own"
    );
}

#[test]
fn y_yanks_the_caret_character_and_capital_y_yanks_whole_lines() {
    let mut caret = App::new(Config::default(), None).unwrap();
    seed(&mut caret, "alpha\nbravo");
    set_cursor(&mut caret, 0, 1);
    press(&mut caret, 'y');
    assert_eq!(caret.registers[&'"'].text, "l");
    assert!(!caret.registers[&'"'].linewise);
    assert_eq!(caret.mode, Mode::Normal);
    press(&mut caret, 'p');
    assert_eq!(text(&caret), "allpha\nbravo");

    // `v` with no motion selects nothing but the caret, so it yanks the
    // same character and hands the mode back like any other selection.
    let mut select = App::new(Config::default(), None).unwrap();
    seed(&mut select, "alpha\nbravo");
    set_cursor(&mut select, 0, 1);
    press(&mut select, 'v');
    press(&mut select, 'y');
    assert_eq!(select.registers[&'"'].text, "l");
    assert!(!select.registers[&'"'].linewise);
    assert_eq!(select.mode, Mode::Normal);

    let mut line = App::new(Config::default(), None).unwrap();
    seed(&mut line, "alpha\nbravo\ncharlie");
    set_cursor(&mut line, 0, 2);
    press(&mut line, 'Y');
    assert_eq!(line.registers[&'"'].text, "alpha\n");
    assert!(line.registers[&'"'].linewise);
    // Unlike `x`, `Y` leaves the caret and the selection alone.
    assert_eq!(cursor(&line), Position::new(0, 2));
    assert_eq!(line.mode, Mode::Normal);
    set_cursor(&mut line, 1, 0);
    press(&mut line, 'p');
    assert_eq!(text(&line), "alpha\nbravo\nalpha\ncharlie");

    // The register `Y` writes is the one `x y` writes, over every row the
    // selection touches rather than only the caret's.
    let mut spanning = App::new(Config::default(), None).unwrap();
    seed(&mut spanning, "alpha\nbravo\ncharlie");
    press(&mut spanning, 'v');
    press(&mut spanning, 'j');
    press(&mut spanning, 'Y');
    assert_eq!(spanning.registers[&'"'].text, "alpha\nbravo\n");
    assert!(spanning.registers[&'"'].linewise);
    assert_eq!(spanning.mode, Mode::Normal);

    let mut extended = App::new(Config::default(), None).unwrap();
    seed(&mut extended, "alpha\nbravo\ncharlie");
    press(&mut extended, 'x');
    press(&mut extended, 'x');
    press(&mut extended, 'y');
    assert_eq!(extended.registers[&'"'].text, "alpha\nbravo\n");

    // The rows come from the raw range rather than from `operative_span`,
    // which on an empty last line resolves backwards onto the previous
    // line's terminator and would yank the row above this one.
    let mut trailing = App::new(Config::default(), None).unwrap();
    seed(&mut trailing, "alpha\nbravo\n");
    set_cursor(&mut trailing, 2, 0);
    press(&mut trailing, 'Y');
    assert_eq!(trailing.registers[&'"'].text, "\n");
    assert!(trailing.registers[&'"'].linewise);
}

#[test]
fn line_commands_hold_back_a_half_open_selection_end_row() {
    let mut yank = App::new(Config::default(), None).unwrap();
    seed(&mut yank, "alpha  \nbravo  \ncharlie");
    let next_row = yank.active_buffer().line_to_offset(1);
    yank.active_mut().selection = Selection::single(Range::new(0, next_row));
    yank.active_mut()
        .mark_selection_semantics(SelectionSemantics::HalfOpen);

    press(&mut yank, 'Y');

    assert_eq!(yank.registers[&'"'].text, "alpha  \n");

    let mut trim = App::new(Config::default(), None).unwrap();
    seed(&mut trim, "alpha  \nbravo  \ncharlie");
    let next_row = trim.active_buffer().line_to_offset(1);
    trim.active_mut().selection = Selection::single(Range::new(0, next_row));
    trim.active_mut()
        .mark_selection_semantics(SelectionSemantics::HalfOpen);

    press(&mut trim, '_');

    assert_eq!(text(&trim), "alpha\nbravo  \ncharlie");
}

#[test]
fn replace_preserves_crlf_terminators_inside_a_selection() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "ab\r\ncd");
    app.mode = Mode::Select;
    app.active_mut().selection = Selection::single(Range::new(0, 5));

    press(&mut app, 'r');
    press(&mut app, 'z');

    assert_eq!(text(&app), "zz\r\nzz");
}

#[test]
fn linewise_delete_and_paste_keep_crlf_terminators_atomic() {
    let mut delete = App::new(Config::default(), None).unwrap();
    seed(&mut delete, "alpha\r\nbravo");
    set_cursor(&mut delete, 1, 0);

    press(&mut delete, 'x');
    press(&mut delete, 'd');

    assert_eq!(text(&delete), "alpha");
    assert_eq!(delete.registers[&'"'].text, "bravo\r\n");
    press(&mut delete, 'p');
    assert_eq!(text(&delete), "alpha\r\nbravo\r\n");

    let mut paste = App::new(Config::default(), None).unwrap();
    seed(&mut paste, "alpha\r\nbravo");
    paste.registers.insert(
        '"',
        Register {
            text: "copied\n".to_owned(),
            linewise: true,
            directory: None,
        },
    );

    press(&mut paste, 'p');

    assert_eq!(text(&paste), "alpha\r\ncopied\nbravo");
}

#[test]
fn open_line_uses_the_surrounding_crlf_style_as_one_undo_group() {
    for (binding, expected) in [('o', "alpha\r\n\r\nbravo"), ('O', "\r\nalpha\r\nbravo")] {
        let mut app = App::new(Config::default(), None).unwrap();
        seed(&mut app, "alpha\r\nbravo");

        press(&mut app, binding);

        assert_eq!(text(&app), expected, "{binding}");
        key(&mut app, KeyCode::Escape, Modifiers::NONE);
        press(&mut app, 'u');
        assert_eq!(text(&app), "alpha\r\nbravo", "undo {binding}");
    }
}

#[test]
fn tab_stops_and_selection_alignment_use_display_columns() {
    let mut tab = App::new(Config::default(), None).unwrap();
    tab.config.editor.tab_width = 4;
    seed(&mut tab, "界");
    tab.mode = Mode::Insert;
    tab.active_mut().selection = Selection::point(1);

    key(&mut tab, KeyCode::Tab, Modifiers::NONE);

    assert_eq!(text(&tab), "界  ");

    let mut tab_after_tab = App::new(Config::default(), None).unwrap();
    tab_after_tab.config.editor.tab_width = 4;
    seed(&mut tab_after_tab, "\tx");
    tab_after_tab.mode = Mode::Insert;
    tab_after_tab.active_mut().selection = Selection::point(1);

    key(&mut tab_after_tab, KeyCode::Tab, Modifiers::NONE);

    assert_eq!(text(&tab_after_tab), "\t    x");

    let mut align = App::new(Config::default(), None).unwrap();
    align.config.editor.tab_width = 4;
    seed(&mut align, "界x\nabc");
    let second = align.active_buffer().line_to_offset(1) + 3;
    align.active_mut().selection = Selection::new(vec![Range::point(1), Range::point(second)], 0);

    press(&mut align, '&');

    assert_eq!(text(&align), "界 x\nabc");

    let mut align_after_tab = App::new(Config::default(), None).unwrap();
    align_after_tab.config.editor.tab_width = 4;
    seed(&mut align_after_tab, "\tx\nabcde");
    let second = align_after_tab.active_buffer().line_to_offset(1) + 5;
    align_after_tab.active_mut().selection =
        Selection::new(vec![Range::point(1), Range::point(second)], 0);

    press(&mut align_after_tab, '&');

    assert_eq!(text(&align_after_tab), "\t x\nabcde");
}

#[test]
fn x_delete_pastes_whole_lines_but_v_delete_remains_characterwise() {
    let mut line = App::new(Config::default(), None).unwrap();
    seed(&mut line, "alpha\nbravo\ncharlie");
    press(&mut line, 'x');
    press(&mut line, 'd');
    assert_eq!(line.registers[&'"'].text, "alpha\n");
    assert!(line.registers[&'"'].linewise);
    assert_eq!(text(&line), "bravo\ncharlie");
    press(&mut line, 'p');
    assert_eq!(text(&line), "bravo\nalpha\ncharlie");

    let mut visual = App::new(Config::default(), None).unwrap();
    seed(&mut visual, "alpha\nbravo");
    press(&mut visual, 'v');
    press(&mut visual, 'l');
    press(&mut visual, 'd');
    assert_eq!(visual.registers[&'"'].text, "al");
    assert!(!visual.registers[&'"'].linewise);
    set_cursor(&mut visual, 0, 1);
    press(&mut visual, 'p');
    assert_eq!(text(&visual), "phala\nbravo");

    let mut final_line = App::new(Config::default(), None).unwrap();
    seed(&mut final_line, "alpha\nbravo");
    set_cursor(&mut final_line, 1, 0);
    press(&mut final_line, 'x');
    press(&mut final_line, 'd');
    assert_eq!(text(&final_line), "alpha");
    press(&mut final_line, 'p');
    assert_eq!(text(&final_line), "alpha\nbravo\n");

    let mut multiple = App::new(Config::default(), None).unwrap();
    seed(&mut multiple, "alpha\nbravo\ncharlie\ndelta");
    set_cursor(&mut multiple, 2, 0);
    press(&mut multiple, 'X');
    press(&mut multiple, 'X');
    press(&mut multiple, 'd');
    assert_eq!(multiple.registers[&'"'].text, "bravo\ncharlie\n");
    assert!(multiple.registers[&'"'].linewise);
    assert_eq!(text(&multiple), "alpha\ndelta");
    set_cursor(&mut multiple, 0, 0);
    press(&mut multiple, 'p');
    assert_eq!(text(&multiple), "alpha\nbravo\ncharlie\ndelta");

    let mut change = App::new(Config::default(), None).unwrap();
    seed(&mut change, "alpha\nbravo");
    set_cursor(&mut change, 1, 0);
    press(&mut change, 'x');
    press(&mut change, 'c');
    press(&mut change, 'X');
    assert_eq!(text(&change), "alpha\nX");
}

#[test]
fn counted_line_selection_takes_that_many_lines() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "alpha\nbravo\ncharlie\ndelta");
    set_cursor(&mut app, 3, 0);

    press(&mut app, '3');
    press(&mut app, 'X');
    assert_eq!(selected_rows(&app), (1, 3));
}

#[test]
fn keyed_and_direct_counted_line_selection_are_equivalent() {
    let mut keyed = App::new(Config::default(), None).unwrap();
    let mut direct = App::new(Config::default(), None).unwrap();
    for app in [&mut keyed, &mut direct] {
        seed(app, "alpha\nbravo\ncharlie\ndelta");
        set_cursor(app, 3, 0);
    }

    press(&mut keyed, '3');
    press(&mut keyed, 'X');
    direct
        .execute(
            CommandInvocation::editor(
                EditorCommand::SelectLineUp,
                CommandExecutionContext::resolved(std::num::NonZeroUsize::new(3).unwrap(), None),
            )
            .unwrap(),
        )
        .unwrap();

    assert_eq!(keyed.mode, direct.mode);
    assert_eq!(keyed.active().selection, direct.active().selection);
    assert_eq!(keyed.line_select, direct.line_select);
}

#[test]
fn replace_case_and_indent_commands_are_registry_driven() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.config.editor.tab_width = 2;
    seed(&mut app, "aß\nb");

    press(&mut app, '~');
    assert_eq!(app.active_buffer().line_string(0), "Aß");
    press(&mut app, 'r');
    press(&mut app, 'z');
    assert_eq!(app.active_buffer().line_string(0), "zß");

    press(&mut app, '%');
    press(&mut app, '>');
    assert_eq!(text(&app), "  zß\n  b");
    press(&mut app, '<');
    assert_eq!(text(&app), "zß\nb");
}

/// An app whose active buffer looks like a saved file, so that the syntax
/// registry can resolve a language from its extension.
fn app_with_file(name: &str, contents: &str) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers[0].path = Some(temporary(name));
    app.buffers[0].kind = crate::buffer::BufferKind::File;
    seed(&mut app, contents);
    app
}

fn ctrl(app: &mut App, character: char) {
    key(app, KeyCode::Char(character), Modifiers::CONTROL);
}

/// Selects whole rows `from..=to`, the way `x` repeated would, without
/// leaving a transient line selection behind.
fn select_rows(app: &mut App, from: usize, to: usize) {
    let buffer = app.active_buffer();
    let start = buffer.line_to_offset(from);
    let end = buffer.line_to_offset(to) + buffer.line_len(to);
    app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(start, end));
}

#[test]
fn comment_toggle_uses_the_language_marker_and_round_trips() {
    let mut app = app_with_file("toggle.rs", "let a = 1;\nlet b = 2;\n");
    select_rows(&mut app, 0, 1);
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "// let a = 1;\n// let b = 2;\n");
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "let a = 1;\nlet b = 2;\n");

    let mut python = app_with_file("toggle.py", "a = 1\nb = 2\n");
    select_rows(&mut python, 0, 1);
    ctrl(&mut python, 'c');
    assert_eq!(text(&python), "# a = 1\n# b = 2\n");
    ctrl(&mut python, 'c');
    assert_eq!(text(&python), "a = 1\nb = 2\n");
}

#[test]
fn comment_toggle_preserves_a_shebang_that_is_the_only_language_signal() {
    let mut app = app_with_file("toggle-script", "#!/bin/sh\necho one\necho two\n");
    select_rows(&mut app, 0, 2);
    ctrl(&mut app, 'c');
    assert_eq!(
        text(&app),
        "#!/bin/sh\n# echo one\n# echo two\n",
        "the shebang must keep the extensionless buffer identifiable"
    );
    assert_eq!(
        buffer_language(app.active_buffer(), &app.registry)
            .map(|language| app.registry.language_name(language)),
        Some("bash")
    );
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "#!/bin/sh\necho one\necho two\n");

    select_rows(&mut app, 0, 0);
    ctrl(&mut app, 'c');
    assert_eq!(
        text(&app),
        "#!/bin/sh\necho one\necho two\n",
        "selecting only the language-bearing shebang is a no-op"
    );
    assert_eq!(
        buffer_language(app.active_buffer(), &app.registry)
            .map(|language| app.registry.language_name(language)),
        Some("bash")
    );
}

#[test]
fn comment_toggle_commits_a_partly_commented_block_before_uncommenting() {
    let mut app = app_with_file("partial.rs", "// one\ntwo\n");
    select_rows(&mut app, 0, 1);
    ctrl(&mut app, 'c');
    assert_eq!(
        text(&app),
        "// // one\n// two\n",
        "a mixed block commits to commented first, so the next press is its inverse"
    );
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "// one\ntwo\n");
}

#[test]
fn comment_toggle_places_the_marker_at_the_shared_minimum_indent() {
    let mut app = app_with_file("nested.rs", "    if x {\n        y();\n    }\n");
    select_rows(&mut app, 0, 2);
    ctrl(&mut app, 'c');
    assert_eq!(
        text(&app),
        "    // if x {\n    //     y();\n    // }\n",
        "relative indentation inside the block survives"
    );
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "    if x {\n        y();\n    }\n");
}

#[test]
fn comment_toggle_leaves_blank_lines_alone_in_both_directions() {
    let mut app = app_with_file("blank.rs", "one\n\n  \ntwo\n");
    select_rows(&mut app, 0, 3);
    ctrl(&mut app, 'c');
    assert_eq!(
        text(&app),
        "// one\n\n  \n// two\n",
        "an empty row says nothing about whether the block is commented"
    );
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "one\n\n  \ntwo\n");
}

#[test]
fn comment_toggle_removes_the_marker_with_or_without_a_following_space() {
    let mut app = app_with_file("spacing.rs", "//tight\n// loose\n//  padded\n");
    select_rows(&mut app, 0, 2);
    ctrl(&mut app, 'c');
    assert_eq!(
        text(&app),
        "tight\nloose\n padded\n",
        "exactly one space after the marker is consumed"
    );
}

#[test]
fn comment_toggle_in_insert_mode_acts_on_each_caret_row() {
    let mut app = app_with_file("insert.rs", "alpha\nbravo\n");
    set_cursor(&mut app, 1, 3);
    press(&mut app, 'i');
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "alpha\n// bravo\n");
    assert_eq!(app.mode, Mode::Insert, "the toggle does not leave Insert");
    assert_eq!(
        cursor(&app),
        Position::new(1, 6),
        "the caret rides the inserted marker and stays on the same character"
    );

    // Entering Insert collapses every range to a point, so multiple carets
    // are the only way a single press reaches more than one row.
    let mut multi = app_with_file("carets.rs", "alpha\nbravo\n");
    multi.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::point(0), Range::point(6)], 0);
    press(&mut multi, 'i');
    ctrl(&mut multi, 'c');
    assert_eq!(text(&multi), "// alpha\n// bravo\n");
}

#[test]
fn comment_toggle_reports_languages_that_have_no_line_comment() {
    let mut app = app_with_file("styles.css", "a { color: red; }\n");
    press(&mut app, '%');
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "a { color: red; }\n", "the file is left alone");
    assert_eq!(app.status, "css has no line comment");

    let mut plain = App::new(Config::default(), None).unwrap();
    seed(&mut plain, "one\ntwo\n");
    press(&mut plain, '%');
    ctrl(&mut plain, 'c');
    assert_eq!(text(&plain), "one\ntwo\n");
    assert_eq!(plain.status, "no language for this buffer");
}

#[test]
fn comment_toggle_is_refused_in_a_read_only_buffer() {
    // The refusal comes from the shared `is_mutating` gate rather than
    // from the toggle itself, which is exactly why the command has to
    // declare itself mutating.
    let mut app = App::new(Config::default(), None).unwrap();
    // Seeded before the kind changes: a read-only buffer refuses the
    // seeding transaction too.
    seed(&mut app, "let a = 1;\n");
    app.buffers[0].kind = crate::buffer::BufferKind::Help;
    press(&mut app, '%');
    ctrl(&mut app, 'c');
    assert_eq!(text(&app), "let a = 1;\n");
    assert_eq!(app.status, "help is read-only");
    assert!(app.status_error);
}
