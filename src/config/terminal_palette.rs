// SPDX-License-Identifier: MPL-2.0

//! ANSI palette resolution stays separate from the emulator: stored indices
//! survive theme changes, review capture and persistent-session attachment.

use std::collections::HashMap;

use super::{Color, Theme, ThemeAppearance, ThemeDefinition, parse_color};

const NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright_black",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "bright_white",
];

pub(super) fn resolve(
    theme: &Theme,
    overrides: &HashMap<String, String>,
) -> anyhow::Result<[Color; 16]> {
    let normal = [
        theme.muted,
        theme.error,
        theme.change_added,
        theme.warning,
        theme.directory,
        theme.change_modified,
        theme.accent,
        theme.foreground,
    ];
    let mut colors = std::array::from_fn(|index| {
        let mut color = normal[index % 8];
        if index >= 8
            && let Some(appearance) = theme.appearance()
        {
            color = color.stepped_off(appearance, 0.12);
        }
        legible_color(theme.background, color)
    });
    for (name, value) in overrides {
        let index = NAMES.iter().position(|slot| *slot == name).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown terminal colour '{name}'; valid slots are {}",
                NAMES.join(", ")
            )
        })?;
        // Explicit choices, including terminal names and reset, are authoritative.
        colors[index] = parse_color(value)?;
    }
    Ok(colors)
}

/// Choose the higher-contrast extreme, including for mid-gray custom grounds
/// that the editor classifies as dark but cannot contrast with white at 3:1.
/// Twenty bounded steps include the exact extreme, so the floor is reachable.
fn legible_color(background: Color, color: Color) -> Color {
    let Some(ground) = background.relative_luminance() else {
        return color;
    };
    let contrast = |text: f64| (ground.max(text) + 0.05) / (ground.min(text) + 0.05);
    let appearance = if contrast(0.0) > contrast(1.0) {
        ThemeAppearance::Light
    } else {
        ThemeAppearance::Dark
    };
    let mut candidate = color;
    for step in 1..=20 {
        let Some(text) = candidate.relative_luminance() else {
            return candidate;
        };
        if contrast(text) >= 3.0 {
            return candidate;
        }
        candidate = color.stepped_off(appearance, f64::from(step) / 20.0);
    }
    candidate
}

pub(super) fn apply_builtin(name: &str, theme: &mut ThemeDefinition) {
    if let Some(colors) = upstream(name) {
        // Upstream terminal palettes include background-like black/white slots.
        // Move low-contrast entries toward readable text on Runyte's ground.
        // Custom overrides bypass this adaptation.
        let background = parse_color(&theme.background).expect("built-in background");
        theme.terminal = NAMES
            .into_iter()
            .zip(colors)
            .map(|(name, rgb)| {
                let color = Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
                let Color::Rgb(r, g, b) = legible_color(background, color) else {
                    unreachable!()
                };
                (name.to_owned(), format!("#{r:02x}{g:02x}{b:02x}"))
            })
            .collect();
    }
}

// Published terminal palettes, with provenance in THIRD_PARTY_NOTICES.md.
// Runyte variants reuse their parent's palette. Original themes derive theirs
// from semantic roles above; Atom's ANSI hues use its official syntax palette.
fn upstream(name: &str) -> Option<[u32; 16]> {
    Some(match name {
        "duckbones-dark" => [
            0x0E101A, 0xE03600, 0x5DCD97, 0xE39500, 0x00A3CB, 0x795CCC, 0x00A3CB, 0xEBEFC0,
            0x2B2F46, 0xFF4821, 0x58DB9E, 0xF6A100, 0x00B4E0, 0xB3A1E6, 0x00B4E0, 0xB3B692,
        ],
        "forestbones-dark" => [
            0x2C343A, 0xE67C7F, 0xA9C181, 0xDDBD7F, 0x7FBCB4, 0xD69AB7, 0x83C193, 0xE7DCC4,
            0x45525C, 0xED9294, 0xB0CE7B, 0xEDC77A, 0x7AC9C0, 0xE5A7C4, 0x7DD093, 0xB2A790,
        ],
        "forestbones-light" => [
            0xFAF3E1, 0xF85550, 0x8DA200, 0xDEA000, 0x3A94C4, 0xDF69BA, 0x36A87E, 0x4F5B62,
            0xDBC988, 0xE6271C, 0x758700, 0xB98500, 0x297CA6, 0xCA43A3, 0x258C67, 0x6E7F88,
        ],
        "frappe" => [
            0x51576d, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xb5bfe2,
            0x626880, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xa5adce,
        ],
        "kanagawabones-dark" => [
            0x1F1F28, 0xE46A78, 0x98BC6D, 0xE5C283, 0x7EB3C9, 0x957FB8, 0x7EB3C9, 0xDDD8BB,
            0x3C3C51, 0xEC818C, 0x9EC967, 0xF1C982, 0x7BC2DF, 0xA98FD2, 0x7BC2DF, 0xA8A48D,
        ],
        "latte" => [
            0xbcc0cc, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0x5c5f77,
            0xacb0be, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0x6c6f85,
        ],
        "macchiato" => [
            0x494d64, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xb8c0e0,
            0x5b6078, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xa5adcb,
        ],
        "mocha" => [
            0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
            0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
        ],
        "neobones-dark" => [
            0x0F191F, 0xDE6E7C, 0x90FF6B, 0xB77E64, 0x8190D4, 0xB279A7, 0x66A5AD, 0xC6D5CF,
            0x263945, 0xE8838F, 0xA0FF85, 0xD68C67, 0x92A0E2, 0xCF86C1, 0x65B8C1, 0x98A39E,
        ],
        "neobones-light" => [
            0xE5EDE6, 0xA8334C, 0x567A30, 0x944927, 0x286486, 0x88507D, 0x3B8992, 0x202E18,
            0xB3C6B6, 0x94253E, 0x3F5A22, 0x803D1C, 0x1D5573, 0x7B3B70, 0x2B747C, 0x415934,
        ],
        "nordbones-dark" | "nordbones-dark-soft" => [
            0x2F3541, 0xC1616A, 0xA4BE8D, 0xCF866F, 0x8FBCBA, 0xB38DAC, 0x87BFCE, 0xEBEEF3,
            0x475063, 0xD6787F, 0xA8CC86, 0xE09680, 0x89CAC8, 0xCF97C5, 0x82CCE0, 0xA5B4CD,
        ],
        "nordfox" | "nordfox-warm" => [
            0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
            0x465780, 0xd06f79, 0xb1d196, 0xf0d399, 0x8cafd2, 0xc895bf, 0x93ccdc, 0xe7ecf4,
        ],
        "rosebones-dark" => [
            0x1A1825, 0xEB7193, 0x317490, 0xF6C074, 0x9CCFD8, 0xC4A7E7, 0x9CCFD8, 0xE1D4D4,
            0x3A3651, 0xF289A4, 0x358DAF, 0xF9CA8E, 0x94DAE6, 0xCEB3EF, 0x94DAE6, 0xBF9B99,
        ],
        "rosebones-light" => [
            0xFBF6F0, 0xB5637A, 0x286A84, 0xEC9D33, 0x5795A0, 0x917BA9, 0x5795A0, 0x724341,
            0xE8C48B, 0xA54A66, 0x1C5970, 0xC68223, 0x407D88, 0x855AAC, 0x407D88, 0xA4635F,
        ],
        "seoulbones-dark" => [
            0x4B4B4B, 0xE388A3, 0x98BD99, 0xFFDF9B, 0x97BDDE, 0xA5A6C5, 0x6FBDBE, 0xDDDDDD,
            0x6C6465, 0xEB99B1, 0x8FCD92, 0xFFE5B3, 0xA2C8E9, 0xB2B3DA, 0x6BCACB, 0xA8A8A8,
        ],
        "seoulbones-light" => [
            0xE2E2E2, 0xDC5284, 0x628562, 0xC48562, 0x0084A3, 0x896788, 0x008586, 0x555555,
            0xBFBABB, 0xBE3C6D, 0x487249, 0xA76B48, 0x006F89, 0x7F4C7E, 0x006F70, 0x777777,
        ],
        "terafox" | "terafox-soft" => [
            0x2f3239, 0xe85c51, 0x7aa4a1, 0xfda47f, 0x5a93aa, 0xad5c7c, 0xa1cdd8, 0xebebeb,
            0x4e5157, 0xeb746b, 0x8eb2af, 0xfdb292, 0x73a3b7, 0xb97490, 0xafd4de, 0xeeeeee,
        ],
        "tokyobones-dark" => [
            0x1A1B26, 0xF77890, 0x74DBCB, 0xE1B068, 0x7BA2F7, 0xBB9BF7, 0x2BC4DE, 0xC0CAF5,
            0x36384D, 0xF98EA0, 0x6DE5D3, 0xF2BA64, 0x90AFFA, 0xC6ACFA, 0x74DBCB, 0x7E98EB,
        ],
        "tokyobones-light" => [
            0xD6D7DC, 0x8B4351, 0x34645D, 0x8F5E14, 0x34548C, 0x5A4A79, 0x176775, 0x333A57,
            0xADB0BD, 0x7E3242, 0x26554F, 0x794E0D, 0x26467A, 0x503875, 0x34645D, 0x56618D,
        ],
        "vimbones-light" => [
            0xF0F0CA, 0xA8334C, 0x4F6C31, 0x944927, 0x286486, 0x88507D, 0x3B8992, 0x353535,
            0xC6C6A3, 0x94253E, 0x3F5A22, 0x803D1C, 0x1D5573, 0x7B3B70, 0x2B747C, 0x5C5C5C,
        ],
        "zenbones-dark" => [
            0x1C1917, 0xDE6E7C, 0x819B69, 0xB77E64, 0x6099C0, 0xB279A7, 0x66A5AD, 0xB4BDC3,
            0x403833, 0xE8838F, 0x8BAE68, 0xD68C67, 0x61ABDA, 0xCF86C1, 0x65B8C1, 0x888F94,
        ],
        "zenbones-light" => [
            0xF0EDEC, 0xA8334C, 0x4F6C31, 0x944927, 0x286486, 0x88507D, 0x3B8992, 0x2C363C,
            0xCFC1BA, 0x94253E, 0x3F5A22, 0x803D1C, 0x1D5573, 0x7B3B70, 0x2B747C, 0x4F5E68,
        ],
        "zenburned-dark" => [
            0x404040, 0xE3716E, 0x819B69, 0xB77E64, 0x6099C0, 0xB279A7, 0x66A5AD, 0xF0E4CF,
            0x625A5B, 0xEC8685, 0x8BAE68, 0xD68C67, 0x61ABDA, 0xCF86C1, 0x65B8C1, 0xC0AB86,
        ],
        "zenwritten-dark" => [
            0x191919, 0xDE6E7C, 0x819B69, 0xB77E64, 0x6099C0, 0xB279A7, 0x66A5AD, 0xBBBBBB,
            0x3D3839, 0xE8838F, 0x8BAE68, 0xD68C67, 0x61ABDA, 0xCF86C1, 0x65B8C1, 0x8E8E8E,
        ],
        "zenwritten-light" => [
            0xEEEEEE, 0xA8334C, 0x4F6C31, 0x944927, 0x286486, 0x88507D, 0x3B8992, 0x353535,
            0xC6C3C3, 0x94253E, 0x3F5A22, 0x803D1C, 0x1D5573, 0x7B3B70, 0x2B747C, 0x5C5C5C,
        ],
        "github-light" => [
            0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
            0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
        ],
        "everforest-dark-hard" => [
            0x414b50, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
            0x414b50, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
        ],
        "everforest-light-hard" => [
            0x5c6a72, 0xf85552, 0x8da101, 0xdfa000, 0x3a94c5, 0xdf69ba, 0x35a77c, 0xedeada,
            0x5c6a72, 0xf85552, 0x8da101, 0xdfa000, 0x3a94c5, 0xdf69ba, 0x35a77c, 0xedeada,
        ],
        "everforest-dark-medium" => [
            0x475258, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
            0x475258, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
        ],
        "everforest-light-medium" => [
            0x5c6a72, 0xf85552, 0x8da101, 0xdfa000, 0x3a94c5, 0xdf69ba, 0x35a77c, 0xe6e2cc,
            0x5c6a72, 0xf85552, 0x8da101, 0xdfa000, 0x3a94c5, 0xdf69ba, 0x35a77c, 0xe6e2cc,
        ],
        "everforest-dark-soft" => [
            0x4d5960, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
            0x4d5960, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
        ],
        "everforest-light-soft" => [
            0x5c6a72, 0xf85552, 0x8da101, 0xdfa000, 0x3a94c5, 0xdf69ba, 0x35a77c, 0xddd8be,
            0x5c6a72, 0xf85552, 0x8da101, 0xdfa000, 0x3a94c5, 0xdf69ba, 0x35a77c, 0xddd8be,
        ],
        "gruvbox" => [
            0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
            0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
        ],
        "base16" => [
            0x181818, 0xab4642, 0xa1b56c, 0xf7ca88, 0x7cafc2, 0xba8baf, 0x86c1b9, 0xd8d8d8,
            0x585858, 0xab4642, 0xa1b56c, 0xf7ca88, 0x7cafc2, 0xba8baf, 0x86c1b9, 0xf8f8f8,
        ],
        "atom-one-light" => [
            0x383a42, 0xe45649, 0x50a14f, 0x986801, 0x4078f2, 0xa626a4, 0x0184bc, 0xa0a1a7,
            0x696c77, 0xca1243, 0x50a14f, 0xc18401, 0x4078f2, 0xa626a4, 0x0184bc, 0x383a42,
        ],
        _ => return None,
    })
}

#[cfg(test)]
#[path = "tests/terminal_palette.rs"]
mod tests;
