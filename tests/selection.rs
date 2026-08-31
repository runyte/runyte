// SPDX-License-Identifier: MPL-2.0

//! Phase 0 gate: multi-range selections and multi-cursor editing.
//!
//! These drive the real key dispatch path rather than calling editing helpers
//! directly, so a binding that is registered but unreachable fails here.

use runyte::{
    app::App,
    buffer::Position,
    command::Mode,
    config::Config,
    input::{KeyCode, KeyStroke, Modifiers},
    selection::{Range, Selection},
    text::Transaction,
};

fn app_with(text: &str) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers[0].apply(&Transaction::insert(0, text));
    app.panes.get_mut(&0).unwrap().selection = Selection::point(0);
    app
}

fn press(app: &mut App, character: char) {
    app.handle_key(KeyStroke::new(KeyCode::Char(character), Modifiers::NONE))
        .unwrap();
}

fn alt(app: &mut App, character: char) {
    app.handle_key(KeyStroke::new(KeyCode::Char(character), Modifiers::ALT))
        .unwrap();
}

fn text(app: &App) -> String {
    app.active_buffer().to_string()
}

fn selection(app: &App) -> &Selection {
    &app.active().selection
}

fn positions(app: &App) -> Vec<Position> {
    selection(app)
        .ranges()
        .iter()
        .map(|range| app.active_buffer().position_of(range.head))
        .collect()
}

fn set_cursor(app: &mut App, row: usize, col: usize) {
    let offset = app.active_buffer().offset_of(Position::new(row, col));
    app.panes.get_mut(&0).unwrap().selection = Selection::point(offset);
}

// -- Normalization --------------------------------------------------------

#[test]
fn overlapping_ranges_merge_into_one() {
    let selection = Selection::new(vec![Range::new(0, 10), Range::new(5, 15)], 0);
    assert_eq!(selection.len(), 1);
    assert_eq!(selection.ranges()[0], Range::new(0, 15));
}

#[test]
fn nested_ranges_collapse_to_the_outer_span() {
    let selection = Selection::new(vec![Range::new(0, 20), Range::new(8, 12)], 1);
    assert_eq!(selection.len(), 1);
    assert_eq!(selection.primary(), Range::new(0, 20));
}

#[test]
fn a_selection_can_never_become_empty() {
    let selection = Selection::point(4);
    assert_eq!(selection.remove_primary().len(), 1);
    assert_eq!(Selection::new(Vec::new(), 3).len(), 1);
}

// -- Multi-cursor editing -------------------------------------------------

#[test]
fn typing_applies_at_every_cursor_and_undoes_once() {
    let mut app = app_with("one\ntwo\nthree");
    press(&mut app, 'C');
    press(&mut app, 'C');
    assert_eq!(selection(&app).len(), 3, "three carets, one per line");

    press(&mut app, 'i');
    press(&mut app, '>');
    assert_eq!(text(&app), ">one\n>two\n>three");

    app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();
    press(&mut app, 'u');
    assert_eq!(
        text(&app),
        "one\ntwo\nthree",
        "one undo reverts all three cursors"
    );
}

#[test]
fn an_insert_session_is_one_undo_step_and_restores_its_starting_cursor() {
    let mut app = app_with("abcdef");
    set_cursor(&mut app, 0, 3);

    press(&mut app, 'i');
    press(&mut app, 'X');
    press(&mut app, 'Y');
    press(&mut app, 'Z');
    app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();

    assert_eq!(text(&app), "abcXYZdef");
    press(&mut app, 'u');
    assert_eq!(text(&app), "abcdef", "one undo removes the full insert");
    assert_eq!(app.cursor_position(), Position::new(0, 3));
}

#[test]
fn open_line_and_change_include_their_entry_edit_in_the_insert_checkpoint() {
    let mut opened = app_with("abc");
    press(&mut opened, 'o');
    press(&mut opened, 'x');
    opened
        .handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();
    press(&mut opened, 'u');
    assert_eq!(text(&opened), "abc");

    let mut changed = app_with("abc");
    press(&mut changed, 'v');
    press(&mut changed, 'l');
    press(&mut changed, 'c');
    press(&mut changed, 'X');
    changed
        .handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();
    press(&mut changed, 'u');
    assert_eq!(text(&changed), "abc");
}

#[test]
fn deletion_at_every_cursor_is_one_step() {
    let mut app = app_with("axb\naxb\naxb");
    press(&mut app, 'C');
    press(&mut app, 'C');
    press(&mut app, 'l');
    press(&mut app, 'd');
    assert_eq!(text(&app), "ab\nab\nab");
    press(&mut app, 'u');
    assert_eq!(text(&app), "axb\naxb\naxb");
}

#[test]
fn cursors_stay_aligned_when_edits_change_length_unevenly() {
    let mut app = app_with("a\nbbbb\ncc");
    press(&mut app, 'C');
    press(&mut app, 'C');
    press(&mut app, 'i');
    for character in "XY".chars() {
        press(&mut app, character);
    }
    assert_eq!(text(&app), "XYa\nXYbbbb\nXYcc");
    // Every caret must sit immediately after its own insertion.
    for range in selection(&app).ranges() {
        let position = app.active_buffer().position_of(range.head);
        assert_eq!(position.col, 2, "caret drifted on row {}", position.row);
    }
}

#[test]
fn a_cursor_is_not_added_past_the_last_row() {
    let mut app = app_with("only");
    press(&mut app, 'C');
    assert_eq!(selection(&app).len(), 1);
    assert!(app.status_error);
}

#[test]
fn a_cursor_is_skipped_on_rows_too_short_for_the_column() {
    let mut app = app_with("longer line\nab\nlonger line");
    set_cursor(&mut app, 0, 8);
    press(&mut app, 'C');
    // Row 1 has no column 8, so the caret lands on row 2 instead.
    assert_eq!(selection(&app).len(), 2);
    let rows: Vec<usize> = selection(&app)
        .ranges()
        .iter()
        .map(|range| app.active_buffer().position_of(range.head).row)
        .collect();
    assert_eq!(rows, vec![0, 2]);
}

/// A row that ends exactly at the column has no character there, so it is
/// skipped rather than catching the caret on its last character.
#[test]
fn a_cursor_is_skipped_on_a_row_that_ends_at_the_column() {
    let mut app = app_with("Some list:\n- Three\n- One abc");
    set_cursor(&mut app, 0, 7);

    press(&mut app, 'C');

    // Row 1 is seven characters long, so column 7 is its terminator.
    assert_eq!(
        positions(&app),
        vec![Position::new(0, 7), Position::new(2, 7)]
    );
}

/// Rows that cannot hold the column must not bend the ones after them: each
/// caret is copied from the column it was created with, so a skipped row leaves
/// every later caret in the same column as the first.
#[test]
fn repeated_copies_hold_the_column_across_rows_that_cannot_take_it() {
    let mut app = app_with("Some list:\n- One abc\n- Two\n- Three\nAnother:\n2. Orange");
    set_cursor(&mut app, 0, 7);

    press(&mut app, 'C');
    press(&mut app, 'C');
    press(&mut app, 'C');

    // Rows 2 and 3 are too short for column 7 and drop out; the longer rows
    // below them still take the caret at column 7 rather than at column 6.
    assert_eq!(
        positions(&app),
        vec![
            Position::new(0, 7),
            Position::new(1, 7),
            Position::new(4, 7),
            Position::new(5, 7)
        ]
    );
}

/// An empty row holds no column at all, including column 0.
#[test]
fn a_cursor_is_skipped_on_an_empty_row() {
    let mut app = app_with("one\n\ntwo");
    press(&mut app, 'C');

    assert_eq!(
        positions(&app),
        vec![Position::new(0, 0), Position::new(2, 0)]
    );
}

#[test]
fn padded_multicursor_adds_carets_on_short_and_empty_rows() {
    let mut app =
        app_with("This is a longer line\nThis is shorter\n\nOne line above was even empty");
    set_cursor(&mut app, 0, 20);

    press(&mut app, 'V');
    press(&mut app, 'V');
    press(&mut app, 'V');

    assert_eq!(
        text(&app),
        "This is a longer line\nThis is shorter      \n                     \nOne line above was even empty"
    );
    let positions: Vec<Position> = selection(&app)
        .ranges()
        .iter()
        .map(|range| app.active_buffer().position_of(range.head))
        .collect();
    assert_eq!(
        positions,
        vec![
            Position::new(0, 20),
            Position::new(1, 20),
            Position::new(2, 20),
            Position::new(3, 20)
        ]
    );
}

/// `Alt-V` mirrors `V` upward, the way `Alt-C` mirrors `C`.
#[test]
fn padded_multicursor_walks_upward_through_short_and_empty_rows() {
    let mut app = app_with("One line below is empty\n\nThis is shorter\nThis is a longer line");
    set_cursor(&mut app, 3, 20);

    alt(&mut app, 'V');
    alt(&mut app, 'V');
    alt(&mut app, 'V');

    assert_eq!(
        text(&app),
        "One line below is empty\n                     \nThis is shorter      \nThis is a longer line"
    );
    let mut rows: Vec<Position> = selection(&app)
        .ranges()
        .iter()
        .map(|range| app.active_buffer().position_of(range.head))
        .collect();
    rows.sort_by_key(|position| position.row);
    assert_eq!(
        rows,
        vec![
            Position::new(0, 20),
            Position::new(1, 20),
            Position::new(2, 20),
            Position::new(3, 20)
        ]
    );
}

/// The first row has nothing above it, so the command refuses rather than
/// padding a row that does not exist.
#[test]
fn padded_multicursor_upward_stops_at_the_first_row() {
    let mut app = app_with("first\nsecond\n");
    set_cursor(&mut app, 0, 3);

    alt(&mut app, 'V');

    assert_eq!(text(&app), "first\nsecond\n");
    assert_eq!(selection(&app).len(), 1);
    assert_eq!(app.status, "no room for another cursor");
}

#[test]
fn padded_multicursor_uses_display_columns_for_tabs_and_wide_characters() {
    let mut app = app_with("\t界x\n\t\n");
    set_cursor(&mut app, 0, 2);

    press(&mut app, 'V');

    assert_eq!(text(&app), "\t界x\n\t   \n");
    let position = app
        .active_buffer()
        .position_of(selection(&app).ranges()[1].head);
    assert_eq!(position, Position::new(1, 3));
}

// -- Selection commands ---------------------------------------------------

#[test]
fn keep_and_remove_primary_reduce_the_selection() {
    let mut app = app_with("a\nb\nc");
    press(&mut app, 'C');
    press(&mut app, 'C');
    assert_eq!(selection(&app).len(), 3);

    alt(&mut app, ',');
    assert_eq!(selection(&app).len(), 2);

    press(&mut app, ',');
    assert_eq!(selection(&app).len(), 1);
}

#[test]
fn rotation_moves_the_primary_without_changing_the_ranges() {
    let mut app = app_with("a\nb\nc");
    press(&mut app, 'C');
    press(&mut app, 'C');
    let before = selection(&app).ranges().to_vec();
    let primary = selection(&app).primary_index();

    press(&mut app, ')');
    assert_eq!(selection(&app).ranges(), before.as_slice());
    assert_ne!(selection(&app).primary_index(), primary);

    press(&mut app, '(');
    assert_eq!(selection(&app).primary_index(), primary);
}

fn keys(app: &mut App, sequence: &str) {
    for character in sequence.chars() {
        press(app, character);
    }
}

fn submit(app: &mut App, pattern: &str) {
    for character in pattern.chars() {
        press(app, character);
    }
    app.handle_key(KeyStroke::new(KeyCode::Enter, Modifiers::NONE))
        .unwrap();
}

#[test]
fn splitting_selected_lines_leaves_one_cursor_per_line_edge() {
    let mut app = app_with("aa\nbb\ncc");
    press(&mut app, '%');
    press(&mut app, ' ');
    press(&mut app, 's');
    press(&mut app, 'e');
    assert_eq!(selection(&app).len(), 3);
    for (row, range) in selection(&app).ranges().iter().enumerate() {
        assert!(range.is_empty(), "line ends carry a bare cursor");
        let position = app.active_buffer().position_of(range.from());
        assert_eq!((position.row, position.col), (row, 2));
    }

    // Typing once now types on every line, which is what the cursors are for.
    press(&mut app, 'a');
    press(&mut app, '!');
    assert_eq!(text(&app), "aa!\nbb!\ncc!");

    let mut app = app_with("aa\nbb\ncc");
    press(&mut app, '%');
    press(&mut app, ' ');
    press(&mut app, 's');
    press(&mut app, 'b');
    assert_eq!(selection(&app).len(), 3);
    for (row, range) in selection(&app).ranges().iter().enumerate() {
        let position = app.active_buffer().position_of(range.from());
        assert_eq!((position.row, position.col), (row, 0));
    }
}

#[test]
fn searching_inside_a_selection_selects_every_match_within_it() {
    let mut app = app_with("foo bar foo baz foo");
    press(&mut app, '%');
    press(&mut app, 'S');
    submit(&mut app, "foo");
    assert_eq!(selection(&app).len(), 3);

    // Editing every match at once is the point of the feature.
    press(&mut app, 'c');
    press(&mut app, 'X');
    assert_eq!(text(&app), "X bar X baz X");
}

#[test]
fn matching_filters_keep_or_drop_ranges() {
    let mut app = app_with("keep\ndrop\nkeep");
    // One range per line, taken from a search rather than a separate splitting
    // command: `(?m)^.+$` is the whole-line match on every row.
    press(&mut app, 'S');
    submit(&mut app, "(?m)^.+$");
    assert_eq!(selection(&app).len(), 3);

    keys(&mut app, " sk");
    submit(&mut app, "keep");
    assert_eq!(selection(&app).len(), 2, "only the two 'keep' rows remain");

    keys(&mut app, " sr");
    submit(&mut app, "keep");
    assert!(app.status_error, "dropping every range is refused");
    assert_eq!(selection(&app).len(), 2);
}

#[test]
fn refusing_to_empty_the_selection_is_an_error_not_a_panic() {
    let mut app = app_with("aaa");
    press(&mut app, '%');
    app.status_error = false;

    keys(&mut app, " sk");
    submit(&mut app, "zzz");
    assert!(app.status_error);
    assert_eq!(selection(&app).len(), 1);
}

#[test]
fn alignment_pads_every_selection_to_one_column() {
    let mut app = app_with("a = 1\nbbbb = 2\ncc = 3");
    press(&mut app, 's');
    submit(&mut app, "= ");
    assert_eq!(selection(&app).len(), 3);

    press(&mut app, '&');
    let columns: Vec<usize> = (0..3)
        .map(|row| app.active_buffer().line_string(row).find('=').unwrap())
        .collect();
    assert_eq!(columns, vec![columns[0]; 3], "every '=' shares a column");
}

#[test]
fn alignment_moves_bare_cursors_to_the_rightmost_column() {
    // The multicursor case: carets left behind by a search sit at different
    // columns, and aligning pads the shorter lines until they line up.
    let mut app = app_with("one two\nlonger word two\nmid two");
    press(&mut app, 's');
    submit(&mut app, "two");
    assert_eq!(selection(&app).len(), 3);

    let column = |app: &App, range: &Range| app.active_buffer().position_of(range.from()).col;
    let before: Vec<usize> = selection(&app)
        .ranges()
        .iter()
        .map(|range| column(&app, range))
        .collect();
    assert_eq!(before, vec![4, 12, 4]);

    keys(&mut app, " sa");
    let after: Vec<usize> = selection(&app)
        .ranges()
        .iter()
        .map(|range| column(&app, range))
        .collect();
    assert_eq!(
        after,
        vec![12, 12, 12],
        "padding can only push right, so the rightmost caret is the target"
    );
    assert_eq!(
        text(&app),
        "one         two\nlonger word two\nmid         two"
    );
}

// -- Trimming -------------------------------------------------------------

#[test]
fn select_all_then_underscore_strips_trailing_whitespace_from_the_file() {
    let mut app = app_with("fn main() {   \n    let x;  \n      \n}");

    press(&mut app, '%');
    press(&mut app, '_');

    assert_eq!(
        text(&app),
        "fn main() {\n    let x;\n\n}",
        "indentation survives; a whitespace-only row is emptied outright"
    );
}

#[test]
fn underscore_trims_only_the_lines_the_selection_touches() {
    let mut app = app_with("one  \ntwo  \nthree  ");
    set_cursor(&mut app, 1, 0);

    press(&mut app, 'x');
    press(&mut app, '_');

    assert_eq!(
        text(&app),
        "one  \ntwo\nthree  ",
        "the untouched rows keep their trailing whitespace"
    );
}

#[test]
fn underscore_reports_when_there_is_nothing_to_trim() {
    let mut app = app_with("clean\nlines");
    let before = text(&app);

    press(&mut app, '%');
    press(&mut app, '_');

    assert_eq!(text(&app), before);
    assert!(!app.status_error, "an already-clean buffer is not an error");
}

#[test]
fn alt_underscore_shrinks_each_selection_past_its_whitespace() {
    let mut app = app_with("  hello  \nnext");

    press(&mut app, 'x');
    alt(&mut app, '_');

    let range = selection(&app).primary();
    assert_eq!(
        app.active_buffer().slice(range.from(), range.to() + 1),
        "hello",
        "the padding and the line terminator are dropped from the range"
    );
    assert_eq!(
        text(&app),
        "  hello  \nnext",
        "trim-selections is not an edit"
    );
}

#[test]
fn an_all_whitespace_range_collapses_to_a_caret() {
    let mut app = app_with("     \nnext");

    press(&mut app, 'x');
    alt(&mut app, '_');

    let range = selection(&app).primary();
    assert!(range.is_empty(), "nothing is left to select");
    assert_eq!(
        app.active_buffer().position_of(range.head),
        Position::new(0, 0)
    );
}

// -- Interaction with the rest of the editor ------------------------------

#[test]
fn caret_moves_to_the_first_non_whitespace_character() {
    let mut app = app_with("zero\n   first");
    set_cursor(&mut app, 1, 7);

    press(&mut app, '^');

    assert_eq!(app.cursor_position(), Position::new(1, 3));
}

#[test]
fn multi_cursor_survives_a_mode_round_trip() {
    let mut app = app_with("a\nb\nc");
    press(&mut app, 'C');
    press(&mut app, 'C');
    press(&mut app, 'i');
    app.handle_key(KeyStroke::new(KeyCode::Escape, Modifiers::NONE))
        .unwrap();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(selection(&app).len(), 3, "carets survive insert and escape");
}

#[test]
fn panes_sharing_a_buffer_both_follow_an_edit() {
    let mut app = app_with("hello");
    for character in [' ', 'w', 'v'] {
        press(&mut app, character);
    }
    let panes: Vec<usize> = app.panes.keys().copied().collect();
    assert_eq!(panes.len(), 2);
    for pane in &panes {
        assert_eq!(app.panes[pane].buffer, app.panes[&panes[0]].buffer);
    }

    // Put the inactive pane's caret past the insertion point.
    let other = *panes.iter().find(|id| **id != app.active_pane).unwrap();
    app.panes.get_mut(&other).unwrap().selection = Selection::point(4);

    press(&mut app, 'i');
    press(&mut app, 'Z');
    assert_eq!(text(&app), "Zhello");
    assert_eq!(
        app.panes[&other].selection.primary().head,
        5,
        "the other pane's caret shifted with the text"
    );
}
