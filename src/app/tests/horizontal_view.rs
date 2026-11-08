// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn newline_and_wrap_prefer_left_edge_but_keep_caret_visible() {
    let geometry = FrameGeometry {
        screen: Rect {
            width: 24,
            height: 12,
            ..Rect::default()
        },
        editor: Rect {
            width: 24,
            height: 10,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    for indentation in [
        "    ".to_owned(),
        "\t\t".to_owned(),
        " ".repeat(30),
        "\t".repeat(12),
    ] {
        let mut config = Config::default();
        config.editor.soft_wrap = false;
        config.editor.smart_newline = false;
        let mut app = App::new(config, None).unwrap();
        seed(&mut app, &format!("{indentation}{}", "x".repeat(60)));
        app.mode = Mode::Insert;
        set_cursor(&mut app, 0, indentation.chars().count() + 60);
        app.prepare_view(geometry);
        assert!(app.active().scroll_col > 0);
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        app.prepare_view(geometry);
        let position = cursor(&app);
        assert_eq!(position, Position::new(1, indentation.chars().count()));
        let width = app.active().wrap_width;
        let scroll = app.active().scroll_col;
        assert_eq!(
            scroll == 0,
            crate::wrap::display_column(&indentation, position.col, 4) < width
        );
        assert!(crate::wrap::cells_from_column(&indentation, scroll, position.col, 4) < width);
        if scroll > 0 {
            assert!(
                crate::wrap::cells_from_column(&indentation, scroll - 1, position.col, 4) >= width
            );
        }
        app.prepare_view(geometry);
        assert_eq!(app.active().scroll_col, scroll);
    }

    for last_word in [
        "gamma",
        "界界界界界界界界界界界界",
        "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}",
    ] {
        let mut config = Config::default();
        config.editor.soft_wrap = false;
        config.editor.hard_wrap_width = 30;
        let mut app = App::new(config, None).unwrap();
        let source = format!("{} {last_word}", "alpha beta ".repeat(8));
        seed(&mut app, &source);
        app.mode = Mode::Select;
        app.replace_active_selection(Selection::single(Range::new(0, source.chars().count() - 1)));
        app.prepare_view(geometry);
        assert!(app.active().scroll_col > 0);
        for stroke in [' ', 'p', 'w'] {
            press(&mut app, stroke);
        }
        app.prepare_view(geometry);
        let scroll = app.active().scroll_col;
        let position = cursor(&app);
        let line = app.active_buffer().line_string(position.row);
        let width = app.active().wrap_width;
        assert!(crate::wrap::cells_from_column(&line, scroll, position.col, 4) < width);
        if last_word == "gamma" {
            assert_eq!(scroll, 0);
        }
        app.prepare_view(geometry);
        assert_eq!(app.active().scroll_col, scroll);
        // An already-satisfied wrap does not move the view.
        app.active_mut().scroll_col = position.col;
        for stroke in [' ', 'p', 'w'] {
            press(&mut app, stroke);
        }
        app.prepare_view(geometry);
        assert_eq!(app.active().scroll_col, position.col);
        for stroke in [' ', 'p', 's'] {
            press(&mut app, stroke);
        }
        app.prepare_view(geometry);
        assert_eq!(app.active().scroll_col, 0);
        for stroke in [' ', 'p', 's'] {
            press(&mut app, stroke);
        }
        app.prepare_view(geometry);
        assert_eq!(app.active().scroll_col, scroll);
    }
}

#[test]
fn ordinary_horizontal_motion_retains_scroll_until_caret_needs_space() {
    let mut config = Config::default();
    config.editor.soft_wrap = false;
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, &"x".repeat(80));
    set_cursor(&mut app, 0, 12);
    app.active_mut().scroll_col = 8;
    let geometry = FrameGeometry {
        screen: Rect {
            width: 24,
            height: 12,
            ..Rect::default()
        },
        editor: Rect {
            width: 24,
            height: 10,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    app.prepare_view(geometry);
    press(&mut app, 'l');
    app.prepare_view(geometry);
    assert_eq!(app.active().scroll_col, 8);
    assert_eq!(cursor(&app), Position::new(0, 13));
    press(&mut app, 'h');
    app.prepare_view(geometry);
    assert_eq!(app.active().scroll_col, 8);
}

fn prepare(app: &mut App) {
    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 60,
            height: 16,
            ..Rect::default()
        },
        editor: Rect {
            width: 60,
            height: 14,
            ..Rect::default()
        },
        ..FrameGeometry::default()
    });
}

#[test]
fn rejected_and_noop_wraps_preserve_the_horizontal_view() {
    for readonly in [false, true] {
        let mut config = Config::default();
        config.editor.soft_wrap = false;
        let mut app = App::new(config, None).unwrap();
        seed(&mut app, "alpha beta gamma");
        if readonly {
            app.buffers[0] = Buffer::virtual_text("[readonly]", "alpha beta gamma");
            app.config.editor.hard_wrap_width = 6;
        }
        app.replace_active_selection(Selection::single(Range::new(0, 14)));
        app.active_mut().scroll_col = 8;
        prepare(&mut app);
        app.hard_wrap_selections(app.config.editor.hard_wrap_width);
        prepare(&mut app);
        assert_eq!(app.active().scroll_col, 8);
        assert!(app.active().horizontal_reset.is_none());
        assert_eq!(text(&app), "alpha beta gamma");
    }
}

#[test]
fn line_structure_edits_request_reset_but_ordinary_edits_do_not() {
    for action in [
        "reflow",
        "open-above",
        "open-below",
        "paste",
        "join",
        "type",
    ] {
        let mut config = Config::default();
        config.editor.soft_wrap = false;
        config.editor.hard_wrap_width = 16;
        config.editor.scratch_markdown = false;
        let mut app = App::new(config, None).unwrap();
        seed(&mut app, "    alpha beta gamma delta epsilon\nsecond");
        set_cursor(&mut app, 0, 18);
        app.active_mut().scroll_col = 10;
        prepare(&mut app);
        match action {
            "reflow" => {
                app.replace_active_selection(Selection::single(Range::new(0, 32)));
                for stroke in [' ', 'p', 'r'] {
                    press(&mut app, stroke);
                }
            }
            "open-above" => press(&mut app, 'O'),
            "open-below" => press(&mut app, 'o'),
            "paste" => {
                app.mode = Mode::Insert;
                app.handle_input(InputEvent::Text("one\ntwo".into()))
                    .unwrap();
            }
            "join" => press(&mut app, 'J'),
            "type" => {
                app.mode = Mode::Insert;
                press(&mut app, 'x');
            }
            _ => unreachable!(),
        }
        assert_eq!(
            app.active().horizontal_reset.is_some(),
            action != "type",
            "{action}"
        );
        prepare(&mut app);
        assert_eq!(
            app.active().scroll_col,
            if action == "type" { 10 } else { 0 },
            "{action}"
        );
        assert!(app.active().horizontal_reset.is_none());
    }
}

#[test]
fn effective_soft_wrap_changes_reset_all_panes_including_configuration_changes() {
    for via_key in [false, true] {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        let mut app = App::new(config, None).unwrap();
        seed(
            &mut app,
            "| A | B | C |\n| --- | --- | --- |\n| alpha | beta | gamma |\n",
        );
        press(&mut app, '?');
        app.split(Axis::Horizontal, None).unwrap();
        prepare(&mut app);
        for pane in app.panes.values_mut() {
            // A wrapped table origin is measured in cells, not characters.
            pane.scroll_col = 7;
            pane.selection = Selection::point(10);
        }
        if via_key {
            for stroke in [' ', 'p', 's'] {
                press(&mut app, stroke);
            }
        } else {
            // The settings page and reload both update this same config value.
            app.config.editor.soft_wrap = false;
        }
        prepare(&mut app);
        assert!(app.panes.values().all(|pane| pane.scroll_col == 0));
    }
}

#[test]
fn soft_wrap_toggle_does_not_reset_unwrapped_diff_panes() {
    use crate::diff_view::{DiffSession, DiffSide};
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, &"x".repeat(120));
    app.split(Axis::Horizontal, None).unwrap();
    let ids: Vec<_> = app.panes.keys().copied().collect();
    app.diffs.push(DiffSession::new(
        DiffSide {
            pane: ids[0],
            buffer: 0,
        },
        DiffSide {
            pane: ids[1],
            buffer: 0,
        },
        &text(&app),
        &text(&app),
    ));
    prepare(&mut app);
    for pane in app.panes.values_mut() {
        pane.selection = Selection::point(30);
        pane.scroll_col = 20;
    }
    for stroke in [' ', 'p', 's'] {
        press(&mut app, stroke);
    }
    // The key must not mutate either side; effective wrapping remains off.
    assert!(app.panes.values().all(|pane| pane.scroll_col == 20));
    assert!(ids.iter().all(|id| !app.pane_soft_wrap(*id)));
    prepare(&mut app);
    // Existing diff settlement aligns both sides at zero independently of
    // soft wrapping, so the reported visible mismatch was not reproducible.
    assert!(app.panes.values().all(|pane| pane.scroll_col == 0));
}

#[test]
fn hostile_zero_width_lines_use_a_bounded_suffix_and_cache_idle_checks() {
    for character in ['\u{200b}', '\u{301}'] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.config.editor.soft_wrap = false;
        seed(&mut app, &character.to_string().repeat(100_000));
        app.mode = Mode::Insert;
        set_cursor(&mut app, 0, 100_000);
        prepare(&mut app);
        assert!(app.active().scroll_col >= 100_000 - 4096 - crate::grapheme::MAX_CLUSTER_CHARS);
        let checked = app.active().horizontal_check;
        for _ in 0..100 {
            prepare(&mut app);
        }
        assert_eq!(app.active().horizontal_check, checked);
        assert_eq!(checked.unwrap().revision, app.active_buffer().revision());
        press(&mut app, 'x');
        prepare(&mut app);
        assert_ne!(app.active().horizontal_check, checked);
    }
}
