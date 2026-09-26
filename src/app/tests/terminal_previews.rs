// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::snapshot::{OverlayKind, OverlayPreview};

fn preview(app: &App, kind: OverlayKind) -> crate::terminal::TerminalView {
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|o| o.kind == kind)
        .unwrap();
    let Some(OverlayPreview::Terminal(view)) = overlay.preview else {
        panic!("expected a live terminal preview");
    };
    view
}

fn screen(view: &crate::terminal::TerminalView) -> String {
    view.rows
        .iter()
        .map(|row| row.iter().map(|cell| cell.text()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_rendered_preview(app: &mut App, needle: &str, colored: bool) {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    let prepared = app.prepare_view(crate::ui::frame_geometry(ratatui::layout::Rect::new(
        0, 0, 120, 30,
    )));
    let editor = app.snapshot(&prepared);
    let frame = crate::workspace::HostFrame {
        id: crate::workspace::FrameId::from_raw(1),
        active_buffer: crate::workspace::BufferId::from_raw(1),
        active_revision: crate::workspace::BufferRevision::from_raw(1),
        editor: editor.clone(),
        overlays: app.overlay_snapshots(),
    };
    let wire = crate::protocol::HostFrame::from(frame);
    let encoded = serde_json::to_vec(&wire).unwrap();
    let decoded: crate::protocol::HostFrame = serde_json::from_slice(&encoded).unwrap();
    let attached = crate::workspace::HostFrame::try_from(decoded).unwrap();
    for persistent in [false, true] {
        terminal
            .draw(|frame| {
                if persistent {
                    crate::ui::render_host_frame_exact_colors_for_test(frame, &attached);
                } else {
                    crate::ui::render_exact_colors_for_test(
                        frame,
                        app,
                        &editor,
                        &crate::key_hints::KeyHintState::default(),
                    );
                }
            })
            .unwrap();
        let cells = &terminal.backend().buffer().content;
        let text = cells.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(
            text.contains(needle),
            "preview missing (persistent={persistent})"
        );
        if colored {
            assert!(
                cells.iter().any(|cell| cell.symbol() == "l"
                    && cell.fg == ratatui::style::Color::Rgb(12, 34, 56))
            );
        }
    }
}

#[test]
fn terminal_list_previews_follow_output_and_selection_without_changing_panes() {
    let root = temporary("live-terminal-previews");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new(Config::default(), Some(root.clone())).unwrap();
    let mut ids = Vec::new();
    for name in ["preview-target-alpha", "preview-target-beta"] {
        app.open_terminal_at(Some(terminal_fixture_command()), root.clone());
        let id = app.active_terminal().unwrap();
        app.terminals
            .get_mut(id)
            .unwrap()
            .rename(Some(name.into()))
            .unwrap();
        app.apply_terminal_output(TerminalOutput::Bytes {
            id,
            bytes: format!("\x1b[31m{name} old\x1b[0m").into_bytes(),
        });
        ids.push(id);
    }
    app.leave_terminal();
    for navigator in [true, false] {
        if navigator {
            app.open_navigator();
        } else {
            app.open_terminal_list();
        }
        app.list.as_mut().unwrap().show_preview = true;
        type_text(&mut app, "preview-target"); // Both terminal names match.
        let order = app
            .list
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|item| item.index)
            .collect::<Vec<_>>();
        key(&mut app, KeyCode::Home, Modifiers::NONE);
        let mut visited = HashSet::new();
        for step in 0..2 {
            let id = match app.selected_list_action().unwrap() {
                ListAction::Destination(OpenDestination::Terminal(id)) => id,
                other => panic!("unexpected selected action: {other:?}"),
            };
            visited.insert(id);
            let session = app.terminals.get_mut(id).unwrap();
            session.begin_review();
            let review = session.view(4);
            let columns = session.columns();
            let before = preview(&app, OverlayKind::ResultList);
            let label = format!("live {} {navigator} {step}", id.get());
            app.apply_terminal_output(TerminalOutput::Bytes {
                id,
                bytes: format!("\r\x1b[2K\x1b[38;2;12;34;56m{label}\x1b[0m").into_bytes(),
            });
            let after = preview(&app, OverlayKind::ResultList);
            assert_ne!(before, after);
            assert!(screen(&after).contains(&label));
            assert!(!after.review);
            assert_eq!(
                after.rows[0][0].foreground,
                crate::terminal::Color::Rgb(12, 34, 56)
            );
            assert_eq!(app.terminals.get(id).unwrap().columns(), columns);
            assert_eq!(app.terminals.get(id).unwrap().view(4).rows, review.rows);
            assert_eq!(app.active_terminal(), None);
            assert_rendered_preview(&mut app, &label, true);
            assert_eq!(app.list.as_ref().unwrap().filter, "preview-target");
            assert_eq!(
                app.list
                    .as_ref()
                    .unwrap()
                    .items
                    .iter()
                    .map(|item| item.index)
                    .collect::<Vec<_>>(),
                order
            );

            key(&mut app, KeyCode::Down, Modifiers::NONE);
        }
        assert_eq!(visited, ids.iter().copied().collect());
        key(&mut app, KeyCode::Escape, Modifiers::NONE);
    }
    close_test_terminals(&mut app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn name_finder_terminal_preview_updates_before_the_content_refresh_tick() {
    let root = temporary("live-finder-terminal-preview");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new(Config::default(), Some(root.clone())).unwrap();
    app.open_terminal_at(Some(terminal_fixture_command()), root.clone());
    let id = app.active_terminal().unwrap();
    app.terminals
        .get_mut(id)
        .unwrap()
        .rename(Some("unique-terminal".into()))
        .unwrap();
    app.leave_terminal();
    app.open_project_picker().unwrap();
    type_text(&mut app, "unique-terminal");
    let before = preview(&app, OverlayKind::FilePicker);
    app.apply_terminal_output(TerminalOutput::Bytes {
        id,
        bytes: b"latest output".to_vec(),
    });
    let after = preview(&app, OverlayKind::FilePicker);
    assert_ne!(before, after);
    assert!(screen(&after).contains("latest output"));
    assert_rendered_preview(&mut app, "latest output", false);
    assert_eq!(app.active_terminal(), None);
    app.close_file_picker();
    close_test_terminals(&mut app);
    fs::remove_dir_all(root).unwrap();
}
