// SPDX-License-Identifier: MPL-2.0

//! Built-in adaptations of the Zenbones colorscheme collection.
//!
//! The values below come from the generated Vim highlight groups at upstream
//! revision `8304d8df9b823ff11e103afa62f38c39f534abe6`. Keeping the resolved RGB
//! values here avoids adding a color-space implementation or executing theme
//! code at runtime. See `THIRD_PARTY_NOTICES.md` and
//! `licenses/Zenbones-MIT.txt` for provenance and licensing.

use super::{JUMP_LABEL_DARK_PRIMARY, JUMP_LABEL_DARK_SECONDARY, ThemeDefinition, syntax_theme};

struct Palette {
    name: &'static str,
    light: bool,
    background: &'static str,
    foreground: &'static str,
    muted: &'static str,
    selection: &'static str,
    selection_primary: &'static str,
    status_background: &'static str,
    error: &'static str,
    warning: &'static str,
    info: &'static str,
    added: &'static str,
    diff_added: &'static str,
    diff_changed: &'static str,
    diff_removed: &'static str,
    constant: &'static str,
    function: &'static str,
    statement: &'static str,
    preprocessor: Option<&'static str>,
    r#type: &'static str,
    special: &'static str,
    delimiter: &'static str,
    identifier: &'static str,
    number: &'static str,
    string: &'static str,
}

macro_rules! palette {
    (
        $name:literal, $light:literal,
        ui: [$background:literal, $foreground:literal, $muted:literal,
            $selection:literal, $selection_primary:literal, $status:literal],
        semantic: [$error:literal, $warning:literal, $info:literal, $added:literal],
        diff: [$diff_added:literal, $diff_changed:literal, $diff_removed:literal],
        syntax: [$constant:literal, $function:literal, $statement:literal,
            $preprocessor:expr, $type:literal, $special:literal, $delimiter:literal,
            $identifier:literal, $number:literal, $string:literal]
    ) => {
        Palette {
            name: $name,
            light: $light,
            background: $background,
            foreground: $foreground,
            muted: $muted,
            selection: $selection,
            selection_primary: $selection_primary,
            status_background: $status,
            error: $error,
            warning: $warning,
            info: $info,
            added: $added,
            diff_added: $diff_added,
            diff_changed: $diff_changed,
            diff_removed: $diff_removed,
            constant: $constant,
            function: $function,
            statement: $statement,
            preprocessor: $preprocessor,
            r#type: $type,
            special: $special,
            delimiter: $delimiter,
            identifier: $identifier,
            number: $number,
            string: $string,
        }
    };
}

const PALETTES: &[Palette] = &[
    palette!(
        "duckbones-dark", false,
        ui: ["#0E101A", "#EBEFC0", "#5A5F7B", "#37382D", "#4D3191", "#232738"],
        semantic: ["#E03600", "#E39500", "#00A3CB", "#5DCD97"],
        diff: ["#15251C", "#17232A", "#311C1A"],
        syntax: ["#AEB18D", "#EBEFC0", "#795CCC", Some("#00A3CB"), "#898FB1", "#5DCD97", "#6D759D", "#C6CAA1", "#AEB18D", "#AEB18D"]
    ),
    palette!(
        "forestbones-dark", false,
        ui: ["#2C343A", "#E7DCC4", "#6E7B85", "#615B51", "#9E5179", "#3E4850"],
        semantic: ["#E67C7F", "#DDBD7F", "#7FBCB4", "#A9C181"],
        diff: ["#3E482D", "#304946", "#643839"],
        syntax: ["#ADA28B", "#E7DCC4", "#A9C181", Some("#83C193"), "#7FBCB4", "#B5AA92", "#7B8E9D", "#C6BAA0", "#ADA28B", "#ADA28B"]
    ),
    palette!(
        "forestbones-light", true,
        ui: ["#FAF3E1", "#4F5B62", "#9A9071", "#D3DFE6", "#EEBADB", "#E3D191"],
        semantic: ["#F85550", "#DEA000", "#3A94C4", "#8DA200"],
        diff: ["#DDE7BD", "#DCE3EB", "#EEDFDF"],
        syntax: ["#73848D", "#4F5B62", "#8DA200", Some("#36A87E"), "#3A94C4", "#6E7F88", "#92865B", "#63727A", "#73848D", "#73848D"]
    ),
    palette!(
        "kanagawabones-dark", false,
        ui: ["#1F1F28", "#DDD8BB", "#696977", "#49473E", "#614A82", "#363644"],
        semantic: ["#E46A78", "#E5C283", "#7EB3C9", "#98BC6D"],
        diff: ["#2A331F", "#22333A", "#47272A"],
        syntax: ["#A29E89", "#DDD8BB", "#DDD8BB", None, "#9797A5", "#ADA992", "#7D7D8D", "#BBB79E", "#A29E89", "#A29E89"]
    ),
    palette!(
        "neobones-dark", false,
        ui: ["#0F191F", "#C6D5CF", "#536977", "#3A3E3D", "#62415B", "#20303A"],
        semantic: ["#DE6E7C", "#B77E64", "#8190D4", "#90FF6B"],
        diff: ["#1C2C19", "#1F2645", "#3B2023"],
        syntax: ["#939E99", "#C6D5CF", "#C6D5CF", None, "#6E99B2", "#9AA6A1", "#5B7E94", "#A7B3AE", "#939E99", "#939E99"]
    ),
    palette!(
        "neobones-light", true,
        ui: ["#E5EDE6", "#202E18", "#878D88", "#ADE48C", "#DCB5D4", "#C2CFC4"],
        semantic: ["#A8334C", "#944927", "#286486", "#567A30"],
        diff: ["#C8E2B5", "#D1DBE5", "#EAD5D7"],
        syntax: ["#476038", "#202E18", "#202E18", None, "#495C4C", "#415934", "#7B837C", "#364A2A", "#476038", "#476038"]
    ),
    palette!(
        "nordbones-dark", false,
        ui: ["#2F3541", "#EBEEF3", "#737C90", "#545F70", "#84637E", "#414959"],
        semantic: ["#C1616A", "#CF866F", "#8FBCBA", "#A4BE8D"],
        diff: ["#3D4B2F", "#324B4B", "#663A3E"],
        syntax: ["#9EAFC9", "#87BFCE", "#81A1C1", None, "#5E81AB", "#ABBAD0", "#818EAB", "#EBEEF3", "#8FBCBA", "#9EAFC9"]
    ),
    palette!(
        "rosebones-dark", false,
        ui: ["#1A1825", "#E1D4D4", "#69657E", "#523A39", "#673592", "#312E43"],
        semantic: ["#EB7193", "#F6C074", "#9CCFD8", "#317490"],
        diff: ["#1D2C34", "#1C2D2F", "#3D2229"],
        syntax: ["#BC9493", "#E1D4D4", "#317490", None, "#DFDEF1", "#9CCFD8", "#7D7997", "#CAB0AF", "#BC9493", "#BC9493"]
    ),
    palette!(
        "rosebones-light", true,
        ui: ["#FBF6F0", "#724341", "#A18E72", "#EADDDC", "#D1C9DC", "#ECD0A9"],
        semantic: ["#B5637A", "#EC9D33", "#5795A0", "#286A84"],
        diff: ["#DDE7ED", "#D6E9ED", "#F0E2E5"],
        syntax: ["#AB6763", "#724341", "#286A84", None, "#57527A", "#5795A0", "#9B835D", "#935855", "#AB6763", "#AB6763"]
    ),
    palette!(
        "seoulbones-dark", false,
        ui: ["#4B4B4B", "#DDDDDD", "#719871", "#777777", "#8283AD", "#5E5E5E"],
        semantic: ["#E388A3", "#FFDF9B", "#97BDDE", "#98BD99"],
        diff: ["#406742", "#466177", "#82505E"],
        syntax: ["#A3A3A3", "#DFDFC1", "#97BDDE", Some("#D590A3"), "#AEAEAE", "#BCBCD3", "#9B9B9B", "#DDDDDD", "#F7E0B3", "#ABC4DB"]
    ),
    palette!(
        "seoulbones-light", true,
        ui: ["#E2E2E2", "#555555", "#628562", "#CCCCCC", "#CBB1CA", "#C4C4C4"],
        semantic: ["#DC5284", "#C48562", "#0084A3", "#628562"],
        diff: ["#AEDEAE", "#C0D5E0", "#E5CBD1"],
        syntax: ["#7C7C7C", "#6C6B20", "#0084A3", Some("#BE6A84"), "#6D4C52", "#755F74", "#7C7C7C", "#555555", "#896500", "#4A7587"]
    ),
    palette!(
        "tokyobones-dark", false,
        ui: ["#1A1B26", "#C0CAF5", "#65677D", "#2C4075", "#6E20BD", "#303142"],
        semantic: ["#F77890", "#E1B068", "#7BA2F7", "#74DBCB"],
        diff: ["#1D2F2C", "#212C44", "#412428"],
        syntax: ["#7592EA", "#C0CAF5", "#BB9BF7", Some("#BB9BF7"), "#9394AA", "#7BA2F7", "#787A94", "#98ABEF", "#2BC4DE", "#7592EA"]
    ),
    palette!(
        "tokyobones-light", true,
        ui: ["#D6D7DC", "#333A57", "#7C7E89", "#BBC0D8", "#B3A9C9", "#B9BBC3"],
        semantic: ["#8B4351", "#8F5E14", "#34548C", "#34645D"],
        diff: ["#A9CEC7", "#C0C6D8", "#DFBEC3"],
        syntax: ["#5B6694", "#333A57", "#5A4A79", Some("#5A4A79"), "#484F6B", "#34548C", "#737686", "#4A537A", "#176775", "#5B6694"]
    ),
    palette!(
        "vimbones-light", true,
        ui: ["#F0F0CA", "#353535", "#8C8C7C", "#D7D7D7", "#DEB9D6", "#D1D1B0"],
        semantic: ["#A8334C", "#944927", "#286486", "#4F6C31"],
        diff: ["#CBE5B8", "#D4DEE7", "#EBD8DA"],
        syntax: ["#636363", "#353535", "#156A29", Some("#35663D"), "#5B5B42", "#5C5C5C", "#85856F", "#505050", "#2A6535", "#636363"]
    ),
    palette!(
        "zenbones-dark", false,
        ui: ["#1C1917", "#B4BDC3", "#6E6763", "#3D4042", "#65435E", "#352F2D"],
        semantic: ["#DE6E7C", "#B77E64", "#6099C0", "#819B69"],
        diff: ["#232D1A", "#1D2C36", "#3E2225"],
        syntax: ["#868C91", "#B4BDC3", "#B4BDC3", None, "#A1938C", "#8D9499", "#867A74", "#979FA4", "#868C91", "#868C91"]
    ),
    palette!(
        "zenbones-light", true,
        ui: ["#F0EDEC", "#2C363C", "#948985", "#CBD9E3", "#DEB9D6", "#D6CDC9"],
        semantic: ["#A8334C", "#944927", "#286486", "#4F6C31"],
        diff: ["#CBE5B8", "#D4DEE7", "#EBD8DA"],
        syntax: ["#556570", "#2C363C", "#2C363C", None, "#6A5549", "#4F5E68", "#8E817B", "#44525B", "#556570", "#556570"]
    ),
    palette!(
        "zenburned-dark", false,
        ui: ["#404040", "#F0E4CF", "#848484", "#746956", "#9C6992", "#555555"],
        semantic: ["#E3716E", "#B77E64", "#6099C0", "#819B69"],
        diff: ["#475737", "#3D5568", "#764544"],
        syntax: ["#BAA681", "#F0E4CF", "#DCA2A2", Some("#FFCDAB"), "#A8A8A8", "#F0F08F", "#939393", "#D5BE95", "#BAA681", "#BAA681"]
    ),
    palette!(
        "zenwritten-dark", false,
        ui: ["#191919", "#BBBBBB", "#686868", "#404040", "#65435E", "#303030"],
        semantic: ["#DE6E7C", "#B77E64", "#6099C0", "#819B69"],
        diff: ["#232D1A", "#1D2C36", "#3E2225"],
        syntax: ["#8B8B8B", "#BBBBBB", "#BBBBBB", None, "#969696", "#939393", "#7C7C7C", "#9E9E9E", "#8B8B8B", "#8B8B8B"]
    ),
    palette!(
        "zenwritten-light", true,
        ui: ["#EEEEEE", "#353535", "#8B8B8B", "#D7D7D7", "#DEB9D6", "#CFCFCF"],
        semantic: ["#A8334C", "#944927", "#286486", "#4F6C31"],
        diff: ["#CBE5B8", "#D4DEE7", "#EBD8DA"],
        syntax: ["#636363", "#353535", "#353535", None, "#735057", "#5C5C5C", "#848484", "#505050", "#636363", "#636363"]
    ),
];

/// Runyte's dimmed text and selection grounds for palettes whose upstream
/// grounds cannot carry it.
///
/// An unfocused pane draws every cell in `jump_text_muted` while a terminal
/// review selection still fills whole cells with `selection_primary`. In these
/// palettes the upstream ground sits so close to the dimmed text that the
/// selected row is the one row of the pane that cannot be read. Each entry
/// brightens the dimmed text and deepens both grounds until the pair clears
/// the same boundary `nordfox-warm` holds: dimmed text at 3:1 against either
/// ground and ordinary text at 4.5:1.
///
/// The grounds therefore no longer differ in lightness, so what separates the
/// primary range from the rest is hue, the way `nordfox-warm` separates its
/// own pair. A ground keeps the palette's hue where deepening already left
/// the two far enough apart, and takes another hue from the same palette
/// where it did not.
fn dimmed_contrast(name: &str) -> Option<(&'static str, &'static str, &'static str)> {
    // (jump_text_muted, selection, selection_primary)
    Some(match name {
        // Nordbones' deepened grounds kept the palette's hues but almost none
        // of their chroma, which left both of them reading as the background
        // wherever a highlight was only a few cells wide. These are the same
        // two hues carrying enough saturation to be seen at that size.
        "nordbones-dark" => ("#9FA6B3", "#334E78", "#6E3763"),
        "rosebones-dark" => ("#8F8BA2", "#523A39", "#572D7C"),
        // Seoulbones grounds a mid-grey background, so the upstream grey
        // secondary was invisible on it whatever its lightness. It takes the
        // palette's rose instead — its `error` hue — which the blue-violet
        // primary is already far from.
        "seoulbones-dark" => ("#A6BFA6", "#4B2831", "#5B5C8A"),
        "tokyobones-dark" => ("#8C8EA2", "#2C4075", "#5C1B9E"),
        // Zenburned deepens to two warm grounds a shade apart, so its
        // secondary takes the palette's own blue — the hue of `info` and of
        // its changed-diff row — to keep the pair told apart by hue.
        "zenburned-dark" => ("#B4B4B4", "#43617A", "#7B5173"),
        _ => return None,
    })
}

/// Nordbones with the glare taken off its text.
///
/// Nordbones' foreground is the one colour in the palette far brighter than
/// the rest: it reads at 10.6:1 against the background where every syntax
/// colour the theme draws with sits between 3.0 and 6.3. Long stretches of
/// ordinary text are therefore the brightest thing on screen by a wide margin.
/// This variant brings that text to 7:1 — a shade below the `nordfox-warm` it
/// sits next to in the list — and changes nothing else. The background, the
/// accents, the selection grounds and the diff rows are all Nordbones'.
///
/// Only the roles that *were* the foreground move with it: ordinary text, the
/// identifier colour Nordbones sets to its foreground, and the Markdown groups
/// that follow it. The dimmed text steps down by the same
/// amount so it still reads as dimmed against the new foreground, and 7:1 is
/// the point where it can do that while staying 3:1 above both grounds — any
/// softer and the grounds would have to move too, which is what would stop
/// this being the same theme.
fn nordbones_dark_soft() -> ThemeDefinition {
    const FOREGROUND: &str = "#BAC4D5";
    const DIMMED: &str = "#9BA2B0";

    let base = PALETTES
        .iter()
        .find(|palette| palette.name == "nordbones-dark")
        .expect("nordbones-dark is one of the palettes above");
    let mut theme = base.theme();
    for color in theme.syntax.values_mut() {
        if color.eq_ignore_ascii_case(base.foreground) {
            FOREGROUND.clone_into(color);
        }
    }
    theme.foreground = FOREGROUND.into();
    theme.status_foreground = FOREGROUND.into();
    theme.jump_text_muted = Some(DIMMED.into());
    theme
}

pub(super) fn themes() -> impl Iterator<Item = (String, ThemeDefinition)> {
    PALETTES
        .iter()
        .map(|palette| (palette.name.to_owned(), palette.theme()))
        .chain(std::iter::once((
            "nordbones-dark-soft".to_owned(),
            nordbones_dark_soft(),
        )))
}

impl Palette {
    fn theme(&self) -> ThemeDefinition {
        let attribute = self.preprocessor.unwrap_or(self.statement);
        let mut syntax = syntax_theme(&[
            ("attribute", attribute),
            ("comment", self.muted),
            ("constant", self.constant),
            ("constructor", self.special),
            ("function", self.function),
            ("keyword", self.statement),
            ("label", self.statement),
            ("namespace", self.constant),
            ("number", self.number),
            ("operator", self.statement),
            ("property", self.identifier),
            ("punctuation", self.delimiter),
            ("string", self.string),
            ("tag", self.special),
            ("type", self.r#type),
            ("variable", self.identifier),
        ]);

        // Zenbones mostly expresses Markdown emphasis through terminal styles,
        // which Runyte's color-only syntax vocabulary cannot represent. Its
        // actual colored groups map as follows in both upstream specifications.
        for (scope, color) in [
            ("markup.bold", self.foreground),
            ("markup.heading", self.foreground),
            ("markup.italic", self.foreground),
            ("markup.link.text", self.special),
            ("markup.link.url", self.constant),
            ("markup.list", self.special),
            ("markup.quote", self.constant),
            ("markup.raw", self.constant),
        ] {
            syntax.insert(scope.to_owned(), color.to_owned());
        }

        let (jump_label_primary, jump_label_secondary) =
            if matches!(self.name, "seoulbones-dark" | "zenburned-dark") {
                // These palettes deliberately use mid-gray backgrounds. The
                // shared dark-theme cyan pair is too dim there, so retain its hue
                // with a brighter pair that meets the same text-contrast boundary.
                ("#8EEAF2", "#72D7E1")
            } else if self.light {
                // Several Zenbones light palettes are darker than Runyte's
                // original near-white themes. Use the same hue one step darker.
                ("#004C58", "#00616E")
            } else {
                (JUMP_LABEL_DARK_PRIMARY, JUMP_LABEL_DARK_SECONDARY)
            };
        // Jump labels are tiny, temporary navigation text rather than an
        // upstream highlight group. Use Runyte-specific reds with WCAG text
        // contrast even where the palette's diagnostic red is intentionally
        // softer against its background.
        let jump_label_immediate = if self.light { "#A8334C" } else { "#FFA0A8" };

        let (jump_text_muted, selection, selection_primary) = match dimmed_contrast(self.name) {
            Some((dimmed, selection, primary)) => (Some(dimmed.into()), selection, primary),
            None => (None, self.selection, self.selection_primary),
        };

        // Command mode is the one editor mode Zenbones has no upstream colour
        // for: these palettes name a blue, a red and an orange for the other
        // three and stop there. Like the jump labels above, the purple is
        // therefore Runyte's own rather than the palette's, and one pair is
        // enough — a single hue per ground keeps CMD the same colour across the
        // whole family, and both values clear text contrast against the
        // lightest dark palette and the darkest light one.
        let cursor_command = if self.light { "#7A3E9D" } else { "#C9A0F0" };

        ThemeDefinition {
            background: self.background.into(),
            foreground: self.foreground.into(),
            muted: self.muted.into(),
            whitespace: None,
            jump_text_muted,
            accent: self.info.into(),
            // Unset: this palette keeps one accent for its borders and its
            // command names alike.
            command: None,
            cursor_normal: Some(self.info.into()),
            cursor_insert: Some(self.error.into()),
            cursor_replace: None,
            cursor_select: Some(self.warning.into()),
            cursor_command: Some(cursor_command.into()),
            directory: Some(self.info.into()),
            selection: selection.into(),
            selection_primary: Some(selection_primary.into()),
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: self.status_background.into(),
            status_foreground: self.foreground.into(),
            error: self.error.into(),
            warning: Some(self.warning.into()),
            info: Some(self.info.into()),
            jump_label_immediate: Some(jump_label_immediate.into()),
            jump_label_primary: jump_label_primary.into(),
            jump_label_secondary: jump_label_secondary.into(),
            change_added: Some(self.added.into()),
            change_modified: Some(self.info.into()),
            change_removed: Some(self.error.into()),
            diff_added: Some(self.diff_added.into()),
            diff_removed: Some(self.diff_removed.into()),
            diff_changed: Some(self.diff_changed.into()),
            syntax,
        }
    }
}
