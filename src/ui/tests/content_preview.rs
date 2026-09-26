// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn content_preview_fills_overlay_and_keeps_match_visible_across_resizes() {
    use crate::file_picker::{FileHits, FilePicker, FilePreview, LineHit, ScanScope};
    use crate::finder::{FinderMode, ResourceFinder, ResourceItem, ResourceKind, ResourceTarget};

    let text = (0..1000)
        .map(|row| format!("context-row-{row:04}"))
        .collect::<Vec<_>>()
        .join("\n");
    for focus in [0, 500, 999] {
        for source in ["files", "finder-file", "finder-buffer"] {
            let mut app = App::new(Config::default(), None).unwrap();
            let root = std::path::PathBuf::from("/project");
            let mut picker = FilePicker::grep(1, root.clone(), ScanScope::ignoring(&root));
            picker.add_content(vec![FileHits {
                path: root.join("notes.txt"),
                lines: vec![LineHit {
                    row: focus,
                    column: 0,
                    text: format!("context-row-{focus:04}"),
                }],
            }]);
            picker.finish(0, false);
            picker.preview = Some(FilePreview::snippet_from_text(&text, focus, vec![0]));
            if source != "files" {
                let mut finder = ResourceFinder::new(FinderMode::Contents);
                if source == "finder-buffer" {
                    finder.replace_items(
                        vec![ResourceItem::new(
                            "notes",
                            "",
                            ResourceTarget::Buffer(0),
                            ResourceKind::Buffer,
                            ["notes".to_owned()],
                        )],
                        &picker,
                        "",
                    );
                    finder.selected = finder.matches.len() - 1;
                    assert!(finder.selected_item().is_some());
                    finder.set_selected_preview(picker.preview.clone());
                } else {
                    finder.merge_files(&picker, "");
                }
                app.finder = Some(finder);
            }
            app.picker = Some(picker);
            let theme = TuiTheme::new(&app.theme);
            for height in [20, 64, 12, 40] {
                for attached in [false, true] {
                    let mut terminal = Terminal::new(TestBackend::new(160, height)).unwrap();
                    terminal
                        .draw(|frame| {
                            let prepared = app.prepare_view(frame_geometry(frame.area()));
                            let snapshot = app.snapshot(&prepared);
                            if attached {
                                let overlay = app
                                    .overlay_snapshots()
                                    .into_iter()
                                    .find(|overlay| overlay.kind == OverlayKind::FilePicker)
                                    .unwrap();
                                draw_snapshot_overlay(frame, &theme, &overlay, &snapshot);
                            } else {
                                render_exact_colors_for_test(
                                    frame,
                                    &app,
                                    &snapshot,
                                    &KeyHintState::default(),
                                );
                            }
                        })
                        .unwrap();
                    let screen = terminal
                        .backend()
                        .buffer()
                        .content
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>();
                    let area = centered(
                        frame_geometry(TuiRect::new(0, 0, 160, height)).editor,
                        90,
                        85,
                        28,
                        8,
                    );
                    let expected = usize::from(area.height.saturating_sub(4));
                    assert_eq!(
                        screen.matches("│ context-row-").count(),
                        expected,
                        "height={height}, focus={focus}, source={source}, attached={attached}: {screen}"
                    );
                    assert!(
                        (1..=4).any(|digits| screen.contains(&format!(
                            "› {:>digits$} │ context-row-{focus:04}",
                            focus + 1
                        ))),
                        "{screen}"
                    );
                }
            }
        }
    }
}

#[test]
fn content_preview_handles_empty_and_single_row_viewports() {
    let app = App::new(Config::default(), None).unwrap();
    let theme = TuiTheme::new(&app.theme);
    let lines = vec!["before".to_owned(), "match".to_owned(), "after".to_owned()];
    assert!(fuzzy_preview_lines("m", &lines, 10, 11, &[0], &theme, 0).is_empty());
    let visible = fuzzy_preview_lines("m", &lines, 10, 11, &[0], &theme, 1);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].to_string(), "› 12 │ match");
    assert_eq!(
        visible[0].spans[1].style.bg,
        Some(theme.fuzzy_match_primary)
    );
}
