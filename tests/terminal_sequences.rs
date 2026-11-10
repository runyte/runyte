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

fn assert_wide_cells(emulator: &Emulator) {
    for row in 0..emulator.rows() {
        for (column, cell) in emulator.grid().line(row).unwrap().iter().enumerate() {
            match cell.width {
                0 => assert!(column > 0 && self::cell(emulator, row, column - 1).width == 2),
                2 => assert!(
                    column + 1 < emulator.columns()
                        && self::cell(emulator, row, column + 1).width == 0
                ),
                1 => (),
                width => panic!("unexpected cell width {width}"),
            }
            if cell.width != 0 {
                assert_eq!(
                    runyte::grapheme::width(&cell.display_text()),
                    usize::from(cell.width)
                );
            }
        }
    }
}

#[test]
fn pasted_emoji_survive_fragmented_output_and_addressed_redraws() {
    for emoji in ["😃", "😶‍🌫️", "❤️", "1️⃣", "👍🏽", "🇵🇱"] {
        let output = format!("A{emoji}{emoji}B");
        for split in 0..=output.len() {
            let mut emulator = Emulator::new(12, 2);
            emulator.feed(&output.as_bytes()[..split]);
            emulator.feed(&output.as_bytes()[split..]);
            assert_eq!(screen(&emulator, 0), output);
            assert_eq!(cursor(&emulator), (0, 6));
            assert_eq!(cell(&emulator, 0, 1).text(), emoji);
            assert_eq!(cell(&emulator, 0, 3).text(), emoji);
            assert_wide_cells(&emulator);

            // A child can address the second half of a glyph, then erase or
            // overwrite it. Neither operation may leave a stale half behind.
            emulator.feed(b"\x1b[1;5H\x1b[K");
            assert_eq!(screen(&emulator, 0), format!("A{emoji}"));
            emulator.feed(b"\x1b[1;3Hx");
            assert_eq!(screen(&emulator, 0), "A x");
            assert_wide_cells(&emulator);
            emulator.feed(format!("\rA{emoji}B\x1b[K").as_bytes());
            assert_eq!(screen(&emulator, 0), format!("A{emoji}B"));
            assert_eq!(cursor(&emulator), (0, 4));
            assert_wide_cells(&emulator);
        }
    }
}

#[test]
fn shell_paste_redraw_and_backspace_keep_wide_cells_consistent() {
    // Fixed output captured from Bash/readline, with two bracketed-pasted
    // emoji, Left, End, Backspace, then a full redraw. Readline may delete a
    // sequence component; the terminal must honor its column commands.
    let mut emulator = Emulator::new(20, 2);
    emulator.feed("$ \x1b[7m😶‍🌫️😶‍🌫️\x1b[27m".as_bytes());
    emulator.feed("\r\x1b[C\x1b[C😶‍🌫️😶‍🌫️\x08".as_bytes());
    assert_eq!(cursor(&emulator), (0, 5));
    assert_eq!(cell(&emulator, 0, 5).width, 0);
    assert_eq!(screen(&emulator, 0), "$ 😶‍🌫️😶‍🌫️");
    emulator.feed(b"\x1b[C\x08\x1b[K");
    assert_eq!(screen(&emulator, 0), "$ 😶‍🌫️");
    assert_wide_cells(&emulator);
    emulator.feed("\x1b[H\x1b[2J$ 😶‍🌫️😶‍".as_bytes());
    assert_eq!(screen(&emulator, 0), "$ 😶‍🌫️😶‍");
    assert_wide_cells(&emulator);
}

#[test]
fn disabling_autowrap_before_a_selector_keeps_the_cluster_on_its_row() {
    let mut emulator = Emulator::new(3, 2);
    emulator.feed("ab❤\x1b[?7l".as_bytes());
    assert_eq!(emulator.feed("️".as_bytes()), 0);
    assert_eq!(screen(&emulator, 0), "ab❤️");
    assert_eq!(screen(&emulator, 1), "");
    assert_eq!(cell(&emulator, 0, 2).display_text(), "❤");
    assert_eq!(cursor(&emulator), (0, 2));
    assert_wide_cells(&emulator);
}

#[test]
fn widening_emoji_insert_the_extra_column_without_overwriting_following_text() {
    for emoji in ["❤️", "1️⃣", "😶‍🌫️"] {
        let mut emulator = Emulator::new(8, 2);
        emulator.feed(b"ab\xe7\x95\x8ccdef\r\x1b[4h");
        for byte in emoji.as_bytes() {
            emulator.feed(&[*byte]);
        }
        assert_eq!(screen(&emulator, 0), format!("{emoji}ab界cd"));
        assert_eq!(cursor(&emulator), (0, 2));
        assert_eq!(cell(&emulator, 0, 0).text(), emoji);
        assert_wide_cells(&emulator);
    }
}

#[test]
fn widening_emoji_wrap_whole_and_count_one_completed_line() {
    for rows in [1, 2] {
        for insert in [false, true] {
            let mut emulator = Emulator::new(3, rows);
            if insert {
                emulator.feed(b"\x1b[4h");
            }
            assert_eq!(emulator.feed("ab\x1b[31m❤".as_bytes()), 0);
            assert_eq!(emulator.feed("\x1b[32m️".as_bytes()), 1);
            let row = rows - 1;
            assert_eq!(screen(&emulator, row), "❤️");
            assert_eq!(cell(&emulator, row, 0).text(), "❤️");
            assert_eq!(cell(&emulator, row, 0).foreground, Color::Indexed(1));
            assert_eq!(cell(&emulator, row, 1).foreground, Color::Indexed(1));
            assert_eq!(cursor(&emulator), (row, 2));
            assert_eq!(emulator.grid().plain_line(0).unwrap(), "ab");
            assert_eq!(emulator.grid().scrollback_len(), usize::from(rows == 1));
            assert_wide_cells(&emulator);
        }
    }

    // At a two-column destination, widening leaves delayed wrap pending.
    let mut emulator = Emulator::new(2, 3);
    assert_eq!(emulator.feed("a❤️".as_bytes()), 1);
    assert_eq!(cursor(&emulator), (1, 1));
    assert!(emulator.grid().cursor.pending_wrap);
    assert_eq!(emulator.feed(b"x"), 1);
    assert_eq!(screen(&emulator, 1), "❤️");
    assert_eq!(screen(&emulator, 2), "x");
    assert_wide_cells(&emulator);
}

#[test]
fn widening_emoji_respect_insert_destinations_and_scroll_regions() {
    let mut emulator = Emulator::new(3, 2);
    emulator.feed("ab\r\ncde\x1b[1;3H\x1b[4h❤️".as_bytes());
    assert_eq!(screen(&emulator, 0), "ab");
    assert_eq!(screen(&emulator, 1), "❤️c");
    assert_wide_cells(&emulator);

    let mut emulator = Emulator::new(3, 3);
    emulator.feed(b"TOP\x1b[2;3r\x1b[3;1H");
    assert_eq!(emulator.feed("ab❤️".as_bytes()), 1);
    assert_eq!(screen(&emulator, 0), "TOP");
    assert_eq!(screen(&emulator, 1), "ab");
    assert_eq!(screen(&emulator, 2), "❤️");
    assert_eq!(emulator.grid().scrollback_len(), 0);
    assert_wide_cells(&emulator);

    emulator.feed(b"\x1b[?1049h");
    assert_eq!(emulator.feed("ab❤️".as_bytes()), 0);
    assert_eq!(screen(&emulator, 1), "❤️");
    assert_wide_cells(&emulator);
}

#[test]
fn cancelled_control_sequences_resume_printing_across_chunk_boundaries() {
    for prefix in [
        b"\x1b".as_slice(),
        b"\x1b(",
        b"\x1b[",
        b"\x1b[31",
        b"\x1b[ ",
        b"\x1b[1?",
        b"\x1b]2;unfinished title",
        b"\x1bPignored",
        b"\x1b_ignored",
        b"\x1b]2;unfinished title\x1b",
    ] {
        for cancel in [0x18, 0x1a] {
            let mut emulator = Emulator::new(20, 2);
            emulator.feed(b"before ");
            emulator.feed(prefix);
            emulator.feed(&[cancel]);
            emulator.feed(b"OK");
            assert_eq!(screen(&emulator, 0), "before OK", "{prefix:?}, {cancel}");
            assert!(emulator.title().is_none());
        }
    }
}

#[test]
fn escape_restarts_interrupted_control_sequences() {
    for prefix in [b"\x1b(".as_slice(), b"\x1b[31", b"\x1b[ ", b"\x1b[1?"] {
        let mut emulator = Emulator::new(20, 2);
        emulator.feed(b"before");
        emulator.feed(prefix);
        emulator.feed(b"\x1b[2J\x1b[Hafter");
        assert_eq!(screen(&emulator, 0), "after", "{prefix:?}");
    }
}

#[test]
fn unsupported_csi_intermediates_do_not_alias_basic_commands() {
    for sequence in [
        b"\x1b[2 @".as_slice(),
        b"\x1b[2 A",
        b"\x1b[2$J",
        b"\x1b[31 m",
        b"\x1b[?1049 h",
        b"\x1b[6 n",
        b"\x1b[ c",
    ] {
        let mut emulator = Emulator::new(12, 3);
        emulator.feed(b"first\r\nsecond\x1b[2;3H");
        let before = (0..3).map(|row| screen(&emulator, row)).collect::<Vec<_>>();
        emulator.feed(sequence);
        assert_eq!(cursor(&emulator), (1, 2), "{sequence:?}");
        assert_eq!(
            (0..3).map(|row| screen(&emulator, row)).collect::<Vec<_>>(),
            before,
            "{sequence:?}"
        );
        assert!(!emulator.alternate_screen());
        assert!(emulator.take_replies().is_empty());
        emulator.feed(b"X");
        assert_eq!(cell(&emulator, 1, 2).foreground, Color::Default);
        assert_eq!(screen(&emulator, 1), "seXond");
    }
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

#[test]
fn rep_reproduces_cpu_meter_runs_across_chunk_boundaries() {
    let bytes = b"\r\n|\x1b[9b!\r\n";
    for split in 0..=bytes.len() {
        let mut emulator = Emulator::new(20, 4);
        emulator.feed(&bytes[..split]);
        emulator.feed(&bytes[split..]);
        assert_eq!(screen(&emulator, 1), "||||||||||!");
        assert_eq!(cursor(&emulator), (2, 0));
    }
}

#[test]
fn rep_defaults_retains_printed_character_and_reset_forgets_it() {
    let mut emulator = Emulator::new(20, 3);
    emulator.feed(b"\x1b[3b");
    assert_eq!(cursor(&emulator), (0, 0));
    emulator.feed(b"A\x1b[b\x1b[0b\x1b[2b");
    assert_eq!(screen(&emulator, 0), "AAAAA");
    // Cursor motion and erasure do not replace the last printed character.
    emulator.feed(b"\r\x1b[2K\x1b[2b");
    assert_eq!(screen(&emulator, 0), "AA");
    emulator.feed(b"\x1bc\x1b[2b");
    assert_eq!(screen(&emulator, 0), "");
    assert_eq!(cursor(&emulator), (0, 0));
    emulator.feed(b"B\x1b[?3b\x1b[3 b!");
    assert_eq!(screen(&emulator, 0), "B!");
}

#[test]
fn rep_uses_current_rendition_insert_mode_and_printed_unicode() {
    let mut emulator = Emulator::new(20, 3);
    emulator.feed(b"abcd\r\x1b[4h\x1b[31m|\x1b[32m\x1b[2b");
    assert_eq!(screen(&emulator, 0), "|||abcd");
    assert_eq!(cell(&emulator, 0, 0).foreground, Color::Indexed(1));
    assert_eq!(cell(&emulator, 0, 1).foreground, Color::Indexed(2));
    assert_eq!(cell(&emulator, 0, 2).foreground, Color::Indexed(2));
    emulator.feed("\x1b[4l\r\n界\x1b[2b!".as_bytes());
    assert_eq!(screen(&emulator, 1), "界界界!");
    assert_eq!(cursor(&emulator), (1, 7));
    assert_wide_cells(&emulator);
}

#[test]
fn rep_wraps_scrolls_and_accounts_for_completed_lines() {
    let mut emulator = Emulator::new(4, 2);
    assert_eq!(emulator.feed(b"xxxx\x1b[4b\x1b[2b"), 2);
    assert_eq!(emulator.plain_text(), "xxxx\nxxxx\nxx\n");
    assert_eq!(cursor(&emulator), (1, 2));
    emulator.feed(b"\x1b[?1049h");
    assert_eq!(emulator.feed(b"yyyy\x1b[4b\x1b[2b"), 0);
    assert_eq!(screen(&emulator, 0), "yyyy");
    assert_eq!(screen(&emulator, 1), "yy");
    emulator.feed(b"\x1b[?7l\x1b[20b!");
    assert_eq!(screen(&emulator, 1), "yyy!");
}

#[test]
fn rep_large_parameters_are_limited_to_one_screen_width() {
    let mut emulator = Emulator::new(256, 2);
    assert_eq!(emulator.feed(b"x\x1b[999999999999999999b!"), 1);
    assert_eq!(cursor(&emulator), (1, 2));
    assert_eq!(emulator.grid().scrollback_len(), 0);
    assert_eq!(screen(&emulator, 0), "x".repeat(256));
    assert_eq!(screen(&emulator, 1), "x!");
}

#[test]
fn rep_matches_literal_output_in_scroll_regions_and_clusters() {
    for (prefix, character, count) in [
        ("top\r\nmid\r\nend\x1b[2;3r\x1b[3;4H", 'x', 3),
        ("\x1b[1;4H", '界', 3),
        ("e", '\u{301}', 3),
        ("\x1b[?7l\x1b[1;4H", 'x', 3),
    ] {
        let mut repeated = Emulator::new(4, 4);
        let mut literal = Emulator::new(4, 4);
        let initial = format!("{prefix}{character}");
        repeated.feed(initial.as_bytes());
        literal.feed(initial.as_bytes());
        let repeated_lines = repeated.feed(format!("\x1b[{count}b").as_bytes());
        let literal_lines = literal.feed(character.to_string().repeat(count).as_bytes());
        assert_eq!(repeated_lines, literal_lines);
        assert_eq!(repeated.plain_text(), literal.plain_text());
        assert_eq!(cursor(&repeated), cursor(&literal));
        for row in 0..4 {
            assert_eq!(repeated.grid().line(row), literal.grid().line(row));
        }
        assert_wide_cells(&repeated);
    }
}
