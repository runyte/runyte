// SPDX-License-Identifier: MPL-2.0
use super::*;
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Style},
    widgets::Paragraph,
};

/// Component diagnostic only: excludes Ratatui rendering/diff generation and
/// native preparation/painting. Retains the prior snapshot across each draw,
/// as the frontend does, and includes releasing it after publishing the next.
/// Run in release mode with --ignored --nocapture and without paint tracing.
#[test]
#[ignore = "opt-in release grid COW component measurement"]
fn retained_snapshot_cow_draw_measurement() {
    use std::{hint::black_box, time::Instant};

    if cfg!(debug_assertions) {
        panic!("run this benchmark with --release");
    }
    assert!(std::env::var_os("RUNYTE_NATIVE_PAINT_TIMING").is_none());
    const SAMPLES: usize = 15;
    const ITERATIONS: usize = 200;
    for (width, height) in [(120u16, 40u16), (240, 80)] {
        for changed_rows in [1u16, 30] {
            for cells_per_row in [1u16, width] {
                let mut backend = GridBackend::new(width, height);
                let mut previous = backend.snapshot();
                let mut symbols = [Cell::default(), Cell::default()];
                symbols[0].set_symbol("a");
                symbols[1].set_symbol("b");
                let mut samples = Vec::new();
                // One discarded warm-up batch uses the same retained snapshots.
                for sample in 0..=SAMPLES {
                    let started = Instant::now();
                    for iteration in 0..ITERATIONS {
                        let symbol = &symbols[iteration % 2];
                        backend
                            .draw(
                                (0..changed_rows)
                                    .flat_map(|y| (0..cells_per_row).map(move |x| (x, y, symbol))),
                            )
                            .unwrap();
                        black_box(&previous);
                        previous = black_box(backend.snapshot());
                    }
                    let us = started.elapsed().as_secs_f64() * 1_000_000.0 / ITERATIONS as f64;
                    if sample != 0 {
                        samples.push(us);
                    }
                }
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "grid-cow cols={width} rows={height} changed_rows={changed_rows} \
                     cells_per_row={cells_per_row} samples={SAMPLES} iterations={ITERATIONS} \
                     median_us={:.3} min_us={:.3} max_us={:.3}",
                    samples[SAMPLES / 2],
                    samples[0],
                    samples[SAMPLES - 1]
                );
                assert_eq!(previous[(0, 0)].symbol(), "b");
            }
        }
    }
}

#[test]
fn changed_rows_are_copied_and_skipped_frames_remain_complete() {
    let mut terminal = Terminal::new(GridBackend::new(8, 3)).unwrap();
    terminal
        .draw(|frame| {
            frame.buffer_mut()[(1, 0)].set_symbol("a");
        })
        .unwrap();
    let first = terminal.backend().snapshot();
    terminal
        .draw(|frame| {
            frame.buffer_mut()[(1, 0)].set_symbol("b");
        })
        .unwrap();
    let skipped = terminal.backend().snapshot();
    assert!(!Arc::ptr_eq(&first.rows[0], &skipped.rows[0]));
    assert!(Arc::ptr_eq(&first.rows[1], &skipped.rows[1]));
    assert!(Arc::ptr_eq(&first.rows[2], &skipped.rows[2]));
    terminal
        .draw(|frame| {
            frame.buffer_mut()[(1, 0)].set_symbol("b");
            frame.buffer_mut()[(2, 1)].set_symbol("c");
        })
        .unwrap();
    let latest = terminal.backend().snapshot();
    assert_eq!(first[(1, 0)].symbol(), "a");
    assert_eq!(latest[(1, 0)].symbol(), "b");
    assert_eq!(latest[(2, 1)].symbol(), "c");
    assert!(Arc::ptr_eq(&skipped.rows[0], &latest.rows[0]));
    assert!(!Arc::ptr_eq(&skipped.rows[1], &latest.rows[1]));
    terminal
        .draw(|frame| {
            frame.buffer_mut()[(1, 0)].set_symbol("b");
            frame.buffer_mut()[(2, 1)].set_symbol("c");
        })
        .unwrap();
    let unchanged = terminal.backend().snapshot();
    assert!(
        latest
            .rows
            .iter()
            .zip(&unchanged.rows)
            .all(|(a, b)| Arc::ptr_eq(a, b))
    );
    terminal.backend_mut().resize(4, 2);
    terminal.draw(|_| {}).unwrap();
    assert_eq!(terminal.backend().snapshot().area, Rect::new(0, 0, 4, 2));
    assert_eq!(first.area, Rect::new(0, 0, 8, 3));
    assert_eq!(latest[(2, 1)].symbol(), "c");
}

#[test]
fn grid_backend_matches_ratatui_for_wide_cells_styles_clear_and_cursor() {
    let mut shared = Terminal::new(GridBackend::new(12, 4)).unwrap();
    let mut reference = Terminal::new(TestBackend::new(12, 4)).unwrap();
    for text in ["界界 a\ne\u{301} 😀", "a\nchanged", "", "last"] {
        let render = |frame: &mut ratatui::Frame<'_>| {
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(Color::Red)),
                frame.area(),
            );
            frame.set_cursor_position((2, 1));
        };
        shared.draw(render).unwrap();
        reference.draw(render).unwrap();
        let grid = shared.backend().snapshot();
        for y in 0..4 {
            for x in 0..12 {
                assert_eq!(grid[(x, y)], reference.backend().buffer()[(x, y)]);
            }
        }
        assert!(shared.backend().cursor_visible());
        assert_eq!(shared.backend().cursor_position(), Position::new(2, 1));
    }
    for region in [
        ClearType::AfterCursor,
        ClearType::BeforeCursor,
        ClearType::CurrentLine,
        ClearType::UntilNewLine,
        ClearType::All,
    ] {
        shared.backend_mut().clear_region(region).unwrap();
        reference.backend_mut().clear_region(region).unwrap();
        let grid = shared.backend().snapshot();
        for y in 0..4 {
            for x in 0..12 {
                assert_eq!(grid[(x, y)], reference.backend().buffer()[(x, y)]);
            }
        }
    }
    shared.draw(|_| {}).unwrap();
    assert!(!shared.backend().cursor_visible());
}
