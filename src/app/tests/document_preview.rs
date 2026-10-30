// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::test_support::TestRuntimeRoot;
#[test]
fn preview_captures_unsaved_text_without_changing_source_or_selection() {
    let root = TestRuntimeRoot::new("document-preview").unwrap();
    let path = root.join("notes.md");
    fs::write(&path, "# Saved\n").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path.clone()).unwrap();
    press(&mut app, 'i');
    press(&mut app, 'X');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    let source = app.active().buffer;
    let selection = app.active().selection.clone();
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let capture = &app.document_previews[&app.active_pane];
    assert_eq!(capture.source, source);
    assert_eq!(capture.text, app.buffers[source].to_string());
    assert_ne!(capture.text, fs::read_to_string(path).unwrap());
    assert_eq!(capture.language, "markdown");
    assert_eq!(app.active().selection, selection);
    assert_eq!(app.active().buffer, source);
    let generation = capture.generation;
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert_eq!(app.document_previews.len(), 1);
    assert!(app.document_previews[&app.active_pane].generation > generation);
    assert!(app.active_buffer().dirty);
}
#[test]
fn preview_dispatches_known_and_unknown_text_and_rejects_terminal_frontend() {
    let root = TestRuntimeRoot::new("preview-dispatch").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert!(app.document_previews.is_empty());
    assert_eq!(
        app.status,
        ":preview needs the Runyte window: use the desktop edition with --window"
    );
    app.native_media = true;
    for (name, expected) in [
        ("a.html", "html"),
        ("a.json", "json"),
        ("a.yaml", "yaml"),
        ("a.rs", "rust"),
        ("a.txt", "text"),
        ("a.svg", "svg"),
    ] {
        let path = root.join(name);
        fs::write(&path, "sample text").unwrap();
        app.open_file(path).unwrap();
        app.execute(parse_colon_command("preview").unwrap())
            .unwrap();
        assert_eq!(app.document_previews[&app.active_pane].language, expected);
    }
}
#[test]
fn preview_scratch_and_limit_are_bounded() {
    let root = TestRuntimeRoot::new("preview-scratch").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert!(app.document_previews[&app.active_pane].path.is_none());
    let path = root.join("large.txt");
    fs::write(&path, "x".repeat(crate::document_preview::MAX_BYTES + 1)).unwrap();
    app.open_file(path).unwrap();
    let generation = app.preview_generation;
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert_eq!(app.preview_generation, generation);
}

#[test]
fn preview_uses_explicit_ranges_and_allows_small_selection_in_large_buffer() {
    let root = TestRuntimeRoot::new("preview-selection").unwrap();
    let path = root.join("sections.md");
    fs::write(
        &path,
        format!(
            "# One\n\n# Two\n\n{}",
            "x".repeat(crate::document_preview::MAX_BYTES)
        ),
    )
    .unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path).unwrap();
    app.active_mut()
        .replace_selection(Selection::new(vec![Range::new(12, 7)], 0));
    let selection = app.active().selection.clone();
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let preview = &app.document_previews[&app.active_pane];
    assert_eq!(preview.text, "# Two");
    assert!(preview.selection_only);
    assert_eq!(app.active().selection, selection);
    app.active_mut().replace_selection(Selection::new(
        vec![Range::new(0, 5), Range::point(6), Range::new(7, 12)],
        0,
    ));
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert_eq!(app.document_previews[&app.active_pane].text, "# One\n# Two");
    assert!(!app.active_buffer().dirty);
}

#[test]
fn preview_selection_matches_yank_inclusive_and_pointer_half_open_semantics() {
    let root = TestRuntimeRoot::new("preview-selection-edges").unwrap();
    let path = root.join("range.txt");
    fs::write(&path, "abcdef").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path).unwrap();
    app.active_mut()
        .replace_selection(Selection::single(Range::new(1, 3)));
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert_eq!(app.document_previews[&app.active_pane].text, "bcd");
    app.active_mut()
        .mark_selection_semantics(SelectionSemantics::HalfOpen);
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert_eq!(app.document_previews[&app.active_pane].text, "bc");
    app.active_mut().replace_selection(Selection::point(2));
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert_eq!(app.document_previews[&app.active_pane].text, "abcdef");
    let pane = app.active_pane;
    app.execute(parse_colon_command("vsplit").unwrap()).unwrap();
    assert!(!app.document_previews.contains_key(&app.active_pane));
    assert!(app.document_previews.contains_key(&pane));
    let buffer_count = app.buffers.len();
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert_eq!(app.buffers.len(), buffer_count);
    assert_eq!(app.document_previews.len(), 2);
}

#[test]
fn dismissing_a_preview_removes_host_capture_without_changing_source() {
    let root = TestRuntimeRoot::new("preview-dismiss").unwrap();
    let path = root.join("notes.md");
    fs::write(&path, "# Source\n").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path).unwrap();
    press(&mut app, 'i');
    press(&mut app, 'X');
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    let pane = app.active_pane;
    let source = app.active().buffer;
    let text = app.active_buffer().to_string();
    let selection = app.active().selection.clone();
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let first = app.document_previews[&pane].generation;
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let current = app.document_previews[&pane].generation;
    assert!(
        !app.dismiss_document_preview(pane, first),
        "a stale frontend cannot dismiss a refreshed capture"
    );
    assert!(!app.dismiss_document_preview(usize::MAX, current));
    assert!(app.dismiss_document_preview(pane, current));
    assert!(
        !app.dismiss_document_preview(pane, current),
        "dismissal is idempotent"
    );
    // Reattaching a native frontend uses this same retained host state.
    app.native_media = false;
    app.native_media = true;
    assert!(app.document_previews.is_empty());
    assert_eq!(app.active().buffer, source);
    assert_eq!(app.active_buffer().to_string(), text);
    assert_eq!(app.active().selection, selection);
    assert!(app.active_buffer().dirty);
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    assert!(app.document_previews[&pane].generation > current);
    assert!(!app.dismiss_document_preview(pane, current));
}
#[test]
fn preview_theme_setting_reaches_frames_live_without_recapturing() {
    use crate::settings::{SettingId, SettingValue};
    let root = TestRuntimeRoot::new("preview-theme").unwrap();
    let path = root.join("notes.md");
    fs::write(&path, "# Notes\n").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    app.open_file(path).unwrap();
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let capture = app.document_previews[&app.active_pane].clone();
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let geometry = crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 80, 24));
    assert!(host.prepare_frame(geometry).editor.preview_theme_colors);
    for enabled in [false, true] {
        SettingId::EditorPreviewThemeColors
            .apply(&SettingValue::Boolean(enabled), &mut host.config)
            .unwrap();
        let frame = host.prepare_frame(geometry);
        assert_eq!(frame.editor.preview_theme_colors, enabled);
        assert_eq!(frame.editor.panes[0].preview.as_ref(), Some(&capture));
    }
}

fn preview_test_geometry() -> FrameGeometry {
    crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 100, 32))
}

#[test]
fn clicking_a_native_preview_focuses_without_changing_source_selection() {
    let root = TestRuntimeRoot::new("preview-focus").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    seed(&mut app, "one two three\n");
    app.active_mut()
        .replace_selection(Selection::single(Range::new(1, 4)));
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let pane = app.active_pane;
    let selection = app.active().selection.clone();
    let capture = app.document_previews[&pane].clone();
    app.execute(parse_colon_command("vsplit").unwrap()).unwrap();
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let frame = host.prepare_frame(preview_test_geometry());
    let body = frame
        .editor
        .panes
        .iter()
        .find(|p| p.pane_id == pane)
        .unwrap()
        .body;
    let event = PointerEvent {
        kind: PointerEventKind::Down(PointerButton::Left),
        column: body.x + 6,
        row: body.y,
        modifiers: Modifiers::NONE,
    };
    host.execute(crate::workspace::HostCommand::Pointer {
        event,
        frame: frame.id,
        repetitions: 1,
    })
    .unwrap();
    assert_eq!(host.active_pane, pane);
    assert_eq!(host.active().selection, selection);
    assert!(host.pointer_drag.is_none());
    assert_eq!(host.document_previews[&pane], capture);
    let frame = host.prepare_frame(preview_test_geometry());
    assert_eq!(
        frame
            .editor
            .panes
            .iter()
            .find(|p| p.pane_id == pane)
            .unwrap()
            .preview
            .as_ref(),
        Some(&capture)
    );

    // A terminal attachment retains the capture but displays editable source.
    host.native_media = false;
    host.execute(crate::workspace::HostCommand::Pointer {
        event,
        frame: frame.id,
        repetitions: 1,
    })
    .unwrap();
    assert_ne!(host.active().selection, selection);
}

#[test]
fn keyboard_and_pointer_swaps_keep_each_preview_with_its_source() {
    for pointer_swap in [false, true] {
        for captures in [0, 1, 2, 3] {
            let root = TestRuntimeRoot::new("preview-swap").unwrap();
            let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
            app.native_media = true;
            let mut ids = Vec::new();
            for (index, text) in ["first source", "second source"].iter().enumerate() {
                if index == 1 {
                    app.execute(parse_colon_command("vsplit").unwrap()).unwrap();
                }
                let path = root.join(format!("{index}.txt"));
                fs::write(&path, text).unwrap();
                app.open_file(path).unwrap();
                ids.push(app.active_pane);
                if captures & (1 << index) != 0 {
                    app.execute(parse_colon_command("preview").unwrap())
                        .unwrap();
                }
            }
            let before = app.document_previews.clone();
            let mut host = crate::workspace::WorkspaceHost::new(app);
            if pointer_swap {
                let frame = host.prepare_frame(preview_test_geometry());
                let middle = |id| {
                    let body = frame
                        .editor
                        .panes
                        .iter()
                        .find(|p| p.pane_id == id)
                        .unwrap()
                        .body;
                    (body.x + body.width / 2, body.y + body.height / 2)
                };
                let start = middle(ids[1]);
                let target = middle(ids[0]);
                for (kind, (column, row)) in [
                    (PointerEventKind::Down(PointerButton::Left), start),
                    (PointerEventKind::Drag(PointerButton::Left), target),
                    (PointerEventKind::Up(PointerButton::Left), target),
                ] {
                    let frame = host.prepare_frame(preview_test_geometry());
                    host.execute(crate::workspace::HostCommand::Pointer {
                        event: PointerEvent {
                            kind,
                            column,
                            row,
                            modifiers: Modifiers::ALT,
                        },
                        frame: frame.id,
                        repetitions: 1,
                    })
                    .unwrap();
                }
            } else {
                key(&mut host, KeyCode::Char('w'), Modifiers::CONTROL);
                press(&mut host, 'x');
            }
            assert_eq!(host.active_pane, ids[0]);
            let frame = host.prepare_frame(preview_test_geometry());
            for (destination, source) in [(ids[0], ids[1]), (ids[1], ids[0])] {
                let pane = frame
                    .editor
                    .panes
                    .iter()
                    .find(|p| p.pane_id == destination)
                    .unwrap();
                assert_eq!(pane.preview.as_ref(), before.get(&source));
                assert_eq!(
                    host.document_previews.get(&destination),
                    before.get(&source)
                );
            }
        }
    }
}

#[test]
fn preview_frames_publish_current_structural_input_ownership() {
    let root = TestRuntimeRoot::new("preview-structural").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    seed(&mut app, "abcx");
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let geometry = preview_test_geometry();
    assert!(!host.prepare_frame(geometry).editor.preview_structural);
    press(&mut host, 'i');
    assert!(host.prepare_frame(geometry).editor.preview_structural);
    key(&mut host, KeyCode::Escape, Modifiers::NONE);
    assert!(!host.prepare_frame(geometry).editor.preview_structural);
    press(&mut host, 'f');
    assert!(host.prepare_frame(geometry).editor.preview_structural);
    press(&mut host, 'x');
    assert!(!host.prepare_frame(geometry).editor.preview_structural);
}

#[test]
fn remapped_preview_focus_is_independent_of_editor_selection_and_link_bindings() {
    let root = TestRuntimeRoot::new("preview-remapped-focus").unwrap();
    let mut app = App::new_in_project(Config::default(), None, &*root).unwrap();
    app.native_media = true;
    seed(&mut app, "https://example.com/preview\n");
    app.active_mut()
        .replace_selection(Selection::single(Range::new(1, 4)));
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let pane = app.active_pane;
    let selection = app.active().selection.clone();
    app.execute(parse_colon_command("vsplit").unwrap()).unwrap();
    let section = serde_yaml::from_str("pointer: {preview-select: [MiddleDrag], select: [], extend: [], open-link: [MiddleClick], copy: [MiddleClick]}").unwrap();
    let compiled = crate::keymap::configured::compile(&section, crate::keymap::default_keymap());
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    app.set_keymap(compiled.keymap);
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let frame = host.prepare_frame(preview_test_geometry());
    let body = frame
        .editor
        .panes
        .iter()
        .find(|p| p.pane_id == pane)
        .unwrap()
        .body;
    host.execute(crate::workspace::HostCommand::Pointer {
        event: PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Middle),
            column: body.x + 6,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        frame: frame.id,
        repetitions: 1,
    })
    .unwrap();
    assert_eq!(host.active_pane, pane);
    assert_eq!(host.active().selection, selection);
    assert!(host.pointer_drag.is_none());
    assert!(host.pointer_link_pressed.is_none());
}

#[test]
fn factory_preview_right_click_retains_source_selection_copy() {
    let root = TestRuntimeRoot::new("preview-copy").unwrap();
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new_in_project(config, None, &*root).unwrap();
    let clipboard = Arc::new(Mutex::new(String::new()));
    app.set_system_clipboard(Box::new(super::editing_and_buffers::MemoryClipboard(
        clipboard.clone(),
    )));
    app.native_media = true;
    seed(&mut app, "alpha beta");
    let selection = Selection::single(Range::new(0, 4));
    app.active_mut().replace_selection(selection.clone());
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let frame = host.prepare_frame(preview_test_geometry());
    let body = frame
        .editor
        .panes
        .iter()
        .find(|p| p.pane_id == host.active_pane)
        .unwrap()
        .body;
    host.execute(crate::workspace::HostCommand::Pointer {
        event: PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Right),
            column: body.x + 2,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        frame: frame.id,
        repetitions: 1,
    })
    .unwrap();
    assert_eq!(&*clipboard.lock().unwrap(), "alpha");
    assert_eq!(host.active().selection, selection);
}
