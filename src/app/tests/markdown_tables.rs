// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::snapshot::{SnapshotRow, TextRole, TextRunKind};

const SOURCE: &str =
    "| Name | Description |\n| --- | --- |\n| A | **alpha beta gamma** |\n| B | delta |\n";

fn rendered(source: &str) -> App {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    config.editor.line_numbers = false;
    config.editor.scroll_offset = 0;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, source);
    press(&mut app, '?');
    app
}

fn prepare(app: &mut App, width: u16, height: u16) -> PreparedView {
    let area = Rect {
        x: 0,
        y: 0,
        width: width + 2,
        height: height + 2,
    };
    app.prepare_view(FrameGeometry {
        screen: area,
        editor: area,
        ..FrameGeometry::default()
    })
}

fn visible(app: &App, view: &PreparedView, pane: usize) -> Vec<String> {
    app.snapshot(view)
        .pane(pane)
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row {
            SnapshotRow::Text(row) => Some(
                row.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn markdown_tables_render_wrapped_cells_with_styles_and_stable_text() {
    let mut app = rendered(SOURCE);
    let text = text(&app);
    let revision = app.active_buffer().revision();
    let view = prepare(&mut app, 17, 20);
    let rows = visible(&app, &view, 0);
    assert!(
        rows.iter().any(|row| row == "A    │ alpha beta"),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|row| row == "     │  gamma    "),
        "{rows:?}"
    );
    let snapshot = app.snapshot(&view);
    assert!(snapshot.pane(0).unwrap().rows.iter().any(|row| match row {
        SnapshotRow::Text(row) => row.runs.iter().any(|run| run.text.contains("gamma") && matches!(run.kind, TextRunKind::Text { scope: Some(scope), .. } if scope.name() == "markup.bold")),
        _ => false,
    }));
    for width in [8, 40, 17] {
        prepare(&mut app, width, 20);
    }
    assert_eq!(app.active_buffer().revision(), revision);
    assert_eq!(app.active_buffer().to_string(), text);
    app.config.editor.soft_wrap = false;
    let view = prepare(&mut app, 80, 20);
    assert!(
        visible(&app, &view, 0)
            .iter()
            .any(|row| row == "A    │ alpha beta gamma")
    );
    press(&mut app, '?');
    assert_eq!(app.active_buffer().to_string(), SOURCE);
}

#[test]
fn markdown_tables_map_mouse_and_vertical_motion_through_cell_continuations() {
    let mut app = rendered(SOURCE);
    let view = prepare(&mut app, 17, 20);
    let pane = view.pane(0).unwrap();
    let row = pane
        .rows
        .iter()
        .position(|row| {
            row.document_row == Some(2) && row.segment.is_some_and(|span| span.table == Some(1))
        })
        .unwrap();
    let x = pane.body.x + 8;
    let y = pane.body.y + row as u16;
    let offset = app.pointer_text_offset(&view, 0, x, y).unwrap();
    assert_eq!(app.active_buffer().text().char_at(offset), Some('g'));
    assert!(
        app.pointer_text_offset(&view, 0, pane.body.x, y).is_none(),
        "empty continuation is not text"
    );
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: x,
            row: y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active().head(), offset);
    press(&mut app, 'j');
    assert_eq!(cursor(&app).row, 3, "down skips the decorative rule");
    press(&mut app, 'k');
    assert_eq!(cursor(&app).row, 2);
    assert_eq!(
        crate::wrap::line_segment_index(app.active_buffer(), 2, cursor(&app).col, 17, 4),
        1
    );
    set_cursor(&mut app, 2, 0);
    press(&mut app, 'j');
    assert_eq!(
        crate::wrap::line_segment_index(app.active_buffer(), 2, cursor(&app).col, 17, 4),
        1,
        "a short first cell cannot trap down movement"
    );
}

#[test]
fn markdown_tables_resize_selections_and_wrap_each_split_independently() {
    let mut app = rendered(SOURCE);
    let buffer = app.active().buffer;
    let content = text(&app);
    let start = content.find("gamma").unwrap();
    // The preceding text contains Unicode separators: selection uses chars.
    let start = content[..start].chars().count();
    app.active_mut().selection = Selection::single(Range::new(start, start + 4));
    app.mode = Mode::Select;
    let selection = app.active().selection.clone();
    app.panes.insert(1, Pane::new(buffer));
    app.layout = Layout::Split {
        axis: Axis::Horizontal,
        ratio: u16::MAX / 3,
        first: Box::new(Layout::Pane(0)),
        second: Box::new(Layout::Pane(1)),
    };
    let view = prepare(&mut app, 60, 20);
    assert_ne!(
        view.pane(0).unwrap().wrap_width,
        view.pane(1).unwrap().wrap_width
    );
    let first = visible(&app, &view, 0);
    let second = visible(&app, &view, 1);
    assert_ne!(first, second);
    assert!(
        second.iter().any(|row| row.contains("alpha beta gamma")),
        "{second:?}"
    );
    let snapshot = app.snapshot(&view);
    assert!(snapshot.pane(0).unwrap().rows.iter().any(|row| match row {
        SnapshotRow::Text(row) => row.runs.iter().any(|run| matches!(
            run.kind,
            TextRunKind::Text {
                role: TextRole::PrimarySelected,
                ..
            }
        ) && run.text.contains("gamm")),
        _ => false,
    }));
    prepare(&mut app, 90, 20);
    assert_eq!(app.active().selection, selection);
    assert_eq!(text(&app), content);
}

#[test]
fn markdown_tables_search_yank_and_jump_labels_use_original_cell_offsets() {
    let mut app = rendered(SOURCE);
    prepare(&mut app, 17, 20);
    press(&mut app, 's');
    for ch in "beta gamma".chars() {
        press(&mut app, ch);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let view = prepare(&mut app, 17, 20);
    assert_eq!(app.yank_value(false).text, "beta gamma");
    let snapshot = app.snapshot(&view);
    let selected: String = snapshot
        .pane(0)
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row {
            SnapshotRow::Text(row) => Some(row),
            _ => None,
        })
        .flat_map(|row| row.runs.iter())
        .filter(|run| {
            matches!(
                run.kind,
                TextRunKind::Text {
                    role: TextRole::Selected | TextRole::PrimarySelected | TextRole::PrimaryCaret,
                    ..
                }
            )
        })
        .map(|run| run.text.as_str())
        .collect();
    assert!(selected.contains("beta"), "{selected:?}");
    assert!(selected.contains("gamma"), "{selected:?}");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.label_visible_words();
    let buffer = app.active_buffer();
    let gamma = buffer.to_string().find("gamma").unwrap();
    let gamma = buffer.to_string()[..gamma].chars().count();
    assert!(app.jump.as_ref().unwrap().label_at(gamma).is_some());
    let snapshot = app.snapshot(&view);
    let row = snapshot
        .pane(0)
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row {
            SnapshotRow::Text(row) => Some(row),
            _ => None,
        })
        .find(|row| row.document_row == 2 && row.continuation)
        .unwrap();
    assert!(
        row.runs
            .iter()
            .any(|run| matches!(run.kind, TextRunKind::JumpLabel(_)))
    );
}

#[test]
fn markdown_tables_overflow_scrolls_horizontally_without_unwrapping_prose() {
    let mut app = rendered(
        "| A | B | C |\n| --- | --- | --- |\n| alpha | beta | gamma |\n\nordinary prose that wraps\n",
    );
    let view = prepare(&mut app, 8, 30);
    let pane = view.pane(0).unwrap();
    let x = pane.body.x;
    let y = pane.body.y;
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::ScrollRight,
            column: x,
            row: y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    let view = prepare(&mut app, 8, 30);
    assert_eq!(view.pane(0).unwrap().scroll_col, 3);
    let rows = visible(&app, &view, 0);
    assert!(rows.iter().any(|row| row == "h │ beta"), "{rows:?}");
    assert!(rows.iter().any(|row| row == "ordinary"), "{rows:?}");
    // Moving into the last column brings that cell back into view.
    let text = text(&app);
    let gamma = text[..text.find("gamma").unwrap()].chars().count();
    app.active_mut().replace_selection(Selection::point(gamma));
    press(&mut app, 'l');
    let view = prepare(&mut app, 8, 30);
    assert!(view.pane(0).unwrap().scroll_col > 3);
    let pane = view.pane(0).unwrap();
    let row = pane
        .rows
        .iter()
        .position(|row| row.document_row == Some(2) && !row.continuation)
        .unwrap();
    let layout = crate::wrap::table_layout(app.active_buffer(), 2, 8, 4).unwrap();
    let screen_x = layout
        .position(app.active_buffer().position_of(gamma).col)
        .1
        - pane.scroll_col;
    assert_eq!(
        app.pointer_text_offset(
            &view,
            0,
            pane.body.x + screen_x as u16,
            pane.body.y + row as u16
        ),
        Some(gamma)
    );
}

#[test]
fn markdown_tables_re_render_replaces_structure_and_rules_are_not_mouse_targets() {
    let mut app = rendered(SOURCE);
    let page = app.active().buffer;
    let view = prepare(&mut app, 17, 20);
    let pane = view.pane(0).unwrap();
    let rule = pane
        .rows
        .iter()
        .position(|row| {
            row.document_row == Some(2) && row.segment.is_some_and(|span| span.table == Some(2))
        })
        .unwrap();
    let before = app.active().head();
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: pane.body.x + 4,
            row: pane.body.y + rule as u16,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active().head(), before);
    press(&mut app, '?');
    let length = app.active_buffer().len_chars();
    app.apply_to_buffer(
        app.active().buffer,
        &Transaction::change(0, length, "plain prose"),
    );
    press(&mut app, '?');
    assert_eq!(app.active().buffer, page);
    let view = prepare(&mut app, 17, 20);
    assert!(
        view.pane(0)
            .unwrap()
            .rows
            .iter()
            .all(|row| row.segment.is_none_or(|span| span.table.is_none()))
    );
    assert_eq!(text(&app), "plain prose\n");
}

#[test]
fn markdown_tables_terminal_frame_keeps_cell_borders_aligned() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = rendered(SOURCE);
    let view = prepare(&mut app, 17, 20);
    let snapshot = app.snapshot(&view);
    let hints = crate::key_hints::KeyHintState::default();
    let mut terminal = Terminal::new(TestBackend::new(19, 22)).unwrap();
    terminal
        .draw(|frame| crate::ui::render_exact_colors_for_test(frame, &app, &snapshot, &hints))
        .unwrap();
    let pane = view.pane(0).unwrap();
    for (index, row) in visible(&app, &view, 0).iter().enumerate() {
        if row.contains('│') {
            let cell = &terminal.backend().buffer()[(pane.body.x + 5, pane.body.y + index as u16)];
            assert_eq!(cell.symbol(), "│");
        }
    }
}

#[test]
fn markdown_tables_page_motion_counts_rules_and_window_motion_skips_them() {
    let mut app = rendered(
        "| A | B |\n| --- | --- |\n| one | first |\n| two | second |\n| three | third |\n| four | fourth |\n",
    );
    prepare(&mut app, 30, 4);
    set_cursor(&mut app, 2, 0);
    app.motion(Motion::PageDown);
    assert_eq!(cursor(&app).row, 4);
    app.motion(Motion::PageUp);
    assert_eq!(cursor(&app).row, 2);
    app.active_mut().scroll_row = 0;
    app.active_mut().scroll_wrap = 0;
    app.active_mut().preserve_scroll = true;
    prepare(&mut app, 30, 4);
    app.motion(Motion::WindowBottom);
    assert_eq!(cursor(&app).row, 2);
}
