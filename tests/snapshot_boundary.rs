// SPDX-License-Identifier: MPL-2.0

#[test]
fn normal_editor_snapshot_has_no_frontend_or_raw_service_types() {
    let source = include_str!("../src/snapshot.rs");
    for forbidden in ["ratatui", "crossterm", "tree_house", "lsp_types"] {
        assert!(
            !source.contains(forbidden),
            "snapshot boundary names forbidden dependency {forbidden}"
        );
    }
}

#[test]
fn normal_pane_and_status_rendering_do_not_reach_back_into_app_state() {
    let source = include_str!("../src/ui.rs");
    let start = source.find("fn draw_pane(").unwrap();
    let end = source.find("fn draw_picker(").unwrap();
    let normal_surface = &source[start..end];

    for forbidden in [
        "app.buffers",
        "app.panes",
        "app.diagnostics",
        "app.status",
        "app.command",
        "app.jump",
    ] {
        assert!(
            !normal_surface.contains(forbidden),
            "normal renderer reads {forbidden} instead of its snapshot"
        );
    }
}

// Every overlay that owns a typed query keeps that query on the first line
// under its title, present and legible before anything has been typed. The
// line used to appear only once it had text, which moved every row under the
// reader's cursor as the first character arrived, and the filterable result
// lists kept their filter among the action hints in the title instead.

use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use runyte::{
    app::App,
    config::Config,
    input::{KeyCode, KeyStroke, Modifiers},
    key_hints::KeyHintState,
    picker::{ListPicker, PickerItem},
    snapshot::{OverlayInput, OverlayKind},
    ui,
    workspace::WorkspaceHost,
};

fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|row| {
            (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn standalone(app: &mut App) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal
        .draw(|frame| {
            let prepared = app.prepare_view(ui::frame_geometry(frame.area()));
            let snapshot = app.snapshot(&prepared);
            ui::render_exact_colors_for_test(frame, app, &snapshot, &KeyHintState::default());
        })
        .unwrap();
    lines(terminal.backend().buffer())
}

fn attached(host: &mut WorkspaceHost) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal
        .draw(|frame| {
            let published = host.prepare_frame(ui::frame_geometry(frame.area()));
            ui::render_host_frame_exact_colors_for_test(frame, &published);
        })
        .unwrap();
    lines(terminal.backend().buffer())
}

/// The screen row a row of the overlay stands on.
fn row_of(screen: &[String], needle: &str) -> usize {
    screen
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("missing {needle:?} in\n{}", screen.join("\n")))
}

fn press(app: &mut App, character: char) {
    app.handle_key(KeyStroke::new(KeyCode::Char(character), Modifiers::NONE))
        .unwrap();
}

fn list_editor() -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.list = Some(ListPicker::new(
        "Commits",
        vec![
            PickerItem::new("alpha", "the first", 0),
            PickerItem::new("beta", "the second", 1),
        ],
    ));
    app
}

#[test]
fn a_filterable_result_list_keeps_its_filter_on_a_query_line_that_is_always_there() {
    let mut app = list_editor();
    let empty = standalone(&mut app);

    assert!(
        empty.iter().any(|line| line.contains("> type to filter")),
        "the empty query line invites the filter it owns:\n{}",
        empty.join("\n")
    );
    let title = row_of(&empty, "Commits");
    assert!(
        !empty[title].contains("type to filter") && !empty[title].contains("filter:"),
        "and the title keeps the surface name and its keys: {}",
        empty[title]
    );
    let first = row_of(&empty, "alpha");

    press(&mut app, 'a');
    let typed = standalone(&mut app);
    assert!(typed.iter().any(|line| line.contains("> a")));
    assert_eq!(
        row_of(&typed, "alpha"),
        first,
        "the rows do not move as the query gains its first character"
    );
}

#[test]
fn the_two_renderers_agree_about_a_result_list_query_line() {
    let mut app = list_editor();
    let drawn = standalone(&mut app);
    let mut host = WorkspaceHost::new(list_editor());
    let published = attached(&mut host);

    assert_eq!(
        row_of(&drawn, "> type to filter"),
        row_of(&published, "> type to filter"),
        "one query line on one row:\nstandalone:\n{}\nattached:\n{}",
        drawn.join("\n"),
        published.join("\n")
    );
    assert_eq!(
        row_of(&drawn, "alpha"),
        row_of(&published, "alpha"),
        "and the rows start immediately under it in both"
    );
}

#[test]
fn the_finder_keeps_its_query_line_before_and_after_its_first_character() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, '/');
    let empty = standalone(&mut app);
    assert!(
        empty.iter().any(|line| line.contains("> type to find")),
        "the finder invites its query while it is empty:\n{}",
        empty.join("\n")
    );

    let query = row_of(&empty, "> type to find");
    let first = row_of(&empty, "\u{25b8} ");
    assert_eq!(first, query + 1, "the rows begin under the query line");

    press(&mut app, 'a');
    let typed = standalone(&mut app);
    assert_eq!(
        (row_of(&typed, "> a"), row_of(&typed, "\u{25b8} ")),
        (query, first),
        "and neither moves as the query gains its first character"
    );

    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == OverlayKind::FilePicker)
        .expect("the finder publishes its own overlay");
    assert_eq!(overlay.input, OverlayInput::Filter);
    assert!(
        !overlay.query_placeholder.is_empty(),
        "the snapshot says the surface owns input and what its line reads while empty"
    );
}

#[test]
fn a_completing_prompt_publishes_no_query_of_its_own() {
    let mut app = App::new(Config::default(), None).unwrap();
    press(&mut app, ':');
    for character in "open ".chars() {
        press(&mut app, character);
    }

    let overlays = app.overlay_snapshots();
    let assistance = overlays
        .iter()
        .find(|overlay| overlay.kind == OverlayKind::PathCompletion)
        .expect("a path argument opens its assistance");
    assert_eq!(
        assistance.input,
        OverlayInput::None,
        "the interaction line owns the typed value"
    );
    assert!(
        assistance.query.is_empty(),
        "so the overlay does not carry it a second time: {:?}",
        assistance.query
    );
    assert!(
        overlays
            .iter()
            .all(|overlay| overlay.kind != OverlayKind::CommandPalette),
        "and the palette itself is not published while a path argument is being completed"
    );
}
