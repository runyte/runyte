// SPDX-License-Identifier: MPL-2.0

//! Phase 0 gate: the text substrate and its transactional history.
//!
//! These cover the properties the rest of V4 is built on — exact inversion,
//! single-step multi-range undo, and history whose cost tracks edit size rather
//! than document size.

use runyte::{
    buffer::Buffer,
    text::{Assoc, Change, Position, Text, Transaction},
};

fn buffer_of(text: &str) -> Buffer {
    let mut buffer = Buffer::scratch();
    buffer.apply(&Transaction::insert(0, text));
    buffer
}

#[test]
fn a_transaction_and_its_inverse_round_trip_exactly() {
    let original = "first line\nsecond line\nthird line";
    let mut text = Text::from_str(original);
    let transaction = Transaction::new(vec![
        Change::new(0, 5, "1st"),
        Change::new(11, 17, "2nd"),
        Change::new(23, 28, "3rd"),
    ]);
    let revert = text.apply(&transaction);
    assert_ne!(text.to_string(), original);
    text.apply(&revert.into_transaction());
    assert_eq!(text.to_string(), original);
}

#[test]
fn changes_spanning_line_boundaries_invert() {
    let original = "alpha\nbeta\ngamma\n";
    let mut text = Text::from_str(original);
    let transaction = Transaction::change(3, 13, "X\nY\nZ");
    let revert = text.apply(&transaction);
    assert_eq!(text.to_string(), "alpX\nY\nZmma\n");
    text.apply(&revert.into_transaction());
    assert_eq!(text.to_string(), original);
}

#[test]
fn insertions_and_deletions_of_uneven_length_invert_together() {
    let original = "aa bb cc dd";
    let mut text = Text::from_str(original);
    let transaction = Transaction::new(vec![
        Change::new(0, 2, "MUCH LONGER"),
        Change::new(3, 5, ""),
        Change::new(6, 8, "x"),
        Change::new(9, 11, "yy"),
    ]);
    let revert = text.apply(&transaction);
    text.apply(&revert.into_transaction());
    assert_eq!(text.to_string(), original);
}

#[test]
fn a_multi_range_edit_is_a_single_undo_step() {
    let mut buffer = buffer_of("a b c d e");
    let transaction = Transaction::new(
        [0usize, 2, 4, 6, 8]
            .into_iter()
            .map(|at| Change::new(at, at + 1, "Z"))
            .collect(),
    );
    let before = buffer.history_len();
    buffer.apply(&transaction);
    assert_eq!(buffer.to_string(), "Z Z Z Z Z");
    assert_eq!(
        buffer.history_len(),
        before + 1,
        "five ranges must add one history entry"
    );

    assert!(buffer.undo());
    assert_eq!(buffer.to_string(), "a b c d e");
    assert_eq!(
        buffer.history_len(),
        before,
        "one undo must consume the whole five-range edit"
    );
}

#[test]
fn redo_replays_a_multi_range_edit_as_one_step() {
    let mut buffer = buffer_of("a b c");
    buffer.apply(&Transaction::new(vec![
        Change::new(0, 1, "X"),
        Change::new(4, 5, "Y"),
    ]));
    buffer.undo();
    assert!(buffer.redo());
    assert_eq!(buffer.to_string(), "X b Y");
    assert!(!buffer.redo());
}

#[test]
fn undo_history_is_bounded_by_edit_size_not_document_size() {
    // A 10 MB document: 100k rows of 100 characters plus terminators.
    let row = "x".repeat(100);
    let document = std::iter::repeat_n(row.as_str(), 100_000)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        document.len() >= 10_000_000,
        "expected a 10 MB fixture, got {} bytes",
        document.len()
    );

    let mut buffer = buffer_of(&document);
    let baseline = buffer.history_footprint();

    // Replacements rather than insertions: the inverse of a replacement has to
    // retain the text it displaced, so this actually measures retained bytes.
    // An insertion's inverse is a deletion, which retains nothing and would
    // make this assertion pass for free.
    for index in 0..10_000 {
        buffer.apply(&Transaction::change(index, index + 1, "y"));
    }

    // Each edit retains exactly the one character it displaced, and the
    // history cap trims below even that. Snapshot undo would instead have
    // retained 10,000 copies of a 10 MB document.
    let footprint = buffer.history_footprint() - baseline;
    assert!(
        footprint <= 10_000,
        "history retains at most one character per edit, got {footprint}"
    );
    assert!(
        footprint * 1_000 < document.chars().count(),
        "history ({footprint} chars) must stay orders of magnitude below the document"
    );
}

#[test]
fn history_is_capped_so_long_sessions_stay_bounded() {
    let mut buffer = buffer_of("");
    for index in 0..2_000 {
        buffer.apply(&Transaction::insert(index, "x"));
    }
    assert!(
        buffer.history_len() <= 1_000,
        "history length {} exceeded the cap",
        buffer.history_len()
    );
}

#[test]
fn positions_round_trip_across_multibyte_and_empty_rows() {
    let text = Text::from_str("aβ🦀\n\nzz\n");
    for offset in 0..=text.len_chars() {
        let position = text.position_of(offset);
        assert_eq!(text.offset_of(position), offset, "offset {offset}");
    }
    assert_eq!(text.position_of(4), Position::new(1, 0));
}

#[test]
fn line_indexing_matches_split_on_newline() {
    for sample in ["", "a", "a\n", "a\nb", "a\nb\n", "\n\n"] {
        let text = Text::from_str(sample);
        let expected: Vec<&str> = sample.split('\n').collect();
        assert_eq!(text.len_lines(), expected.len(), "sample {sample:?}");
        for (row, line) in expected.iter().enumerate() {
            assert_eq!(text.line_string(row), *line, "sample {sample:?} row {row}");
        }
    }
}

#[test]
fn offsets_map_through_a_transaction_by_association() {
    let transaction = Transaction::new(vec![Change::new(2, 6, "ab"), Change::new(10, 10, "xyz")]);
    // Before every change: unchanged.
    assert_eq!(transaction.map_offset(0, Assoc::After), 0);
    // Inside a replaced region: collapses to one side.
    assert_eq!(transaction.map_offset(4, Assoc::Before), 2);
    assert_eq!(transaction.map_offset(4, Assoc::After), 4);
    // After the shrinking change, before the insertion.
    assert_eq!(transaction.map_offset(8, Assoc::After), 6);
    // After both.
    assert_eq!(transaction.map_offset(12, Assoc::After), 13);
}

#[test]
fn read_only_buffers_have_no_history_and_reject_transactions() {
    let mut buffer = Buffer::virtual_text("[projection]", "durable projection");
    assert!(!buffer.apply(&Transaction::insert(0, "x")));
    assert_eq!(buffer.to_string(), "durable projection");
    assert_eq!(buffer.history_len(), 0);
    assert!(!buffer.undo());
    assert!(!buffer.redo());
    assert!(!buffer.dirty);
}
