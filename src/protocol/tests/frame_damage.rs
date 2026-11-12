// SPDX-License-Identifier: MPL-2.0

use super::tests::{editor_frame, terminal_frame};
use super::*;

fn cloned_terminal_shell(old: &PaneSnapshot, new: &PaneSnapshot) -> bool {
    let mut old_shell = old.clone();
    let mut new_shell = new.clone();
    old_shell.title = new_shell.title.clone();
    old_shell.terminal = None;
    new_shell.terminal = None;
    old_shell == new_shell
}

#[test]
fn borrowed_terminal_shell_matches_clone_comparison_and_rejects_content_changes() {
    let base = terminal_frame(1, 1, 'a');
    let old = &base.editor.panes[0];
    let mutations: &[fn(&mut PaneSnapshot)] = &[
        |p| p.compact = !p.compact,
        |p| p.left_separator = !p.left_separator,
        |p| {
            p.preview = Some(DocumentPreview {
                generation: 1,
                source: 0,
                selection_only: false,
                text: "preview".into(),
                language: "text".into(),
                path: None,
            })
        },
        |p| {
            p.media = Some(MediaSnapshot {
                path: b"/workspace/picture.png".to_vec(),
                page: 0,
                page_buffer: false,
                pages: 1,
            })
        },
        |p| p.pane_id += 1,
        |p| p.area.width += 1,
        |p| p.body.height += 1,
        |p| p.active = !p.active,
        |p| p.jump_active = !p.jump_active,
        |p| p.dimmed = !p.dimmed,
        |p| p.drawable = !p.drawable,
        |p| p.line_numbers = !p.line_numbers,
        |p| p.relative_line_origin = Some(9),
        |p| p.line_digits += 1,
        |p| p.signs = !p.signs,
        |p| p.changes = !p.changes,
        |p| p.text_width += 1,
        |p| p.gutter_width += 1,
        |p| p.content_indent += 1,
        |p| p.scroll_row += 1,
        |p| p.scroll_wrap += 1,
        |p| p.wrap_width += 1,
        |p| p.cursor_screen_row = Some(4),
        |p| p.rows.push(SnapshotRow::Filler),
    ];
    assert!(same_terminal_shell(old, old));
    for mutate in mutations {
        let mut next = base.clone();
        mutate(&mut next.editor.panes[0]);
        let new = &next.editor.panes[0];
        assert!(!same_terminal_shell(old, new));
        assert_eq!(
            same_terminal_shell(old, new),
            cloned_terminal_shell(old, new)
        );
        assert!(TerminalDamageFrame::between(&base, &next).is_none());
    }
    let mut next = terminal_frame(2, 2, 'b');
    next.editor.panes[0].title.name = "changed terminal".into();
    assert!(same_terminal_shell(old, &next.editor.panes[0]));
    assert_eq!(
        same_terminal_shell(old, &next.editor.panes[0]),
        cloned_terminal_shell(old, &next.editor.panes[0])
    );
    let damage = TerminalDamageFrame::between(&base, &next).unwrap();
    let mut applied = base;
    assert!(damage.apply(&mut applied));
    assert_eq!(applied, next);
}

/// Isolates the pane-shell comparison, not socket, JSON or typing latency.
/// Run serially in release mode with --ignored --nocapture. Fixture setup,
/// damage validation and warmup are excluded; result destruction is included.
#[test]
#[ignore = "manual serialized release microbenchmark"]
fn terminal_damage_shell_benchmark() {
    use std::{hint::black_box, time::Instant};
    if cfg!(debug_assertions) {
        panic!("run this benchmark with --release");
    }
    for (columns, rows) in [(120, 40), (240, 80)] {
        for pane_count in [1, 4] {
            let mut base = terminal_frame(1, 1, 'a');
            let pane = &mut base.editor.panes[0];
            let terminal = pane.terminal.as_mut().unwrap();
            let cell = terminal.rows[0][0].clone();
            terminal.columns = columns;
            terminal.rows = vec![vec![cell; columns]; rows];
            terminal.line_ids = (0..rows).map(|row| Some(row as u64 + 1)).collect();
            let pane = pane.clone();
            base.editor.panes = (0..pane_count)
                .map(|id| {
                    let mut pane = pane.clone();
                    pane.pane_id = id;
                    pane
                })
                .collect();
            let mut sibling = editor_frame(1).editor.panes.remove(0);
            sibling.pane_id = pane_count;
            sibling.rows = (0..rows)
                .map(|document_row| {
                    SnapshotRow::Text(VisibleRow {
                        document_row,
                        continuation: false,
                        folded: false,
                        cursor_row: false,
                        diagnostic_sign: None,
                        change: None,
                        diff: None,
                        compared: None,
                        notification_severity: None,
                        runs: vec![TextRun {
                            text: "unchanged document row ".repeat(4),
                            kind: TextRunKind::Hint,
                        }],
                    })
                })
                .collect();
            base.editor.panes.push(sibling);
            let mut next = base.clone();
            next.id = FrameId::from_raw(2);
            let terminal = next.editor.panes[0].terminal.as_mut().unwrap();
            terminal.revision += 1;
            terminal.rows[0][0].character = 'b';
            let damage = TerminalDamageFrame::between(&base, &next).unwrap();
            let mut applied = base.clone();
            assert!(damage.apply(&mut applied));
            assert_eq!(applied, next);
            for sample in 0..7 {
                let orders = if sample % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                };
                for borrowed in orders {
                    let start = Instant::now();
                    for _ in 0..500 {
                        for (old, new) in black_box(&base.editor.panes)
                            .iter()
                            .zip(black_box(&next.editor.panes))
                        {
                            let same = if borrowed {
                                same_terminal_shell(black_box(old), black_box(new))
                            } else {
                                cloned_terminal_shell(black_box(old), black_box(new))
                            };
                            assert!(black_box(same));
                        }
                    }
                    let elapsed = start.elapsed().as_nanos() as f64 / 500.0 / 1000.0;
                    if sample > 0 {
                        println!(
                            "terminal_shell columns={columns} rows={rows} terminals={pane_count} sample={sample} borrowed={borrowed} us_per_frame={elapsed:.6}"
                        );
                    }
                }
            }
        }
    }
}

/// Synthetic component costs only: no host preparation, socket scheduling,
/// terminal decoding, GPU work or end-to-end input latency is measured.
#[test]
#[ignore = "manual serialized release microbenchmark"]
fn typing_frame_pipeline_benchmark() {
    use crate::protocol::HostResponse;
    use std::{hint::black_box, time::Instant};
    if cfg!(debug_assertions) {
        panic!("run this benchmark with --release");
    }
    const ITERATIONS: usize = 200;
    for (columns, rows) in [(120_u16, 40_u16), (240, 80)] {
        let mut frame = editor_frame(2);
        frame.editor.mode = Mode::Insert;
        frame.editor.geometry = FrameGeometry {
            screen: Rect {
                x: 0,
                y: 0,
                width: columns,
                height: rows,
            },
            editor: Rect {
                x: 0,
                y: 0,
                width: columns,
                height: rows - 2,
            },
            global_status_line: Rect {
                x: 0,
                y: rows - 2,
                width: columns,
                height: 1,
            },
            interaction_line: Rect {
                x: 0,
                y: rows - 1,
                width: columns,
                height: 1,
            },
        };
        let template = frame.editor.panes.remove(0);
        frame.editor.panes = (0..2)
            .map(|id| {
                let mut pane = template.clone();
                pane.pane_id = id;
                pane.active = id == 0;
                pane.title.name = format!("/workspace/document-{id}.txt");
                pane.title.dirty = id == 0;
                pane.area = Rect {
                    x: id as u16 * (columns / 2),
                    y: 0,
                    width: columns / 2,
                    height: rows - 2,
                };
                pane.body = Rect {
                    x: pane.area.x + 1,
                    y: 1,
                    width: columns / 2 - 2,
                    height: rows - 4,
                };
                pane.line_numbers = true;
                pane.line_digits = 3;
                pane.gutter_width = 5;
                pane.text_width = usize::from(pane.body.width) - pane.gutter_width;
                pane.wrap_width = pane.text_width;
                pane.cursor_screen_row = (id == 0).then_some(0);
                pane.rows = (0..usize::from(pane.body.height))
                    .map(|document_row| {
                        let text = format!("line {document_row:04}: repeated editing fixture ")
                            .repeat(4)
                            .chars()
                            .take(pane.text_width)
                            .collect();
                        SnapshotRow::Text(VisibleRow {
                            document_row,
                            continuation: false,
                            folded: false,
                            cursor_row: id == 0 && document_row == 0,
                            diagnostic_sign: None,
                            change: None,
                            diff: None,
                            compared: None,
                            notification_severity: None,
                            runs: vec![TextRun {
                                text,
                                kind: TextRunKind::Text {
                                    role: TextRole::Plain,
                                    scope: None,
                                    diagnostic: None,
                                    directory: false,
                                    whitespace: false,
                                    count: None,
                                },
                            }],
                        })
                    })
                    .collect();
                pane
            })
            .collect();
        let response = HostResponse::Frame {
            frame: Box::new(frame.clone()),
        };
        let encoded = serde_json::to_vec(&response).unwrap();
        let decoded: HostResponse = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, response);
        let core = CoreHostFrame::try_from(frame.clone()).unwrap();
        assert_eq!(HostFrame::from(core), frame);
        println!(
            "typing_pipeline columns={columns} rows={rows} payload_bytes={}",
            encoded.len()
        );
        for sample in 0..7 {
            // Rotate stage order; first complete pass is discarded warmup.
            for offset in 0..4 {
                let stage = (sample + offset) % 4;
                // Ownership conversion consumes frames. Prepare its owned
                // inputs outside timing rather than charging a hidden clone.
                let inputs = if stage == 3 {
                    vec![frame.clone(); ITERATIONS]
                } else {
                    Vec::new()
                };
                let start = Instant::now();
                match stage {
                    0 => {
                        for _ in 0..ITERATIONS {
                            drop(black_box(black_box(&response).clone()));
                        }
                    }
                    1 => {
                        for _ in 0..ITERATIONS {
                            drop(black_box(serde_json::to_vec(black_box(&response)).unwrap()));
                        }
                    }
                    2 => {
                        for _ in 0..ITERATIONS {
                            drop(black_box(
                                serde_json::from_slice::<HostResponse>(black_box(&encoded))
                                    .unwrap(),
                            ));
                        }
                    }
                    _ => {
                        for input in inputs {
                            drop(black_box(
                                CoreHostFrame::try_from(black_box(input)).unwrap(),
                            ));
                        }
                    }
                }
                let elapsed = start.elapsed().as_nanos() as f64 / ITERATIONS as f64 / 1000.0;
                if sample > 0 {
                    let name = [
                        "response_clone",
                        "json_encode",
                        "json_decode",
                        "wire_to_core",
                    ][stage];
                    println!(
                        "typing_pipeline columns={columns} rows={rows} sample={sample} stage={name} us_per_frame={elapsed:.6}"
                    );
                }
            }
        }
    }
}
