// SPDX-License-Identifier: MPL-2.0
use super::*;
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Style},
    widgets::Paragraph,
};

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
