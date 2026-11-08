// SPDX-License-Identifier: MPL-2.0

use super::super::*;

fn contrast(left: Color, right: Color) -> f64 {
    let left = left.relative_luminance().unwrap();
    let right = right.relative_luminance().unwrap();
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

/// CIE76 over CIELAB, the measure the other selection tests use: a selection
/// ground can differ from its background by hue alone.
fn distance(left: Color, right: Color) -> f64 {
    fn lab(color: Color) -> [f64; 3] {
        let (r, g, b) = color.channels().unwrap();
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (channel(r), channel(g), channel(b));
        let transfer = |value: f64| {
            if value > 0.008_856 {
                value.cbrt()
            } else {
                7.787 * value + 16.0 / 116.0
            }
        };
        let x = transfer((0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047);
        let y = transfer(0.2126 * r + 0.7152 * g + 0.0722 * b);
        let z = transfer((0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883);
        [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
    }
    let (left, right) = (lab(left), lab(right));
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn imported_names() -> Vec<&'static str> {
    super::PALETTES
        .iter()
        .chain(&nightfox::VARIANTS)
        .map(|palette| palette.name)
        .collect()
}

/// `solarized-light`'s text is below 4.5:1 even on white, so it keeps most of
/// its own contrast instead of holding the floor the others do.
#[test]
fn imported_selection_grounds_stay_readable_and_visible() {
    let config = Config::default();
    for name in imported_names() {
        let theme = config.resolve_theme(name).unwrap();
        let floor = if name == "solarized-light" {
            contrast(theme.foreground, theme.background) * 0.86
        } else {
            4.5
        };
        for (label, ground) in [
            ("selection", theme.selection),
            ("primary", theme.selection_primary),
        ] {
            assert!(
                contrast(theme.foreground, ground) >= floor,
                "{name}: text on {label}"
            );
            assert!(
                contrast(theme.jump_text_muted, ground) >= 3.0,
                "{name}: dimmed text on {label}"
            );
            assert!(
                distance(ground, theme.background) >= 18.0,
                "{name}: {label} disappears into the background"
            );
        }
        assert!(
            distance(theme.selection, theme.selection_primary) >= 26.0,
            "{name}: selection grounds look alike"
        );
    }
}

#[test]
fn imported_carets_and_notifications_stand_off_the_background() {
    let config = Config::default();
    for name in imported_names() {
        let theme = config.resolve_theme(name).unwrap();
        for (mode, caret) in [
            ("normal", theme.cursor_normal),
            ("insert", theme.cursor_insert),
            ("select", theme.cursor_select),
            ("command", theme.cursor_command),
            ("error", theme.error),
            ("warning", theme.warning),
            ("info", theme.info),
        ] {
            assert!(
                contrast(caret, theme.background) >= 3.0,
                "{name}: {mode} colour"
            );
        }
    }
}

#[test]
fn every_imported_palette_carries_a_terminal_table() {
    let config = Config::default();
    for name in imported_names() {
        assert_eq!(config.themes[name].terminal.len(), 16, "{name}");
    }
}

#[test]
fn imported_palettes_keep_their_published_colours() {
    let config = Config::default();
    let rgb = |value: u32| {
        Color::Rgb(
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
        )
    };
    let keyword = crate::syntax::Scope::named("keyword").unwrap();
    for (name, background, foreground, keyword_color, ansi_red) in [
        ("dracula", 0x282a36, 0xf8f8f2, 0xff79c6, 0xff5555),
        ("nord", 0x2e3440, 0xd8dee9, 0x81a1c1, 0xbf616a),
        ("solarized-light", 0xfdf6e3, 0x657b83, 0x859900, 0xdc322f),
        ("tomorrow-night", 0x1d1f21, 0xc5c8c6, 0xb294bb, 0xcc6666),
        ("kanagawa-wave", 0x1f1f28, 0xdcd7ba, 0x957fb8, 0xc34043),
        ("rose-pine-moon", 0x232136, 0xe0def4, 0x3e8fb0, 0xeb6f92),
        ("nightfox", 0x192330, 0xcdcecf, 0x9d79d6, 0xc94f6d),
        ("github-dark", 0x0d1117, 0xe6edf3, 0xff7b72, 0xff7b72),
        ("atom-one-dark", 0x282c34, 0xabb2bf, 0xc678dd, 0xe06c75),
    ] {
        let theme = config.resolve_theme(name).unwrap();
        assert_eq!(theme.background, rgb(background), "{name}");
        assert_eq!(theme.foreground, rgb(foreground), "{name}");
        assert_eq!(
            theme.syntax_color(keyword),
            Some(rgb(keyword_color)),
            "{name}"
        );
        assert_eq!(theme.terminal[1], rgb(ansi_red), "{name}");
    }

    let dracula = config.resolve_theme("dracula").unwrap();
    assert_eq!(
        dracula.terminal[1..=6],
        [0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd].map(rgb)
    );
    assert_eq!(
        dracula.terminal[9..=14],
        [0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff].map(rgb)
    );
    let doom = config.resolve_theme("doom-one").unwrap();
    assert_eq!(doom.terminal[9], rgb(0xff8281));
    assert_eq!(doom.terminal[15], rgb(0xdfdfdf));
}
