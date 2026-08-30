// SPDX-License-Identifier: MPL-2.0

//! Runyte's original standalone theme definitions.

use std::collections::HashMap;

use super::{
    JUMP_LABEL_DARK_PRIMARY, JUMP_LABEL_DARK_SECONDARY, JUMP_LABEL_LIGHT_PRIMARY,
    JUMP_LABEL_LIGHT_SECONDARY, ThemeDefinition, syntax_theme,
};

pub(super) fn themes() -> impl Iterator<Item = (String, ThemeDefinition)> {
    let mut themes = HashMap::new();
    let mut base16 = ThemeDefinition {
        // Base16's comment grey is dark enough that dimmed text on either
        // selection ground was all but invisible — 1.08:1 on the blue one.
        // The grounds themselves stand off the background well, so only
        // the dimmed text moves.
        jump_text_muted: Some("#a3a3a3".into()),
        cursor_insert: Some("#ab4642".into()),
        cursor_replace: None,
        cursor_select: Some("#dc9656".into()),
        cursor_command: Some("#ba8baf".into()),
        directory: Some("#7cafc2".into()),
        selection: "#365864".into(),
        selection_primary: Some("#5a3b2a".into()),
        jump_label_immediate: Some("#e65c57".into()),
        ..ThemeDefinition::default()
    };
    base16
        .syntax
        .insert("property".into(), base16.foreground.clone());
    themes.insert("base16".into(), base16);
    // Runyte's branded pair starts from the runyte.com workspace palette.
    // The dark surface, text, and red accent are the site's own values, and
    // Normal mode and directory entries stay unset so they always track that
    // accent. Insert mode took the site's blue instead. Command mode swapped
    // places with Replace in the mode vocabulary, and `keyword`/`attribute`
    // (Command's syntax counterparts) swapped hues with `string` (Replace's)
    // right along with it — a deliberate choice to give keywords the same
    // mint green `string` used to carry, even though `syntax_theme` derives
    // several Markdown roles from both: inline code and link URLs (from
    // `string`) turn purple, and bold/list text (from `keyword`) turns green.
    let mut default_dark_syntax = syntax_theme(&[
        ("attribute", "#8ddb8c"),
        ("comment", "#8b8b90"),
        ("constant", "#f0a868"),
        ("constructor", "#6cb6ff"),
        ("function", "#6cb6ff"),
        ("keyword", "#8ddb8c"),
        ("label", "#f0a868"),
        ("namespace", "#62d6d7"),
        ("number", "#f0a868"),
        ("operator", "#b9b9be"),
        ("property", "#b9b9be"),
        ("punctuation", "#8b8b90"),
        ("string", "#d2a8ff"),
        ("tag", "#c96870"),
        ("type", "#62d6d7"),
        ("variable", "#b9b9be"),
    ]);
    default_dark_syntax.insert("markup.heading".into(), "#c96870".into());
    themes.insert(
        "default-dark".into(),
        ThemeDefinition {
            // One step lighter than the surface's original `#16181d`: the
            // active pane now sits where the inactive pane used to, and the
            // inactive pane is derived a further step lighter still.
            background: "#1f2126".into(),
            foreground: "#b9b9be".into(),
            muted: "#8b8b90".into(),
            // A background this much lighter left the marker only 8-11
            // levels off it, well short of the 17-20 the original background
            // gave it; nudged out a little further than that original gap.
            whitespace: Some("#35373c".into()),
            jump_text_muted: None,
            accent: "#c96870".into(),
            // Left unset so Normal mode's caret always matches the accent
            // that also colours the active pane border.
            cursor_normal: None,
            cursor_insert: Some("#6cb6ff".into()),
            cursor_replace: Some("#d2a8ff".into()),
            cursor_select: Some("#f0a868".into()),
            cursor_command: Some("#8ddb8c".into()),
            directory: None,
            selection: "#30475f".into(),
            selection_primary: Some("#593d2d".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#111318".into(),
            status_foreground: "#b9b9be".into(),
            error: "#d06a73".into(),
            warning: Some("#f0a868".into()),
            info: Some("#8ddb8c".into()),
            jump_label_immediate: Some("#ef7078".into()),
            jump_label_primary: JUMP_LABEL_DARK_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_DARK_SECONDARY.into(),
            change_added: Some("#8ddb8c".into()),
            change_modified: Some("#f0a868".into()),
            change_removed: Some("#d06a73".into()),
            diff_added: Some("#18271d".into()),
            diff_removed: Some("#2d1b20".into()),
            diff_changed: Some("#2d251b".into()),
            syntax: default_dark_syntax,
        },
    );
    let mut default_light_syntax = syntax_theme(&[
        ("attribute", "#23733a"),
        ("comment", "#656872"),
        ("constant", "#9a5518"),
        ("constructor", "#1f65a6"),
        ("function", "#1f65a6"),
        ("keyword", "#23733a"),
        ("label", "#9a5518"),
        ("namespace", "#176d70"),
        ("number", "#9a5518"),
        ("operator", "#292a30"),
        ("property", "#292a30"),
        ("punctuation", "#656872"),
        ("string", "#754b97"),
        ("tag", "#a33d49"),
        ("type", "#176d70"),
        ("variable", "#292a30"),
    ]);
    default_light_syntax.insert("markup.heading".into(), "#a33d49".into());
    themes.insert(
        "default-light".into(),
        ThemeDefinition {
            // One step darker than the surface's original `#ececef`: the
            // active pane now sits where the inactive pane used to, and the
            // inactive pane is derived a further step darker still.
            background: "#e3e3e5".into(),
            foreground: "#292a30".into(),
            muted: "#656872".into(),
            // A background this much darker left the marker only 18-22
            // levels off it, well short of the 28-31 the original background
            // gave it; nudged out a little further than that original gap,
            // just inside the 31-level "near background" ceiling.
            whitespace: Some("#c5c5c7".into()),
            jump_text_muted: Some("#878a92".into()),
            accent: "#a33d49".into(),
            // Left unset so Normal mode's caret always matches the accent
            // that also colours the active pane border.
            cursor_normal: None,
            cursor_insert: Some("#1f65a6".into()),
            cursor_replace: Some("#754b97".into()),
            cursor_select: Some("#9a5518".into()),
            cursor_command: Some("#23733a".into()),
            directory: None,
            selection: "#bfd6ea".into(),
            selection_primary: Some("#e9cdb2".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#d9dade".into(),
            status_foreground: "#292a30".into(),
            error: "#a33d49".into(),
            warning: Some("#9a5518".into()),
            info: Some("#23733a".into()),
            jump_label_immediate: Some("#a33d49".into()),
            jump_label_primary: JUMP_LABEL_LIGHT_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_LIGHT_SECONDARY.into(),
            change_added: Some("#23733a".into()),
            change_modified: Some("#9a5518".into()),
            change_removed: Some("#a33d49".into()),
            diff_added: Some("#d9e8dc".into()),
            diff_removed: Some("#efd8da".into()),
            diff_changed: Some("#ecdfce".into()),
            syntax: default_light_syntax,
        },
    );
    // `dark` and `light` are the two themes people reach for by name, so
    // they are neutral by design: no palette identity of their own, just a
    // legible pair that reads correctly on a dark and on a light terminal.
    themes.insert(
        "dark".into(),
        ThemeDefinition {
            background: "#16181d".into(),
            foreground: "#d6dae0".into(),
            muted: "#6b7280".into(),
            whitespace: None,
            jump_text_muted: None,
            accent: "#6cb6ff".into(),
            cursor_normal: None,
            cursor_insert: Some("#f87171".into()),
            cursor_replace: None,
            cursor_select: Some("#f0a868".into()),
            cursor_command: Some("#d2a8ff".into()),
            directory: Some("#6cb6ff".into()),
            selection: "#34506a".into(),
            selection_primary: Some("#5a3f2b".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#21242b".into(),
            status_foreground: "#d6dae0".into(),
            error: "#f87171".into(),
            warning: Some("#f0a868".into()),
            info: Some("#8ddb8c".into()),
            jump_label_immediate: Some("#f87171".into()),
            jump_label_primary: JUMP_LABEL_DARK_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_DARK_SECONDARY.into(),
            change_added: Some("#8ddb8c".into()),
            change_modified: Some("#f0a868".into()),
            change_removed: Some("#f87171".into()),
            diff_added: Some("#16281c".into()),
            diff_removed: Some("#2b1a1e".into()),
            diff_changed: Some("#2a2318".into()),
            syntax: syntax_theme(&[
                ("attribute", "#d2a8ff"),
                ("comment", "#6b7280"),
                ("constant", "#f0a868"),
                ("constructor", "#6cb6ff"),
                ("function", "#6cb6ff"),
                ("keyword", "#d2a8ff"),
                ("label", "#f0a868"),
                ("namespace", "#7ee0c0"),
                ("number", "#f0a868"),
                ("operator", "#d6dae0"),
                ("property", "#d6dae0"),
                ("punctuation", "#9aa3af"),
                ("string", "#8ddb8c"),
                ("tag", "#f87171"),
                ("type", "#7ee0c0"),
                ("variable", "#d6dae0"),
            ]),
        },
    );
    themes.insert(
        "light".into(),
        ThemeDefinition {
            background: "#fbfbfa".into(),
            foreground: "#24292f".into(),
            muted: "#6e7781".into(),
            whitespace: None,
            jump_text_muted: Some("#a8adb2".into()),
            accent: "#0550ae".into(),
            cursor_normal: Some("#0550ae".into()),
            cursor_insert: Some("#cf222e".into()),
            cursor_replace: None,
            cursor_select: Some("#953800".into()),
            cursor_command: Some("#8250df".into()),
            directory: Some("#0550ae".into()),
            selection: "#cfe3ff".into(),
            selection_primary: Some("#ffe2c2".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#e8eaed".into(),
            status_foreground: "#24292f".into(),
            error: "#b3261e".into(),
            warning: Some("#953800".into()),
            info: Some("#0a6b26".into()),
            jump_label_immediate: Some("#b3261e".into()),
            jump_label_primary: JUMP_LABEL_LIGHT_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_LIGHT_SECONDARY.into(),
            change_added: Some("#0a6b26".into()),
            change_modified: Some("#953800".into()),
            change_removed: Some("#b3261e".into()),
            diff_added: Some("#e3f5e6".into()),
            diff_removed: Some("#fce4e4".into()),
            diff_changed: Some("#fdf0dc".into()),
            syntax: syntax_theme(&[
                ("attribute", "#8250df"),
                ("comment", "#6e7781"),
                ("constant", "#953800"),
                ("constructor", "#0550ae"),
                ("function", "#0550ae"),
                ("keyword", "#8250df"),
                ("label", "#953800"),
                ("namespace", "#0f6b5c"),
                ("number", "#953800"),
                ("operator", "#24292f"),
                ("property", "#24292f"),
                ("punctuation", "#57606a"),
                ("string", "#0a6b26"),
                ("tag", "#b3261e"),
                ("type", "#0f6b5c"),
                ("variable", "#24292f"),
            ]),
        },
    );
    themes.insert(
        "paper".into(),
        ThemeDefinition {
            background: "#eeeeee".into(),
            foreground: "#303030".into(),
            muted: "#808080".into(),
            whitespace: None,
            jump_text_muted: Some("#aaaaaa".into()),
            accent: "#005faf".into(),
            cursor_normal: Some("#005faf".into()),
            cursor_insert: Some("#af0000".into()),
            cursor_replace: None,
            cursor_select: Some("#d75f00".into()),
            cursor_command: Some("#8700af".into()),
            directory: Some("#005faf".into()),
            selection: "#afd7ff".into(),
            selection_primary: Some("#ffd7af".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#d0d0d0".into(),
            status_foreground: "#202020".into(),
            error: "#af0000".into(),
            warning: Some("#d75f00".into()),
            info: Some("#005f00".into()),
            jump_label_immediate: Some("#af0000".into()),
            jump_label_primary: JUMP_LABEL_LIGHT_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_LIGHT_SECONDARY.into(),
            change_added: Some("#005f00".into()),
            change_modified: Some("#d75f00".into()),
            change_removed: Some("#af0000".into()),
            diff_added: Some("#dcecdc".into()),
            diff_removed: Some("#f2dcdc".into()),
            diff_changed: Some("#f0e6d2".into()),
            syntax: syntax_theme(&[
                ("attribute", "#8700af"),
                ("comment", "#808080"),
                ("constant", "#d75f00"),
                ("constructor", "#005faf"),
                ("function", "#005faf"),
                ("keyword", "#8700af"),
                ("label", "#d75f00"),
                ("namespace", "#875f00"),
                ("number", "#d75f00"),
                ("operator", "#303030"),
                ("property", "#303030"),
                ("punctuation", "#606060"),
                ("string", "#005f00"),
                ("tag", "#af0000"),
                ("type", "#875f00"),
                ("variable", "#303030"),
            ]),
        },
    );
    themes.insert(
        "gruvbox".into(),
        ThemeDefinition {
            background: "#282828".into(),
            foreground: "#ebdbb2".into(),
            muted: "#928374".into(),
            whitespace: None,
            jump_text_muted: None,
            accent: "#fabd2f".into(),
            cursor_normal: None,
            cursor_insert: Some("#fb4934".into()),
            cursor_replace: None,
            cursor_select: Some("#fe8019".into()),
            cursor_command: Some("#d3869b".into()),
            directory: Some("#83a598".into()),
            selection: "#3c5154".into(),
            selection_primary: Some("#66502f".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#3c3836".into(),
            status_foreground: "#ebdbb2".into(),
            error: "#fb4934".into(),
            warning: Some("#fe8019".into()),
            info: Some("#b8bb26".into()),
            jump_label_immediate: Some("#ff7b6b".into()),
            jump_label_primary: JUMP_LABEL_DARK_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_DARK_SECONDARY.into(),
            change_added: Some("#b8bb26".into()),
            change_modified: Some("#fabd2f".into()),
            change_removed: Some("#fb4934".into()),
            diff_added: Some("#26301f".into()),
            diff_removed: Some("#3a2424".into()),
            diff_changed: Some("#38301c".into()),
            syntax: syntax_theme(&[
                ("attribute", "#fabd2f"),
                ("comment", "#928374"),
                ("constant", "#d3869b"),
                ("constructor", "#8ec07c"),
                ("function", "#b8bb26"),
                ("keyword", "#fb4934"),
                ("label", "#fe8019"),
                ("namespace", "#fabd2f"),
                ("number", "#d3869b"),
                ("operator", "#ebdbb2"),
                ("property", "#ebdbb2"),
                ("punctuation", "#a89984"),
                ("string", "#b8bb26"),
                ("tag", "#fb4934"),
                ("type", "#fabd2f"),
                ("variable", "#ebdbb2"),
            ]),
        },
    );
    themes.into_iter()
}
