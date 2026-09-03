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

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

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

static NEXT_PROJECT: AtomicU64 = AtomicU64::new(0);

/// A throwaway project directory under the system temporary directory.
struct TempProject(PathBuf);

impl TempProject {
    fn new(label: &str) -> Self {
        let number = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "runyte-snapshot-boundary-{label}-{}-{number}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

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
    for character in [' ', '/', 'f'] {
        press(&mut app, character);
    }
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

/// Retypes the finder's query from scratch, so each selection in the test
/// below is reached the way a person reaches it.
fn requery(app: &mut App, query: &str) {
    for _ in 0..40 {
        app.handle_key(KeyStroke::new(KeyCode::Backspace, Modifiers::NONE))
            .unwrap();
    }
    for character in query.chars() {
        press(app, character);
    }
}

/// The preview beside a finder's rows is the reason the overlay is as wide as
/// it is, and it is the one part of that overlay whose content comes from
/// outside the editor. Each shape the selected entry can take has to reach the
/// screen: a file's first lines, a directory's entries, a refusal to show
/// bytes that are not text, and a file that has gone since it was listed.
#[test]
fn a_finder_preview_is_drawn_for_every_shape_the_selected_entry_can_have() {
    let project = TempProject::new("finder-preview-shapes");
    fs::write(
        project.path().join("alpha.txt"),
        "preview-only-line\nsecond line\n",
    )
    .unwrap();
    fs::write(project.path().join("ledger.bin"), [0u8, 1, 2, b'x']).unwrap();
    fs::create_dir(project.path().join("nested")).unwrap();
    fs::write(project.path().join("nested/inside.txt"), "inside\n").unwrap();
    fs::write(project.path().join("ghost.txt"), "gone\n").unwrap();

    // Rooted at the throwaway project rather than at whatever workspace the
    // test process happens to have been started in.
    let mut app = App::new_in_project(Config::default(), None, project.path()).unwrap();
    press(&mut app, ' ');
    press(&mut app, 'f');

    requery(&mut app, "alpha.txt");
    let text = standalone(&mut app);
    assert!(
        text.iter().any(|line| line.contains("Preview")),
        "the preview pane is titled:\n{}",
        text.join("\n")
    );
    assert!(
        text.iter().any(|line| line.contains("preview-only-line")),
        "a text file is previewed by its first lines:\n{}",
        text.join("\n")
    );

    requery(&mut app, "ledger.bin");
    let binary = standalone(&mut app);
    assert!(
        binary.iter().any(|line| line.contains("Binary file")),
        "bytes that are not text are refused rather than drawn:\n{}",
        binary.join("\n")
    );

    requery(&mut app, "nested");
    let directory = standalone(&mut app);
    assert!(
        directory.iter().any(|line| line.contains("inside.txt")),
        "a directory is previewed by what it holds:\n{}",
        directory.join("\n")
    );

    // The operating system's wording for a missing file is its own, so what
    // is asserted here is that the pane is still drawn and that it does not
    // show the content the file used to have.
    fs::remove_file(project.path().join("ghost.txt")).unwrap();
    requery(&mut app, "ghost.txt");
    let missing = standalone(&mut app);
    assert!(
        missing.iter().any(|line| line.contains("ghost.txt")),
        "a file that has gone since it was listed is still a row:\n{}",
        missing.join("\n")
    );
    assert!(
        !missing.iter().any(|line| line.contains("gone")),
        "and its former content is not shown:\n{}",
        missing.join("\n")
    );

    // The attached client draws the same overlay from the same snapshot.
    requery(&mut app, "alpha.txt");
    let mut host = WorkspaceHost::new(app);
    let published = attached(&mut host);
    assert!(
        published
            .iter()
            .any(|line| line.contains("preview-only-line")),
        "the attached renderer draws the preview too:\n{}",
        published.join("\n")
    );
}

/// The buffer picker's own action menu is a standalone-frontend widget drawn
/// over the list it acts on, rather than one of the shared overlay snapshots.
/// It has to name the buffer it will act on and offer that buffer's actions,
/// because the list underneath it says nothing about which row Tab opened.
#[test]
fn the_buffer_action_menu_names_its_buffer_and_lists_that_buffer_s_actions() {
    let project = TempProject::new("buffer-action-menu");
    let path = project.path().join("notes.txt");
    fs::write(&path, "before").unwrap();
    let mut app =
        App::new_in_project(Config::default(), Some(path.clone()), project.path()).unwrap();

    press(&mut app, ' ');
    press(&mut app, 'b');
    press(&mut app, 'b');
    app.handle_key(KeyStroke::new(KeyCode::Tab, Modifiers::NONE))
        .unwrap();

    let screen = standalone(&mut app);
    assert!(
        screen.iter().any(|line| line.contains("notes.txt")),
        "the menu is titled with the buffer it acts on:\n{}",
        screen.join("\n")
    );
    assert!(
        screen
            .iter()
            .any(|line| line.to_lowercase().contains("close")),
        "and offers that buffer's actions:\n{}",
        screen.join("\n")
    );
}
