// SPDX-License-Identifier: MPL-2.0

//! Runyte's original standalone theme definitions.

use std::collections::HashMap;

use super::{
    CHANGE_ADDED_DARK, CHANGE_ADDED_LIGHT, CHANGE_MODIFIED_DARK, CHANGE_MODIFIED_LIGHT,
    CHANGE_REMOVED_DARK, CHANGE_REMOVED_LIGHT, DIFF_ADDED_DARK, DIFF_ADDED_LIGHT,
    DIFF_CHANGED_DARK, DIFF_CHANGED_LIGHT, DIFF_REMOVED_DARK, DIFF_REMOVED_LIGHT,
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
        diff_added: Some(DIFF_ADDED_DARK.into()),
        diff_removed: Some(DIFF_REMOVED_DARK.into()),
        ..ThemeDefinition::default()
    };
    base16
        .syntax
        .insert("property".into(), base16.foreground.clone());
    themes.insert("base16".into(), base16);
    // Runyte's branded pair starts from the runyte.com workspace palette.
    // The dark surface, text, and red accent are the site's own values;
    // directory entries stay unset so they keep tracking that accent.
    //
    // The mode vocabulary is the pair's own rather than the one most bundled
    // themes use: Normal is green, Insert takes the brand red, Select is
    // orange, Replace is purple, and Command is the site's blue. Command's
    // blue is also what the command palette lists its command names in, so
    // the mode label and the list it belongs to say the same thing.
    //
    // The syntax palette keeps `keyword`/`attribute` on the mint green that
    // `string` used to carry, and `string` on purple, which is why
    // `syntax_theme`'s derived Markdown roles read the way they do: inline
    // code and link URLs (from `string`) turn purple, and bold and list text
    // (from `keyword`) turn green. That swap predates the mode retune and is
    // independent of it — syntax scopes are not mode colours.
    let mut warning_dark_syntax = syntax_theme(&[
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
    warning_dark_syntax.insert("markup.heading".into(), "#c96870".into());
    themes.insert(
        "warning-dark".into(),
        ThemeDefinition {
            // Two steps lighter than the surface's original `#16181d`: the
            // active pane sits where the inactive pane used to under the
            // first step, and inactive and overlay grounds are derived a
            // further step lighter still, so the three stay in order.
            background: "#282a2f".into(),
            foreground: "#b9b9be".into(),
            muted: "#8b8b90".into(),
            // The marker is an explicit hex rather than a value derived from
            // `background`, so it moves with the ground by hand: 22 levels
            // off it, the separation the surface's first step settled on.
            whitespace: Some("#3e4045".into()),
            jump_text_muted: None,
            accent: "#c96870".into(),
            // The accent stays the brand red and keeps colouring the active
            // pane border; `command` takes the palette's command names off it
            // so the two can differ, and does, in blue.
            command: Some("#6cb6ff".into()),
            // Normal is green, Insert is the brand red, Select is pink, and
            // Command is the blue the palette lists commands in, so the mode
            // label answers the same colour question the reader just asked of
            // the palette. Normal therefore names its colour instead of
            // tracking `accent`.
            cursor_normal: Some("#8ddb8c".into()),
            cursor_insert: Some("#c96870".into()),
            cursor_replace: Some("#d2a8ff".into()),
            cursor_select: Some("#f07ab4".into()),
            cursor_command: Some("#6cb6ff".into()),
            directory: None,
            // The branded pair separates its two selection grounds by hue
            // alone rather than by the cool-against-warm every other
            // Runyte-original theme uses: the primary range answers Select
            // mode's own pink, and the secondary ground is saturated far
            // enough off the surface to be found at a glance while ordinary
            // text still clears 5:1 on it.
            selection: "#0b3f8c".into(),
            selection_primary: Some("#5e2e4d".into()),
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
            change_added: Some(CHANGE_ADDED_DARK.into()),
            change_modified: Some(CHANGE_MODIFIED_DARK.into()),
            change_removed: Some(CHANGE_REMOVED_DARK.into()),
            diff_added: Some(DIFF_ADDED_DARK.into()),
            diff_removed: Some(DIFF_REMOVED_DARK.into()),
            diff_changed: Some(DIFF_CHANGED_DARK.into()),
            syntax: warning_dark_syntax,
        },
    );
    let mut warning_light_syntax = syntax_theme(&[
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
    warning_light_syntax.insert("markup.heading".into(), "#a33d49".into());
    themes.insert(
        "warning-light".into(),
        ThemeDefinition {
            // Two steps darker than the surface's original `#ececef`, which
            // is the light mirror of what `warning-dark` does: the active
            // pane moves toward the inactive one rather than away from it, so
            // the pair separates its panes by the same amount either way.
            background: "#dadadc".into(),
            foreground: "#292a30".into(),
            muted: "#656872".into(),
            // The marker is an explicit hex rather than a value derived from
            // `background`, so it moves with the ground by hand: 30 levels
            // off it, just inside the 31-level "near background" ceiling
            // `every_theme_has_a_near_background_whitespace_color` enforces.
            whitespace: Some("#bcbcbe".into()),
            jump_text_muted: Some("#878a92".into()),
            accent: "#a33d49".into(),
            // See `warning-dark`: the accent keeps the pane border, and the
            // palette's command names are named separately so they can be
            // blue without taking the border with them.
            command: Some("#1f65a6".into()),
            cursor_normal: Some("#23733a".into()),
            cursor_insert: Some("#a33d49".into()),
            cursor_replace: Some("#754b97".into()),
            cursor_select: Some("#a4276f".into()),
            cursor_command: Some("#1f65a6".into()),
            directory: None,
            // See `warning-dark`: the same pink primary and vivid blue
            // secondary, carried down to hold their contrast against a light
            // ground instead of a dark one.
            selection: "#8fc6fb".into(),
            selection_primary: Some("#f2b8da".into()),
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
            change_added: Some(CHANGE_ADDED_LIGHT.into()),
            change_modified: Some(CHANGE_MODIFIED_LIGHT.into()),
            change_removed: Some(CHANGE_REMOVED_LIGHT.into()),
            diff_added: Some(DIFF_ADDED_LIGHT.into()),
            diff_removed: Some(DIFF_REMOVED_LIGHT.into()),
            diff_changed: Some(DIFF_CHANGED_LIGHT.into()),
            syntax: warning_light_syntax,
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
            // Unset: this palette keeps one accent for its borders and its
            // command names alike.
            command: None,
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
            change_added: Some(CHANGE_ADDED_DARK.into()),
            change_modified: Some(CHANGE_MODIFIED_DARK.into()),
            change_removed: Some(CHANGE_REMOVED_DARK.into()),
            diff_added: Some(DIFF_ADDED_DARK.into()),
            diff_removed: Some(DIFF_REMOVED_DARK.into()),
            diff_changed: Some(DIFF_CHANGED_DARK.into()),
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
            // Unset: this palette keeps one accent for its borders and its
            // command names alike.
            command: None,
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
            change_added: Some(CHANGE_ADDED_LIGHT.into()),
            change_modified: Some(CHANGE_MODIFIED_LIGHT.into()),
            change_removed: Some(CHANGE_REMOVED_LIGHT.into()),
            diff_added: Some(DIFF_ADDED_LIGHT.into()),
            diff_removed: Some(DIFF_REMOVED_LIGHT.into()),
            diff_changed: Some(DIFF_CHANGED_LIGHT.into()),
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
            // Unset: this palette keeps one accent for its borders and its
            // command names alike.
            command: None,
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
            change_added: Some(CHANGE_ADDED_LIGHT.into()),
            change_modified: Some(CHANGE_MODIFIED_LIGHT.into()),
            change_removed: Some(CHANGE_REMOVED_LIGHT.into()),
            diff_added: Some(DIFF_ADDED_LIGHT.into()),
            diff_removed: Some(DIFF_REMOVED_LIGHT.into()),
            diff_changed: Some(DIFF_CHANGED_LIGHT.into()),
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
            // Unset: this palette keeps one accent for its borders and its
            // command names alike.
            command: None,
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
            change_added: Some(CHANGE_ADDED_DARK.into()),
            change_modified: Some(CHANGE_MODIFIED_DARK.into()),
            change_removed: Some(CHANGE_REMOVED_DARK.into()),
            diff_added: Some(DIFF_ADDED_DARK.into()),
            diff_removed: Some(DIFF_REMOVED_DARK.into()),
            diff_changed: Some(DIFF_CHANGED_DARK.into()),
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
    // A green-phosphor terminal theme: an almost-black ground with a green
    // cast, one saturated code green, and a ladder of greens between them.
    // Blue is spent deliberately rather than spread around — the azure that
    // names functions, directories and Command mode, the cyan that names
    // types, and the indigo the Replace caret is diverted to — so the few
    // blue things on screen are the ones worth finding.
    //
    // Three colours are not the palette's own and cannot be: the Git gutter
    // and diff grounds are one shared semantic palette across every bundled
    // theme, and `built_in_jump_labels_are_red_and_one_neon_cyan_hue` requires
    // a red one-key jump label and the shared neon cyan for the two-key pair.
    // The red is therefore kept to exactly two roles, errors and that label,
    // and named once here so it reads as a decision rather than a leak.
    let matrix_red = "#ff5f52";
    themes.insert(
        "matrix".into(),
        ThemeDefinition {
            background: "#0b0f0c".into(),
            foreground: "#b6f2c8".into(),
            muted: "#4e8161".into(),
            // Named rather than derived, and hued rather than gray: the
            // marker is the faint grid under the text, so it takes the
            // background's own green cast 16 to 27 levels up from it.
            whitespace: Some("#1b2a1f".into()),
            // Dimmed text has to stay legible on both selection grounds while
            // still reading as dimmed, which the comment green is too dark to
            // do: it lands above 5:1 on either ground and 1.74:1 against
            // ordinary text.
            jump_text_muted: Some("#8fb89c".into()),
            accent: "#00ff41".into(),
            // Unset: only the branded pair splits the palette's command names
            // off the accent, and every other bundled theme keeps one colour
            // for both.
            command: None,
            // Normal is unset so the caret is the accent green itself, which
            // is what a phosphor terminal's cursor looked like. The rest walk
            // the hue circle from there toward blue — lime, spring green,
            // azure — so the four modes are told apart by hue alone without
            // leaving the palette. Replace is the indigo the green branch of
            // `default_replace_color` calls for; a green mode is already
            // spoken for, so it cannot also be green.
            cursor_normal: None,
            cursor_insert: Some("#a8ff60".into()),
            cursor_replace: Some("#7060ff".into()),
            cursor_select: Some("#25f5b0".into()),
            cursor_command: Some("#4d9fff".into()),
            directory: Some("#4d9fff".into()),
            // The usual cool-secondary, warm-primary split has no warm half
            // to spend here, so the pair separates by the palette's own two
            // families instead: ordinary ranges sit on deep green, and the
            // primary range takes the blue, which is the one the reader is
            // looking for. Ordinary text clears 8.8:1 on either.
            selection: "#14432c".into(),
            selection_primary: Some("#123a66".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#0f1511".into(),
            status_foreground: "#b6f2c8".into(),
            error: matrix_red.into(),
            warning: Some("#d8ff4a".into()),
            info: Some("#25f5b0".into()),
            jump_label_immediate: Some(matrix_red.into()),
            jump_label_primary: JUMP_LABEL_DARK_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_DARK_SECONDARY.into(),
            change_added: Some(CHANGE_ADDED_DARK.into()),
            change_modified: Some(CHANGE_MODIFIED_DARK.into()),
            change_removed: Some(CHANGE_REMOVED_DARK.into()),
            diff_added: Some(DIFF_ADDED_DARK.into()),
            diff_removed: Some(DIFF_REMOVED_DARK.into()),
            diff_changed: Some(DIFF_CHANGED_DARK.into()),
            // Six colours, assigned by how far a scope is from ordinary text:
            // identifiers and operators stay on the foreground, comments and
            // punctuation drop to the comment green, keywords and tags take
            // the accent, and the three families that name things — calls,
            // types, and literals — take azure, cyan, and lime. Strings take
            // the spring green, which is also what carries Markdown's inline
            // code and link URLs through `syntax_theme`'s derived roles;
            // headings follow `function` into azure, and bold and list text
            // follow `keyword` into the accent green.
            syntax: syntax_theme(&[
                ("attribute", "#25f5b0"),
                ("comment", "#4e8161"),
                ("constant", "#a8ff60"),
                ("constructor", "#4d9fff"),
                ("function", "#4d9fff"),
                ("keyword", "#00ff41"),
                ("label", "#a8ff60"),
                ("namespace", "#2bf7ff"),
                ("number", "#a8ff60"),
                ("operator", "#b6f2c8"),
                ("property", "#b6f2c8"),
                ("punctuation", "#4e8161"),
                ("string", "#25f5b0"),
                ("tag", "#00ff41"),
                ("type", "#2bf7ff"),
                ("variable", "#b6f2c8"),
            ]),
        },
    );
    // `ocean-light` and `ocean-dark` are one palette seen from two grounds:
    // the wave in the photograph they were drawn from, which has no warm
    // colour anywhere in it. Deep water is the ground, the turquoise face of
    // the wave is the accent, and foam and sky are what the palette lightens
    // toward. The pair share a structure rather than a set of hex values:
    // every role sits at the same contrast from its own ground as its
    // counterpart does from the opposite one, so a change to one of them
    // belongs in the other.
    //
    // The band is deliberately narrow. Ordinary text reads at 8.6:1 rather
    // than the 13:1 the first draft of these palettes used, which is the step
    // `terafox-soft` takes and for the same reason: long stretches of text
    // should not be the brightest thing on a dark ground, or the darkest on a
    // light one. It cannot soften further on either side — Runyte's shared
    // `diff_added` ground is what ordinary text has to stay legible on, and
    // 8.6:1 is where that floor sits. The hued colours follow the text down so
    // the palette keeps its order: keywords and strings a step below ordinary
    // text, calls and literals a step below those, comments at 3.9:1.
    //
    // Two warm colours survive that discipline because the interface cannot
    // afford to have them blend into the water: the error red, which is also
    // the one-key jump label `built_in_jump_labels_are_red_and_one_neon_cyan_hue`
    // requires to be red, and the amber a warning is drawn in. Each is named
    // once per variant so it reads as a decision rather than a leak. The Git
    // gutter and diff grounds are the shared semantic palette every bundled
    // theme carries, and the two-key jump labels are the shared neon cyan.
    //
    // The mode carets walk one hue ladder from the wave to dusk — turquoise,
    // cyan, sky, indigo, orchid — so the five are told apart by hue without
    // leaving the palette. Normal is unset, which leaves it on the accent
    // turquoise; because that reads as green to `default_replace_color`'s
    // hue test, Replace is diverted to the orchid end of the ladder rather
    // than to a green that would answer Normal.
    let ocean_dark_red = "#ff6b6b";
    themes.insert(
        "ocean-dark".into(),
        ThemeDefinition {
            background: "#0b1f2a".into(),
            foreground: "#a6bdc5".into(),
            muted: "#5b7f90".into(),
            // Named rather than derived, and hued rather than gray: the marker
            // is the faint grid under the text, so it keeps the ground's own
            // blue 12 to 21 levels up from it.
            whitespace: Some("#17323f".into()),
            // Dimmed text sits below the neon cyan of the two-key jump labels
            // it has to recede behind, and stays legible on both selection
            // grounds: 5.9:1 against the ground, above 3:1 on either
            // selection, and 1.45:1 from ordinary text.
            jump_text_muted: Some("#7e9eaa".into()),
            accent: "#00b6a2".into(),
            // Unset: only the branded pair splits the palette's command names
            // off the accent, and every other bundled theme keeps one colour
            // for both.
            command: None,
            cursor_normal: None,
            cursor_insert: Some("#69c3d0".into()),
            cursor_replace: Some("#ad81f3".into()),
            cursor_select: Some("#3f9de9".into()),
            cursor_command: Some("#677dea".into()),
            directory: Some("#3f9de9".into()),
            // The usual cool-secondary, warm-primary split has no warm half to
            // spend here, so the pair separates by the palette's own two
            // families instead: ordinary ranges sit on deep water blue, and
            // the primary range takes the wave's teal, which is the one the
            // reader is looking for. Both are dark enough that the softened
            // text still clears 4.5:1 on them, which is what fixes them this
            // far below the ground rather than just off it.
            selection: "#123f66".into(),
            selection_primary: Some("#00524d".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#0f2733".into(),
            status_foreground: "#a6bdc5".into(),
            error: ocean_dark_red.into(),
            warning: Some("#deb349".into()),
            info: Some("#34b78d".into()),
            jump_label_immediate: Some(ocean_dark_red.into()),
            jump_label_primary: JUMP_LABEL_DARK_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_DARK_SECONDARY.into(),
            change_added: Some(CHANGE_ADDED_DARK.into()),
            change_modified: Some(CHANGE_MODIFIED_DARK.into()),
            change_removed: Some(CHANGE_REMOVED_DARK.into()),
            diff_added: Some(DIFF_ADDED_DARK.into()),
            diff_removed: Some(DIFF_REMOVED_DARK.into()),
            diff_changed: Some(DIFF_CHANGED_DARK.into()),
            // Six colours besides ordinary text, assigned by how far a scope
            // is from it: identifiers and operators stay on the foreground,
            // comments and punctuation drop to the deep-water haze, keywords
            // and tags take the accent turquoise, and the families that name
            // things take the crest cyan (types), the sky (calls), and the
            // wave's own green (strings). Literals take the orchid, which is
            // also where the Replace caret sits. Through `syntax_theme`'s
            // derived roles that puts Markdown's inline code and link URLs on
            // the wave green, headings on the sky, and bold and list text on
            // the accent.
            syntax: syntax_theme(&[
                ("attribute", "#00b6a2"),
                ("comment", "#5b7f90"),
                ("constant", "#ad81f3"),
                ("constructor", "#3f9de9"),
                ("function", "#3f9de9"),
                ("keyword", "#00b6a2"),
                ("label", "#ad81f3"),
                ("namespace", "#00b0ca"),
                ("number", "#ad81f3"),
                ("operator", "#a6bdc5"),
                ("property", "#a6bdc5"),
                ("punctuation", "#5b7f90"),
                ("string", "#34b78d"),
                ("tag", "#00b6a2"),
                ("type", "#00b0ca"),
                ("variable", "#a6bdc5"),
            ]),
        },
    );
    // The same palette seen from the shallows rather than from deep water:
    // every role keeps its hue and its distance from the ground, and crosses
    // that ground, so the foam that was the brightest thing in `ocean-dark` is
    // the ground here and the deep water that was the ground is the text. The
    // ground is a shade below the palest foam for the same reason the text is
    // a shade above the deepest water: neither end of the pair should be the
    // brightest thing a reader looks at for an hour. See that theme for why
    // the two warm colours are there and why Replace sits at the orchid end.
    let ocean_light_red = "#b0281f";
    themes.insert(
        "ocean-light".into(),
        ThemeDefinition {
            background: "#dae7ef".into(),
            foreground: "#264050".into(),
            muted: "#517584".into(),
            // 20 to 29 levels off the ground, the same faint grid the dark
            // variant draws, carried to the other side of it.
            whitespace: Some("#bdd0db".into()),
            // The one role that does not mirror its counterpart's contrast.
            // Dimmed text has to recede behind the two-key jump labels, and
            // the shared light labels are far softer than the shared dark
            // ones, so this sits at 2.6:1 — where `light` and `paper` put
            // theirs — rather than at the dark variant's 5.9:1.
            jump_text_muted: Some("#7b929b".into()),
            accent: "#00594c".into(),
            // Unset: see `ocean-dark`.
            command: None,
            cursor_normal: None,
            cursor_insert: Some("#004464".into()),
            cursor_replace: Some("#742ebd".into()),
            cursor_select: Some("#0053b1".into()),
            cursor_command: Some("#5455d6".into()),
            directory: Some("#0053b1".into()),
            // The dark variant's two grounds carried across: deep water blue
            // for ordinary ranges and the wave's teal for the primary one.
            // These are the one pair mirrored on how far they look from the
            // ground rather than on contrast against it — a selection is an
            // area of colour, and the ratio that reads as a light touch on
            // deep water reads as a heavy block on foam.
            selection: "#8eb3cd".into(),
            selection_primary: Some("#7fc4b5".into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#ccdde7".into(),
            status_foreground: "#264050".into(),
            error: ocean_light_red.into(),
            warning: Some("#5d3400".into()),
            info: Some("#005845".into()),
            jump_label_immediate: Some(ocean_light_red.into()),
            jump_label_primary: JUMP_LABEL_LIGHT_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_LIGHT_SECONDARY.into(),
            change_added: Some(CHANGE_ADDED_LIGHT.into()),
            change_modified: Some(CHANGE_MODIFIED_LIGHT.into()),
            change_removed: Some(CHANGE_REMOVED_LIGHT.into()),
            diff_added: Some(DIFF_ADDED_LIGHT.into()),
            diff_removed: Some(DIFF_REMOVED_LIGHT.into()),
            diff_changed: Some(DIFF_CHANGED_LIGHT.into()),
            // The dark variant's six, role for role, at the same distance from
            // this ground as they sit from the other.
            syntax: syntax_theme(&[
                ("attribute", "#00594c"),
                ("comment", "#517584"),
                ("constant", "#742ebd"),
                ("constructor", "#0053b1"),
                ("function", "#0053b1"),
                ("keyword", "#00594c"),
                ("label", "#742ebd"),
                ("namespace", "#005670"),
                ("number", "#742ebd"),
                ("operator", "#264050"),
                ("property", "#264050"),
                ("punctuation", "#517584"),
                ("string", "#005845"),
                ("tag", "#00594c"),
                ("type", "#005670"),
                ("variable", "#264050"),
            ]),
        },
    );
    themes.into_iter()
}
