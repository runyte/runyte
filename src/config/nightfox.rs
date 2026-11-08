// SPDX-License-Identifier: MPL-2.0

//! Nightfox-derived palettes and Runyte's explicit variants of them.

use super::{
    CHANGE_ADDED_DARK, CHANGE_MODIFIED_DARK, CHANGE_REMOVED_DARK, DIFF_ADDED_DARK,
    DIFF_CHANGED_DARK, DIFF_REMOVED_DARK, JUMP_LABEL_DARK_PRIMARY, JUMP_LABEL_DARK_SECONDARY,
    ThemeDefinition, imported::Palette, syntax_theme,
};

/// Runyte roles mapped onto Nightfox's canonical Nordfox palette and spec.
///
/// Source: <https://github.com/EdenEast/nightfox.nvim/blob/main/lua/nightfox/palette/nordfox.lua>
fn nordfox_theme() -> ThemeDefinition {
    ThemeDefinition {
        background: "#2e3440".into(),
        foreground: "#cdcecf".into(),
        muted: "#60728a".into(),
        whitespace: None,
        jump_text_muted: None,
        accent: "#8cafd2".into(),
        // Unset: this palette keeps one accent for its borders and its
        // command names alike.
        command: None,
        cursor_normal: None,
        cursor_insert: Some("#bf616a".into()),
        cursor_replace: None,
        cursor_select: Some("#c9826b".into()),
        cursor_command: Some("#b48ead".into()),
        cursor_secondary: None,
        directory: Some("#81a1c1".into()),
        destination_file: None,
        destination_explorer: None,
        destination_generated: None,
        destination_scratch: None,
        destination_terminal: None,
        selection: "#3e4a5b".into(),
        selection_primary: Some("#4f6074".into()),
        fuzzy_match_secondary: None,
        fuzzy_match_primary: None,
        status_background: "#232831".into(),
        status_foreground: "#abb1bb".into(),
        error: "#bf616a".into(),
        warning: Some("#c9826b".into()),
        info: Some("#a3be8c".into()),
        jump_label_immediate: Some("#f08a92".into()),
        jump_label_primary: JUMP_LABEL_DARK_PRIMARY.into(),
        jump_label_secondary: JUMP_LABEL_DARK_SECONDARY.into(),
        change_added: Some(CHANGE_ADDED_DARK.into()),
        change_modified: Some(CHANGE_MODIFIED_DARK.into()),
        change_removed: Some(CHANGE_REMOVED_DARK.into()),
        diff_added: Some(DIFF_ADDED_DARK.into()),
        diff_removed: Some(DIFF_REMOVED_DARK.into()),
        diff_changed: Some(DIFF_CHANGED_DARK.into()),
        terminal: std::collections::HashMap::new(),
        syntax: syntax_theme(&[
            ("attribute", "#d092ce"),
            ("comment", "#60728a"),
            ("constant", "#d89079"),
            ("constructor", "#93ccdc"),
            ("function", "#8cafd2"),
            ("keyword", "#b48ead"),
            ("label", "#c9826b"),
            ("namespace", "#88c0d0"),
            ("number", "#c9826b"),
            ("operator", "#abb1bb"),
            ("property", "#81a1c1"),
            ("punctuation", "#abb1bb"),
            ("string", "#a3be8c"),
            ("tag", "#bf616a"),
            ("type", "#ebcb8b"),
            ("variable", "#e5e9f0"),
        ]),
    }
}

/// Nordfox with brighter dimmed text and warm selection grounds.
///
/// The base and syntax palette stay canonical Nordfox. The selection grounds
/// are deliberately dark tints rather than Nightfox text accents: Runyte fills
/// whole cells with them, including terminal-review selections whose text is
/// using `jump_text_muted` at the same time.
fn nordfox_warm_theme() -> ThemeDefinition {
    let mut theme = nordfox_theme();
    theme.muted = "#71839a".into();
    theme.jump_text_muted = Some("#929fae".into());
    theme.selection = "#603f54".into();
    theme.selection_primary = Some("#5c4e27".into());
    theme
}

/// Runyte roles mapped onto Nightfox's canonical Terafox palette and spec.
///
/// Source: <https://github.com/EdenEast/nightfox.nvim/blob/main/lua/nightfox/palette/terafox.lua>
fn terafox_theme() -> ThemeDefinition {
    ThemeDefinition {
        background: "#152528".into(),
        foreground: "#e6eaea".into(),
        muted: "#6d7f8b".into(),
        whitespace: None,
        // Terafox's own selection grounds are read against dimmed text as well
        // as ordinary text: an unfocused pane draws every cell in
        // `jump_text_muted` while a terminal review selection still fills whole
        // cells with `selection_primary`. Canonical `#425e5e` is too close to
        // any legible dimmed text to leave the selected row readable, so the
        // dimmed text is brightened and the primary ground deepened until the
        // pair clears the boundary `nordfox-warm` holds: dimmed text at 3:1
        // against either ground, ordinary text at 4.5:1.
        jump_text_muted: Some("#8998a2".into()),
        accent: "#73a3b7".into(),
        // Unset: this palette keeps one accent for its borders and its
        // command names alike.
        command: None,
        cursor_normal: None,
        cursor_insert: Some("#e85c51".into()),
        cursor_replace: None,
        cursor_select: Some("#ff8349".into()),
        cursor_command: Some("#ad5c7c".into()),
        cursor_secondary: None,
        directory: Some("#5a93aa".into()),
        destination_file: None,
        destination_explorer: None,
        destination_generated: None,
        destination_scratch: None,
        destination_terminal: None,
        // Canonical `#293e40` is barely a shade off the background, so a match
        // or a secondary range highlighted with it did not read as highlighted
        // at all. This is the same teal carrying enough saturation to be seen.
        selection: "#264e59".into(),
        // Deepening the canonical teal ground left the two selections the same
        // hue at nearly the same lightness, which is the one thing the primary
        // range cannot afford: it has to be told apart from the others at a
        // glance. Terafox's orange — the hue of its `warning`, `number` and
        // `label` — separates the pair by hue instead, the way `nordfox-warm`
        // separates its own, at the ground lightness and saturation that
        // theme's grounds already use.
        selection_primary: Some("#6a3c25".into()),
        fuzzy_match_secondary: None,
        fuzzy_match_primary: None,
        status_background: "#0f1c1e".into(),
        status_foreground: "#cbd9d8".into(),
        error: "#e85c51".into(),
        warning: Some("#ff8349".into()),
        info: Some("#7aa4a1".into()),
        jump_label_immediate: Some("#e85c51".into()),
        jump_label_primary: JUMP_LABEL_DARK_PRIMARY.into(),
        jump_label_secondary: JUMP_LABEL_DARK_SECONDARY.into(),
        change_added: Some(CHANGE_ADDED_DARK.into()),
        change_modified: Some(CHANGE_MODIFIED_DARK.into()),
        change_removed: Some(CHANGE_REMOVED_DARK.into()),
        diff_added: Some(DIFF_ADDED_DARK.into()),
        diff_removed: Some(DIFF_REMOVED_DARK.into()),
        diff_changed: Some(DIFF_CHANGED_DARK.into()),
        terminal: std::collections::HashMap::new(),
        syntax: syntax_theme(&[
            ("attribute", "#d38d97"),
            ("comment", "#6d7f8b"),
            ("constant", "#ff9664"),
            ("constructor", "#afd4de"),
            ("function", "#73a3b7"),
            ("keyword", "#ad5c7c"),
            ("label", "#ff8349"),
            ("namespace", "#a1cdd8"),
            ("number", "#ff8349"),
            ("operator", "#cbd9d8"),
            ("property", "#5a93aa"),
            ("punctuation", "#cbd9d8"),
            ("string", "#7aa4a1"),
            ("tag", "#e85c51"),
            ("type", "#fda47f"),
            ("variable", "#ebebeb"),
        ]),
    }
}

/// Terafox with the glare taken off its text.
///
/// Terafox's text is the brightest thing in the palette by a wide margin: its
/// foreground reads at 13.1:1 against the background, its identifiers at
/// 13.3:1, and its operators and punctuation at 10.9:1, where every hued colour
/// the theme draws with sits between 3.5 and 10.0. Long stretches of ordinary
/// text are therefore near-white on a dark teal ground. This variant brings
/// that band down and changes nothing else — the background, the hued syntax
/// colours, the accents, and the selection grounds are all Terafox's. Both
/// variants use Runyte's shared Git colours.
///
/// The step is the one `nordbones-dark-soft` takes: 15 points of CIELAB
/// lightness off each of the three neutral text values, which puts ordinary
/// text at 8.6:1. Moving all three keeps Terafox's own ordering intact, so
/// punctuation does not end up brighter than the identifiers it separates.
/// The dimmed text stays where Terafox already pins it: it is at the boundary its
/// selection grounds allow, and 8.6:1 is comfortably far enough above it to
/// still read as ordinary text next to dimmed.
///
/// Terafox's pale cyans — `constructor` and `namespace` — are left brighter
/// than the softened text. They are hued accents on sparse constructs rather
/// than running text, and repainting them would be repainting Terafox.
fn terafox_soft_theme() -> ThemeDefinition {
    /// Terafox's three neutral text values and the softened value each takes.
    const SOFTENED: [(&str, &str); 3] = [
        // Ordinary buffer text.
        ("#e6eaea", "#bcc0c0"),
        // Identifiers, which Terafox paints a shade brighter still.
        ("#ebebeb", "#c1c1c1"),
        // Operators and punctuation, which share one value.
        ("#cbd9d8", "#a6b1b0"),
    ];

    fn soften(color: &str) -> Option<String> {
        SOFTENED
            .iter()
            .find(|(from, _)| color.eq_ignore_ascii_case(from))
            .map(|(_, to)| (*to).to_owned())
    }

    let mut theme = terafox_theme();
    for color in theme.syntax.values_mut() {
        if let Some(softened) = soften(color) {
            *color = softened;
        }
    }
    for color in [&mut theme.foreground, &mut theme.status_foreground] {
        if let Some(softened) = soften(color) {
            *color = softened;
        }
    }
    theme
}

/// The remaining Nightfox flavours, in the shared imported-palette form.
pub(super) static VARIANTS: [Palette; 5] = [
    Palette {
        name: "nightfox",
        light: false,
        background: "#192330",
        foreground: "#cdcecf",
        muted: "#738091",
        jump_text_muted: Some("#9fa6af"),
        accent: "#86abdc",
        cursor_normal: None,
        cursor_insert: "#c94f6d",
        cursor_select: "#f4a261",
        cursor_command: "#9d79d6",
        directory: "#719cd6",
        selection: "#3c596d",
        selection_primary: "#3f2619",
        status_background: "#131a24",
        status_foreground: "#aeafb0",
        error: "#c94f6d",
        warning: "#f4a261",
        info: "#81b29a",
        jump_label_immediate: "#d16881",
        diff_rows: None,
        syntax: [
            "#dc8ed9", "#738091", "#f6b079", "#7ad5d6", "#86abdc", "#9d79d6", "#f4a261", "#63cdcf",
            "#f4a261", "#aeafb0", "#719cd6", "#aeafb0", "#81b29a", "#c94f6d", "#dbc074", "#dfdfe0",
        ],
    },
    Palette {
        name: "duskfox",
        light: false,
        background: "#232136",
        foreground: "#e0def4",
        muted: "#817c9c",
        jump_text_muted: Some("#a19dba"),
        accent: "#65b1cd",
        cursor_normal: None,
        cursor_insert: "#eb6f92",
        cursor_select: "#ea9a97",
        cursor_command: "#c4a7e7",
        directory: "#569fba",
        selection: "#364778",
        selection_primary: "#644b4f",
        status_background: "#191726",
        status_foreground: "#cdcbe0",
        error: "#eb6f92",
        warning: "#ea9a97",
        info: "#a3be8c",
        jump_label_immediate: "#eb6f92",
        diff_rows: None,
        syntax: [
            "#f0a6cc", "#817c9c", "#f0a4a2", "#a6dae3", "#65b1cd", "#c4a7e7", "#ea9a97", "#9ccfd8",
            "#ea9a97", "#cdcbe0", "#569fba", "#cdcbe0", "#a3be8c", "#eb6f92", "#f6c177", "#e0def4",
        ],
    },
    Palette {
        name: "carbonfox",
        light: false,
        background: "#161616",
        foreground: "#f2f4f8",
        muted: "#6e6f70",
        jump_text_muted: Some("#838486"),
        accent: "#8cb6ff",
        cursor_normal: None,
        cursor_insert: "#ee5396",
        cursor_select: "#3ddbd9",
        cursor_command: "#be95ff",
        directory: "#78a9ff",
        selection: "#2a3b53",
        selection_primary: "#3a2e0c",
        status_background: "#0c0c0c",
        status_foreground: "#b6b8bb",
        error: "#ee5396",
        warning: "#3ddbd9",
        info: "#25be6a",
        jump_label_immediate: "#ee5396",
        diff_rows: None,
        syntax: [
            "#ff91c1", "#6e6f70", "#5ae0df", "#52bdff", "#8cb6ff", "#be95ff", "#3ddbd9", "#33b1ff",
            "#3ddbd9", "#b6b8bb", "#78a9ff", "#b6b8bb", "#25be6a", "#ee5396", "#08bdba", "#dfdfe0",
        ],
    },
    Palette {
        name: "dawnfox",
        light: true,
        background: "#faf4ed",
        foreground: "#575279",
        muted: "#9893a5",
        jump_text_muted: Some("#7a7591"),
        accent: "#295e73",
        cursor_normal: None,
        cursor_insert: "#b4637a",
        cursor_select: "#c67874",
        cursor_command: "#907aa9",
        directory: "#286983",
        selection: "#b8dbf5",
        selection_primary: "#f0dea8",
        status_background: "#ebe5df",
        status_foreground: "#625c87",
        error: "#b4637a",
        warning: "#c67874",
        info: "#618774",
        jump_label_immediate: "#a45a6f",
        diff_rows: None,
        syntax: [
            "#c9709e", "#9893a5", "#ca6e69", "#50848c", "#295e73", "#907aa9", "#d7827e", "#56949f",
            "#d7827e", "#625c87", "#286983", "#625c87", "#618774", "#b4637a", "#ea9d34", "#575279",
        ],
    },
    Palette {
        name: "dayfox",
        light: true,
        background: "#f6f2ee",
        foreground: "#3d2b5a",
        muted: "#837a72",
        jump_text_muted: Some("#685b69"),
        accent: "#223d90",
        cursor_normal: None,
        cursor_insert: "#a5222f",
        cursor_select: "#955f61",
        cursor_command: "#6e33ce",
        directory: "#2848a9",
        selection: "#a8c1d2",
        selection_primary: "#beb198",
        status_background: "#e4dcd4",
        status_foreground: "#643f61",
        error: "#a5222f",
        warning: "#955f61",
        info: "#396847",
        jump_label_immediate: "#a5222f",
        diff_rows: None,
        syntax: [
            "#8b369a", "#837a72", "#7f5152", "#22676d", "#223d90", "#6e33ce", "#955f61", "#287980",
            "#955f61", "#643f61", "#2848a9", "#643f61", "#396847", "#a5222f", "#ac5402", "#352c24",
        ],
    },
];

pub(super) fn themes() -> impl Iterator<Item = (String, ThemeDefinition)> {
    [
        ("nordfox", nordfox_theme()),
        ("nordfox-warm", nordfox_warm_theme()),
        ("terafox", terafox_theme()),
        ("terafox-soft", terafox_soft_theme()),
    ]
    .into_iter()
    .map(|(name, theme)| (name.to_owned(), theme))
    .chain(VARIANTS.iter().map(Palette::entry))
}
