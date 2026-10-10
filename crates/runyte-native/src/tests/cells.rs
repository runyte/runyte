// SPDX-License-Identifier: MPL-2.0
use super::{FrameData, GlyphCache, Modifier, color};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use std::sync::Arc;

fn frame() -> FrameData {
    FrameData {
        attachment: 0,
        background: 0x181818,
        foreground: 0xdddddd,
        id: None,
        cells: Buffer::empty(Rect::new(0, 0, 8, 2)).into(),
        previews: Vec::new(),
        media: vec![],
        cursor: None,
        overlays: vec![],
        media_input: false,
        routing_serial: 0,
        metadata_paths: vec![],
    }
}

#[test]
fn unchanged_rows_reuse_prepared_styles_and_changed_rows_do_not() {
    let mut frame = frame();
    frame.cells[(0, 0)].set_symbol("a");
    frame.cells[(0, 1)].set_symbol("b");
    let mut cache = GlyphCache::default();
    cache.prepare(&frame);
    let first = cache.rows[0].as_ref().unwrap().source.clone();
    let second_cells = cache.rows[1].as_ref().unwrap().cells.as_ptr();
    let backgrounds = cache.rows[1].as_ref().unwrap().backgrounds.as_ptr();
    frame.cells[(0, 0)].set_fg(Color::Red);
    cache.prepare(&frame);
    assert!(!Arc::ptr_eq(
        &first,
        &cache.rows[0].as_ref().unwrap().source
    ));
    assert_eq!(
        cache.rows[0].as_ref().unwrap().cells[0].fg,
        color(Color::Red, 0)
    );
    assert_eq!(second_cells, cache.rows[1].as_ref().unwrap().cells.as_ptr());
    assert_eq!(
        backgrounds,
        cache.rows[1].as_ref().unwrap().backgrounds.as_ptr()
    );
    // Preparing row styles does not retain any shaped glyphs outside FaceCache.
    assert!(
        cache
            .faces
            .iter()
            .all(|face| face.entries.is_empty() && face.ascii.iter().all(Option::is_none))
    );
}

#[test]
fn prepared_rows_refresh_theme_and_media_coverage_without_changed_cells() {
    let mut frame = frame();
    frame.cells[(0, 0)].set_symbol("a");
    let source = frame.cells.rows[0].clone();
    let mut cache = GlyphCache::default();
    cache.prepare(&frame);
    frame.foreground = 0x112233;
    frame.background = 0x445566;
    cache.prepare(&frame);
    let row = cache.rows[0].as_ref().unwrap();
    assert!(Arc::ptr_eq(&source, &row.source));
    assert_eq!(row.cells[0].fg, color(Color::Reset, 0x112233));
    assert_eq!(row.backgrounds[0].2, color(Color::Reset, 0x445566));
    frame.media.push(super::super::MediaPane {
        pane: 0,
        path: "image.png".into(),
        page: 1,
        body: runyte::layout::Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 1,
        },
    });
    cache.prepare(&frame);
    assert!(cache.rows[0].as_ref().unwrap().cells.is_empty());
    assert!(cache.rows[0].as_ref().unwrap().backgrounds.is_empty());
    frame.overlays.push(runyte::layout::Rect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    });
    cache.prepare(&frame);
    assert_eq!(cache.rows[0].as_ref().unwrap().cells.len(), 1);
    assert_eq!(cache.rows[0].as_ref().unwrap().backgrounds.len(), 1);
    frame.media.clear();
    frame.overlays.clear();
    cache.prepare(&frame);
    assert_eq!(cache.rows[0].as_ref().unwrap().backgrounds[0].1, 8);
}

#[test]
fn prepared_wide_and_hidden_cells_preserve_logical_advance_and_styles() {
    let mut frame = frame();
    frame.cells[(1, 0)].set_symbol("界");
    frame.cells[(2, 0)].set_symbol("skip");
    frame.cells[(3, 0)]
        .set_symbol("e\u{301}")
        .set_fg(Color::Red)
        .set_bg(Color::Blue)
        .set_style(Modifier::REVERSED | Modifier::DIM);
    frame.cells[(4, 0)]
        .set_symbol("h")
        .set_style(Modifier::HIDDEN);
    let mut cache = GlyphCache::default();
    cache.prepare(&frame);
    let row = cache.rows[0].as_ref().unwrap();
    assert_eq!(
        row.cells
            .iter()
            .map(|cell| (cell.x, cell.width))
            .collect::<Vec<_>>(),
        [(1, 2), (3, 1)]
    );
    let mut expected = color(Color::Blue, 0);
    expected.l *= 0.65;
    assert_eq!(row.cells[1].fg, expected);
}

#[test]
fn document_preview_hides_source_cells_but_preserves_overlay_cells() {
    let mut frame = frame();
    frame.previews.push(super::super::preview::Pane {
        pane: 1,
        active: true,
        body: runyte::layout::Rect {
            x: 1,
            y: 0,
            width: 6,
            height: 2,
        },
        document: runyte::document_preview::DocumentPreview {
            generation: 1,
            source: 0,
            text: "hello".into(),
            language: "text".into(),
            path: None,
            selection_only: false,
        },
    });
    frame.overlays.push(runyte::layout::Rect {
        x: 3,
        y: 0,
        width: 2,
        height: 1,
    });
    let covered = super::Coverage::row(&frame, 0);
    assert!(covered.hides(1));
    assert!(!covered.hides(3));
    assert!(!covered.hides(0));
    assert!(frame.under_media(1, 0));
    assert!(!frame.under_media(3, 0));
}
