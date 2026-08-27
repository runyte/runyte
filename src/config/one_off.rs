// SPDX-License-Identifier: MPL-2.0

//! Standalone imported palettes that do not share a family adapter.

use super::{JUMP_LABEL_LIGHT_PRIMARY, JUMP_LABEL_LIGHT_SECONDARY, ThemeDefinition, syntax_theme};

/// Runyte roles mapped onto projekt0n's GitHub Light palettes and syntax spec.
///
/// Source:
/// <https://github.com/projekt0n/github-nvim-theme/blob/c106c9472154d6b2c74b74565616b877ae8ed31d/lua/github-theme/palette/github_light.lua>
fn github_light_theme() -> ThemeDefinition {
    ThemeDefinition {
        background: "#ffffff".into(),
        foreground: "#1f2328".into(),
        muted: "#6e7781".into(),
        whitespace: None,
        jump_text_muted: Some("#afb8c1".into()),
        accent: "#0969da".into(),
        cursor_normal: Some("#0969da".into()),
        cursor_insert: Some("#d1242f".into()),
        cursor_select: Some("#bc4c00".into()),
        cursor_command: Some("#6639ba".into()),
        directory: Some("#6639ba".into()),
        selection: "#dae9f9".into(),
        selection_primary: Some("#e1d1b3".into()),
        fuzzy_match_secondary: Some("#c2e2ff".into()),
        fuzzy_match_primary: Some("#e1d1b3".into()),
        status_background: "#5094e4".into(),
        status_foreground: "#f6f8fa".into(),
        error: "#d1242f".into(),
        warning: Some("#9a6700".into()),
        info: Some("#0969da".into()),
        jump_label_immediate: Some("#d1242f".into()),
        jump_label_primary: JUMP_LABEL_LIGHT_PRIMARY.into(),
        jump_label_secondary: JUMP_LABEL_LIGHT_SECONDARY.into(),
        change_added: Some("#1a7f37".into()),
        change_modified: Some("#9a6700".into()),
        change_removed: Some("#d1242f".into()),
        diff_added: Some("#b8d0bb".into()),
        diff_removed: Some("#e4b7be".into()),
        diff_changed: Some("#d8cab3".into()),
        syntax: syntax_theme(&[
            ("attribute", "#0550ae"),
            ("comment", "#57606a"),
            ("constant", "#0550ae"),
            ("constructor", "#953800"),
            ("function", "#6639ba"),
            ("keyword", "#cf222e"),
            ("label", "#cf222e"),
            ("namespace", "#953800"),
            ("number", "#0550ae"),
            ("operator", "#0550ae"),
            ("property", "#0550ae"),
            ("punctuation", "#1f2328"),
            ("string", "#0a3069"),
            ("tag", "#116329"),
            ("type", "#953800"),
            ("variable", "#1f2328"),
        ]),
    }
}

/// Runyte roles mapped onto Atom's official One Light UI and syntax palettes.
///
/// Sources:
/// - <https://github.com/atom/one-light-ui/blob/master/styles/ui-variables.less>
/// - <https://github.com/atom/one-light-syntax/blob/master/styles/colors.less>
fn atom_one_light_theme() -> ThemeDefinition {
    let mut syntax = syntax_theme(&[
        ("attribute", "#986801"),
        ("comment", "#a0a1a7"),
        ("constant", "#986801"),
        ("constructor", "#c18401"),
        ("function", "#4078f2"),
        ("keyword", "#a626a4"),
        ("label", "#0184bc"),
        ("namespace", "#a626a4"),
        ("number", "#986801"),
        ("operator", "#383a42"),
        ("property", "#383a42"),
        ("punctuation", "#696c77"),
        ("string", "#50a14f"),
        ("tag", "#e45649"),
        ("type", "#c18401"),
        ("variable", "#e45649"),
    ]);
    // The shared bundled-theme derivation is a useful fallback, but Atom's
    // GFM stylesheet gives these roles explicit colours of its own.
    for (scope, color) in [
        ("markup.bold", "#986801"),
        ("markup.heading", "#e45649"),
        ("markup.italic", "#a626a4"),
        ("markup.link.text", "#0184bc"),
        ("markup.link.url", "#0184bc"),
        ("markup.list", "#a626a4"),
        ("markup.quote", "#986801"),
        ("markup.raw", "#50a14f"),
    ] {
        syntax.insert(scope.into(), color.into());
    }

    ThemeDefinition {
        background: "#fafafa".into(),
        foreground: "#383a42".into(),
        muted: "#a0a1a7".into(),
        whitespace: None,
        jump_text_muted: Some("#b8b9bd".into()),
        accent: "#4078f2".into(),
        cursor_normal: Some("#526fff".into()),
        cursor_insert: Some("#e45649".into()),
        cursor_select: Some("#986801".into()),
        cursor_command: Some("#a626a4".into()),
        directory: Some("#4078f2".into()),
        // Atom has one neutral selection colour. Runyte distinguishes primary
        // and secondary selections, so these are light tints of Atom blue and
        // orange while preserving the palette's source-text contrast.
        selection: "#dce6fc".into(),
        selection_primary: Some("#f3e5c7".into()),
        fuzzy_match_secondary: None,
        fuzzy_match_primary: None,
        status_background: "#eaeaeb".into(),
        status_foreground: "#424243".into(),
        error: "#f42a2a".into(),
        warning: Some("#d5880b".into()),
        info: Some("#2db448".into()),
        jump_label_immediate: Some("#ca1243".into()),
        jump_label_primary: JUMP_LABEL_LIGHT_PRIMARY.into(),
        jump_label_secondary: JUMP_LABEL_LIGHT_SECONDARY.into(),
        change_added: Some("#2db448".into()),
        change_modified: Some("#f2a60d".into()),
        change_removed: Some("#ff1414".into()),
        diff_added: Some("#e5f5e8".into()),
        diff_removed: Some("#fde6e6".into()),
        diff_changed: Some("#f8eed8".into()),
        syntax,
    }
}

pub(super) fn themes() -> impl Iterator<Item = (String, ThemeDefinition)> {
    [
        ("atom-one-light", atom_one_light_theme()),
        ("github-light", github_light_theme()),
    ]
    .into_iter()
    .map(|(name, theme)| (name.to_owned(), theme))
}
