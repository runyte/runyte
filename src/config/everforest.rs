// SPDX-License-Identifier: MPL-2.0

//! Everforest background variants and their shared Runyte role mapping.

use super::{
    CHANGE_ADDED_DARK, CHANGE_ADDED_LIGHT, CHANGE_MODIFIED_DARK, CHANGE_MODIFIED_LIGHT,
    CHANGE_REMOVED_DARK, CHANGE_REMOVED_LIGHT, DIFF_ADDED_DARK, DIFF_ADDED_LIGHT,
    DIFF_CHANGED_DARK, DIFF_CHANGED_LIGHT, DIFF_REMOVED_DARK, DIFF_REMOVED_LIGHT,
    JUMP_LABEL_DARK_PRIMARY, JUMP_LABEL_DARK_SECONDARY, JUMP_LABEL_LIGHT_PRIMARY,
    JUMP_LABEL_LIGHT_SECONDARY, ThemeDefinition, syntax_theme,
};

#[derive(Clone, Copy)]
struct EverforestBackground {
    background: &'static str,
    status_background: &'static str,
    /// Dimmed text, where the ground it has to be read against decides it.
    ///
    /// This belongs to the background rather than the foreground because the
    /// three dark grounds sit at different lightnesses: the same dimmed text
    /// cannot clear all three. The light variants leave it unset, since a
    /// light ground carries dimmed text the way the built-in `light` theme
    /// does rather than at the dark themes' 3:1.
    dimmed_text: Option<&'static str>,
    selection: &'static str,
    selection_primary: &'static str,
}

fn everforest_dark(background: EverforestBackground) -> ThemeDefinition {
    everforest_theme(
        background,
        EverforestForeground {
            foreground: "#d3c6aa",
            muted: "#859289",
            red: "#e67e80",
            orange: "#e69875",
            yellow: "#dbbc7f",
            green: "#a7c080",
            aqua: "#83c092",
            blue: "#7fbbb3",
            purple: "#d699b6",
            command: "#d699b6",
            jump_label_immediate: "#ff9b9d",
            jump_label_primary: JUMP_LABEL_DARK_PRIMARY,
            jump_label_secondary: JUMP_LABEL_DARK_SECONDARY,
        },
        false,
    )
}

fn everforest_light(background: EverforestBackground) -> ThemeDefinition {
    everforest_theme(
        background,
        EverforestForeground {
            foreground: "#5c6a72",
            muted: "#939f91",
            red: "#f85552",
            orange: "#f57d26",
            yellow: "#dfa000",
            green: "#8da101",
            aqua: "#35a77c",
            blue: "#3a94c5",
            purple: "#df69ba",
            command: "#bf4d9a",
            jump_label_immediate: "#b92f2c",
            jump_label_primary: JUMP_LABEL_LIGHT_PRIMARY,
            jump_label_secondary: JUMP_LABEL_LIGHT_SECONDARY,
        },
        true,
    )
}

#[derive(Clone, Copy)]
struct EverforestForeground {
    foreground: &'static str,
    muted: &'static str,
    red: &'static str,
    orange: &'static str,
    yellow: &'static str,
    green: &'static str,
    aqua: &'static str,
    blue: &'static str,
    purple: &'static str,
    /// The Command caret. Dark grounds use the palette's own purple; the light
    /// ones need it a few steps darker, the same accommodation the jump labels
    /// already make, because Everforest's light purple is a pale magenta that
    /// disappears behind a caret glyph painted in the background.
    command: &'static str,
    jump_label_immediate: &'static str,
    jump_label_primary: &'static str,
    jump_label_secondary: &'static str,
}

fn everforest_theme(
    background: EverforestBackground,
    foreground: EverforestForeground,
    light: bool,
) -> ThemeDefinition {
    let (change_added, change_modified, change_removed, diff_added, diff_changed, diff_removed) =
        if light {
            (
                CHANGE_ADDED_LIGHT,
                CHANGE_MODIFIED_LIGHT,
                CHANGE_REMOVED_LIGHT,
                DIFF_ADDED_LIGHT,
                DIFF_CHANGED_LIGHT,
                DIFF_REMOVED_LIGHT,
            )
        } else {
            (
                CHANGE_ADDED_DARK,
                CHANGE_MODIFIED_DARK,
                CHANGE_REMOVED_DARK,
                DIFF_ADDED_DARK,
                DIFF_CHANGED_DARK,
                DIFF_REMOVED_DARK,
            )
        };
    ThemeDefinition {
        background: background.background.into(),
        foreground: foreground.foreground.into(),
        muted: foreground.muted.into(),
        whitespace: None,
        jump_text_muted: background.dimmed_text.map(Into::into),
        accent: foreground.green.into(),
        // Unset: this palette keeps one accent for its borders and its
        // command names alike.
        command: None,
        cursor_normal: Some(foreground.blue.into()),
        cursor_insert: Some(foreground.red.into()),
        cursor_replace: None,
        cursor_select: Some(foreground.orange.into()),
        cursor_command: Some(foreground.command.into()),
        directory: Some(foreground.blue.into()),
        selection: background.selection.into(),
        selection_primary: Some(background.selection_primary.into()),
        fuzzy_match_secondary: None,
        fuzzy_match_primary: None,
        status_background: background.status_background.into(),
        status_foreground: foreground.foreground.into(),
        error: foreground.red.into(),
        warning: Some(foreground.orange.into()),
        info: Some(foreground.green.into()),
        jump_label_immediate: Some(foreground.jump_label_immediate.into()),
        jump_label_primary: foreground.jump_label_primary.into(),
        jump_label_secondary: foreground.jump_label_secondary.into(),
        change_added: Some(change_added.into()),
        change_modified: Some(change_modified.into()),
        change_removed: Some(change_removed.into()),
        diff_added: Some(diff_added.into()),
        diff_removed: Some(diff_removed.into()),
        diff_changed: Some(diff_changed.into()),
        syntax: syntax_theme(&[
            ("attribute", foreground.purple),
            ("comment", foreground.muted),
            ("constant", foreground.aqua),
            ("constructor", foreground.green),
            ("function", foreground.green),
            ("keyword", foreground.red),
            ("label", foreground.orange),
            ("namespace", foreground.yellow),
            ("number", foreground.purple),
            ("operator", foreground.orange),
            ("property", foreground.blue),
            ("punctuation", foreground.muted),
            ("string", foreground.aqua),
            ("tag", foreground.orange),
            ("type", foreground.yellow),
            ("variable", foreground.foreground),
        ]),
    }
}

pub(super) fn themes() -> impl Iterator<Item = (String, ThemeDefinition)> {
    [
        (
            "everforest-dark-hard",
            EverforestBackground {
                background: "#272e33",
                status_background: "#2e383c",
                dimmed_text: Some("#909c94"),
                selection: "#2a4f66",
                selection_primary: "#5a3e22",
            },
            true,
        ),
        (
            "everforest-dark-medium",
            EverforestBackground {
                background: "#2d353b",
                status_background: "#343f44",
                dimmed_text: Some("#99a49d"),
                selection: "#30566e",
                selection_primary: "#60432a",
            },
            true,
        ),
        (
            "everforest-dark-soft",
            EverforestBackground {
                background: "#333c43",
                status_background: "#3a464c",
                dimmed_text: Some("#9da8a0"),
                selection: "#265a70",
                selection_primary: "#563a1e",
            },
            true,
        ),
        (
            "everforest-light-hard",
            EverforestBackground {
                background: "#fffbef",
                status_background: "#f8f5e4",
                dimmed_text: None,
                selection: "#b4eedc",
                selection_primary: "#ffe7a8",
            },
            false,
        ),
        (
            "everforest-light-medium",
            EverforestBackground {
                background: "#fdf6e3",
                status_background: "#f4f0d9",
                dimmed_text: None,
                selection: "#b0ead8",
                selection_primary: "#fde3a4",
            },
            false,
        ),
        (
            "everforest-light-soft",
            EverforestBackground {
                background: "#f3ead3",
                status_background: "#eae4ca",
                dimmed_text: None,
                selection: "#b7e6d5",
                selection_primary: "#f9dfa6",
            },
            false,
        ),
    ]
    .into_iter()
    .map(|(name, background, dark)| {
        let theme = if dark {
            everforest_dark(background)
        } else {
            everforest_light(background)
        };
        (name.to_owned(), theme)
    })
}
