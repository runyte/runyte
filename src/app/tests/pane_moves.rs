// SPDX-License-Identifier: MPL-2.0

use super::*;

const WIDTH: u16 = 80;
const HEIGHT: u16 = 30;

fn geometry() -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            width: WIDTH,
            height: HEIGHT + 2,
            ..Rect::default()
        },
        editor: Rect {
            width: WIDTH,
            height: HEIGHT,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    }
}

/// Pane 0 across the top, with panes 1 and 2 side by side beneath it.
fn wide_top_over_two_columns() -> App {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "one\ntwo\nthree\n");
    app.panes.insert(1, Pane::new(0));
    app.panes.insert(2, Pane::new(0));
    app.layout = Layout::Split {
        axis: Axis::Vertical,
        ratio: u16::MAX / 2 + 1,
        first: Box::new(Layout::Pane(0)),
        second: Box::new(Layout::Split {
            axis: Axis::Horizontal,
            ratio: u16::MAX / 2 + 1,
            first: Box::new(Layout::Pane(1)),
            second: Box::new(Layout::Pane(2)),
        }),
    };
    app
}

fn area(app: &mut App, pane: usize) -> Rect {
    app.prepare_view(geometry()).pane(pane).unwrap().area
}

fn full(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn window_keys(app: &mut App, keys: &[(char, Modifiers)]) {
    for (character, modifiers) in keys {
        key(app, KeyCode::Char(*character), *modifiers);
    }
    assert!(app.pending_sequence().is_empty());
}

fn pointer(app: &mut App, kind: PointerEventKind, column: u16, row: u16, modifiers: Modifiers) {
    let view = app.prepare_view(geometry());
    app.handle_pointer(
        PointerEvent {
            kind,
            column,
            row,
            modifiers,
        },
        &view,
    )
    .unwrap();
}

/// Presses Alt on `pane`'s middle and drags to `(column, row)` without
/// releasing.
fn alt_drag(app: &mut App, pane: usize, column: u16, row: u16) {
    let start = area(app, pane);
    pointer(
        app,
        PointerEventKind::Down(PointerButton::Left),
        start.x + start.width / 2,
        start.y + start.height / 2,
        Modifiers::ALT,
    );
    pointer(
        app,
        PointerEventKind::Drag(PointerButton::Left),
        column,
        row,
        Modifiers::ALT,
    );
}

fn release(app: &mut App, column: u16, row: u16) {
    pointer(
        app,
        PointerEventKind::Up(PointerButton::Left),
        column,
        row,
        Modifiers::ALT,
    );
}

fn drop_preview(app: &mut App) -> Option<Rect> {
    let view = app.prepare_view(geometry());
    app.snapshot(&view).pane_drop
}

#[test]
fn ctrl_w_shift_h_lays_a_lower_pane_along_the_whole_left_side() {
    let mut app = wide_top_over_two_columns();
    app.active_pane = 1;

    window_keys(
        &mut app,
        &[('w', Modifiers::CONTROL), ('H', Modifiers::SHIFT)],
    );
    assert_eq!(app.status, "moved pane to the left edge");
    assert_eq!(app.active_pane, 1, "focus stays with the moved content");
    assert_eq!(area(&mut app, 1), full(0, 0, 40, 30));
    assert_eq!(area(&mut app, 0), full(40, 0, 40, 15));
    assert_eq!(area(&mut app, 2), full(40, 15, 40, 15));

    window_keys(
        &mut app,
        &[('w', Modifiers::CONTROL), ('H', Modifiers::SHIFT)],
    );
    assert_eq!(app.status, "pane already spans the left edge");
    assert_eq!(area(&mut app, 1), full(0, 0, 40, 30));
}

#[test]
fn space_w_shifted_letters_reach_every_side() {
    // What pane 2 leaves behind is pane 0 over pane 1: one column, two rows.
    // A moved pane takes one level share of that, so half the width beside
    // it and a third of the height above or below it.
    let cases = [
        ('H', full(0, 0, 40, 30), "left"),
        ('L', full(40, 0, 40, 30), "right"),
        ('K', full(0, 0, 80, 10), "top"),
        ('J', full(0, 20, 80, 10), "bottom"),
    ];
    for (letter, expected, side) in cases {
        let mut app = wide_top_over_two_columns();
        app.active_pane = 2;
        window_keys(
            &mut app,
            &[
                (' ', Modifiers::NONE),
                ('w', Modifiers::NONE),
                (letter, Modifiers::SHIFT),
            ],
        );
        assert_eq!(area(&mut app, 2), expected, "Space w {letter}");
        assert_eq!(app.status, format!("moved pane to the {side} edge"));
    }
}

#[test]
fn ctrl_w_moves_work_from_insert_mode() {
    let mut app = wide_top_over_two_columns();
    app.active_pane = 2;
    app.mode = Mode::Insert;
    window_keys(
        &mut app,
        &[('w', Modifiers::CONTROL), ('L', Modifiers::SHIFT)],
    );
    assert_eq!(area(&mut app, 2), full(40, 0, 40, 30));
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(text(&app), "one\ntwo\nthree\n");
}

#[test]
fn moving_a_pane_is_refused_while_maximized_and_with_one_pane() {
    let mut app = wide_top_over_two_columns();
    app.active_pane = 1;
    window_keys(
        &mut app,
        &[
            (' ', Modifiers::NONE),
            ('w', Modifiers::NONE),
            ('f', Modifiers::NONE),
        ],
    );
    window_keys(
        &mut app,
        &[('w', Modifiers::CONTROL), ('H', Modifiers::SHIFT)],
    );
    assert!(app.status.contains("keeps the current pane maximized"));
    assert!(app.status_error);
    window_keys(
        &mut app,
        &[
            (' ', Modifiers::NONE),
            ('w', Modifiers::NONE),
            ('f', Modifiers::NONE),
        ],
    );
    assert_eq!(area(&mut app, 1), full(0, 15, 40, 15), "layout untouched");

    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut alone = App::new(config, None).unwrap();
    window_keys(
        &mut alone,
        &[('w', Modifiers::CONTROL), ('J', Modifiers::SHIFT)],
    );
    assert_eq!(alone.status, "only one pane");
}

#[test]
fn alt_drag_to_a_layout_side_previews_and_then_moves_the_pane() {
    let mut app = wide_top_over_two_columns();
    app.active_pane = 0;

    alt_drag(&mut app, 1, 0, 20);
    assert_eq!(app.active_pane, 1, "the press picks the pane up");
    assert_eq!(app.status, "release to move pane to the left edge");
    assert_eq!(drop_preview(&mut app), Some(full(0, 0, 40, 30)));
    assert_eq!(area(&mut app, 1), full(0, 15, 40, 15), "nothing moves yet");

    release(&mut app, 0, 20);
    assert_eq!(app.status, "moved pane to the left edge");
    assert_eq!(area(&mut app, 1), full(0, 0, 40, 30));
    assert_eq!(area(&mut app, 0), full(40, 0, 40, 15));
    assert_eq!(drop_preview(&mut app), None);
    assert!(app.pointer_drag.is_none());
}

#[test]
fn alt_drag_into_a_side_of_another_pane_splits_it_there() {
    let mut app = wide_top_over_two_columns();
    let target = area(&mut app, 2);

    // The bottom quarter of pane 2 is nearer its bottom than any other side.
    let row = target.y + target.height - 3;
    alt_drag(&mut app, 0, target.x + target.width / 2, row);
    assert_eq!(
        app.status,
        "release to move pane into the bottom half of this pane"
    );
    let preview = drop_preview(&mut app).unwrap();
    release(&mut app, target.x + target.width / 2, row);

    assert_eq!(
        app.status,
        "moved pane into the bottom half of another pane"
    );
    assert_eq!(area(&mut app, 0), preview);
    assert_eq!(area(&mut app, 0), full(40, 15, 40, 15));
    assert_eq!(area(&mut app, 2), full(40, 0, 40, 15));
    assert_eq!(area(&mut app, 1), full(0, 0, 40, 30));
}

#[test]
fn alt_drag_into_the_middle_of_another_pane_swaps_contents() {
    let mut app = wide_top_over_two_columns();
    let target = area(&mut app, 0);
    let (column, row) = (target.x + target.width / 2, target.y + target.height / 2);

    alt_drag(&mut app, 2, column, row);
    assert_eq!(app.status, "release to swap pane contents");
    assert_eq!(drop_preview(&mut app), Some(target));
    release(&mut app, column, row);

    assert_eq!(app.status, "swapped pane contents");
    assert_eq!(app.active_pane, 0, "focus follows the content to its place");
    assert_eq!(area(&mut app, 0), target, "the tree keeps its shape");
    assert_eq!(area(&mut app, 2), full(40, 15, 40, 15));
}

#[test]
fn releasing_where_nothing_would_change_leaves_the_layout_alone() {
    let mut app = wide_top_over_two_columns();
    let own = area(&mut app, 1);

    alt_drag(&mut app, 1, own.x + 10, own.y + 7);
    assert_eq!(drop_preview(&mut app), None);
    assert!(app.status.starts_with("moving pane:"));
    release(&mut app, own.x + 10, own.y + 7);
    assert_eq!(app.status, "pane left where it was");
    assert_eq!(area(&mut app, 1), own);

    // Pane 0 already spans the top, so the top band offers it nothing.
    alt_drag(&mut app, 0, 40, 0);
    assert_eq!(drop_preview(&mut app), None);
    release(&mut app, 40, 0);
    assert_eq!(area(&mut app, 0), full(0, 0, 80, 15));
}

#[test]
fn a_key_press_abandons_a_pane_drag() {
    let mut app = wide_top_over_two_columns();
    alt_drag(&mut app, 1, 0, 20);
    assert!(drop_preview(&mut app).is_some());

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.pointer_drag.is_none());
    assert_eq!(drop_preview(&mut app), None);
    release(&mut app, 0, 20);
    assert_eq!(area(&mut app, 1), full(0, 15, 40, 15));
}

#[test]
fn alt_on_a_shared_border_still_resizes() {
    let mut app = wide_top_over_two_columns();
    let left = area(&mut app, 1);
    let boundary = left.x + left.width;

    pointer(
        &mut app,
        PointerEventKind::Down(PointerButton::Left),
        boundary,
        left.y + 5,
        Modifiers::ALT,
    );
    assert!(matches!(app.pointer_drag, Some(PointerDrag::Resize { .. })));
    pointer(
        &mut app,
        PointerEventKind::Drag(PointerButton::Left),
        boundary + 6,
        left.y + 5,
        Modifiers::ALT,
    );
    release(&mut app, boundary + 6, left.y + 5);
    assert_eq!(area(&mut app, 1).width, 46);
}

#[test]
fn alt_drag_belongs_to_the_editor_over_a_terminal_reporting_the_mouse() {
    let mut app = wide_top_over_two_columns();
    let terminal = app.terminals.insert_test_session(38, 13);
    app.panes.get_mut(&1).unwrap().terminal = Some(terminal);
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b[?1000h\x1b[?1006h".to_vec(),
    });
    let generation = app.terminals.get(terminal).unwrap().input_generation();

    alt_drag(&mut app, 1, 0, 20);
    release(&mut app, 0, 20);

    assert_eq!(area(&mut app, 1), full(0, 0, 40, 30));
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        generation,
        "no pointer report reached the child"
    );
    close_test_terminals(&mut app);
}

#[test]
fn the_drop_preview_is_outlined_over_the_panes() {
    let mut app = wide_top_over_two_columns();
    alt_drag(&mut app, 1, 0, 20);

    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(WIDTH, HEIGHT + 2)).unwrap();
    terminal
        .draw(|frame| {
            let view = app.prepare_view(geometry());
            let snapshot = app.snapshot(&view);
            crate::ui::render_exact_colors_for_test(
                frame,
                &app,
                &snapshot,
                &crate::key_hints::KeyHintState::default(),
            );
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(0, 0)].symbol(), "┏");
    assert_eq!(buffer[(39, 29)].symbol(), "┛");
    assert_eq!(buffer[(0, 20)].symbol(), "┃");
}

#[test]
fn releasing_a_pane_over_the_directory_sidebar_cancels_the_drag() {
    let mut app = wide_top_over_two_columns();
    let root = temporary("pane-drag-sidebar");
    fs::create_dir_all(&root).unwrap();
    app.directory_tree.root = root.clone();
    app.directory_tree.visible = true;
    let before = area(&mut app, 1);
    let layout = app.prepare_view(geometry()).layout_area();
    alt_drag(&mut app, 1, layout.x, 20);
    assert!(drop_preview(&mut app).is_some());
    pointer(
        &mut app,
        PointerEventKind::Drag(PointerButton::Left),
        1,
        5,
        Modifiers::ALT,
    );
    assert!(drop_preview(&mut app).is_none());
    release(&mut app, 1, 5);
    assert!(app.pointer_drag.is_none());
    assert_eq!(area(&mut app, 1), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn refused_and_cancelled_pane_gestures_never_report_an_unmatched_terminal_release() {
    for modifiers in [Modifiers::ALT, Modifiers::ALT | Modifiers::SHIFT] {
        for state in ["single", "maximized", "cancelled", "semantic"] {
            let mut app = wide_top_over_two_columns();
            app.active_pane = 1;
            if state == "single" {
                app.layout = Layout::Pane(1);
                app.panes.retain(|id, _| *id == 1);
            } else if state == "maximized" {
                window_keys(
                    &mut app,
                    &[
                        (' ', Modifiers::NONE),
                        ('w', Modifiers::NONE),
                        ('f', Modifiers::NONE),
                    ],
                );
            }
            let terminal = app.terminals.insert_test_session(78, 28);
            app.panes.get_mut(&1).unwrap().terminal = Some(terminal);
            app.apply_terminal_output(TerminalOutput::Bytes {
                id: terminal,
                bytes: b"\x1b[?1003h\x1b[?1006h".to_vec(),
            });
            let body = app.prepare_view(geometry()).pane(1).unwrap().body;
            let (column, row) = (body.x + 3, body.y + 3);
            let generation = app.terminals.get(terminal).unwrap().input_generation();
            pointer(
                &mut app,
                PointerEventKind::Down(PointerButton::Left),
                column,
                row,
                modifiers,
            );
            // Passive motion must also remain editor-owned with any-motion reporting.
            pointer(
                &mut app,
                PointerEventKind::Moved,
                column,
                row,
                Modifiers::NONE,
            );
            assert_eq!(
                app.terminals.get(terminal).unwrap().input_generation(),
                generation,
                "press and passive motion: {state}"
            );
            if state == "cancelled" {
                key(&mut app, KeyCode::Escape, Modifiers::NONE);
            } else if state == "semantic" {
                app.execute(
                    CommandInvocation::editor(
                        EditorCommand::EqualizeWindows,
                        CommandExecutionContext::default(),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            // A key may itself be legitimate terminal input. The remainder of
            // the cancelled pointer gesture must not add any mouse reports.
            let generation = app.terminals.get(terminal).unwrap().input_generation();
            pointer(
                &mut app,
                PointerEventKind::Drag(PointerButton::Left),
                column + 1,
                row,
                Modifiers::NONE,
            );
            pointer(
                &mut app,
                PointerEventKind::Moved,
                column,
                row,
                Modifiers::NONE,
            );
            release(&mut app, column, row);
            assert!(app.pointer_drag.is_none(), "{state}");
            assert!(!app.pane_pointer_owned, "{state}");
            assert_eq!(
                app.terminals.get(terminal).unwrap().input_generation(),
                generation,
                "{state}"
            );
            pointer(
                &mut app,
                PointerEventKind::Down(PointerButton::Left),
                column,
                row,
                Modifiers::NONE,
            );
            assert!(
                app.terminals.get(terminal).unwrap().input_generation() > generation,
                "next ordinary press still reaches the child: {state}"
            );
            close_test_terminals(&mut app);
        }
    }
}

#[test]
fn losing_pointer_capture_clears_the_drop_outline_without_moving() {
    let mut app = wide_top_over_two_columns();
    let before = area(&mut app, 1);
    alt_drag(&mut app, 1, 0, 20);
    assert!(drop_preview(&mut app).is_some());
    assert!(app.cancel_pointer_drag());
    assert!(!app.pane_pointer_owned);
    assert!(drop_preview(&mut app).is_none());
    assert_eq!(area(&mut app, 1), before);
    assert!(!app.cancel_pointer_drag());
}

fn start_expansion(app: &mut App, pane: usize, column: u16, row: u16) {
    let body = app.prepare_view(geometry()).pane(pane).unwrap().body;
    pointer(
        app,
        PointerEventKind::Down(PointerButton::Left),
        body.x + body.width / 2,
        body.y + body.height / 2,
        Modifiers::ALT | Modifiers::SHIFT,
    );
    // The press owns the gesture even after both modifiers are released.
    pointer(
        app,
        PointerEventKind::Drag(PointerButton::Left),
        column,
        row,
        Modifiers::NONE,
    );
}

#[test]
fn shift_alt_drag_extends_through_the_requested_three_layouts() {
    let mut app = wide_top_over_two_columns();
    app.native_media = true;
    app.active_pane = 2;
    app.active_mut()
        .replace_selection(Selection::single(Range::new(1, 4)));
    app.execute(parse_colon_command("preview").unwrap())
        .unwrap();
    let selected = app.active().selection.clone();
    let capture = app.document_previews[&2].clone();
    let buffer = app.active().buffer;
    let text = app.buffers[buffer].to_string();
    let dirty = app.buffers[buffer].dirty;

    start_expansion(&mut app, 2, 60, 5);
    assert_eq!(drop_preview(&mut app), Some(full(40, 0, 40, 30)));
    assert_eq!(
        area(&mut app, 0),
        full(0, 0, 80, 15),
        "motion only previews"
    );
    release(&mut app, 60, 5);
    assert_eq!(area(&mut app, 0), full(0, 0, 40, 15));
    assert_eq!(area(&mut app, 1), full(0, 15, 40, 15));
    assert_eq!(area(&mut app, 2), full(40, 0, 40, 30));
    assert_eq!(app.active_pane, 2);
    assert_eq!(app.active().selection, selected);
    assert_eq!(app.document_previews[&2], capture);

    start_expansion(&mut app, 1, 60, 20);
    assert_eq!(drop_preview(&mut app), Some(full(0, 15, 80, 15)));
    release(&mut app, 60, 20);
    assert_eq!(area(&mut app, 0), full(0, 0, 40, 15));
    assert_eq!(area(&mut app, 2), full(40, 0, 40, 15));
    assert_eq!(area(&mut app, 1), full(0, 15, 80, 15));

    start_expansion(&mut app, 1, 20, 5);
    assert_eq!(drop_preview(&mut app), Some(full(0, 0, 80, 30)));
    release(&mut app, 20, 5);
    assert_eq!(app.active_pane, 1);
    assert_eq!(app.panes.len(), 1);
    assert_eq!(area(&mut app, 1), full(0, 0, 80, 30));
    assert!(!app.document_previews.contains_key(&2));
    assert!(!app.closed_buffers.contains(&buffer));
    assert_eq!(app.buffers[buffer].to_string(), text);
    assert_eq!(app.buffers[buffer].dirty, dirty);
}

#[test]
fn expansion_can_be_cancelled_and_shared_borders_still_resize() {
    for cancel in ["escape", "focus", "outside"] {
        let mut app = wide_top_over_two_columns();
        start_expansion(&mut app, 2, 60, 5);
        assert!(drop_preview(&mut app).is_some());
        match cancel {
            "escape" => key(&mut app, KeyCode::Escape, Modifiers::NONE),
            "focus" => {
                app.cancel_pointer_drag();
            }
            _ => {}
        }
        release(
            &mut app,
            60,
            if cancel == "outside" { HEIGHT + 1 } else { 5 },
        );
        assert_eq!(area(&mut app, 0), full(0, 0, 80, 15));
        assert_eq!(area(&mut app, 2), full(40, 15, 40, 15));
        assert_eq!(app.panes.len(), 3);
        assert!(drop_preview(&mut app).is_none());
    }
    let mut app = wide_top_over_two_columns();
    pointer(
        &mut app,
        PointerEventKind::Down(PointerButton::Left),
        40,
        20,
        Modifiers::ALT | Modifiers::SHIFT,
    );
    assert!(matches!(app.pointer_drag, Some(PointerDrag::Resize { .. })));
}

#[test]
fn expansion_closes_terminal_panes_without_stopping_their_sessions() {
    let mut app = wide_top_over_two_columns();
    let terminal = app.terminals.insert_test_session(38, 13);
    app.panes.get_mut(&2).unwrap().terminal = Some(terminal);
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b[?1003h\x1b[?1006h".to_vec(),
    });
    let generation = app.terminals.get(terminal).unwrap().input_generation();
    start_expansion(&mut app, 2, 60, 5);
    release(&mut app, 60, 5);
    assert_eq!(app.panes[&2].terminal, Some(terminal));
    assert_eq!(
        app.terminals.get(terminal).unwrap().input_generation(),
        generation
    );
    start_expansion(&mut app, 0, 60, 5);
    release(&mut app, 60, 5);
    start_expansion(&mut app, 0, 60, 20);
    release(&mut app, 60, 20);
    assert!(!app.panes.contains_key(&2));
    assert!(app.terminals.get(terminal).is_some());
    close_test_terminals(&mut app);
}

#[test]
fn extending_over_a_temporary_comparison_side_restores_the_source_like_window_close() {
    use crate::git::{MemoryGitProvider, Repository};
    for side in [Side::Left, Side::Right] {
        let root = crate::test_support::TestRuntimeRoot::new("expand-comparison").unwrap();
        let path = root.join("source.txt");
        fs::write(&path, "working\n").unwrap();
        let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        )))));
        ports.replace_git(Box::new(
            MemoryGitProvider::new(Repository::new(&*root))
                .with_staged("source.txt", "staged\n")
                .with_working("source.txt", "working\n"),
        ));
        let mut app = App::new_in_isolated_project(&*root, ports).unwrap();
        app.open_file(path).unwrap();
        let source = app.active().buffer;
        for character in [' ', 'g', 'D'] {
            press(&mut app, character);
        }
        let pane = app.diffs[0].side(side).pane;
        let other = app.diffs[0]
            .panes()
            .into_iter()
            .find(|id| *id != pane)
            .unwrap();
        let target = area(&mut app, other);
        let (column, row) = (target.x + target.width / 2, target.y + target.height / 2);
        start_expansion(&mut app, pane, column, row);
        assert_eq!(drop_preview(&mut app), Some(full(0, 0, WIDTH, HEIGHT)));
        release(&mut app, column, row);
        assert_eq!(app.active_pane, pane);
        assert_eq!(app.panes.len(), 1);
        assert!(app.diffs.is_empty());
        assert_eq!(app.active().buffer, source);
        assert_eq!(app.active_buffer().to_string(), "working\n");
    }
}

#[test]
fn dragging_either_segment_of_a_grid_preserves_the_other_pair() {
    for axis in [Axis::Vertical, Axis::Horizontal] {
        for pair in [(0, 1), (2, 3)] {
            let cross = if axis == Axis::Vertical {
                Axis::Horizontal
            } else {
                Axis::Vertical
            };
            let mut app = wide_top_over_two_columns();
            app.panes.insert(3, Pane::new(0));
            app.layout = Layout::Pane(0);
            app.layout.split(0, 1, axis);
            app.layout.split(0, 2, cross);
            app.layout.split(1, 3, cross);
            let before: Vec<_> = (0..4).map(|id| area(&mut app, id)).collect();
            let leading = before[pair.0];
            let (column, row) = match axis {
                Axis::Horizontal => (leading.x + leading.width, leading.y + 4),
                Axis::Vertical => (leading.x + 4, leading.y + leading.height),
            };
            pointer(
                &mut app,
                PointerEventKind::Down(PointerButton::Left),
                column,
                row,
                Modifiers::NONE,
            );
            for delta in 1..=4 {
                let (x, y) = match axis {
                    Axis::Horizontal => (column + delta, row),
                    Axis::Vertical => (column, row + delta),
                };
                pointer(
                    &mut app,
                    PointerEventKind::Drag(PointerButton::Left),
                    x,
                    y,
                    Modifiers::NONE,
                );
            }
            let after: Vec<_> = (0..4).map(|id| area(&mut app, id)).collect();
            for id in 0..4 {
                if id != pair.0 && id != pair.1 {
                    assert_eq!(after[id], before[id], "{axis:?}, pair {pair:?}, pane {id}");
                }
            }
            match axis {
                Axis::Horizontal => {
                    assert_eq!(after[pair.0].width, leading.width + 4);
                    assert_eq!(after[pair.1].width, before[pair.1].width - 4);
                }
                Axis::Vertical => {
                    assert_eq!(after[pair.0].height, leading.height + 4);
                    assert_eq!(after[pair.1].height, before[pair.1].height - 4);
                }
            }
        }
    }
}
