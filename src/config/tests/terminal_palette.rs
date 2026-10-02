// SPDX-License-Identifier: MPL-2.0

use super::super::*;

#[test]
fn terminal_palette_defaults_and_partial_overrides_follow_roles() {
    let mut definition = ThemeDefinition {
        background: "#101010".into(),
        error: "#e06060".into(),
        warning: Some("#d0a040".into()),
        change_added: Some("#60b060".into()),
        ..ThemeDefinition::default()
    };
    let derived = Theme::try_from(&definition).unwrap();
    assert_eq!(derived.terminal[1], derived.error);
    assert_eq!(derived.terminal[2], derived.change_added);
    assert_eq!(derived.terminal[3], derived.warning);
    assert_ne!(derived.terminal[9], derived.terminal[1]);
    definition.terminal.insert("red".into(), "#010203".into());
    definition
        .terminal
        .insert("bright_cyan".into(), "reset".into());
    let custom = Theme::try_from(&definition).unwrap();
    assert_eq!(custom.terminal[1], Color::Rgb(1, 2, 3));
    assert_eq!(custom.terminal[14], Color::Reset);
    assert_eq!(custom.terminal[9], derived.terminal[9]);
    assert_eq!(custom.terminal[2], derived.terminal[2]);
    definition.background = "reset".into();
    let reset = Theme::try_from(&definition).unwrap();
    assert_eq!(reset.terminal[1], Color::Rgb(1, 2, 3));
    assert_eq!(reset.terminal[9], reset.error);
}

#[test]
fn terminal_palette_rejects_unknown_names_and_invalid_colours() {
    for (name, value, message) in [
        (
            "bright_redd",
            "#123456",
            "unknown terminal colour 'bright_redd'",
        ),
        ("red", "#oops", "invalid"),
    ] {
        let mut definition = ThemeDefinition::default();
        definition.terminal.insert(name.into(), value.into());
        let error = Theme::try_from(&definition).unwrap_err().to_string();
        assert!(error.contains(message), "{error}");
    }
    let definition: ThemeDefinition =
        serde_yaml::from_str("terminal: {red: '#123456', bright_white: blue}").unwrap();
    let theme = Theme::try_from(&definition).unwrap();
    assert_eq!(theme.terminal[1], Color::Rgb(0x12, 0x34, 0x56));
    assert_eq!(theme.terminal[15], Color::Blue);
}

#[test]
fn bundled_terminal_palettes_are_legible_on_their_own_ground() {
    let config = Config::default();
    for name in config.theme_names() {
        let theme = config.resolve_theme(name).unwrap();
        let ground = theme.background.relative_luminance().unwrap();
        for (index, color) in theme.terminal.iter().enumerate() {
            assert!(matches!(color, Color::Rgb(..)), "{name} index {index}");
            let text = color.relative_luminance().unwrap();
            let contrast = (ground.max(text) + 0.05) / (ground.min(text) + 0.05);
            assert!(contrast >= 3.0, "{name} index {index}: {contrast}");
        }
    }
    // These published entries already have enough contrast and stay exact.
    assert_eq!(
        config.resolve_theme("mocha").unwrap().terminal[1],
        Color::Rgb(0xf3, 0x8b, 0xa8)
    );
    assert_eq!(
        config.resolve_theme("nordfox").unwrap().terminal[4],
        Color::Rgb(0x81, 0xa1, 0xc1)
    );
    assert_eq!(
        config.resolve_theme("gruvbox").unwrap().terminal[9],
        Color::Rgb(0xfb, 0x49, 0x34)
    );
}

#[test]
fn derived_terminal_colours_contrast_even_on_mid_gray_custom_grounds() {
    for ground in ["#808080", "#999999", "#aaaaaa", "#bcbcbc"] {
        let definition = ThemeDefinition {
            background: ground.into(),
            error: ground.into(),
            ..ThemeDefinition::default()
        };
        let theme = Theme::try_from(&definition).unwrap();
        let background = theme.background.relative_luminance().unwrap();
        for color in theme.terminal {
            let foreground = color.relative_luminance().unwrap();
            let contrast =
                (background.max(foreground) + 0.05) / (background.min(foreground) + 0.05);
            assert!(contrast >= 3.0, "{ground} {color:?}: {contrast}");
        }
    }
}
