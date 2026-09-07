// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{config::Config, selection::Selection, syntax::DocumentSyntax, text::Transaction};

#[test]
fn deep_wrapped_unicode_rows_preserve_text_and_syntax_after_resizing() {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    let source = format!(
        "[{}null]",
        "{\"name\":\"λ界e\u{301}\",\"id\":123},".repeat(1000)
    );
    app.buffers[0].apply(&Transaction::insert(0, source));
    let language = app.registry.language_for_name("json").unwrap();
    app.syntax[0] = DocumentSyntax::new(app.buffers[0].text(), language, &app.registry);
    let length = app.buffers[0].text().len_chars();
    let highlights = app.highlights(0, 0, length);
    assert!(!highlights.is_empty());
    for width in [40, 73, 40] {
        app.panes.get_mut(&0).unwrap().selection = Selection::point(length - 400);
        let screen = Rect {
            width,
            height: 12,
            ..Rect::default()
        };
        let prepared = app.prepare_view(crate::app::FrameGeometry {
            screen,
            editor: screen,
            status: Rect::default(),
            message: Rect::default(),
        });
        let snapshot = app.snapshot(&prepared);
        let pane = &snapshot.panes[0];
        assert!(pane.scroll_wrap > 100);
        let spans = crate::wrap::segments(&app.buffers[0].line_string(0), pane.wrap_width, 4);
        for (index, row) in pane.rows.iter().enumerate() {
            let SnapshotRow::Text(row) = row else {
                continue;
            };
            let segment = spans[pane.scroll_wrap + index];
            let expected = app.buffers[0]
                .text()
                .slice_string(segment.start, segment.end);
            let drawn = row
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>();
            assert_eq!(drawn, expected);
            let mut offset = segment.start;
            for run in &row.runs {
                let TextRunKind::Text { scope, .. } = run.kind else {
                    panic!("expected document text")
                };
                for _ in run.text.chars() {
                    let expected_scope = highlights
                        .iter()
                        .find(|span| span.from <= offset && offset < span.to)
                        .map(|span| span.scope);
                    assert_eq!(scope, expected_scope, "scope at {offset}");
                    offset += 1;
                }
            }
        }
    }
}
