// SPDX-License-Identifier: MPL-2.0

//! Fixed control sequences applied straight to the emulator.
//!
//! `tests/terminal.rs` drives a real child on a real pseudoterminal, which is
//! how the pane as a whole is proven. The sequences below need no child: they
//! are the VT vocabulary a full-screen program addresses the screen with, and
//! each one has an answer that is either right or wrong. Feeding them
//! directly keeps that answer legible and keeps a compatibility regression
//! from having to be reproduced through a shell first.

use runyte::terminal::{Attributes, Cell, Color, emulator::Emulator};

/// One screen row as text, past whatever has already scrolled off the top.
fn screen(emulator: &Emulator, row: usize) -> String {
    let grid = emulator.grid();
    grid.plain_line(grid.scrollback_len() + row)
        .expect("row is on the screen")
}

fn cell(emulator: &Emulator, row: usize, column: usize) -> Cell {
    emulator.grid().line(row).expect("row is on the screen")[column]
}

fn cursor(emulator: &Emulator) -> (usize, usize) {
    let cursor = emulator.grid().cursor;
    (cursor.row, cursor.column)
}

#[test]
fn cursor_motion_sequences_address_the_screen_by_row_and_column() {
    let mut emulator = Emulator::new(20, 10);

    emulator.feed(b"\x1b[5;7H");
    assert_eq!(cursor(&emulator), (4, 6), "CUP is one-based in both axes");
    emulator.feed(b"\x1b[2A");
    assert_eq!(cursor(&emulator), (2, 6), "CUU");
    emulator.feed(b"\x1b[3B");
    assert_eq!(cursor(&emulator), (5, 6), "CUD");
    emulator.feed(b"\x1b[4C");
    assert_eq!(cursor(&emulator), (5, 10), "CUF");
    emulator.feed(b"\x1b[2D");
    assert_eq!(cursor(&emulator), (5, 8), "CUB");
    emulator.feed(b"\x1b[2E");
    assert_eq!(
        cursor(&emulator),
        (7, 0),
        "CNL moves down and to column one"
    );
    emulator.feed(b"\x1b[3F");
    assert_eq!(cursor(&emulator), (4, 0), "CPL moves up and to column one");
    emulator.feed(b"\x1b[12G");
    assert_eq!(cursor(&emulator), (4, 11), "CHA addresses a column");
    emulator.feed(b"\x1b[9d");
    assert_eq!(cursor(&emulator), (8, 11), "VPA addresses a row");

    emulator.feed(b"\x1b[99C");
    assert_eq!(cursor(&emulator), (8, 19), "CUF stops at the last column");
    emulator.feed(b"\x1b[99D");
    assert_eq!(cursor(&emulator), (8, 0), "CUB stops at the first column");
    emulator.feed(b"\x1b[99B");
    assert_eq!(cursor(&emulator), (9, 0), "CUD stops at the last row");
    emulator.feed(b"\x1b[99A");
    assert_eq!(cursor(&emulator), (0, 0), "CUU stops at the first row");
}

/// Vertical motion is bounded by the scrolling region only for a cursor that
/// starts inside it. A cursor an application has parked outside the region —
/// in a status area above or below it — keeps the whole screen to move in,
/// which is what stops the region from trapping it there.
#[test]
fn vertical_motion_bounds_are_the_region_only_from_inside_it() {
    let mut emulator = Emulator::new(20, 10);
    emulator.feed(b"\x1b[3;8r");
    assert_eq!(
        emulator.grid().scroll_region(),
        (2, 7),
        "DECSTBM is one-based and inclusive"
    );
    assert_eq!(
        cursor(&emulator),
        (0, 0),
        "without origin mode DECSTBM homes to the screen, not to the region"
    );

    emulator.feed(b"\x1b[5;1H\x1b[99A");
    assert_eq!(
        cursor(&emulator),
        (2, 0),
        "from inside, CUU stops at the top"
    );
    emulator.feed(b"\x1b[99B");
    assert_eq!(
        cursor(&emulator),
        (7, 0),
        "from inside, CUD stops at the foot"
    );

    emulator.feed(b"\x1b[2;1H\x1b[99A");
    assert_eq!(
        cursor(&emulator),
        (0, 0),
        "from above the region, CUU reaches the first screen row"
    );
    emulator.feed(b"\x1b[10;1H\x1b[99B");
    assert_eq!(
        cursor(&emulator),
        (9, 0),
        "from below the region, CUD reaches the last screen row"
    );
}

/// Origin mode makes every row an application names relative to the region it
/// set, so a full-screen program can address its own pane without arithmetic.
#[test]
fn origin_mode_addresses_and_reports_rows_inside_the_scrolling_region() {
    let mut emulator = Emulator::new(20, 8);
    emulator.feed(b"\x1b[3;6r\x1b[?6h");
    assert_eq!(
        cursor(&emulator),
        (2, 0),
        "origin mode homes into the region"
    );

    emulator.feed(b"\x1b[2;1H");
    assert_eq!(
        cursor(&emulator),
        (3, 0),
        "row two is the region's second row"
    );
    emulator.feed(b"\x1b[9;1H");
    assert_eq!(
        cursor(&emulator),
        (5, 0),
        "a row past the region's foot stops there"
    );

    let _ = emulator.take_replies();
    emulator.feed(b"\x1b[6n");
    assert_eq!(
        emulator.take_replies(),
        b"\x1b[4;1R".to_vec(),
        "the cursor is reported relative to the region"
    );

    emulator.feed(b"\x1b[?6l");
    assert_eq!(
        cursor(&emulator),
        (0, 0),
        "leaving origin mode homes the screen"
    );
    emulator.feed(b"\x1b[2;1H\x1b[6n");
    assert_eq!(
        emulator.take_replies(),
        b"\x1b[2;1R".to_vec(),
        "without origin mode the report is absolute"
    );
}

#[test]
fn device_queries_are_answered_without_asking_the_child_again() {
    let mut emulator = Emulator::new(20, 4);
    emulator.feed(b"\x1b[c");
    assert_eq!(
        emulator.take_replies(),
        b"\x1b[?6c".to_vec(),
        "primary device attributes"
    );
    emulator.feed(b"\x1b[5n");
    assert_eq!(
        emulator.take_replies(),
        b"\x1b[0n".to_vec(),
        "device status"
    );
}

#[test]
fn tab_sequences_move_between_stops_and_clear_them() {
    let mut emulator = Emulator::new(40, 4);

    emulator.feed(b"\x1b[1;21H\x1b[Z");
    assert_eq!(cursor(&emulator).1, 16, "CBT reaches the previous stop");
    emulator.feed(b"\x1b[2Z");
    assert_eq!(cursor(&emulator).1, 0, "CBT counts stops");
    emulator.feed(b"\x1b[3I");
    assert_eq!(cursor(&emulator).1, 24, "CHT counts stops forward");

    emulator.feed(b"\x1b[1;4H\x1bH\x1b[1;1H\x1b[I");
    assert_eq!(cursor(&emulator).1, 3, "HTS sets a stop at the cursor");

    emulator.feed(b"\x1b[1;4H\x1b[g\x1b[1;1H\x1b[I");
    assert_eq!(
        cursor(&emulator).1,
        8,
        "TBC clears the stop the cursor is on"
    );

    emulator.feed(b"\x1b[3g\x1b[1;1H\x1b[I");
    assert_eq!(
        cursor(&emulator).1,
        39,
        "with every stop cleared a tab runs to the last column"
    );
}

#[test]
fn character_editing_sequences_shift_and_blank_within_one_row() {
    let mut emulator = Emulator::new(10, 4);
    emulator.feed(b"abcdefghij\x1b[1;1H");

    emulator.feed(b"\x1b[3@");
    assert_eq!(
        screen(&emulator, 0),
        "   abcdefg",
        "ICH shifts the row right and drops what leaves it"
    );

    emulator.feed(b"\x1b[3P");
    assert_eq!(
        screen(&emulator, 0),
        "abcdefg",
        "DCH shifts the rest of the row left"
    );

    emulator.feed(b"\x1b[1;3H\x1b[2X");
    assert_eq!(
        screen(&emulator, 0),
        "ab  efg",
        "ECH blanks cells without shifting"
    );
}

#[test]
fn line_editing_sequences_shift_rows_within_the_scrolling_region() {
    let mut emulator = Emulator::new(10, 5);
    emulator.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");

    emulator.feed(b"\x1b[2;1H\x1b[M");
    assert_eq!(
        (0..5).map(|row| screen(&emulator, row)).collect::<Vec<_>>(),
        ["one", "three", "four", "five", ""],
        "DL removes the cursor's row and blanks the region's foot"
    );

    emulator.feed(b"\x1b[L");
    assert_eq!(
        (0..5).map(|row| screen(&emulator, row)).collect::<Vec<_>>(),
        ["one", "", "three", "four", "five"],
        "IL opens a row at the cursor and drops the region's foot"
    );

    emulator.feed(b"\x1b[2T");
    assert_eq!(
        (0..5).map(|row| screen(&emulator, row)).collect::<Vec<_>>(),
        ["", "", "one", "", "three"],
        "SD moves the region down and discards what leaves its foot"
    );
    assert_eq!(
        emulator.grid().scrollback_len(),
        0,
        "nothing leaves the top of the screen when the region moves down"
    );
}

/// A reverse index at the region's head scrolls it down rather than moving the
/// cursor off the top, which is how an application inserts a line above the
/// one it is on.
#[test]
fn reverse_index_scrolls_at_the_head_and_moves_the_cursor_elsewhere() {
    let mut emulator = Emulator::new(10, 4);
    emulator.feed(b"one\r\ntwo\r\nthree");

    emulator.feed(b"\x1b[3;1H\x1bM");
    assert_eq!(
        cursor(&emulator),
        (1, 0),
        "away from the head it only moves"
    );

    emulator.feed(b"\x1b[1;1H\x1bM");
    assert_eq!(cursor(&emulator), (0, 0), "at the head the cursor stays");
    assert_eq!(
        (0..4).map(|row| screen(&emulator, row)).collect::<Vec<_>>(),
        ["", "one", "two", "three"],
        "the region moved down instead"
    );
}

#[test]
fn the_saved_cursor_is_restored_by_both_the_escape_and_the_csi_form() {
    let mut emulator = Emulator::new(10, 5);

    emulator.feed(b"\x1b[3;4H\x1b[1m\x1b7");
    emulator.feed(b"\x1b[1;1H\x1b[0m");
    emulator.feed(b"\x1b8");
    assert_eq!(cursor(&emulator), (2, 3), "DECRC restores the position");
    emulator.feed(b"X");
    assert!(
        cell(&emulator, 2, 3).attributes.contains(Attributes::BOLD),
        "DECRC restores the pen the cursor was saved with"
    );

    emulator.feed(b"\x1b[5;6H\x1b[s\x1b[1;1H\x1b[u");
    assert_eq!(cursor(&emulator), (4, 5), "the CSI form restores it too");
}

#[test]
fn graphic_rendition_attributes_are_set_and_cleared_one_at_a_time() {
    let mut emulator = Emulator::new(20, 3);

    emulator.feed(b"\x1b[3;4;5;7;8;9mA");
    let attributes = cell(&emulator, 0, 0).attributes;
    for (attribute, name) in [
        (Attributes::ITALIC, "italic"),
        (Attributes::UNDERLINE, "underline"),
        (Attributes::BLINK, "blink"),
        (Attributes::REVERSE, "reverse"),
        (Attributes::HIDDEN, "hidden"),
        (Attributes::STRIKETHROUGH, "strikethrough"),
    ] {
        assert!(attributes.contains(attribute), "{name} was not set");
    }

    emulator.feed(b"\x1b[23;24;25;27;28;29mB");
    assert!(
        cell(&emulator, 0, 1).attributes.is_empty(),
        "each attribute has its own reset"
    );

    emulator.feed(b"\x1b[6mC");
    assert!(
        cell(&emulator, 0, 2).attributes.contains(Attributes::BLINK),
        "the second blink parameter is the same attribute"
    );

    emulator.feed(b"\x1b[0m\x1b[105mD");
    assert_eq!(
        cell(&emulator, 0, 3).background,
        Color::Indexed(13),
        "a bright background names the upper half of the basic palette"
    );
}

#[test]
fn insert_mode_shifts_the_row_and_replace_mode_overwrites_it() {
    let mut emulator = Emulator::new(10, 3);
    emulator.feed(b"abcdef");

    emulator.feed(b"\x1b[1;1H\x1b[4hXY");
    assert_eq!(
        screen(&emulator, 0),
        "XYabcdef",
        "IRM makes a written character open a cell"
    );

    emulator.feed(b"\x1b[4l\x1b[1;1HZ");
    assert_eq!(
        screen(&emulator, 0),
        "ZYabcdef",
        "with IRM off the character replaces the cell"
    );
}

#[test]
fn autowrap_can_be_turned_off_so_the_final_column_is_overwritten() {
    let mut emulator = Emulator::new(4, 3);
    emulator.feed(b"\x1b[?7l");
    assert!(!emulator.modes.autowrap, "DECAWM off");
    emulator.feed(b"abcdef");
    assert_eq!(screen(&emulator, 0), "abcf", "the last column is rewritten");
    assert_eq!(
        screen(&emulator, 1),
        "",
        "nothing wrapped onto the next row"
    );

    emulator.feed(b"\x1b[?7h");
    assert!(emulator.modes.autowrap, "DECAWM on");
}

#[test]
fn keypad_and_character_set_sequences_are_recorded_or_ignored_but_never_printed() {
    let mut emulator = Emulator::new(10, 3);

    emulator.feed(b"\x1b=");
    assert!(emulator.modes.application_keypad, "DECKPAM");
    emulator.feed(b"\x1b>");
    assert!(!emulator.modes.application_keypad, "DECKPNM");

    emulator.feed(b"\x1b(B\x1b)0\x1b#8ok");
    assert_eq!(
        screen(&emulator, 0),
        "ok",
        "character-set and line-attribute designations print nothing"
    );
}
