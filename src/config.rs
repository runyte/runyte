// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::command::GrammarKind;
use crate::notification::DEFAULT_HISTORY_LIMIT;

mod catppuccin;
mod core_themes;
mod everforest;
mod nightfox;
mod one_off;
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
    /// Minimum spacing and maximum staleness for automatic visible Git
    /// refreshes. Zero disables filesystem invalidation and fallback work.
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
pub const DEFAULT_THEME: &str = "default-dark";

// Diff rows are deliberately shared by appearance rather than softened into
// each palette's background. They occupy a whole row, but additions and
// removals still need to read immediately at a glance: the light pair is the
// crisp mint and pink used by Runyte's reference diff presentation, and the
// dark pair carries the same saturation on grounds that preserve light text.
const DIFF_ADDED_DARK: &str = "#174b2c";
const DIFF_REMOVED_DARK: &str = "#54252b";
const DIFF_ADDED_LIGHT: &str = "#dafbe1";
const DIFF_REMOVED_LIGHT: &str = "#ffebe9";

// Two-key jump labels use one neon-cyan hue. The second key recedes on dark
// backgrounds and advances on light backgrounds without changing hue.
const JUMP_LABEL_DARK_PRIMARY: &str = "#5fd7e7";
const JUMP_LABEL_DARK_SECONDARY: &str = "#4ab7c6";
const JUMP_LABEL_LIGHT_PRIMARY: &str = "#00616e";
// Two steps darker than `#007583`, one for each step `default-light` has
// taken toward its inactive pane: each darkening of that ground put the
// previous value back under the 4.5:1 legibility floor against it. Every
// other light theme keeps a lighter ground, so a darker secondary only reads
// better there, and it stays lighter than the primary, which is what makes
// the two characters read as one label pointing at one place.
const JUMP_LABEL_LIGHT_SECONDARY: &str = "#006673";

// Replace is deliberately louder than a palette's ordinary added-text green:
// entering an overwrite mode should be impossible to overlook. A light ground
// needs the same saturated hue at a darker value to keep the caret glyph
// legible. Magenta is the escape hatch for a theme that already gives green to
// another mode.
const CURSOR_REPLACE_DARK: Color = Color::Rgb(0x39, 0xff, 0x14);
const CURSOR_REPLACE_LIGHT: Color = Color::Rgb(0x00, 0x8f, 0x11);
const CURSOR_REPLACE_DARK_ALTERNATE: Color = Color::Rgb(0xff, 0x2b, 0xd6);
const CURSOR_REPLACE_LIGHT_ALTERNATE: Color = Color::Rgb(0xa0, 0x00, 0x8f);

/// Language server configuration.
///
/// Servers are keyed by the language names in `syntax::grammars`, which is what
/// makes a buffer's language the same question for highlighting and for LSP.
/// An unlisted language simply has no server, which is the offline default.
#[derive(Clone, Debug)]
pub struct LspConfig {
    pub enable: bool,
    pub servers: HashMap<String, LanguageServerConfig>,
}

/// On disk, language names live directly below `lsp`. The former `servers`
/// wrapper remains readable so existing configurations do not break, but it
/// is not part of the preferred shape.
#[derive(Deserialize)]
struct LspConfigDocument {
    #[serde(default)]
    enable: LspConfigField<bool>,
    #[serde(default)]
    servers: LspConfigField<HashMap<String, LanguageServerConfig>>,
    #[serde(flatten)]
    languages: HashMap<String, serde_yaml::Value>,
}

/// Distinguishes an omitted field from an explicit YAML null. `Option<T>`
/// treats both as `None`, but the typed schema rejected null before the flat
/// compatibility reader existed and must not silently enable default servers.
#[derive(Default)]
enum LspConfigField<T> {
    #[default]
    Missing,
    Value(T),
}

impl<'de, T> Deserialize<'de> for LspConfigField<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Value)
    }
}

impl<'de> Deserialize<'de> for LspConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let document = LspConfigDocument::deserialize(deserializer)?;
        let legacy_shape = matches!(&document.servers, LspConfigField::Value(_));
        let defaults = Self::default();
        let mut servers = match document.servers {
            LspConfigField::Missing => defaults.servers,
            LspConfigField::Value(servers) => servers,
        };

        for (language, value) in document.languages {
            if !crate::syntax::is_builtin_language_name(&language) {
                return Err(D::Error::custom(format!(
                    "unknown LSP language {language:?}"
                )));
            }
            let server = serde_yaml::from_value(value).map_err(D::Error::custom)?;
            if legacy_shape && servers.contains_key(&language) {
                return Err(D::Error::custom(format!(
                    "lsp.{language} is declared both directly and under lsp.servers"
                )));
            }
            servers.insert(language, server);
        }

        Ok(Self {
            enable: match document.enable {
                LspConfigField::Missing => defaults.enable,
                LspConfigField::Value(enable) => enable,
            },
            servers,
        })
    }
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
    /// Draw visible markers for spaces, tabs, and line terminators.
    pub render_whitespace: bool,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct ThemeDefinition {
    pub background: String,
    pub foreground: String,
    pub muted: String,
    /// Visible whitespace markers. An omitted value is derived one small step
    /// away from `background`, preserving custom themes written before the
    /// role existed.
    pub whitespace: Option<String>,
    /// Ordinary buffer text while `goto-word` labels are active. An omitted
    /// value uses `muted`, preserving themes written before jump dimming had
    /// its own role.
    pub jump_text_muted: Option<String>,
    pub accent: String,
    /// Command names offered in the command palette. An omitted value uses
    /// `accent`, so a theme written before the roles were split keeps one
    /// colour for the palette and for the pane and overlay borders that also
    /// read `accent`. Naming it separates the two: a palette listing what can
    /// be run is not a frame around a pane, and a theme is entitled to say so.
    pub command: Option<String>,
    /// Normal-mode caret colour. An omitted value uses `accent`.
    pub cursor_normal: Option<String>,
    /// Insert-mode caret colour. An omitted value uses `error`.
    pub cursor_insert: Option<String>,
    /// Replace-mode caret colour. An omitted value uses Runyte's neon green,
    /// or neon magenta when another mode already uses green.
    pub cursor_replace: Option<String>,
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
    /// Compatibility-only status ground. Both bundled frontends deliberately
    /// render the global status line on `background`, but this public field is
    /// retained for downstream Rust and serialized-theme compatibility.
    pub status_background: String,
    /// Compatibility-only status text colour. Both bundled frontends use
    /// `foreground` outside the mode label; see `status_background`.
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
    pub(crate) fn stepped_off(self, appearance: ThemeAppearance, step: f64) -> Self {
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
    pub whitespace: Color,
    pub jump_text_muted: Color,
    pub accent: Color,
    pub command: Color,
    pub cursor_normal: Color,
    pub cursor_insert: Color,
    pub cursor_replace: Color,
    pub cursor_select: Color,
    pub cursor_command: Color,
    pub directory: Color,
    pub selection: Color,
    pub selection_primary: Color,
    pub fuzzy_match_secondary: Color,
    pub fuzzy_match_primary: Color,
    /// Compatibility-only resolved status ground; bundled frontends do not
    /// render with this field.
    pub status_background: Color,
    /// Compatibility-only resolved status text colour; bundled frontends do
    /// not render with this field.
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
    /// Whether a colour reads as green rather than as a neutral or a nearby
    /// yellow/cyan. The spread guard keeps tiny channel differences in grays
    /// from claiming the hue.
    fn is_green_hued(color: Color) -> bool {
        let Some((red, green, blue)) = color.channels() else {
            return false;
        };
        green > red && green > blue && green.saturating_sub(red.min(blue)) >= 0x20
    }

    /// The emphatic Replace colour for a theme that did not name one.
    ///
    /// The terminal-owned fallback remains terminal-owned because the actual
    /// ground is unknowable; otherwise the appearance chooses a neon value
    /// designed for that ground. A green mode diverts Replace to magenta so
    /// the two modes cannot be mistaken for one another.
    fn default_replace_color(background: Color, other_modes: [Color; 4]) -> Color {
        let green_is_occupied = other_modes.into_iter().any(Self::is_green_hued);
        let candidate = match (background.relative_luminance(), green_is_occupied) {
            (Some(luminance), false) if luminance < 0.5 => CURSOR_REPLACE_DARK,
            (Some(_), false) => CURSOR_REPLACE_LIGHT,
            (Some(luminance), true) if luminance < 0.5 => CURSOR_REPLACE_DARK_ALTERNATE,
            (Some(_), true) => CURSOR_REPLACE_LIGHT_ALTERNATE,
            (None, false) => Color::Green,
            (None, true) => Color::Magenta,
        };
        Self::legible_mode_color(background, candidate)
    }

    /// Keeps a mode caret legible while retaining the candidate palette hue.
    fn legible_mode_color(background: Color, mut candidate: Color) -> Color {
        let Some(background_luminance) = background.relative_luminance() else {
            return candidate;
        };
        let appearance = if background_luminance < 0.5 {
            ThemeAppearance::Dark
        } else {
            ThemeAppearance::Light
        };
        for _ in 0..8 {
            let Some(candidate_luminance) = candidate.relative_luminance() else {
                return candidate;
            };
            let contrast = (background_luminance.max(candidate_luminance) + 0.05)
                / (background_luminance.min(candidate_luminance) + 0.05);
            if contrast >= 3.0 {
                break;
            }
            candidate = candidate.stepped_off(appearance, 0.15);
        }
        candidate
    }

    /// A marker colour only slightly separated from the editor ground.
    ///
    /// Whitespace is structural information rather than ordinary text, so it
    /// should remain visible without competing with syntax. A terminal-owned
    /// background cannot be derived from; those themes fall back to `muted`.
    fn derived_whitespace(background: Color, muted: Color) -> Color {
        const STEP: f64 = 0.12;

        let luminance = background.relative_luminance();
        match luminance {
            Some(value) if value < 0.5 => background.stepped_off(ThemeAppearance::Dark, STEP),
            Some(_) => background.stepped_off(ThemeAppearance::Light, STEP),
            None => muted,
        }
    }

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

fn built_in_themes() -> HashMap<String, ThemeDefinition> {
    let mut themes = HashMap::new();
    let registered = core_themes::themes()
        .chain(catppuccin::themes())
        .chain(everforest::themes())
        .chain(one_off::themes())
        .chain(nightfox::themes())
        .chain(zenbones::themes());
    for (name, theme) in registered {
        assert!(
            themes.insert(name.clone(), theme).is_none(),
            "built-in theme '{name}' is registered by more than one family"
        );
    }
    themes
}

impl Default for Config {
    fn default() -> Self {
        Self {
            editor: EditorConfig::default(),
            workspace: WorkspaceConfig::default(),
            lsp: LspConfig::default(),
            git: GitConfig::default(),
            notifications: NotificationsConfig::default(),
            theme: None,
            themes: built_in_themes(),
        }
    }
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 60,
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
            render_whitespace: false,
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
            whitespace: None,
            jump_text_muted: None,
            accent: "#7cafc2".into(),
            command: None,
            cursor_normal: None,
            cursor_insert: None,
            cursor_replace: None,
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
        let mut servers = self.lsp.servers.iter().collect::<Vec<_>>();
        servers.sort_unstable_by_key(|(language, _)| language.as_str());
        for (language, server) in servers {
            if !crate::syntax::is_builtin_language_name(language) {
                return Err(format!("unknown LSP language {language:?}"));
            }
            if server.command.as_os_str().is_empty() {
                return Err(format!("lsp.{language}.command must not be empty"));
            }
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
        let background = parse_color(&value.background)?;
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
        let whitespace = value
            .whitespace
            .as_deref()
            .map(parse_color)
            .transpose()?
            .unwrap_or_else(|| Theme::derived_whitespace(background, muted));
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
        let cursor_normal = value
            .cursor_normal
            .as_deref()
            .map(parse_color)
            .transpose()?
            .unwrap_or(accent);
        let cursor_insert = value
            .cursor_insert
            .as_deref()
            .map(parse_color)
            .transpose()?
            .unwrap_or(error);
        let cursor_select = value
            .cursor_select
            .as_deref()
            .map(parse_color)
            .transpose()?
            .unwrap_or(warning);
        let cursor_command = value
            .cursor_command
            .as_deref()
            .map(parse_color)
            .transpose()?
            .unwrap_or(info);
        let cursor_replace = match value.cursor_replace.as_deref() {
            Some(color) => Theme::legible_mode_color(background, parse_color(color)?),
            None => Theme::default_replace_color(
                background,
                [cursor_normal, cursor_insert, cursor_select, cursor_command],
            ),
        };
        Ok(Self {
            background,
            foreground: parse_color(&value.foreground)?,
            muted,
            whitespace,
            jump_text_muted: value
                .jump_text_muted
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(muted),
            accent,
            command: value
                .command
                .as_deref()
                .map(parse_color)
                .transpose()?
                .unwrap_or(accent),
            cursor_normal,
            cursor_insert,
            cursor_replace,
            cursor_select,
            cursor_command,
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
            Self(path.canonicalize().unwrap())
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
    fn git_reconciliation_defaults_to_sixty_seconds() {
        assert_eq!(Config::default().git.refresh_interval_seconds, 60);
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
    fn language_servers_can_be_declared_directly_below_lsp() {
        let config: Config =
            serde_yaml::from_str("lsp:\n  markdown:\n    command: marksman\n    args: [server]\n")
                .unwrap();

        assert_eq!(
            config.lsp.servers["markdown"].command,
            PathBuf::from("marksman")
        );
        assert_eq!(config.lsp.servers["markdown"].args, ["server"]);
        assert_eq!(
            config.lsp.servers["rust"].command,
            PathBuf::from("rust-analyzer"),
            "the flat form must retain built-in servers"
        );
    }

    #[test]
    fn legacy_and_flat_lsp_declarations_cannot_disagree() {
        let error = serde_yaml::from_str::<Config>(
            "lsp:\n  servers:\n    markdown:\n      command: old\n  markdown:\n    command: new\n",
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("lsp.markdown is declared both directly and under lsp.servers"),
            "{error}"
        );
    }

    #[test]
    fn a_language_server_needs_a_command() {
        let config: Config =
            serde_yaml::from_str("lsp:\n  markdown:\n    args: [server]\n").unwrap();

        assert_eq!(
            config.validate_settings().unwrap_err(),
            "lsp.markdown.command must not be empty"
        );
    }

    #[test]
    fn a_misspelled_flat_language_is_rejected() {
        let error = serde_yaml::from_str::<Config>(
            "lsp:\n  markdwon:\n    command: marksman\n    args: [server]\n",
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("unknown LSP language \"markdwon\""),
            "{error}"
        );
    }

    #[test]
    fn unknown_lsp_keys_are_rejected_consistently() {
        for source in [
            "lsp:\n  timeout: 5\n",
            "lsp:\n  launcher:\n    command: wrapper\n",
            "lsp:\n  markdwon:\n    executable: marksman\n",
        ] {
            let error = serde_yaml::from_str::<Config>(source)
                .unwrap_err()
                .to_string();
            assert!(error.contains("unknown LSP language"), "{error}");
        }
    }

    #[test]
    fn explicit_null_lsp_fields_are_rejected_instead_of_defaulted() {
        for source in ["lsp:\n  enable:\n", "lsp:\n  servers: null\n"] {
            assert!(serde_yaml::from_str::<Config>(source).is_err(), "{source}");
        }
    }

    #[test]
    fn a_misspelled_legacy_language_is_rejected_during_validation() {
        let config: Config =
            serde_yaml::from_str("lsp:\n  servers:\n    markdwon:\n      command: marksman\n")
                .unwrap();

        assert_eq!(
            config.validate_settings().unwrap_err(),
            "unknown LSP language \"markdwon\""
        );
    }

    #[test]
    fn documented_language_server_snippets_are_valid_configurations() {
        for (name, source) in [
            (
                "rust-analyzer",
                include_str!("../docs/lsp/rust-analyzer.yaml"),
            ),
            ("pyright", include_str!("../docs/lsp/pyright.yaml")),
            (
                "sourcekit-lsp",
                include_str!("../docs/lsp/sourcekit-lsp.yaml"),
            ),
            ("clangd", include_str!("../docs/lsp/clangd.yaml")),
            (
                "typescript-language-server",
                include_str!("../docs/lsp/typescript-language-server.yaml"),
            ),
            ("gopls", include_str!("../docs/lsp/gopls.yaml")),
            ("marksman", include_str!("../docs/lsp/marksman.yaml")),
        ] {
            let config = serde_yaml::from_str::<Config>(source)
                .unwrap_or_else(|error| panic!("{name} example does not parse: {error}"));
            config
                .validate_settings()
                .unwrap_or_else(|error| panic!("{name} example is invalid: {error}"));
        }
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
    fn compatibility_status_theme_keys_remain_resolved() {
        let config: Config = serde_yaml::from_str(
            "themes:\n  legacy:\n    status_background: '#123456'\n    status_foreground: '#abcdef'\n",
        )
        .unwrap();

        let legacy = config.resolve_theme("legacy").unwrap();
        assert_eq!(legacy.status_background, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(legacy.status_foreground, Color::Rgb(0xab, 0xcd, 0xef));
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
            "terafox-soft",
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
    fn every_theme_has_a_near_background_whitespace_color() {
        let config = Config::default();
        for name in config.theme_names() {
            let theme = config.resolve_theme(name).unwrap();
            let background = theme.background.channels().unwrap();
            let whitespace = theme.whitespace.channels().unwrap();
            assert_ne!(whitespace, background, "{name}");
            for (marker, ground) in [
                (whitespace.0, background.0),
                (whitespace.1, background.1),
                (whitespace.2, background.2),
            ] {
                assert!(marker.abs_diff(ground) <= 31, "{name}");
            }
        }

        let custom: Config = serde_yaml::from_str(
            "themes:\n  custom:\n    background: '#101010'\n    whitespace: '#232425'\n",
        )
        .unwrap();
        assert_eq!(
            custom.resolve_theme("custom").unwrap().whitespace,
            Color::Rgb(0x23, 0x24, 0x25)
        );
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
                "default-dark",
                "default-light",
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
                "nordbones-dark-soft",
                "nordfox",
                "nordfox-warm",
                "paper",
                "rosebones-dark",
                "rosebones-light",
                "seoulbones-dark",
                "seoulbones-light",
                "terafox",
                "terafox-soft",
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
    fn runyte_default_themes_share_the_brand_and_mode_palette() {
        let config = Config::default();
        let rgb = |(red, green, blue)| Color::Rgb(red, green, blue);
        let heading = crate::syntax::Scope::named("markup.heading").unwrap();

        for (name, background, foreground, accent, normal, insert, replace, select, command) in [
            (
                "default-dark",
                (0x28, 0x2a, 0x2f),
                (0xb9, 0xb9, 0xbe),
                (0xc9, 0x68, 0x70),
                (0x8d, 0xdb, 0x8c),
                (0xc9, 0x68, 0x70),
                (0xd2, 0xa8, 0xff),
                (0xf0, 0x7a, 0xb4),
                (0x6c, 0xb6, 0xff),
            ),
            (
                "default-light",
                (0xda, 0xda, 0xdc),
                (0x29, 0x2a, 0x30),
                (0xa3, 0x3d, 0x49),
                (0x23, 0x73, 0x3a),
                (0xa3, 0x3d, 0x49),
                (0x75, 0x4b, 0x97),
                (0xa4, 0x27, 0x6f),
                (0x1f, 0x65, 0xa6),
            ),
        ] {
            let theme = config.resolve_theme(name).unwrap();
            assert_eq!(theme.background, rgb(background));
            assert_eq!(theme.foreground, rgb(foreground));
            assert_eq!(theme.accent, rgb(accent));
            assert_eq!(theme.cursor_normal, rgb(normal));
            assert_eq!(theme.cursor_insert, rgb(insert));
            assert_eq!(theme.cursor_replace, rgb(replace));
            assert_eq!(theme.cursor_select, rgb(select));
            assert_eq!(theme.cursor_command, rgb(command));
            // Command mode and the palette's command names answer the same
            // colour; the accent stays behind for borders and headings.
            assert_eq!(theme.command, theme.cursor_command);
            assert_ne!(theme.command, theme.accent);
            assert_eq!(theme.directory, theme.accent);
            assert_eq!(theme.syntax_color(heading), Some(theme.accent));
        }

        // Every other bundled theme leaves the split unmade, so the palette
        // and the borders keep answering one accent.
        for name in config.theme_names() {
            if name.starts_with("default-") {
                continue;
            }
            let theme = config.resolve_theme(name).unwrap();
            assert_eq!(theme.command, theme.accent, "{name}");
        }

        assert_eq!(DEFAULT_THEME, "default-dark");
    }

    #[test]
    fn family_registrations_are_disjoint_and_cover_the_built_in_inventory() {
        let families = [
            ("core", core_themes::themes().collect::<Vec<_>>()),
            ("catppuccin", catppuccin::themes().collect()),
            ("everforest", everforest::themes().collect()),
            ("one-off", one_off::themes().collect()),
            ("nightfox", nightfox::themes().collect()),
            ("zenbones", zenbones::themes().collect()),
        ];
        let mut registered = HashMap::new();
        for (family, themes) in families {
            for (name, theme) in themes {
                assert!(
                    registered.insert(name.clone(), theme).is_none(),
                    "theme {name} is registered by more than one family (latest: {family})"
                );
            }
        }

        assert_eq!(registered, Config::default().themes);
    }

    #[test]
    fn every_built_in_theme_uses_crisp_diff_grounds_for_its_appearance() {
        let config = Config::default();
        for name in config.theme_names() {
            let theme = config.resolve_theme(name).unwrap();
            let (added, removed) = match theme.appearance().unwrap() {
                ThemeAppearance::Dark => {
                    (Color::Rgb(0x17, 0x4b, 0x2c), Color::Rgb(0x54, 0x25, 0x2b))
                }
                ThemeAppearance::Light => {
                    (Color::Rgb(0xda, 0xfb, 0xe1), Color::Rgb(0xff, 0xeb, 0xe9))
                }
            };
            assert_eq!(theme.diff_added, Some(added), "wrong {name} added ground");
            assert_eq!(
                theme.diff_removed,
                Some(removed),
                "wrong {name} removed ground"
            );
        }
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
        assert_eq!(tokyo.selection_primary, rgb(0x5c1b9e));
        assert_eq!(tokyo.status_background, rgb(0x303142));
        assert_eq!(tokyo.cursor_normal, rgb(0x7ba2f7));
        assert_eq!(tokyo.cursor_insert, rgb(0xf77890));
        assert_eq!(tokyo.cursor_select, rgb(0xe1b068));
        assert_eq!(tokyo.change_added, rgb(0x74dbcb));
        assert_eq!(tokyo.change_modified, rgb(0x7ba2f7));
        assert_eq!(tokyo.change_removed, rgb(0xf77890));
        assert_eq!(tokyo.diff_added, Some(rgb(0x174b2c)));
        assert_eq!(tokyo.diff_changed, Some(rgb(0x212c44)));
        assert_eq!(tokyo.diff_removed, Some(rgb(0x54252b)));
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
        assert_eq!(theme.diff_added, Some(Color::Rgb(0xda, 0xfb, 0xe1)));
        assert_eq!(theme.diff_removed, Some(Color::Rgb(0xff, 0xeb, 0xe9)));
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
    fn everforest_variants_use_the_upstream_palettes_and_runyte_grounds() {
        let config = Config::default();
        for (name, background, status, selection, primary) in [
            (
                "everforest-dark-hard",
                (0x27, 0x2e, 0x33),
                (0x2e, 0x38, 0x3c),
                (0x2a, 0x4f, 0x66),
                (0x5a, 0x3e, 0x22),
            ),
            (
                "everforest-dark-medium",
                (0x2d, 0x35, 0x3b),
                (0x34, 0x3f, 0x44),
                (0x30, 0x56, 0x6e),
                (0x60, 0x43, 0x2a),
            ),
            (
                "everforest-dark-soft",
                (0x33, 0x3c, 0x43),
                (0x3a, 0x46, 0x4c),
                (0x26, 0x5a, 0x70),
                (0x56, 0x3a, 0x1e),
            ),
            (
                "everforest-light-hard",
                (0xff, 0xfb, 0xef),
                (0xf8, 0xf5, 0xe4),
                (0xb4, 0xee, 0xdc),
                (0xff, 0xe7, 0xa8),
            ),
            (
                "everforest-light-medium",
                (0xfd, 0xf6, 0xe3),
                (0xf4, 0xf0, 0xd9),
                (0xb0, 0xea, 0xd8),
                (0xfd, 0xe3, 0xa4),
            ),
            (
                "everforest-light-soft",
                (0xf3, 0xea, 0xd3),
                (0xea, 0xe4, 0xca),
                (0xb7, 0xe6, 0xd5),
                (0xf9, 0xdf, 0xa6),
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
    fn bundled_themes_color_every_semantic_syntax_scope() {
        let config = Config::default();

        for name in config.themes.keys() {
            let theme = config
                .resolve_theme(name)
                .unwrap_or_else(|error| panic!("bundled theme {name} failed: {error}"));
            for scope in crate::syntax::SCOPES {
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
        assert_eq!(nordfox.diff_removed, Some(Color::Rgb(0x54, 0x25, 0x2b)));
        assert_eq!(
            nordfox.syntax_color(crate::syntax::Scope::named("function").unwrap()),
            Some(Color::Rgb(0x8c, 0xaf, 0xd2))
        );

        let terafox = config.resolve_theme("terafox").unwrap();
        assert_eq!(terafox.background, Color::Rgb(0x15, 0x25, 0x28));
        assert_eq!(terafox.foreground, Color::Rgb(0xe6, 0xea, 0xea));
        assert_eq!(terafox.selection, Color::Rgb(0x26, 0x4e, 0x59));
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
    fn nordbones_dark_soft_is_nordbones_with_softer_text() {
        fn contrast(left: Color, right: Color) -> f64 {
            let left = left.relative_luminance().unwrap();
            let right = right.relative_luminance().unwrap();
            (left.max(right) + 0.05) / (left.min(right) + 0.05)
        }

        let config = Config::default();
        let base = config.resolve_theme("nordbones-dark").unwrap();
        let soft = config.resolve_theme("nordbones-dark-soft").unwrap();

        // The point of the variant is the text and nothing else, so everything
        // that gives Nordbones its identity has to survive untouched.
        assert_eq!(soft.background, base.background);
        assert_eq!(soft.muted, base.muted);
        assert_eq!(soft.selection, base.selection);
        assert_eq!(soft.selection_primary, base.selection_primary);
        assert_eq!(soft.accent, base.accent);
        assert_eq!(soft.status_background, base.status_background);
        assert_eq!(soft.diff_added, base.diff_added);
        assert_eq!(soft.diff_removed, base.diff_removed);
        assert_eq!(soft.diff_changed, base.diff_changed);

        assert_eq!(soft.foreground, Color::Rgb(0xba, 0xc4, 0xd5));
        assert_eq!(soft.status_foreground, soft.foreground);
        // Nordbones paints identifiers in its foreground, so they move with it
        // rather than staying the brightest thing left on screen.
        for scope in ["variable", "property", "markup.heading"] {
            let scope = crate::syntax::Scope::named(scope).unwrap();
            assert_eq!(
                soft.syntax_color(scope),
                Some(soft.foreground),
                "{scope:?} did not follow the softened foreground"
            );
            assert_eq!(base.syntax_color(scope), Some(base.foreground));
        }
        // Every colour Nordbones draws code with is unchanged.
        for scope in ["keyword", "string", "type", "function", "comment", "number"] {
            let scope = crate::syntax::Scope::named(scope).unwrap();
            assert_eq!(
                soft.syntax_color(scope),
                base.syntax_color(scope),
                "{scope:?} should be Nordbones' own colour"
            );
        }

        let text = contrast(soft.foreground, soft.background);
        assert!(
            (6.8..=7.2).contains(&text),
            "softened text should sit near 7:1, not {text}"
        );
        assert!(
            text < contrast(base.foreground, base.background) / 1.4,
            "the variant has to be a visible step down from Nordbones"
        );
        // Softer text drags the readable band down with it: the dimmed text
        // has to stay below the foreground and above both grounds at once,
        // which is the constraint that fixes the foreground at 7:1.
        assert!(
            contrast(soft.foreground, soft.jump_text_muted) >= 1.4,
            "dimmed text is too close to ordinary text to read as dimmed"
        );
        for ground in [soft.selection, soft.selection_primary] {
            assert!(
                contrast(soft.jump_text_muted, ground) >= 3.0,
                "dimmed text is unreadable on a selection ground"
            );
            assert!(
                contrast(soft.foreground, ground) >= 4.5,
                "ordinary text is unreadable on a selection ground"
            );
        }
    }

    #[test]
    fn terafox_soft_is_terafox_with_softer_text() {
        fn contrast(left: Color, right: Color) -> f64 {
            let left = left.relative_luminance().unwrap();
            let right = right.relative_luminance().unwrap();
            (left.max(right) + 0.05) / (left.min(right) + 0.05)
        }

        let config = Config::default();
        let base = config.resolve_theme("terafox").unwrap();
        let soft = config.resolve_theme("terafox-soft").unwrap();

        // The point of the variant is the text and nothing else, so everything
        // that gives Terafox its identity has to survive untouched.
        assert_eq!(soft.background, base.background);
        assert_eq!(soft.muted, base.muted);
        assert_eq!(soft.jump_text_muted, base.jump_text_muted);
        assert_eq!(soft.selection, base.selection);
        assert_eq!(soft.selection_primary, base.selection_primary);
        assert_eq!(soft.accent, base.accent);
        assert_eq!(soft.status_background, base.status_background);
        assert_eq!(soft.diff_added, base.diff_added);
        assert_eq!(soft.diff_removed, base.diff_removed);
        assert_eq!(soft.diff_changed, base.diff_changed);

        assert_eq!(soft.foreground, Color::Rgb(0xbc, 0xc0, 0xc0));
        assert_eq!(soft.status_foreground, Color::Rgb(0xa6, 0xb1, 0xb0));
        // All three of Terafox's neutral text values move together, so the
        // palette's own ordering survives: identifiers stay a shade above
        // ordinary text, and the operators and punctuation between them stay a
        // shade below rather than becoming the brightest thing on the line.
        assert_eq!(
            soft.syntax_color(crate::syntax::Scope::named("variable").unwrap()),
            Some(Color::Rgb(0xc1, 0xc1, 0xc1))
        );
        for scope in ["operator", "punctuation"] {
            let scope = crate::syntax::Scope::named(scope).unwrap();
            assert_eq!(
                soft.syntax_color(scope),
                Some(soft.status_foreground),
                "{scope:?} did not follow the softened neutral text"
            );
        }
        let identifiers = soft
            .syntax_color(crate::syntax::Scope::named("variable").unwrap())
            .unwrap();
        let punctuation = soft
            .syntax_color(crate::syntax::Scope::named("punctuation").unwrap())
            .unwrap();
        assert!(
            contrast(identifiers, soft.background) > contrast(soft.foreground, soft.background),
            "identifiers should stay above ordinary text, as in Terafox"
        );
        assert!(
            contrast(punctuation, soft.background) < contrast(soft.foreground, soft.background),
            "punctuation should stay below ordinary text, as in Terafox"
        );
        // Every hued colour Terafox draws code with is unchanged, including the
        // pale cyans the softened text now sits below.
        for scope in [
            "keyword",
            "string",
            "type",
            "function",
            "comment",
            "number",
            "constructor",
            "namespace",
            "property",
            "tag",
        ] {
            let scope = crate::syntax::Scope::named(scope).unwrap();
            assert_eq!(
                soft.syntax_color(scope),
                base.syntax_color(scope),
                "{scope:?} should be Terafox' own colour"
            );
        }

        let text = contrast(soft.foreground, soft.background);
        assert!(
            (8.4..=8.8).contains(&text),
            "softened text should sit near 8.6:1, not {text}"
        );
        assert!(
            text < contrast(base.foreground, base.background) / 1.4,
            "the variant has to be a visible step down from Terafox"
        );
        // Terafox already pins its dimmed text at the boundary its selection
        // grounds allow, so the softening has to stop while ordinary text is
        // still far enough above that fixed value to read as ordinary.
        assert!(
            contrast(soft.foreground, soft.jump_text_muted) >= 1.4,
            "dimmed text is too close to ordinary text to read as dimmed"
        );
        for ground in [soft.selection, soft.selection_primary] {
            assert!(
                contrast(soft.jump_text_muted, ground) >= 3.0,
                "dimmed text is unreadable on a selection ground"
            );
            assert!(
                contrast(soft.foreground, ground) >= 4.5,
                "ordinary text is unreadable on a selection ground"
            );
        }
    }

    #[test]
    fn adjusted_dark_themes_carry_dimmed_text_on_their_selections() {
        fn contrast(left: Color, right: Color) -> f64 {
            let left = left.relative_luminance().unwrap();
            let right = right.relative_luminance().unwrap();
            (left.max(right) + 0.05) / (left.min(right) + 0.05)
        }

        // How far apart two colours look, rather than how far apart their
        // luminances are. A selection ground is a few cells wide, and one that
        // differs from the background only in hue is perfectly visible while
        // scoring almost 1:1 on contrast — so contrast is the wrong measure of
        // whether a ground can be seen, and using it once certified grounds
        // that were invisible in practice. This is CIE76 over CIELAB, which is
        // crude for near-identical colours and entirely good enough for the
        // question being asked here.
        fn perceptual_distance(left: Color, right: Color) -> f64 {
            fn lab(color: Color) -> [f64; 3] {
                let Color::Rgb(r, g, b) = color else {
                    unreachable!("built-in themes resolve to RGB")
                };
                let channel = |v: u8| {
                    let v = f64::from(v) / 255.0;
                    if v <= 0.03928 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                };
                let (r, g, b) = (channel(r), channel(g), channel(b));
                // D65, then the CIELAB transfer function.
                let f = |t: f64| {
                    if t > 0.008_856 {
                        t.cbrt()
                    } else {
                        7.787 * t + 16.0 / 116.0
                    }
                };
                let x = f((0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047);
                let y = f(0.2126 * r + 0.7152 * g + 0.0722 * b);
                let z = f((0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883);
                [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
            }
            let (left, right) = (lab(left), lab(right));
            left.iter()
                .zip(right)
                .map(|(l, r)| (l - r).powi(2))
                .sum::<f64>()
                .sqrt()
        }

        // An unfocused pane draws its cells in `jump_text_muted` while a
        // terminal review selection still fills whole cells with
        // `selection_primary`. These palettes' upstream pairs left the selected
        // row unreadable, so each was moved to the boundary `nordfox-warm`
        // holds. Pinning the values keeps a later palette refresh from quietly
        // restoring the unreadable ones.
        let config = Config::default();
        for (name, dimmed, selection, primary) in [
            ("nordbones-dark", 0x9fa6b3, 0x334e78, 0x6e3763),
            ("rosebones-dark", 0x8f8ba2, 0x523a39, 0x572d7c),
            ("seoulbones-dark", 0xa6bfa6, 0x4b2831, 0x5b5c8a),
            ("terafox", 0x8998a2, 0x264e59, 0x6a3c25),
            ("tokyobones-dark", 0x8c8ea2, 0x2c4075, 0x5c1b9e),
            ("zenburned-dark", 0xb4b4b4, 0x43617a, 0x7b5173),
            ("base16", 0xa3a3a3, 0x365864, 0x5a3b2a),
            ("everforest-dark-hard", 0x909c94, 0x2a4f66, 0x5a3e22),
            ("everforest-dark-medium", 0x99a49d, 0x30566e, 0x60432a),
            ("everforest-dark-soft", 0x9da8a0, 0x265a70, 0x563a1e),
        ] {
            let theme = config.resolve_theme(name).unwrap();
            let rgb = |value: u32| {
                Color::Rgb(
                    ((value >> 16) & 0xff) as u8,
                    ((value >> 8) & 0xff) as u8,
                    (value & 0xff) as u8,
                )
            };
            assert_eq!(
                theme.jump_text_muted,
                rgb(dimmed),
                "wrong {name} dimmed text"
            );
            assert_ne!(
                theme.jump_text_muted, theme.muted,
                "{name} left dimmed text on its comment colour"
            );
            assert_eq!(theme.selection, rgb(selection), "wrong {name} selection");
            assert_eq!(
                theme.selection_primary,
                rgb(primary),
                "wrong {name} primary selection"
            );
            for (ground, label) in [
                (theme.selection, "selection"),
                (theme.selection_primary, "primary selection"),
            ] {
                assert!(
                    contrast(theme.jump_text_muted, ground) >= 3.0,
                    "{name} dimmed text is unreadable on its {label}"
                );
                // Everforest's dark-soft ground starts from a lighter
                // background than the rest, which leaves its own text only
                // 6.65:1 to spend; its blue ground lands a hair under 4.5.
                let floor = if name == "everforest-dark-soft" {
                    4.4
                } else {
                    4.5
                };
                assert!(
                    contrast(theme.foreground, ground) >= floor,
                    "{name} ordinary text is unreadable on its {label}"
                );
                // Calibrated on what reads correctly and what does not: every
                // ground accepted so far sits at 18 or above, while the ones
                // reported as invisible measured between 6 and 14.
                assert!(
                    perceptual_distance(ground, theme.background) >= 18.0,
                    "{name} {label} disappears into its background"
                );
            }
            // The two grounds are told apart by hue, not lightness, so this
            // has to be perceptual too. Accepted pairs sit at 27 and above.
            assert!(
                perceptual_distance(theme.selection, theme.selection_primary) >= 26.0,
                "{name} cannot tell its primary selection from the rest"
            );
            assert!(
                contrast(theme.foreground, theme.jump_text_muted) >= 1.4,
                "{name} dimmed text is too close to ordinary text to read as dimmed"
            );
        }
    }

    #[test]
    fn every_built_in_command_cursor_is_legible_against_its_own_ground() {
        // The caret paints its glyph in the theme background, so the colour
        // behind it has to stand off that ground. Command is a mode colour
        // Runyte chose for every bundled theme rather than lifting from the
        // palette, which is why it is worth pinning here.
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
            for (label, other) in [
                ("NOR", theme.cursor_normal),
                ("INS", theme.cursor_insert),
                ("SEL", theme.cursor_select),
                ("CMD", theme.cursor_command),
            ] {
                assert_ne!(theme.cursor_replace, other, "{name}: REP = {label}");
            }
            let ground = theme.background.relative_luminance().unwrap();
            let replace = theme.cursor_replace.relative_luminance().unwrap();
            let replace_contrast = (ground.max(replace) + 0.05) / (ground.min(replace) + 0.05);
            assert!(
                replace_contrast >= 3.0,
                "{name} Replace cursor obscures its glyph: {replace_contrast}"
            );
            let green_is_occupied = [
                theme.cursor_normal,
                theme.cursor_insert,
                theme.cursor_select,
                theme.cursor_command,
            ]
            .into_iter()
            .any(Theme::is_green_hued);
            if green_is_occupied {
                let (red, green, blue) = theme.cursor_replace.channels().unwrap();
                assert!(
                    red > green && blue > green,
                    "{name}: REP should switch to magenta because another mode uses green"
                );
            } else {
                assert!(
                    Theme::is_green_hued(theme.cursor_replace),
                    "{name}: REP should read as neon green"
                );
            }
        }
        let light = config.resolve_theme("light").unwrap();
        assert_eq!(light.cursor_normal, Color::Rgb(0x05, 0x50, 0xae));
        assert_eq!(light.cursor_insert, Color::Rgb(0xcf, 0x22, 0x2e));
        assert_eq!(light.cursor_replace, CURSOR_REPLACE_LIGHT);
        assert_eq!(light.cursor_select, Color::Rgb(0x95, 0x38, 0x00));
        assert_eq!(light.cursor_command, Color::Rgb(0x82, 0x50, 0xdf));

        let dark = config.resolve_theme("dark").unwrap();
        assert_eq!(dark.cursor_replace, CURSOR_REPLACE_DARK);

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
    fn default_themes_use_a_pink_primary_selection_and_a_vivid_blue_secondary() {
        // `built_in_search_selection_palettes_are_legible_and_role_distinct`
        // covers the bundled themes that answer Select mode in orange, and its
        // hue rule is why the branded pair is not in that list: `default-dark`
        // and `default-light` answer it in pink instead. The same legibility
        // and role questions still have to be asked of them, so they are asked
        // here against the pink grammar.
        fn channels(color: Color) -> (u8, u8, u8) {
            match color {
                Color::Rgb(red, green, blue) => (red, green, blue),
                other => panic!("built-in theme color should be RGB, got {other:?}"),
            }
        }

        fn contrast(left: Color, right: Color) -> f64 {
            let left = left.relative_luminance().unwrap();
            let right = right.relative_luminance().unwrap();
            (left.max(right) + 0.05) / (left.min(right) + 0.05)
        }

        // CIE76 over CIELAB, for the same reason
        // `adjusted_dark_themes_carry_dimmed_text_on_their_selections` needs
        // it: two grounds told apart by hue can be plainly different and still
        // score almost 1:1 on contrast.
        fn perceptual_distance(left: Color, right: Color) -> f64 {
            fn lab(color: Color) -> [f64; 3] {
                let (red, green, blue) = channels(color);
                let channel = |value: u8| {
                    let value = f64::from(value) / 255.0;
                    if value <= 0.03928 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
                };
                let (r, g, b) = (channel(red), channel(green), channel(blue));
                let f = |t: f64| {
                    if t > 0.008_856 {
                        t.cbrt()
                    } else {
                        7.787 * t + 16.0 / 116.0
                    }
                };
                let x = f((0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047);
                let y = f(0.2126 * r + 0.7152 * g + 0.0722 * b);
                let z = f((0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883);
                [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
            }
            let (left, right) = (lab(left), lab(right));
            left.iter()
                .zip(right)
                .map(|(l, r)| (l - r).powi(2))
                .sum::<f64>()
                .sqrt()
        }

        let config = Config::default();
        for (name, secondary, primary, select) in [
            ("default-dark", 0x0b3f8c, 0x5e2e4d, 0xf07ab4),
            ("default-light", 0x8fc6fb, 0xf2b8da, 0xa4276f),
        ] {
            let theme = config.resolve_theme(name).unwrap();
            let rgb = |value: u32| {
                Color::Rgb(
                    ((value >> 16) & 0xff) as u8,
                    ((value >> 8) & 0xff) as u8,
                    (value & 0xff) as u8,
                )
            };
            assert_eq!(theme.selection, rgb(secondary), "wrong {name} selection");
            assert_eq!(
                theme.selection_primary,
                rgb(primary),
                "wrong {name} primary selection"
            );
            assert_eq!(
                theme.cursor_select,
                rgb(select),
                "wrong {name} Select cursor"
            );

            // Pink is red first, then blue, then green; a warm orange orders
            // the last two the other way round, so this is the assertion that
            // would catch a quiet return to the old palette.
            for (role, color) in [
                ("primary selection", theme.selection_primary),
                ("Select cursor", theme.cursor_select),
            ] {
                let (red, green, blue) = channels(color);
                assert!(
                    red > blue && blue > green,
                    "{name} {role} should read as pink"
                );
            }
            let (secondary_red, _, secondary_blue) = channels(theme.selection);
            assert!(
                secondary_blue > secondary_red,
                "{name} secondary selection should read as cool blue"
            );

            for (role, ground) in [
                ("secondary selection", theme.selection),
                ("primary selection", theme.selection_primary),
            ] {
                assert!(
                    contrast(theme.foreground, ground) >= 4.5,
                    "{name} ordinary text is unreadable on its {role}"
                );
                assert!(
                    perceptual_distance(ground, theme.background) >= 18.0,
                    "{name} {role} disappears into its background"
                );
            }
            // The Select badge paints its label in the theme background, so
            // the colour behind it has to stand off that ground.
            assert!(
                contrast(theme.background, theme.cursor_select) >= 3.0,
                "{name} Select cursor obscures its glyph"
            );
            assert!(
                perceptual_distance(theme.selection, theme.selection_primary) >= 26.0,
                "{name} cannot tell its primary selection from the rest"
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
    fn custom_theme_cursor_colors_use_mode_specific_fallbacks() {
        let config: Config =
            serde_yaml::from_str("themes:\n  custom:\n    accent: '#123456'\n").unwrap();
        let theme = config.resolve_theme("custom").unwrap();

        assert_eq!(theme.cursor_normal, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(theme.cursor_insert, theme.error);
        assert_eq!(theme.cursor_replace, CURSOR_REPLACE_DARK_ALTERNATE);
        assert_eq!(theme.cursor_select, theme.warning);
        assert_eq!(theme.directory, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(theme.selection_primary, theme.selection);
        assert_eq!(theme.fuzzy_match_secondary, theme.selection);
        assert_eq!(theme.fuzzy_match_primary, theme.selection_primary);

        let configured: Config =
            serde_yaml::from_str("themes:\n  custom:\n    cursor_command: '#ba8baf'\n").unwrap();
        assert_eq!(
            configured.resolve_theme("custom").unwrap().cursor_replace,
            CURSOR_REPLACE_DARK
        );

        let configured: Config = serde_yaml::from_str(
            "themes:\n  custom:\n    cursor_command: '#ba8baf'\n    cursor_replace: '#12ef34'\n",
        )
        .unwrap();
        assert_eq!(
            configured.resolve_theme("custom").unwrap().cursor_replace,
            Color::Rgb(0x12, 0xef, 0x34)
        );

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
    fn whitespace_rendering_is_default_off_and_configurable() {
        assert!(!Config::default().editor.render_whitespace);
        let config: Config = serde_yaml::from_str("editor:\n  render_whitespace: true\n").unwrap();
        assert!(config.editor.render_whitespace);
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
                (Color::Rgb(0x00, 0x61, 0x6e), Color::Rgb(0x00, 0x66, 0x73))
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
