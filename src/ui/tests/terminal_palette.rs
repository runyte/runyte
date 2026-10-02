// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    config::Config,
    settings::{SettingId, SettingValue},
    terminal::{Color, emulator::Emulator},
};
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn terminal_palette_resolves_all_sgr_forms_before_client_depth_adaptation() {
    let mut emulator = Emulator::new(64, 1);
    for index in 0..16 {
        let fg = if index < 8 {
            30 + index
        } else {
            90 + index - 8
        };
        let bg = fg + 10;
        emulator.feed(format!("\x1b[{fg};{bg}mx\x1b[38;5;{index};48;5;{index}my").as_bytes());
    }
    let cells = emulator.grid().line(0).unwrap();
    let source = Config::default().resolve_theme("latte").unwrap();
    for depth in [
        TerminalColorDepth::TrueColor,
        TerminalColorDepth::Indexed,
        TerminalColorDepth::Basic,
    ] {
        let mut theme = TuiTheme::with_color_depth(&source, depth);
        for index in 0..16 {
            for offset in [0, 1] {
                let cell = &cells[index * 2 + offset];
                assert_eq!(cell.foreground, Color::Indexed(index as u8));
                assert_eq!(cell.background, Color::Indexed(index as u8));
                let style = terminal_style(&theme, true, cell);
                let expected = to_tui_color_for(source.terminal[index], depth);
                assert_eq!(style.fg, Some(expected));
                assert_eq!(style.bg, Some(expected));
            }
        }
        for enabled in [true, false] {
            theme.terminal_theme_colors = enabled;
            for index in 16..=255 {
                let expected = if depth == TerminalColorDepth::Basic {
                    let [r, g, b] = xterm_color(index);
                    depth.rgb(r, g, b)
                } else {
                    ratatui::style::Color::Indexed(index)
                };
                assert_eq!(
                    terminal_color(&theme, Color::Indexed(index), theme.foreground),
                    expected
                );
            }
            assert_eq!(
                terminal_color(&theme, Color::Rgb(1, 2, 3), theme.foreground),
                depth.rgb(1, 2, 3)
            );
            assert_eq!(
                terminal_color(&theme, Color::Default, theme.foreground),
                theme.foreground
            );
        }
        for index in 0..16 {
            let expected = if depth == TerminalColorDepth::Basic {
                let [r, g, b] = xterm_color(index);
                depth.rgb(r, g, b)
            } else {
                ratatui::style::Color::Indexed(index)
            };
            assert_eq!(
                terminal_color(&theme, Color::Indexed(index), theme.foreground),
                expected
            );
        }
    }
}

#[test]
fn terminal_palette_live_setting_and_theme_switch_repaint_both_frontends() {
    let mut app = App::new(Config::default(), None).unwrap();
    let config_path = std::env::temp_dir().join(format!(
        "runyte-terminal-palette-{}.yaml",
        std::process::id()
    ));
    // Theme assignment below is presentation-only; keep even future config
    // persistence confined to this fixture's temporary path.
    app.note_loaded_config(&config_path);
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let cell = TerminalCell {
        character: 'x',
        foreground: Color::Indexed(1),
        background: Color::Indexed(4),
        ..TerminalCell::default()
    };
    let mut backend = Terminal::new(TestBackend::new(80, 24)).unwrap();
    for name in ["ocean-dark", "ocean-light"] {
        host.theme = host.config.resolve_theme(name).unwrap();
        for enabled in [true, false, true] {
            SettingId::EditorTerminalThemeColors
                .apply(&SettingValue::Boolean(enabled), &mut host.config)
                .unwrap();
            let mut snapshot = host.prepare_frame(frame_geometry(TuiRect::new(0, 0, 80, 24)));
            assert_eq!(snapshot.editor.terminal_theme_colors, enabled);
            let pane = &mut snapshot.editor.panes[0];
            let (x, y) = (pane.body.x, pane.body.y);
            pane.terminal = Some(TerminalView {
                revision: 1,
                columns: 1,
                rows: vec![vec![cell]],
                line_ids: vec![None],
                cursor: None,
                scrollback: 0,
                live: false,
                review: true,
                newer_output: false,
                highlights: Vec::new(),
            });
            let wire: crate::protocol::HostFrame = snapshot.clone().into();
            let decoded: crate::protocol::HostFrame =
                serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap();
            let attached: HostFrame = decoded.try_into().unwrap();
            for depth in [
                TerminalColorDepth::TrueColor,
                TerminalColorDepth::Indexed,
                TerminalColorDepth::Basic,
            ] {
                let mut theme = TuiTheme::with_color_depth(&host.theme, depth);
                theme.terminal_theme_colors = enabled;
                let expected = terminal_style(&theme, true, &cell);
                for remote in [false, true] {
                    backend
                        .draw(|frame| {
                            if remote {
                                render_host_frame(frame, &attached, depth);
                            } else {
                                render_editor_frame(
                                    frame,
                                    &host,
                                    &snapshot.editor,
                                    &KeyHintState::default(),
                                    depth,
                                );
                            }
                        })
                        .unwrap();
                    let painted = &backend.backend().buffer()[(x, y)];
                    assert_eq!(painted.symbol(), "x");
                    assert_eq!(Some(painted.fg), expected.fg);
                    assert_eq!(Some(painted.bg), expected.bg);
                }
                let preview = terminal_preview_lines(
                    &theme,
                    snapshot.editor.panes[0].terminal.as_ref().unwrap(),
                    1,
                );
                assert_eq!(preview[0].spans[0].style.fg, expected.fg);
            }
            assert_eq!(
                snapshot.editor.panes[0].terminal.as_ref().unwrap().rows[0][0],
                cell
            );
        }
    }
}
