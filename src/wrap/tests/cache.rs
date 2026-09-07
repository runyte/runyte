// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::text::Transaction;

#[test]
fn cached_geometry_preserves_unicode_tabs_and_word_boundaries() {
    let mut buffer = Buffer::scratch();
    buffer.apply(&Transaction::insert(
        0,
        "ab界 e\u{301}\tlongword xyz\r\n\nlast",
    ));
    for row in 0..buffer.len_lines() {
        let line = buffer.line_string(row);
        for width in [1, 4, 9, 80] {
            for tab_width in [1, 4, 8] {
                let spans = line_segments(&buffer, row, width, tab_width);
                assert_eq!(&*spans, segments(&line, width, tab_width));
                for column in 0..=buffer.line_len(row) {
                    assert_eq!(
                        line_screen_column(&buffer, row, column, width, tab_width),
                        screen_column(&line, column, width, tab_width)
                    );
                }
                for index in 0..spans.len() {
                    for desired in 0..=width {
                        assert_eq!(
                            line_column_for_screen(&buffer, row, index, desired, width, tab_width),
                            column_for_screen(&line, index, desired, width, tab_width)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn geometry_is_reused_across_panes_and_invalidated_by_edit_undo_and_redo() {
    let mut buffer = Buffer::scratch();
    buffer.apply(&Transaction::insert(0, "alpha beta gamma\nsecond line"));
    let original = line_segments(&buffer, 0, 8, 4);
    assert!(Arc::ptr_eq(&original, &line_segments(&buffer, 0, 8, 4)));
    let narrow = line_segments(&buffer, 0, 4, 4);
    assert_ne!(original, narrow);
    assert!(Arc::ptr_eq(&original, &line_segments(&buffer, 0, 8, 4)));

    // Moving line identities must invalidate row-keyed entries too.
    buffer.apply(&Transaction::insert(0, "new\n"));
    assert_eq!(&*line_segments(&buffer, 0, 8, 4), segments("new", 8, 4));
    assert_eq!(line_segments(&buffer, 1, 8, 4), original);
    assert!(buffer.undo());
    assert_eq!(line_segments(&buffer, 0, 8, 4), original);
    assert!(buffer.redo());
    assert_eq!(&*line_segments(&buffer, 0, 8, 4), segments("new", 8, 4));

    let mut copy = buffer.clone();
    copy.apply(&Transaction::insert(0, "different "));
    assert_ne!(
        line_segments(&copy, 0, 8, 4),
        line_segments(&buffer, 0, 8, 4)
    );
}

#[test]
fn cache_eviction_and_oversized_geometry_preserve_results() {
    let mut buffer = Buffer::scratch();
    buffer.apply(&Transaction::insert(0, "abcdefghij\n".repeat(20)));
    let original = line_segments(&buffer, 0, 3, 4);
    for row in 1..20 {
        line_segments(&buffer, row, 3, 4);
    }
    let rebuilt = line_segments(&buffer, 0, 3, 4);
    assert_eq!(original, rebuilt);
    assert!(!Arc::ptr_eq(&original, &rebuilt));

    buffer.apply(&Transaction::insert(0, "x".repeat(262_145)));
    let oversized = line_segments(&buffer, 0, 1, 4);
    assert_eq!(oversized.len(), buffer.line_len(0));
    assert!(buffer.wrap_cache.0.lock().unwrap().is_empty());
    // Two individually cacheable layouts must also respect the total cap.
    line_segments(&buffer, 0, 2, 4);
    line_segments(&buffer, 0, 2, 8);
    assert_eq!(buffer.wrap_cache.0.lock().unwrap().len(), 1);
}
