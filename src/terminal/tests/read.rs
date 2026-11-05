// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::terminal::{
    TerminalOutput, TerminalSessions, grid::DEFAULT_SCROLLBACK_LINES, tests::session,
};

fn capture(session: &TerminalSession, region: Region) -> Snapshot {
    session
        .read_output(region, Limits::default(), None)
        .unwrap()
}

fn texts(snapshot: &Snapshot) -> Vec<&str> {
    snapshot.rows.iter().map(|row| row.text.as_str()).collect()
}

#[test]
fn screen_excludes_history_and_tail_returns_newest_rows_in_source_order() {
    let mut terminal = session(12, 2);
    terminal.feed(b"one\r\ntwo\r\nthree\r\nfour");
    let screen = capture(&terminal, Region::Screen);
    assert_eq!(texts(&screen), ["three", "four"]);
    assert_eq!(screen.history_rows, 2);
    assert_eq!(screen.available_rows, 2);
    assert_eq!(screen.rows[0].index, 2);
    assert_eq!(screen.lost_history_rows, 0);
    let tail = terminal
        .read_output(
            Region::Tail,
            Limits {
                rows: 3,
                ..Limits::default()
            },
            Some(screen.revision),
        )
        .unwrap();
    assert_eq!(texts(&tail), ["two", "three", "four"]);
    assert_eq!(tail.available_rows, 4);
    assert!(tail.truncation.rows);
    assert!(!tail.truncation.bytes && !tail.truncation.cells);
    assert_eq!(tail.rows[1].id, screen.rows[0].id);
    assert_eq!(tail.terminal, terminal.id());
    assert!(tail.live);
}

#[test]
fn byte_budget_favors_newest_tail_rows_without_splitting_a_cell() {
    let mut terminal = session(8, 3);
    terminal.feed("older\r\n界\u{301}\r\nend".as_bytes());
    let tail = terminal
        .read_output(
            Region::Tail,
            Limits {
                bytes: 8,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&tail), ["", "界\u{301}", "end"]);
    assert!(tail.rows[0].clipped);
    assert_eq!(tail.rows[0].columns, 0);
    assert!(tail.truncation.bytes);
    assert_eq!(tail.rows.iter().map(|row| row.text.len()).sum::<usize>(), 8);
    let less = terminal
        .read_output(
            Region::Tail,
            Limits {
                bytes: 7,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&less), ["", "end"]);
    assert!(less.rows[0].clipped);
    assert_eq!(less.rows[0].columns, 0);
}

#[test]
fn cell_budget_bounds_wide_blank_rows_without_materializing_them() {
    let mut terminal = session(10_000, 2);
    terminal.feed(b"\x1b[2;9999HX");
    let result = terminal
        .read_output(
            Region::Tail,
            Limits {
                cells: 5,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&result), [""]);
    assert_eq!(result.rows[0].columns, 5);
    assert!(result.rows[0].clipped);
    assert_eq!(result.visited_cells, 5);
    assert_eq!(
        result.truncation,
        Truncation {
            cells: true,
            ..Truncation::default()
        }
    );
}

#[test]
fn wide_cells_and_combining_marks_are_atomic_at_limits() {
    let mut terminal = session(8, 1);
    terminal.feed("界e\u{301} x\u{301}  ".as_bytes());
    let result = capture(&terminal, Region::Screen);
    assert_eq!(texts(&result), ["界e\u{301} x\u{301}"]);
    assert_eq!(result.visited_cells, 8);
    assert_eq!(result.rows[0].columns, 8);
    for limit in [1, 2] {
        let result = terminal
            .read_output(
                Region::Screen,
                Limits {
                    bytes: limit,
                    ..Limits::default()
                },
                None,
            )
            .unwrap();
        assert_eq!(texts(&result), [""]);
        assert!(result.truncation.bytes);
        assert_eq!(result.rows[0].columns, 0);
    }
    let result = terminal
        .read_output(
            Region::Screen,
            Limits {
                cells: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&result), [""]);
    assert_eq!(result.rows[0].columns, 0);
    assert_eq!(result.visited_cells, 1);
    assert!(result.truncation.cells);
    let result = terminal
        .read_output(
            Region::Screen,
            Limits {
                cells: 2,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&result), ["界"]);
    assert_eq!(result.rows[0].columns, 2);
}

#[test]
fn blank_rows_survive_and_do_not_consume_the_text_byte_budget() {
    let mut terminal = session(4, 3);
    terminal.feed(b"\x1b[3;1HX");
    let result = terminal
        .read_output(
            Region::Tail,
            Limits {
                bytes: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&result), ["", "", "X"]);
    assert_eq!(result.truncation, Truncation::default());
    let clipped = terminal
        .read_output(
            Region::Screen,
            Limits {
                cells: 4,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&clipped), [""]);
    assert!(!clipped.rows[0].clipped);
    assert!(clipped.truncation.cells);
    let screen = terminal
        .read_output(
            Region::Screen,
            Limits {
                rows: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&screen), [""]);
    assert!(screen.truncation.rows);
}

#[test]
fn internal_spaces_are_preserved_without_allocating_trailing_blanks() {
    let mut terminal = session(16, 1);
    terminal.feed(b" a  b ");
    assert_eq!(texts(&capture(&terminal, Region::Screen)), [" a  b"]);
    let result = terminal
        .read_output(
            Region::Screen,
            Limits {
                bytes: 3,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(texts(&result), [" a"]);
    assert_eq!(result.rows[0].columns, 4);
    assert!(result.rows[0].clipped);
    assert!(result.truncation.bytes);
}

#[test]
fn capture_removes_styles_and_osc_payloads_but_preserves_unicode() {
    let mut terminal = session(20, 2);
    terminal.feed("\x1b[31mé\x1b[0m\x1b]0;not content\x07\r\nnext".as_bytes());
    assert_eq!(texts(&capture(&terminal, Region::Tail)), ["é", "next"]);
}

#[test]
fn alternate_screen_never_splices_primary_history() {
    let mut terminal = session(12, 2);
    terminal.feed(b"old\r\nprimary\r\nnewest");
    let primary = capture(&terminal, Region::Tail);
    terminal.feed(b"\x1b[?1049hALT");
    let alternate = capture(&terminal, Region::Tail);
    assert!(alternate.alternate_screen);
    assert_eq!(alternate.history_rows, 0);
    assert_eq!(alternate.lost_history_rows, 0);
    assert_eq!(texts(&alternate), ["ALT", ""]);
    assert_ne!(primary.revision, alternate.revision);
    terminal.feed(b"\x1b[?1049l");
    let restored = capture(&terminal, Region::Tail);
    assert!(!restored.alternate_screen);
    assert_eq!(texts(&restored), texts(&primary));
    assert_ne!(restored.revision, primary.revision);
}

#[test]
fn top_anchored_inline_output_remains_available_before_the_composer() {
    let mut terminal = session(12, 3);
    terminal.feed(b"\x1b[3;1Hcomposer\x1b[1;2r\x1b[1;1Hone\r\ntwo\r\nthree");
    let result = capture(&terminal, Region::Tail);
    assert_eq!(texts(&result), ["one", "two", "three", "composer"]);
    assert_eq!(result.history_rows, 1);
}

#[test]
fn snapshots_are_immutable_and_revisions_cover_rewrites_resize_and_reset() {
    let mut terminal = session(10, 2);
    terminal.feed(b"before");
    let before = capture(&terminal, Region::Screen);
    terminal.feed(b"\rafter ");
    assert_eq!(texts(&before), ["before", ""]);
    assert_eq!(texts(&capture(&terminal, Region::Screen)), ["after", ""]);
    assert!(matches!(
        terminal.read_output(Region::Tail, Limits::default(), Some(before.revision)),
        Err(ReadError::Stale { .. })
    ));
    let revision = terminal.read_revision();
    assert!(!terminal.resize(10, 2));
    assert_eq!(terminal.read_revision(), revision);
    assert!(terminal.resize(8, 1));
    assert_ne!(terminal.read_revision(), revision);
    let resized = terminal.read_revision();
    assert!(terminal.resize(10, 2));
    assert_ne!(terminal.read_revision(), resized);
    for sequence in [b"\x1b[2J".as_slice(), b"\x1bc", b"partial"] {
        let revision = terminal.read_revision();
        terminal.feed(sequence);
        assert_ne!(terminal.read_revision(), revision);
    }
}

#[test]
fn reading_live_output_leaves_frozen_review_attention_and_viewport_untouched() {
    let mut terminal = session(12, 2);
    terminal.feed(b"one\r\ntwo\r\nthree\x07");
    let revision = terminal.read_revision();
    terminal.begin_review();
    terminal.select_all_review();
    terminal.scroll_to_oldest();
    assert_eq!(terminal.read_revision(), revision);
    terminal.feed(b"\rnew");
    let review_text = terminal.review_selection_text();
    let scroll = terminal.scroll();
    let view = terminal.view(2);
    let result = capture(&terminal, Region::Screen);
    assert_eq!(texts(&result), ["two", "newee"]);
    assert!(terminal.reviewing());
    assert_eq!(terminal.review_selection_text(), review_text);
    assert_eq!(terminal.scroll(), scroll);
    assert_eq!(terminal.view(2), view);
    let preview = terminal.preview_view();
    assert!(!preview.review);
    assert_eq!(preview.scrollback, 0);
    assert_eq!(
        preview
            .rows
            .iter()
            .map(|row| row
                .iter()
                .map(|cell| cell.text())
                .collect::<String>()
                .trim_end()
                .to_owned())
            .collect::<Vec<_>>(),
        ["two", "newee"]
    );
    assert_eq!(terminal.view(2), view);
    assert!(terminal.unread_activity());
    assert!(terminal.bell());
    let revision = terminal.read_revision();
    terminal.mark_viewed();
    terminal.scroll_to_live();
    assert_eq!(terminal.read_revision(), revision);
}

#[test]
fn lost_history_distinguishes_capacity_eviction_clear_and_emulator_reset() {
    let mut terminal = session(2, 1);
    for _ in 0..DEFAULT_SCROLLBACK_LINES + 3 {
        terminal.feed(b"x\r\n");
    }
    let result = capture(&terminal, Region::Tail);
    assert_eq!(result.history_rows, DEFAULT_SCROLLBACK_LINES);
    assert_eq!(result.lost_history_rows, 3);
    terminal.feed(b"\x1b[3J");
    let cleared = capture(&terminal, Region::Tail);
    assert_eq!(cleared.history_rows, 0);
    assert_eq!(
        cleared.lost_history_rows,
        DEFAULT_SCROLLBACK_LINES as u64 + 3
    );
    assert_ne!(cleared.revision, result.revision);
    terminal.feed(b"\x1bc");
    assert_eq!(capture(&terminal, Region::Tail).lost_history_rows, 0);
}

#[test]
fn terminal_history_limits_are_independent_and_reviews_survive_trimming() {
    let mut terminals = TerminalSessions::new();
    terminals.set_scrollback_lines(3);
    let first = terminals.insert_test_session(10, 2);
    let second = terminals.insert_test_session(10, 2);
    terminals.apply(TerminalOutput::Bytes {
        id: first,
        bytes: b"one\r\ntwo\r\nthree\r\nfour".to_vec(),
    });
    let terminal = terminals.get_mut(first).unwrap();
    terminal.search_review("two", false).unwrap();
    let view = terminal.view(2);
    let revision = terminal.read_revision();
    terminals.apply(TerminalOutput::Bytes {
        id: second,
        bytes: b"a\r\nb\r\nc\r\nd\r\ne\r\nf".to_vec(),
    });
    assert_eq!(terminals.get(first).unwrap().scrollback_rows(), 2);
    assert_eq!(terminals.get(second).unwrap().scrollback_rows(), 3);
    assert_eq!(terminals.get(first).unwrap().read_revision(), revision);
    terminals.set_scrollback_lines(1);
    let terminal = terminals.get_mut(first).unwrap();
    assert_eq!(terminal.scrollback_rows(), 1);
    assert_ne!(terminal.read_revision(), revision);
    assert_eq!(terminal.view(2).rows, view.rows);
    assert_eq!(terminal.review_selection_text(), "two");
    let snapshot = capture(terminal, Region::Tail);
    assert_eq!(texts(&snapshot), ["two", "three", "four"]);
    assert_eq!(snapshot.lost_history_rows, 1);
    terminal.scroll_to_live();
    assert!(!terminal.reviewing());
    terminals.set_scrollback_lines(4);
    assert_eq!(terminals.get(first).unwrap().scrollback_rows(), 1);
    let third = terminals.insert_test_session(10, 2);
    terminals.apply(TerminalOutput::Bytes {
        id: third,
        bytes: b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7".to_vec(),
    });
    assert_eq!(terminals.get(third).unwrap().scrollback_rows(), 4);
}

#[test]
fn alternate_screen_limit_changes_trim_primary_history_and_survive_reset() {
    let mut terminal = session(10, 2);
    terminal.feed(b"one\r\ntwo\r\nthree\r\nfour");
    terminal.feed(b"\x1b[?1049hfull screen");
    let alternate = capture(&terminal, Region::Screen);
    let revision = terminal.read_revision();
    terminal.set_scrollback_lines(1);
    let after = capture(&terminal, Region::Screen);
    assert_eq!(after.rows, alternate.rows);
    assert!(after.alternate_screen);
    assert_eq!(after.lost_history_rows, 0);
    assert_ne!(after.revision, revision);
    terminal.feed(b"\x1b[?1049l");
    let primary = capture(&terminal, Region::Tail);
    assert_eq!(texts(&primary), ["two", "three", "four"]);
    assert_eq!(primary.history_rows, 1);
    assert_eq!(primary.lost_history_rows, 1);
    terminal.feed(b"\x1bcone\r\ntwo\r\nthree\r\nfour");
    assert_eq!(terminal.scrollback_rows(), 1);
    terminal.set_scrollback_lines(0);
    terminal.feed(b"\x1bcone\r\ntwo\r\nthree");
    assert_eq!(terminal.scrollback_rows(), 0);
    terminal.select_all_review();
    assert_eq!(terminal.review_selection_text(), "two\nthree");
}

#[test]
fn line_activation_reuses_review_and_refuses_to_replace_it_with_new_output() {
    let mut terminal = session(10, 2);
    terminal.feed(b"one\r\ntwo");
    let first = terminal.emulator.grid().retained_lines().next().unwrap().0;
    assert!(terminal.begin_review_at_line(first));
    let frozen = terminal.view(2).rows;
    terminal.set_scrollback_lines(0);
    terminal.feed(b"\r\nthree\r\nfour");
    let newest = terminal.emulator.grid().retained_lines().last().unwrap().0;
    assert!(!terminal.begin_review_at_line(newest));
    assert!(terminal.begin_review_at_line(first));
    assert_eq!(terminal.view(2).rows, frozen);
    terminal.scroll_to_live();
    assert!(terminal.begin_review_at_line(newest));
    assert_ne!(terminal.view(2).rows, frozen);
}

#[test]
fn exited_retained_terminal_remains_readable_with_a_new_revision() {
    let mut terminals = TerminalSessions::new();
    let mut terminal = session(8, 1);
    terminal.feed(b"done");
    let revision = terminal.read_revision();
    let id = terminal.id();
    terminals.sessions.insert(id, terminal);
    terminals.apply(TerminalOutput::Exited { id, code: Some(0) });
    let snapshot = capture(terminals.get(id).unwrap(), Region::Tail);
    assert!(!snapshot.live);
    assert_ne!(snapshot.revision, revision);
    assert_eq!(texts(&snapshot), ["done"]);
    assert!(terminals.close(id));
    assert!(terminals.get(id).is_none());
    assert_eq!(texts(&snapshot), ["done"]);
}

#[test]
fn invalid_limits_are_rejected_before_any_capture() {
    let terminal = session(4, 1);
    for limits in [
        Limits {
            rows: 0,
            ..Limits::default()
        },
        Limits {
            rows: MAX_ROWS + 1,
            ..Limits::default()
        },
        Limits {
            bytes: 0,
            ..Limits::default()
        },
        Limits {
            bytes: MAX_BYTES + 1,
            ..Limits::default()
        },
        Limits {
            cells: 0,
            ..Limits::default()
        },
        Limits {
            cells: MAX_CELLS + 1,
            ..Limits::default()
        },
    ] {
        assert_eq!(
            terminal.read_output(Region::Screen, limits, None),
            Err(ReadError::InvalidLimits)
        );
    }
    assert_eq!(
        terminal.read_output(Region::Screen, Limits::default(), Some(0)),
        Err(ReadError::Stale {
            actual_revision: terminal.read_revision()
        })
    );
}
