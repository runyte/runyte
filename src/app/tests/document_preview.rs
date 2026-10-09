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
