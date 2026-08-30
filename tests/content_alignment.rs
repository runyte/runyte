// SPDX-License-Identifier: MPL-2.0

//! Content alignment: where a generated page's text is drawn, and what that
//! placement is not allowed to change about the buffer under it.

use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use runyte::{
    app::{App, PreparedView},
    command::{CommandExecutionContext, CommandInvocation, EditorCommand, HelpInvocation},
    config::Config,
    input::{Modifiers, PointerButton, PointerEvent, PointerEventKind},
    key_hints::KeyHintState,
    snapshot::SnapshotRow,
    ui,
};

const DESCRIPTION: &str = "A fast modal terminal editor with selection-first editing.";

fn run(app: &mut App, command: EditorCommand) {
    app.execute(CommandInvocation::editor(command, CommandExecutionContext::default()).unwrap())
        .unwrap();
}

fn prepare(app: &mut App, width: u16, height: u16) -> PreparedView {
    app.prepare_view(ui::frame_geometry(Rect::new(0, 0, width, height)))
}

/// The about page, opened in a pane that already has geometry, exactly as a
/// running editor opens it.
fn about(width: u16, height: u16) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    prepare(&mut app, width, height);
    run(&mut app, EditorCommand::ShowAbout);
    app
}

/// Every drawn cell of the screen, one string per row.
fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let prepared = app.prepare_view(ui::frame_geometry(frame.area()));
            let snapshot = app.snapshot(&prepared);
            ui::render_exact_colors_for_test(frame, app, &snapshot, &KeyHintState::default());
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|row| {
            (0..width)
                .map(|column| buffer[(column, row)].symbol())
                .collect()
        })
        .collect()
}

/// The rows of the pane body with the border and gutter stripped off.
fn body(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let pane = prepare(app, width, height).pane(0).unwrap().clone();
    screen(app, width, height)
        .into_iter()
        .skip(usize::from(pane.body.y))
        .take(usize::from(pane.body.height))
        .map(|row| {
            row.chars()
                .skip(usize::from(pane.body.x) + pane.gutter_width)
                .take(pane.content_indent + pane.text_width)
                .collect()
        })
        .collect()
}

/// The widest line of the page is drawn at the pane's own centre, and the
/// margin in front of it is drawn rather than stored.
#[test]
fn a_centered_page_is_drawn_at_the_middle_of_its_pane() {
    let mut app = about(120, 40);
    let pane = prepare(&mut app, 120, 40).pane(0).unwrap().clone();
    let drawn = body(&mut app, 120, 40);
    let description = drawn
        .iter()
        .find(|row| row.contains("fast modal"))
        .expect("the description is on screen");

    assert!(pane.content_indent > 0);
    assert_eq!(
        pane.content_indent,
        (pane.content_indent + pane.text_width - DESCRIPTION.chars().count()) / 2,
        "half the space the block leaves in the text column"
    );
    assert_eq!(
        description.trim_end(),
        format!("{}{DESCRIPTION}", " ".repeat(pane.content_indent)),
        "the sentence is drawn after the margin"
    );
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .lines()
            .any(|line| line == DESCRIPTION),
        "and carries no margin in the buffer"
    );
}

/// Resizing recomputes the margin from the live geometry. Nothing regenerates
/// the page: the text is the same at every width.
#[test]
fn resizing_the_pane_re_centres_the_page_without_rewriting_it() {
    let mut app = about(120, 40);
    let before = app.active_buffer().text().to_string();
    let wide = prepare(&mut app, 120, 40).pane(0).unwrap().content_indent;
    let narrow = prepare(&mut app, 90, 40).pane(0).unwrap().content_indent;
    let cramped = prepare(&mut app, 60, 40).pane(0).unwrap().content_indent;

    assert!(wide > narrow, "{wide} should exceed {narrow}");
    assert!(narrow > 0);
    assert_eq!(
        cramped, 0,
        "a block wider than the pane is shown from its first column"
    );
    assert_eq!(app.active_buffer().text().to_string(), before);
}

/// The alignment belongs to the buffer, so reopening the page onto the one
/// already there keeps it centred instead of quietly dropping to the left.
#[test]
fn reopening_a_centered_page_keeps_its_alignment() {
    let mut app = about(120, 40);
    let first = prepare(&mut app, 120, 40).pane(0).unwrap().content_indent;
    run(&mut app, EditorCommand::ShowAbout);

    assert_eq!(
        prepare(&mut app, 120, 40).pane(0).unwrap().content_indent,
        first
    );
}

/// A page short enough to fit is held down the pane, in blank space rather
/// than under a run of past-the-end markers.
#[test]
fn a_centered_page_that_fits_is_held_down_the_pane() {
    let mut app = about(120, 44);
    let prepared = prepare(&mut app, 120, 44);
    let height = prepared.pane(0).unwrap().body_height;
    let rows = app.snapshot(&prepared).pane(0).unwrap().rows.clone();

    let above = rows
        .iter()
        .take_while(|row| matches!(row, SnapshotRow::Padding))
        .count();
    let below = rows
        .iter()
        .rev()
        .take_while(|row| matches!(row, SnapshotRow::Padding))
        .count();
    let text = rows
        .iter()
        .filter(|row| matches!(row, SnapshotRow::Text(_)))
        .count();

    assert!(above > 0 && below > 0, "{above} above, {below} below");
    assert_eq!(above, (height - text) / 2);
    assert_eq!(above + text + below, height);
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, SnapshotRow::Placeholder)),
        "a centred page holds the whole pane open"
    );

    let drawn = body(&mut app, 120, 44);
    assert!(
        drawn[..above].iter().all(|row| row.trim().is_empty()),
        "padding is drawn as space, not as a marker: {:?}",
        &drawn[..above]
    );
}

/// There is nothing off-screen to reach, so scrolling a page that fits leaves
/// it where it is instead of dragging it out from under its own margin.
#[test]
fn scrolling_a_page_that_fits_leaves_it_centred() {
    let mut app = about(120, 44);
    let prepared = prepare(&mut app, 120, 44);
    let pane = prepared.pane(0).unwrap().clone();
    let before = app.snapshot(&prepared).pane(0).unwrap().rows.clone();

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::ScrollDown,
            column: pane.body.x,
            row: pane.body.y,
            modifiers: Modifiers::NONE,
        },
        &prepared,
    )
    .unwrap();

    let prepared = prepare(&mut app, 120, 44);
    assert_eq!(prepared.pane(0).unwrap().scroll_row, 0);
    assert_eq!(app.snapshot(&prepared).pane(0).unwrap().rows, before);
}

/// A page too tall to fit is not centred, and scrolls like any other
/// read-only buffer.
#[test]
fn a_centered_page_taller_than_the_pane_still_scrolls() {
    let mut app = about(120, 14);
    let prepared = prepare(&mut app, 120, 14);
    let pane = prepared.pane(0).unwrap().clone();
    assert!(
        !app.snapshot(&prepared)
            .pane(0)
            .unwrap()
            .rows
            .iter()
            .any(|row| matches!(row, SnapshotRow::Padding)),
        "no height to spare, so nothing is held open"
    );

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::ScrollDown,
            column: pane.body.x,
            row: pane.body.y,
            modifiers: Modifiers::NONE,
        },
        &prepared,
    )
    .unwrap();

    let scrolled = prepare(&mut app, 120, 14);
    assert!(
        scrolled.pane(0).unwrap().scroll_row > 0,
        "a page that does not fit scrolls"
    );
    assert!(
        scrolled.pane(0).unwrap().content_indent > 0,
        "and stays centred sideways"
    );
}

/// Alignment belongs to the view: two panes on the same page place it against
/// their own widths, and neither of them touches the text.
#[test]
fn two_panes_on_one_page_are_centred_independently() {
    let mut app = about(200, 40);
    run(&mut app, EditorCommand::SplitVertical);
    let prepared = prepare(&mut app, 200, 40);
    let panes = &prepared.panes;

    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].buffer_id, panes[1].buffer_id);
    for pane in panes {
        assert!(pane.content_indent > 0);
        assert_eq!(
            pane.gutter_width + pane.content_indent + pane.text_width,
            pane.body_width
        );
    }
}

/// Rows and columns mean what the generated text says, at every pane size, so
/// anything anchored to them survives a resize.
#[test]
fn alignment_never_moves_anything_in_the_buffer() {
    let mut wide = about(140, 40);
    let mut narrow = about(70, 40);
    let widely = prepare(&mut wide, 140, 40);
    let narrowly = prepare(&mut narrow, 70, 40);

    assert_ne!(
        widely.pane(0).unwrap().content_indent,
        narrowly.pane(0).unwrap().content_indent
    );
    assert_eq!(
        wide.active_buffer().text().to_string(),
        narrow.active_buffer().text().to_string()
    );

    let described = |app: &App| {
        let text = app.active_buffer().text().to_string();
        let row = text
            .lines()
            .position(|line| line == DESCRIPTION)
            .expect("the description is a line of its own");
        (row, app.active_buffer().line_string(row))
    };
    assert_eq!(described(&wide), described(&narrow));
}

/// A pointer names a column of the buffer, not a cell of the pane: a click is
/// translated back through the margin it was drawn with.
#[test]
fn clicking_centred_text_lands_on_the_character_under_the_pointer() {
    let mut app = about(120, 40);
    let prepared = prepare(&mut app, 120, 40);
    let pane = prepared.pane(0).unwrap().clone();
    let screen_row = app
        .snapshot(&prepared)
        .pane(0)
        .unwrap()
        .rows
        .iter()
        .position(|row| match row {
            SnapshotRow::Text(row) => row.runs.iter().any(|run| run.text.contains("fast modal")),
            _ => false,
        })
        .expect("the description is on screen");

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: pane.body.x + (pane.gutter_width + pane.content_indent) as u16 + 7,
            row: pane.body.y + screen_row as u16,
            modifiers: Modifiers::NONE,
        },
        &prepared,
    )
    .unwrap();

    let position = app.cursor_position();
    assert_eq!(position.col, 7, "the eighth character of the sentence");
    assert_eq!(app.active_buffer().line_string(position.row), DESCRIPTION);
}

/// A click in the margin names the first column of the row, the way a click on
/// the gutter already does, rather than a column the padding invented.
#[test]
fn clicking_the_margin_names_the_start_of_the_row() {
    let mut app = about(120, 40);
    let prepared = prepare(&mut app, 120, 40);
    let pane = prepared.pane(0).unwrap().clone();
    let screen_row = app
        .snapshot(&prepared)
        .pane(0)
        .unwrap()
        .rows
        .iter()
        .position(|row| match row {
            SnapshotRow::Text(row) => row.runs.iter().any(|run| run.text.contains("fast modal")),
            _ => false,
        })
        .unwrap();

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: pane.body.x + pane.gutter_width as u16 + 1,
            row: pane.body.y + screen_row as u16,
            modifiers: Modifiers::NONE,
        },
        &prepared,
    )
    .unwrap();

    assert_eq!(app.cursor_position().col, 0);
}

/// Nothing else is moved. An ordinary document and rendered help both read
/// from the left edge of their pane.
#[test]
fn only_a_page_that_asked_for_it_is_aligned() {
    let mut app = App::new(Config::default(), None).unwrap();
    assert_eq!(
        prepare(&mut app, 120, 40).pane(0).unwrap().content_indent,
        0
    );

    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    assert_eq!(
        prepare(&mut app, 120, 40).pane(0).unwrap().content_indent,
        0,
        "help is prose, and prose reads from the left"
    );
}
