// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::command::GrammarKind;
use crate::notification::DEFAULT_HISTORY_LIMIT;

mod zenbones;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub editor: EditorConfig,
    pub workspace: WorkspaceConfig,
    pub lsp: LspConfig,
    pub git: GitConfig,
    pub notifications: NotificationsConfig,
    /// The theme to start in, or `None` to use [`DEFAULT_THEME`].
    ///
    /// Theme choices made inside the editor are persisted back to this field,
    /// so configuration remains the single source of truth.
    pub theme: Option<String>,
    pub themes: HashMap<String, ThemeDefinition>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    /// Number of newest workspace-lifetime notifications retained in memory.
    pub history_limit: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct GitConfig {
    /// Automatic refresh interval. Zero disables periodic Git work.
    pub refresh_interval_seconds: usize,
}

/// Largest interval accepted for automatic Git refreshes.
pub(crate) const MAX_GIT_REFRESH_INTERVAL_SECONDS: usize = 3_600;

/// Largest idle-retirement interval accepted for a persistent workspace host.
///
/// Thirty days is beyond the useful range for a minute-based timer. Zero has
/// its own meaning (never retire), so it remains the lower bound.
pub(crate) const MAX_IDLE_RETIREMENT_MINUTES: usize = 43_200;

/// The theme Runyte starts in when nothing else has been chosen.
pub const DEFAULT_THEME: &str = "light";

// Two-key jump labels use one neon-cyan hue. The second key recedes on dark
// backgrounds and advances on light backgrounds without changing hue.
const JUMP_LABEL_DARK_PRIMARY: &str = "#5fd7e7";
const JUMP_LABEL_DARK_SECONDARY: &str = "#4ab7c6";
const JUMP_LABEL_LIGHT_PRIMARY: &str = "#00616e";
const JUMP_LABEL_LIGHT_SECONDARY: &str = "#007583";

/// Language server configuration.
///
/// Servers are keyed by the language names in `syntax::grammars`, which is what
/// makes a buffer's language the same question for highlighting and for LSP.
/// An unlisted language simply has no server, which is the offline default.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct LspConfig {
    pub enable: bool,
    pub servers: HashMap<String, LanguageServerConfig>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LanguageServerConfig {
    pub command: PathBuf,
    pub args: Vec<String>,
    /// Passed verbatim as the `initializationOptions` of the handshake.
    pub initialization_options: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    /// Where this workspace keeps its local runtime state, normally
    /// `.runyte`. Relative paths are resolved from the discovered or
    /// explicitly confirmed project directory. The workspace's own root is
    /// that project directory, not this path.
    ///
    /// `root` is the original spelling and remains accepted. It was renamed
    /// because it read as the workspace's root while naming the state
    /// directory nested inside it.
    #[serde(alias = "root")]
    pub state: PathBuf,
    pub mode: WorkspaceMode,
    /// Minutes a clean host with no client or wait request remains alive.
    /// Zero disables automatic retirement.
    pub idle_retirement_minutes: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceMode {
    #[default]
    Standalone,
    Persistent,
}

impl WorkspaceMode {
    pub const ALL: &'static [Self] = &[Self::Standalone, Self::Persistent];
}

impl fmt::Display for WorkspaceMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Standalone => formatter.write_str("standalone"),
            Self::Persistent => formatter.write_str("persistent"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    pub grammar: GrammarKind,
    pub line_numbers: bool,
    pub tab_width: usize,
    /// Add syntax indentation and align list continuations when inserting a newline.
    pub smart_newline: bool,
    pub scroll_offset: usize,
    /// Number of cursor motions dispatched for one held-key repeat event.
    pub motion_repeat_multiplier: usize,
    pub show_hidden_files: bool,
    pub soft_wrap: bool,
    /// Maximum text width of the centred viewport used by `:zen`.
    pub zen_width: usize,
    /// Default character width used by hard-wrap and reflow.
    pub hard_wrap_width: usize,
    /// Remove spaces and tabs at line ends when writing text files.
    pub trim_trailing_whitespace: bool,
    /// Capture terminal mouse events for selection, scrolling, and resizing.
    pub mouse: bool,
    /// Offer words already open elsewhere in the workspace while typing.
    pub word_completion: bool,
    /// Prefix length before word-completion candidates appear.
    pub word_completion_minimum: usize,
    /// Move between panes with `Ctrl-h/j/k/l` alone, without the `Ctrl-w`
    /// prefix.
    ///
    /// Off by default because the four keys are not free everywhere. Turning
    /// them on takes `Ctrl-j` and `Ctrl-k` away from Insert-mode editing, and
    /// takes all four away from a live terminal's child, which is where
    /// `Ctrl-h` is backspace and `Ctrl-l` clears the screen. That is the same
    /// trade a tmux user makes for prefix-free pane movement, so it is theirs
    /// to make rather than the default.
    pub fast_pane_keys: bool,
    /// Gray out the text in every pane while a command prompt is open.
    ///
    /// On by default: a prompt takes the keyboard away from the panes, and
    /// dimming them says so at a glance rather than leaving the editor looking
    /// exactly as it does when the keys still reach the buffer. Turning it off
    /// keeps every pane at its ordinary colours, which is what someone
    /// reading a pane while composing a command wants.
    pub command_mode_dim: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ThemeDefinition {
    pub background: String,
    pub foreground: String,
    pub muted: String,
    /// Ordinary buffer text while `goto-word` labels are active. An omitted
    /// value uses `muted`, preserving themes written before jump dimming had
    /// its own role.
    pub jump_text_muted: Option<String>,
    pub accent: String,
    /// Normal-mode caret colour. An omitted value uses `accent`.
    pub cursor_normal: Option<String>,
    /// Insert-mode caret colour. An omitted value uses `error`.
    pub cursor_insert: Option<String>,
    /// Select-mode caret colour. An omitted value uses `warning`.
    pub cursor_select: Option<String>,
    /// Command-mode caret colour. An omitted value uses `info`, which is the
    /// one semantic role the other three modes have not already claimed, so a
    /// theme written before command mode had a colour still distinguishes all
    /// four.
    pub cursor_command: Option<String>,
    /// Directory entries in explorer buffers. An omitted value uses `accent`.
    pub directory: Option<String>,
    pub selection: String,
    /// Primary and ordinary Select-mode ranges. An omitted value uses
    /// `selection`, preserving custom themes written before this field existed.
    pub selection_primary: Option<String>,
    /// Background for non-contiguous fuzzy-grep match characters. An omitted
    /// value uses `selection`, so existing themes agree with secondary search
    /// selections.
    pub fuzzy_match_secondary: Option<String>,
    /// Background for a contiguous fuzzy-grep substring. An omitted value uses
    /// `selection_primary`, so existing themes agree with the primary search
    /// selection.
    pub fuzzy_match_primary: Option<String>,
    pub status_background: String,
    pub status_foreground: String,
    pub error: String,
    /// Warning notifications. Older themes fall back to `change_modified`.
    pub warning: Option<String>,
    /// Informational notifications. Older themes fall back to `change_added`.
    pub info: Option<String>,
    /// A one-key `goto-word` label, or the remaining key after narrowing. An
    /// omitted value uses `error`, preserving themes written before mixed
    /// labels existed.
    pub jump_label_immediate: Option<String>,
    /// The leading character of a `goto-word` jump label, which is the one
    /// people aim at, so it carries the stronger of the two colours.
    pub jump_label_primary: String,
    /// The trailing character of a jump label, kept less emphatic so a label
    /// reads as one token pointing at one place rather than as two loose
    /// letters. Its brightness direction depends on the theme background.
    pub jump_label_secondary: String,
    /// Gutter mark for a line this working tree added. An omitted value uses
    /// the terminal's own green, which is legible on any palette; the bundled
    /// themes name a colour that belongs to theirs.
    pub change_added: Option<String>,
    /// Gutter mark for a line whose content differs from the staged one.
    pub change_modified: Option<String>,
    /// Gutter mark for lines that were removed, drawn against the row that
    /// closed over them.
    pub change_removed: Option<String>,
    /// Line background for a line only the right side of a side-by-side diff
    /// has. Distinct from `change_added`, which is a gutter mark against
    /// Git's staged text: these three fill a whole line, so they must be
    /// tints of the background rather than the strong colours a mark uses. An
    /// omitted value leaves the line unfilled and lets the gutter bar carry
    /// the difference on its own.
    pub diff_added: Option<String>,
    /// Line background for a line only the left side of a diff has.
    pub diff_removed: Option<String>,
    /// Line background for a line that answers to a different line on the
    /// other side.
    pub diff_changed: Option<String>,
    /// Per-scope syntax colours, keyed by the names in `syntax::SCOPES`.
    /// Unlisted scopes fall back to the theme foreground.
    #[serde(default)]
    pub syntax: HashMap<String, String>,
}

/// A resolved presentation color independent of any frontend toolkit.
///
/// The variants intentionally match the color names accepted in Runyte's
/// configuration. Frontends translate these values at their own boundary;
/// core editor state never stores a Ratatui color.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Color {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Gray,
    DarkGray,
    Rgb(u8, u8, u8),
}

impl Color {
    /// The color a terminal is asked to paint, as sRGB channels.
    ///
    /// `Reset` has none: it defers to whatever the terminal already uses, and
    /// that is not knowable from here. The named variants use the standard
    /// palette values so a theme written with them is classified the same way
    /// a hex one is.
    pub(crate) fn channels(self) -> Option<(u8, u8, u8)> {
        Some(match self {
            Self::Reset => return None,
            Self::Black => (0x00, 0x00, 0x00),
            Self::Red => (0xaa, 0x00, 0x00),
            Self::Green => (0x00, 0xaa, 0x00),
            Self::Yellow => (0xaa, 0x55, 0x00),
            Self::Blue => (0x00, 0x00, 0xaa),
            Self::Magenta => (0xaa, 0x00, 0xaa),
            Self::Cyan => (0x00, 0xaa, 0xaa),
            Self::White => (0xff, 0xff, 0xff),
            Self::Gray => (0xaa, 0xaa, 0xaa),
            Self::DarkGray => (0x55, 0x55, 0x55),
            Self::Rgb(red, green, blue) => (red, green, blue),
        })
    }

    /// Relative luminance as defined by WCAG, or `None` for `Reset`.
    fn relative_luminance(self) -> Option<f64> {
        let (red, green, blue) = self.channels()?;
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        Some(0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue))
    }

    /// This colour moved one step away from the ground `appearance` names:
    /// toward white on a dark theme, toward black on a light one.
    ///
    /// The step is a fraction of the distance still remaining to that
    /// extreme, so a nearly-black and a nearly-white background move by about
    /// the same number of levels instead of one of them barely moving at all.
    /// `Reset` has no channels to move and is returned unchanged.
    fn stepped_off(self, appearance: ThemeAppearance, step: f64) -> Self {
        let Some((red, green, blue)) = self.channels() else {
            return self;
        };
        let stepped = |value: u8| {
            let value = f64::from(value);
            let stepped = match appearance {
                ThemeAppearance::Dark => value + (255.0 - value) * step,
                ThemeAppearance::Light => value * (1.0 - step),
            };
            stepped.round().clamp(0.0, 255.0) as u8
        };
        Self::Rgb(stepped(red), stepped(green), stepped(blue))
    }
}

/// Which ground a theme paints on.
///
/// Derived from the theme's background rather than from its name, so a theme
/// declared in a person's own configuration is classified the same way a
/// built-in one is and neither has to be told which it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeAppearance {
    Dark,
    Light,
}

impl fmt::Display for ThemeAppearance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Dark => "dark",
            Self::Light => "light",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub muted: Color,
    pub jump_text_muted: Color,
    pub accent: Color,
    pub cursor_normal: Color,
    pub cursor_insert: Color,
    pub cursor_select: Color,
    pub cursor_command: Color,
    pub directory: Color,
    pub selection: Color,
    pub selection_primary: Color,
    pub fuzzy_match_secondary: Color,
    pub fuzzy_match_primary: Color,
    pub status_background: Color,
    pub status_foreground: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub jump_label_immediate: Color,
    pub jump_label_primary: Color,
    pub jump_label_secondary: Color,
    pub change_added: Color,
    pub change_modified: Color,
    pub change_removed: Color,
    /// Whole-line backgrounds for a side-by-side diff. `None` means the theme
    /// named no fill, and the line keeps the ordinary background.
    pub diff_added: Option<Color>,
    pub diff_removed: Option<Color>,
    pub diff_changed: Option<Color>,
    /// Indexed by `syntax::Scope::index`; `None` means "use the foreground".
    pub syntax: Vec<Option<Color>>,
}

impl Theme {
    /// The ground an inactive pane paints on.
    ///
    /// It sits halfway between the active pane and an overlay, keeping the
    /// focused pane visually foremost without making another pane look like a
    /// popup. Like the overlay ground, this is derived for custom themes too.
    pub fn inactive_background(&self) -> Color {
        const STEP: f64 = 0.04;

        match self.appearance() {
            Some(appearance) => self.background.stepped_off(appearance, STEP),
            None => self.background,
        }
    }

    pub fn syntax_color(&self, scope: crate::syntax::Scope) -> Option<Color> {
        self.syntax.get(scope.index()).copied().flatten()
    }

    /// Whether the theme is a dark or a light one.
    ///
    /// `None` means the background is the terminal's own, which leaves the
    /// question unanswerable here; such a theme belongs to neither group.
    pub fn appearance(&self) -> Option<ThemeAppearance> {
        let luminance = self.background.relative_luminance()?;
        Some(if luminance < 0.5 {
            ThemeAppearance::Dark
        } else {
            ThemeAppearance::Light
        })
    }

    /// The ground an overlay paints on.
    ///
    /// One step off the pane's own background — lighter on a dark theme,
    /// darker on a light one — so a popup reads as floating above the text
    /// rather than as cut out of it. It is derived rather than named by the
    /// theme, so a theme written in a person's own configuration separates
    /// its overlays exactly as a bundled one does without having to declare a
    /// second background, and no theme can forget to.
    ///
    /// A theme painting on the terminal's own ground has no appearance to
    /// step off, and keeps one background everywhere.
    pub fn overlay_background(&self) -> Color {
        /// Far enough to draw an edge, close enough that both grounds still
        /// read as the same colour: about twenty of 255 levels per channel on
        /// the backgrounds the bundled themes use.
        const STEP: f64 = 0.08;

        match self.appearance() {
            Some(appearance) => self.background.stepped_off(appearance, STEP),
            None => self.background,
        }
    }
}

/// Catppuccin's shared palette roles, adapted to Runyte's presentation roles.
///
/// The four flavours intentionally share this mapping so a syntax or editor
/// role does not change meaning when someone moves between them. Selection and
/// diff backgrounds are palette-local tints because Runyte fills whole cells
/// for those roles, while Catppuccin's accent colours are designed as text.
struct CatppuccinPalette {
    base: &'static str,
    mantle: &'static str,
    text: &'static str,
    overlay0: &'static str,
    overlay2: &'static str,
    blue: &'static str,
    sapphire: &'static str,
    sky: &'static str,
    teal: &'static str,
    green: &'static str,
    yellow: &'static str,
    peach: &'static str,
    red: &'static str,
    mauve: &'static str,
    lavender: &'static str,
    jump_label_primary: &'static str,
    jump_label_secondary: &'static str,
    cursor_select: &'static str,
    selection: &'static str,
    selection_primary: &'static str,
    diff_added: &'static str,
    diff_removed: &'static str,
    diff_changed: &'static str,
}

fn catppuccin_theme(palette: CatppuccinPalette) -> ThemeDefinition {
    ThemeDefinition {
        background: palette.base.into(),
        foreground: palette.text.into(),
        muted: palette.overlay0.into(),
        jump_text_muted: None,
        accent: palette.blue.into(),
        cursor_normal: Some(palette.blue.into()),
        cursor_insert: Some(palette.red.into()),
        cursor_select: Some(palette.cursor_select.into()),
        cursor_command: Some(palette.mauve.into()),
        directory: Some(palette.blue.into()),
        selection: palette.selection.into(),
        selection_primary: Some(palette.selection_primary.into()),
        fuzzy_match_secondary: None,
        fuzzy_match_primary: None,
        status_background: palette.mantle.into(),
        status_foreground: palette.text.into(),
        error: palette.red.into(),
        warning: Some(palette.peach.into()),
        info: Some(palette.green.into()),
        jump_label_immediate: Some(palette.red.into()),
        jump_label_primary: palette.jump_label_primary.into(),
        jump_label_secondary: palette.jump_label_secondary.into(),
        change_added: Some(palette.green.into()),
        change_modified: Some(palette.yellow.into()),
        change_removed: Some(palette.red.into()),
        diff_added: Some(palette.diff_added.into()),
        diff_removed: Some(palette.diff_removed.into()),
        diff_changed: Some(palette.diff_changed.into()),
        syntax: syntax_theme(&[
            ("attribute", palette.yellow),
            ("comment", palette.overlay0),
            ("constant", palette.peach),
            ("constructor", palette.sapphire),
            ("function", palette.blue),
            ("keyword", palette.mauve),
            ("label", palette.sapphire),
            ("namespace", palette.teal),
            ("number", palette.peach),
            ("operator", palette.sky),
            ("property", palette.lavender),
            ("punctuation", palette.overlay2),
            ("string", palette.green),
            ("tag", palette.mauve),
            ("type", palette.yellow),
            ("variable", palette.text),
        ]),
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut themes = HashMap::new();
        let base16 = ThemeDefinition {
            cursor_insert: Some("#ab4642".into()),
            cursor_select: Some("#dc9656".into()),
            cursor_command: Some("#ba8baf".into()),
            directory: Some("#7cafc2".into()),
            selection: "#365864".into(),
            selection_primary: Some("#5a3b2a".into()),
            jump_label_immediate: Some("#e65c57".into()),
            ..ThemeDefinition::default()
        };
        themes.insert("base16".into(), base16);
        // `dark` and `light` are the two themes people reach for by name, so
        // they are neutral by design: no palette identity of their own, just a
        // legible pair that reads correctly on a dark and on a light terminal.
        themes.insert(
            "dark".into(),
            ThemeDefinition {
                background: "#16181d".into(),
                foreground: "#d6dae0".into(),
                muted: "#6b7280".into(),
                jump_text_muted: None,
                accent: "#6cb6ff".into(),
                cursor_normal: None,
                cursor_insert: Some("#f87171".into()),
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
                jump_text_muted: Some("#a8adb2".into()),
                accent: "#0550ae".into(),
                cursor_normal: Some("#0550ae".into()),
                cursor_insert: Some("#cf222e".into()),
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
                jump_text_muted: Some("#aaaaaa".into()),
                accent: "#005faf".into(),
                cursor_normal: Some("#005faf".into()),
                cursor_insert: Some("#af0000".into()),
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
                jump_text_muted: None,
                accent: "#fabd2f".into(),
                cursor_normal: None,
                cursor_insert: Some("#fb4934".into()),
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
                    ("punctuation", "#a89984"),
                    ("string", "#b8bb26"),
                    ("tag", "#fb4934"),
                    ("type", "#fabd2f"),
                    ("variable", "#ebdbb2"),
                ]),
            },
        );
        for (name, palette) in [
            (
                "everforest-dark-hard",
                EverforestBackground {
                    background: "#272e33",
                    status_background: "#2e383c",
                    selection: "#384b55",
                    selection_primary: "#45443c",
                    diff_added: "#3c4841",
                    diff_removed: "#493b40",
                    diff_changed: "#45443c",
                },
            ),
            (
                "everforest-dark-medium",
                EverforestBackground {
                    background: "#2d353b",
                    status_background: "#343f44",
                    selection: "#3a515d",
                    selection_primary: "#4d4c43",
                    diff_added: "#425047",
                    diff_removed: "#514045",
                    diff_changed: "#4d4c43",
                },
            ),
            (
                "everforest-dark-soft",
                EverforestBackground {
                    background: "#333c43",
                    status_background: "#3a464c",
                    selection: "#3f5865",
                    selection_primary: "#55544a",
                    diff_added: "#48584e",
                    diff_removed: "#59464c",
                    diff_changed: "#55544a",
                },
            ),
        ] {
            themes.insert(name.into(), everforest_dark(palette));
        }
        for (name, palette) in [
            (
                "everforest-light-hard",
                EverforestBackground {
                    background: "#fffbef",
                    status_background: "#f8f5e4",
                    selection: "#ecf5ed",
                    selection_primary: "#fef2d5",
                    diff_added: "#f3f5d9",
                    diff_removed: "#ffe7de",
                    diff_changed: "#fef2d5",
                },
            ),
            (
                "everforest-light-medium",
                EverforestBackground {
                    background: "#fdf6e3",
                    status_background: "#f4f0d9",
                    selection: "#e9f0e9",
                    selection_primary: "#faedcd",
                    diff_added: "#f0f1d2",
                    diff_removed: "#fde3da",
                    diff_changed: "#faedcd",
                },
            ),
            (
                "everforest-light-soft",
                EverforestBackground {
                    background: "#f3ead3",
                    status_background: "#eae4ca",
                    selection: "#e1e7dd",
                    selection_primary: "#f1e4c5",
                    diff_added: "#e5e6c5",
                    diff_removed: "#fadbd0",
                    diff_changed: "#f1e4c5",
                },
            ),
        ] {
            themes.insert(name.into(), everforest_light(palette));
        }
        themes.insert(
            "latte".into(),
            catppuccin_theme(CatppuccinPalette {
                base: "#eff1f5",
                mantle: "#e6e9ef",
                text: "#4c4f69",
                overlay0: "#9ca0b0",
                overlay2: "#7c7f93",
                blue: "#1e66f5",
                sapphire: "#209fb5",
                sky: "#04a5e5",
                teal: "#179299",
                green: "#40a02b",
                yellow: "#df8e1d",
                peach: "#fe640b",
                red: "#d20f39",
                mauve: "#8839ef",
                lavender: "#7287fd",
                jump_label_primary: JUMP_LABEL_LIGHT_PRIMARY,
                jump_label_secondary: JUMP_LABEL_LIGHT_SECONDARY,
                cursor_select: "#c45500",
                selection: "#c3d4f3",
                selection_primary: "#f6d3bd",
                diff_added: "#e3eed8",
                diff_removed: "#f4d9df",
                diff_changed: "#f5e3cf",
            }),
        );
        themes.insert(
            "frappe".into(),
            catppuccin_theme(CatppuccinPalette {
                base: "#303446",
                mantle: "#292c3c",
                text: "#c6d0f5",
                overlay0: "#737994",
                overlay2: "#949cbb",
                blue: "#8caaee",
                sapphire: "#85c1dc",
                sky: "#99d1db",
                teal: "#81c8be",
                green: "#a6d189",
                yellow: "#e5c890",
                peach: "#ef9f76",
                red: "#e78284",
                mauve: "#ca9ee6",
                lavender: "#babbf1",
                jump_label_primary: JUMP_LABEL_DARK_PRIMARY,
                jump_label_secondary: JUMP_LABEL_DARK_SECONDARY,
                cursor_select: "#ef9f76",
                selection: "#414c66",
                selection_primary: "#59473e",
                diff_added: "#35463d",
                diff_removed: "#4b373d",
                diff_changed: "#4a4038",
            }),
        );
        themes.insert(
            "macchiato".into(),
            catppuccin_theme(CatppuccinPalette {
                base: "#24273a",
                mantle: "#1e2030",
                text: "#cad3f5",
                overlay0: "#6e738d",
                overlay2: "#939ab7",
                blue: "#8aadf4",
                sapphire: "#7dc4e4",
                sky: "#91d7e3",
                teal: "#8bd5ca",
                green: "#a6da95",
                yellow: "#eed49f",
                peach: "#f5a97f",
                red: "#ed8796",
                mauve: "#c6a0f6",
                lavender: "#b7bdf8",
                jump_label_primary: JUMP_LABEL_DARK_PRIMARY,
                jump_label_secondary: JUMP_LABEL_DARK_SECONDARY,
                cursor_select: "#f5a97f",
                selection: "#35405b",
                selection_primary: "#504036",
                diff_added: "#293d35",
                diff_removed: "#402e36",
                diff_changed: "#40372f",
            }),
        );
        themes.insert(
            "mocha".into(),
            catppuccin_theme(CatppuccinPalette {
                base: "#1e1e2e",
                mantle: "#181825",
                text: "#cdd6f4",
                overlay0: "#6c7086",
                overlay2: "#9399b2",
                blue: "#89b4fa",
                sapphire: "#74c7ec",
                sky: "#89dceb",
                teal: "#94e2d5",
                green: "#a6e3a1",
                yellow: "#f9e2af",
                peach: "#fab387",
                red: "#f38ba8",
                mauve: "#cba6f7",
                lavender: "#b4befe",
                jump_label_primary: JUMP_LABEL_DARK_PRIMARY,
                jump_label_secondary: JUMP_LABEL_DARK_SECONDARY,
                cursor_select: "#fab387",
                selection: "#2e3d59",
                selection_primary: "#49392f",
                diff_added: "#23362d",
                diff_removed: "#38272f",
                diff_changed: "#382f28",
            }),
        );
        themes.insert("atom-one-light".into(), atom_one_light_theme());
        themes.insert("github-light".into(), github_light_theme());
        themes.insert("nordfox".into(), nordfox_theme());
        themes.insert("nordfox-warm".into(), nordfox_warm_theme());
        themes.insert("terafox".into(), terafox_theme());
        themes.extend(zenbones::themes());
        Self {
            editor: EditorConfig::default(),
            workspace: WorkspaceConfig::default(),
            lsp: LspConfig::default(),
            git: GitConfig::default(),
            notifications: NotificationsConfig::default(),
            theme: None,
            themes,
        }
    }
}

/// Runyte roles mapped onto projekt0n's GitHub Light palettes and syntax spec.
///
/// Source:
/// <https://github.com/projekt0n/github-nvim-theme/blob/c106c9472154d6b2c74b74565616b877ae8ed31d/lua/github-theme/palette/github_light.lua>
fn github_light_theme() -> ThemeDefinition {
    ThemeDefinition {
        background: "#ffffff".into(),
        foreground: "#1f2328".into(),
        muted: "#6e7781".into(),
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

#[derive(Clone, Copy)]
struct EverforestBackground {
    background: &'static str,
    status_background: &'static str,
    selection: &'static str,
    selection_primary: &'static str,
    diff_added: &'static str,
    diff_removed: &'static str,
    diff_changed: &'static str,
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
) -> ThemeDefinition {
    ThemeDefinition {
        background: background.background.into(),
        foreground: foreground.foreground.into(),
        muted: foreground.muted.into(),
        jump_text_muted: None,
        accent: foreground.green.into(),
        cursor_normal: Some(foreground.blue.into()),
        cursor_insert: Some(foreground.red.into()),
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
        change_added: Some(foreground.green.into()),
        change_modified: Some(foreground.yellow.into()),
        change_removed: Some(foreground.red.into()),
        diff_added: Some(background.diff_added.into()),
        diff_removed: Some(background.diff_removed.into()),
        diff_changed: Some(background.diff_changed.into()),
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

/// Runyte roles mapped onto Nightfox's canonical Nordfox palette and spec.
///
/// Source: <https://github.com/EdenEast/nightfox.nvim/blob/main/lua/nightfox/palette/nordfox.lua>
fn nordfox_theme() -> ThemeDefinition {
    ThemeDefinition {
        background: "#2e3440".into(),
        foreground: "#cdcecf".into(),
        muted: "#60728a".into(),
        jump_text_muted: None,
        accent: "#8cafd2".into(),
        cursor_normal: None,
        cursor_insert: Some("#bf616a".into()),
        cursor_select: Some("#c9826b".into()),
        cursor_command: Some("#b48ead".into()),
        directory: Some("#81a1c1".into()),
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
        change_added: Some("#a3be8c".into()),
        change_modified: Some("#ebcb8b".into()),
        change_removed: Some("#bf616a".into()),
        diff_added: Some("#3c4548".into()),
        diff_removed: Some("#403843".into()),
        diff_changed: Some("#364150".into()),
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
        jump_text_muted: None,
        accent: "#73a3b7".into(),
        cursor_normal: None,
        cursor_insert: Some("#e85c51".into()),
        cursor_select: Some("#ff8349".into()),
        cursor_command: Some("#ad5c7c".into()),
        directory: Some("#5a93aa".into()),
        selection: "#293e40".into(),
        selection_primary: Some("#425e5e".into()),
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
        change_added: Some("#7aa4a1".into()),
        change_modified: Some("#fda47f".into()),
        change_removed: Some("#e85c51".into()),
        diff_added: Some("#293e40".into()),
        diff_removed: Some("#4a3332".into()),
        diff_changed: Some("#31474b".into()),
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

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 5,
        }
    }
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            history_limit: DEFAULT_HISTORY_LIMIT,
        }
    }
}

impl Default for LspConfig {
    /// Only `rust-analyzer` is configured by default.
    ///
    /// Every other language Runyte highlights has a server that must be
    /// installed separately, and starting a process that is not there produces
    /// a startup error for no benefit. Adding one is two lines of YAML.
    fn default() -> Self {
        let mut servers = HashMap::new();
        servers.insert(
            "rust".to_owned(),
            LanguageServerConfig {
                command: PathBuf::from("rust-analyzer"),
                args: Vec::new(),
                initialization_options: None,
            },
        );
        Self {
            enable: true,
            servers,
        }
    }
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            state: PathBuf::from(".runyte"),
            mode: WorkspaceMode::Standalone,
            idle_retirement_minutes: 1440,
        }
    }
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            grammar: GrammarKind::Runyte,
            line_numbers: true,
            tab_width: 4,
            smart_newline: true,
            scroll_offset: 3,
            motion_repeat_multiplier: 2,
            show_hidden_files: false,
            soft_wrap: false,
            zen_width: 100,
            hard_wrap_width: 80,
            trim_trailing_whitespace: true,
            mouse: true,
            word_completion: true,
            word_completion_minimum: 3,
            fast_pane_keys: false,
            command_mode_dim: true,
        }
    }
}

impl Default for ThemeDefinition {
    fn default() -> Self {
        Self {
            background: "#181818".into(),
            foreground: "#d8d8d8".into(),
            muted: "#585858".into(),
            jump_text_muted: None,
            accent: "#7cafc2".into(),
            cursor_normal: None,
            cursor_insert: None,
            cursor_select: None,
            cursor_command: None,
            directory: None,
            selection: "#383838".into(),
            selection_primary: None,
            fuzzy_match_secondary: None,
            fuzzy_match_primary: None,
            status_background: "#282828".into(),
            status_foreground: "#d8d8d8".into(),
            error: "#ab4642".into(),
            warning: Some("#dc9656".into()),
            info: Some("#a1b56c".into()),
            jump_label_immediate: None,
            jump_label_primary: JUMP_LABEL_DARK_PRIMARY.into(),
            jump_label_secondary: JUMP_LABEL_DARK_SECONDARY.into(),
            change_added: Some("#a1b56c".into()),
            change_modified: Some("#f7ca88".into()),
            change_removed: Some("#ab4642".into()),
            diff_added: Some("#1e2a1a".into()),
            diff_removed: Some("#2e1c1c".into()),
            diff_changed: Some("#2c2618".into()),
            syntax: syntax_theme(&[
                ("attribute", "#f7ca88"),
                ("comment", "#585858"),
                ("constant", "#dc9656"),
                ("constructor", "#7cafc2"),
                ("function", "#7cafc2"),
                ("keyword", "#ba8baf"),
                ("label", "#dc9656"),
                ("namespace", "#f7ca88"),
                ("number", "#dc9656"),
                ("operator", "#d8d8d8"),
                ("punctuation", "#b8b8b8"),
                ("string", "#a1b56c"),
                ("tag", "#ab4642"),
                ("type", "#f7ca88"),
                ("variable", "#d8d8d8"),
            ]),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<(Self, Option<PathBuf>)> {
        let path = path.map(Path::to_path_buf).or_else(default_config_path);
        let Some(path) = path else {
            return Ok((Self::default(), None));
        };
        if path.is_absolute() {
            return Self::load_absolute(path);
        }
        let launch_directory = std::env::current_dir()
            .context("failed to resolve relative config path from the launch directory")?;
        Self::load_from(path, &launch_directory)
    }

    fn load_from(path: PathBuf, launch_directory: &Path) -> Result<(Self, Option<PathBuf>)> {
        let path = absolute_config_path(path, launch_directory);
        Self::load_absolute(path)
    }

    fn load_absolute(path: PathBuf) -> Result<(Self, Option<PathBuf>)> {
        debug_assert!(path.is_absolute());
        match fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Self::default(), Some(path)));
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect config {}", path.display()));
            }
        }

        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let mut config: Self = serde_yaml::from_str(&source)
            .with_context(|| format!("invalid YAML in {}", path.display()))?;
        config.merge_builtin_defaults();
        config
            .validate_settings()
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("invalid settings in {}", path.display()))?;
        Ok((config, Some(path)))
    }

    /// Merge additive built-ins after deserializing a user configuration.
    ///
    /// Kept as one seam so validation performed by lossless setting writes
    /// observes the same effective themes and servers as normal startup.
    pub(crate) fn with_builtin_defaults(mut self) -> Self {
        self.merge_builtin_defaults();
        self
    }

    pub(crate) fn validate_settings(&self) -> std::result::Result<(), String> {
        if !(1..=16).contains(&self.editor.tab_width) {
            return Err("editor.tab_width must be between 1 and 16".to_owned());
        }
        if self.editor.scroll_offset > 100 {
            return Err("editor.scroll_offset must be between 0 and 100".to_owned());
        }
        if !(1..=10).contains(&self.editor.motion_repeat_multiplier) {
            return Err("editor.motion_repeat_multiplier must be between 1 and 10".to_owned());
        }
        if !(1..=1000).contains(&self.editor.hard_wrap_width) {
            return Err("editor.hard_wrap_width must be between 1 and 1000".to_owned());
        }
        if !(1..=1000).contains(&self.editor.zen_width) {
            return Err("editor.zen_width must be between 1 and 1000".to_owned());
        }
        if !(1..=32).contains(&self.editor.word_completion_minimum) {
            return Err("editor.word_completion_minimum must be between 1 and 32".to_owned());
        }
        if !(crate::notification::MIN_HISTORY_LIMIT..=crate::notification::MAX_HISTORY_LIMIT)
            .contains(&self.notifications.history_limit)
        {
            return Err(format!(
                "notifications.history_limit must be between {} and {}",
                crate::notification::MIN_HISTORY_LIMIT,
                crate::notification::MAX_HISTORY_LIMIT
            ));
        }
        if self.workspace.idle_retirement_minutes > MAX_IDLE_RETIREMENT_MINUTES {
            return Err(format!(
                "workspace.idle_retirement_minutes must be between 0 and {MAX_IDLE_RETIREMENT_MINUTES}"
            ));
        }
        if self.git.refresh_interval_seconds > MAX_GIT_REFRESH_INTERVAL_SECONDS {
            return Err(format!(
                "git.refresh_interval_seconds must be between 0 and {MAX_GIT_REFRESH_INTERVAL_SECONDS}"
            ));
        }
        Ok(())
    }

    fn merge_builtin_defaults(&mut self) {
        let defaults = Self::default();
        for (name, theme) in defaults.themes {
            self.themes.entry(name).or_insert(theme);
        }
        // Adding one language server should not silently remove the built-in
        // ones, which is what replacing the whole map would do.
        for (language, server) in defaults.lsp.servers {
            self.lsp.servers.entry(language).or_insert(server);
        }
    }

    /// Every configured theme name, ordered so the list a person reads is the
    /// same on every run regardless of how the map happened to hash.
    pub fn theme_names(&self) -> Vec<&str> {
        let mut names = self.themes.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    /// The theme to start in, and the resolved palette for it.
    ///
    /// A name in the configuration file wins, then [`DEFAULT_THEME`]. A name
    /// that no longer resolves falls through to the default rather than
    /// refusing to start.
    pub fn startup_theme(&self) -> Result<(String, Theme)> {
        let candidates = self
            .theme
            .clone()
            .into_iter()
            .chain([DEFAULT_THEME.to_owned()]);
        let mut first_error = None;
        for name in candidates {
            match self.resolve_theme(&name) {
                Ok(theme) => return Ok((name, theme)),
                Err(error) => first_error.get_or_insert(error),
            };
        }
        Err(first_error.unwrap_or_else(|| anyhow::anyhow!("no theme could be resolved")))
    }

    pub fn resolve_theme(&self, name: &str) -> Result<Theme> {
        let definition = self
            .themes
            .get(name)
            .with_context(|| format!("unknown theme '{name}'"))?;
        Theme::try_from(definition)
    }
}

impl TryFrom<&ThemeDefinition> for Theme {
    type Error = anyhow::Error;

    fn try_from(value: &ThemeDefinition) -> Result<Self> {
        let accent = parse_color(&value.accent)?;
        let selection = parse_color(&value.selection)?;
        let selection_primary = value
            .selection_primary
            .as_deref()
            .map(parse_color)
            .transpose()?
            .unwrap_or(selection);
        let error = parse_color(&value.error)?;
        let muted = parse_color(&value.muted)?;
        let warning = optional_color(
            value
                .warning
                .as_deref()
                .or(value.change_modified.as_deref()),
            Color::Yellow,
        )?;
        let info = optional_color(
            value.info.as_deref().or(value.change_added.as_deref()),
            Color::Green,
        )?;
        Ok(Self {
            background: parse_color(&value.background)?,
            foreground: parse_color(&value.foreground)?,
            muted,
            jump_text_muted: value
                .jump_text_muted
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(muted),
            accent,
            cursor_normal: value
                .cursor_normal
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(accent),
            cursor_insert: value
                .cursor_insert
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(error),
            cursor_select: value
                .cursor_select
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(warning),
            cursor_command: value
                .cursor_command
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(info),
            directory: value
                .directory
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(accent),
            selection,
            selection_primary,
            fuzzy_match_secondary: value
                .fuzzy_match_secondary
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(selection),
            fuzzy_match_primary: value
                .fuzzy_match_primary
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(selection_primary),
            status_background: parse_color(&value.status_background)?,
            status_foreground: parse_color(&value.status_foreground)?,
            error,
            warning,
            info,
            jump_label_immediate: value
                .jump_label_immediate
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(error),
            jump_label_primary: parse_color(&value.jump_label_primary)?,
            jump_label_secondary: parse_color(&value.jump_label_secondary)?,
            change_added: optional_color(value.change_added.as_deref(), Color::Green)?,
            change_modified: optional_color(value.change_modified.as_deref(), Color::Yellow)?,
            change_removed: optional_color(value.change_removed.as_deref(), Color::Red)?,
            diff_added: value.diff_added.as_deref().map(parse_color).transpose()?,
            diff_removed: value.diff_removed.as_deref().map(parse_color).transpose()?,
            diff_changed: value.diff_changed.as_deref().map(parse_color).transpose()?,
            syntax: resolve_syntax_colors(&value.syntax)?,
        })
    }
}

/// Resolves a colour a theme may leave out.
///
/// The fallback is a terminal colour rather than a hex value on purpose: a
/// theme that says nothing about diff marks gets the palette the person
/// already chose for their terminal, which is legible there by construction.
fn optional_color(value: Option<&str>, fallback: Color) -> Result<Color> {
    value
        .map(parse_color)
        .transpose()
        .map(|color| color.unwrap_or(fallback))
}

/// Resolves a scope-name map into a slice indexed by `syntax::Scope`.
///
/// An unknown scope name is an error rather than a silent no-op: a typo in a
/// theme should be reported, not swallowed into an invisible colour.
fn resolve_syntax_colors(values: &HashMap<String, String>) -> Result<Vec<Option<Color>>> {
    let mut colors = vec![None; crate::syntax::SCOPES.len()];
    for (name, value) in values {
        let index = crate::syntax::SCOPES
            .iter()
            .position(|scope| scope == name)
            .with_context(|| {
                format!(
                    "unknown syntax scope '{name}'; valid scopes are {}",
                    crate::syntax::SCOPES.join(", ")
                )
            })?;
        colors[index] = Some(parse_color(value)?);
    }
    Ok(colors)
}

fn syntax_theme(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    let mut theme = pairs
        .iter()
        .map(|(scope, color)| ((*scope).to_owned(), (*color).to_owned()))
        .collect::<HashMap<_, _>>();

    // Bundled themes derive Markdown roles from their existing semantic
    // palette. The roles remain independently configurable in YAML, while a
    // custom theme written before they existed keeps its foreground fallback.
    for (markup, source) in [
        ("markup.bold", "keyword"),
        ("markup.heading", "function"),
        ("markup.italic", "attribute"),
        ("markup.link.text", "label"),
        ("markup.link.url", "string"),
        ("markup.list", "keyword"),
        ("markup.quote", "comment"),
        ("markup.raw", "string"),
    ] {
        if let Some(color) = theme.get(source).cloned() {
            theme.insert(markup.to_owned(), color);
        }
    }
    theme
}

/// Directory containing Runyte's default per-user configuration file.
pub fn default_config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|root| root.join("runyte"))
}

/// Absolute directory that owns a loaded or prospective configuration file.
///
/// Existing files are canonicalized so a symlink into project storage cannot
/// disguise an overlap. A missing file is resolved from the launch directory,
/// which is also how a relative `--config` path is interpreted by the loader.
pub fn config_root_for(path: &Path, launch_directory: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        launch_directory.join(path)
    };
    let resolved = absolute.canonicalize().unwrap_or_else(|_| {
        absolute
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .and_then(|parent| absolute.file_name().map(|name| parent.join(name)))
            .unwrap_or(absolute)
    });
    resolved
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(launch_directory)
        .to_path_buf()
}

fn default_config_path() -> Option<PathBuf> {
    default_config_root().map(|root| root.join("config.yaml"))
}

/// Pin configuration identity to the directory from which it was discovered.
///
/// Runyte may enter a newly initialized workspace after loading configuration.
/// Keeping a relative path here would make later settings writes address a
/// different file after that directory change. This deliberately does not
/// canonicalize: a configured symlink is retained as the authored identity and
/// the settings writer resolves its target while preserving the link itself.
fn absolute_config_path(path: PathBuf, launch_directory: &Path) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    launch_directory.join(path)
}

fn parse_color(value: &str) -> Result<Color> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        anyhow::ensure!(hex.len() == 6, "invalid color '{value}'");
        let number =
            u32::from_str_radix(hex, 16).with_context(|| format!("invalid color '{value}'"))?;
        return Ok(Color::Rgb(
            (number >> 16) as u8,
            (number >> 8) as u8,
            number as u8,
        ));
    }

    let color = match value.to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "grey" | "gray" => Color::Gray,
        "dark-grey" | "dark-gray" => Color::DarkGray,
        "reset" => Color::Reset,
        _ => anyhow::bail!("unknown color '{value}'"),
    };
    Ok(color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("runyte-config-{}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_hex_colors() {
        assert_eq!(
            parse_color("#12abEF").unwrap(),
            Color::Rgb(0x12, 0xab, 0xef)
        );
    }

    #[test]
    fn loading_rejects_editor_values_outside_the_settings_registry_bounds() {
        let directory = std::env::temp_dir();
        let path = directory.join(format!("runyte-invalid-editor-{}.yaml", std::process::id()));
        fs::write(&path, "editor:\n  tab_width: 0\n").unwrap();
        let error = format!("{:#}", Config::load(Some(&path)).unwrap_err());
        fs::remove_file(path).unwrap();
        assert!(error.contains("editor.tab_width must be between 1 and 16"));
    }

    #[test]
    fn hard_wrap_width_is_configurable_and_validated() {
        let config: Config = serde_yaml::from_str("editor:\n  hard_wrap_width: 100\n").unwrap();
        assert_eq!(config.editor.hard_wrap_width, 100);

        let directory = std::env::temp_dir();
        let path = directory.join(format!("runyte-invalid-wrap-{}.yaml", std::process::id()));
        fs::write(&path, "editor:\n  hard_wrap_width: 0\n").unwrap();
        let error = format!("{:#}", Config::load(Some(&path)).unwrap_err());
        fs::remove_file(path).unwrap();
        assert!(error.contains("editor.hard_wrap_width must be between 1 and 1000"));
    }

    #[test]
    fn zen_width_defaults_to_one_hundred_and_is_configurable_and_validated() {
        assert_eq!(Config::default().editor.zen_width, 100);
        let config: Config = serde_yaml::from_str("editor:\n  zen_width: 88\n").unwrap();
        assert_eq!(config.editor.zen_width, 88);

        let mut invalid = Config::default();
        invalid.editor.zen_width = 0;
        assert_eq!(
            invalid.validate_settings().unwrap_err(),
            "editor.zen_width must be between 1 and 1000"
        );
    }

    #[test]
    fn notification_history_defaults_to_fifty_and_is_bounded() {
        assert_eq!(Config::default().notifications.history_limit, 50);
        let configured: Config =
            serde_yaml::from_str("notifications:\n  history_limit: 75\n").unwrap();
        assert_eq!(configured.notifications.history_limit, 75);

        let mut invalid = Config::default();
        invalid.notifications.history_limit = 0;
        assert_eq!(
            invalid.validate_settings().unwrap_err(),
            "notifications.history_limit must be between 1 and 1000"
        );
    }

    #[test]
    fn motion_repeat_multiplier_defaults_to_two_and_is_bounded() {
        let config: Config =
            serde_yaml::from_str("editor:\n  motion_repeat_multiplier: 4\n").unwrap();
        assert_eq!(config.editor.motion_repeat_multiplier, 4);

        let directory = std::env::temp_dir();
        let path = directory.join(format!(
            "runyte-invalid-motion-repeat-{}.yaml",
            std::process::id()
        ));
        fs::write(&path, "editor:\n  motion_repeat_multiplier: 0\n").unwrap();
        let error = format!("{:#}", Config::load(Some(&path)).unwrap_err());
        fs::remove_file(path).unwrap();
        assert!(error.contains("editor.motion_repeat_multiplier must be between 1 and 10"));
    }

    #[test]
    fn editor_grammar_is_typed_and_accepts_the_helix_compatibility_name() {
        let helix: Config = serde_yaml::from_str("editor:\n  grammar: helix\n").unwrap();
        assert_eq!(helix.editor.grammar, GrammarKind::Runyte);
        assert!(serde_yaml::from_str::<Config>("editor:\n  grammar: vim\n").is_err());
        assert!(serde_yaml::from_str::<Config>("editor:\n  grammar: simple\n").is_err());
        assert!(serde_yaml::from_str::<Config>("editor:\n  grammar: emacs\n").is_err());
    }

    #[test]
    fn workspace_state_defaults_and_accepts_the_original_root_spelling() {
        assert_eq!(Config::default().workspace.state, PathBuf::from(".runyte"));
        assert_eq!(Config::default().workspace.mode, WorkspaceMode::Standalone);
        assert_eq!(Config::default().workspace.idle_retirement_minutes, 1440);

        let renamed: Config = serde_yaml::from_str("workspace:\n  state: .state\n").unwrap();
        assert_eq!(renamed.workspace.state, PathBuf::from(".state"));

        // No config struct denies unknown fields, so a dropped alias would
        // silently reinstate the default instead of failing to parse.
        let original: Config = serde_yaml::from_str("workspace:\n  root: .legacy\n").unwrap();
        assert_eq!(original.workspace.state, PathBuf::from(".legacy"));

        let persistent: Config =
            serde_yaml::from_str("workspace:\n  mode: persistent\n  idle_retirement_minutes: 30\n")
                .unwrap();
        assert_eq!(persistent.workspace.mode, WorkspaceMode::Persistent);
        assert_eq!(persistent.workspace.idle_retirement_minutes, 30);
    }

    #[test]
    fn a_filename_only_config_path_resolves_from_the_launch_directory() {
        let launch = TempDir::new();
        let config_path = launch.path("config.yaml");
        fs::write(&config_path, "theme: paper\n").unwrap();

        let (config, loaded_path) =
            Config::load_from(PathBuf::from("config.yaml"), &launch.0).unwrap();

        assert_eq!(loaded_path.as_deref(), Some(config_path.as_path()));
        assert_eq!(config.theme.as_deref(), Some("paper"));
        assert_eq!(
            config_root_for(Path::new("config.yaml"), &launch.0),
            launch.0
        );
    }

    #[test]
    fn loading_enforces_every_registry_backed_runtime_interval_bound() {
        let directory = TempDir::new();
        for (name, source, expected) in [
            (
                "idle.yaml",
                "workspace:\n  idle_retirement_minutes: 43201\n",
                "workspace.idle_retirement_minutes must be between 0 and 43200",
            ),
            (
                "git.yaml",
                "git:\n  refresh_interval_seconds: 3601\n",
                "git.refresh_interval_seconds must be between 0 and 3600",
            ),
        ] {
            let path = directory.path(name);
            fs::write(&path, source).unwrap();
            let error = format!("{:#}", Config::load(Some(&path)).unwrap_err());
            assert!(error.contains(expected), "{error}");
            assert_eq!(fs::read_to_string(path).unwrap(), source);
        }
    }

    #[cfg(unix)]
    #[test]
    fn loading_a_dangling_config_symlink_reports_the_broken_identity() {
        let directory = TempDir::new();
        let target = directory.path("missing.yaml");
        let link = directory.path("config.yaml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let error = format!("{:#}", Config::load(Some(&link)).unwrap_err());

        assert!(error.contains("failed to read config"), "{error}");
        assert!(error.contains("config.yaml"), "{error}");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!target.exists());
    }

    const ABSOLUTE_CONFIG_HELPER_PATH: &str = "RUNYTE_TEST_ABSOLUTE_CONFIG_PATH";
    const ABSOLUTE_CONFIG_HELPER_CWD: &str = "RUNYTE_TEST_REMOVED_CONFIG_CWD";

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess helper for absolute_config_load_does_not_require_a_live_cwd"]
    fn absolute_config_without_cwd_process_helper() {
        let Some(config_path) = std::env::var_os(ABSOLUTE_CONFIG_HELPER_PATH).map(PathBuf::from)
        else {
            return;
        };
        let cwd = PathBuf::from(
            std::env::var_os(ABSOLUTE_CONFIG_HELPER_CWD).expect("helper cwd was not supplied"),
        );
        std::env::set_current_dir(&cwd).unwrap();
        fs::remove_dir(&cwd).unwrap();
        assert!(std::env::current_dir().is_err());

        let (config, loaded_path) = Config::load(Some(&config_path)).unwrap();
        assert_eq!(loaded_path.as_deref(), Some(config_path.as_path()));
        assert_eq!(config.theme.as_deref(), Some("paper"));
    }

    #[cfg(unix)]
    #[test]
    fn absolute_config_load_does_not_require_a_live_cwd() {
        use std::process::Command;

        let directory = TempDir::new();
        let config_path = directory.path("config.yaml");
        let removed_cwd = directory.path("removed-cwd");
        fs::write(&config_path, "theme: paper\n").unwrap();
        fs::create_dir(&removed_cwd).unwrap();

        let output = Command::new(std::env::current_exe().unwrap())
            .arg("config::tests::absolute_config_without_cwd_process_helper")
            .arg("--ignored")
            .arg("--exact")
            .env(ABSOLUTE_CONFIG_HELPER_PATH, &config_path)
            .env(ABSOLUTE_CONFIG_HELPER_CWD, &removed_cwd)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "absolute config helper failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "absolute config helper did not run:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn rust_analyzer_is_configured_by_default_and_lsp_can_be_switched_off() {
        let config = Config::default();
        assert!(config.lsp.enable);
        assert_eq!(
            config.lsp.servers["rust"].command,
            PathBuf::from("rust-analyzer")
        );

        let disabled: Config = serde_yaml::from_str("lsp:\n  enable: false\n").unwrap();
        assert!(!disabled.lsp.enable);
    }

    #[test]
    fn adding_a_language_server_keeps_the_built_in_ones() {
        let directory = std::env::temp_dir();
        let path = directory.join(format!("runyte-lsp-config-{}.yaml", std::process::id()));
        fs::write(
            &path,
            "lsp:\n  servers:\n    json:\n      command: json-ls\n      args: ['--stdio']\n",
        )
        .unwrap();
        let (config, _) = Config::load(Some(&path)).unwrap();
        fs::remove_file(path).unwrap();

        assert_eq!(config.lsp.servers["json"].command, PathBuf::from("json-ls"));
        assert_eq!(config.lsp.servers["json"].args, vec!["--stdio".to_owned()]);
        assert_eq!(
            config.lsp.servers["rust"].command,
            PathBuf::from("rust-analyzer"),
            "adding one server must not remove the defaults"
        );
    }

    #[test]
    fn loading_custom_themes_keeps_builtins() {
        let directory = std::env::temp_dir();
        let path = directory.join(format!("runyte-config-test-{}.yaml", std::process::id()));
        fs::write(
            &path,
            "theme: midnight\nthemes:\n  midnight:\n    accent: '#ffffff'\n",
        )
        .unwrap();
        let (config, _) = Config::load(Some(&path)).unwrap();
        fs::remove_file(path).unwrap();

        assert!(config.resolve_theme("midnight").is_ok());
        assert!(config.resolve_theme("gruvbox").is_ok());
    }

    #[test]
    fn a_configured_theme_is_used_and_an_unknown_one_falls_back() {
        let config = Config::default();
        assert_eq!(config.startup_theme().unwrap().0, DEFAULT_THEME);

        let configured = Config {
            theme: Some("paper".into()),
            ..Config::default()
        };
        assert_eq!(configured.startup_theme().unwrap().0, "paper");

        let unknown = Config {
            theme: Some("deleted".into()),
            ..Config::default()
        };
        assert_eq!(unknown.startup_theme().unwrap().0, DEFAULT_THEME);
    }

    /// Every built-in theme is classified, and the classification agrees with
    /// whatever its name claims.
    ///
    /// Only the themes whose names say nothing are listed here, because those
    /// are the ones the appearance has to be read off the background for. A
    /// theme named `-dark` or `-light` needs no entry: the loop derives what it
    /// should be from the name and checks the background against it, which is
    /// what catches a palette copied into the wrong flavour. A new theme that
    /// neither says nor is listed fails rather than passing unclassified.
    #[test]
    fn theme_appearance_follows_the_background_rather_than_the_name() {
        let config = Config::default();
        let dark = [
            "base16",
            "frappe",
            "gruvbox",
            "macchiato",
            "mocha",
            "nordfox",
            "nordfox-warm",
            "terafox",
        ];
        let light = ["latte", "paper"];
        for name in config.theme_names() {
            let expected = if dark.contains(&name) {
                ThemeAppearance::Dark
            } else if light.contains(&name) {
                ThemeAppearance::Light
            } else if name.contains("dark") {
                ThemeAppearance::Dark
            } else if name.contains("light") {
                ThemeAppearance::Light
            } else {
                panic!(
                    "{name} neither names its appearance nor is listed in this test; \
                     add it to the dark or light list"
                );
            };
            assert_eq!(
                config.resolve_theme(name).unwrap().appearance(),
                Some(expected),
                "{name} should be a {} theme",
                if expected == ThemeAppearance::Dark {
                    "dark"
                } else {
                    "light"
                }
            );
        }
    }

    /// A background left to the terminal cannot be classified from here, so
    /// the theme belongs to neither group rather than being guessed into one.
    #[test]
    fn a_terminal_background_leaves_the_appearance_unknown() {
        let mut definition = Config::default().themes.remove("dark").unwrap();
        definition.background = "reset".to_owned();
        assert_eq!(Theme::try_from(&definition).unwrap().appearance(), None);
    }

    #[test]
    fn named_colors_are_classified_like_the_hex_ones() {
        let mut definition = Config::default().themes.remove("dark").unwrap();
        definition.background = "black".to_owned();
        assert_eq!(
            Theme::try_from(&definition).unwrap().appearance(),
            Some(ThemeAppearance::Dark)
        );
        definition.background = "white".to_owned();
        assert_eq!(
            Theme::try_from(&definition).unwrap().appearance(),
            Some(ThemeAppearance::Light)
        );
    }

    /// Every theme orders active panes, inactive panes, and overlays in the
    /// direction its own ground calls for, without naming extra colours.
    #[test]
    fn every_theme_orders_its_active_inactive_and_overlay_grounds() {
        let config = Config::default();
        for name in config.theme_names() {
            let theme = config.resolve_theme(name).unwrap();
            let inactive = theme.inactive_background();
            let overlay = theme.overlay_background();
            assert_ne!(
                inactive, theme.background,
                "{name} does not dim inactive panes"
            );
            assert_ne!(
                inactive, overlay,
                "{name} makes inactive panes look like overlays"
            );
            assert_ne!(
                overlay, theme.background,
                "{name} draws its overlays on the pane's own ground"
            );
            let pane = theme.background.relative_luminance().unwrap();
            let idle = inactive.relative_luminance().unwrap();
            let popup = overlay.relative_luminance().unwrap();
            match theme.appearance().unwrap() {
                ThemeAppearance::Dark => assert!(
                    pane < idle && idle < popup,
                    "{name} should order dark grounds active < inactive < overlay: \
                     {pane}, {idle}, {popup}"
                ),
                ThemeAppearance::Light => assert!(
                    pane > idle && idle > popup,
                    "{name} should order light grounds active > inactive > overlay: \
                     {pane}, {idle}, {popup}"
                ),
            }
        }
    }

    /// The step is a separation, not a recolouring: it moves every channel by
    /// about the same amount whichever end of the range the ground sits at,
    /// and stays small enough that the two grounds still read as one palette.
    #[test]
    fn the_overlay_step_is_small_and_even_at_both_ends_of_the_range() {
        let ground = |background: Color| {
            let mut definition = Config::default().themes.remove("dark").unwrap();
            definition.background = match background {
                Color::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
                _ => unreachable!("the test only names hex backgrounds"),
            };
            Theme::try_from(&definition).unwrap().overlay_background()
        };
        assert_eq!(ground(Color::Rgb(0x00, 0x00, 0x00)), Color::Rgb(20, 20, 20));
        assert_eq!(
            ground(Color::Rgb(0xff, 0xff, 0xff)),
            Color::Rgb(235, 235, 235)
        );
        assert_eq!(ground(Color::Rgb(0x16, 0x18, 0x1d)), Color::Rgb(41, 42, 47));
        assert_eq!(
            ground(Color::Rgb(0xef, 0xf1, 0xf5)),
            Color::Rgb(220, 222, 225)
        );

        let dark = Config::default().resolve_theme("dark").unwrap();
        assert_eq!(dark.inactive_background(), Color::Rgb(31, 33, 38));
        let light = Config::default().resolve_theme("light").unwrap();
        assert_eq!(light.inactive_background(), Color::Rgb(241, 241, 240));
    }

    /// A terminal's own ground cannot be stepped off, so such a theme keeps
    /// one background rather than being guessed into a second.
    #[test]
    fn a_terminal_background_leaves_overlays_on_the_same_ground() {
        let mut definition = Config::default().themes.remove("dark").unwrap();
        definition.background = "reset".to_owned();
        let theme = Theme::try_from(&definition).unwrap();
        assert_eq!(theme.inactive_background(), Color::Reset);
        assert_eq!(theme.overlay_background(), Color::Reset);
    }

    #[test]
    fn built_in_themes_are_listed_in_a_stable_order() {
        let config = Config::default();
        assert_eq!(
            config.theme_names(),
            [
                "atom-one-light",
                "base16",
                "dark",
                "duckbones-dark",
                "everforest-dark-hard",
                "everforest-dark-medium",
                "everforest-dark-soft",
                "everforest-light-hard",
                "everforest-light-medium",
                "everforest-light-soft",
                "forestbones-dark",
                "forestbones-light",
                "frappe",
                "github-light",
                "gruvbox",
                "kanagawabones-dark",
                "latte",
                "light",
                "macchiato",
                "mocha",
                "neobones-dark",
                "neobones-light",
                "nordbones-dark",
                "nordfox",
                "nordfox-warm",
                "paper",
                "rosebones-dark",
                "rosebones-light",
                "seoulbones-dark",
                "seoulbones-light",
                "terafox",
                "tokyobones-dark",
                "tokyobones-light",
                "vimbones-light",
                "zenbones-dark",
                "zenbones-light",
                "zenburned-dark",
                "zenwritten-dark",
                "zenwritten-light",
            ]
        );

        assert!(!config.theme_names().contains(&"randombones"));

        let dark = config.resolve_theme("dark").unwrap();
        let light = config.resolve_theme("light").unwrap();
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.foreground, light.foreground);
    }

    #[test]
    fn zenbones_variants_follow_the_pinned_upstream_generated_palettes() {
        fn rgb(value: u32) -> Color {
            Color::Rgb(
                ((value >> 16) & 0xff) as u8,
                ((value >> 8) & 0xff) as u8,
                (value & 0xff) as u8,
            )
        }

        let config = Config::default();
        for (name, background, foreground) in [
            ("duckbones-dark", 0x0e101a, 0xebefc0),
            ("forestbones-dark", 0x2c343a, 0xe7dcc4),
            ("forestbones-light", 0xfaf3e1, 0x4f5b62),
            ("kanagawabones-dark", 0x1f1f28, 0xddd8bb),
            ("neobones-dark", 0x0f191f, 0xc6d5cf),
            ("neobones-light", 0xe5ede6, 0x202e18),
            ("nordbones-dark", 0x2f3541, 0xebeef3),
            ("rosebones-dark", 0x1a1825, 0xe1d4d4),
            ("rosebones-light", 0xfbf6f0, 0x724341),
            ("seoulbones-dark", 0x4b4b4b, 0xdddddd),
            ("seoulbones-light", 0xe2e2e2, 0x555555),
            ("tokyobones-dark", 0x1a1b26, 0xc0caf5),
            ("tokyobones-light", 0xd6d7dc, 0x333a57),
            ("vimbones-light", 0xf0f0ca, 0x353535),
            ("zenbones-dark", 0x1c1917, 0xb4bdc3),
            ("zenbones-light", 0xf0edec, 0x2c363c),
            ("zenburned-dark", 0x404040, 0xf0e4cf),
            ("zenwritten-dark", 0x191919, 0xbbbbbb),
            ("zenwritten-light", 0xeeeeee, 0x353535),
        ] {
            let theme = config.resolve_theme(name).unwrap();
            assert_eq!(theme.background, rgb(background), "wrong {name} background");
            assert_eq!(theme.foreground, rgb(foreground), "wrong {name} foreground");
        }

        let tokyo = config.resolve_theme("tokyobones-dark").unwrap();
        assert_eq!(tokyo.selection, rgb(0x2c4075));
        assert_eq!(tokyo.selection_primary, rgb(0x6e20bd));
        assert_eq!(tokyo.status_background, rgb(0x303142));
        assert_eq!(tokyo.cursor_normal, rgb(0x7ba2f7));
        assert_eq!(tokyo.cursor_insert, rgb(0xf77890));
        assert_eq!(tokyo.cursor_select, rgb(0xe1b068));
        assert_eq!(tokyo.change_added, rgb(0x74dbcb));
        assert_eq!(tokyo.change_modified, rgb(0x7ba2f7));
        assert_eq!(tokyo.change_removed, rgb(0xf77890));
        assert_eq!(tokyo.diff_added, Some(rgb(0x1d2f2c)));
        assert_eq!(tokyo.diff_changed, Some(rgb(0x212c44)));
        assert_eq!(tokyo.diff_removed, Some(rgb(0x412428)));
        assert_eq!(
            tokyo.syntax_color(crate::syntax::Scope::named("keyword").unwrap()),
            Some(rgb(0xbb9bf7))
        );
        assert_eq!(
            tokyo.syntax_color(crate::syntax::Scope::named("number").unwrap()),
            Some(rgb(0x2bc4de))
        );
        assert_eq!(
            tokyo.syntax_color(crate::syntax::Scope::named("markup.heading").unwrap()),
            Some(rgb(0xc0caf5))
        );
    }

    #[test]
    fn github_light_follows_the_pinned_upstream_palette_and_spec() {
        let theme = Config::default().resolve_theme("github-light").unwrap();
        assert_eq!(theme.background, Color::Rgb(0xff, 0xff, 0xff));
        assert_eq!(theme.foreground, Color::Rgb(0x1f, 0x23, 0x28));
        assert_eq!(theme.selection, Color::Rgb(0xda, 0xe9, 0xf9));
        assert_eq!(theme.selection_primary, Color::Rgb(0xe1, 0xd1, 0xb3));
        assert_eq!(theme.fuzzy_match_secondary, Color::Rgb(0xc2, 0xe2, 0xff));
        assert_eq!(theme.fuzzy_match_primary, Color::Rgb(0xe1, 0xd1, 0xb3));
        assert_eq!(theme.status_background, Color::Rgb(0x50, 0x94, 0xe4));
        assert_eq!(theme.status_foreground, Color::Rgb(0xf6, 0xf8, 0xfa));
        assert_eq!(theme.cursor_normal, Color::Rgb(0x09, 0x69, 0xda));
        assert_eq!(theme.cursor_insert, Color::Rgb(0xd1, 0x24, 0x2f));
        assert_eq!(theme.cursor_select, Color::Rgb(0xbc, 0x4c, 0x00));
        assert_eq!(theme.directory, Color::Rgb(0x66, 0x39, 0xba));
        assert_eq!(theme.change_added, Color::Rgb(0x1a, 0x7f, 0x37));
        assert_eq!(theme.change_modified, Color::Rgb(0x9a, 0x67, 0x00));
        assert_eq!(theme.change_removed, Color::Rgb(0xd1, 0x24, 0x2f));
        assert_eq!(theme.diff_added, Some(Color::Rgb(0xb8, 0xd0, 0xbb)));
        assert_eq!(theme.diff_removed, Some(Color::Rgb(0xe4, 0xb7, 0xbe)));
        assert_eq!(theme.diff_changed, Some(Color::Rgb(0xd8, 0xca, 0xb3)));

        for (scope, color) in [
            ("comment", (0x57, 0x60, 0x6a)),
            ("function", (0x66, 0x39, 0xba)),
            ("keyword", (0xcf, 0x22, 0x2e)),
            ("string", (0x0a, 0x30, 0x69)),
            ("tag", (0x11, 0x63, 0x29)),
            ("type", (0x95, 0x38, 0x00)),
        ] {
            assert_eq!(
                theme.syntax_color(crate::syntax::Scope::named(scope).unwrap()),
                Some(Color::Rgb(color.0, color.1, color.2)),
                "wrong GitHub Light color for {scope}"
            );
        }
    }

    #[test]
    fn atom_one_light_uses_the_official_ui_and_syntax_palettes() {
        let theme = Config::default().resolve_theme("atom-one-light").unwrap();
        assert_eq!(theme.background, Color::Rgb(0xfa, 0xfa, 0xfa));
        assert_eq!(theme.foreground, Color::Rgb(0x38, 0x3a, 0x42));
        assert_eq!(theme.status_background, Color::Rgb(0xea, 0xea, 0xeb));
        assert_eq!(theme.status_foreground, Color::Rgb(0x42, 0x42, 0x43));
        assert_eq!(theme.cursor_normal, Color::Rgb(0x52, 0x6f, 0xff));
        assert_eq!(theme.cursor_insert, Color::Rgb(0xe4, 0x56, 0x49));
        assert_eq!(theme.cursor_select, Color::Rgb(0x98, 0x68, 0x01));
        assert_eq!(theme.directory, Color::Rgb(0x40, 0x78, 0xf2));
        assert_eq!(theme.change_added, Color::Rgb(0x2d, 0xb4, 0x48));
        assert_eq!(theme.change_modified, Color::Rgb(0xf2, 0xa6, 0x0d));
        assert_eq!(theme.change_removed, Color::Rgb(0xff, 0x14, 0x14));

        for (scope, color) in [
            ("comment", (0xa0, 0xa1, 0xa7)),
            ("function", (0x40, 0x78, 0xf2)),
            ("keyword", (0xa6, 0x26, 0xa4)),
            ("string", (0x50, 0xa1, 0x4f)),
            ("tag", (0xe4, 0x56, 0x49)),
            ("markup.heading", (0xe4, 0x56, 0x49)),
            ("markup.raw", (0x50, 0xa1, 0x4f)),
        ] {
            assert_eq!(
                theme.syntax_color(crate::syntax::Scope::named(scope).unwrap()),
                Some(Color::Rgb(color.0, color.1, color.2)),
                "wrong Atom One Light color for {scope}"
            );
        }
    }

    #[test]
    fn everforest_variants_use_the_upstream_palettes_and_runyte_roles() {
        let config = Config::default();
        for (name, background, status, selection, primary) in [
            (
                "everforest-dark-hard",
                (0x27, 0x2e, 0x33),
                (0x2e, 0x38, 0x3c),
                (0x38, 0x4b, 0x55),
                (0x45, 0x44, 0x3c),
            ),
            (
                "everforest-dark-medium",
                (0x2d, 0x35, 0x3b),
                (0x34, 0x3f, 0x44),
                (0x3a, 0x51, 0x5d),
                (0x4d, 0x4c, 0x43),
            ),
            (
                "everforest-dark-soft",
                (0x33, 0x3c, 0x43),
                (0x3a, 0x46, 0x4c),
                (0x3f, 0x58, 0x65),
                (0x55, 0x54, 0x4a),
            ),
            (
                "everforest-light-hard",
                (0xff, 0xfb, 0xef),
                (0xf8, 0xf5, 0xe4),
                (0xec, 0xf5, 0xed),
                (0xfe, 0xf2, 0xd5),
            ),
            (
                "everforest-light-medium",
                (0xfd, 0xf6, 0xe3),
                (0xf4, 0xf0, 0xd9),
                (0xe9, 0xf0, 0xe9),
                (0xfa, 0xed, 0xcd),
            ),
            (
                "everforest-light-soft",
                (0xf3, 0xea, 0xd3),
                (0xea, 0xe4, 0xca),
                (0xe1, 0xe7, 0xdd),
                (0xf1, 0xe4, 0xc5),
            ),
        ] {
            let theme = config.resolve_theme(name).unwrap();
            assert_eq!(
                theme.background,
                Color::Rgb(background.0, background.1, background.2)
            );
            assert_eq!(
                theme.status_background,
                Color::Rgb(status.0, status.1, status.2)
            );
            assert_eq!(
                theme.selection,
                Color::Rgb(selection.0, selection.1, selection.2)
            );
            assert_eq!(
                theme.selection_primary,
                Color::Rgb(primary.0, primary.1, primary.2)
            );
        }

        let dark = config.resolve_theme("everforest-dark-medium").unwrap();
        assert_eq!(dark.foreground, Color::Rgb(0xd3, 0xc6, 0xaa));
        assert_eq!(dark.cursor_normal, Color::Rgb(0x7f, 0xbb, 0xb3));
        assert_eq!(dark.cursor_insert, Color::Rgb(0xe6, 0x7e, 0x80));
        assert_eq!(dark.cursor_select, Color::Rgb(0xe6, 0x98, 0x75));
        assert_eq!(dark.directory, Color::Rgb(0x7f, 0xbb, 0xb3));
        assert_eq!(
            dark.syntax_color(crate::syntax::Scope::named("property").unwrap()),
            Some(Color::Rgb(0x7f, 0xbb, 0xb3))
        );

        let light = config.resolve_theme("everforest-light-medium").unwrap();
        assert_eq!(light.foreground, Color::Rgb(0x5c, 0x6a, 0x72));
        assert_eq!(light.cursor_normal, Color::Rgb(0x3a, 0x94, 0xc5));
        assert_eq!(light.cursor_insert, Color::Rgb(0xf8, 0x55, 0x52));
        assert_eq!(light.cursor_select, Color::Rgb(0xf5, 0x7d, 0x26));
        assert_eq!(light.directory, Color::Rgb(0x3a, 0x94, 0xc5));
        assert_eq!(
            light.syntax_color(crate::syntax::Scope::named("property").unwrap()),
            Some(Color::Rgb(0x3a, 0x94, 0xc5))
        );
    }

    #[test]
    fn all_catppuccin_flavours_resolve_with_their_official_core_palette() {
        let config = Config::default();
        let keyword = crate::syntax::Scope::named("keyword").unwrap();

        for (name, background, foreground, blue, mauve) in [
            (
                "latte",
                (0xef, 0xf1, 0xf5),
                (0x4c, 0x4f, 0x69),
                (0x1e, 0x66, 0xf5),
                (0x88, 0x39, 0xef),
            ),
            (
                "frappe",
                (0x30, 0x34, 0x46),
                (0xc6, 0xd0, 0xf5),
                (0x8c, 0xaa, 0xee),
                (0xca, 0x9e, 0xe6),
            ),
            (
                "macchiato",
                (0x24, 0x27, 0x3a),
                (0xca, 0xd3, 0xf5),
                (0x8a, 0xad, 0xf4),
                (0xc6, 0xa0, 0xf6),
            ),
            (
                "mocha",
                (0x1e, 0x1e, 0x2e),
                (0xcd, 0xd6, 0xf4),
                (0x89, 0xb4, 0xfa),
                (0xcb, 0xa6, 0xf7),
            ),
        ] {
            let theme = config.resolve_theme(name).unwrap();
            assert_eq!(
                theme.background,
                Color::Rgb(background.0, background.1, background.2)
            );
            assert_eq!(
                theme.foreground,
                Color::Rgb(foreground.0, foreground.1, foreground.2)
            );
            assert_eq!(theme.accent, Color::Rgb(blue.0, blue.1, blue.2));
            assert_eq!(theme.directory, theme.accent);
            assert_eq!(
                theme.syntax_color(keyword),
                Some(Color::Rgb(mauve.0, mauve.1, mauve.2))
            );
        }
    }

    #[test]
    fn bundled_themes_color_every_semantic_markdown_scope() {
        let config = Config::default();
        let markdown_scopes = [
            "markup.bold",
            "markup.heading",
            "markup.italic",
            "markup.link.text",
            "markup.link.url",
            "markup.list",
            "markup.quote",
            "markup.raw",
        ];

        for name in config.themes.keys() {
            let theme = config
                .resolve_theme(name)
                .unwrap_or_else(|error| panic!("bundled theme {name} failed: {error}"));
            for scope in markdown_scopes {
                let scope = crate::syntax::Scope::named(scope).unwrap();
                assert!(
                    theme.syntax_color(scope).is_some(),
                    "bundled theme {name} leaves {} uncolored",
                    scope.name()
                );
            }
        }
    }

    #[test]
    fn fox_themes_follow_the_authoritative_nightfox_palettes() {
        let config = Config::default();
        let nordfox = config.resolve_theme("nordfox").unwrap();
        assert_eq!(nordfox.background, Color::Rgb(0x2e, 0x34, 0x40));
        assert_eq!(nordfox.foreground, Color::Rgb(0xcd, 0xce, 0xcf));
        assert_eq!(nordfox.selection, Color::Rgb(0x3e, 0x4a, 0x5b));
        assert_eq!(nordfox.change_added, Color::Rgb(0xa3, 0xbe, 0x8c));
        assert_eq!(nordfox.diff_removed, Some(Color::Rgb(0x40, 0x38, 0x43)));
        assert_eq!(
            nordfox.syntax_color(crate::syntax::Scope::named("function").unwrap()),
            Some(Color::Rgb(0x8c, 0xaf, 0xd2))
        );

        let terafox = config.resolve_theme("terafox").unwrap();
        assert_eq!(terafox.background, Color::Rgb(0x15, 0x25, 0x28));
        assert_eq!(terafox.foreground, Color::Rgb(0xe6, 0xea, 0xea));
        assert_eq!(terafox.selection, Color::Rgb(0x29, 0x3e, 0x40));
        assert_eq!(terafox.change_removed, Color::Rgb(0xe8, 0x5c, 0x51));
        assert_eq!(terafox.diff_changed, Some(Color::Rgb(0x31, 0x47, 0x4b)));
        assert_eq!(
            terafox.syntax_color(crate::syntax::Scope::named("string").unwrap()),
            Some(Color::Rgb(0x7a, 0xa4, 0xa1))
        );
    }

    #[test]
    fn nordfox_warm_pairs_legible_dimmed_text_with_warm_selections() {
        fn contrast(left: Color, right: Color) -> f64 {
            let left = left.relative_luminance().unwrap();
            let right = right.relative_luminance().unwrap();
            (left.max(right) + 0.05) / (left.min(right) + 0.05)
        }

        let config = Config::default();
        let nordfox = config.resolve_theme("nordfox").unwrap();
        let warm = config.resolve_theme("nordfox-warm").unwrap();

        assert_eq!(warm.background, nordfox.background);
        assert_eq!(warm.foreground, nordfox.foreground);
        assert_eq!(warm.syntax, nordfox.syntax);
        assert_eq!(warm.muted, Color::Rgb(0x71, 0x83, 0x9a));
        assert_eq!(warm.jump_text_muted, Color::Rgb(0x92, 0x9f, 0xae));
        assert_eq!(warm.selection, Color::Rgb(0x60, 0x3f, 0x54));
        assert_eq!(warm.selection_primary, Color::Rgb(0x5c, 0x4e, 0x27));
        assert!(
            contrast(warm.jump_text_muted, warm.selection) >= 3.0,
            "dimmed text should remain readable on the pink selection"
        );
        assert!(
            contrast(warm.jump_text_muted, warm.selection_primary) >= 3.0,
            "dimmed text should remain readable on the yellow selection"
        );
        assert!(
            contrast(warm.foreground, warm.selection) >= 4.5,
            "ordinary text should remain readable on the pink selection"
        );
        assert!(
            contrast(warm.foreground, warm.selection_primary) >= 4.5,
            "ordinary text should remain readable on the yellow selection"
        );
    }

    #[test]
    fn every_built_in_command_cursor_is_legible_against_its_own_ground() {
        // The caret paints its glyph in the theme background, so the colour
        // behind it has to stand off that ground. Command is the only mode
        // colour Runyte chose for every bundled theme rather than lifting from
        // the palette, which is exactly why it is the one worth pinning here.
        let config = Config::default();
        for name in config.theme_names() {
            let theme = config.resolve_theme(name).unwrap();
            let ground = theme.background.relative_luminance().unwrap();
            let caret = theme.cursor_command.relative_luminance().unwrap();
            let contrast = (ground.max(caret) + 0.05) / (ground.min(caret) + 0.05);
            assert!(
                contrast >= 3.0,
                "{name} Command cursor obscures its glyph: {contrast}"
            );
        }
    }

    #[test]
    fn built_in_themes_use_mode_specific_cursor_colors() {
        let config = Config::default();
        for name in config.theme_names() {
            let theme = config.resolve_theme(name).unwrap();
            assert_ne!(
                theme.cursor_normal, theme.cursor_insert,
                "{name}: NOR = INS"
            );
            assert_ne!(
                theme.cursor_normal, theme.cursor_select,
                "{name}: NOR = SEL"
            );
            assert_ne!(
                theme.cursor_insert, theme.cursor_select,
                "{name}: INS = SEL"
            );
            assert_ne!(
                theme.cursor_normal, theme.cursor_command,
                "{name}: NOR = CMD"
            );
            assert_ne!(
                theme.cursor_insert, theme.cursor_command,
                "{name}: INS = CMD"
            );
            assert_ne!(
                theme.cursor_select, theme.cursor_command,
                "{name}: SEL = CMD"
            );
            // Command mode is the one mode whose colour is a Runyte decision
            // rather than an upstream one, so every theme is held to the hue
            // the four-mode vocabulary promises: blue, red, orange, purple.
            let (red, green, blue) = theme.cursor_command.channels().unwrap();
            assert!(
                red > green && blue > green,
                "{name}: CMD should read as purple"
            );
        }
        let light = config.resolve_theme("light").unwrap();
        assert_eq!(light.cursor_normal, Color::Rgb(0x05, 0x50, 0xae));
        assert_eq!(light.cursor_insert, Color::Rgb(0xcf, 0x22, 0x2e));
        assert_eq!(light.cursor_select, Color::Rgb(0x95, 0x38, 0x00));
        assert_eq!(light.cursor_command, Color::Rgb(0x82, 0x50, 0xdf));

        let paper = config.resolve_theme("paper").unwrap();
        assert_eq!(paper.cursor_normal, Color::Rgb(0x00, 0x5f, 0xaf));
        assert_eq!(paper.cursor_insert, Color::Rgb(0xaf, 0x00, 0x00));
        assert_eq!(paper.cursor_select, Color::Rgb(0xd7, 0x5f, 0x00));
        assert_eq!(paper.cursor_command, Color::Rgb(0x87, 0x00, 0xaf));

        for (name, insert, select) in [
            ("base16", (0xab, 0x46, 0x42), (0xdc, 0x96, 0x56)),
            ("dark", (0xf8, 0x71, 0x71), (0xf0, 0xa8, 0x68)),
            ("frappe", (0xe7, 0x82, 0x84), (0xef, 0x9f, 0x76)),
            ("gruvbox", (0xfb, 0x49, 0x34), (0xfe, 0x80, 0x19)),
            ("latte", (0xd2, 0x0f, 0x39), (0xc4, 0x55, 0x00)),
            ("macchiato", (0xed, 0x87, 0x96), (0xf5, 0xa9, 0x7f)),
            ("mocha", (0xf3, 0x8b, 0xa8), (0xfa, 0xb3, 0x87)),
        ] {
            let theme = config.resolve_theme(name).unwrap();
            assert_eq!(
                theme.cursor_insert,
                Color::Rgb(insert.0, insert.1, insert.2)
            );
            assert_eq!(
                theme.cursor_select,
                Color::Rgb(select.0, select.1, select.2)
            );
        }
    }

    #[test]
    fn built_in_themes_add_palette_local_primary_selection_colors() {
        let config = Config::default();
        for (name, secondary, primary) in [
            ("base16", (0x36, 0x58, 0x64), (0x5a, 0x3b, 0x2a)),
            ("dark", (0x34, 0x50, 0x6a), (0x5a, 0x3f, 0x2b)),
            ("frappe", (0x41, 0x4c, 0x66), (0x59, 0x47, 0x3e)),
            ("gruvbox", (0x3c, 0x51, 0x54), (0x66, 0x50, 0x2f)),
            ("latte", (0xc3, 0xd4, 0xf3), (0xf6, 0xd3, 0xbd)),
            ("light", (0xcf, 0xe3, 0xff), (0xff, 0xe2, 0xc2)),
            ("macchiato", (0x35, 0x40, 0x5b), (0x50, 0x40, 0x36)),
            ("mocha", (0x2e, 0x3d, 0x59), (0x49, 0x39, 0x2f)),
            ("paper", (0xaf, 0xd7, 0xff), (0xff, 0xd7, 0xaf)),
        ] {
            let theme = config.resolve_theme(name).unwrap();
            assert_eq!(
                theme.selection,
                Color::Rgb(secondary.0, secondary.1, secondary.2)
            );
            assert_eq!(
                theme.selection_primary,
                Color::Rgb(primary.0, primary.1, primary.2)
            );
        }
    }

    #[test]
    fn built_in_search_selection_palettes_are_legible_and_role_distinct() {
        fn channels(color: Color) -> (u8, u8, u8) {
            match color {
                Color::Rgb(red, green, blue) => (red, green, blue),
                other => panic!("built-in theme color should be RGB, got {other:?}"),
            }
        }

        fn luminance(color: Color) -> f64 {
            let (red, green, blue) = channels(color);
            let channel = |value: u8| {
                let value = f64::from(value) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue)
        }

        fn contrast(left: Color, right: Color) -> f64 {
            let left = luminance(left);
            let right = luminance(right);
            (left.max(right) + 0.05) / (left.min(right) + 0.05)
        }

        let config = Config::default();
        for name in [
            "base16",
            "dark",
            "frappe",
            "gruvbox",
            "latte",
            "light",
            "macchiato",
            "mocha",
            "paper",
        ] {
            let theme = config.resolve_theme(name).unwrap();
            assert!(
                contrast(theme.foreground, theme.selection) >= 4.5,
                "{name} secondary selection obscures text"
            );
            let minimum_background_contrast = match name {
                "base16" | "dark" => 2.0,
                _ => 1.1,
            };
            assert!(
                contrast(theme.background, theme.selection) >= minimum_background_contrast,
                "{name} secondary selection disappears into the editor background"
            );
            assert!(
                contrast(theme.foreground, theme.selection_primary) >= 4.5,
                "{name} primary selection obscures text"
            );
            assert!(
                contrast(theme.background, theme.cursor_select) >= 3.0,
                "{name} Select cursor obscures its glyph"
            );
            assert!(
                contrast(theme.background, theme.cursor_insert) >= 3.0,
                "{name} Insert cursor obscures its glyph"
            );
            assert!(
                contrast(theme.background, theme.cursor_command) >= 3.0,
                "{name} Command cursor obscures its glyph"
            );

            let (secondary_red, _, secondary_blue) = channels(theme.selection);
            assert!(
                secondary_blue > secondary_red,
                "{name} secondary selection should read as cool blue"
            );
            let (primary_red, primary_green, primary_blue) = channels(theme.selection_primary);
            assert!(
                primary_red > primary_green && primary_green > primary_blue,
                "{name} primary selection should read as warm orange"
            );
            let (select_red, select_green, select_blue) = channels(theme.cursor_select);
            assert!(
                select_red > select_green && select_green > select_blue,
                "{name} Select cursor should read as orange"
            );
            let (insert_red, insert_green, insert_blue) = channels(theme.cursor_insert);
            assert!(
                insert_red > insert_green && insert_red > insert_blue,
                "{name} Insert cursor should read as red"
            );
            let (command_red, command_green, command_blue) = channels(theme.cursor_command);
            assert!(
                command_red > command_green && command_blue > command_green,
                "{name} Command cursor should read as purple"
            );
        }
    }

    #[test]
    fn built_in_themes_use_palette_specific_blue_directory_colors() {
        let config = Config::default();
        for (name, expected) in [
            ("base16", (0x7c, 0xaf, 0xc2)),
            ("dark", (0x6c, 0xb6, 0xff)),
            ("frappe", (0x8c, 0xaa, 0xee)),
            ("gruvbox", (0x83, 0xa5, 0x98)),
            ("latte", (0x1e, 0x66, 0xf5)),
            ("light", (0x05, 0x50, 0xae)),
            ("macchiato", (0x8a, 0xad, 0xf4)),
            ("mocha", (0x89, 0xb4, 0xfa)),
            ("paper", (0x00, 0x5f, 0xaf)),
        ] {
            assert_eq!(
                config.resolve_theme(name).unwrap().directory,
                Color::Rgb(expected.0, expected.1, expected.2)
            );
        }
    }

    #[test]
    fn custom_theme_cursor_colors_fall_back_to_semantic_mode_colors() {
        let config: Config =
            serde_yaml::from_str("themes:\n  custom:\n    accent: '#123456'\n").unwrap();
        let theme = config.resolve_theme("custom").unwrap();

        assert_eq!(theme.cursor_normal, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(theme.cursor_insert, theme.error);
        assert_eq!(theme.cursor_select, theme.warning);
        assert_eq!(theme.directory, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(theme.selection_primary, theme.selection);
        assert_eq!(theme.fuzzy_match_secondary, theme.selection);
        assert_eq!(theme.fuzzy_match_primary, theme.selection_primary);

        let configured: Config = serde_yaml::from_str(
            "themes:\n  custom:\n    accent: '#123456'\n    cursor_select: '#654321'\n",
        )
        .unwrap();
        assert_eq!(
            configured.resolve_theme("custom").unwrap().cursor_select,
            Color::Rgb(0x65, 0x43, 0x21)
        );

        let configured: Config = serde_yaml::from_str(
            "themes:\n  custom:\n    selection: '#112233'\n    selection_primary: '#654321'\n",
        )
        .unwrap();
        assert_eq!(
            configured
                .resolve_theme("custom")
                .unwrap()
                .selection_primary,
            Color::Rgb(0x65, 0x43, 0x21)
        );

        let configured: Config = serde_yaml::from_str(
            "themes:\n  custom:\n    fuzzy_match_secondary: '#112233'\n    fuzzy_match_primary: '#654321'\n",
        )
        .unwrap();
        let theme = configured.resolve_theme("custom").unwrap();
        assert_eq!(theme.fuzzy_match_secondary, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(theme.fuzzy_match_primary, Color::Rgb(0x65, 0x43, 0x21));
    }

    #[test]
    fn trailing_whitespace_trimming_is_default_on_and_configurable() {
        assert!(Config::default().editor.trim_trailing_whitespace);
        let config: Config =
            serde_yaml::from_str("editor:\n  trim_trailing_whitespace: false\n").unwrap();
        assert!(!config.editor.trim_trailing_whitespace);
    }

    #[test]
    fn smart_newline_is_default_on_and_configurable() {
        assert!(Config::default().editor.smart_newline);
        let config: Config = serde_yaml::from_str("editor:\n  smart_newline: false\n").unwrap();
        assert!(!config.editor.smart_newline);
    }

    /// A light theme that paints light text on a light background is unusable,
    /// so the pair has to actually sit on opposite sides of the divide.
    #[test]
    fn dark_and_light_put_their_text_on_the_opposite_side_of_their_background() {
        let config = Config::default();
        let luminance = |color| match color {
            Color::Rgb(r, g, b) => {
                0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)
            }
            other => panic!("built-in themes use hex colors, got {other:?}"),
        };

        let dark = config.resolve_theme("dark").unwrap();
        assert!(luminance(dark.background) < 64.0);
        assert!(luminance(dark.foreground) > 160.0);

        let light = config.resolve_theme("light").unwrap();
        assert!(luminance(light.background) > 192.0);
        assert!(luminance(light.foreground) < 96.0);
    }

    #[test]
    fn built_in_jump_labels_are_red_and_one_neon_cyan_hue() {
        fn channels(color: Color) -> (u8, u8, u8) {
            match color {
                Color::Rgb(red, green, blue) => (red, green, blue),
                other => panic!("built-in theme color should be RGB, got {other:?}"),
            }
        }

        fn luminance(color: Color) -> f64 {
            let (red, green, blue) = channels(color);
            let channel = |value: u8| {
                let value = f64::from(value) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue)
        }

        fn contrast(left: Color, right: Color) -> f64 {
            let left = luminance(left);
            let right = luminance(right);
            (left.max(right) + 0.05) / (left.min(right) + 0.05)
        }

        let config = Config::default();
        for name in config.theme_names() {
            let theme = config.resolve_theme(name).unwrap();
            for (role, color) in [
                ("immediate", theme.jump_label_immediate),
                ("primary", theme.jump_label_primary),
                ("secondary", theme.jump_label_secondary),
            ] {
                assert!(
                    contrast(color, theme.background) >= 4.5,
                    "{name} {role} jump-label character is difficult to read"
                );
            }
            assert!(
                contrast(theme.jump_label_primary, theme.jump_label_secondary) <= 1.75,
                "{name} two-key characters differ too much in visual weight"
            );
            let (immediate_red, immediate_green, immediate_blue) =
                channels(theme.jump_label_immediate);
            assert!(
                immediate_red > immediate_green && immediate_red > immediate_blue,
                "{name} immediate jump label should be red"
            );
            let dark = luminance(theme.background) < 0.5;
            let expected = if matches!(name, "seoulbones-dark" | "zenburned-dark") {
                (Color::Rgb(0x8e, 0xea, 0xf2), Color::Rgb(0x72, 0xd7, 0xe1))
            } else if name.ends_with("bones-light") || name == "zenwritten-light" {
                (Color::Rgb(0x00, 0x4c, 0x58), Color::Rgb(0x00, 0x61, 0x6e))
            } else if dark {
                (Color::Rgb(0x5f, 0xd7, 0xe7), Color::Rgb(0x4a, 0xb7, 0xc6))
            } else {
                (Color::Rgb(0x00, 0x61, 0x6e), Color::Rgb(0x00, 0x75, 0x83))
            };
            assert_eq!(
                (theme.jump_label_primary, theme.jump_label_secondary),
                expected,
                "{name} should use the shared neon-cyan hue"
            );
            if dark {
                assert!(
                    luminance(theme.jump_label_secondary) < luminance(theme.jump_label_primary),
                    "{name} secondary jump-label character should be darker"
                );
            } else {
                assert!(
                    luminance(theme.jump_label_secondary) > luminance(theme.jump_label_primary),
                    "{name} secondary jump-label character should be lighter"
                );
            }
        }
    }

    #[test]
    fn light_and_paper_use_a_lighter_gray_only_while_jump_labels_are_active() {
        let config = Config::default();
        let light = config.resolve_theme("light").unwrap();
        assert_eq!(light.jump_text_muted, Color::Rgb(0xa8, 0xad, 0xb2));
        assert_ne!(light.jump_text_muted, light.muted);

        let paper = config.resolve_theme("paper").unwrap();
        assert_eq!(paper.jump_text_muted, Color::Rgb(0xaa, 0xaa, 0xaa));
        assert_ne!(paper.jump_text_muted, paper.muted);
    }

    #[test]
    fn an_older_theme_uses_muted_for_jump_dimming() {
        let definition = ThemeDefinition {
            muted: "#708090".into(),
            jump_text_muted: None,
            ..ThemeDefinition::default()
        };
        let theme = Theme::try_from(&definition).unwrap();
        assert_eq!(theme.jump_text_muted, Color::Rgb(0x70, 0x80, 0x90));
    }

    #[test]
    fn a_theme_can_override_the_immediate_jump_label_red() {
        let mut config = Config::default();
        config
            .themes
            .get_mut("base16")
            .unwrap()
            .jump_label_immediate = Some("#ffffff".into());
        assert_eq!(
            config.resolve_theme("base16").unwrap().jump_label_immediate,
            Color::Rgb(255, 255, 255)
        );
    }

    #[test]
    fn an_older_theme_uses_its_error_color_for_immediate_jump_labels() {
        let definition = ThemeDefinition {
            error: "#c01020".into(),
            jump_label_immediate: None,
            ..ThemeDefinition::default()
        };
        let theme = Theme::try_from(&definition).unwrap();
        assert_eq!(theme.jump_label_immediate, Color::Rgb(0xc0, 0x10, 0x20));
    }

    #[test]
    fn older_themes_derive_notification_colors_from_change_roles() {
        let definition = ThemeDefinition {
            warning: None,
            info: None,
            change_modified: Some("#d08020".into()),
            change_added: Some("#208040".into()),
            ..ThemeDefinition::default()
        };
        let theme = Theme::try_from(&definition).unwrap();
        assert_eq!(theme.warning, Color::Rgb(0xd0, 0x80, 0x20));
        assert_eq!(theme.info, Color::Rgb(0x20, 0x80, 0x40));
    }
}
