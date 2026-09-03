// SPDX-License-Identifier: MPL-2.0

use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Modifier};
use runyte::{
    app::App,
    command::{CommandInvocation, EditorCommand, GrammarKind, HelpInvocation, Mode},
    config::Config,
    external_open::ProgramCache,
    input::{KeyCode, KeyStroke, Modifiers},
    key_hints::{
        HintEventResult, KeyHintState, key_hint_description, key_hint_keys, key_hint_layout,
    },
    keymap::{
        Binding, BindingAvailability, BindingRole, BindingScope, BindingTarget, Key, Keymap,
        default_keymap,
    },
    picker::{ListPicker, PickerItem},
    selection::Selection,
    text::Transaction,
    ui,
    workspace::WorkspaceHost,
};
use unicode_width::UnicodeWidthStr as _;

fn render_buffer(width: u16, height: u16, app: &mut App, hints: &KeyHintState) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let prepared = app.prepare_view(ui::frame_geometry(frame.area()));
            let snapshot = app.snapshot(&prepared);
            ui::render_exact_colors_for_test(frame, app, &snapshot, hints);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn render(width: u16, height: u16, app: &mut App, hints: &KeyHintState) -> String {
    let buffer = render_buffer(width, height, app, hints);
    (0..height)
        .map(|row| {
            (0..width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_attached(width: u16, height: u16, app: App, hints: &KeyHintState) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut host = WorkspaceHost::new(app);
    terminal
        .draw(|frame| {
            let snapshot =
                host.prepare_frame_with_hints(ui::frame_geometry(frame.area()), Some(hints));
            ui::render_host_frame_exact_colors_for_test(frame, &snapshot);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn key_hint_surface(buffer: &Buffer) -> Vec<String> {
    let rows = (0..buffer.area.height)
        .map(|row| {
            (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let top = rows
        .iter()
        .position(|row| row.starts_with("┌ Keys:"))
        .expect("key-hint top border");
    let bottom = rows[top..]
        .iter()
        .position(|row| row.starts_with('└'))
        .map(|row| top + row)
        .expect("key-hint bottom border");
    rows[top..=bottom].to_vec()
}

fn hint_metrics(app: &App, hints: &KeyHintState) -> (usize, usize, Vec<String>) {
    let mode = app.key_hint_mode().unwrap_or(app.mode);
    let mut rows = hints.rows_in(app.keymap(), mode, app.key_binding_scope());
    let capabilities = app.command_capabilities();
    for row in &mut rows {
        row.apply_capabilities(&capabilities);
    }
    let widest_key = rows
        .iter()
        .map(|row| key_hint_keys(row).width())
        .max()
        .unwrap_or_default();
    let descriptions = rows.iter().map(key_hint_description).collect::<Vec<_>>();
    let widest_description = descriptions
        .iter()
        .map(|description| description.width())
        .max()
        .unwrap_or_default();
    (widest_key, widest_description, descriptions)
}

fn first_cell_of(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for row in 0..buffer.area.height {
        let text = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        if let Some(column) = text.find(needle) {
            return (column as u16, row);
        }
    }
    panic!("missing {needle:?} in rendered buffer");
}

fn stroke(code: KeyCode, modifiers: Modifiers) -> KeyStroke {
    KeyStroke::new(code, modifiers)
}

fn dispatch_with_hints(app: &mut App, hints: &mut KeyHintState, key: KeyStroke) {
    let hint_result = match app.key_hint_mode_for_key(key) {
        Some(mode) => hints.observe_in(key, mode, app.key_binding_scope(), app.keymap()),
        None => {
            hints.clear();
            HintEventResult::Forward
        }
    };
    if hint_result == HintEventResult::Forward {
        app.handle_key(key).unwrap();
    }
}

#[test]
fn replacement_space_is_not_observed_as_a_space_command_prefix() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers[0].apply(&Transaction::insert(0, "ab"));
    let mut hints = KeyHintState::default();

    dispatch_with_hints(&mut app, &mut hints, KeyStroke::char('r'));
    assert_eq!(app.key_hint_mode(), None);
    assert_eq!(app.mode, Mode::Normal);

    dispatch_with_hints(&mut app, &mut hints, KeyStroke::char(' '));
    assert_eq!(app.buffers[0].text().to_string(), " b");
    assert_eq!(app.key_hint_mode(), Some(Mode::Normal));
    assert!(!hints.is_visible());
    assert!(!hints.is_pending());

    dispatch_with_hints(&mut app, &mut hints, KeyStroke::char(' '));
    assert_eq!(hints.display_pending(), "Space");
    assert!(hints.is_visible());
    assert!(hints.is_pending());
}

#[cfg(unix)]
#[test]
fn terminal_control_w_starts_the_insert_pane_navigation_prefix() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    render(80, 20, &mut app, &hints);
    type_colon(&mut app, "terminal /bin/cat");
    assert_eq!(app.mode, Mode::Insert);

    dispatch_with_hints(&mut app, &mut hints, KeyStroke::ctrl('w'));

    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.pending_sequence().to_string(), "Ctrl-w");
    assert!(hints.is_visible());
    assert!(hints.is_pending());
}

/// Runs a colon command the way a person does, through the palette.
fn type_colon(app: &mut App, command: &str) {
    app.handle_key(KeyStroke::char(':')).unwrap();
    for character in command.chars() {
        app.handle_key(KeyStroke::char(character)).unwrap();
    }
    app.handle_key(KeyStroke::plain(KeyCode::Enter)).unwrap();
}

#[test]
fn prefix_popup_is_readable_at_standard_and_wide_sizes() {
    for (width, height) in [(80, 24), (160, 50)] {
        let mut app = App::new(Config::default(), None).unwrap();
        let mut hints = KeyHintState::default();
        hints.observe(
            stroke(KeyCode::Char(' '), Modifiers::NONE),
            Mode::Normal,
            default_keymap(),
        );

        let screen = render(width, height, &mut app, &hints);
        assert!(screen.contains("Keys: Space"));
        assert!(screen.contains("Clipboard ›"));
        assert!(screen.contains("Language (LSP) ›"));
        assert!(screen.contains("Terminals ›"));
    }
}

#[test]
fn standalone_and_attached_hints_share_responsive_grid_boundaries() {
    for prefix in [KeyStroke::char(' '), KeyStroke::ctrl('w')] {
        let mut hints = KeyHintState::default();
        hints.observe(prefix, Mode::Normal, default_keymap());
        let metrics_app = App::new(Config::default(), None).unwrap();
        let (widest_key, widest_description, descriptions) = hint_metrics(&metrics_app, &hints);
        let key_width = widest_key.clamp(12, 20);
        let stride = key_width + 1 + widest_description + 2;

        for (expected_columns, width) in [
            (1, 2 * stride + 1),
            (2, 2 * stride + 2),
            (2, 3 * stride + 1),
            (3, 3 * stride + 2),
        ] {
            let width = width as u16;
            let height = 20;
            let layout = key_hint_layout(
                width,
                height,
                descriptions.len(),
                widest_key,
                widest_description,
                0,
            );
            assert_eq!(layout.columns, expected_columns, "prefix {prefix}");

            let standalone = render_buffer(
                width,
                height,
                &mut App::new(Config::default(), None).unwrap(),
                &hints,
            );
            let attached = render_attached(
                width,
                height,
                App::new(Config::default(), None).unwrap(),
                &hints,
            );
            let standalone_surface = key_hint_surface(&standalone);
            let attached_surface = key_hint_surface(&attached);
            assert_eq!(
                attached_surface, standalone_surface,
                "prefix {prefix} at width {width}"
            );

            let visible = standalone_surface.join("\n");
            for description in descriptions.iter().take(layout.visible_rows) {
                assert!(
                    visible.contains(description),
                    "clipped {description:?} for prefix {prefix} at width {width}:\n{visible}"
                );
            }
            if expected_columns == 3 {
                let positions = [0, layout.content_rows, layout.content_rows * 2]
                    .map(|index| first_cell_of(&standalone, &descriptions[index]));
                assert_eq!(positions[0].1, positions[1].1, "prefix {prefix}");
                assert_eq!(positions[1].1, positions[2].1, "prefix {prefix}");
                assert!(
                    positions[0].0 < positions[1].0 && positions[1].0 < positions[2].0,
                    "rows did not fill down each column for prefix {prefix}: {positions:?}"
                );
            }
        }
    }
}

#[test]
fn attached_hint_scrolling_uses_the_shared_multicolumn_capacity() {
    let mut hints = KeyHintState::default();
    hints.observe(KeyStroke::char(' '), Mode::Normal, default_keymap());
    let metrics_app = App::new(Config::default(), None).unwrap();
    let (widest_key, widest_description, descriptions) = hint_metrics(&metrics_app, &hints);
    let width = (2 + 2 * (widest_key.clamp(12, 20) + 1 + widest_description + 2)) as u16;
    let height = 10;

    let _ = render_buffer(
        width,
        height,
        &mut App::new(Config::default(), None).unwrap(),
        &hints,
    );
    for _ in 0..descriptions.len() {
        hints.observe(KeyStroke::ctrl('n'), Mode::Normal, default_keymap());
    }
    let layout = key_hint_layout(
        width,
        height - 2,
        descriptions.len(),
        widest_key,
        widest_description,
        hints.scroll_offset(),
    );
    assert_eq!(layout.columns, 2);
    assert_eq!(hints.scroll_offset(), descriptions.len() - layout.capacity);

    let standalone = render_buffer(
        width,
        height,
        &mut App::new(Config::default(), None).unwrap(),
        &hints,
    );
    let attached = render_attached(
        width,
        height,
        App::new(Config::default(), None).unwrap(),
        &hints,
    );
    assert_eq!(key_hint_surface(&attached), key_hint_surface(&standalone));
    let title = key_hint_surface(&attached)[0].clone();
    assert!(
        title.contains(&format!(
            "{}-{}/{}",
            layout.offset + 1,
            layout.offset + layout.visible_rows,
            descriptions.len()
        )),
        "{title}"
    );
}

/// The `Space` namespace is longer than the smallest supported terminal, so
/// the last rows are reached by scrolling rather than by being on screen. The
/// popup says so in its own header, and the assertion here is that the row is
/// reachable — not that a fixed number of them happen to fit.
#[test]
fn the_last_prefix_row_is_reachable_at_the_smallest_supported_size() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    assert!(!render(80, 24, &mut app, &hints).contains("Syntax (Tree-sitter) ›"));

    hints.observe(
        stroke(KeyCode::Down, Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    hints.observe(
        stroke(KeyCode::Down, Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    assert!(render(80, 24, &mut app, &hints).contains("Syntax (Tree-sitter) ›"));
}

#[test]
fn unavailable_language_and_syntax_namespaces_are_dimmed_but_navigable() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );

    let buffer = render_buffer(160, 30, &mut app, &hints);
    for label in ["Language (LSP)", "Syntax (Tree-sitter)"] {
        let cell = first_cell_of(&buffer, label);
        assert!(buffer[cell].modifier.contains(Modifier::DIM), "{label}");
    }

    hints.observe(
        stroke(KeyCode::Char('l'), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    let language = render(160, 30, &mut app, &hints);
    assert!(language.contains("Keys: Space l"), "{language}");
    assert!(language.contains("lsp status"), "{language}");
    assert!(language.contains("no LSP"), "{language}");
}

#[test]
fn unavailable_git_namespace_is_dimmed_but_navigable() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );

    let buffer = render_buffer(160, 30, &mut app, &hints);
    let cell = first_cell_of(&buffer, "Git ›");
    assert!(buffer[cell].modifier.contains(Modifier::DIM));

    hints.observe(
        stroke(KeyCode::Char('g'), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    let git = render(160, 30, &mut app, &hints);
    assert!(git.contains("Keys: Space g"), "{git}");
    assert!(git.contains("git status"), "{git}");
    assert!(git.contains("no Git"), "{git}");
}

#[test]
fn the_search_namespaces_and_their_prompts_are_discoverable_on_screen() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    // Searching and selecting are two namespaces now, and the Space row says so.
    let space = render(160, 40, &mut app, &hints);
    assert!(space.contains("Look past this buffer \u{203a}"), "{space}");
    assert!(space.contains("Selections \u{203a}"), "{space}");
    assert!(space.contains("open finder"), "{space}");

    hints.observe(
        stroke(KeyCode::Char('s'), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    let selections = render(100, 40, &mut app, &hints);
    for entry in [
        "split selection at line ends",
        "split selection at line starts",
        "Drop every selection except the primary",
        "keep matching selections",
        "remove matching selections",
        "align selections",
    ] {
        assert!(
            selections.contains(entry),
            "missing {entry:?}: {selections}"
        );
    }
    // Nothing that looks past the selection is left behind under this prefix.
    for entry in ["Search for text", "Fuzzy-", "Search the workspace"] {
        assert!(
            !selections.contains(entry),
            "{entry:?} still sits under Space s: {selections}"
        );
    }
    // The namespace is the only way to reach these, so the rows must name the
    // keys that actually work.
    for row in ["Space s c", "Space s k", "Space s r"] {
        assert!(selections.contains(row), "missing {row:?}: {selections}");
    }
    // Where the same command also answers to a short key, the row names it:
    // the namespace is where someone meets the command for the first time.
    for row in ["Space s a, &", "Space s c, ,"] {
        assert!(
            selections.contains(row),
            "row does not name its short spelling {row:?}: {selections}"
        );
    }

    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    hints.observe(
        stroke(KeyCode::Char('/'), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    let project = render(100, 40, &mut app, &hints);
    for entry in [
        "global search regex",
        "Search the workspace, ignoring case",
        "open finder",
        "open all files picker",
        "open path file picker",
    ] {
        assert!(project.contains(entry), "missing {entry:?}: {project}");
    }
    assert!(
        project.contains("Space / f, Space f"),
        "the finder row does not name its short spelling: {project}"
    );
    // Only the flavour that has one; the rest carry no alias.
    assert!(
        project.contains("Space / s          Search the workspace, ignoring case"),
        "unaliased project rows changed shape: {project}"
    );

    // Each flavour names itself at the prompt rather than sharing one label.
    for (opener, label) in [('s', "search: "), ('/', "search (regex): ")] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.handle_key(stroke(KeyCode::Char(opener), Modifiers::NONE))
            .unwrap();
        let screen = render(100, 40, &mut app, &KeyHintState::default());
        assert!(screen.contains(label), "{opener} prompt: {screen}");
    }
}

#[test]
fn every_pending_prefix_opens_the_popup() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char('g'), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );

    let screen = render(120, 30, &mut app, &hints);
    assert!(screen.contains("Keys: g"));
    assert!(screen.contains("Move to file start"));
    assert!(screen.contains("Move to file end"));
}

#[test]
fn a_key_column_separates_the_sequence_from_its_description() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );

    let screen = render(160, 50, &mut app, &hints);
    assert!(screen.contains("Space e      open explorer"));
    assert!(!screen.contains("Space eOpen"));
}

#[test]
fn narrow_popup_is_bounded_and_scrollable() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );

    let first = render(40, 10, &mut app, &hints);
    assert!(first.contains("1-6/"));
    assert!(first.contains("Ctrl-n/p"));
    assert!(first.contains("↑/↓"));
    assert!(!first.contains("Alt-j/k"));

    assert_eq!(
        hints.observe(
            stroke(KeyCode::Down, Modifiers::NONE),
            Mode::Normal,
            default_keymap(),
        ),
        HintEventResult::Consumed
    );
    let scrolled = render(40, 10, &mut app, &hints);
    assert!(scrolled.contains("2-7/"));
}

#[test]
fn popup_with_bound_arrows_advertises_only_control_scroll() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char('z'), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );

    let first = render(36, 10, &mut app, &hints);
    assert!(first.contains("Ctrl-n/p"));
    assert!(!first.contains("Alt-j/k"));
    assert!(!first.contains("↑/↓"));

    assert_eq!(
        hints.observe(KeyStroke::ctrl('n'), Mode::Normal, default_keymap(),),
        HintEventResult::Consumed
    );
    assert_eq!(hints.scroll_offset(), 1);
    assert_eq!(hints.pending().to_string(), "z");
}

#[test]
fn ctrl_w_popup_scrolls_with_arrows_and_advertises_them() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(KeyStroke::ctrl('w'), Mode::Normal, default_keymap());

    let first = render(40, 10, &mut app, &hints);
    assert!(first.contains("Ctrl-n/p"));
    assert!(first.contains("↑/↓"));
    assert!(!first.contains("Alt-j/k"));

    assert_eq!(
        hints.observe(
            KeyStroke::plain(KeyCode::Down),
            Mode::Normal,
            default_keymap(),
        ),
        HintEventResult::Consumed
    );
    assert_eq!(hints.scroll_offset(), 1);
    assert_eq!(hints.pending().to_string(), "Ctrl-w");

    assert_eq!(
        hints.observe(
            KeyStroke::plain(KeyCode::Up),
            Mode::Normal,
            default_keymap(),
        ),
        HintEventResult::Consumed
    );
    assert_eq!(hints.scroll_offset(), 0);
    assert_eq!(hints.pending().to_string(), "Ctrl-w");
}

#[test]
fn ctrl_w_popup_advertises_terminal_creation_from_the_registry() {
    let mut hints = KeyHintState::default();
    hints.observe(KeyStroke::ctrl('w'), Mode::Normal, default_keymap());

    let terminal = hints
        .rows(default_keymap(), Mode::Normal)
        .into_iter()
        .find(|row| row.sequence == [Key::ctrl('w'), Key::char('t')].into())
        .expect("Ctrl-w hints include terminal creation");

    assert_eq!(
        terminal.target,
        Some(BindingTarget::Editor(EditorCommand::OpenTerminal))
    );
    assert_eq!(terminal.description, "Run a shell or command in this pane");
    assert_eq!(terminal.role, BindingRole::Compatibility);
}

#[test]
fn alt_j_and_k_cancel_insert_ctrl_w_instead_of_scrolling() {
    for character in ['j', 'k'] {
        let mut app = App::new(Config::default(), None).unwrap();
        let mut hints = KeyHintState::default();
        app.handle_key(KeyStroke::char('i')).unwrap();

        dispatch_with_hints(&mut app, &mut hints, KeyStroke::ctrl('w'));
        assert_eq!(app.pending_sequence().to_string(), "Ctrl-w");
        assert_eq!(hints.pending().to_string(), "Ctrl-w");

        dispatch_with_hints(
            &mut app,
            &mut hints,
            KeyStroke::new(KeyCode::Char(character), Modifiers::ALT),
        );
        assert_eq!(app.mode, Mode::Insert);
        assert!(app.pending_sequence().is_empty());
        assert!(!hints.is_pending());
        assert!(app.buffers[0].text().is_empty());
    }
}

#[cfg(unix)]
#[test]
fn terminal_ctrl_w_hint_scrolls_with_control_n_and_p_without_dispatching() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    render(40, 10, &mut app, &hints);
    type_colon(&mut app, "terminal /bin/cat");

    dispatch_with_hints(&mut app, &mut hints, KeyStroke::ctrl('w'));
    hints.note_scroll_limit(10);
    dispatch_with_hints(&mut app, &mut hints, KeyStroke::ctrl('n'));

    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.pending_sequence().to_string(), "Ctrl-w");
    assert_eq!(hints.pending().to_string(), "Ctrl-w");
    assert_eq!(hints.scroll_offset(), 1);

    dispatch_with_hints(&mut app, &mut hints, KeyStroke::ctrl('p'));
    assert_eq!(app.pending_sequence().to_string(), "Ctrl-w");
    assert_eq!(hints.pending().to_string(), "Ctrl-w");
    assert_eq!(hints.scroll_offset(), 0);
}

#[test]
fn repeated_arrow_scroll_saturates_at_the_rendered_end() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    assert!(render(40, 10, &mut app, &hints).contains("1-6/18"));

    for _ in 0..30 {
        assert_eq!(
            hints.observe(
                stroke(KeyCode::Down, Modifiers::NONE),
                Mode::Normal,
                default_keymap(),
            ),
            HintEventResult::Consumed
        );
    }
    assert_eq!(hints.scroll_offset(), 12);
    assert!(render(40, 10, &mut app, &hints).contains("13-18/18"));

    assert_eq!(
        hints.observe(
            stroke(KeyCode::Up, Modifiers::NONE),
            Mode::Normal,
            default_keymap(),
        ),
        HintEventResult::Consumed
    );
    assert_eq!(hints.scroll_offset(), 11);
    assert!(render(40, 10, &mut app, &hints).contains("12-17/18"));
}

#[test]
fn unavailable_action_is_dimmed_and_labeled() {
    const NORMAL: &[Mode] = &[Mode::Normal];
    let keymap: &'static Keymap = Box::leak(Box::new(
        Keymap::new(vec![Binding {
            modes: NORMAL,
            scope: BindingScope::Global,
            sequence: [Key::char(' '), Key::char('x')].into(),
            target: BindingTarget::Editor(EditorCommand::SelectLine),
            description: "Parser action",
            availability: BindingAvailability::Planned("requires parser"),
            role: BindingRole::Primary,
            alias: None,
            alias_modes: None,
        }])
        .unwrap(),
    ));
    let mut app = App::new(Config::default(), None).unwrap();
    app.set_keymap(keymap);
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        keymap,
    );

    let screen = render(80, 24, &mut app, &hints);
    assert!(screen.contains("Parser action"));
    assert!(screen.contains("planned: requires parser"));
}

#[test]
fn invalid_sequence_renders_a_concise_message() {
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    hints.observe(
        stroke(KeyCode::Char('z'), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    let mut app = App::new(Config::default(), None).unwrap();

    let screen = render(40, 10, &mut app, &hints);
    assert!(screen.contains("No binding: Space z"));
}

#[test]
fn resolved_bindings_never_open_the_popup() {
    let mut app = App::new(Config::default(), None).unwrap();
    for key in [
        stroke(KeyCode::Char('h'), Modifiers::NONE),
        stroke(KeyCode::Char('k'), Modifiers::NONE),
        stroke(KeyCode::Char('x'), Modifiers::NONE),
        stroke(KeyCode::Char('~'), Modifiers::NONE),
    ] {
        let mut hints = KeyHintState::default();
        hints.observe(key, Mode::Normal, default_keymap());

        assert!(!hints.is_visible());
        assert!(!render(80, 24, &mut app, &hints).contains("Keys:"));
    }
}

#[test]
fn higher_priority_overlays_hide_key_hints() {
    let mut hints = KeyHintState::default();
    hints.observe(
        stroke(KeyCode::Char(' '), Modifiers::NONE),
        Mode::Normal,
        default_keymap(),
    );
    let mut app = App::new(Config::default(), None).unwrap();
    app.list = Some(ListPicker::new(
        "Overlay",
        vec![PickerItem::new("only entry", "detail", 0)],
    ));

    let screen = render(80, 24, &mut app, &hints);
    assert!(screen.contains("Overlay"));
    assert!(!screen.contains("Keys: Space"));
}

/// Help is an ordinary buffer, so what it says is what the buffer holds.
/// A read-only buffer says so in the pane title and the status line, and its
/// help says so in the title. The pane title identifies the buffer, while the
/// global status line pairs active-buffer state with workspace context. The
/// help title describes the buffer type the document is about. Both markers
/// must be present for a read-only view.
#[test]
fn a_read_only_buffer_is_marked_on_every_surface() {
    let hints = KeyHintState::default();
    let mut app = App::new(Config::default(), None).unwrap();

    // An ordinary scratch buffer claims nothing, on either line.
    let editable = render(100, 20, &mut app, &hints);
    assert!(!editable.contains("[RO]"), "{editable}");

    type_colon(&mut app, "config");

    // The pane title and global status line both carry the compact marker, but
    // only the pane title repeats the buffer identity.
    let screen = render(100, 20, &mut app, &hints);
    let marked: Vec<&str> = screen
        .lines()
        .filter(|line| line.contains("[RO]"))
        .collect();
    assert_eq!(
        marked.len(),
        2,
        "expected the pane title and the status line to be marked:\n{screen}"
    );
    assert!(
        marked.iter().any(|line| line.contains("[config]"))
            && marked.iter().any(|line| line.contains("Workspace:")),
        "the pane identity and workspace status markers diverged:\n{screen}"
    );

    // Its help states the same fact in full, in the title.
    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    let document = app.active_buffer().to_string();
    assert!(
        document.starts_with("Help · RUNYTE · CONFIG · Read-only"),
        "{document}"
    );
}

#[test]
fn pane_titles_show_structural_file_and_explorer_types() {
    let directory =
        std::env::temp_dir().join(format!("runyte-pane-buffer-types-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let path = directory.join("notes.txt");
    std::fs::write(&path, "notes").unwrap();
    let hints = KeyHintState::default();

    let mut file = App::new(Config::default(), Some(path.clone())).unwrap();
    let screen = render(180, 20, &mut file, &hints);
    assert!(
        screen
            .lines()
            .next()
            .unwrap()
            .contains(&format!("[file] {}", path.display())),
        "{screen}"
    );

    let mut explorer = App::new(Config::default(), Some(directory.clone())).unwrap();
    let screen = render(180, 20, &mut explorer, &hints);
    assert!(
        screen
            .lines()
            .next()
            .unwrap()
            .contains(&format!("[explorer] {}", directory.display())),
        "{screen}"
    );

    std::fs::remove_dir_all(directory).unwrap();
}

/// A maximized pane names the view it is showing in its own title, so the one
/// pane left on screen still says why the split it came from is not there. The
/// tag appears only while the view is on, and the two views never both claim
/// the title.
#[test]
fn a_maximized_pane_is_tagged_with_the_view_it_shows() {
    let hints = KeyHintState::default();
    let mut app = App::new(Config::default(), None).unwrap();

    let ordinary = render(100, 20, &mut app, &hints);
    assert!(!ordinary.contains("[zen]"), "{ordinary}");
    assert!(!ordinary.contains("[fullscreen]"), "{ordinary}");

    type_colon(&mut app, "zen");
    let zen = render(100, 20, &mut app, &hints);
    let title = zen.lines().next().unwrap();
    assert!(title.contains("[zen]"), "{zen}");
    assert!(!zen.contains("[fullscreen]"), "{zen}");

    // The two views are one state, so asking for the other one replaces the
    // tag rather than adding a second.
    type_colon(&mut app, "fullscreen");
    let fullscreen = render(100, 20, &mut app, &hints);
    let title = fullscreen.lines().next().unwrap();
    assert!(title.contains("[fullscreen]"), "{fullscreen}");
    assert!(!fullscreen.contains("[zen]"), "{fullscreen}");

    type_colon(&mut app, "fullscreen");
    let restored = render(100, 20, &mut app, &hints);
    assert!(!restored.contains("[fullscreen]"), "{restored}");
    assert!(!restored.contains("[zen]"), "{restored}");
}

/// Help for an editable buffer type says nothing about being read-only, even
/// though the help buffer showing it is. The title is about the subject; the
/// status line is about what is on screen.
#[test]
fn help_for_an_editable_view_is_not_titled_read_only() {
    let hints = KeyHintState::default();
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();

    let document = app.active_buffer().to_string();
    assert!(document.starts_with("Help · RUNYTE · TEXT\n"), "{document}");
    assert!(!document.contains("Read-only"), "{document}");

    // The help buffer itself is read-only, and says so where it is displayed.
    let screen = render(100, 20, &mut app, &hints);
    assert!(screen.contains("[RO]"), "{screen}");
}

#[test]
fn help_opens_a_buffer_describing_the_current_view() {
    let directory = std::env::temp_dir().join(format!("runyte-help-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("child")).unwrap();

    let mut app = App::new(Config::default(), None).unwrap();
    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();

    assert!(app.active_buffer().is_help());
    let normal = app.active_buffer().to_string();
    assert!(normal.starts_with("Help · RUNYTE · TEXT"), "{normal}");
    assert!(normal.contains("modal editor"), "{normal}");
    // Derived rather than curated, so an ordinary motion is documented too.
    assert!(normal.contains("Move to next word start"), "{normal}");
    assert!(normal.contains("Where to start"), "{normal}");
    assert!(normal.contains("Direct keys"), "{normal}");

    let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    let explorer = app.active_buffer().to_string();
    assert!(
        explorer.starts_with("Help · RUNYTE · EXPLORER"),
        "{explorer}"
    );
    assert!(
        explorer.contains("editable directory listing"),
        "{explorer}"
    );
    assert!(explorer.contains("Buffer keys"), "{explorer}");
    assert!(explorer.contains("Show or hide dotfiles"), "{explorer}");

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn removed_vim_configuration_falls_back_to_runyte_help_and_hints() {
    let mut hints = KeyHintState::default();
    let mut config = Config::default();
    config.editor.grammar = GrammarKind::Vim;
    let mut app = App::new(config, None).unwrap();
    assert_eq!(app.grammar_kind(), GrammarKind::Runyte);
    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();

    let help = app.active_buffer().to_string();
    assert!(help.starts_with("Help · RUNYTE · TEXT"), "{help}");
    assert!(
        help.contains("Runyte is a selection-first modal editor"),
        "{help}"
    );
    assert!(!help.contains("Vim grammar"), "{help}");
    app.handle_key(KeyStroke::char('q')).unwrap();

    hints.observe(KeyStroke::char('z'), Mode::Normal, default_keymap());
    app.handle_key(KeyStroke::char('z')).unwrap();
    let viewport = render(100, 30, &mut app, &hints);
    assert!(viewport.contains("Align the cursor line"), "{viewport}");
}

/// The popup this replaced truncated to the window and showed no indicator,
/// so a short terminal silently lost rows. A buffer holds the whole document
/// at every size; only how much of it is on screen changes.
#[test]
fn help_is_complete_at_every_terminal_size() {
    let hints = KeyHintState::default();
    let mut reference = App::new(Config::default(), None).unwrap();
    reference
        .execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    let document = reference.active_buffer().to_string();
    assert!(
        document.lines().count() > 40,
        "help got short enough to fit a window, weakening this test"
    );

    for (width, height) in [(24, 6), (40, 8), (80, 24)] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
            .unwrap();
        assert_eq!(app.active_buffer().to_string(), document);

        // The top of the document is what a reader sees first at any size.
        let screen = render(width, height, &mut app, &hints);
        assert!(
            screen.contains("Help ·"),
            "{width}x{height} lost the title:\n{screen}"
        );
    }
}

/// Ordinary motions and search work in help because it is an ordinary buffer.
/// That is the whole reason it stopped being an overlay.
#[test]
fn help_scrolls_and_searches_like_any_other_buffer() {
    let hints = KeyHintState::default();
    let mut app = App::new(Config::default(), None).unwrap();
    app.execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();

    // A section far enough down to be off-screen at a standard size.
    let opening = render(80, 24, &mut app, &hints);
    assert!(!opening.contains("Ctrl chords"), "{opening}");

    app.handle_key(KeyStroke::char('g')).unwrap();
    app.handle_key(KeyStroke::char('e')).unwrap();
    let end = render(80, 24, &mut app, &hints);
    assert!(
        end.contains("Ctrl-o") || end.contains("Arrows and named keys"),
        "moving to the end did not scroll help:\n{end}"
    );

    // Editing it is refused by name rather than by silence.
    app.handle_key(KeyStroke::char('d')).unwrap();
    assert_eq!(app.status, "help is read-only");
}

#[test]
fn list_overlay_enter_does_not_become_a_normal_mode_hint() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.list = Some(ListPicker::new(
        "Sessions",
        vec![PickerItem::new("demo", "Demo workspace", 0)],
    ));
    let mut hints = KeyHintState::default();
    let key = stroke(KeyCode::Enter, Modifiers::NONE);

    let hint_result = if app.has_input_overlay() {
        hints.clear();
        HintEventResult::Forward
    } else {
        hints.observe(KeyStroke::plain(KeyCode::Enter), app.mode, app.keymap())
    };
    if hint_result == HintEventResult::Forward {
        app.handle_key(key).unwrap();
    }

    assert!(app.list.is_none());
    assert!(!hints.is_visible());
    assert_eq!(hints.message(), None);
}

#[test]
fn insert_and_command_modes_hide_normal_mode_hints() {
    for (stroke, key) in [
        (
            KeyStroke::char('i'),
            stroke(KeyCode::Char('i'), Modifiers::NONE),
        ),
        (
            KeyStroke::char(':'),
            stroke(KeyCode::Char(':'), Modifiers::NONE),
        ),
    ] {
        let mut hints = KeyHintState::default();
        let mut app = App::new(Config::default(), None).unwrap();
        hints.observe(stroke, app.mode, app.keymap());
        app.handle_key(key).unwrap();

        let screen = render(80, 24, &mut app, &hints);
        assert!(!screen.contains("Keys:"));
    }
}

#[test]
fn exact_one_key_action_is_forwarded_without_a_post_factum_hint() {
    let mut hints = KeyHintState::default();
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers[0].apply(&Transaction::insert(0, "ab"));
    app.panes.get_mut(&0).unwrap().selection = Selection::point(1);
    let key = stroke(KeyCode::Char('h'), Modifiers::NONE);

    assert_eq!(
        hints.observe(KeyStroke::char('h'), app.mode, app.keymap()),
        HintEventResult::Forward
    );
    app.handle_key(key).unwrap();

    assert_eq!(app.cursor_position().col, 0);
    assert!(!hints.is_visible());
    assert!(!hints.is_pending());
}

/// A binary file named on the command line is asked about, not opened, and the
/// remembered programs are offered above the prompt.
#[test]
fn a_binary_argument_opens_the_open_with_prompt_over_its_hints() {
    let directory = std::env::temp_dir().join(format!("runyte-open-with-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("photo.png");
    std::fs::write(&binary, [0x89, b'P', b'N', b'G', 0x00]).unwrap();

    let hints = KeyHintState::default();
    let mut app = App::new(Config::default(), Some(binary)).unwrap();
    assert!(app.buffers[0].to_string().is_empty(), "a scratch buffer");

    // A cache under the temporary directory, never the person's real one.
    app.programs = ProgramCache::load(Some(directory.join("cache")));
    app.programs.remember("feh").unwrap();
    app.programs.remember("xdg-open").unwrap();
    app.command.clear();
    app.command_cursor = 0;

    let screen = render(76, 14, &mut app, &hints);
    assert!(screen.contains("Open with"), "{screen}");
    assert!(screen.contains("open with:"), "{screen}");
    assert!(screen.contains("Enter open"), "{screen}");
    assert!(screen.contains("Tab actions"), "{screen}");
    assert!(screen.contains("default · system opener"), "{screen}");
    assert!(screen.contains("xdg-open"), "{screen}");
    assert!(screen.contains("feh"), "{screen}");

    app.handle_key(KeyStroke::plain(KeyCode::Down)).unwrap();
    app.handle_key(KeyStroke::plain(KeyCode::Tab)).unwrap();
    let actions = render(76, 14, &mut app, &hints);
    assert!(actions.contains("Delete"), "{actions}");
    assert!(actions.contains("Set as default"), "{actions}");
    assert!(actions.contains("Tab/Esc back"), "{actions}");

    std::fs::remove_dir_all(directory).unwrap();
}
