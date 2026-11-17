// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::config::FocusConfig;

#[test]
fn focus_curve_is_symmetric_bounded_and_smooth() {
    let config = FocusConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.radius, 2);
    assert_eq!(config.fade_distance, 50);
    assert_eq!(config.minimum_opacity, 70);
    assert_eq!(config.inactive_opacity, 70);
    assert_eq!(focus_opacity(config, true, Some(40), 40), 1.0);
    assert_eq!(focus_opacity(config, true, Some(40), 42), 1.0);
    assert_eq!(focus_opacity(config, true, Some(40), 92), 0.7);
    assert_eq!(focus_opacity(config, true, None, 0), 0.7);
    assert_eq!(focus_opacity(config, false, Some(40), 40), 0.7);
    let mut previous = 1.0;
    for distance in 0..40 {
        let opacity = focus_opacity(config, true, Some(40), 40 + distance);
        assert_eq!(
            opacity,
            focus_opacity(config, true, Some(40), 40 - distance)
        );
        assert!(opacity <= previous && opacity >= 0.7);
        previous = opacity;
    }
}

#[test]
fn focus_toggles_and_restores_buffer_rendering() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers[0].apply(&Transaction::insert(0, "colored text\n".repeat(40)));
    let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
    let hints = KeyHintState::default();
    let original = terminal
        .draw(|frame| render_test_frame(frame, &mut app, &hints))
        .unwrap()
        .buffer
        .clone();
    for c in ":focus".chars() {
        app.handle_key(crate::input::KeyStroke::char(c)).unwrap();
    }
    app.handle_key(crate::input::KeyStroke::plain(crate::input::KeyCode::Enter))
        .unwrap();
    assert!(app.config.focus.enabled);
    let focused = terminal
        .draw(|frame| render_test_frame(frame, &mut app, &hints))
        .unwrap()
        .buffer
        .clone();
    let prepared = app.prepare_view(frame_geometry(TuiRect::new(0, 0, 60, 24)));
    let snapshot = app.snapshot(&prepared);
    let pane = &snapshot.panes[0];
    let body = to_tui_rect(pane.body);
    assert_eq!(original[(body.x, body.y)], focused[(body.x, body.y)]);
    assert_ne!(
        original[(body.x, body.bottom() - 1)].fg,
        focused[(body.x, body.bottom() - 1)].fg
    );
    for c in ":focus".chars() {
        app.handle_key(crate::input::KeyStroke::char(c)).unwrap();
    }
    app.handle_key(crate::input::KeyStroke::plain(crate::input::KeyCode::Enter))
        .unwrap();
    assert!(!app.config.focus.enabled);
    let restored = terminal
        .draw(|frame| render_test_frame(frame, &mut app, &hints))
        .unwrap()
        .buffer
        .clone();
    for y in body.y..body.bottom() {
        for x in body.x..body.right() {
            assert_eq!(original[(x, y)], restored[(x, y)]);
        }
    }
}

#[test]
fn focus_preserves_active_terminals_and_fades_inactive_terminal_colors() {
    let mut app = App::new(Config::default(), None).unwrap();
    let prepared = app.prepare_view(frame_geometry(TuiRect::new(0, 0, 30, 12)));
    let mut pane = app.snapshot(&prepared).panes.remove(0);
    let theme = TuiTheme::new(&app.theme);
    let body = to_tui_rect(pane.body);
    let mut emulator =
        crate::terminal::emulator::Emulator::new(usize::from(body.width), usize::from(body.height));
    emulator.feed(b"\x1b[31;44;4mhello\r\nworld");
    pane.terminal = Some(TerminalView {
        revision: 0,
        columns: usize::from(body.width),
        rows: (0..usize::from(body.height))
            .map(|r| emulator.grid().line(r).unwrap().clone())
            .collect(),
        line_ids: vec![],
        cursor: Some((0, 0)),
        scrollback: 0,
        live: true,
        review: false,
        newer_output: false,
        highlights: vec![],
    });
    let mut terminal = Terminal::new(TestBackend::new(30, 12)).unwrap();
    let config = FocusConfig {
        enabled: true,
        radius: 0,
        fade_distance: 1,
        minimum_opacity: 0,
        inactive_opacity: 0,
    };
    for review in [false, true] {
        for cursor in [None, Some((0, 0)), Some((4, 2))] {
            let view = pane.terminal.as_mut().unwrap();
            view.review = review;
            view.cursor = cursor;
            let original = terminal
                .draw(|f| draw_pane(f, &theme, Mode::Normal, &pane))
                .unwrap()
                .buffer
                .clone();
            let focused = terminal
                .draw(|f| {
                    draw_pane(f, &theme, Mode::Normal, &pane);
                    apply_focus(f, &theme, &pane, Some(config));
                })
                .unwrap()
                .buffer
                .clone();
            assert_eq!(focused, original, "review={review}, cursor={cursor:?}");
        }
    }
    pane.active = false;
    let faded = terminal
        .draw(|f| {
            draw_pane(f, &theme, Mode::Normal, &pane);
            apply_focus(f, &theme, &pane, Some(config));
        })
        .unwrap()
        .buffer
        .clone();
    assert_eq!(faded[(body.x, body.y)].fg, theme.inactive_background);
    assert_eq!(faded[(body.x, body.y)].bg, theme.inactive_background);
    assert_eq!(
        faded[(body.x, body.y + 1)].underline_color,
        theme.inactive_background
    );
    assert_eq!(faded[(body.x, body.y + 1)].symbol(), "w");
}

#[test]
fn focus_uses_wrapped_screen_rows_and_matches_attached_and_native_renderers() {
    let mut config = Config::default();
    config.editor.soft_wrap = true;
    config.focus.radius = 0;
    config.focus.fade_distance = 2;
    let mut app = App::new(config, None).unwrap();
    app.buffers[0].apply(&Transaction::insert(0, "word ".repeat(100)));
    app.config.focus.enabled = true;
    let mut terminal = Terminal::new(TestBackend::new(30, 12)).unwrap();
    let direct = terminal
        .draw(|f| render_test_frame(f, &mut app, &KeyHintState::default()))
        .unwrap()
        .buffer
        .clone();
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let frame = host.prepare_frame(frame_geometry(TuiRect::new(0, 0, 30, 12)));
    let pane = &frame.editor.panes[0];
    assert_eq!(pane.cursor_screen_row, Some(0));
    assert!(
        pane.rows
            .iter()
            .filter(|r| matches!(r, SnapshotRow::Text(_)))
            .count()
            > 2
    );
    let body = to_tui_rect(pane.body);
    let x = body.x + pane.gutter_width as u16 + 2;
    assert_ne!(direct[(x, body.y)].fg, direct[(x, body.y + 2)].fg);
    for native in [false, true] {
        let attached = terminal
            .draw(|f| {
                if native {
                    render_native_frame(
                        f,
                        &frame,
                        SessionMode::Standalone,
                        TerminalColorDepth::TrueColor,
                    );
                } else {
                    render_host_frame_exact_colors_for_test(f, &frame);
                }
            })
            .unwrap()
            .buffer
            .clone();
        for y in body.y..body.bottom() {
            for x in body.x..body.right() {
                assert_eq!(direct[(x, y)], attached[(x, y)]);
            }
        }
    }
}

#[test]
fn focus_bakes_dim_lightness_for_black_white_and_saturated_colors() {
    assert_eq!(dim_terminal_rgb([0, 0, 0]), [0, 0, 0]);
    assert_eq!(dim_terminal_rgb([255, 255, 255]), [166, 166, 166]);
    assert_eq!(dim_terminal_rgb([255, 0, 0]), [166, 0, 0]);
    assert_eq!(dim_terminal_rgb([255, 128, 128]), [249, 0, 0]);
}

#[test]
fn focus_colors_survive_backend_output_inside_another_terminal() {
    use ratatui::backend::{Backend, CrosstermBackend};
    use ratatui::style::Color;

    // Crossterm suppresses colours under NO_COLOR. Run this boundary test
    // in an isolated child when needed, without mutating the test process.
    if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        let root = crate::test_support::TestRuntimeRoot::new("focus-color").unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        crate::test_support::isolate_user_paths(&mut command, root.path());
        let output = command
            .env_remove("NO_COLOR")
            .args([
                "--exact",
                "ui::tests::focus::focus_colors_survive_backend_output_inside_another_terminal",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let mut app = App::new(Config::default(), None).unwrap();
    let prepared = app.prepare_view(frame_geometry(TuiRect::new(0, 0, 30, 12)));
    let mut pane = app.snapshot(&prepared).panes.remove(0);
    pane.active = false;
    let mut theme = TuiTheme::new(&app.theme);
    theme.inactive_background = Color::Rgb(0, 0, 0);
    let body = to_tui_rect(pane.body);
    let mut terminal = Terminal::new(TestBackend::new(30, 12)).unwrap();
    let rendered = terminal
        .draw(|frame| {
            let cell = &mut frame.buffer_mut()[(body.x, body.y)];
            cell.set_symbol("x");
            cell.fg = Color::Rgb(200, 220, 240);
            cell.bg = Color::Rgb(40, 60, 80);
            // These faded channels are also valid, destructive SGR commands.
            cell.underline_color = Color::Rgb(62, 84, 14);
            // Pinned at 50% so the faded channels stay the SGR codes above.
            let focus = FocusConfig {
                inactive_opacity: 50,
                ..FocusConfig::default()
            };
            apply_focus(frame, &theme, &pane, Some(focus));
        })
        .unwrap()
        .buffer
        .clone();
    let expected = &rendered[(body.x, body.y)];
    let mut bytes = Vec::new();
    CrosstermBackend::new(&mut bytes)
        .draw(std::iter::once((body.x, body.y, expected)))
        .unwrap();
    let mut emulator = crate::terminal::emulator::Emulator::new(30, 12);
    emulator.feed(&bytes);
    let actual = &emulator.grid().line(usize::from(body.y)).unwrap()[usize::from(body.x)];
    assert_eq!(
        actual.foreground,
        crate::terminal::grid::Color::Rgb(100, 110, 120)
    );
    assert_eq!(
        actual.background,
        crate::terminal::grid::Color::Rgb(20, 30, 40)
    );
    assert_eq!(
        actual.attributes,
        crate::terminal::grid::Attributes::default()
    );
}
