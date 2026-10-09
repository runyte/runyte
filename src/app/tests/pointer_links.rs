// SPDX-License-Identifier: MPL-2.0

use super::*;

type OpenedLinks = Arc<Mutex<Vec<String>>>;

fn fixture() -> (crate::test_support::TestRuntimeRoot, App, OpenedLinks) {
    let root = crate::test_support::TestRuntimeRoot::new("pointer-links").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    let opened = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&opened);
    app.ports.browser = Box::new(move |url| {
        recorded.lock().unwrap().push(url.to_owned());
        Ok(external_open::Dispatch::Accepted)
    });
    (root, app, opened)
}

fn geometry(width: u16) -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            width,
            height: 12,
            ..Rect::default()
        },
        editor: Rect {
            width,
            height: 10,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    }
}

fn link_modifier() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::SUPER
    } else {
        Modifiers::CONTROL
    }
}

fn click(app: &mut App, view: &PreparedView, column: u16, row: u16) {
    for kind in [
        PointerEventKind::Down(PointerButton::Left),
        PointerEventKind::Drag(PointerButton::Left),
        PointerEventKind::Up(PointerButton::Left),
    ] {
        app.handle_pointer(
            PointerEvent {
                kind,
                column,
                row,
                modifiers: link_modifier(),
            },
            view,
        )
        .unwrap();
    }
    assert!(!app.pointer_link_pressed);
    assert!(app.pointer_drag.is_none());
}

fn text_cell(app: &App, view: &PreparedView, pane: usize, offset: usize) -> (u16, u16) {
    let body = view.pane(pane).unwrap().body;
    for row in body.y..body.y + body.height {
        for column in body.x..body.x + body.width {
            if app.pointer_text_offset(view, pane, column, row) == Some(offset) {
                return (column, row);
            }
        }
    }
    panic!("offset {offset} is not visible");
}

#[test]
fn native_link_click_uses_clicked_pane_and_markdown_label_not_selection() {
    let (root, mut app, opened) = fixture();
    let source = root.join("links.md");
    fs::write(
        &source,
        "[docs](https://example.com/clicked)\nhttps://wrong.example.com\n",
    )
    .unwrap();
    app.open_file(source).unwrap();
    let pane = app.active_pane;
    app.active_mut()
        .replace_selection(Selection::single(Range::new(36, 50)));
    app.split(Axis::Horizontal, None).unwrap();
    let other = app.active_pane;
    assert_ne!(pane, other);
    let view = app.prepare_view(geometry(100));
    let (column, row) = text_cell(&app, &view, pane, 2);
    click(&mut app, &view, column, row);
    assert_eq!(app.active_pane, pane);
    assert_eq!(*opened.lock().unwrap(), ["https://example.com/clicked"]);
}

#[test]
fn native_link_click_rendered_markdown_follows_relative_path_and_heading() {
    let (root, mut app, _) = fixture();
    let notes = root.join("notes");
    fs::create_dir(&notes).unwrap();
    let source = notes.join("source.md");
    let target = notes.join("target.md");
    fs::write(&source, "[Chapter](target.md#second)\n").unwrap();
    fs::write(&target, "# First\n\n# Second\n").unwrap();
    app.open_file(source).unwrap();
    press(&mut app, '?');
    let rendered = app.active().buffer;
    assert!(app.active_buffer().markdown_render_source().is_some());
    let offset = text(&app).find("Chapter").unwrap();
    let view = app.prepare_view(geometry(80));
    let (column, row) = text_cell(&app, &view, app.active_pane, offset);
    click(&mut app, &view, column, row);
    assert_ne!(app.active().buffer, rendered);
    assert_eq!(app.active_buffer().path.as_ref(), Some(&target));
    assert_eq!(cursor(&app).row, 2);
}

#[test]
fn native_link_click_uses_wrapped_and_scrolled_text_and_rejects_blank_cells() {
    for wrap in [false, true] {
        let (_root, mut app, opened) = fixture();
        app.config.editor.line_numbers = false;
        app.config.editor.soft_wrap = wrap;
        seed(&mut app, "ab\t界 https://example.com/long/path");
        let head = if wrap { 16 } else { 2 };
        app.active_mut().replace_selection(Selection::point(head));
        app.active_mut().scroll_col = if wrap { 0 } else { 2 };
        app.active_mut().preserve_scroll = true;
        let view = app.prepare_view(geometry(24));
        let (column, row) = text_cell(&app, &view, app.active_pane, 16);
        click(&mut app, &view, column, row);
        assert_eq!(*opened.lock().unwrap(), ["https://example.com/long/path"]);
        let pane = view.pane(app.active_pane).unwrap();
        click(
            &mut app,
            &view,
            pane.body.x,
            pane.body.y + pane.body.height - 1,
        );
        assert_eq!(opened.lock().unwrap().len(), 1);
    }
}

#[test]
fn native_link_click_obeys_overlays_and_rejects_stale_panes() {
    let (root, mut app, opened) = fixture();
    seed(&mut app, "https://example.com");
    let view = app.prepare_view(geometry(80));
    let (column, row) = text_cell(&app, &view, app.active_pane, 3);
    press(&mut app, ':');
    click(&mut app, &view, column, row);
    assert_eq!(app.mode, Mode::Command);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    let path = root.join("other.txt");
    fs::write(&path, "https://wrong.example.com").unwrap();
    app.open_file(path).unwrap();
    click(&mut app, &view, column, row);
    assert!(opened.lock().unwrap().is_empty());
}

#[test]
fn native_link_click_terminal_links_preserves_live_or_frozen_state_and_owns_release() {
    for review in [false, true] {
        let (_root, mut app, opened) = fixture();
        let terminal = app.terminals.insert_test_session(20, 8);
        app.active_mut().terminal = Some(terminal);
        app.mode = Mode::Insert;
        app.apply_terminal_output(TerminalOutput::Bytes {
            id: terminal,
            bytes: b"\x1b[?1000h\x1b[?1006hhttps://example.com/long/path".to_vec(),
        });
        if review {
            let session = app.terminals.get_mut(terminal).unwrap();
            session.begin_review();
            session.select_all_review();
            app.apply_terminal_output(TerminalOutput::Bytes {
                id: terminal,
                bytes: b"\x1b[2J\x1b[Hhttps://wrong.example".to_vec(),
            });
        }
        app.mode = if review { Mode::Select } else { Mode::Insert };
        let view = app.prepare_view(geometry(22));
        let body = view.pane(app.active_pane).unwrap().body;
        let session = app.terminals.get(terminal).unwrap();
        let revision = session.revision();
        let generation = session.input_generation();
        let anchor = session.review_selection_anchor();
        // The continuation row resolves to the full URL, not just its suffix.
        click(&mut app, &view, body.x + 2, body.y + 1);
        assert_eq!(*opened.lock().unwrap(), ["https://example.com/long/path"]);
        assert_eq!(app.active_terminal(), Some(terminal));
        assert_eq!(app.mode, if review { Mode::Select } else { Mode::Insert });
        let session = app.terminals.get(terminal).unwrap();
        assert_eq!(session.reviewing(), review);
        assert_eq!(session.revision(), revision);
        assert_eq!(session.input_generation(), generation);
        assert_eq!(session.review_selection_anchor(), anchor);
        assert!(session.live());
        // Releasing Control before the mouse must not forward a stray release.
        app.handle_pointer(
            PointerEvent {
                kind: PointerEventKind::Down(PointerButton::Left),
                column: body.x + 2,
                row: body.y + 1,
                modifiers: link_modifier(),
            },
            &view,
        )
        .unwrap();
        app.handle_pointer(
            PointerEvent {
                kind: PointerEventKind::Up(PointerButton::Left),
                column: body.x + 2,
                row: body.y + 1,
                modifiers: Modifiers::NONE,
            },
            &view,
        )
        .unwrap();
        assert_eq!(
            app.terminals.get(terminal).unwrap().input_generation(),
            generation
        );
    }
}

#[test]
fn tui_ctrl_click_still_belongs_to_terminal_mouse_reporting() {
    let (_root, mut app, opened) = fixture();
    app.native_media = false;
    let terminal = app.terminals.insert_test_session(40, 8);
    app.active_mut().terminal = Some(terminal);
    app.mode = Mode::Insert;
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b[?1000h\x1b[?1006hhttps://example.com".to_vec(),
    });
    let view = app.prepare_view(geometry(42));
    let body = view.pane(app.active_pane).unwrap().body;
    click(&mut app, &view, body.x + 3, body.y);
    assert!(opened.lock().unwrap().is_empty());
    assert!(app.terminals.get(terminal).unwrap().input_generation() > 0);
}

#[test]
fn native_link_click_resolves_terminal_relative_paths_without_creating_review() {
    let (root, mut app, _) = fixture();
    let directory = root.join("terminal");
    fs::create_dir(&directory).unwrap();
    let target = directory.join("file.txt");
    fs::write(&target, "target").unwrap();
    let terminal = app.terminals.insert_test_session(40, 8);
    app.active_mut().terminal = Some(terminal);
    app.mode = Mode::Insert;
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: format!(
            "\x1b]7;{}\x07界 file.txt",
            crate::lsp::path_to_uri(&directory).unwrap().as_str()
        )
        .into_bytes(),
    });
    // OSC 7 uses a file URI, which drops the Windows verbatim path prefix.
    // Compare filesystem identities rather than the two path spellings.
    assert_eq!(
        app.terminals
            .get(terminal)
            .unwrap()
            .directory()
            .canonicalize()
            .unwrap(),
        directory.canonicalize().unwrap()
    );
    let view = app.prepare_view(geometry(42));
    let body = view.pane(app.active_pane).unwrap().body;
    click(&mut app, &view, body.x + 5, body.y);
    assert_eq!(
        app.active_buffer()
            .path
            .as_ref()
            .unwrap()
            .canonicalize()
            .unwrap(),
        target.canonicalize().unwrap()
    );
    assert!(app.active_terminal().is_none());
    assert_eq!(app.mode, Mode::Normal);
    let session = app.terminals.get(terminal).unwrap();
    assert!(!session.reviewing());
    assert!(session.live());
}

#[test]
fn extra_link_click_modifiers_remain_terminal_input() {
    let (_root, mut app, opened) = fixture();
    let terminal = app.terminals.insert_test_session(40, 8);
    app.active_mut().terminal = Some(terminal);
    app.mode = Mode::Insert;
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b[?1000h\x1b[?1006hhttps://example.com".to_vec(),
    });
    let view = app.prepare_view(geometry(42));
    let body = view.pane(app.active_pane).unwrap().body;
    for modifier in [Modifiers::SHIFT, Modifiers::ALT] {
        let generation = app.terminals.get(terminal).unwrap().input_generation();
        app.handle_pointer(
            PointerEvent {
                kind: PointerEventKind::Down(PointerButton::Left),
                column: body.x + 3,
                row: body.y,
                modifiers: link_modifier() | modifier,
            },
            &view,
        )
        .unwrap();
        assert!(app.terminals.get(terminal).unwrap().input_generation() > generation);
        assert!(opened.lock().unwrap().is_empty());
    }
}

#[test]
fn native_link_click_leaves_tree_focus_for_active_and_inactive_terminals() {
    for inactive in [false, true] {
        let (_root, mut app, opened) = fixture();
        let terminal = app.terminals.insert_test_session(40, 8);
        app.active_mut().terminal = Some(terminal);
        let pane = app.active_pane;
        app.mode = Mode::Insert;
        app.apply_terminal_output(TerminalOutput::Bytes {
            id: terminal,
            bytes: b"https://example.com/tree".to_vec(),
        });
        if inactive {
            app.panes.insert(1, Pane::new(0));
            app.layout = Layout::Split {
                axis: Axis::Horizontal,
                ratio: u16::MAX / 2,
                first: Box::new(Layout::Pane(pane)),
                second: Box::new(Layout::Pane(1)),
            };
            app.active_pane = 1;
            app.mode = Mode::Normal;
        }
        app.enter_directory_tree();
        assert!(app.directory_tree.focused);
        let view = app.prepare_view(geometry(120));
        let body = view.pane(pane).unwrap().body;
        app.terminals
            .get_mut(terminal)
            .unwrap()
            .resize(body.width as usize, body.height as usize);
        click(&mut app, &view, body.x + 8, body.y);
        assert_eq!(*opened.lock().unwrap(), ["https://example.com/tree"]);
        assert!(!app.directory_tree.focused);
        assert_eq!(app.active_pane, pane);
        assert_eq!(app.mode, Mode::Insert);
        assert!(!app.terminals.get(terminal).unwrap().reviewing());
    }
}

#[test]
fn native_link_click_reports_open_failures_and_runs_buffer_lifecycle() {
    let (root, mut app, _) = fixture();
    open_filler_special_buffers(&mut app, SPECIAL_BUFFER_RETENTION_LIMIT);
    let target = root.join("target.txt");
    fs::write(&target, "target").unwrap();
    app.open_virtual_page(
        GeneratedViewIdentity::Named("pointer-links".into()),
        "[links]".into(),
        "target.txt\n",
        ContentAlignment::default(),
    );
    assert!(app.closed_buffers.is_empty());
    let view = app.prepare_view(geometry(80));
    let (column, row) = text_cell(&app, &view, app.active_pane, 3);
    click(&mut app, &view, column, row);
    assert_eq!(app.active_buffer().path.as_ref(), Some(&target));
    assert!(
        !app.closed_buffers.is_empty(),
        "navigation retires old special buffers"
    );

    let source = root.join("failure.txt");
    fs::write(&source, "https://example.com/failure").unwrap();
    app.open_file(source).unwrap();
    app.ports.browser = Box::new(|_| bail!("test browser unavailable"));
    let view = app.prepare_view(geometry(80));
    let (column, row) = text_cell(&app, &view, app.active_pane, 3);
    // Errors are reported as a changed frame, not lost as pointer failures.
    click(&mut app, &view, column, row);
    assert_eq!(app.unread_notification_counts().errors, 1);
    assert!(app.status.contains("test browser unavailable"));
}
