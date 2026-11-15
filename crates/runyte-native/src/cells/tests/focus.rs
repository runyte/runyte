// SPDX-License-Identifier: MPL-2.0
use super::*;

#[test]
fn focus_zero_opacity_dim_glyphs_match_the_native_background() {
    use runyte::{
        app::App,
        config::{Config, FocusConfig},
        terminal::{Attributes, Cell, TerminalView},
        workspace::WorkspaceHost,
    };
    let root = tempfile::tempdir().unwrap();
    let app = App::new_in_project(
        runyte::test_support::isolated_startup_config(Config::default()),
        None,
        root.path(),
    )
    .unwrap();
    let mut host = WorkspaceHost::new(app);
    let mut snapshot = host.prepare_frame(runyte::ui::frame_geometry(Rect::new(0, 0, 30, 12)));
    snapshot.editor.focus = Some(FocusConfig {
        radius: 0,
        fade_distance: 1,
        minimum_opacity: 0,
        inactive_opacity: 0,
    });
    let body = snapshot.editor.panes[0].body;
    let mut cell = Cell {
        character: 'x',
        foreground: runyte::terminal::Color::Rgb(240, 100, 80),
        background: runyte::terminal::Color::Rgb(80, 120, 200),
        attributes: Attributes::DIM,
        ..Cell::default()
    };
    for reverse in [false, true] {
        if reverse {
            cell.attributes.insert(Attributes::REVERSE);
        }
        snapshot.editor.panes[0].terminal = Some(TerminalView {
            revision: 0,
            columns: usize::from(body.width),
            rows: vec![vec![cell; usize::from(body.width)]; usize::from(body.height)],
            line_ids: vec![],
            cursor: Some((0, 0)),
            scrollback: 0,
            live: true,
            review: false,
            newer_output: false,
            highlights: vec![],
        });
        for active in [true, false] {
            snapshot.editor.panes[0].active = active;
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 12)).unwrap();
            let rendered = terminal
                .draw(|f| {
                    runyte::ui::render_native_frame(
                        f,
                        &snapshot,
                        runyte::ui::SessionMode::Standalone,
                        runyte::ui::TerminalColorDepth::TrueColor,
                    )
                })
                .unwrap()
                .buffer
                .clone();
            let mut native = frame();
            native.cells = rendered.into();
            let mut glyphs = super::super::GlyphCache::default();
            glyphs.prepare(&native);
            // Sample away from the cursor: active terminals preserve even
            // DIM/REVERSED glyphs, while inactive ones vanish at zero opacity.
            let y = body.y + 1;
            let row = glyphs.rows[usize::from(y)].as_ref().unwrap();
            let glyph = row.cells.iter().find(|c| c.x == body.x + 1).unwrap();
            let painted = &native.cells[(body.x + 1, y)];
            if active {
                let foreground = if reverse { painted.bg } else { painted.fg };
                let mut expected = color(foreground, native.foreground);
                expected.l *= 0.65;
                assert_eq!(glyph.fg, expected);
                assert!(glyph.modifier.contains(Modifier::DIM));
            } else {
                let ground = if reverse { painted.fg } else { painted.bg };
                assert_eq!(glyph.fg, color(ground, native.background));
                assert!(!glyph.modifier.contains(Modifier::DIM));
            }
        }
    }
}
