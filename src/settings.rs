// SPDX-License-Identifier: MPL-2.0

//! Typed, presentation-neutral editor settings and conservative YAML writes.
//!
//! This module deliberately does not round-trip YAML through `serde_yaml`.
//! The configuration file is user-authored text, so changing one supported
//! scalar must leave comments, unknown fields, ordering, quoting elsewhere,
//! and line endings alone. Documents using YAML features that cannot be
//! patched with that guarantee are rejected before any file is written.

use std::{
    collections::HashSet,
    error::Error,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    command::GrammarKind,
    config::{
        Config, DEFAULT_THEME, MAX_GIT_REFRESH_INTERVAL_SECONDS, MAX_IDLE_RETIREMENT_MINUTES,
        WorkspaceMode,
    },
};

use unicode_width::UnicodeWidthChar;

pub const SETTINGS_BUFFER_NAME: &str = "[config]";
pub const SETTINGS_PAGE_WIDTH: usize = 80;
const SETTING_COLUMN_WIDTH: usize = 32;
const DESCRIPTION_COLUMN_WIDTH: usize = 34;
const VALUE_COLUMN_WIDTH: usize = 10;
const COLUMN_GAP: &str = "  ";

/// A stable identity shared by configuration discovery and persistence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SettingId {
    EditorGrammar,
    EditorLineNumbers,
    EditorTabWidth,
    EditorSmartNewline,
    EditorScrollOffset,
    EditorMotionRepeatMultiplier,
    EditorShowHiddenFiles,
    EditorSoftWrap,
    EditorZenWidth,
    EditorHardWrapWidth,
    EditorTrimTrailingWhitespace,
    EditorMouse,
    EditorWordCompletion,
    EditorWordCompletionMinimum,
    EditorFastPaneKeys,
    EditorCommandModeDim,
    WorkspaceMode,
    WorkspaceIdleRetirementMinutes,
    Theme,
    LspEnable,
    GitRefreshIntervalSeconds,
    NotificationsHistoryLimit,
}

/// A setting value after parsing, independent of a menu or frontend widget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingValue {
    Grammar(GrammarKind),
    Boolean(bool),
    Integer(usize),
    WorkspaceMode(WorkspaceMode),
    Text(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingType {
    Grammar,
    Boolean,
    Integer {
        minimum: usize,
        maximum: usize,
    },
    Theme,
    WorkspaceMode,
    /// An unrestricted string entered directly rather than chosen from a list.
    Text,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewPolicy {
    Immediate,
    RestartRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistencePolicy {
    ConfigFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingDescriptor {
    pub id: SettingId,
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub value_type: SettingType,
    pub preview: PreviewPolicy,
    pub persistence: PersistencePolicy,
}

const DESCRIPTORS: &[SettingDescriptor] = &[
    SettingDescriptor {
        id: SettingId::EditorGrammar,
        key: "editor.grammar",
        title: "Editing grammar",
        description: "Input grammar used for editing commands",
        value_type: SettingType::Grammar,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorLineNumbers,
        key: "editor.line_numbers",
        title: "Line numbers",
        description: "Show line numbers beside editable buffers",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorTabWidth,
        key: "editor.tab_width",
        title: "Tab width",
        description: "Display and indentation width of a tab",
        value_type: SettingType::Integer {
            minimum: 1,
            maximum: 16,
        },
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorSmartNewline,
        key: "editor.smart_newline",
        title: "Smart newline",
        description: "Add syntax indentation and align list continuations",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorScrollOffset,
        key: "editor.scroll_offset",
        title: "Scroll offset",
        description: "Rows kept visible above and below the cursor",
        value_type: SettingType::Integer {
            minimum: 0,
            maximum: 100,
        },
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorMotionRepeatMultiplier,
        key: "editor.motion_repeat_multiplier",
        title: "Held-motion speed",
        description: "Cursor motions dispatched for each held-key repeat",
        value_type: SettingType::Integer {
            minimum: 1,
            maximum: 10,
        },
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorShowHiddenFiles,
        key: "editor.show_hidden_files",
        title: "Hidden files",
        description: "Show dotfiles in the explorer, file picker, and search",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorSoftWrap,
        key: "editor.soft_wrap",
        title: "Soft wrap",
        description: "Wrap long visual lines without changing the buffer",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorZenWidth,
        key: "editor.zen_width",
        title: "Zen width",
        description: "Maximum text width of the centered zen viewport",
        value_type: SettingType::Integer {
            minimum: 1,
            maximum: 1000,
        },
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorHardWrapWidth,
        key: "editor.hard_wrap_width",
        title: "Hard wrap width",
        description: "Default character width for hard-wrap and reflow",
        value_type: SettingType::Integer {
            minimum: 1,
            maximum: 1000,
        },
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorTrimTrailingWhitespace,
        key: "editor.trim_trailing_whitespace",
        title: "Trim trailing whitespace",
        description: "Remove spaces and tabs at line ends when saving text files",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorMouse,
        key: "editor.mouse",
        title: "Mouse capture",
        description: "Enable terminal mouse selection, scrolling, and pane resizing",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::RestartRequired,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorWordCompletion,
        key: "editor.word_completion",
        title: "Word completion",
        description: "Suggest words already open elsewhere in the workspace",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorWordCompletionMinimum,
        key: "editor.word_completion_minimum",
        title: "Word completion trigger length",
        description: "Prefix length before word candidates appear",
        value_type: SettingType::Integer {
            minimum: 1,
            maximum: 32,
        },
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorFastPaneKeys,
        key: "editor.fast_pane_keys",
        title: "Fast pane keys",
        description: "Move between panes with Ctrl-h/j/k/l, without Ctrl-w",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::EditorCommandModeDim,
        key: "editor.command_mode_dim",
        title: "Command mode dim",
        description: "Gray out pane text while a command prompt is open",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::WorkspaceMode,
        key: "workspace.mode",
        title: "Workspace mode",
        description: "Default bare launches to standalone or persistent mode",
        value_type: SettingType::WorkspaceMode,
        preview: PreviewPolicy::RestartRequired,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::WorkspaceIdleRetirementMinutes,
        key: "workspace.idle_retirement_minutes",
        title: "Workspace idle retirement",
        description: "Minutes a clean unattached host lives; zero keeps it",
        value_type: SettingType::Integer {
            minimum: 0,
            maximum: MAX_IDLE_RETIREMENT_MINUTES,
        },
        // The host reads this each time it considers retiring, so a change
        // takes effect without restarting the very host it governs.
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::Theme,
        key: "theme",
        title: "Theme",
        description: "Named color theme used by the editor",
        value_type: SettingType::Theme,
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::LspEnable,
        key: "lsp.enable",
        title: "Language servers",
        description: "Start configured language servers for semantic features",
        value_type: SettingType::Boolean,
        preview: PreviewPolicy::RestartRequired,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::GitRefreshIntervalSeconds,
        key: "git.refresh_interval_seconds",
        title: "Git refresh interval",
        description: "Seconds between visible Git refreshes; zero disables",
        value_type: SettingType::Integer {
            minimum: 0,
            maximum: MAX_GIT_REFRESH_INTERVAL_SECONDS,
        },
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
    SettingDescriptor {
        id: SettingId::NotificationsHistoryLimit,
        key: "notifications.history_limit",
        title: "Notification history",
        description: "Newest workspace notifications retained in memory",
        value_type: SettingType::Integer {
            minimum: crate::notification::MIN_HISTORY_LIMIT,
            maximum: crate::notification::MAX_HISTORY_LIMIT,
        },
        preview: PreviewPolicy::Immediate,
        persistence: PersistencePolicy::ConfigFile,
    },
];

impl SettingId {
    pub const ALL: &'static [Self] = &[
        Self::EditorGrammar,
        Self::EditorLineNumbers,
        Self::EditorTabWidth,
        Self::EditorSmartNewline,
        Self::EditorScrollOffset,
        Self::EditorMotionRepeatMultiplier,
        Self::EditorShowHiddenFiles,
        Self::EditorSoftWrap,
        Self::EditorZenWidth,
        Self::EditorHardWrapWidth,
        Self::EditorTrimTrailingWhitespace,
        Self::EditorMouse,
        Self::EditorWordCompletion,
        Self::EditorWordCompletionMinimum,
        Self::EditorFastPaneKeys,
        Self::EditorCommandModeDim,
        Self::WorkspaceMode,
        Self::WorkspaceIdleRetirementMinutes,
        Self::Theme,
        Self::LspEnable,
        Self::GitRefreshIntervalSeconds,
        Self::NotificationsHistoryLimit,
    ];

    pub fn descriptor(self) -> &'static SettingDescriptor {
        DESCRIPTORS
            .iter()
            .find(|descriptor| descriptor.id == self)
            .expect("every setting identity has one descriptor")
    }

    /// The value written in the typed configuration model.
    ///
    /// This is intentionally not named `current_value`: runtime grammar and
    /// theme selection are session state and can differ from persisted YAML.
    /// A settings frontend must obtain those effective values from `App`.
    pub fn configured_value(self, config: &Config) -> SettingValue {
        match self {
            Self::EditorGrammar => SettingValue::Grammar(config.editor.grammar),
            Self::EditorLineNumbers => SettingValue::Boolean(config.editor.line_numbers),
            Self::EditorTabWidth => SettingValue::Integer(config.editor.tab_width),
            Self::EditorSmartNewline => SettingValue::Boolean(config.editor.smart_newline),
            Self::EditorScrollOffset => SettingValue::Integer(config.editor.scroll_offset),
            Self::EditorMotionRepeatMultiplier => {
                SettingValue::Integer(config.editor.motion_repeat_multiplier)
            }
            Self::EditorShowHiddenFiles => SettingValue::Boolean(config.editor.show_hidden_files),
            Self::EditorSoftWrap => SettingValue::Boolean(config.editor.soft_wrap),
            Self::EditorZenWidth => SettingValue::Integer(config.editor.zen_width),
            Self::EditorHardWrapWidth => SettingValue::Integer(config.editor.hard_wrap_width),
            Self::EditorTrimTrailingWhitespace => {
                SettingValue::Boolean(config.editor.trim_trailing_whitespace)
            }
            Self::EditorMouse => SettingValue::Boolean(config.editor.mouse),
            Self::EditorWordCompletion => SettingValue::Boolean(config.editor.word_completion),
            Self::EditorWordCompletionMinimum => {
                SettingValue::Integer(config.editor.word_completion_minimum)
            }
            Self::EditorFastPaneKeys => SettingValue::Boolean(config.editor.fast_pane_keys),
            Self::EditorCommandModeDim => SettingValue::Boolean(config.editor.command_mode_dim),
            Self::Theme => SettingValue::Text(
                config
                    .theme
                    .clone()
                    .unwrap_or_else(|| DEFAULT_THEME.to_owned()),
            ),
            Self::LspEnable => SettingValue::Boolean(config.lsp.enable),
            Self::GitRefreshIntervalSeconds => {
                SettingValue::Integer(config.git.refresh_interval_seconds)
            }
            Self::NotificationsHistoryLimit => {
                SettingValue::Integer(config.notifications.history_limit)
            }
            Self::WorkspaceMode => SettingValue::WorkspaceMode(config.workspace.mode),
            Self::WorkspaceIdleRetirementMinutes => {
                SettingValue::Integer(config.workspace.idle_retirement_minutes)
            }
        }
    }

    /// Values a discovery surface can present without duplicating policy.
    pub fn allowed_values(self, config: &Config) -> Vec<String> {
        match self.descriptor().value_type {
            SettingType::Grammar => GrammarKind::ALL
                .iter()
                .map(|value| value.name().to_owned())
                .collect(),
            SettingType::Boolean => vec!["true".to_owned(), "false".to_owned()],
            SettingType::Theme => config
                .theme_names()
                .into_iter()
                .filter(|name| config.resolve_theme(name).is_ok())
                .map(str::to_owned)
                .collect(),
            SettingType::WorkspaceMode => {
                WorkspaceMode::ALL.iter().map(ToString::to_string).collect()
            }
            SettingType::Integer { .. } | SettingType::Text => Vec::new(),
        }
    }

    pub fn validate(self, value: &SettingValue, config: &Config) -> Result<(), SettingError> {
        let invalid_type = || SettingError::InvalidValue {
            setting: self,
            message: format!("expected {}", self.descriptor().value_type),
        };
        match (self.descriptor().value_type, value) {
            (SettingType::Grammar, SettingValue::Grammar(_))
            | (SettingType::Boolean, SettingValue::Boolean(_))
            | (SettingType::WorkspaceMode, SettingValue::WorkspaceMode(_)) => Ok(()),
            (SettingType::Integer { minimum, maximum }, SettingValue::Integer(value)) => {
                if (minimum..=maximum).contains(value) {
                    Ok(())
                } else {
                    Err(SettingError::InvalidValue {
                        setting: self,
                        message: format!("expected an integer from {minimum} through {maximum}"),
                    })
                }
            }
            (SettingType::Theme, SettingValue::Text(value)) => config
                .resolve_theme(value)
                .map(|_| ())
                .map_err(|error| SettingError::InvalidValue {
                    setting: self,
                    message: format!("unusable theme '{value}': {error}"),
                }),
            (SettingType::Text, SettingValue::Text(_)) => Ok(()),
            _ => Err(invalid_type()),
        }
    }

    /// Validate and apply a value to an in-memory configuration.
    ///
    /// Frontends can use this for preview and restore a cloned `Config` when
    /// the preview is cancelled; persistence remains a separate explicit act.
    pub fn apply(self, value: &SettingValue, config: &mut Config) -> Result<(), SettingError> {
        self.validate(value, config)?;
        match (self, value) {
            (Self::EditorGrammar, SettingValue::Grammar(value)) => config.editor.grammar = *value,
            (Self::EditorLineNumbers, SettingValue::Boolean(value)) => {
                config.editor.line_numbers = *value;
            }
            (Self::EditorTabWidth, SettingValue::Integer(value)) => {
                config.editor.tab_width = *value;
            }
            (Self::EditorSmartNewline, SettingValue::Boolean(value)) => {
                config.editor.smart_newline = *value;
            }
            (Self::EditorScrollOffset, SettingValue::Integer(value)) => {
                config.editor.scroll_offset = *value;
            }
            (Self::EditorMotionRepeatMultiplier, SettingValue::Integer(value)) => {
                config.editor.motion_repeat_multiplier = *value;
            }
            (Self::EditorShowHiddenFiles, SettingValue::Boolean(value)) => {
                config.editor.show_hidden_files = *value;
            }
            (Self::EditorSoftWrap, SettingValue::Boolean(value)) => {
                config.editor.soft_wrap = *value;
            }
            (Self::EditorZenWidth, SettingValue::Integer(value)) => {
                config.editor.zen_width = *value;
            }
            (Self::EditorHardWrapWidth, SettingValue::Integer(value)) => {
                config.editor.hard_wrap_width = *value;
            }
            (Self::EditorTrimTrailingWhitespace, SettingValue::Boolean(value)) => {
                config.editor.trim_trailing_whitespace = *value;
            }
            (Self::EditorMouse, SettingValue::Boolean(value)) => config.editor.mouse = *value,
            (Self::EditorWordCompletion, SettingValue::Boolean(value)) => {
                config.editor.word_completion = *value;
            }
            (Self::EditorWordCompletionMinimum, SettingValue::Integer(value)) => {
                config.editor.word_completion_minimum = *value;
            }
            (Self::EditorFastPaneKeys, SettingValue::Boolean(value)) => {
                config.editor.fast_pane_keys = *value;
            }
            (Self::EditorCommandModeDim, SettingValue::Boolean(value)) => {
                config.editor.command_mode_dim = *value;
            }
            (Self::Theme, SettingValue::Text(value)) => config.theme = Some(value.clone()),
            (Self::LspEnable, SettingValue::Boolean(value)) => config.lsp.enable = *value,
            (Self::GitRefreshIntervalSeconds, SettingValue::Integer(value)) => {
                config.git.refresh_interval_seconds = *value;
            }
            (Self::NotificationsHistoryLimit, SettingValue::Integer(value)) => {
                config.notifications.history_limit = *value;
            }
            (Self::WorkspaceMode, SettingValue::WorkspaceMode(value)) => {
                config.workspace.mode = *value;
            }
            (Self::WorkspaceIdleRetirementMinutes, SettingValue::Integer(value)) => {
                config.workspace.idle_retirement_minutes = *value;
            }
            _ => unreachable!("validation accepted only the setting's value type"),
        }
        Ok(())
    }
}

impl fmt::Display for SettingType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grammar => formatter.write_str("an editing grammar"),
            Self::Boolean => formatter.write_str("a boolean"),
            Self::Integer { minimum, maximum } => {
                write!(formatter, "an integer from {minimum} through {maximum}")
            }
            Self::Theme => formatter.write_str("a theme name"),
            Self::WorkspaceMode => formatter.write_str("a workspace mode"),
            Self::Text => formatter.write_str("text"),
        }
    }
}

impl fmt::Display for SettingValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grammar(value) => value.fmt(formatter),
            Self::Boolean(value) => value.fmt(formatter),
            Self::Integer(value) => value.fmt(formatter),
            Self::WorkspaceMode(value) => value.fmt(formatter),
            Self::Text(value) => formatter.write_str(value),
        }
    }
}

/// The stable registry consumed by future configuration frontends.
pub struct SettingRegistry;

impl SettingRegistry {
    pub const fn descriptors() -> &'static [SettingDescriptor] {
        DESCRIPTORS
    }

    pub fn find(key: &str) -> Option<&'static SettingDescriptor> {
        DESCRIPTORS.iter().find(|descriptor| descriptor.key == key)
    }
}

/// The text and stable per-row setting identities of the read-only config page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsPage {
    pub text: String,
    pub rows: Vec<Option<SettingId>>,
}

/// Renders the setting registry as a fixed-width, frontend-neutral document.
///
/// Physical continuation lines keep the same identity as their first line, so
/// a frontend can activate a wrapped setting without deriving identity from
/// presentation columns.
pub fn render_settings_page(values: &[(SettingId, String)]) -> SettingsPage {
    let mut text = String::new();
    let mut rows = Vec::new();
    push_settings_row(
        &mut text,
        &mut rows,
        None,
        "Setting",
        "Description",
        "Value",
    );
    text.push_str(&"─".repeat(SETTINGS_PAGE_WIDTH));
    text.push('\n');
    rows.push(None);

    for (setting, value) in values {
        let descriptor = setting.descriptor();
        let names = wrap_cells(descriptor.key, SETTING_COLUMN_WIDTH);
        let descriptions = wrap_cells(descriptor.description, DESCRIPTION_COLUMN_WIDTH);
        let values = wrap_cells(value, VALUE_COLUMN_WIDTH);
        let height = names.len().max(descriptions.len()).max(values.len());
        for row in 0..height {
            push_settings_row(
                &mut text,
                &mut rows,
                Some(*setting),
                names.get(row).map_or("", String::as_str),
                descriptions.get(row).map_or("", String::as_str),
                values.get(row).map_or("", String::as_str),
            );
        }
    }
    SettingsPage { text, rows }
}

fn push_settings_row(
    text: &mut String,
    rows: &mut Vec<Option<SettingId>>,
    setting: Option<SettingId>,
    name: &str,
    description: &str,
    value: &str,
) {
    text.push_str(&pad_cells(name, SETTING_COLUMN_WIDTH));
    text.push_str(COLUMN_GAP);
    text.push_str(&pad_cells(description, DESCRIPTION_COLUMN_WIDTH));
    text.push_str(COLUMN_GAP);
    text.push_str(&pad_cells(value, VALUE_COLUMN_WIDTH));
    text.push('\n');
    rows.push(setting);
}

fn pad_cells(value: &str, width: usize) -> String {
    let cells = value
        .chars()
        .map(|character| character.width().unwrap_or(0))
        .sum::<usize>();
    format!("{value}{}", " ".repeat(width.saturating_sub(cells)))
}

fn wrap_cells(value: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for word in value.split_whitespace() {
        let word_width = word
            .chars()
            .map(|character| character.width().unwrap_or(0))
            .sum::<usize>();
        if word_width <= width {
            let gap = usize::from(!line.is_empty());
            if line_width + gap + word_width > width {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            if !line.is_empty() {
                line.push(' ');
                line_width += 1;
            }
            line.push_str(word);
            line_width += word_width;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        for character in word.chars() {
            let cells = character.width().unwrap_or(0);
            if line_width + cells > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            line.push(character);
            line_width += cells;
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

#[derive(Debug)]
pub enum SettingError {
    InvalidValue {
        setting: SettingId,
        message: String,
    },
    UnsafeYaml {
        line: usize,
        reason: &'static str,
    },
    InvalidYaml(String),
    InvalidConfig(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for SettingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue { setting, message } => {
                write!(
                    formatter,
                    "invalid value for {}: {message}",
                    setting.descriptor().key
                )
            }
            Self::UnsafeYaml { line, reason } => write!(
                formatter,
                "cannot safely update configuration at line {line}: {reason}"
            ),
            Self::InvalidYaml(message) => write!(formatter, "invalid YAML: {message}"),
            Self::InvalidConfig(message) => write!(formatter, "invalid Runyte config: {message}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for SettingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Patch one supported scalar and atomically replace the resolved config file.
///
/// If `path` is a symlink, its target is replaced and the link remains intact.
/// The returned configuration has the same built-in map merging as
/// [`Config::load`].
pub fn persist_setting(
    path: &Path,
    setting: SettingId,
    value: &SettingValue,
) -> Result<Config, SettingError> {
    let target = resolve_write_target(path)?;
    let source = match fs::read_to_string(&target) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(io_error("read", &target, source)),
    };
    // Safety-shape validation deliberately precedes deserialization so an
    // anchor or duplicate is reported as the unsupported construct it is.
    scan_document(&source)?;
    let current = parse_config(&source)?;
    setting.validate(value, &current)?;
    let patched = patch_scalar(&source, setting, value)?;
    let updated = parse_config(&patched)?;
    setting.validate(value, &updated)?;
    if setting.configured_value(&updated) != *value {
        return Err(SettingError::InvalidConfig(format!(
            "patched {} did not resolve to the requested value",
            setting.descriptor().key
        )));
    }
    atomic_write(&target, patched.as_bytes())?;
    Ok(updated)
}

fn parse_config(source: &str) -> Result<Config, SettingError> {
    if !source.is_empty() {
        serde_yaml::from_str::<serde_yaml::Value>(source)
            .map_err(|error| SettingError::InvalidYaml(error.to_string()))?;
    }
    let config = if source.is_empty() {
        Config::default()
    } else {
        serde_yaml::from_str::<Config>(source)
            .map_err(|error| SettingError::InvalidConfig(error.to_string()))?
    };
    let config = config.with_builtin_defaults();
    config
        .validate_settings()
        .map_err(SettingError::InvalidConfig)?;
    Ok(config)
}

fn patch_scalar(
    source: &str,
    setting: SettingId,
    value: &SettingValue,
) -> Result<String, SettingError> {
    let lines = scan_document(source)?;
    let scalar = yaml_scalar(value);
    let path = setting.descriptor().key.split('.').collect::<Vec<_>>();
    match path.as_slice() {
        [key] => patch_root_scalar(source, &lines, key, &scalar),
        [parent, key] => patch_child_scalar(source, &lines, parent, key, &scalar),
        _ => unreachable!("setting registry paths have at most two components"),
    }
}

#[derive(Clone, Debug)]
struct Line<'a> {
    number: usize,
    end: usize,
    indent: usize,
    text: &'a str,
    entry: Option<Entry<'a>>,
}

#[derive(Clone, Debug)]
struct Entry<'a> {
    key: &'a str,
    value_start: usize,
    value_end: usize,
}

fn scan_document(source: &str) -> Result<Vec<Line<'_>>, SettingError> {
    let mut lines = Vec::new();
    let mut offset = 0;
    let mut scopes = vec![(0usize, HashSet::<String>::new())];
    for (index, chunk) in source.split_inclusive('\n').enumerate() {
        let end = offset + chunk.len();
        let content_end = end - usize::from(chunk.ends_with('\n'));
        let content_end = content_end - usize::from(source[..content_end].ends_with('\r'));
        let text = &source[offset..content_end];
        let indent = text
            .bytes()
            .take_while(|byte| *byte == b' ' || *byte == b'\t')
            .count();
        if text[..indent].contains('\t') {
            return Err(SettingError::UnsafeYaml {
                line: index + 1,
                reason: "tab indentation is not losslessly patchable; use spaces",
            });
        }
        reject_unsafe_tokens(text, index + 1)?;
        let entry = parse_entry(text, offset, indent);
        if let Some(entry) = &entry {
            while scopes.len() > 1 && scopes.last().is_some_and(|(level, _)| *level > indent) {
                scopes.pop();
            }
            if scopes.last().is_none_or(|(level, _)| *level < indent) {
                scopes.push((indent, HashSet::new()));
            }
            let seen = &mut scopes.last_mut().expect("root scope exists").1;
            if !seen.insert(entry.key.to_owned()) {
                return Err(SettingError::UnsafeYaml {
                    line: index + 1,
                    reason: "duplicate mapping keys are ambiguous",
                });
            }
        } else if text[indent..].starts_with('-') {
            while scopes.len() > 1 && scopes.last().is_some_and(|(level, _)| *level > indent) {
                scopes.pop();
            }
        }
        lines.push(Line {
            number: index + 1,
            end,
            indent,
            text,
            entry,
        });
        offset = end;
    }
    if source.is_empty() {
        return Ok(lines);
    }
    // `split_inclusive` covers a final unterminated line, so `offset` is exact.
    debug_assert_eq!(offset, source.len());
    Ok(lines)
}

fn reject_unsafe_tokens(line: &str, number: usize) -> Result<(), SettingError> {
    let mut single = false;
    let mut double = false;
    let mut chars = line.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        if single {
            if character == '\'' {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    chars.next();
                } else {
                    single = false;
                }
            }
            continue;
        }
        if double {
            if character == '\\' {
                chars.next();
            } else if character == '"' {
                double = false;
            }
            continue;
        }
        match character {
            '\'' => single = true,
            '"' => double = true,
            '#' if index == 0 || line[..index].ends_with(char::is_whitespace) => break,
            '{' | '}' => {
                return Err(SettingError::UnsafeYaml {
                    line: number,
                    reason: "flow mappings cannot be updated without normalizing the file",
                });
            }
            '&' | '*' => {
                return Err(SettingError::UnsafeYaml {
                    line: number,
                    reason: "anchors and aliases cannot be updated safely",
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_entry<'a>(text: &'a str, line_start: usize, indent: usize) -> Option<Entry<'a>> {
    let content = &text[indent..];
    if content.is_empty() || content.starts_with('#') || content.starts_with('-') {
        return None;
    }
    let colon = content.find(':')?;
    let key = &content[..colon];
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        || content
            .as_bytes()
            .get(colon + 1)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }
    let after_colon = indent + colon + 1;
    let relative_start = after_colon
        + text[after_colon..]
            .bytes()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
    let value_end = scalar_end(text, relative_start);
    Some(Entry {
        key,
        value_start: line_start + relative_start,
        value_end: line_start + value_end,
    })
}

fn scalar_end(text: &str, start: usize) -> usize {
    let mut single = false;
    let mut double = false;
    let mut comment = text.len();
    let mut chars = text[start..].char_indices().peekable();
    while let Some((relative, character)) = chars.next() {
        let index = start + relative;
        if single {
            if character == '\'' {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    chars.next();
                } else {
                    single = false;
                }
            }
        } else if double {
            if character == '\\' {
                chars.next();
            } else if character == '"' {
                double = false;
            }
        } else {
            match character {
                '\'' => single = true,
                '"' => double = true,
                '#' if index == start || text[..index].ends_with(char::is_whitespace) => {
                    comment = index;
                    break;
                }
                _ => {}
            }
        }
    }
    text[..comment].trim_end().len().max(start)
}

fn patch_root_scalar(
    source: &str,
    lines: &[Line<'_>],
    key: &str,
    scalar: &str,
) -> Result<String, SettingError> {
    if let Some(line) = direct_entry(lines, 0, key) {
        return replace_scalar(source, line, scalar);
    }
    Ok(append_block(source, &format!("{key}: {scalar}")))
}

fn patch_child_scalar(
    source: &str,
    lines: &[Line<'_>],
    parent: &str,
    key: &str,
    scalar: &str,
) -> Result<String, SettingError> {
    let Some(parent_index) = lines.iter().position(|line| {
        line.indent == 0 && line.entry.as_ref().is_some_and(|entry| entry.key == parent)
    }) else {
        let newline = newline(source);
        return Ok(append_block(
            source,
            &format!("{parent}:{newline}  {key}: {scalar}"),
        ));
    };
    let parent_line = &lines[parent_index];
    ensure_mapping(parent_line)?;
    let block_end = lines[parent_index + 1..]
        .iter()
        .position(|line| is_content(line) && line.indent <= parent_line.indent)
        .map_or(lines.len(), |offset| parent_index + 1 + offset);
    let child_indent = lines[parent_index + 1..block_end]
        .iter()
        .filter(|line| line.entry.is_some())
        .map(|line| line.indent)
        .min()
        .unwrap_or(parent_line.indent + 2);
    if let Some(line) = lines[parent_index + 1..block_end].iter().find(|line| {
        line.indent == child_indent && line.entry.as_ref().is_some_and(|entry| entry.key == key)
    }) {
        return replace_scalar(source, line, scalar);
    }
    // Insert immediately after the parent's last semantic child. Root-level
    // comments before the next mapping therefore remain attached to that
    // following section rather than appearing inside the setting block.
    let insertion = lines[parent_index + 1..block_end]
        .iter()
        .rfind(|line| is_content(line))
        .map_or(parent_line.end, |line| line.end);
    let prefix = if insertion > 0 && !source[..insertion].ends_with('\n') {
        newline(source)
    } else {
        ""
    };
    let addition = format!(
        "{prefix}{}{key}: {scalar}{}",
        " ".repeat(child_indent),
        newline(source)
    );
    Ok(insert_at(source, insertion, &addition))
}

fn direct_entry<'a>(lines: &'a [Line<'a>], indent: usize, key: &str) -> Option<&'a Line<'a>> {
    lines.iter().find(|line| {
        line.indent == indent && line.entry.as_ref().is_some_and(|entry| entry.key == key)
    })
}

fn is_content(line: &Line<'_>) -> bool {
    let trimmed = line.text.trim();
    !trimmed.is_empty() && !trimmed.starts_with('#')
}

fn ensure_mapping(line: &Line<'_>) -> Result<(), SettingError> {
    let entry = line.entry.as_ref().expect("caller found a mapping entry");
    if entry.value_start == entry.value_end {
        Ok(())
    } else {
        Err(SettingError::UnsafeYaml {
            line: line.number,
            reason: "the setting parent must be an ordinary block mapping",
        })
    }
}

fn replace_scalar(source: &str, line: &Line<'_>, scalar: &str) -> Result<String, SettingError> {
    let entry = line.entry.as_ref().expect("caller found a mapping entry");
    let existing = &source[entry.value_start..entry.value_end];
    if existing.starts_with('|') || existing.starts_with('>') || existing.starts_with('[') {
        return Err(SettingError::UnsafeYaml {
            line: line.number,
            reason: "the setting value is not an ordinary scalar",
        });
    }
    let mut patched = String::with_capacity(source.len() + scalar.len());
    patched.push_str(&source[..entry.value_start]);
    patched.push_str(scalar);
    if entry.value_start == entry.value_end && source[entry.value_end..].starts_with('#') {
        patched.push(' ');
    }
    patched.push_str(&source[entry.value_end..]);
    Ok(patched)
}

fn append_block(source: &str, block: &str) -> String {
    let separator = if source.is_empty() || source.ends_with('\n') {
        ""
    } else {
        newline(source)
    };
    format!("{source}{separator}{block}{}", newline(source))
}

fn insert_at(source: &str, offset: usize, addition: &str) -> String {
    let mut patched = String::with_capacity(source.len() + addition.len());
    patched.push_str(&source[..offset]);
    patched.push_str(addition);
    patched.push_str(&source[offset..]);
    patched
}

fn newline(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn yaml_scalar(value: &SettingValue) -> String {
    match value {
        SettingValue::Grammar(value) => value.to_string(),
        SettingValue::Boolean(value) => value.to_string(),
        SettingValue::Integer(value) => value.to_string(),
        SettingValue::WorkspaceMode(value) => value.to_string(),
        SettingValue::Text(value) => format!("'{}'", value.replace('\'', "''")),
    }
}

fn resolve_write_target(path: &Path) -> Result<PathBuf, SettingError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)
            .map_err(|source| io_error("resolve config symlink", path, source)),
        Ok(_) => Ok(path.to_path_buf()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(source) => Err(io_error("inspect", path, source)),
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), SettingError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| io_error("create directory", parent, source))?;
    let permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut temporary = None;
    let mut file = None;
    for attempt in 0..100u32 {
        let candidate = parent.join(format!(".{stem}.runyte-{nonce}-{attempt}.tmp"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(opened) => {
                temporary = Some(candidate);
                file = Some(opened);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error("create temporary config", &candidate, source)),
        }
    }
    let temporary = temporary.ok_or_else(|| SettingError::Io {
        operation: "create unique temporary config",
        path: parent.to_path_buf(),
        source: io::Error::new(io::ErrorKind::AlreadyExists, "temporary names exhausted"),
    })?;
    let result = (|| {
        let mut file = file.expect("temporary path and file are set together");
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)
                .map_err(|source| io_error("preserve permissions on", &temporary, source))?;
        }
        file.write_all(contents)
            .map_err(|source| io_error("write", &temporary, source))?;
        file.sync_all()
            .map_err(|source| io_error("sync", &temporary, source))?;
        drop(file);
        fs::rename(&temporary, path).map_err(|source| io_error("replace", path, source))?;
        // Some platforms do not allow directory handles. When the handle is
        // available, a failed directory-entry sync is a persistence failure,
        // not a success that happens to be less durable than advertised.
        if let Ok(directory) = fs::File::open(parent) {
            directory
                .sync_all()
                .map_err(|source| io_error("sync directory", parent, source))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> SettingError {
    SettingError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
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
                std::env::temp_dir().join(format!("runyte-settings-{}-{id}", std::process::id()));
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
    fn registry_has_stable_unique_keys_ids_and_typed_configured_values() {
        let config = Config::default();
        assert_eq!(SettingRegistry::descriptors().len(), SettingId::ALL.len());
        let keys = SettingRegistry::descriptors()
            .iter()
            .map(|descriptor| descriptor.key)
            .collect::<HashSet<_>>();
        assert_eq!(keys.len(), SettingId::ALL.len());
        let ids = SettingRegistry::descriptors()
            .iter()
            .map(|descriptor| descriptor.id)
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), SettingId::ALL.len());
        assert_eq!(
            SettingRegistry::descriptors()
                .iter()
                .map(|descriptor| descriptor.id)
                .collect::<Vec<_>>(),
            SettingId::ALL
        );
        assert_eq!(
            SettingId::EditorGrammar.configured_value(&config),
            SettingValue::Grammar(GrammarKind::Runyte)
        );
        assert_eq!(
            SettingId::EditorTrimTrailingWhitespace.configured_value(&config),
            SettingValue::Boolean(true)
        );
        assert_eq!(
            SettingId::WorkspaceMode.configured_value(&config),
            SettingValue::WorkspaceMode(WorkspaceMode::Standalone)
        );
        assert_eq!(
            SettingId::WorkspaceMode.allowed_values(&config),
            vec!["standalone", "persistent"]
        );
        assert_eq!(
            SettingId::Theme.allowed_values(&config),
            vec![
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
        assert_eq!(
            SettingId::EditorGrammar.allowed_values(&config),
            vec!["runyte"]
        );
    }

    #[test]
    fn config_page_is_eighty_cells_wide_and_wrapped_rows_keep_identity() {
        let values = SettingId::ALL
            .iter()
            .copied()
            .map(|setting| {
                (
                    setting,
                    setting.configured_value(&Config::default()).to_string(),
                )
            })
            .collect::<Vec<_>>();
        let page = render_settings_page(&values);
        for line in page.text.lines() {
            assert_eq!(
                line.chars()
                    .map(|character| character.width().unwrap_or(0))
                    .sum::<usize>(),
                SETTINGS_PAGE_WIDTH,
                "{line:?}"
            );
        }
        let wrapped = page
            .rows
            .iter()
            .filter(|setting| **setting == Some(SettingId::EditorShowHiddenFiles))
            .count();
        assert!(wrapped > 1, "the long description was not wrapped");
    }

    #[test]
    fn config_values_wrap_inside_the_ten_cell_value_column() {
        assert_eq!(VALUE_COLUMN_WIDTH, 10);
        let value_start =
            SETTING_COLUMN_WIDTH + COLUMN_GAP.len() + DESCRIPTION_COLUMN_WIDTH + COLUMN_GAP.len();
        let page = render_settings_page(&[(SettingId::Theme, "abcdefghijklmno".to_owned())]);
        let value_cells = page
            .text
            .lines()
            .zip(&page.rows)
            .filter(|(_, setting)| **setting == Some(SettingId::Theme))
            .map(|(line, _)| line.chars().skip(value_start).collect::<String>())
            .collect::<Vec<_>>();

        assert_eq!(value_cells[0], "abcdefghij");
        assert_eq!(value_cells[1], "klmno     ");
        assert!(page.text.lines().all(|line| {
            line.chars()
                .map(|character| character.width().unwrap_or(0))
                .sum::<usize>()
                == SETTINGS_PAGE_WIDTH
        }));
    }

    #[test]
    fn validation_is_typed_and_bounded() {
        let mut config = Config::default();
        assert!(
            SettingId::EditorTabWidth
                .validate(&SettingValue::Integer(16), &config)
                .is_ok()
        );
        assert!(
            SettingId::EditorTabWidth
                .validate(&SettingValue::Integer(0), &config)
                .is_err()
        );
        let mut preview = config.clone();
        SettingId::EditorTabWidth
            .apply(&SettingValue::Integer(2), &mut preview)
            .unwrap();
        assert_eq!(preview.editor.tab_width, 2);
        assert_eq!(config.editor.tab_width, 4);
        assert_eq!(config.editor.motion_repeat_multiplier, 2);
        assert!(
            SettingId::EditorTabWidth
                .validate(&SettingValue::Boolean(true), &config)
                .is_err()
        );
        assert!(
            SettingId::EditorMotionRepeatMultiplier
                .validate(&SettingValue::Integer(10), &config)
                .is_ok()
        );
        assert!(
            SettingId::EditorMotionRepeatMultiplier
                .validate(&SettingValue::Integer(11), &config)
                .is_err()
        );
        assert!(
            SettingId::EditorHardWrapWidth
                .validate(&SettingValue::Integer(1000), &config)
                .is_ok()
        );
        assert!(
            SettingId::EditorHardWrapWidth
                .validate(&SettingValue::Integer(0), &config)
                .is_err()
        );
        assert!(
            SettingId::EditorZenWidth
                .validate(&SettingValue::Integer(1000), &config)
                .is_ok()
        );
        assert!(
            SettingId::EditorZenWidth
                .validate(&SettingValue::Integer(0), &config)
                .is_err()
        );
        assert!(
            SettingId::Theme
                .validate(&SettingValue::Text("missing".into()), &config)
                .is_err()
        );
        config.themes.get_mut("paper").unwrap().accent = "not-a-color".to_owned();
        assert!(
            SettingId::Theme
                .validate(&SettingValue::Text("paper".into()), &config)
                .is_err()
        );
        assert!(
            !SettingId::Theme
                .allowed_values(&config)
                .contains(&"paper".to_owned())
        );
    }

    #[test]
    fn equal_keys_in_distinct_mapping_scopes_are_not_duplicates() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(
            &path,
            "editor:\n  tab_width: 2\nprivate:\n  tab_width: 99\n",
        )
        .unwrap();
        persist_setting(&path, SettingId::EditorTabWidth, &SettingValue::Integer(8)).unwrap();
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "editor:\n  tab_width: 8\nprivate:\n  tab_width: 99\n"
        );
    }

    #[test]
    fn replacing_a_scalar_preserves_every_other_byte() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(
            &path,
            "# mine\neditor:\n    tab_width: 2   # keep this\n    private_option: 'yes'\nunknown: [one, two]\n",
        )
        .unwrap();
        let config =
            persist_setting(&path, SettingId::EditorTabWidth, &SettingValue::Integer(8)).unwrap();
        assert_eq!(config.editor.tab_width, 8);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# mine\neditor:\n    tab_width: 8   # keep this\n    private_option: 'yes'\nunknown: [one, two]\n"
        );
    }

    #[test]
    fn comment_only_scalar_is_replaced_without_losing_its_comment() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(&path, "theme: # pick one\n").unwrap();

        let config =
            persist_setting(&path, SettingId::Theme, &SettingValue::Text("paper".into())).unwrap();

        assert_eq!(config.theme.as_deref(), Some("paper"));
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "theme: 'paper' # pick one\n"
        );
    }

    #[test]
    fn commented_mapping_header_remains_a_writable_parent() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(&path, "editor: # settings\n  tab_width: 2\n").unwrap();

        let config =
            persist_setting(&path, SettingId::EditorTabWidth, &SettingValue::Integer(8)).unwrap();

        assert_eq!(config.editor.tab_width, 8);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "editor: # settings\n  tab_width: 8\n"
        );
    }

    #[test]
    fn inserts_missing_keys_and_blocks_without_rewriting_comments() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(
            &path,
            "# heading\neditor:\n  tab_width: 2\n# root\ntheme: paper\n",
        )
        .unwrap();
        persist_setting(
            &path,
            SettingId::EditorSoftWrap,
            &SettingValue::Boolean(true),
        )
        .unwrap();
        persist_setting(&path, SettingId::LspEnable, &SettingValue::Boolean(false)).unwrap();
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# heading\neditor:\n  tab_width: 2\n  soft_wrap: true\n# root\ntheme: paper\nlsp:\n  enable: false\n"
        );
    }

    /// The realistic first use: the key is not in the file yet, because it is
    /// off by default and nobody has ever written it down.
    #[test]
    fn fast_pane_keys_is_written_into_a_config_that_never_mentioned_it() {
        assert!(!Config::default().editor.fast_pane_keys);

        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(&path, "# mine\neditor:\n  tab_width: 2\n").unwrap();

        let config = persist_setting(
            &path,
            SettingId::EditorFastPaneKeys,
            &SettingValue::Boolean(true),
        )
        .unwrap();

        assert!(config.editor.fast_pane_keys);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# mine\neditor:\n  tab_width: 2\n  fast_pane_keys: true\n"
        );
    }

    #[test]
    fn workspace_mode_persists_as_a_typed_unquoted_yaml_choice() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(&path, "# keep\nworkspace:\n  state: .editor-state # mine\n").unwrap();

        let config = persist_setting(
            &path,
            SettingId::WorkspaceMode,
            &SettingValue::WorkspaceMode(WorkspaceMode::Persistent),
        )
        .unwrap();

        assert_eq!(config.workspace.mode, WorkspaceMode::Persistent);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# keep\nworkspace:\n  state: .editor-state # mine\n  mode: persistent\n"
        );
    }

    /// Retirement is what turns a running host into a stopped row, so it is
    /// reachable from the settings view rather than only from the config file.
    /// Zero stays inside the accepted range because it is how "never retire"
    /// is spelled.
    #[test]
    fn idle_retirement_is_an_editable_setting_that_admits_zero() {
        let mut config = Config::default();
        assert_eq!(
            SettingId::WorkspaceIdleRetirementMinutes.configured_value(&config),
            SettingValue::Integer(1440)
        );
        assert!(SettingId::ALL.contains(&SettingId::WorkspaceIdleRetirementMinutes));

        for minutes in [0, 30, MAX_IDLE_RETIREMENT_MINUTES] {
            let value = SettingValue::Integer(minutes);
            SettingId::WorkspaceIdleRetirementMinutes
                .apply(&value, &mut config)
                .unwrap();
            assert_eq!(config.workspace.idle_retirement_minutes, minutes);
        }

        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(&path, "# keep\nworkspace:\n  mode: persistent\n").unwrap();
        let persisted = persist_setting(
            &path,
            SettingId::WorkspaceIdleRetirementMinutes,
            &SettingValue::Integer(45),
        )
        .unwrap();
        assert_eq!(persisted.workspace.idle_retirement_minutes, 45);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# keep\nworkspace:\n  mode: persistent\n  idle_retirement_minutes: 45\n"
        );
    }

    #[test]
    fn insertions_preserve_crlf_line_endings() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(&path, "theme: light\r\n").unwrap();
        persist_setting(
            &path,
            SettingId::EditorSoftWrap,
            &SettingValue::Boolean(true),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "theme: light\r\neditor:\r\n  soft_wrap: true\r\n"
        );
    }

    #[test]
    fn creates_a_missing_config_and_quotes_theme_names() {
        let directory = TempDir::new();
        let path = directory.path("nested/config.yaml");
        // Persist validation reads the themes from the file, so first create
        // an ordinary config containing the custom theme.
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "themes:\n  writer's room:\n    accent: '#123456'\n").unwrap();
        persist_setting(
            &path,
            SettingId::Theme,
            &SettingValue::Text("writer's room".into()),
        )
        .unwrap();
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .ends_with("theme: 'writer''s room'\n")
        );

        let fresh = directory.path("fresh/config.yaml");
        persist_setting(
            &fresh,
            SettingId::EditorLineNumbers,
            &SettingValue::Boolean(false),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(fresh).unwrap(),
            "editor:\n  line_numbers: false\n"
        );
    }

    #[test]
    fn refuses_unsafe_yaml_without_modifying_it() {
        let cases = [
            ("editor:\n\ttab_width: 2\n", "tab indentation"),
            ("editor: { tab_width: 2 }\n", "flow mappings"),
            (
                "defaults: &defaults\neditor: *defaults\n",
                "anchors and aliases",
            ),
            (
                "editor:\n  tab_width: 2\n  tab_width: 4\n",
                "duplicate mapping keys",
            ),
        ];
        for (index, (source, expected)) in cases.into_iter().enumerate() {
            let directory = TempDir::new();
            let path = directory.path(&format!("config-{index}.yaml"));
            fs::write(&path, source).unwrap();
            let error =
                persist_setting(&path, SettingId::EditorTabWidth, &SettingValue::Integer(8))
                    .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            assert_eq!(fs::read_to_string(path).unwrap(), source);
        }
    }

    #[test]
    fn invalid_typed_result_is_never_written() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        let source = "editor:\n  tab_width: nope\n";
        fs::write(&path, source).unwrap();
        let error = persist_setting(
            &path,
            SettingId::EditorLineNumbers,
            &SettingValue::Boolean(false),
        )
        .unwrap_err();
        assert!(matches!(error, SettingError::InvalidConfig(_)));
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlink_and_preserves_target_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let directory = TempDir::new();
        let target = directory.path("actual.yaml");
        let link = directory.path("config.yaml");
        fs::write(&target, "editor:\n  soft_wrap: false\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        persist_setting(
            &link,
            SettingId::EditorSoftWrap,
            &SettingValue::Boolean(true),
        )
        .unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "editor:\n  soft_wrap: true\n"
        );
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn broken_symlink_is_rejected_without_replacing_the_link() {
        let directory = TempDir::new();
        let target = directory.path("missing.yaml");
        let link = directory.path("config.yaml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let error = persist_setting(
            &link,
            SettingId::EditorSoftWrap,
            &SettingValue::Boolean(true),
        )
        .unwrap_err();

        assert!(matches!(error, SettingError::Io { .. }));
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!target.exists());
    }

    #[test]
    fn failed_validation_leaves_no_temporary_file() {
        let directory = TempDir::new();
        let path = directory.path("config.yaml");
        fs::write(&path, "theme: light\n").unwrap();
        let error = persist_setting(
            &path,
            SettingId::Theme,
            &SettingValue::Text("not-installed".into()),
        )
        .unwrap_err();
        assert!(matches!(error, SettingError::InvalidValue { .. }));
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }
}
