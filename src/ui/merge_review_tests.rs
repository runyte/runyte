// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::HostPorts,
    clipboard::SystemClipboard,
    config::Config,
    snapshot::{OverlayAction, OverlayInput, OverlayPurpose, OverlayRow},
};
use ratatui::{Terminal, backend::TestBackend};

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        Ok(String::new())
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> (Fixture, App) {
    let root = std::env::temp_dir().join(format!(
        "runyte-merge-render-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let app =
        App::new_in_isolated_project(&root, HostPorts::isolated(Box::new(Clipboard))).unwrap();
    (Fixture(root), app)
}
fn row(identity: String, label: String) -> OverlayRow {
    OverlayRow {
        heading: false,
        identity: identity.into(),
        label,
        detail: String::new(),
        trailing_detail: String::new(),
        available: true,
        dimmed: false,
        muted: vec![],
        emphasis: vec![],
        detail_emphasis: vec![],
        tints: vec![],
        elide_from: None,
    }
}
fn review(count: usize) -> OverlaySnapshot {
    let mut rows = (0..count)
        .map(|i| row(format!("body:{i}"), format!("file-{i:03}.rs")))
        .collect::<Vec<_>>();
    rows.push(row("approve".into(), "Approve".into()));
    rows.push(row("cancel".into(), "Cancel".into()));
    OverlaySnapshot {
        kind: OverlayKind::GitMergeReview, layout: OverlayLayout::GitMergeReview,
        purpose: OverlayPurpose::Confirmation, input: OverlayInput::None,
        actions: vec![OverlayAction::new("Enter", "inspect / activate"), OverlayAction::new("Esc", "cancel")],
        legend: vec![], title: "Merge topic into main".into(),
        query: String::new(), query_placeholder: "type exact destination branch".into(),
        column_header: None, rows, selected: Some(count + 1), scroll_anchor: None,
        row_offset: 0, message: Some("Current: main abc12345 · Other: topic def67890\nClean merge awaiting an explicit commit\n80 changed files · No conflicts".into()),
        omitted_rows: 0, total_rows: count + 2, query_cursor: None,
        show_preview: false, preview_title: None, preview: None,
    }
}
fn draw(
    app: &mut App,
    theme: &TuiTheme,
    overlay: &OverlaySnapshot,
    width: u16,
    height: u16,
) -> (ratatui::buffer::Buffer, ScreenPosition, Rect) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut editor = Rect::default();
    terminal
        .draw(|frame| {
            let prepared = app.prepare_view(frame_geometry(frame.area()));
            let snapshot = app.snapshot(&prepared);
            editor = snapshot.geometry.editor;
            draw_snapshot_overlay(frame, theme, overlay, &snapshot);
        })
        .unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    (terminal.backend().buffer().clone(), cursor, editor)
}
fn find(buffer: &ratatui::buffer::Buffer, text: &str) -> Option<(u16, u16)> {
    for y in 0..buffer.area.height {
        let line = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        if let Some(byte) = line.find(text) {
            return Some((line[..byte].width() as u16, y));
        }
    }
    None
}

#[test]
fn merge_review_cancel_keeps_initial_body_top_and_both_actions_pinned() {
    let (_fixture, mut app) = fixture();
    let theme = TuiTheme::new(&app.theme);
    let overlay = review(80);
    let (buffer, _, editor) = draw(&mut app, &theme, &overlay, 100, 24);
    let view = crate::merge_review_layout::merge_review_layout(
        editor,
        overlay.message.as_deref().unwrap(),
        false,
    );
    assert_eq!(find(&buffer, "file-000.rs").unwrap().1, view.body.y);
    assert!(find(&buffer, "Current: main").is_some());
    assert!(find(&buffer, "Other: topic").is_some());
    assert_eq!(find(&buffer, "Approve").unwrap().1, view.footer.y);
    let (x, y) = find(&buffer, "Cancel").unwrap();
    assert_eq!(y, view.footer.y);
    assert_eq!(buffer[(x, y)].bg, theme.selection);
    assert!(find(&buffer, "file-079.rs").is_none());
}

#[test]
fn merge_review_body_focus_and_captured_offset_do_not_scroll_footer() {
    let (_fixture, mut app) = fixture();
    let theme = TuiTheme::new(&app.theme);
    let mut overlay = review(80);
    overlay.row_offset = 10;
    for (i, row) in overlay.rows.iter_mut().take(80).enumerate() {
        row.label = format!("file-{:03}.rs", i + 10);
    }
    overlay.total_rows = 92;
    overlay.selected = Some(39);
    let (buffer, _, editor) = draw(&mut app, &theme, &overlay, 100, 24);
    let view = crate::merge_review_layout::merge_review_layout(
        editor,
        overlay.message.as_deref().unwrap(),
        false,
    );
    let (x, y) = find(&buffer, "file-049.rs").unwrap();
    assert_eq!(y, view.body.y + view.body.height - 1);
    assert_eq!(buffer[(x, y)].bg, theme.selection);
    assert_eq!(find(&buffer, "Approve").unwrap().1, view.footer.y);
    assert_eq!(find(&buffer, "Cancel").unwrap().1, view.footer.y);
}

#[test]
fn merge_review_narrow_terminal_stacks_complete_footer_labels_and_reserves_input() {
    let (_fixture, mut app) = fixture();
    let theme = TuiTheme::new(&app.theme);
    let mut overlay = review(80);
    overlay.rows[80].label = "[A]pprove merge".into();
    overlay.rows[81].label = "[C]ancel merge".into();
    overlay.rows[80].available = false;
    overlay.input = OverlayInput::Text;
    overlay.query = "main".into();
    for selected in [Some(80), Some(81), None] {
        overlay.selected = selected;
        overlay.query_cursor = selected.is_none().then_some(4);
        let (buffer, cursor, editor) = draw(&mut app, &theme, &overlay, 28, 12);
        let view = crate::merge_review_layout::merge_review_layout(
            editor,
            overlay.message.as_deref().unwrap(),
            true,
        );
        assert!(view.stacked_footer);
        let (approve_x, approve_y) = find(&buffer, "[A]pprove merge").unwrap();
        let (cancel_x, cancel_y) = find(&buffer, "[C]ancel merge").unwrap();
        assert_eq!(approve_y, view.footer.y);
        assert_eq!(cancel_y, approve_y + 1);
        assert_eq!(buffer[(approve_x, approve_y)].fg, theme.muted);
        assert!(
            buffer[(approve_x, approve_y)]
                .modifier
                .contains(Modifier::DIM)
        );
        if selected == Some(80) {
            assert_eq!(buffer[(approve_x, approve_y)].bg, theme.selection);
        }
        if selected == Some(81) {
            assert_eq!(buffer[(cancel_x, cancel_y)].bg, theme.selection);
        }
        let query_line = (0..buffer.area.width)
            .map(|x| buffer[(x, view.query.y)].symbol())
            .collect::<String>();
        assert!(query_line.contains("main"));
        if selected.is_none() {
            assert_eq!(cursor.y, view.query.y);
        }
    }
}

#[test]
fn merge_review_details_keep_back_cancel_and_report_clipped_lines_after_wire_round_trip() {
    let (_fixture, mut app) = fixture();
    let theme = TuiTheme::new(&app.theme);
    let mut overlay = review(80);
    overlay.rows[0].label = "李小龍 e\u{301} 👩‍💻 ".repeat(40);
    overlay.rows[80] = row("back".into(), "Back".into());
    overlay.selected = Some(80);
    overlay.row_offset = 200;
    overlay.total_rows = 500;
    let prepared = app.prepare_view(frame_geometry(TuiRect::new(0, 0, 100, 24)));
    let wire = crate::protocol::HostFrame {
        id: crate::protocol::FrameId::from_raw(1),
        active_buffer: crate::workspace::BufferId::from_raw(0).into(),
        active_revision: crate::workspace::BufferRevision::from_raw(0).into(),
        editor: app.snapshot(&prepared).into(),
        overlays: vec![overlay.clone().into()],
    };
    let mut decoded =
        serde_json::from_slice::<crate::protocol::HostFrame>(&serde_json::to_vec(&wire).unwrap())
            .unwrap();
    let restored: OverlaySnapshot = decoded.overlays.pop().unwrap().try_into().unwrap();
    assert_eq!(restored, overlay);
    let (buffer, _, _) = draw(&mut app, &theme, &restored, 100, 24);
    assert!(find(&buffer, "Back").is_some());
    assert!(find(&buffer, "Cancel").is_some());
    assert!(find(&buffer, "Approve").is_none());
    assert!(find(&buffer, "… clips long lines").is_some());
    assert!(merge_review_label(&overlay.rows[0].label, 20).ends_with('…'));
    assert!(merge_review_label(&overlay.rows[0].label, 20).width() <= 20);
    assert!(!merge_review_label("👩‍💻", 1).contains('👩'));
}

#[test]
fn merge_review_resizes_and_long_unicode_acknowledgment_keep_cursor_in_query() {
    let (_fixture, mut app) = fixture();
    let theme = TuiTheme::new(&app.theme);
    let mut overlay = review(80);
    overlay.input = OverlayInput::Text;
    overlay.query = "界".repeat(80);
    overlay.query_cursor = Some(80);
    overlay.selected = None;
    for (width, height) in [(100, 24), (28, 12), (8, 8), (3, 3), (1, 1), (0, 0)] {
        let (buffer, cursor, editor) = draw(&mut app, &theme, &overlay, width, height);
        let view = crate::merge_review_layout::merge_review_layout(
            editor,
            overlay.message.as_deref().unwrap(),
            true,
        );
        assert_eq!(buffer.area.width, width);
        if editor.width >= 3 && editor.height >= 3 && view.query.height > 0 {
            assert_eq!(cursor.y, view.query.y);
            assert!(cursor.x >= view.query.x && cursor.x < view.query.x + view.query.width);
        }
    }
}

#[test]
fn merge_review_selection_and_disabled_actions_remain_distinct_in_light_and_dark_themes() {
    let (_fixture, mut app) = fixture();
    let mut overlay = review(80);
    overlay.rows[80].available = false;
    for name in ["light", "dark"] {
        let theme = TuiTheme::new(&Config::default().resolve_theme(name).unwrap());
        let (buffer, _, _) = draw(&mut app, &theme, &overlay, 100, 24);
        let (x, y) = find(&buffer, "Approve").unwrap();
        assert_eq!(buffer[(x, y)].fg, theme.muted);
        assert!(buffer[(x, y)].modifier.contains(Modifier::DIM));
        let (x, y) = find(&buffer, "Cancel").unwrap();
        assert_eq!(buffer[(x, y)].bg, theme.selection);
        assert_eq!(buffer[(x, y)].fg, theme.foreground);
        assert_ne!(theme.selection, theme.overlay_background);
        let (x, y) = find(&buffer, "file-000.rs").unwrap();
        assert_eq!(buffer[(x, y)].bg, theme.overlay_background);
    }
}
