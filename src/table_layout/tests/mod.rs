// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{markdown, text::Transaction};

fn page(source: &str) -> Buffer {
    let rendered = markdown::render(source);
    let mut buffer = Buffer::scratch();
    buffer.apply(&Transaction::insert(0, rendered.text()));
    buffer.wrap_cache.tables = Tables::new(buffer.revision(), rendered.tables);
    buffer
}

fn lines(buffer: &Buffer, row: usize, width: usize) -> Vec<String> {
    let layout = wrap::table_layout(buffer, row, width, 4).unwrap();
    (0..layout.segments.len())
        .map(|index| {
            layout
                .visible(buffer, row, index, 0, width)
                .iter()
                .map(|atom| atom.ch)
                .collect()
        })
        .collect()
}

#[test]
fn table_layout_wraps_cells_independently_and_preserves_row_boundaries() {
    let buffer =
        page("| Name | Description |\n| --- | --- |\n| A | alpha beta gamma |\n| B | delta |\n");
    assert_eq!(
        lines(&buffer, 0, 17),
        ["Name │ Descriptio", "     │ n         "]
    );
    assert_eq!(lines(&buffer, 1, 17), ["─────┼───────────"]);
    assert_eq!(
        lines(&buffer, 2, 17),
        [
            "A    │ alpha beta",
            "     │  gamma    ",
            "─────┼───────────"
        ]
    );
    assert_eq!(lines(&buffer, 3, 17), ["B    │ delta     "]);
    assert!(buffer.to_string().contains("A    │ alpha beta gamma"));
}

#[test]
fn table_layout_maps_unicode_and_tabs_without_selecting_padding() {
    let buffer = page("| A | B |\n| --- | --- |\n| 界界界 | e\u{301}\txxxx |\n| | last |\n");
    for width in [1, 8, 13, 23, 80] {
        let layout = wrap::table_layout(&buffer, 2, width, 4).unwrap();
        for row in 0..layout.height {
            for atom in layout.visible(&buffer, 2, row, 0, layout.width) {
                if let Some(column) = atom.offset
                    && atom.width > 0
                {
                    let mapped = layout.column(row, atom.x, true).unwrap();
                    // A combining mark shares the preceding glyph's cell.
                    assert_eq!(
                        buffer.text().line(2).char(mapped),
                        buffer.text().line(2).char(column)
                    );
                    assert_eq!(layout.position(column), (row, atom.x));
                }
            }
        }
        assert_eq!(layout.column(layout.height, 0, false), None);
        assert_eq!(layout.column(0, layout.cells[1].x - 2, true), None);
    }
}

#[test]
fn table_layout_keeps_short_columns_and_overflows_only_below_minimum_width() {
    assert_eq!(column_widths(&[2, 50, 50], 30), [2, 11, 11]);
    assert_eq!(column_widths(&[2, 50, 50], 5), [2, 4, 4]);
    assert_eq!(column_widths(&[2, 3], 80), [2, 3]);
    assert_eq!(column_widths(&[0, 0], 1), [1, 1]);
    let buffer = page("| A | B | C |\n| --- | --- | --- |\n| alpha | beta | gamma |\n");
    let layout = wrap::table_layout(&buffer, 2, 5, 4).unwrap();
    let all = layout.visible(&buffer, 2, 0, 0, layout.width);
    let scrolled = layout.visible(&buffer, 2, 0, 7, 5);
    assert_eq!(
        scrolled.iter().map(|atom| atom.ch).collect::<String>(),
        all.iter()
            .filter(|atom| atom.x >= 7 && atom.x < 12)
            .map(|atom| atom.ch)
            .collect::<String>()
    );
}

#[test]
fn table_layout_reuses_widths_and_invalidates_metadata_after_replacement() {
    let mut buffer = page("| A | B |\n| --- | --- |\n| first | alpha beta gamma |\n");
    let original = wrap::table_layout(&buffer, 2, 15, 4).unwrap();
    assert!(Arc::ptr_eq(
        &original,
        &wrap::table_layout(&buffer, 2, 15, 4).unwrap()
    ));
    let wide = wrap::table_layout(&buffer, 2, 80, 4).unwrap();
    assert!(original.height > wide.height);
    let cloned = buffer.clone();
    assert_eq!(lines(&cloned, 2, 15), lines(&buffer, 2, 15));
    assert!(!Arc::ptr_eq(
        &original,
        &wrap::table_layout(&cloned, 2, 15, 4).unwrap()
    ));
    for width in 20..40 {
        wrap::table_layout(&buffer, 2, width, 4).unwrap();
    }
    assert!(!Arc::ptr_eq(
        &original,
        &wrap::table_layout(&buffer, 2, 15, 4).unwrap()
    ));
    buffer.apply(&Transaction::insert(0, "changed\n"));
    assert!(wrap::table_layout(&buffer, 2, 15, 4).is_none());
}

#[test]
fn table_layout_bounds_deep_visible_text_and_retained_geometry() {
    let buffer = page(&format!(
        "| A | B |\n| --- | --- |\n| x | {} |\n",
        "abcdef ".repeat(30_000)
    ));
    let layout = wrap::table_layout(&buffer, 2, 24, 4).unwrap();
    let index = layout.height - 2;
    let atoms = layout.visible(&buffer, 2, index, 0, 24);
    assert!(atoms.len() <= 24);
    assert!(atoms.iter().any(|atom| atom.ch == 'a'));
    assert!(buffer.wrap_cache.tables.cache.lock().unwrap().is_empty());
}

#[test]
fn table_layout_preserves_combining_marks_at_the_column_edge_and_clipped_tabs() {
    let buffer = page("| A | B |\n| --- | --- |\n| e\u{301}e\u{301}e\u{301}e\u{301} | a\tb |\n");
    let layout = wrap::table_layout(&buffer, 2, 8, 4).unwrap();
    let atoms = layout.visible(&buffer, 2, 0, 0, layout.width);
    assert_eq!(atoms.iter().filter(|atom| atom.ch == '\u{301}').count(), 4);
    let layout = wrap::table_layout(&buffer, 2, 80, 4).unwrap();
    let scroll = layout.cells[1].x + 2;
    let atoms = layout.visible(&buffer, 2, 0, scroll, 2);
    assert_eq!(atoms[0].ch, '\t');
    assert_eq!(atoms[0].x, 0);
    assert_eq!(atoms[0].width, 2);
}
