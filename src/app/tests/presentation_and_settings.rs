// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn folds_share_one_projection_across_snapshot_motion_and_panes() {
    let path = temporary("syntax-fold.rs");
    fs::write(&path, "fn outer() {\n    let value = 1;\n}\nlast\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.panes.insert(1, Pane::new(0));
    app.layout = Layout::Split {
        axis: Axis::Horizontal,
        ratio: u16::MAX / 2 + 1,
        first: Box::new(Layout::Pane(0)),
        second: Box::new(Layout::Pane(1)),
    };

    press(&mut app, ' ');
    press(&mut app, 'x');
    press(&mut app, 'f');
    assert!(!app.panes[&0].folds.collapsed.is_empty());
    assert!(app.panes[&1].folds.collapsed.is_empty());

    let geometry = FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 12,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        },
        status: Rect {
            x: 0,
            y: 10,
            width: 80,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 11,
            width: 80,
            height: 1,
        },
    };
    let prepared = app.prepare_view(geometry);
    let snapshot = app.snapshot(&prepared);
    let folded = snapshot.pane(0).unwrap();
    assert_eq!(
        folded
            .rows
            .iter()
            .filter_map(|row| match row {
                crate::snapshot::SnapshotRow::Text(row) => Some(row.document_row),
                crate::snapshot::SnapshotRow::Placeholder
                | crate::snapshot::SnapshotRow::Padding
                | crate::snapshot::SnapshotRow::Filler => None,
            })
            .collect::<Vec<_>>(),
        vec![0, 2, 3, 4]
    );
    assert!(folded.rows.iter().any(|row| match row {
        crate::snapshot::SnapshotRow::Text(row) => row.runs.iter().any(|run| {
            run.kind == crate::snapshot::TextRunKind::FoldMarker && run.text.contains("… 1 line")
        }),
        crate::snapshot::SnapshotRow::Placeholder
        | crate::snapshot::SnapshotRow::Padding
        | crate::snapshot::SnapshotRow::Filler => false,
    }));
    assert!(matches!(
        &folded.rows[0],
        crate::snapshot::SnapshotRow::Text(row) if row.folded
    ));
    assert_eq!(snapshot, app.snapshot(&prepared), "snapshot is immutable");

    app.motion(Motion::Down);
    assert_eq!(app.cursor_position().row, 2, "motion skips hidden rows");

    app.active_pane = 1;
    let prepared = app.prepare_view(geometry);
    assert!(
        prepared
            .pane(1)
            .unwrap()
            .rows
            .iter()
            .any(|row| row.document_row == Some(1))
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn markdown_fold_hides_the_final_content_line_at_eof() {
    let path = temporary("syntax-fold-final-content.md");
    fs::write(
        &path,
        "## License\n\nFirst paragraph.\n\nThird-party assets have their own licenses.",
    )
    .unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.fold_all_syntax();

    let prepared = app.prepare_view(FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 8,
        },
        status: Rect {
            x: 0,
            y: 8,
            width: 80,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 9,
            width: 80,
            height: 1,
        },
    });
    let rows = &prepared.pane(0).unwrap().rows;
    assert_eq!(
        rows.iter().map(|row| row.document_row).collect::<Vec<_>>(),
        vec![Some(0)]
    );
    assert!(rows[0].folded);
    assert_eq!(rows[0].folded_lines, Some(4));

    let snapshot = app.snapshot(&prepared);
    let crate::snapshot::SnapshotRow::Text(anchor) = &snapshot.pane(0).unwrap().rows[0] else {
        panic!("fold anchor is visible");
    };
    assert!(anchor.folded);
    assert!(anchor.runs.iter().any(|run| {
        run.kind == crate::snapshot::TextRunKind::FoldMarker && run.text.contains("… 4 lines")
    }));
    fs::remove_file(path).unwrap();
}

#[test]
fn fold_closer_detection_does_not_leak_a_python_call_body_line() {
    let path = temporary("syntax-fold-final-call.py");
    fs::write(&path, "def run():\n    first()\n    final()").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.fold_all_syntax();

    let fold = app
        .resolved_folds(0)
        .into_iter()
        .find(|fold| fold.anchor_row == 0)
        .expect("function fold");
    assert!(fold.hides(1));
    assert!(
        fold.hides(2),
        "the final call is body content, not a `)` closer"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn pointer_click_drag_wheel_and_resize_use_the_prepared_projection() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    seed(
        &mut app,
        "a界b\none\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\ntwelve\nthirteen\nfourteen\nfifteen",
    );
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 12,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 10,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let body = view.pane(0).unwrap().body;

    // The second cell inside the double-width glyph maps to its character
    // boundary rather than a byte or terminal-cell offset.
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: body.x + 2,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(cursor(&app), Position::new(0, 1));

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Drag(PointerButton::Left),
            column: body.x + 4,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    // The drag ends past the end of the row, and a selection addresses
    // characters, so its head is the row's last one rather than the place
    // after it that only an Insert caret may occupy.
    assert_eq!(app.active().selection.primary(), Range::new(1, 2));
    assert_eq!(app.mode, Mode::Select);
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Up(PointerButton::Left),
            column: body.x + 4,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::ScrollDown,
            column: body.x,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active().scroll_row, 3);

    app.handle_pointer_repeated(
        PointerEvent {
            kind: PointerEventKind::ScrollDown,
            column: body.x,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
        2,
    )
    .unwrap();
    assert_eq!(app.active().scroll_row, 9);

    app.split(Axis::Horizontal, None).unwrap();
    let before = app.prepare_view(geometry);
    let left = before.pane(0).unwrap().area;
    let boundary = left.x + left.width;
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: boundary,
            row: 2,
            modifiers: Modifiers::NONE,
        },
        &before,
    )
    .unwrap();
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Drag(PointerButton::Left),
            column: boundary + 4,
            row: 2,
            modifiers: Modifiers::NONE,
        },
        &before,
    )
    .unwrap();
    let after = app.prepare_view(geometry);
    assert_eq!(after.pane(0).unwrap().area.width, left.width + 4);
}

/// Presses at `from` and drags to `to`, both as cell columns on the first row
/// of the pane body, and releases there.
fn drag_across(app: &mut App, view: &PreparedView, from: u16, to: u16) {
    let body = view.pane(0).unwrap().body;
    for (kind, column) in [
        (PointerEventKind::Down(PointerButton::Left), from),
        (PointerEventKind::Drag(PointerButton::Left), to),
        (PointerEventKind::Up(PointerButton::Left), to),
    ] {
        app.handle_pointer(
            PointerEvent {
                kind,
                column: body.x + column,
                row: body.y,
                modifiers: Modifiers::NONE,
            },
            view,
        )
        .unwrap();
    }
}

#[test]
fn a_pointer_drag_selects_through_the_character_it_ends_on() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "Testtest rest");
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);

    // The pointer names a character, not the boundary before it, so the word
    // is covered by pressing on its first letter and releasing on its last.
    drag_across(&mut app, &view, 0, 7);
    assert_eq!(app.active().selection.primary(), Range::new(0, 7));
    assert_eq!(app.mode, Mode::Select);
    press(&mut app, 'y');
    assert_eq!(app.registers[&'"'].text, "Testtest");

    // Dragging the other way covers the same word: the pressed cell is the
    // one the selection ends on rather than the one it starts before.
    drag_across(&mut app, &view, 7, 0);
    assert_eq!(app.active().selection.primary(), Range::new(7, 0));
    assert_eq!(app.mode, Mode::Select);
    press(&mut app, 'y');
    assert_eq!(app.registers[&'"'].text, "Testtest");

    // A press that goes nowhere is still a caret rather than a selection.
    drag_across(&mut app, &view, 4, 4);
    assert_eq!(app.active().selection.primary(), Range::new(4, 4));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn right_click_on_any_selection_yanks_all_selections_to_the_system_clipboard() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let shared = Arc::new(Mutex::new(String::new()));
    let mut app = App::new(config, None).unwrap();
    app.set_system_clipboard(Box::new(super::editing_and_buffers::MemoryClipboard(
        shared.clone(),
    )));
    seed(&mut app, "alpha beta gamma");
    let selection = Selection::new([Range::new(0, 4), Range::new(11, 15)].to_vec(), 1);
    app.active_mut().replace_selection(selection.clone());
    app.mode = Mode::Select;

    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let body = view.pane(0).unwrap().body;

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Right),
            column: body.x + 13,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();

    assert_eq!(&*shared.lock().unwrap(), "alpha\ngamma");
    assert_eq!(app.active().selection, selection);
    assert_eq!(app.mode, Mode::Select);
    assert_eq!(app.status, "yanked to system clipboard");

    *shared.lock().unwrap() = "unchanged".to_owned();
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Right),
            column: body.x + 7,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(&*shared.lock().unwrap(), "unchanged");
}

#[test]
fn right_click_does_not_clamp_gutter_or_trailing_cells_onto_a_selection() {
    let shared = Arc::new(Mutex::new("unchanged".to_owned()));
    let mut app = App::new(Config::default(), None).unwrap();
    app.set_system_clipboard(Box::new(super::editing_and_buffers::MemoryClipboard(
        shared.clone(),
    )));
    seed(&mut app, "alpha");
    app.active_mut()
        .replace_selection(Selection::single(Range::new(0, 4)));
    app.mode = Mode::Select;

    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let pane = view.pane(0).unwrap();
    assert!(pane.gutter_width > 0);

    for column in [pane.body.x, pane.body.x + pane.body.width - 1] {
        app.handle_pointer(
            PointerEvent {
                kind: PointerEventKind::Down(PointerButton::Right),
                column,
                row: pane.body.y,
                modifiers: Modifiers::NONE,
            },
            &view,
        )
        .unwrap();
        assert_eq!(&*shared.lock().unwrap(), "unchanged");
    }
}

#[test]
fn right_click_yanks_the_visible_caret_on_an_empty_row() {
    let shared = Arc::new(Mutex::new(String::new()));
    let mut app = App::new(Config::default(), None).unwrap();
    app.set_system_clipboard(Box::new(super::editing_and_buffers::MemoryClipboard(
        shared.clone(),
    )));
    seed(&mut app, "one\n\nthree");
    app.active_mut().replace_selection(Selection::point(4));

    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let pane = view.pane(0).unwrap();
    let text_x = pane.body.x + pane.gutter_width as u16;
    let empty_screen_row = pane
        .rows
        .iter()
        .position(|row| row.document_row == Some(1))
        .unwrap() as u16;
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Right),
            column: text_x,
            row: pane.body.y + empty_screen_row,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();

    assert_eq!(&*shared.lock().unwrap(), "\n");
}

#[test]
fn right_click_uses_viewport_relative_tab_stops_after_horizontal_scroll() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    config.editor.tab_width = 4;
    let shared = Arc::new(Mutex::new(String::new()));
    let mut app = App::new(config, None).unwrap();
    app.set_system_clipboard(Box::new(super::editing_and_buffers::MemoryClipboard(
        shared.clone(),
    )));
    seed(&mut app, "ab\tz");
    app.active_mut().replace_selection(Selection::point(2));
    app.active_mut().scroll_col = 2;
    app.active_mut().preserve_scroll = true;

    let geometry = FrameGeometry {
        screen: Rect {
            width: 10,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 10,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let pane = view.pane(0).unwrap();
    assert_eq!(pane.scroll_col, 2);
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Right),
            column: pane.body.x + 3,
            row: pane.body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();

    assert_eq!(&*shared.lock().unwrap(), "\t");
}

#[cfg(unix)]
#[test]
fn right_click_yanks_a_wide_terminal_review_caret_from_its_second_cell() {
    let shared = Arc::new(Mutex::new(String::new()));
    let mut app = App::new(Config::default(), None).unwrap();
    app.set_system_clipboard(Box::new(super::editing_and_buffers::MemoryClipboard(
        shared.clone(),
    )));
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: "界\r\n".as_bytes().to_vec(),
    });
    app.terminals.get_mut(terminal).unwrap().begin_review();
    app.mode = Mode::Normal;

    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let body = view.pane(0).unwrap().body;
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Right),
            column: body.x + 1,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();

    assert_eq!(&*shared.lock().unwrap(), "界");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.status,
        "terminal review selection yanked to system clipboard"
    );
}

/// Presses at a cell column on `row` of the pane body and releases there.
fn press_at(app: &mut App, view: &PreparedView, column: u16, row: u16) {
    let body = view.pane(0).unwrap().body;
    for kind in [
        PointerEventKind::Down(PointerButton::Left),
        PointerEventKind::Up(PointerButton::Left),
    ] {
        app.handle_pointer(
            PointerEvent {
                kind,
                column: body.x + column,
                row: body.y + row,
                modifiers: Modifiers::NONE,
            },
            view,
        )
        .unwrap();
    }
}

#[test]
fn a_press_past_the_end_of_a_line_lands_where_that_mode_lets_a_caret_sit() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    seed(
        &mut app,
        "abc

longer line",
    );
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);

    // A Normal caret addresses a character, so the blank area past a line
    // names its last one rather than the place after it, exactly where `$`
    // would leave the caret.
    press_at(&mut app, &view, 9, 0);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.active().head(), 2);
    press(&mut app, 'y');
    assert_eq!(app.registers[&'"'].text, "c");

    // An empty row has no character to land on, so its own offset is where
    // the caret belongs.
    press_at(&mut app, &view, 5, 1);
    assert_eq!(app.active().head(), 4);

    // An Insert caret may sit past the last character, which is what makes
    // clicking the blank area past a line append to it.
    press(&mut app, 'i');
    assert_eq!(app.mode, Mode::Insert);
    press_at(&mut app, &view, 9, 0);
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.active().head(), 3);
    press(&mut app, 'd');
    assert_eq!(
        text(&app),
        "abcd

longer line"
    );
}

#[test]
fn a_pointer_drag_under_the_vim_grammar_covers_the_same_characters() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    config.editor.grammar = GrammarKind::Vim;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "Testtest rest");
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 6,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 4,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);

    // Vim writes the same span down with its leading end one past the last
    // covered character, which is what its half-open semantics mean.
    drag_across(&mut app, &view, 0, 7);
    assert_eq!(app.active().selection.primary(), Range::new(0, 8));
    assert_eq!(
        app.active().selection_semantics(),
        SelectionSemantics::HalfOpen
    );
    press(&mut app, 'y');
    assert_eq!(app.registers[&'"'].text, "Testtest");

    drag_across(&mut app, &view, 7, 0);
    assert_eq!(app.active().selection.primary(), Range::new(8, 0));
    press(&mut app, 'y');
    assert_eq!(app.registers[&'"'].text, "Testtest");
}

#[cfg(unix)]
#[test]
fn a_reported_terminal_click_enters_input_before_forwarding() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let id = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id,
        bytes: b"\x1b[?1000h\x1b[?1006h".to_vec(),
    });
    app.panes.insert(1, Pane::new(0));
    app.layout = Layout::Split {
        axis: Axis::Horizontal,
        ratio: u16::MAX / 2 + 1,
        first: Box::new(Layout::Pane(0)),
        second: Box::new(Layout::Pane(1)),
    };
    app.active_pane = 1;
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 12,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 10,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let body = view.pane(0).unwrap().body;
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: body.x,
            row: body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active_pane, 0);
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.active_terminal(), Some(id));
    assert_eq!(
        app.handle_pointer_repeated(
            PointerEvent {
                kind: PointerEventKind::ScrollDown,
                column: body.x,
                row: body.y,
                modifiers: Modifiers::NONE,
            },
            &view,
            50,
        )
        .unwrap(),
        PointerOutcome::Unchanged
    );
}

#[cfg(unix)]
#[test]
fn pointer_focus_uses_insert_for_a_live_terminal_and_preserves_review() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.mode = Mode::Normal;
    app.panes.insert(1, Pane::new(0));
    app.layout = Layout::Split {
        axis: Axis::Horizontal,
        ratio: u16::MAX / 2 + 1,
        first: Box::new(Layout::Pane(0)),
        second: Box::new(Layout::Pane(1)),
    };
    app.active_pane = 1;
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 12,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 10,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let terminal_body = view.pane(0).unwrap().body;
    let document_body = view.pane(1).unwrap().body;

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: terminal_body.x,
            row: terminal_body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active_pane, 0);
    assert_eq!(app.mode, Mode::Insert);
    assert!(!app.terminals.get(terminal).unwrap().reviewing());

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: document_body.x,
            row: document_body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active_pane, 1);
    assert_eq!(app.mode, Mode::Normal);

    app.terminals.get_mut(terminal).unwrap().begin_review();
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: terminal_body.x,
            row: terminal_body.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active_pane, 0);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.terminals.get(terminal).unwrap().reviewing());

    app.handle_key(KeyStroke::char('i')).unwrap();
    assert_eq!(app.mode, Mode::Insert);
    assert!(!app.terminals.get(terminal).unwrap().reviewing());
}

#[test]
fn pointer_resize_requires_the_pointer_to_be_on_the_shared_edge_segment() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "top\nbottom\n");
    app.panes.insert(1, Pane::new(0));
    app.panes.insert(2, Pane::new(0));
    app.layout = Layout::Split {
        axis: Axis::Vertical,
        ratio: u16::MAX / 2 + 1,
        first: Box::new(Layout::Split {
            axis: Axis::Horizontal,
            ratio: u16::MAX / 2 + 1,
            first: Box::new(Layout::Pane(0)),
            second: Box::new(Layout::Pane(2)),
        }),
        second: Box::new(Layout::Pane(1)),
    };
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let left = view.pane(0).unwrap().area;
    let bottom = view.pane(1).unwrap().area;
    let boundary = left.x + left.width;

    assert_eq!(
        pointer_resize_pair(&view, boundary, left.y + 2),
        Some((0, 2, Axis::Horizontal))
    );
    assert_eq!(pointer_pane(&view, boundary, bottom.y + 2), Some(1));
    assert_eq!(pointer_resize_pair(&view, boundary, bottom.y + 2), None);

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: boundary,
            row: bottom.y + 2,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active_pane, 1);

    // A frontend may race a pane close with a previously prepared frame;
    // stale pane identities must be ignored rather than indexed.
    let stale_area = view.pane(2).unwrap().area;
    app.panes.remove(&2);
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::ScrollDown,
            column: stale_area.x + 1,
            row: stale_area.y + 1,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
}

#[test]
fn pointer_respects_prompt_and_insert_ownership_and_cancels_modal_state() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "abcdef");
    let geometry = FrameGeometry {
        screen: Rect {
            width: 30,
            height: 8,
            ..Rect::default()
        },
        editor: Rect {
            width: 30,
            height: 6,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let body = view.pane(0).unwrap().body;
    let click = PointerEvent {
        kind: PointerEventKind::Down(PointerButton::Left),
        column: body.x + 3,
        row: body.y,
        modifiers: Modifiers::NONE,
    };

    app.status = "keep this".to_owned();
    app.status_error = true;
    let before_motion = app.active().selection.clone();
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Moved,
            ..click
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.status, "keep this");
    assert!(app.status_error);
    assert_eq!(app.active().selection, before_motion);

    app.mode = Mode::Command;
    app.command = "write".to_owned();
    let before = app.active().selection.clone();
    app.handle_pointer(click, &view).unwrap();
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.command, "write");
    assert_eq!(app.active().selection, before);

    app.mode = Mode::Insert;
    app.handle_pointer(click, &view).unwrap();
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.active().head(), 3);

    app.mode = Mode::Normal;
    app.replace_active_selection(Selection::point(0));
    app.handle_key(KeyStroke::char('2')).unwrap();
    app.handle_pointer(
        PointerEvent {
            column: body.x,
            ..click
        },
        &view,
    )
    .unwrap();
    app.handle_key(KeyStroke::char('l')).unwrap();
    assert_eq!(app.active().head(), 1, "pointer press clears modal count");

    app.jump = crate::jump_labels::JumpLabels::new([4]);
    app.handle_pointer(
        PointerEvent {
            column: body.x + 2,
            ..click
        },
        &view,
    )
    .unwrap();
    assert!(
        app.jump.is_none(),
        "pointer press cancels stale jump labels"
    );
    app.handle_key(KeyStroke::char('d')).unwrap();
    assert_eq!(app.buffers[0].text().to_string(), "abdef");

    let mut second = App::new(Config::default(), None).unwrap();
    seed(&mut second, "abcdef");
    let second_view = second.prepare_view(geometry);
    let second_body = second_view.pane(0).unwrap().body;
    second
        .handle_pointer(
            PointerEvent {
                kind: PointerEventKind::Down(PointerButton::Left),
                column: second_body.x + second_view.pane(0).unwrap().gutter_width as u16 + 4,
                row: second_body.y,
                modifiers: Modifiers::NONE,
            },
            &second_view,
        )
        .unwrap();
    second
        .handle_pointer(
            PointerEvent {
                kind: PointerEventKind::Drag(PointerButton::Left),
                column: second_body.x + second_view.pane(0).unwrap().gutter_width as u16 + 2,
                row: second_body.y,
                modifiers: Modifiers::NONE,
            },
            &second_view,
        )
        .unwrap();
    second
        .handle_key(KeyStroke::plain(KeyCode::Escape))
        .unwrap();
    second.handle_key(KeyStroke::char('d')).unwrap();
    assert_eq!(second.buffers[0].text().to_string(), "abdef");
}

#[test]
fn direct_pointer_input_cannot_interleave_with_macro_replay() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "abcdef");
    let geometry = FrameGeometry {
        screen: Rect {
            width: 30,
            height: 8,
            ..Rect::default()
        },
        editor: Rect {
            width: 30,
            height: 6,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let body = view.pane(0).unwrap().body;
    app.macros
        .insert('a', vec![InputEvent::Key(KeyStroke::char('l'))]);
    app.replay_macro('a', 1_000).unwrap();
    let selection = app.active().selection.clone();

    let outcome = app
        .handle_pointer_repeated(
            PointerEvent {
                kind: PointerEventKind::Down(PointerButton::Left),
                column: body.x + 5,
                row: body.y,
                modifiers: Modifiers::NONE,
            },
            &view,
            1,
        )
        .unwrap();

    assert_eq!(outcome, PointerOutcome::Unchanged);
    assert_eq!(app.active().selection, selection);
    assert!(app.macro_replay_pending());
}

#[test]
fn pointer_focus_leaves_insert_and_drag_selection_still_enters_select() {
    let mut config = Config::default();
    config.editor.line_numbers = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "abcdef");
    app.split(Axis::Horizontal, None).unwrap();
    app.activate_pane(0);
    app.mode = Mode::Insert;
    let geometry = FrameGeometry {
        screen: Rect {
            width: 40,
            height: 10,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 8,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let destination = view.pane(1).unwrap().body;

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: destination.x + 1,
            row: destination.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active_pane, 1);
    assert_eq!(app.mode, Mode::Normal);

    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Drag(PointerButton::Left),
            column: destination.x + 4,
            row: destination.y,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(app.active().selection.primary(), Range::new(1, 4));
    assert_eq!(app.mode, Mode::Select);
}

#[test]
fn pointer_rows_skip_folded_document_lines() {
    let path = temporary("pointer-fold.rs");
    fs::write(
        &path,
        "fn folded() {\n    let one = 1;\n    let two = 2;\n}\nafter\n",
    )
    .unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.fold_all_syntax();
    let geometry = FrameGeometry {
        screen: Rect {
            width: 50,
            height: 12,
            ..Rect::default()
        },
        editor: Rect {
            width: 50,
            height: 10,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let pane = view.pane(0).unwrap();
    assert_ne!(pane.rows[1].document_row, Some(1));
    let expected_row = pane.rows[1].document_row.expect("a text row");
    app.handle_pointer(
        PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: pane.body.x + pane.gutter_width as u16,
            row: pane.body.y + 1,
            modifiers: Modifiers::NONE,
        },
        &view,
    )
    .unwrap();
    assert_eq!(cursor(&app).row, expected_row);

    fs::remove_file(path).unwrap();
}

#[test]
fn folds_invalidate_in_every_shared_pane_and_gotos_reveal_hidden_targets() {
    let path = temporary("syntax-fold-invalidation.rs");
    fs::write(&path, "fn outer() {\n    let value = 1;\n}\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let folds = app.syntax[0]
        .as_ref()
        .unwrap()
        .folds(app.buffers[0].text(), &app.registry)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.range)
        .collect::<Vec<_>>();
    app.panes.get_mut(&0).unwrap().folds.collapsed = folds.clone();
    app.panes.insert(1, Pane::new(0));
    app.panes.get_mut(&1).unwrap().folds.collapsed = folds;

    app.replace_active_selection(Selection::point(app.buffers[0].line_to_offset(1)));
    app.reveal_active_selection_from_folds();
    assert!(app.panes[&0].folds.collapsed.is_empty());
    assert!(!app.panes[&1].folds.collapsed.is_empty());

    app.replace_active_selection(Selection::point(0));
    app.insert_char(' ');
    assert!(
        app.panes
            .values()
            .all(|pane| pane.folds.collapsed.is_empty())
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn folded_soft_wrap_projection_skips_hidden_rows_for_view_and_scroll() {
    let path = temporary("syntax-fold-wrap.rs");
    fs::write(
        &path,
        "fn outer_with_a_deliberately_long_name() {\n    let hidden = 1;\n}\nlast\n",
    )
    .unwrap();
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    let mut app = App::new(config, Some(path.clone())).unwrap();
    app.fold_all_syntax();
    let geometry = FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 24,
            height: 10,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 24,
            height: 8,
        },
        status: Rect {
            x: 0,
            y: 8,
            width: 24,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 9,
            width: 24,
            height: 1,
        },
    };
    let prepared = app.prepare_view(geometry);
    let rows = &prepared.pane(0).unwrap().rows;
    assert!(rows.iter().any(|row| row.continuation));
    assert!(rows.iter().all(|row| row.document_row != Some(1)));

    while app.active().scroll_row == 0 {
        app.scroll_view(1);
    }
    assert_eq!(app.active().scroll_row, 2);
    fs::remove_file(path).unwrap();
}

#[test]
fn toggle_opens_the_effective_outer_fold_when_nested_ranges_share_an_anchor() {
    let path = temporary("syntax-fold-nested-anchor.rs");
    fs::write(&path, "fn outer() { if ready {\n    hidden();\n}\n}\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    app.fold_all_syntax();
    let before = app.resolved_folds(0);
    assert!(before.iter().filter(|fold| fold.anchor_row == 0).count() >= 2);
    let widest = before
        .iter()
        .filter(|fold| fold.anchor_row == 0)
        .map(|fold| fold.end_hidden_row)
        .max()
        .unwrap();

    app.toggle_syntax_fold();

    let after = app.resolved_folds(0);
    assert!(
        after
            .iter()
            .filter(|fold| fold.anchor_row == 0)
            .all(|fold| fold.end_hidden_row < widest),
        "the visible widest fold must open before a hidden child"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn fold_degradation_status_never_claims_a_partial_projection_is_complete() {
    assert_eq!(fold_degradation_suffix(0, false), "");
    assert!(fold_degradation_suffix(0, true).contains("partial"));
    assert!(fold_degradation_suffix(2, false).contains("degraded"));
    let both = fold_degradation_suffix(2, true);
    assert!(both.contains("partial") && both.contains("2 syntax issue"));
}

#[test]
fn config_commands_and_binding_open_the_registry_backed_buffer() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.config.editor.soft_wrap = true;
    assert!(app.pane_soft_wrap(app.active_pane));
    app.execute_command("config").unwrap();
    assert_eq!(app.active_buffer().display_name(), "[config]");
    assert!(app.active_buffer().is_read_only());
    assert!(!app.pane_soft_wrap(app.active_pane));
    let text = app.active_buffer().to_string();
    assert!(text.contains("Setting"));
    assert!(text.contains("Description"));
    assert!(text.contains("Value"));
    assert!(text.contains("editor.grammar"));
    assert!(text.contains("Input grammar used for editing"));
    assert!(text.contains("commands"));
    assert!(text.contains("runyte"));

    app.execute_command("settings").unwrap();
    assert_eq!(app.active_buffer().display_name(), "[config]");
    press(&mut app, ' ');
    press(&mut app, 'o');
    press(&mut app, 'o');
    assert_eq!(app.active_buffer().display_name(), "[config]");
    assert_eq!(
        app.buffers
            .iter()
            .filter(|buffer| buffer.is_settings())
            .count(),
        1
    );
}

#[test]
fn notification_commands_open_one_complete_buffer_and_acknowledge_history() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.error("git failed\nstdout line\nstderr line");
    assert_eq!(app.unread_notification_counts().errors, 1);

    app.execute_command("not").unwrap();

    assert_eq!(app.active_buffer().display_name(), "[notifications]");
    assert!(app.active_buffer().is_read_only());
    assert!(
        app.active_buffer()
            .to_string()
            .contains("stdout line\nstderr line")
    );
    assert_eq!(
        app.unread_notification_counts(),
        NotificationCounts::default()
    );

    app.execute_command("notifications").unwrap();
    assert_eq!(
        app.buffers
            .iter()
            .filter(|buffer| buffer.is_notifications())
            .count(),
        1
    );
}

/// `acknowledge` in `NotificationCenter` only marks read what already
/// existed when the buffer opened; anything that arrives afterward
/// legitimately raises the count again. This confirms the buffer itself
/// is not a snapshot frozen at open time either: `push_notification`
/// rewrites every open `[notifications]` buffer through
/// `refresh_notification_buffers`, so a notification that arrives while
/// the buffer is still on screen shows up in it without the person
/// having to close and reopen it.
#[test]
fn the_notifications_buffer_refreshes_while_open_and_the_new_entry_counts_as_unread_again() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.error("first failure");
    app.execute_command("not").unwrap();
    assert_eq!(
        app.unread_notification_counts(),
        NotificationCounts::default()
    );
    assert!(!app.active_buffer().to_string().contains("second failure"));

    app.error("second failure");

    assert!(app.active_buffer().to_string().contains("second failure"));
    assert_eq!(app.unread_notification_counts().errors, 1);
}

#[test]
fn delayed_git_failure_never_marks_a_newer_action_echo() {
    let mut app = App::new(Config::default(), None).unwrap();
    let request = GitRequestId::from_raw(99);
    app.git_state.action_origins_mut().insert(request, 1);
    app.action_feedback = Some(ActionFeedback {
        id: 2,
        spelling: "g l".to_owned(),
        text: "g l (Move to line end)".to_owned(),
        is_error: false,
    });

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: request,
        operation: GitOperation::Mutate {
            repository: Repository::new("/repository"),
            mutation: GitMutation::Stage(vec![PathBuf::from("/repository/file")]),
            refresh: RefreshSpec::default(),
        },
        result: Box::new(Err(crate::git::GitError::Failed {
            command: "git add".to_owned(),
            code: Some(1),
            stderr: "refused".to_owned(),
        })),
        state: GitServiceState::Failed,
        coalesced: false,
    });

    assert_eq!(app.displayed_status_message(), "g l (Move to line end)");
    assert_eq!(app.unread_notification_counts().errors, 1);
}

#[test]
fn asynchronous_git_mutation_failure_echoes_its_message_inline() {
    let mut app = App::new(Config::default(), None).unwrap();
    let request = GitRequestId::from_raw(11);
    app.git_state.action_origins_mut().insert(request, 3);
    app.action_feedback = Some(ActionFeedback {
        id: 3,
        spelling: "Tab s".to_owned(),
        text: "Tab s (Stage selected files)".to_owned(),
        is_error: false,
    });

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: request,
        operation: GitOperation::Mutate {
            repository: Repository::new("/repository"),
            mutation: GitMutation::Stage(vec![PathBuf::from("/repository/file")]),
            refresh: RefreshSpec::default(),
        },
        result: Box::new(Err(crate::git::GitError::Failed {
            command: "git add".to_owned(),
            code: Some(1),
            stderr: "refused".to_owned(),
        })),
        state: GitServiceState::Failed,
        coalesced: false,
    });

    assert_eq!(
        app.displayed_status_message(),
        "Tab s (Stage selected files · failed: `git add` failed with status 1: \
             refused; showing the last known Git state)"
    );
    assert!(app.displayed_status_message_is_error());
    assert_eq!(app.unread_notification_counts().errors, 1);
}

#[test]
fn asynchronous_git_one_shot_read_failure_has_no_prior_state_to_show() {
    let mut app = App::new(Config::default(), None).unwrap();

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(12),
        operation: GitOperation::CommitDetail {
            repository: Repository::new("/repository"),
            oid: "a".repeat(40),
        },
        result: Box::new(Err(crate::git::GitError::TooLarge {
            command: "git show".to_owned(),
            limit: 16,
        })),
        state: GitServiceState::Failed,
        coalesced: false,
    });

    assert_eq!(
        app.status,
        "`git show` produced more than 16 bytes of output"
    );
    assert!(app.status_error);
    assert!(!app.git_state.snapshot_stale());
}

#[test]
fn asynchronous_git_success_updates_its_echo_and_retains_multiline_output() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.action_feedback = Some(ActionFeedback {
        id: 7,
        spelling: "Tab c".to_owned(),
        text: "Tab c (Commit staged changes)".to_owned(),
        is_error: false,
    });

    app.apply_git_mutation_result(
        GitMutation::Commit {
            message: "summary".to_owned(),
        },
        Vec::new(),
        Some("[main abc123] summary\n 2 files changed".to_owned()),
        None,
        GitServiceState::Completed,
        Some(7),
    );

    assert_eq!(
        app.displayed_status_message(),
        "Tab c ([main abc123] summary)"
    );
    assert_eq!(app.unread_notification_counts().infos, 1);
    assert!(
        app.notifications.entries()[0]
            .body
            .contains("2 files changed")
    );
}

#[test]
fn successful_worktree_creation_attaches_only_in_persistent_mode() {
    let destination = PathBuf::from("/repository/linked");
    let mutation = || {
        GitMutation::CreateWorktree(WorktreeCreate {
            destination: destination.clone(),
            start: "main".to_owned(),
            new_branch: Some("linked".to_owned()),
        })
    };
    let mut standalone = App::new(Config::default(), None).unwrap();

    standalone.apply_git_mutation_result(
        mutation(),
        Vec::new(),
        None,
        None,
        GitServiceState::Completed,
        None,
    );

    assert!(standalone.take_workspace_switch().is_none());
    assert!(!standalone.should_quit);
    assert!(standalone.status.contains("created worktree"));

    let mut persistent = App::new(Config::default(), None).unwrap();
    persistent.enable_persistent_session();
    persistent.apply_git_mutation_result(
        mutation(),
        Vec::new(),
        None,
        None,
        GitServiceState::Completed,
        None,
    );

    assert!(persistent.should_quit);
    assert_eq!(
        persistent
            .take_workspace_switch()
            .map(|request| request.selector),
        Some(destination)
    );
}

#[test]
fn uncertain_worktree_creation_never_requests_attachment() {
    let destination = PathBuf::from("/repository/uncertain");
    let mut app = App::new(Config::default(), None).unwrap();
    app.enable_persistent_session();

    app.apply_git_mutation_result(
        GitMutation::CreateWorktree(WorktreeCreate {
            destination,
            start: "main".to_owned(),
            new_branch: None,
        }),
        Vec::new(),
        None,
        Some(crate::git::GitError::Failed {
            command: "git worktree add".to_owned(),
            code: None,
            stderr: "outcome unknown".to_owned(),
        }),
        GitServiceState::CompletedWithUncertainState,
        None,
    );

    assert!(app.take_workspace_switch().is_none());
    assert!(!app.should_quit);
    assert!(app.status_error);
}

#[test]
fn notifications_remain_bounded_without_materializing_a_hidden_buffer() {
    let mut app = App::new(Config::default(), None).unwrap();
    let buffers = app.buffers.len();
    for index in 0..10 {
        app.push_notification(NotificationDraft::new(
            NotificationSeverity::Error,
            "Git",
            format!("failure {index}"),
            "x".repeat(crate::notification::MAX_NOTIFICATION_BYTES + 1),
        ));
    }

    assert_eq!(app.buffers.len(), buffers);
    assert!(app.notifications.retained_bytes() <= crate::notification::MAX_HISTORY_BYTES);
}

#[test]
fn config_vertical_motion_ignores_the_global_soft_wrap_setting() {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    let mut app = App::new(config, None).unwrap();
    app.open_settings_buffer();
    app.active_mut().wrap_width = 20;
    set_cursor(&mut app, 2, 0);

    press(&mut app, 'j');
    assert_eq!(cursor(&app), Position::new(3, 0));
    press(&mut app, 'k');
    assert_eq!(cursor(&app), Position::new(2, 0));
}

#[test]
fn enter_on_a_wrapped_config_continuation_opens_that_settings_choices() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_settings_buffer();
    let rows = (0..app.active_buffer().len_lines())
        .filter(|row| {
            app.active_buffer().setting_at(*row) == Some(SettingId::EditorShowHiddenFiles)
        })
        .collect::<Vec<_>>();
    assert!(rows.len() > 1);
    let offset = app.active_buffer().line_to_offset(rows[1]);
    app.active_mut().replace_selection(Selection::point(offset));

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(matches!(app.settings_view, Some(SettingsView::Values(_))));
    assert!(app.list_actions.iter().all(|action| matches!(
        action,
        ListAction::SettingValue {
            setting: SettingId::EditorShowHiddenFiles,
            ..
        }
    )));
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::ResultList)
        .unwrap();
    assert_eq!(
        overlay.layout,
        crate::snapshot::OverlayLayout::SettingChoice
    );
    assert_eq!(overlay.purpose, crate::snapshot::OverlayPurpose::Choice);
}

#[test]
fn hard_wrap_width_setting_uses_a_typed_prompt_and_persists_on_enter() {
    let path = temporary("settings-hard-wrap-width.yaml");
    fs::write(&path, "editor:\n  hard_wrap_width: 80\n").unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);
    app.open_settings_buffer();
    let width = (0..app.active_buffer().len_lines())
        .find(|row| app.active_buffer().setting_at(*row) == Some(SettingId::EditorHardWrapWidth))
        .unwrap();
    let offset = app.active_buffer().line_to_offset(width);
    app.active_mut().replace_selection(Selection::point(offset));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.list.is_none());
    assert_eq!(
        app.prompt_kind,
        PromptKind::SettingValue(SettingId::EditorHardWrapWidth)
    );
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    press(&mut app, '0');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.prompt_kind,
        PromptKind::SettingValue(SettingId::EditorHardWrapWidth)
    );
    assert_eq!(app.config.editor.hard_wrap_width, 80);
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    for digit in ['9', '6'] {
        press(&mut app, digit);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.config.editor.hard_wrap_width, 96);
    assert_eq!(app.persisted_config.editor.hard_wrap_width, 96);
    assert_eq!(app.prompt_kind, PromptKind::Command);
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("hard_wrap_width: 96")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn git_refresh_interval_uses_a_typed_seconds_prompt_and_accepts_zero() {
    let path = temporary("settings-git-refresh-interval.yaml");
    fs::write(&path, "git:\n  refresh_interval_seconds: 5\n").unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);
    app.open_settings_buffer();
    let interval = (0..app.active_buffer().len_lines())
        .find(|row| {
            app.active_buffer().setting_at(*row) == Some(SettingId::GitRefreshIntervalSeconds)
        })
        .unwrap();
    let offset = app.active_buffer().line_to_offset(interval);
    app.active_mut().replace_selection(Selection::point(offset));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.list.is_none(), "the integer choice list was reopened");
    assert_eq!(
        app.prompt_kind,
        PromptKind::SettingValue(SettingId::GitRefreshIntervalSeconds)
    );
    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    press(&mut app, 'x');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.prompt_kind,
        PromptKind::SettingValue(SettingId::GitRefreshIntervalSeconds)
    );
    assert!(app.status.contains("integer from 0 through 3600"));

    key(&mut app, KeyCode::Char('u'), Modifiers::CONTROL);
    press(&mut app, '0');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.config.git.refresh_interval_seconds, 0);
    assert_eq!(app.persisted_config.git.refresh_interval_seconds, 0);
    assert_eq!(app.prompt_kind, PromptKind::Command);
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("refresh_interval_seconds: 0")
    );
    fs::remove_file(path).unwrap();
}

/// The keymap is a reading of configuration, so toggling the option has to
/// move key execution, help, and the hint popup together — which it only
/// does if the registry itself changes.
#[test]
fn toggling_fast_pane_keys_swaps_the_registry_the_editor_answers_from() {
    use crate::keymap::{Key, KeySequence, Lookup};

    let path = temporary("settings-fast-pane-keys.yaml");
    fs::write(&path, "editor:\n  fast_pane_keys: false\n").unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);

    let moves_panes = |app: &App| {
        matches!(
            app.keymap()
                .lookup(Mode::Normal, &KeySequence::from(Key::ctrl('l'))),
            Lookup::Exact(binding)
                if binding.target
                    == BindingTarget::Editor(EditorCommand::FocusWindowRight)
        )
    };
    assert!(!moves_panes(&app));

    app.open_setting_values(SettingId::EditorFastPaneKeys);
    let enabled = app
        .list_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ListAction::SettingValue {
                    value: SettingValue::Boolean(true),
                    ..
                }
            )
        })
        .unwrap();
    app.list.as_mut().unwrap().selected = enabled;

    // A preview is live, and a cancelled preview leaves nothing behind.
    app.preview_selected_setting_value();
    assert!(moves_panes(&app));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(!moves_panes(&app));

    app.open_setting_values(SettingId::EditorFastPaneKeys);
    app.list.as_mut().unwrap().selected = enabled;
    app.preview_selected_setting_value();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.config.editor.fast_pane_keys);
    assert!(moves_panes(&app));
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("fast_pane_keys: true")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn immediate_setting_preview_rolls_back_and_enter_persists_losslessly() {
    let path = temporary("settings-preview.yaml");
    let original = "# keep this comment\nunknown: kept\neditor:\n  line_numbers: true\n";
    fs::write(&path, original).unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);

    app.open_setting_values(SettingId::EditorLineNumbers);
    let disabled = app
        .list_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ListAction::SettingValue {
                    value: SettingValue::Boolean(false),
                    ..
                }
            )
        })
        .unwrap();
    app.list.as_mut().unwrap().selected = disabled;
    app.preview_selected_setting_value();
    assert!(!app.config.editor.line_numbers);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.config.editor.line_numbers);

    app.open_setting_values(SettingId::EditorLineNumbers);
    app.list.as_mut().unwrap().selected = disabled;
    app.preview_selected_setting_value();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("# keep this comment"));
    assert!(saved.contains("unknown: kept"));
    assert!(saved.contains("line_numbers: false"));
    assert!(!app.config.editor.line_numbers);
    assert!(app.list.is_none());
    fs::remove_file(path).unwrap();
}

#[test]
fn focused_theme_setting_previews_without_remembering_and_saves_on_enter() {
    let path = temporary("theme-settings.yaml");
    fs::write(&path, "theme: light\n").unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);

    app.open_setting_values(SettingId::Theme);
    let dark = app
        .list_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ListAction::SettingValue {
                    value: SettingValue::Text(value),
                    ..
                } if value == "dark"
            )
        })
        .unwrap();
    app.list.as_mut().unwrap().selected = dark;
    app.preview_selected_setting_value();
    assert_eq!(app.theme_name, "dark");
    assert_eq!(
        app.terminals.default_colors(),
        DefaultColors::new(Some((0xd6, 0xda, 0xe0)), Some((0x16, 0x18, 0x1d)))
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "theme: light\n");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.theme_name, "light");
    assert_eq!(
        app.terminals.default_colors(),
        DefaultColors::new(Some((0x24, 0x29, 0x2f)), Some((0xfb, 0xfb, 0xfa)))
    );

    app.open_setting_values(SettingId::Theme);
    app.list.as_mut().unwrap().selected = dark;
    app.preview_selected_setting_value();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.theme_name, "dark");
    assert_eq!(
        app.terminals.default_colors(),
        DefaultColors::new(Some((0xd6, 0xda, 0xe0)), Some((0x16, 0x18, 0x1d)))
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "theme: 'dark'\n");
    fs::remove_file(path).unwrap();
}

#[test]
fn restart_required_setting_is_saved_without_claiming_a_live_transition() {
    let mut disabled_config = Config::default();
    disabled_config.lsp.enable = false;
    let mut disabled = App::new(disabled_config, None).unwrap();
    let (disabled_handle, _disabled_commands) = crate::lsp::command_channel();
    disabled.attach_lsp(disabled_handle);
    assert_eq!(
        disabled.effective_setting_value(SettingId::LspEnable),
        SettingValue::Boolean(false),
        "an attached manager does not enable a disabled runtime policy"
    );

    let path = temporary("settings-lsp.yaml");
    fs::write(&path, "lsp:\n  enable: true\n").unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);
    assert!(!app.ports.has_lsp());
    assert_eq!(
        app.effective_setting_value(SettingId::LspEnable),
        SettingValue::Boolean(true),
        "startup policy is effective before the manager attaches"
    );

    app.open_setting_values(SettingId::LspEnable);
    let disabled = app
        .list_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ListAction::SettingValue {
                    value: SettingValue::Boolean(false),
                    ..
                }
            )
        })
        .unwrap();
    app.list.as_mut().unwrap().selected = disabled;
    app.preview_selected_setting_value();
    assert!(
        app.config.lsp.enable,
        "restart-required preview is not live"
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.ports.has_lsp());
    assert!(
        app.config.lsp.enable,
        "runtime LSP policy changes on restart"
    );
    assert!(!app.persisted_config.lsp.enable);
    assert!(app.status.contains("restart Runyte to apply"));
    assert!(fs::read_to_string(&path).unwrap().contains("enable: false"));
    app.open_settings_buffer();
    let lsp_row = (0..app.active_buffer().len_lines())
        .find(|row| app.active_buffer().setting_at(*row) == Some(SettingId::LspEnable))
        .unwrap();
    assert!(app.active_buffer().line_string(lsp_row).contains("false"));
    fs::remove_file(path).unwrap();
}

#[test]
fn workspace_mode_is_visible_and_saved_for_future_launches_only() {
    let path = temporary("settings-workspace-mode.yaml");
    fs::write(&path, "workspace:\n  mode: standalone\n").unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);
    app.open_settings_buffer();

    let row = (0..app.active_buffer().len_lines())
        .find(|row| app.active_buffer().setting_at(*row) == Some(SettingId::WorkspaceMode))
        .unwrap();
    assert!(app.active_buffer().line_string(row).contains("standalone"));

    app.open_setting_values(SettingId::WorkspaceMode);
    let persistent = app
        .list_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ListAction::SettingValue {
                    value: SettingValue::WorkspaceMode(WorkspaceMode::Persistent),
                    ..
                }
            )
        })
        .unwrap();
    app.list.as_mut().unwrap().selected = persistent;
    app.preview_selected_setting_value();
    assert_eq!(
        app.config.workspace.mode,
        WorkspaceMode::Standalone,
        "the launch mode was already selected before App construction"
    );

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(app.config.workspace.mode, WorkspaceMode::Standalone);
    assert_eq!(
        app.persisted_config.workspace.mode,
        WorkspaceMode::Persistent
    );
    assert!(app.status.contains("restart Runyte to apply"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "workspace:\n  mode: persistent\n"
    );
    app.open_settings_buffer();
    let row = (0..app.active_buffer().len_lines())
        .find(|row| app.active_buffer().setting_at(*row) == Some(SettingId::WorkspaceMode))
        .unwrap();
    assert!(app.active_buffer().line_string(row).contains("persistent"));
    fs::remove_file(path).unwrap();
}

#[test]
fn failed_setting_write_keeps_the_picker_but_rolls_back_its_live_preview() {
    let path = temporary("settings-unsafe.yaml");
    let original = "shared: &defaults true\neditor:\n  line_numbers: true\n";
    fs::write(&path, original).unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.note_loaded_config(&path);
    app.open_setting_values(SettingId::EditorLineNumbers);
    let disabled = app
        .list_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ListAction::SettingValue {
                    value: SettingValue::Boolean(false),
                    ..
                }
            )
        })
        .unwrap();
    app.list.as_mut().unwrap().selected = disabled;
    app.preview_selected_setting_value();
    assert!(!app.config.editor.line_numbers);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert!(app.status.contains("preview rolled back"));
    assert!(app.list.is_some());
    assert!(app.config.editor.line_numbers);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.config.editor.line_numbers);
    fs::remove_file(path).unwrap();
}

#[test]
fn missing_config_path_refuses_the_save_and_rolls_back_its_live_preview() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_setting_values(SettingId::EditorLineNumbers);
    let disabled = app
        .list_actions
        .iter()
        .position(|action| {
            matches!(
                action,
                ListAction::SettingValue {
                    value: SettingValue::Boolean(false),
                    ..
                }
            )
        })
        .unwrap();
    app.list.as_mut().unwrap().selected = disabled;
    app.preview_selected_setting_value();
    assert!(!app.config.editor.line_numbers);

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.status_error);
    assert!(app.status.contains("no config path"));
    assert!(app.status.contains("preview rolled back"));
    assert!(app.config.editor.line_numbers);
    assert!(app.list.is_some());
}
