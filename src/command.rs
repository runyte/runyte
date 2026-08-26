// SPDX-License-Identifier: MPL-2.0

use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

/// The editor modes understood by commands and key bindings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Normal,
    Insert,
    Select,
    Command,
}

impl Mode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "NOR",
            Self::Insert => "INS",
            Self::Select => "SEL",
            Self::Command => "CMD",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandMetadata {
    pub name: &'static str,
    pub description: &'static str,
    pub capability: Option<CommandCapability>,
}

/// Optional editor capability required to execute a command meaningfully.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandCapability {
    Syntax,
    LspDocument,
    LspManager,
    GitProject,
}

/// Stable identities for commands that currently exist only on the colon
/// surface.  Commands already represented by `EditorCommand` reuse that
/// identity in the command inventory instead of acquiring a second one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ColonCommand {
    ChangeDirectory,
    CloseBuffer,
    DiffOff,
    DiffThis,
    ForceCloseBuffer,
    Format,
    GitBranches,
    GitBlame,
    GitBlameFile,
    GitCancel,
    GitCommit,
    GitDiff,
    GitDiffSideBySide,
    GitDiscard,
    GitIndex,
    GitLog,
    GitSearchCommits,
    GitRefresh,
    GitStage,
    GitStatus,
    GitStashes,
    GitStashTracked,
    GitStashAll,
    GitStashUntracked,
    GitStashApply,
    GitStashDrop,
    GitStageHunk,
    GitUnstageHunk,
    GitStageLines,
    GitUnstage,
    GitWorktrees,
    Grammar,
    LspRestart,
    LspStatus,
    Notifications,
    Open,
    Path,
    Detach,
    Quit,
    ForceQuit,
    QuitAll,
    ForceQuitAll,
    QuitHere,
    ForceQuitHere,
    Reload,
    ResizeRight,
    ResizeLeft,
    ResizeTop,
    ResizeBottom,
    ServiceHealth,
    WriteQuit,
    WriteBufferClose,
    SessionAttach,
    SessionList,
    SessionStart,
    SessionStop,
    SessionRename,
}

/// Editing grammar selected for interactive input.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GrammarKind {
    /// Runyte's Helix-style selection-first grammar.
    #[default]
    #[serde(alias = "helix")]
    Runyte,
    /// Retained so old programmatic callers receive an explicit removal
    /// error. It is not accepted from configuration or command text.
    #[serde(skip)]
    Vim,
}

impl GrammarKind {
    pub const ALL: &'static [Self] = &[Self::Runyte];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Runyte => "runyte",
            Self::Vim => "vim",
        }
    }
}

impl fmt::Display for GrammarKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

impl std::str::FromStr for GrammarKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "runyte" | "helix" => Ok(Self::Runyte),
            _ => Err(()),
        }
    }
}

/// Stable, presentation-neutral groups shared by command discovery surfaces.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandCategory {
    Application,
    Editing,
    Movement,
    Selection,
    Syntax,
    Search,
    View,
    File,
    Git,
    Window,
    Language,
    Clipboard,
    Register,
    Configuration,
    Terminal,
    Help,
}

impl CommandCategory {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Application => "Application",
            Self::Editing => "Editing",
            Self::Movement => "Movement",
            Self::Selection => "Selection",
            Self::Syntax => "Syntax",
            Self::Search => "Search",
            Self::View => "View",
            Self::File => "File",
            Self::Git => "Git",
            Self::Window => "Window",
            Self::Language => "Language",
            Self::Clipboard => "Clipboard",
            Self::Register => "Register",
            Self::Configuration => "Configuration",
            Self::Terminal => "Terminal",
            Self::Help => "Help",
        }
    }
}

/// How an editor command is exposed by the Phase 0 inventory.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandExposure {
    Bound,
    SharedColon,
    UnsupportedBinding,
    /// Reached only through one grammar's own parser, never through the shared
    /// keymap.
    GrammarOnly,
    Internal,
}

impl ColonCommand {
    pub const ALL: &'static [Self] = &[
        Self::ChangeDirectory,
        Self::CloseBuffer,
        Self::DiffOff,
        Self::DiffThis,
        Self::ForceCloseBuffer,
        Self::Format,
        Self::GitBranches,
        Self::GitBlame,
        Self::GitBlameFile,
        Self::GitCancel,
        Self::GitCommit,
        Self::GitDiff,
        Self::GitDiffSideBySide,
        Self::GitDiscard,
        Self::GitIndex,
        Self::GitLog,
        Self::GitSearchCommits,
        Self::GitRefresh,
        Self::GitStage,
        Self::GitStatus,
        Self::GitStashes,
        Self::GitStashTracked,
        Self::GitStashAll,
        Self::GitStashUntracked,
        Self::GitStashApply,
        Self::GitStashDrop,
        Self::GitStageHunk,
        Self::GitUnstageHunk,
        Self::GitStageLines,
        Self::GitUnstage,
        Self::GitWorktrees,
        Self::Grammar,
        Self::LspRestart,
        Self::LspStatus,
        Self::Notifications,
        Self::Open,
        Self::Path,
        Self::Detach,
        Self::Quit,
        Self::ForceQuit,
        Self::QuitAll,
        Self::ForceQuitAll,
        Self::QuitHere,
        Self::ForceQuitHere,
        Self::Reload,
        Self::ResizeRight,
        Self::ResizeLeft,
        Self::ResizeTop,
        Self::ResizeBottom,
        Self::ServiceHealth,
        Self::WriteQuit,
        Self::WriteBufferClose,
        Self::SessionAttach,
        Self::SessionList,
        Self::SessionStart,
        Self::SessionStop,
        Self::SessionRename,
    ];

    pub const fn category(self) -> CommandCategory {
        match self {
            Self::ChangeDirectory
            | Self::CloseBuffer
            | Self::ForceCloseBuffer
            | Self::Open
            | Self::Reload
            | Self::WriteQuit
            | Self::WriteBufferClose => CommandCategory::File,
            Self::ResizeRight | Self::ResizeLeft | Self::ResizeTop | Self::ResizeBottom => {
                CommandCategory::Window
            }
            Self::DiffThis | Self::DiffOff | Self::Notifications | Self::Path => {
                CommandCategory::View
            }
            Self::Format | Self::LspRestart | Self::LspStatus => CommandCategory::Language,
            Self::GitBranches
            | Self::GitBlame
            | Self::GitBlameFile
            | Self::GitCancel
            | Self::GitCommit
            | Self::GitDiff
            | Self::GitDiffSideBySide
            | Self::GitDiscard
            | Self::GitIndex
            | Self::GitLog
            | Self::GitSearchCommits
            | Self::GitRefresh
            | Self::GitStage
            | Self::GitStatus
            | Self::GitStashes
            | Self::GitStashTracked
            | Self::GitStashAll
            | Self::GitStashUntracked
            | Self::GitStashApply
            | Self::GitStashDrop
            | Self::GitStageHunk
            | Self::GitUnstageHunk
            | Self::GitStageLines
            | Self::GitUnstage
            | Self::GitWorktrees => CommandCategory::Git,
            Self::Detach
            | Self::Quit
            | Self::ForceQuit
            | Self::QuitAll
            | Self::ForceQuitAll
            | Self::QuitHere
            | Self::ForceQuitHere => CommandCategory::Application,
            Self::Grammar | Self::ServiceHealth => CommandCategory::Configuration,
            Self::SessionAttach
            | Self::SessionList
            | Self::SessionStart
            | Self::SessionStop
            | Self::SessionRename => CommandCategory::Application,
        }
    }
}

macro_rules! editor_commands {
    ($( $variant:ident => ($name:literal, $description:literal) ),+ $(,)?) => {
        /// Stable command identities shared by dispatch, help, and key hints.
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
        #[serde(rename_all = "kebab-case")]
        pub enum EditorCommand {
            $( $variant, )+
        }

        impl EditorCommand {
            pub const ALL: &'static [Self] = &[
                $( Self::$variant, )+
            ];

            pub const fn metadata(self) -> CommandMetadata {
                let capability = self.capability();
                match self {
                    $(
                        Self::$variant => CommandMetadata {
                            name: $name,
                            description: $description,
                            capability,
                        },
                    )+
                }
            }

            /// Whether this command consumes the next character as an
            /// operand instead of treating it as another key binding.
            pub const fn takes_character(self) -> bool {
                matches!(
                    self,
                    Self::FindNextChar
                        | Self::FindPreviousChar
                        | Self::FindTillNextChar
                        | Self::FindTillPreviousChar
                        | Self::ReplaceChar
                        | Self::SelectRegister
                        | Self::RecordMacro
                        | Self::ReplayMacro
                )
            }

            /// Whether the semantic command accepts a numeric address or
            /// repetition count. Input grammars own their separate policy
            /// for interpreting a count prefix.
            pub const fn accepts_count(self) -> bool {
                matches!(
                    self,
                    Self::MoveLeft
                        | Self::MoveRight
                        | Self::MoveUp
                        | Self::MoveDown
                        | Self::MoveWordForward
                        | Self::MoveWordBackward
                        | Self::MoveWordEnd
                        | Self::MoveLongWordForward
                        | Self::MoveLongWordBackward
                        | Self::MoveLongWordEnd
                        | Self::GotoNextParagraph
                        | Self::GotoPreviousParagraph
                        | Self::MoveFileStart
                        | Self::MoveFileEnd
                        | Self::PageUp
                        | Self::PageDown
                        | Self::HalfPageUp
                        | Self::HalfPageDown
                        | Self::ScrollViewDown
                        | Self::ScrollViewUp
                        | Self::SelectLine
                        | Self::SelectLineUp
                        | Self::PasteAfter
                        | Self::PasteBefore
                        | Self::ClipboardPasteAfter
                        | Self::ClipboardPasteBefore
                        | Self::Undo
                        | Self::Redo
                        | Self::ReplayMacro
                        | Self::ReplayDefaultMacro
                        | Self::ExpandSyntaxSelection
                        | Self::ShrinkSyntaxSelection
                        | Self::SelectSyntaxParent
                        | Self::SelectSyntaxChild
                        | Self::SelectPreviousSyntaxSibling
                        | Self::SelectNextSyntaxSibling
                        | Self::GotoPreviousSyntaxFunction
                        | Self::GotoNextSyntaxFunction
                        | Self::GotoPreviousSyntaxClass
                        | Self::GotoNextSyntaxClass
                        | Self::GotoPreviousSyntaxParameter
                        | Self::GotoNextSyntaxParameter
                )
            }
        }
    };
}

editor_commands! {
    EnterNormalMode => ("enter-normal-mode", "Return to normal mode"),
    OpenCommandPalette => ("open-command-palette", "Open the command palette"),

    MoveLeft => ("move-left", "Move left"),
    MoveRight => ("move-right", "Move right"),
    MoveUp => ("move-up", "Move up"),
    MoveDown => ("move-down", "Move down"),
    MoveLineStart => ("move-line-start", "Move to line start"),
    MoveLineEnd => ("move-line-end", "Move to line end"),
    MoveFirstNonWhitespace => (
        "move-first-non-whitespace",
        "Move to first non-whitespace character"
    ),
    MoveFileStart => ("move-file-start", "Move to file start"),
    MoveFileEnd => ("move-file-end", "Move to file end"),
    MoveWordForward => ("move-word-forward", "Move to next word start"),
    MoveWordBackward => ("move-word-backward", "Move to previous word start"),
    MoveWordEnd => ("move-word-end", "Move to next word end"),
    MoveLongWordForward => ("move-long-word-forward", "Move to next WORD start"),
    MoveLongWordBackward => ("move-long-word-backward", "Move to previous WORD start"),
    MoveLongWordEnd => ("move-long-word-end", "Move to next WORD end"),
    GotoNextParagraph => ("goto-next-paragraph", "Go to the next paragraph"),
    GotoPreviousParagraph => ("goto-previous-paragraph", "Go to the previous paragraph"),
    FindNextChar => ("find-next-char", "Find next character"),
    FindPreviousChar => ("find-previous-char", "Find previous character"),
    FindTillNextChar => ("find-till-next-char", "Find before next character"),
    FindTillPreviousChar => ("find-till-previous-char", "Find after previous character"),
    PageUp => ("page-up", "Move one page up"),
    PageDown => ("page-down", "Move one page down"),
    HalfPageUp => ("half-page-up", "Move half a page up"),
    HalfPageDown => ("half-page-down", "Move half a page down"),

    EnterInsertMode => ("enter-insert-mode", "Insert before the selection"),
    AppendAfter => ("append-after", "Insert after the selection"),
    InsertLineStart => ("insert-line-start", "Insert at line start"),
    InsertLineEnd => ("insert-line-end", "Insert at line end"),
    OpenLineBelow => ("open-line-below", "Open a line below"),
    OpenLineAbove => ("open-line-above", "Open a line above"),
    ReplaceChar => ("replace-char", "Replace selection with a character"),
    ToggleCase => ("toggle-case", "Switch case of the selection"),
    Undo => ("undo", "Undo the last change"),
    Redo => ("redo", "Redo the last change"),
    Yank => ("yank", "Yank the selection or character"),
    YankLine => ("yank-line", "Yank the lines the selection touches"),
    PasteAfter => ("paste-after", "Paste after the selection"),
    PasteBefore => ("paste-before", "Paste before the selection"),
    Indent => ("indent", "Indent the selected lines"),
    Unindent => ("unindent", "Unindent the selected lines"),
    ToggleComments => ("toggle-comments", "Comment or uncomment the selected lines"),
    DeleteSelection => ("delete-selection", "Delete the selection or character"),
    ChangeSelection => ("change-selection", "Change the selection or character"),

    EnterSelectMode => ("enter-select-mode", "Toggle select mode"),
    SelectLine => ("select-line", "Select the current line, then extend downward"),
    SelectLineUp => ("select-line-up", "Select the current line, then extend upward"),
    SelectAll => ("select-all", "Select all text"),
    CollapseSelection => ("collapse-selection", "Collapse selection to the cursor"),
    FlipSelection => ("flip-selection", "Flip the selection anchor and cursor"),
    ExpandSyntaxSelection => ("expand-syntax-selection", "Expand to the enclosing syntax node"),
    ShrinkSyntaxSelection => ("shrink-syntax-selection", "Shrink the syntax selection"),
    SelectSyntaxParent => ("select-syntax-parent", "Select the enclosing syntax node"),
    SelectSyntaxChild => ("select-syntax-child", "Select the first child syntax node"),
    SelectPreviousSyntaxSibling => (
        "select-previous-syntax-sibling",
        "Select the previous sibling syntax node"
    ),
    SelectNextSyntaxSibling => (
        "select-next-syntax-sibling",
        "Select the next sibling syntax node"
    ),
    SelectSyntaxFunction => ("select-syntax-function", "Select the enclosing function"),
    SelectInsideSyntaxFunction => (
        "select-inside-syntax-function",
        "Select inside the enclosing function"
    ),
    SelectSyntaxClass => ("select-syntax-class", "Select the enclosing class-like item"),
    SelectInsideSyntaxClass => (
        "select-inside-syntax-class",
        "Select inside the enclosing class-like item"
    ),
    SelectSyntaxParameter => ("select-syntax-parameter", "Select the enclosing parameter"),
    SelectInsideSyntaxParameter => (
        "select-inside-syntax-parameter",
        "Select inside the enclosing parameter"
    ),
    SelectAroundParentheses => ("select-around-parentheses", "Select around parentheses"),
    SelectInsideParentheses => ("select-inside-parentheses", "Select inside parentheses"),
    SelectAroundSquareBrackets => ("select-around-square-brackets", "Select around square brackets"),
    SelectInsideSquareBrackets => ("select-inside-square-brackets", "Select inside square brackets"),
    SelectAroundBraces => ("select-around-braces", "Select around braces"),
    SelectInsideBraces => ("select-inside-braces", "Select inside braces"),
    SelectAroundAngleBrackets => ("select-around-angle-brackets", "Select around angle brackets"),
    SelectInsideAngleBrackets => ("select-inside-angle-brackets", "Select inside angle brackets"),
    SelectAroundDoubleQuotes => ("select-around-double-quotes", "Select around double quotes"),
    SelectInsideDoubleQuotes => ("select-inside-double-quotes", "Select inside double quotes"),
    SelectAroundSingleQuotes => ("select-around-single-quotes", "Select around single quotes"),
    SelectInsideSingleQuotes => ("select-inside-single-quotes", "Select inside single quotes"),
    SelectAroundBackticks => ("select-around-backticks", "Select around backticks"),
    SelectInsideBackticks => ("select-inside-backticks", "Select inside backticks"),
    SelectAroundClosestDelimiter => (
        "select-around-closest-delimiter",
        "Select around the closest delimiter pair"
    ),
    SelectInsideClosestDelimiter => (
        "select-inside-closest-delimiter",
        "Select inside the closest delimiter pair"
    ),
    GotoPreviousSyntaxFunction => (
        "goto-previous-syntax-function",
        "Go to the previous function"
    ),
    GotoNextSyntaxFunction => ("goto-next-syntax-function", "Go to the next function"),
    GotoPreviousSyntaxClass => (
        "goto-previous-syntax-class",
        "Go to the previous class-like item"
    ),
    GotoNextSyntaxClass => ("goto-next-syntax-class", "Go to the next class-like item"),
    GotoPreviousSyntaxParameter => (
        "goto-previous-syntax-parameter",
        "Go to the previous parameter"
    ),
    GotoNextSyntaxParameter => ("goto-next-syntax-parameter", "Go to the next parameter"),
    DocumentOutline => ("document-outline", "Open the document outline"),
    ToggleSyntaxFold => ("toggle-syntax-fold", "Toggle the syntax fold at the cursor"),
    FoldAllSyntax => ("fold-all-syntax", "Fold every syntax region"),
    UnfoldAllSyntax => ("unfold-all-syntax", "Unfold every syntax region"),

    SplitSelectionAtLineEnds => (
        "split-selection-at-line-ends",
        "Place a cursor at the end of every selected line"
    ),
    SplitSelectionAtLineStarts => (
        "split-selection-at-line-starts",
        "Place a cursor at the start of every selected line"
    ),
    KeepPrimarySelection => ("keep-primary-selection", "Drop every selection except the primary"),
    RemovePrimarySelection => ("remove-primary-selection", "Drop the primary selection"),
    CopySelectionDown => ("copy-selection-down", "Add a cursor on the line below"),
    CopySelectionUp => ("copy-selection-up", "Add a cursor on the line above"),
    CopySelectionDownPadded => (
        "copy-selection-down-padded",
        "Add a cursor on the line below, padding it when needed"
    ),
    CopySelectionUpPadded => (
        "copy-selection-up-padded",
        "Add a cursor on the line above, padding it when needed"
    ),
    RotateSelectionForward => ("rotate-selection-forward", "Make the next selection primary"),
    RotateSelectionBackward => ("rotate-selection-backward", "Make the previous selection primary"),
    RotateSelectionContentsForward => (
        "rotate-selection-contents-forward",
        "Rotate selected text forward"
    ),
    RotateSelectionContentsBackward => (
        "rotate-selection-contents-backward",
        "Rotate selected text backward"
    ),
    KeepMatchingSelections => ("keep-matching-selections", "Keep selections matching a regular expression"),
    RemoveMatchingSelections => ("remove-matching-selections", "Remove selections matching a regular expression"),
    AlignSelections => (
        "align-selections",
        "Pad with spaces so every cursor shares the rightmost column"
    ),
    TrimSelections => ("trim-selections", "Trim whitespace from every selection"),
    TrimTrailingWhitespace => (
        "trim-trailing-whitespace",
        "Delete trailing whitespace from every selected line"
    ),
    HardWrap => ("hard-wrap", "Hard-wrap the selection"),
    Reflow => ("reflow", "Reflow paragraphs in the selection"),
    JoinSelections => ("join-selections", "Join the selected lines with a typed delimiter"),
    FormatTable => ("format-table", "Align the columns of the selected table"),

    Search => ("search", "Search for text, ignoring case"),
    SearchRegex => ("search-regex", "Search with a regular expression"),
    SearchForward => ("search-forward", "Search forward"),
    SearchBackward => ("search-backward", "Search backward"),
    SearchNext => ("search-next", "Select only the next search match"),
    SearchPrevious => ("search-previous", "Select only the previous search match"),
    SearchSelection => ("search-selection", "Select every match of the selection or word"),

    AlignViewCenter => ("align-view-center", "Center the cursor line in the view"),
    AlignViewTop => ("align-view-top", "Align the cursor line at the top"),
    AlignViewBottom => ("align-view-bottom", "Align the cursor line at the bottom"),
    AlignViewMiddle => ("align-view-middle", "Center the cursor column in the view"),
    ScrollViewDown => ("scroll-view-down", "Scroll the view down"),
    ScrollViewUp => ("scroll-view-up", "Scroll the view up"),
    GotoWindowTop => ("goto-window-top", "Move to the top of the view"),
    GotoWindowCenter => ("goto-window-center", "Move to the center of the view"),
    GotoWindowBottom => ("goto-window-bottom", "Move to the bottom of the view"),
    GotoWord => (
        "goto-word",
        "Label visible words by proximity and jump to one"
    ),
    ToggleSoftWrap => ("toggle-soft-wrap", "Toggle soft wrapping"),
    ToggleZen => ("toggle-zen", "Toggle the centred, maximized writing view"),
    ToggleFullscreen => (
        "toggle-fullscreen",
        "Toggle the active pane across the whole editor area"
    ),

    OpenExplorer => (
        "open-explorer",
        "Open file explorer in the active buffer's directory"
    ),
    OpenWorkingDirectoryExplorer => (
        "open-working-directory-explorer",
        "Open file explorer in the working directory"
    ),
    OpenFilePicker => (
        "open-file-picker",
        "Find project files, open buffers, and terminals"
    ),
    OpenDirectoryFilePicker => (
        "open-directory-file-picker",
        "Fuzzy-find a file or directory below the active directory"
    ),
    OpenFuzzyGrep => ("open-fuzzy-grep", "Fuzzy-search project file contents"),
    OpenDirectoryFuzzyGrep => (
        "open-directory-fuzzy-grep",
        "Fuzzy-search file contents below the active directory"
    ),
    OpenDirectoryEntry => ("open-directory-entry", "Open the selected directory entry"),
    OpenParentDirectory => ("open-parent-directory", "Open the parent directory"),
    RefreshDirectory => ("refresh-directory", "Reload the directory from disk"),
    ToggleHiddenFiles => ("toggle-hidden-files", "Show or hide dotfiles in the explorer"),
    OpenChangedFile => ("open-changed-file", "Open the file on this line"),
    StageAllChangedFiles => ("stage-all-changed-files", "Stage every changed file"),
    CheckoutBranch => ("checkout-branch", "Check out the branch on this line"),
    CreateBranch => ("create-branch", "Create a branch at the one on this line and switch to it"),
    DeleteBranch => ("delete-branch", "Delete the branch on this line, after a confirmation"),
    PullBranch => ("pull-branch", "Fast-forward the current branch onto what it tracks"),
    PushBranch => ("push-branch", "Publish this branch to what it tracks"),
    OpenWorktree => ("open-worktree", "Attach to the worktree on this line"),
    CreateWorktree => (
        "create-worktree",
        "Create a worktree from this row; attach in persistent mode"
    ),
    CreateNewWorktree => (
        "create-new-worktree",
        "Create a new branch and worktree; attach in persistent mode"
    ),
    RemoveWorktree => ("remove-worktree", "Remove the worktree on this row, leaving its branch"),
    NextGitLogPage => ("next-git-log-page", "Show the next page of the Git log"),
    PreviousGitLogPage => ("previous-git-log-page", "Show the previous page of the Git log"),
    OpenGitCommit => ("open-git-commit", "Open the commit on this log or blame row"),
    OpenWorkspaceSearchResult => (
        "open-workspace-search-result",
        "Open the workspace-search result on this line"
    ),
    ActivateSetting => ("activate-setting", "Change the setting on this line"),
    OpenSettings => ("open-settings", "Open editor settings"),
    OpenThemeSettings => ("open-theme-settings", "Choose and save the editor theme"),
    SplitVertical => ("split-vertical", "Create a side-by-side split"),
    SplitHorizontal => ("split-horizontal", "Create a stacked split"),
    Save => ("save", "Write the active buffer"),
    ForceSave => ("force-save", "Write the active buffer, replacing an existing file"),
    // Quitting has no key binding on purpose: leaving the editor is a typed
    // decision, so `:quit[!]` and `:q[!]` are the whole surface for it.
    ShowHelp => ("show-help", "Open general or contextual Runyte help"),
    ShowAbout => ("show-about", "Introduce Runyte and show its version"),
    FocusWindowLeft => ("focus-window-left", "Focus the pane to the left"),
    FocusWindowDown => ("focus-window-down", "Focus the pane below"),
    FocusWindowUp => ("focus-window-up", "Focus the pane above"),
    FocusWindowRight => ("focus-window-right", "Focus the pane to the right"),
    NextWindow => ("next-window", "Focus the next pane"),
    CloseWindow => ("close-window", "Close the active pane"),
    OnlyWindow => ("only-window", "Close every pane except the active pane"),
    EqualizeWindows => (
        "equalize-windows",
        "Equalize pane widths, then pane heights within each column"
    ),

    OpenTerminal => ("open-terminal", "Run a shell or command in this pane"),
    OpenTerminalFileDirectory => (
        "open-terminal-file-directory",
        "Run a terminal in the active file's directory"
    ),
    OpenTerminalDirectoryRoot => (
        "open-terminal-directory-root",
        "Run a terminal at the active directory root"
    ),
    OpenTerminalSelectedDirectory => (
        "open-terminal-selected-directory",
        "Run a terminal at the selected directory"
    ),
    OpenTerminalSessionDirectory => (
        "open-terminal-session-directory",
        "Run a terminal at another terminal's last safe directory"
    ),
    OpenTerminalList => ("open-terminal-list", "Show the running terminals"),
    ShowTerminal => ("show-terminal", "Show a terminal by stable ID or name"),
    RenameTerminal => ("rename-terminal", "Name the active terminal session"),
    LeaveTerminal => ("leave-terminal", "Show this pane's buffer again"),
    CopyTerminalOutput => (
        "copy-terminal-output",
        "Open this terminal's output as a buffer"
    ),
    SendToTerminal => ("send-to-terminal", "Send the selection to a terminal"),

    DeleteWordBackward => ("delete-word-backward", "Delete the previous word"),
    DeleteWordForward => ("delete-word-forward", "Delete the next word"),
    DeleteToLineStart => ("delete-to-line-start", "Delete to the start of the line"),
    DeleteToLineEnd => ("delete-to-line-end", "Delete to the end of the line"),
    DeleteCharBackward => ("delete-char-backward", "Delete the previous character"),
    DeleteCharForward => ("delete-char-forward", "Delete the next character"),
    InsertNewline => ("insert-newline", "Insert a new line"),
    InsertTab => ("insert-tab", "Insert indentation"),
    InsertLiteralTab => ("insert-literal-tab", "Insert a tab character"),
    CommitUndoCheckpoint => ("commit-undo-checkpoint", "Commit an undo checkpoint"),

    GotoDefinition => ("goto-definition", "Go to definition"),
    GotoDeclaration => ("goto-declaration", "Go to declaration"),
    GotoTypeDefinition => ("goto-type-definition", "Go to type definition"),
    GotoReferences => ("goto-references", "Go to references"),
    GotoImplementation => ("goto-implementation", "Go to implementation"),
    NewBuffer => ("new-buffer", "Open a new scratch buffer"),
    OpenBufferPicker => ("open-buffer-picker", "Open the buffer picker"),
    GlobalSearch => ("global-search", "Search the workspace, ignoring case"),
    GlobalSearchRegex => (
        "global-search-regex",
        "Search the workspace with a regular expression"
    ),
    ShowDocumentation => ("show-documentation", "Show documentation"),
    DocumentSymbols => ("document-symbols", "Open document symbols"),
    WorkspaceSymbols => ("workspace-symbols", "Open workspace symbols"),
    Diagnostics => ("diagnostics", "Open diagnostics"),
    TriggerCompletion => ("trigger-completion", "Ask the language server for completions"),
    JumpBackward => ("jump-backward", "Jump to the previous position"),
    JumpForward => ("jump-forward", "Jump to the next position"),
    JumpBackwardBuffer => (
        "jump-backward-buffer",
        "Jump to the previous buffer or terminal surface"
    ),
    JumpForwardBuffer => (
        "jump-forward-buffer",
        "Jump to the next buffer or terminal surface"
    ),
    RenameSymbol => ("rename-symbol", "Rename symbol"),
    CodeAction => ("code-action", "Apply a code action"),
    ClipboardPasteAfter => ("clipboard-paste-after", "Paste from the system clipboard"),
    ClipboardPasteBefore => ("clipboard-paste-before", "Paste before from the system clipboard"),
    ClipboardYank => ("clipboard-yank", "Yank to the system clipboard"),
    SelectRegister => ("select-register", "Select a register"),
    RecordMacro => ("record-macro", "Record a macro named by the next key"),
    RecordDefaultMacro => ("record-default-macro", "Record the default macro, or stop recording"),
    StopMacroRecording => ("stop-macro-recording", "Stop recording the current macro"),
    ReplayMacro => ("replay-macro", "Replay the macro named by the next key"),
    ReplayDefaultMacro => ("replay-default-macro", "Replay the default macro"),
    ListMacros => ("list-macros", "List the recorded macros"),
    ShellPipe => ("shell-pipe", "Pipe the selection through a shell command"),
    MatchBracket => ("match-bracket", "Go to the matching syntax bracket"),
}

impl EditorCommand {
    /// Whether executing this command ends at a buffer transaction.
    ///
    /// A read-only buffer refuses these, so the same predicate has to answer
    /// both "may this run" and "is this worth listing in help". Advertising a
    /// key that only ever reports a refusal is worse than omitting it.
    pub const fn is_mutating(self) -> bool {
        matches!(
            self,
            Self::EnterInsertMode
                | Self::AppendAfter
                | Self::InsertLineStart
                | Self::InsertLineEnd
                | Self::OpenLineBelow
                | Self::OpenLineAbove
                | Self::ToggleCase
                | Self::Undo
                | Self::Redo
                | Self::PasteAfter
                | Self::PasteBefore
                | Self::ClipboardPasteAfter
                | Self::ClipboardPasteBefore
                | Self::Indent
                | Self::Unindent
                | Self::ToggleComments
                | Self::DeleteSelection
                | Self::ChangeSelection
                | Self::ReplaceChar
                | Self::Save
                | Self::DeleteWordBackward
                | Self::DeleteWordForward
                | Self::DeleteToLineStart
                | Self::DeleteToLineEnd
                | Self::DeleteCharBackward
                | Self::DeleteCharForward
                | Self::InsertNewline
                | Self::InsertTab
                | Self::InsertLiteralTab
                | Self::RotateSelectionContentsForward
                | Self::RotateSelectionContentsBackward
                | Self::HardWrap
                | Self::Reflow
                | Self::JoinSelections
                | Self::FormatTable
                // These reach a buffer through the language server rather than
                // directly, but they still end at a transaction.
                | Self::RenameSymbol
                | Self::CodeAction
                | Self::TriggerCompletion
        )
    }

    pub const fn capability(self) -> Option<CommandCapability> {
        match self {
            Self::ExpandSyntaxSelection
            | Self::ShrinkSyntaxSelection
            | Self::SelectSyntaxParent
            | Self::SelectSyntaxChild
            | Self::SelectPreviousSyntaxSibling
            | Self::SelectNextSyntaxSibling
            | Self::SelectSyntaxFunction
            | Self::SelectInsideSyntaxFunction
            | Self::SelectSyntaxClass
            | Self::SelectInsideSyntaxClass
            | Self::SelectSyntaxParameter
            | Self::SelectInsideSyntaxParameter
            | Self::SelectAroundParentheses
            | Self::SelectInsideParentheses
            | Self::SelectAroundSquareBrackets
            | Self::SelectInsideSquareBrackets
            | Self::SelectAroundBraces
            | Self::SelectInsideBraces
            | Self::SelectAroundAngleBrackets
            | Self::SelectInsideAngleBrackets
            | Self::SelectAroundDoubleQuotes
            | Self::SelectInsideDoubleQuotes
            | Self::SelectAroundSingleQuotes
            | Self::SelectInsideSingleQuotes
            | Self::SelectAroundBackticks
            | Self::SelectInsideBackticks
            | Self::SelectAroundClosestDelimiter
            | Self::SelectInsideClosestDelimiter
            | Self::GotoPreviousSyntaxFunction
            | Self::GotoNextSyntaxFunction
            | Self::GotoPreviousSyntaxClass
            | Self::GotoNextSyntaxClass
            | Self::GotoPreviousSyntaxParameter
            | Self::GotoNextSyntaxParameter
            | Self::DocumentOutline
            | Self::ToggleSyntaxFold
            | Self::FoldAllSyntax
            | Self::UnfoldAllSyntax => Some(CommandCapability::Syntax),
            Self::GotoDefinition
            | Self::GotoDeclaration
            | Self::GotoTypeDefinition
            | Self::GotoReferences
            | Self::GotoImplementation
            | Self::ShowDocumentation
            | Self::DocumentSymbols
            | Self::WorkspaceSymbols
            | Self::TriggerCompletion
            | Self::RenameSymbol
            | Self::CodeAction => Some(CommandCapability::LspDocument),
            _ if matches!(self.category(), CommandCategory::Git) => {
                Some(CommandCapability::GitProject)
            }
            _ => None,
        }
    }

    pub const fn category(self) -> CommandCategory {
        match self {
            Self::EnterNormalMode | Self::OpenCommandPalette => CommandCategory::Application,
            Self::MoveLeft
            | Self::MoveRight
            | Self::MoveUp
            | Self::MoveDown
            | Self::MoveLineStart
            | Self::MoveLineEnd
            | Self::MoveFirstNonWhitespace
            | Self::MoveFileStart
            | Self::MoveFileEnd
            | Self::MoveWordForward
            | Self::MoveWordBackward
            | Self::MoveWordEnd
            | Self::MoveLongWordForward
            | Self::MoveLongWordBackward
            | Self::MoveLongWordEnd
            | Self::GotoNextParagraph
            | Self::GotoPreviousParagraph
            | Self::FindNextChar
            | Self::FindPreviousChar
            | Self::FindTillNextChar
            | Self::FindTillPreviousChar
            | Self::PageUp
            | Self::PageDown
            | Self::HalfPageUp
            | Self::HalfPageDown
            | Self::GotoWindowTop
            | Self::GotoWindowCenter
            | Self::GotoWindowBottom
            | Self::GotoWord
            | Self::JumpBackward
            | Self::JumpForward
            | Self::JumpBackwardBuffer
            | Self::JumpForwardBuffer => CommandCategory::Movement,
            Self::EnterInsertMode
            | Self::AppendAfter
            | Self::InsertLineStart
            | Self::InsertLineEnd
            | Self::OpenLineBelow
            | Self::OpenLineAbove
            | Self::ReplaceChar
            | Self::ToggleCase
            | Self::Undo
            | Self::Redo
            | Self::Yank
            | Self::YankLine
            | Self::PasteAfter
            | Self::PasteBefore
            | Self::Indent
            | Self::Unindent
            | Self::ToggleComments
            | Self::DeleteSelection
            | Self::ChangeSelection
            | Self::DeleteWordBackward
            | Self::DeleteWordForward
            | Self::DeleteToLineStart
            | Self::DeleteToLineEnd
            | Self::DeleteCharBackward
            | Self::DeleteCharForward
            | Self::InsertNewline
            | Self::InsertTab
            | Self::InsertLiteralTab
            | Self::CommitUndoCheckpoint
            | Self::ShellPipe => CommandCategory::Editing,
            Self::EnterSelectMode
            | Self::SelectLine
            | Self::SelectLineUp
            | Self::SelectAll
            | Self::CollapseSelection
            | Self::FlipSelection
            | Self::SplitSelectionAtLineEnds
            | Self::SplitSelectionAtLineStarts
            | Self::KeepPrimarySelection
            | Self::RemovePrimarySelection
            | Self::CopySelectionDown
            | Self::CopySelectionUp
            | Self::CopySelectionDownPadded
            | Self::CopySelectionUpPadded
            | Self::RotateSelectionForward
            | Self::RotateSelectionBackward
            | Self::RotateSelectionContentsForward
            | Self::RotateSelectionContentsBackward
            | Self::KeepMatchingSelections
            | Self::RemoveMatchingSelections
            | Self::AlignSelections
            | Self::TrimSelections => CommandCategory::Selection,
            Self::HardWrap
            | Self::Reflow
            | Self::JoinSelections
            | Self::FormatTable
            | Self::TrimTrailingWhitespace => CommandCategory::Editing,
            Self::ExpandSyntaxSelection
            | Self::ShrinkSyntaxSelection
            | Self::SelectSyntaxParent
            | Self::SelectSyntaxChild
            | Self::SelectPreviousSyntaxSibling
            | Self::SelectNextSyntaxSibling
            | Self::SelectSyntaxFunction
            | Self::SelectInsideSyntaxFunction
            | Self::SelectSyntaxClass
            | Self::SelectInsideSyntaxClass
            | Self::SelectSyntaxParameter
            | Self::SelectInsideSyntaxParameter
            | Self::SelectAroundParentheses
            | Self::SelectInsideParentheses
            | Self::SelectAroundSquareBrackets
            | Self::SelectInsideSquareBrackets
            | Self::SelectAroundBraces
            | Self::SelectInsideBraces
            | Self::SelectAroundAngleBrackets
            | Self::SelectInsideAngleBrackets
            | Self::SelectAroundDoubleQuotes
            | Self::SelectInsideDoubleQuotes
            | Self::SelectAroundSingleQuotes
            | Self::SelectInsideSingleQuotes
            | Self::SelectAroundBackticks
            | Self::SelectInsideBackticks
            | Self::SelectAroundClosestDelimiter
            | Self::SelectInsideClosestDelimiter
            | Self::GotoPreviousSyntaxFunction
            | Self::GotoNextSyntaxFunction
            | Self::GotoPreviousSyntaxClass
            | Self::GotoNextSyntaxClass
            | Self::GotoPreviousSyntaxParameter
            | Self::GotoNextSyntaxParameter
            | Self::DocumentOutline
            | Self::ToggleSyntaxFold
            | Self::FoldAllSyntax
            | Self::UnfoldAllSyntax => CommandCategory::Syntax,
            Self::Search
            | Self::SearchRegex
            | Self::SearchForward
            | Self::SearchBackward
            | Self::SearchNext
            | Self::SearchPrevious
            | Self::SearchSelection
            | Self::OpenFuzzyGrep
            | Self::OpenDirectoryFuzzyGrep
            | Self::GlobalSearch
            | Self::GlobalSearchRegex => CommandCategory::Search,
            Self::AlignViewCenter
            | Self::AlignViewTop
            | Self::AlignViewBottom
            | Self::AlignViewMiddle
            | Self::ScrollViewDown
            | Self::ScrollViewUp => CommandCategory::View,
            Self::ToggleSoftWrap | Self::ToggleZen | Self::ToggleFullscreen => {
                CommandCategory::View
            }
            Self::OpenExplorer
            | Self::OpenWorkingDirectoryExplorer
            | Self::OpenFilePicker
            | Self::OpenDirectoryFilePicker
            | Self::OpenDirectoryEntry
            | Self::OpenParentDirectory
            | Self::RefreshDirectory
            | Self::ToggleHiddenFiles
            | Self::Save
            | Self::ForceSave
            | Self::NewBuffer
            | Self::OpenBufferPicker => CommandCategory::File,
            Self::SplitVertical
            | Self::SplitHorizontal
            | Self::FocusWindowLeft
            | Self::FocusWindowDown
            | Self::FocusWindowUp
            | Self::FocusWindowRight
            | Self::NextWindow
            | Self::CloseWindow
            | Self::OnlyWindow
            | Self::EqualizeWindows => CommandCategory::Window,
            Self::GotoDefinition
            | Self::GotoDeclaration
            | Self::GotoTypeDefinition
            | Self::GotoReferences
            | Self::GotoImplementation
            | Self::ShowDocumentation
            | Self::DocumentSymbols
            | Self::WorkspaceSymbols
            | Self::Diagnostics
            | Self::TriggerCompletion
            | Self::RenameSymbol
            | Self::CodeAction
            | Self::MatchBracket => CommandCategory::Language,
            Self::ClipboardPasteAfter | Self::ClipboardPasteBefore | Self::ClipboardYank => {
                CommandCategory::Clipboard
            }
            Self::SelectRegister
            | Self::RecordMacro
            | Self::RecordDefaultMacro
            | Self::StopMacroRecording
            | Self::ReplayMacro
            | Self::ReplayDefaultMacro
            | Self::ListMacros => CommandCategory::Register,
            Self::OpenChangedFile
            | Self::StageAllChangedFiles
            | Self::CheckoutBranch
            | Self::CreateBranch
            | Self::DeleteBranch
            | Self::PullBranch
            | Self::PushBranch => CommandCategory::Git,
            Self::OpenWorktree
            | Self::CreateWorktree
            | Self::CreateNewWorktree
            | Self::RemoveWorktree => CommandCategory::Git,
            Self::NextGitLogPage | Self::PreviousGitLogPage | Self::OpenGitCommit => {
                CommandCategory::Git
            }
            Self::ActivateSetting | Self::OpenSettings | Self::OpenThemeSettings => {
                CommandCategory::Configuration
            }
            Self::OpenWorkspaceSearchResult => CommandCategory::View,
            Self::ShowHelp | Self::ShowAbout => CommandCategory::Help,
            Self::OpenTerminal
            | Self::OpenTerminalFileDirectory
            | Self::OpenTerminalDirectoryRoot
            | Self::OpenTerminalSelectedDirectory
            | Self::OpenTerminalSessionDirectory
            | Self::OpenTerminalList
            | Self::ShowTerminal
            | Self::RenameTerminal
            | Self::LeaveTerminal
            | Self::CopyTerminalOutput
            | Self::SendToTerminal => CommandCategory::Terminal,
        }
    }
}

/// Commands that exist to name an internal editor transition rather than a
/// current user-facing binding or colon spelling.
pub const INTERNAL_EDITOR_COMMANDS: &[EditorCommand] = &[
    EditorCommand::CommitUndoCheckpoint,
    EditorCommand::RefreshDirectory,
    EditorCommand::StopMacroRecording,
];

/// Commands one grammar reaches through its own parser rather than through the
/// shared keymap.
///
/// The Vim grammar keeps `/` and `?` as directional single-match searches, so
/// both identities stay alive even though the Runyte grammar spends those keys
/// on the regular-expression search and on nothing at all.
pub const GRAMMAR_ONLY_EDITOR_COMMANDS: &[EditorCommand] =
    &[EditorCommand::SearchForward, EditorCommand::SearchBackward];

/// One identity shared by every currently inventoried command surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandId {
    Editor(EditorCommand),
    Colon(ColonCommand),
}

impl From<EditorCommand> for CommandId {
    fn from(command: EditorCommand) -> Self {
        Self::Editor(command)
    }
}

impl From<ColonCommand> for CommandId {
    fn from(command: ColonCommand) -> Self {
        Self::Colon(command)
    }
}

impl CommandId {
    pub const fn category(self) -> CommandCategory {
        match self {
            Self::Editor(command) => command.category(),
            Self::Colon(command) => command.category(),
        }
    }

    pub const fn capability(self) -> Option<CommandCapability> {
        match self {
            Self::Editor(command) => command.capability(),
            Self::Colon(ColonCommand::Format) => Some(CommandCapability::LspDocument),
            Self::Colon(ColonCommand::LspRestart | ColonCommand::LspStatus) => {
                Some(CommandCapability::LspManager)
            }
            Self::Colon(command) if matches!(command.category(), CommandCategory::Git) => {
                Some(CommandCapability::GitProject)
            }
            Self::Colon(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentKind {
    Path,
    FreeText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandArguments {
    None,
    Optional(ArgumentKind),
    Required(ArgumentKind),
}

impl CommandArguments {
    pub const fn accepts_arguments(self) -> bool {
        !matches!(self, Self::None)
    }

    pub const fn is_required(self) -> bool {
        matches!(self, Self::Required(_))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CommandSpec {
    pub id: CommandId,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub usage: &'static str,
    pub description: &'static str,
    pub arguments: CommandArguments,
}

impl CommandSpec {
    /// Every spelling that resolves to this command, canonical name first.
    pub fn names(&'static self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name).chain(self.aliases.iter().copied())
    }

    pub const fn category(self) -> CommandCategory {
        self.id.category()
    }

    pub const fn capability(self) -> Option<CommandCapability> {
        self.id.capability()
    }
}

macro_rules! spec {
    ($id:expr, $name:literal, [$($alias:literal),*], $usage:literal, $description:literal, $arguments:expr) => {
        CommandSpec {
            id: $id,
            name: $name,
            aliases: &[$($alias),*],
            usage: $usage,
            description: $description,
            arguments: $arguments,
        }
    };
}

macro_rules! editor_spec {
    ($command:expr, $name:literal, [$($alias:literal),*], $usage:literal, $arguments:expr) => {
        CommandSpec {
            id: EditorId($command),
            name: $name,
            aliases: &[$($alias),*],
            usage: $usage,
            description: $command.metadata().description,
            arguments: $arguments,
        }
    };
}

use ArgumentKind::{FreeText, Path};
use ColonCommand as Colon;
use CommandArguments::{None as NoArguments, Optional, Required};
use CommandId::{Colon as ColonId, Editor as EditorId};
use EditorCommand as Editor;

pub const COMMANDS: &[CommandSpec] = &[
    spec!(
        ColonId(Colon::ChangeDirectory),
        "cd",
        [],
        "cd <path>",
        "Change the editor working directory",
        Required(Path)
    ),
    spec!(
        ColonId(Colon::SessionAttach),
        "session-attach",
        [],
        "session-attach <workspace>",
        "Attach to another workspace's persistent session",
        Required(Path)
    ),
    spec!(
        ColonId(Colon::SessionList),
        "session-list",
        ["sl"],
        "session-list",
        "List known persistent sessions and switch to one",
        NoArguments
    ),
    spec!(
        ColonId(Colon::SessionStart),
        "session-start",
        [],
        "session-start [workspace]",
        "Start a persistent session without switching",
        Optional(Path)
    ),
    spec!(
        ColonId(Colon::SessionStop),
        "session-stop",
        [],
        "session-stop [workspace]",
        "Stop a clean persistent session",
        Optional(Path)
    ),
    spec!(
        ColonId(Colon::SessionRename),
        "session-rename",
        [],
        "session-rename <workspace> <name>",
        "Rename a persistent session",
        Required(FreeText)
    ),
    spec!(
        ColonId(Colon::DiffThis),
        "diff-this",
        ["difft", "dt"],
        "diff-this",
        "Compare this buffer with the next one marked",
        NoArguments
    ),
    spec!(
        ColonId(Colon::DiffOff),
        "diff-off",
        ["do"],
        "diff-off",
        "Close the comparison this buffer is part of",
        NoArguments
    ),
    editor_spec!(
        Editor::CloseWindow,
        "window-close",
        ["wc"],
        "window-close",
        NoArguments
    ),
    editor_spec!(
        Editor::OpenWorkingDirectoryExplorer,
        "explorer",
        ["files"],
        "explorer [path]",
        Optional(Path)
    ),
    editor_spec!(
        Editor::OpenFilePicker,
        "file-picker",
        [],
        "file-picker",
        NoArguments
    ),
    editor_spec!(
        Editor::OpenDirectoryFilePicker,
        "file-picker-directory",
        [],
        "file-picker-directory",
        NoArguments
    ),
    editor_spec!(
        Editor::OpenFuzzyGrep,
        "fuzzy-grep",
        [],
        "fuzzy-grep",
        NoArguments
    ),
    editor_spec!(
        Editor::OpenDirectoryFuzzyGrep,
        "fuzzy-grep-directory",
        [],
        "fuzzy-grep-directory",
        NoArguments
    ),
    spec!(
        ColonId(Colon::Format),
        "format",
        ["fmt"],
        "format",
        "Format the active buffer with its language server",
        NoArguments
    ),
    spec!(
        ColonId(Colon::Grammar),
        "grammar",
        [],
        "grammar [runyte]",
        "Report the active Runyte editing grammar",
        Optional(FreeText)
    ),
    editor_spec!(
        Editor::ShowHelp,
        "help",
        ["?"],
        "help [topic]",
        Optional(FreeText)
    ),
    editor_spec!(Editor::ShowAbout, "about", [], "about", NoArguments),
    editor_spec!(
        Editor::OpenTerminal,
        "terminal",
        ["t", "term"],
        "terminal [command]",
        Optional(FreeText)
    ),
    editor_spec!(
        Editor::OpenTerminalFileDirectory,
        "terminal-file-directory",
        [],
        "terminal-file-directory [command]",
        Optional(FreeText)
    ),
    editor_spec!(
        Editor::OpenTerminalDirectoryRoot,
        "terminal-directory-root",
        [],
        "terminal-directory-root [command]",
        Optional(FreeText)
    ),
    editor_spec!(
        Editor::OpenTerminalSelectedDirectory,
        "terminal-selected-directory",
        [],
        "terminal-selected-directory [command]",
        Optional(FreeText)
    ),
    editor_spec!(
        Editor::OpenTerminalSessionDirectory,
        "terminal-session-directory",
        [],
        "terminal-session-directory <id|name>",
        Required(FreeText)
    ),
    editor_spec!(
        Editor::OpenTerminalList,
        "terminals",
        [],
        "terminals",
        NoArguments
    ),
    editor_spec!(
        Editor::ShowTerminal,
        "terminal-show",
        [],
        "terminal-show <id|name>",
        Required(FreeText)
    ),
    editor_spec!(
        Editor::RenameTerminal,
        "terminal-rename",
        [],
        "terminal-rename <name>",
        Required(FreeText)
    ),
    editor_spec!(
        Editor::CopyTerminalOutput,
        "terminal-output",
        [],
        "terminal-output",
        NoArguments
    ),
    editor_spec!(
        Editor::SendToTerminal,
        "terminal-send",
        [],
        "terminal-send [id|name]",
        Optional(FreeText)
    ),
    editor_spec!(Editor::ToggleZen, "zen", [], "zen", NoArguments),
    editor_spec!(
        Editor::ToggleFullscreen,
        "fullscreen",
        [],
        "fullscreen",
        NoArguments
    ),
    spec!(
        ColonId(Colon::ServiceHealth),
        "service-health",
        ["health"],
        "service-health",
        "Inspect optional editor service health",
        NoArguments
    ),
    spec!(
        ColonId(Colon::Notifications),
        "notifications",
        ["not"],
        "notifications",
        "Open the retained notification history",
        NoArguments
    ),
    spec!(
        ColonId(Colon::Path),
        "path",
        [],
        "path",
        "Show the active buffer's absolute path",
        NoArguments
    ),
    editor_spec!(
        Editor::OpenSettings,
        "config",
        ["settings"],
        "config",
        NoArguments
    ),
    editor_spec!(
        Editor::OpenThemeSettings,
        "theme",
        [],
        "theme [name]",
        Optional(FreeText)
    ),
    editor_spec!(
        Editor::SplitHorizontal,
        "hsplit",
        ["split"],
        "hsplit [path]",
        Optional(Path)
    ),
    spec!(
        ColonId(Colon::CloseBuffer),
        "close",
        ["c", "buffer-close", "bc", "close-buffer", "cb"],
        "close",
        "Close the active buffer without changing the pane layout",
        NoArguments
    ),
    spec!(
        ColonId(Colon::ForceCloseBuffer),
        "close!",
        ["c!", "buffer-close!", "bc!"],
        "close!",
        "Close the active buffer and discard its unsaved text",
        NoArguments
    ),
    editor_spec!(
        Editor::NewBuffer,
        "buffer-new",
        ["new"],
        "buffer-new",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitBranches),
        "git-branches",
        [],
        "git-branches",
        "Open the local branch list",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitLog),
        "git-log",
        [],
        "git-log",
        "Open the Git log, or refresh it from its first page",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitSearchCommits),
        "git-search-commits",
        [],
        "git-search-commits",
        "Fuzzy-search commits reachable from HEAD by message, object ID, author, or date",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitBlame),
        "git-blame",
        [],
        "git-blame",
        "Show attribution for the primary line using live buffer text",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitBlameFile),
        "git-blame-file",
        [],
        "git-blame-file",
        "Open full-file attribution using live buffer text",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitWorktrees),
        "git-worktrees",
        [],
        "git-worktrees",
        "Open or refresh the repository worktree list",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitCommit),
        "git-commit",
        [],
        "git-commit",
        "Write a message and commit what is staged",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitDiff),
        "git-diff",
        [],
        "git-diff",
        "Show the active file's unstaged diff",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitDiffSideBySide),
        "git-diff-side-by-side",
        [],
        "git-diff-side-by-side",
        "Compare the two complete versions of the active file",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitDiscard),
        "git-discard",
        [],
        "git-discard",
        "Throw away a file's uncommitted changes, after a confirmation",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitIndex),
        "git-index",
        [],
        "git-index",
        "Review everything staged for the next commit",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitCancel),
        "git-cancel",
        [],
        "git-cancel",
        "Stop the active Git operation and reconcile uncertain mutations",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitStatus),
        "git-status",
        [],
        "git-status",
        "Open the changed-file list",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitStashes),
        "git-stashes",
        [],
        "git-stashes",
        "Open or refresh the bounded stash list",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitStashTracked),
        "git-stash-tracked",
        [],
        "git-stash-tracked <name>",
        "Confirm a tracked-worktree snapshot while keeping the index applied",
        Optional(FreeText)
    ),
    spec!(
        ColonId(Colon::GitStashAll),
        "git-stash-all",
        [],
        "git-stash-all <name>",
        "Confirm a named stash of tracked worktree and index changes",
        Optional(FreeText)
    ),
    spec!(
        ColonId(Colon::GitStashUntracked),
        "git-stash-untracked",
        [],
        "git-stash-untracked <name>",
        "Confirm a named stash including untracked files",
        Optional(FreeText)
    ),
    spec!(
        ColonId(Colon::GitStashApply),
        "git-stash-apply",
        [],
        "git-stash-apply",
        "Confirm applying the selected stash without dropping it",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitStashDrop),
        "git-stash-drop",
        [],
        "git-stash-drop",
        "Confirm dropping the selected stash",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitStageHunk),
        "git-stage-hunk",
        [],
        "git-stage-hunk",
        "Stage the exact hunk under the cursor",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitUnstageHunk),
        "git-unstage-hunk",
        [],
        "git-unstage-hunk",
        "Unstage the exact hunk under the cursor",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitStageLines),
        "git-stage-lines",
        [],
        "git-stage-lines",
        "Stage the supported saved source-line selection",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitStage),
        "git-stage",
        [],
        "git-stage",
        "Stage the active file",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitUnstage),
        "git-unstage",
        [],
        "git-unstage",
        "Unstage the active file",
        NoArguments
    ),
    spec!(
        ColonId(Colon::GitRefresh),
        "git-refresh",
        [],
        "git-refresh",
        "Re-read branch, changed files, and changed lines from Git",
        NoArguments
    ),
    spec!(
        ColonId(Colon::LspRestart),
        "lsp-restart",
        [],
        "lsp-restart [language]",
        "Restart stopped language servers",
        Optional(FreeText)
    ),
    spec!(
        ColonId(Colon::LspStatus),
        "lsp-status",
        [],
        "lsp-status",
        "Report language server state",
        NoArguments
    ),
    spec!(
        ColonId(Colon::Open),
        "open",
        ["e", "edit"],
        "open <path>",
        "Open a file or directory in the active pane",
        Required(Path)
    ),
    spec!(
        ColonId(Colon::Detach),
        "detach",
        [],
        "detach",
        "Detach the TUI from its persistent session",
        NoArguments
    ),
    spec!(
        ColonId(Colon::Quit),
        "quit",
        ["q"],
        "quit",
        "Close the active pane and its unique buffer, or quit from the last pane",
        NoArguments
    ),
    spec!(
        ColonId(Colon::ForceQuit),
        "quit!",
        ["q!"],
        "quit!",
        "Discard a unique buffer and close its pane, or force quit from the last pane",
        NoArguments
    ),
    spec!(
        ColonId(Colon::QuitAll),
        "quit-all",
        ["qa"],
        "quit-all",
        "Quit if all buffers are saved and no standalone terminal is running",
        NoArguments
    ),
    spec!(
        ColonId(Colon::ForceQuitAll),
        "quit-all!",
        ["qa!"],
        "quit-all!",
        "Quit and discard unsaved buffers, but never terminate terminals",
        NoArguments
    ),
    spec!(
        ColonId(Colon::QuitHere),
        "quit-here",
        ["qh"],
        "quit-here",
        "Quit and hand the active directory to the shell wrapper",
        NoArguments
    ),
    spec!(
        ColonId(Colon::ForceQuitHere),
        "quit-here!",
        ["qh!"],
        "quit-here!",
        "Discard changes, quit, and hand the active directory to the shell wrapper",
        NoArguments
    ),
    spec!(
        ColonId(Colon::Reload),
        "reload",
        [],
        "reload",
        "Reload the active file or refresh the active explorer or Git list",
        NoArguments
    ),
    spec!(
        ColonId(Colon::ResizeRight),
        "resize-right",
        [],
        "resize-right <+|-> <cells>",
        "Grow or shrink the active pane at its right edge",
        Required(FreeText)
    ),
    spec!(
        ColonId(Colon::ResizeLeft),
        "resize-left",
        [],
        "resize-left <+|-> <cells>",
        "Grow or shrink the active pane at its left edge",
        Required(FreeText)
    ),
    spec!(
        ColonId(Colon::ResizeTop),
        "resize-top",
        [],
        "resize-top <+|-> <cells>",
        "Grow or shrink the active pane at its top edge",
        Required(FreeText)
    ),
    spec!(
        ColonId(Colon::ResizeBottom),
        "resize-bottom",
        [],
        "resize-bottom <+|-> <cells>",
        "Grow or shrink the active pane at its bottom edge",
        Required(FreeText)
    ),
    editor_spec!(
        Editor::DocumentOutline,
        "outline",
        ["document-outline"],
        "outline",
        NoArguments
    ),
    editor_spec!(
        Editor::SplitVertical,
        "vsplit",
        [],
        "vsplit [path]",
        Optional(Path)
    ),
    editor_spec!(
        Editor::Save,
        "write",
        ["w", "save"],
        "write [path]",
        Optional(Path)
    ),
    editor_spec!(
        Editor::ForceSave,
        "write!",
        ["w!", "save!"],
        "write! [path]",
        Optional(Path)
    ),
    spec!(
        ColonId(Colon::WriteQuit),
        "write-quit",
        ["wq"],
        "write-quit",
        "Write the active buffer, then close its pane or quit if it is the last pane",
        NoArguments
    ),
    spec!(
        ColonId(Colon::WriteBufferClose),
        "write-buffer-close",
        ["wbc"],
        "write-buffer-close",
        "Write and close the active buffer without changing the pane layout",
        NoArguments
    ),
];

pub fn resolve_command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .iter()
        .find(|spec| spec.name == name || spec.aliases.contains(&name))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HelpInvocation {
    ActiveView,
    Manual(Option<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvocationParameters {
    None,
    PaneResize(i16),
    Path(PathBuf),
    OptionalPath(Option<PathBuf>),
    OptionalText(Option<String>),
    SessionRename { workspace: PathBuf, name: String },
    Grammar(Option<GrammarKind>),
    Help(HelpInvocation),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommandExecutionContext {
    count: Option<std::num::NonZeroUsize>,
    character: Option<char>,
}

impl CommandExecutionContext {
    pub const fn resolved(count: std::num::NonZeroUsize, character: Option<char>) -> Self {
        Self {
            count: Some(count),
            character,
        }
    }

    /// The explicitly supplied count. `None` is distinct from `Some(1)` for
    /// commands such as `gg`, where `1gg` addresses line one while bare `gg`
    /// addresses the file boundary.
    pub const fn count(self) -> Option<usize> {
        match self.count {
            Some(count) => Some(count.get()),
            None => None,
        }
    }

    pub const fn repetitions(self) -> usize {
        match self.count {
            Some(count) => count.get(),
            None => 1,
        }
    }

    pub const fn character(self) -> Option<char> {
        self.character
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandInvocation {
    id: CommandId,
    parameters: InvocationParameters,
    execution: CommandExecutionContext,
    unavailable: Option<CommandUnavailable>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandUnavailable {
    Planned(&'static str),
    Unsupported(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandInvocationError {
    MissingCharacter(EditorCommand),
    UnexpectedCharacter(EditorCommand),
    CountNotSupported(EditorCommand),
    HelpOriginRequired,
    InvalidParameters(CommandId),
}

impl fmt::Display for CommandInvocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCharacter(command) => {
                write!(
                    formatter,
                    "{} requires a character operand",
                    command.metadata().name
                )
            }
            Self::UnexpectedCharacter(command) => {
                write!(
                    formatter,
                    "{} does not take a character operand",
                    command.metadata().name
                )
            }
            Self::CountNotSupported(command) => {
                write!(
                    formatter,
                    "{} does not support a count",
                    command.metadata().name
                )
            }
            Self::HelpOriginRequired => {
                formatter.write_str("help invocation requires an explicit origin")
            }
            Self::InvalidParameters(id) => write!(formatter, "invalid parameters for {id:?}"),
        }
    }
}

impl std::error::Error for CommandInvocationError {}

impl CommandInvocation {
    fn new(id: CommandId, parameters: InvocationParameters) -> Self {
        Self {
            id,
            parameters,
            execution: CommandExecutionContext::default(),
            unavailable: None,
        }
    }

    /// Validated construction for headless hosts and future protocol
    /// frontends. Every inventory identity is accepted with its corresponding
    /// typed parameters, so callers never need to synthesize colon text.
    pub fn from_parts(
        id: CommandId,
        parameters: InvocationParameters,
        execution: CommandExecutionContext,
    ) -> Result<Self, CommandInvocationError> {
        let valid = match id {
            CommandId::Editor(EditorCommand::ShowHelp) => {
                matches!(parameters, InvocationParameters::Help(_))
                    && execution == CommandExecutionContext::default()
            }
            CommandId::Editor(EditorCommand::OpenThemeSettings) => {
                if matches!(parameters, InvocationParameters::OptionalText(_)) {
                    validate_editor_execution(EditorCommand::OpenThemeSettings, execution)?;
                    true
                } else {
                    false
                }
            }
            CommandId::Editor(
                EditorCommand::OpenWorkingDirectoryExplorer
                | EditorCommand::SplitVertical
                | EditorCommand::SplitHorizontal
                | EditorCommand::Save
                | EditorCommand::ForceSave,
            ) => {
                if matches!(parameters, InvocationParameters::OptionalPath(_)) {
                    let CommandId::Editor(command) = id else {
                        unreachable!()
                    };
                    validate_editor_execution(command, execution)?;
                    true
                } else {
                    false
                }
            }
            CommandId::Editor(command) => {
                if !matches!(parameters, InvocationParameters::None) {
                    false
                } else {
                    validate_editor_execution(command, execution)?;
                    true
                }
            }
            CommandId::Colon(command) => {
                if execution != CommandExecutionContext::default() {
                    false
                } else {
                    valid_colon_parameters(command, &parameters)
                }
            }
        };
        if !valid {
            return Err(CommandInvocationError::InvalidParameters(id));
        }
        Ok(Self {
            id,
            parameters,
            execution,
            unavailable: None,
        })
    }

    pub fn editor(
        command: EditorCommand,
        execution: CommandExecutionContext,
    ) -> Result<Self, CommandInvocationError> {
        if command == EditorCommand::ShowHelp {
            return Err(CommandInvocationError::HelpOriginRequired);
        }
        validate_editor_execution(command, execution)?;
        let parameters = match command {
            EditorCommand::OpenWorkingDirectoryExplorer
            | EditorCommand::SplitVertical
            | EditorCommand::SplitHorizontal
            | EditorCommand::Save
            | EditorCommand::ForceSave => InvocationParameters::OptionalPath(None),
            EditorCommand::OpenThemeSettings => InvocationParameters::OptionalText(None),
            _ => InvocationParameters::None,
        };
        Ok(Self {
            id: CommandId::Editor(command),
            parameters,
            execution,
            unavailable: None,
        })
    }

    /// Represents a registry binding that has a semantic identity but must
    /// not execute yet. The adapter supplies only the registry availability;
    /// `App::execute` owns the typed outcome and exact user-facing message.
    pub fn unavailable_editor(command: EditorCommand, unavailable: CommandUnavailable) -> Self {
        let parameters = match command {
            EditorCommand::ShowHelp => InvocationParameters::Help(HelpInvocation::ActiveView),
            EditorCommand::OpenWorkingDirectoryExplorer
            | EditorCommand::SplitVertical
            | EditorCommand::SplitHorizontal
            | EditorCommand::Save
            | EditorCommand::ForceSave => InvocationParameters::OptionalPath(None),
            _ => InvocationParameters::None,
        };
        Self {
            id: CommandId::Editor(command),
            parameters,
            execution: CommandExecutionContext::default(),
            unavailable: Some(unavailable),
        }
    }

    pub fn help(request: HelpInvocation) -> Self {
        Self::new(
            CommandId::Editor(EditorCommand::ShowHelp),
            InvocationParameters::Help(request),
        )
    }

    /// Builds the same semantic save invocation used by `:write`, without
    /// requiring a caller to manufacture command-line text.
    pub fn save(path: Option<PathBuf>) -> Self {
        Self::new(
            CommandId::Editor(EditorCommand::Save),
            InvocationParameters::OptionalPath(path),
        )
    }

    pub fn split_vertical(path: Option<PathBuf>) -> Self {
        Self::new(
            CommandId::Editor(EditorCommand::SplitVertical),
            InvocationParameters::OptionalPath(path),
        )
    }

    pub fn split_horizontal(path: Option<PathBuf>) -> Self {
        Self::new(
            CommandId::Editor(EditorCommand::SplitHorizontal),
            InvocationParameters::OptionalPath(path),
        )
    }

    pub fn open_explorer(path: Option<PathBuf>) -> Self {
        Self::new(
            CommandId::Editor(EditorCommand::OpenWorkingDirectoryExplorer),
            InvocationParameters::OptionalPath(path),
        )
    }

    pub fn open(path: PathBuf) -> Result<Self, CommandInvocationError> {
        Self::from_parts(
            CommandId::Colon(ColonCommand::Open),
            InvocationParameters::Path(path),
            CommandExecutionContext::default(),
        )
    }

    pub fn lsp_status() -> Self {
        Self::new(
            CommandId::Colon(ColonCommand::LspStatus),
            InvocationParameters::None,
        )
    }

    pub fn service_health() -> Self {
        Self::new(
            CommandId::Colon(ColonCommand::ServiceHealth),
            InvocationParameters::None,
        )
    }

    pub fn lsp_restart(language: Option<String>) -> Result<Self, CommandInvocationError> {
        Self::from_parts(
            CommandId::Colon(ColonCommand::LspRestart),
            InvocationParameters::OptionalText(language),
            CommandExecutionContext::default(),
        )
    }

    pub const fn id(&self) -> CommandId {
        self.id
    }

    pub const fn parameters(&self) -> &InvocationParameters {
        &self.parameters
    }

    pub const fn execution(&self) -> CommandExecutionContext {
        self.execution
    }

    pub fn into_parts(
        self,
    ) -> (
        CommandId,
        InvocationParameters,
        CommandExecutionContext,
        Option<CommandUnavailable>,
    ) {
        (self.id, self.parameters, self.execution, self.unavailable)
    }
}

fn valid_colon_parameters(command: ColonCommand, parameters: &InvocationParameters) -> bool {
    use ColonCommand as Colon;

    match (command, parameters) {
        (
            Colon::CloseBuffer
            | Colon::DiffOff
            | Colon::DiffThis
            | Colon::ForceCloseBuffer
            | Colon::Format
            | Colon::GitBranches
            | Colon::GitBlame
            | Colon::GitBlameFile
            | Colon::GitCancel
            | Colon::GitCommit
            | Colon::GitDiff
            | Colon::GitDiffSideBySide
            | Colon::GitDiscard
            | Colon::GitIndex
            | Colon::GitLog
            | Colon::GitSearchCommits
            | Colon::GitRefresh
            | Colon::GitStage
            | Colon::GitStatus
            | Colon::GitStashes
            | Colon::GitStashApply
            | Colon::GitStashDrop
            | Colon::GitStageHunk
            | Colon::GitUnstageHunk
            | Colon::GitStageLines
            | Colon::GitUnstage
            | Colon::GitWorktrees
            | Colon::LspStatus
            | Colon::Notifications
            | Colon::Path
            | Colon::ServiceHealth
            | Colon::Detach
            | Colon::Quit
            | Colon::ForceQuit
            | Colon::QuitAll
            | Colon::ForceQuitAll
            | Colon::QuitHere
            | Colon::ForceQuitHere
            | Colon::Reload
            | Colon::WriteQuit
            | Colon::WriteBufferClose
            | Colon::SessionList,
            InvocationParameters::None,
        ) => true,
        (
            Colon::GitStashTracked | Colon::GitStashAll | Colon::GitStashUntracked,
            InvocationParameters::OptionalText(value),
        ) => value.as_ref().is_some_and(|value| !value.trim().is_empty()),
        (
            Colon::ResizeRight | Colon::ResizeLeft | Colon::ResizeTop | Colon::ResizeBottom,
            InvocationParameters::PaneResize(delta),
        ) => *delta != 0,
        (
            Colon::ChangeDirectory | Colon::Open | Colon::SessionAttach,
            InvocationParameters::Path(path),
        ) => !path.as_os_str().is_empty(),
        (Colon::SessionStart | Colon::SessionStop, InvocationParameters::OptionalPath(_)) => true,
        (Colon::SessionRename, InvocationParameters::SessionRename { workspace, name }) => {
            !workspace.as_os_str().is_empty() && !name.trim().is_empty()
        }
        (Colon::LspRestart, InvocationParameters::OptionalText(value)) => {
            value.as_ref().is_none_or(|value| !value.is_empty())
        }
        (Colon::Grammar, InvocationParameters::Grammar(_)) => true,
        _ => false,
    }
}

fn validate_editor_execution(
    command: EditorCommand,
    execution: CommandExecutionContext,
) -> Result<(), CommandInvocationError> {
    match (command.takes_character(), execution.character()) {
        (true, None) => return Err(CommandInvocationError::MissingCharacter(command)),
        (false, Some(_)) => return Err(CommandInvocationError::UnexpectedCharacter(command)),
        _ => {}
    }
    if execution.count().is_some_and(|count| count > 1) && !command.accepts_count() {
        return Err(CommandInvocationError::CountNotSupported(command));
    }
    Ok(())
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandParseError {
    Empty,
    Unknown(String),
    MissingArgument(&'static str),
    UnexpectedArgument(&'static str),
    UnbalancedPathQuote(&'static str),
    TooManyArguments(&'static str),
    InvalidArgument {
        command: &'static str,
        value: String,
        expected: &'static str,
    },
    InvalidInventory(CommandId),
}

impl fmt::Display for CommandParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("empty command"),
            Self::Unknown(name) => write!(formatter, "unknown command: {name}"),
            Self::MissingArgument(name) => write!(formatter, "{name} requires an argument"),
            Self::UnexpectedArgument(name) => write!(formatter, "{name} does not take arguments"),
            Self::UnbalancedPathQuote(name) => {
                write!(formatter, "{name} has an unbalanced quoted path")
            }
            Self::TooManyArguments(name) => write!(formatter, "{name} has too many arguments"),
            Self::InvalidArgument {
                command,
                value,
                expected,
            } => write!(
                formatter,
                "{command} has invalid argument `{value}`; expected {expected}"
            ),
            Self::InvalidInventory(id) => {
                write!(formatter, "command inventory has no parser for {id:?}")
            }
        }
    }
}

impl std::error::Error for CommandParseError {}

enum ParsedArgument {
    None,
    Path(Option<PathBuf>),
    Text(Option<String>),
}

pub fn parse_colon_command(input: &str) -> Result<CommandInvocation, CommandParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(CommandParseError::Empty);
    }
    let split = input.find(char::is_whitespace).unwrap_or(input.len());
    let name = &input[..split];
    let remainder = input[split..].trim();
    parse_named_command(name, (!remainder.is_empty()).then_some(remainder))
}

/// Resolves one inventory identity with a separately framed argument.
///
/// Protocol frontends use this entry point so command identity is not encoded
/// as a line of colon-command text.
pub fn parse_named_command(
    name: &str,
    argument: Option<&str>,
) -> Result<CommandInvocation, CommandParseError> {
    if name.is_empty() {
        return Err(CommandParseError::Empty);
    }
    let spec = resolve_command(name).ok_or_else(|| CommandParseError::Unknown(name.to_owned()))?;
    let argument = parse_argument(spec, argument.unwrap_or_default())?;
    invocation_from_parts(spec.id, argument)
}

fn parse_argument(
    spec: &'static CommandSpec,
    value: &str,
) -> Result<ParsedArgument, CommandParseError> {
    match spec.arguments {
        CommandArguments::None if value.is_empty() => Ok(ParsedArgument::None),
        CommandArguments::None => Err(CommandParseError::UnexpectedArgument(spec.name)),
        CommandArguments::Required(_) if value.is_empty() => {
            Err(CommandParseError::MissingArgument(spec.name))
        }
        CommandArguments::Required(ArgumentKind::Path)
        | CommandArguments::Optional(ArgumentKind::Path) => {
            let value = (!value.is_empty())
                .then(|| parse_path_argument(spec.name, value))
                .transpose()?;
            Ok(ParsedArgument::Path(value))
        }
        CommandArguments::Required(ArgumentKind::FreeText)
        | CommandArguments::Optional(ArgumentKind::FreeText) => Ok(ParsedArgument::Text(
            (!value.is_empty()).then(|| value.to_owned()),
        )),
    }
}

fn parse_path_argument(command: &'static str, value: &str) -> Result<PathBuf, CommandParseError> {
    let Some(quote @ ('\'' | '"')) = value.chars().next() else {
        return Ok(PathBuf::from(value));
    };
    if value.len() < 2 || !value.ends_with(quote) {
        return Err(CommandParseError::UnbalancedPathQuote(command));
    }
    let inner = &value[quote.len_utf8()..value.len() - quote.len_utf8()];
    let mut unquoted = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            if character != quote && character != '\\' {
                unquoted.push('\\');
            }
            unquoted.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote {
            return Err(CommandParseError::UnbalancedPathQuote(command));
        } else {
            unquoted.push(character);
        }
    }
    if escaped {
        return Err(CommandParseError::UnbalancedPathQuote(command));
    }
    Ok(PathBuf::from(unquoted))
}

fn invocation_from_parts(
    id: CommandId,
    argument: ParsedArgument,
) -> Result<CommandInvocation, CommandParseError> {
    let invalid = || CommandParseError::InvalidInventory(id);
    match id {
        CommandId::Editor(command) => match (command, argument) {
            (EditorCommand::CloseWindow, ParsedArgument::None)
            | (EditorCommand::OpenFilePicker, ParsedArgument::None)
            | (EditorCommand::OpenDirectoryFilePicker, ParsedArgument::None)
            | (EditorCommand::OpenFuzzyGrep, ParsedArgument::None)
            | (EditorCommand::OpenDirectoryFuzzyGrep, ParsedArgument::None)
            | (EditorCommand::NewBuffer, ParsedArgument::None)
            | (EditorCommand::OpenSettings, ParsedArgument::None)
            | (EditorCommand::ExpandSyntaxSelection, ParsedArgument::None)
            | (EditorCommand::ShrinkSyntaxSelection, ParsedArgument::None)
            | (EditorCommand::SelectSyntaxParent, ParsedArgument::None)
            | (EditorCommand::SelectSyntaxChild, ParsedArgument::None)
            | (EditorCommand::SelectPreviousSyntaxSibling, ParsedArgument::None)
            | (EditorCommand::SelectNextSyntaxSibling, ParsedArgument::None)
            | (EditorCommand::SelectSyntaxFunction, ParsedArgument::None)
            | (EditorCommand::SelectInsideSyntaxFunction, ParsedArgument::None)
            | (EditorCommand::SelectSyntaxClass, ParsedArgument::None)
            | (EditorCommand::SelectInsideSyntaxClass, ParsedArgument::None)
            | (EditorCommand::SelectSyntaxParameter, ParsedArgument::None)
            | (EditorCommand::SelectInsideSyntaxParameter, ParsedArgument::None)
            | (EditorCommand::SelectAroundParentheses, ParsedArgument::None)
            | (EditorCommand::SelectInsideParentheses, ParsedArgument::None)
            | (EditorCommand::SelectAroundSquareBrackets, ParsedArgument::None)
            | (EditorCommand::SelectInsideSquareBrackets, ParsedArgument::None)
            | (EditorCommand::SelectAroundBraces, ParsedArgument::None)
            | (EditorCommand::SelectInsideBraces, ParsedArgument::None)
            | (EditorCommand::SelectAroundAngleBrackets, ParsedArgument::None)
            | (EditorCommand::SelectInsideAngleBrackets, ParsedArgument::None)
            | (EditorCommand::SelectAroundDoubleQuotes, ParsedArgument::None)
            | (EditorCommand::SelectInsideDoubleQuotes, ParsedArgument::None)
            | (EditorCommand::SelectAroundSingleQuotes, ParsedArgument::None)
            | (EditorCommand::SelectInsideSingleQuotes, ParsedArgument::None)
            | (EditorCommand::SelectAroundBackticks, ParsedArgument::None)
            | (EditorCommand::SelectInsideBackticks, ParsedArgument::None)
            | (EditorCommand::SelectAroundClosestDelimiter, ParsedArgument::None)
            | (EditorCommand::SelectInsideClosestDelimiter, ParsedArgument::None)
            | (EditorCommand::GotoPreviousSyntaxFunction, ParsedArgument::None)
            | (EditorCommand::GotoNextSyntaxFunction, ParsedArgument::None)
            | (EditorCommand::GotoPreviousSyntaxClass, ParsedArgument::None)
            | (EditorCommand::GotoNextSyntaxClass, ParsedArgument::None)
            | (EditorCommand::GotoPreviousSyntaxParameter, ParsedArgument::None)
            | (EditorCommand::GotoNextSyntaxParameter, ParsedArgument::None)
            | (EditorCommand::DocumentOutline, ParsedArgument::None)
            | (EditorCommand::ToggleSyntaxFold, ParsedArgument::None)
            | (EditorCommand::FoldAllSyntax, ParsedArgument::None)
            | (EditorCommand::UnfoldAllSyntax, ParsedArgument::None)
            | (EditorCommand::OpenTerminalList, ParsedArgument::None)
            | (EditorCommand::CopyTerminalOutput, ParsedArgument::None) => {
                Ok(CommandInvocation::new(id, InvocationParameters::None))
            }
            (EditorCommand::OpenWorkingDirectoryExplorer, ParsedArgument::Path(path)) => Ok(
                CommandInvocation::new(id, InvocationParameters::OptionalPath(path)),
            ),
            (EditorCommand::OpenThemeSettings, ParsedArgument::Text(name)) => Ok(
                CommandInvocation::new(id, InvocationParameters::OptionalText(name)),
            ),
            (EditorCommand::ShowHelp, ParsedArgument::Text(topic)) => {
                Ok(CommandInvocation::help(HelpInvocation::Manual(topic)))
            }
            (
                EditorCommand::ShowAbout
                | EditorCommand::ToggleZen
                | EditorCommand::ToggleFullscreen,
                ParsedArgument::None,
            ) => Ok(CommandInvocation::new(id, InvocationParameters::None)),
            (
                EditorCommand::OpenTerminal
                | EditorCommand::OpenTerminalFileDirectory
                | EditorCommand::OpenTerminalDirectoryRoot
                | EditorCommand::OpenTerminalSelectedDirectory
                | EditorCommand::OpenTerminalSessionDirectory,
                ParsedArgument::Text(command),
            ) => Ok(CommandInvocation::new(
                id,
                InvocationParameters::OptionalText(command),
            )),
            (
                EditorCommand::ShowTerminal
                | EditorCommand::RenameTerminal
                | EditorCommand::SendToTerminal,
                ParsedArgument::Text(value),
            ) => Ok(CommandInvocation::new(
                id,
                InvocationParameters::OptionalText(value),
            )),
            (
                EditorCommand::SplitHorizontal
                | EditorCommand::SplitVertical
                | EditorCommand::Save
                | EditorCommand::ForceSave,
                ParsedArgument::Path(path),
            ) => Ok(CommandInvocation::new(
                id,
                InvocationParameters::OptionalPath(path),
            )),
            _ => Err(invalid()),
        },
        CommandId::Colon(command) => match (command, argument) {
            (
                ColonCommand::ChangeDirectory | ColonCommand::SessionAttach,
                ParsedArgument::Path(Some(path)),
            ) => Ok(CommandInvocation::new(id, InvocationParameters::Path(path))),
            (
                ColonCommand::CloseBuffer
                | ColonCommand::DiffOff
                | ColonCommand::DiffThis
                | ColonCommand::ForceCloseBuffer
                | ColonCommand::Format
                | ColonCommand::GitBranches
                | ColonCommand::GitBlame
                | ColonCommand::GitBlameFile
                | ColonCommand::GitCancel
                | ColonCommand::GitCommit
                | ColonCommand::GitDiff
                | ColonCommand::GitDiffSideBySide
                | ColonCommand::GitDiscard
                | ColonCommand::GitIndex
                | ColonCommand::GitLog
                | ColonCommand::GitSearchCommits
                | ColonCommand::GitRefresh
                | ColonCommand::GitStage
                | ColonCommand::GitStatus
                | ColonCommand::GitStashes
                | ColonCommand::GitStashApply
                | ColonCommand::GitStashDrop
                | ColonCommand::GitStageHunk
                | ColonCommand::GitUnstageHunk
                | ColonCommand::GitStageLines
                | ColonCommand::GitUnstage
                | ColonCommand::GitWorktrees
                | ColonCommand::LspStatus
                | ColonCommand::Notifications
                | ColonCommand::Path
                | ColonCommand::ServiceHealth
                | ColonCommand::Detach
                | ColonCommand::Quit
                | ColonCommand::ForceQuit
                | ColonCommand::QuitAll
                | ColonCommand::ForceQuitAll
                | ColonCommand::QuitHere
                | ColonCommand::ForceQuitHere
                | ColonCommand::Reload
                | ColonCommand::WriteQuit
                | ColonCommand::WriteBufferClose
                | ColonCommand::SessionList,
                ParsedArgument::None,
            ) => Ok(CommandInvocation::new(id, InvocationParameters::None)),
            (
                ColonCommand::GitStashTracked
                | ColonCommand::GitStashAll
                | ColonCommand::GitStashUntracked,
                ParsedArgument::Text(value),
            ) => Ok(CommandInvocation::new(
                id,
                InvocationParameters::OptionalText(value),
            )),
            (ColonCommand::Open, ParsedArgument::Path(Some(path))) => {
                Ok(CommandInvocation::new(id, InvocationParameters::Path(path)))
            }
            (
                ColonCommand::SessionStart | ColonCommand::SessionStop,
                ParsedArgument::Path(path),
            ) => Ok(CommandInvocation::new(
                id,
                InvocationParameters::OptionalPath(path),
            )),
            (ColonCommand::SessionRename, ParsedArgument::Text(Some(value))) => {
                let (workspace, name) = parse_session_rename(command, &value)?;
                Ok(CommandInvocation::new(
                    id,
                    InvocationParameters::SessionRename { workspace, name },
                ))
            }
            (
                ColonCommand::ResizeRight
                | ColonCommand::ResizeLeft
                | ColonCommand::ResizeTop
                | ColonCommand::ResizeBottom,
                ParsedArgument::Text(Some(value)),
            ) => Ok(CommandInvocation::new(
                id,
                InvocationParameters::PaneResize(parse_pane_resize(command, &value)?),
            )),
            (ColonCommand::LspRestart, ParsedArgument::Text(value)) => Ok(CommandInvocation::new(
                id,
                InvocationParameters::OptionalText(value),
            )),
            (ColonCommand::Grammar, ParsedArgument::Text(value)) => {
                let grammar = value
                    .map(|value| {
                        value
                            .parse()
                            .map_err(|()| CommandParseError::InvalidArgument {
                                command: "grammar",
                                value,
                                expected: "runyte",
                            })
                    })
                    .transpose()?;
                Ok(CommandInvocation::new(
                    id,
                    InvocationParameters::Grammar(grammar),
                ))
            }
            _ => Err(invalid()),
        },
    }
}

fn parse_session_rename(
    command: ColonCommand,
    value: &str,
) -> Result<(PathBuf, String), CommandParseError> {
    let command_name = COMMANDS
        .iter()
        .find(|spec| spec.id == CommandId::Colon(command))
        .map(|spec| spec.name)
        .expect("session rename command is inventoried");
    let (selector, remainder) = if let Some(quote @ ('\'' | '"')) = value.chars().next() {
        let mut escaped = false;
        let mut closing = None;
        for (index, character) in value.char_indices().skip(1) {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == quote {
                closing = Some(index + character.len_utf8());
                break;
            }
        }
        let closing = closing.ok_or(CommandParseError::UnbalancedPathQuote(command_name))?;
        let selector = parse_path_argument(command_name, &value[..closing])?;
        let after_selector = &value[closing..];
        if after_selector
            .chars()
            .next()
            .is_some_and(|character| !character.is_whitespace())
        {
            return Err(CommandParseError::InvalidArgument {
                command: command_name,
                value: value.to_owned(),
                expected: "WORKSPACE NAME",
            });
        }
        (selector, after_selector.trim())
    } else {
        let split = value.find(char::is_whitespace).unwrap_or(value.len());
        (PathBuf::from(&value[..split]), value[split..].trim())
    };
    if selector.as_os_str().is_empty() || remainder.is_empty() {
        return Err(CommandParseError::InvalidArgument {
            command: command_name,
            value: value.to_owned(),
            expected: "WORKSPACE NAME",
        });
    }
    Ok((selector, remainder.to_owned()))
}

fn parse_pane_resize(command: ColonCommand, value: &str) -> Result<i16, CommandParseError> {
    let name = COMMANDS
        .iter()
        .find(|spec| spec.id == CommandId::Colon(command))
        .expect("pane resize commands belong to the inventory")
        .name;
    let compact = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let (sign, magnitude) = compact.split_at(compact.chars().next().map_or(0, char::len_utf8));
    let magnitude = magnitude.parse::<i16>().ok().filter(|value| *value > 0);
    match (sign, magnitude) {
        ("+", Some(magnitude)) => Ok(magnitude),
        ("-", Some(magnitude)) => Ok(-magnitude),
        _ => Err(CommandParseError::InvalidArgument {
            command: name,
            value: value.to_owned(),
            expected: "+ N or - N, where N is 1 through 32767",
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf};

    use super::*;

    #[test]
    fn syntax_category_and_capability_are_one_exhaustive_command_set() {
        let syntax_commands = [
            EditorCommand::ExpandSyntaxSelection,
            EditorCommand::ShrinkSyntaxSelection,
            EditorCommand::SelectSyntaxParent,
            EditorCommand::SelectSyntaxChild,
            EditorCommand::SelectPreviousSyntaxSibling,
            EditorCommand::SelectNextSyntaxSibling,
            EditorCommand::SelectSyntaxFunction,
            EditorCommand::SelectInsideSyntaxFunction,
            EditorCommand::SelectSyntaxClass,
            EditorCommand::SelectInsideSyntaxClass,
            EditorCommand::SelectSyntaxParameter,
            EditorCommand::SelectInsideSyntaxParameter,
            EditorCommand::SelectAroundParentheses,
            EditorCommand::SelectInsideParentheses,
            EditorCommand::SelectAroundSquareBrackets,
            EditorCommand::SelectInsideSquareBrackets,
            EditorCommand::SelectAroundBraces,
            EditorCommand::SelectInsideBraces,
            EditorCommand::SelectAroundAngleBrackets,
            EditorCommand::SelectInsideAngleBrackets,
            EditorCommand::SelectAroundDoubleQuotes,
            EditorCommand::SelectInsideDoubleQuotes,
            EditorCommand::SelectAroundSingleQuotes,
            EditorCommand::SelectInsideSingleQuotes,
            EditorCommand::SelectAroundBackticks,
            EditorCommand::SelectInsideBackticks,
            EditorCommand::SelectAroundClosestDelimiter,
            EditorCommand::SelectInsideClosestDelimiter,
            EditorCommand::GotoPreviousSyntaxFunction,
            EditorCommand::GotoNextSyntaxFunction,
            EditorCommand::GotoPreviousSyntaxClass,
            EditorCommand::GotoNextSyntaxClass,
            EditorCommand::GotoPreviousSyntaxParameter,
            EditorCommand::GotoNextSyntaxParameter,
            EditorCommand::DocumentOutline,
            EditorCommand::ToggleSyntaxFold,
            EditorCommand::FoldAllSyntax,
            EditorCommand::UnfoldAllSyntax,
        ];
        for command in EditorCommand::ALL {
            let expected = syntax_commands.contains(command);
            assert_eq!(command.category() == CommandCategory::Syntax, expected);
            assert_eq!(
                command.capability() == Some(CommandCapability::Syntax),
                expected
            );
            assert_eq!(command.metadata().capability, command.capability());
        }
        for spec in COMMANDS {
            assert_eq!(spec.capability(), spec.id.capability());
        }
    }

    #[test]
    fn command_metadata_names_are_unique_and_nonempty() {
        let mut names = HashSet::new();
        for command in EditorCommand::ALL {
            let metadata = command.metadata();
            assert!(!metadata.description.is_empty());
            assert!(names.insert(metadata.name));
        }
    }

    /// `ColonCommand::ALL` is the identity inventory used by exhaustive
    /// audits. A live palette command omitted here can evade every future
    /// check that begins from the enum rather than from the presentation
    /// table, so the two sets must stay identical.
    #[test]
    fn colon_command_inventory_matches_the_palette_registry() {
        let identities = ColonCommand::ALL.iter().copied().collect::<HashSet<_>>();
        assert_eq!(identities.len(), ColonCommand::ALL.len());

        let registered_commands = COMMANDS
            .iter()
            .filter_map(|spec| match spec.id {
                CommandId::Colon(command) => Some(command),
                CommandId::Editor(_) => None,
            })
            .collect::<Vec<_>>();
        let registered = registered_commands.iter().copied().collect::<HashSet<_>>();
        assert_eq!(registered.len(), registered_commands.len());
        assert_eq!(identities, registered);
        assert!(identities.contains(&ColonCommand::Path));
    }

    #[test]
    fn command_palette_spellings_are_globally_unique() {
        let mut spellings = HashSet::new();
        for spec in COMMANDS {
            for spelling in spec.names() {
                assert!(
                    spellings.insert(spelling),
                    "duplicate command-palette spelling: {spelling}"
                );
            }
        }
    }

    #[test]
    fn shared_editor_inventory_uses_the_semantic_command_description() {
        let mut shared = HashSet::new();
        for spec in COMMANDS {
            let CommandId::Editor(command) = spec.id else {
                continue;
            };
            assert!(
                shared.insert(command),
                "duplicate shared editor identity: {command:?}"
            );
            assert_eq!(
                spec.description,
                command.metadata().description,
                "description drift for shared editor identity {command:?}"
            );
        }
        assert!(
            !shared.is_empty(),
            "command inventory has no shared identities"
        );
    }

    #[test]
    fn settings_and_theme_commands_use_the_new_public_spellings() {
        assert!(
            COMMANDS
                .iter()
                .flat_map(CommandSpec::names)
                .all(|name| !name.starts_with("syntax-")),
            "structural commands must only be exposed through editor bindings"
        );
        assert!(matches!(
            parse_colon_command("syntax-expand"),
            Err(CommandParseError::Unknown(name)) if name == "syntax-expand"
        ));

        let session_list = resolve_command("session-list").unwrap();
        assert_eq!(session_list.aliases, &["sl"]);
        assert_eq!(resolve_command("sl").unwrap().id, session_list.id);
        for removed in [
            "workspace-list",
            "wls",
            "workspace-start",
            "workspace-stop",
            "wst",
            "workspace-attach",
            "wat",
        ] {
            assert!(
                resolve_command(removed).is_none(),
                "{removed} still resolves"
            );
        }

        let config = resolve_command("config").unwrap();
        assert_eq!(config.id, CommandId::Editor(EditorCommand::OpenSettings));
        assert_eq!(config.aliases, &["settings"]);
        assert_eq!(resolve_command("settings").unwrap().id, config.id);

        let theme = resolve_command("theme").unwrap();
        assert_eq!(
            theme.id,
            CommandId::Editor(EditorCommand::OpenThemeSettings)
        );
        assert_eq!(
            theme.arguments,
            CommandArguments::Optional(ArgumentKind::FreeText)
        );

        for retired in ["config-menu", "settings-theme", "theme-list"] {
            assert!(
                resolve_command(retired).is_none(),
                ":{retired} still resolves"
            );
        }
    }

    #[test]
    fn terminal_is_reachable_as_t_term_and_terminal() {
        let terminal = resolve_command("terminal").unwrap();
        assert_eq!(terminal.aliases, &["t", "term"]);
        for spelling in ["t", "term", "terminal"] {
            assert_eq!(
                resolve_command(spelling).unwrap().id,
                terminal.id,
                ":{spelling} must open a terminal"
            );
            assert_eq!(
                parse_colon_command(spelling),
                Ok(CommandInvocation::new(
                    CommandId::Editor(EditorCommand::OpenTerminal),
                    InvocationParameters::OptionalText(None)
                ))
            );
            assert_eq!(
                parse_colon_command(&format!("{spelling} htop")),
                Ok(CommandInvocation::new(
                    CommandId::Editor(EditorCommand::OpenTerminal),
                    InvocationParameters::OptionalText(Some("htop".to_owned()))
                ))
            );
        }
    }

    #[test]
    fn required_and_argumentless_commands_are_validated_from_the_schema() {
        for spec in COMMANDS {
            match spec.arguments {
                CommandArguments::Required(_) => assert_eq!(
                    parse_colon_command(spec.name),
                    Err(CommandParseError::MissingArgument(spec.name))
                ),
                CommandArguments::None => assert_eq!(
                    parse_colon_command(&format!("{} unexpected", spec.name)),
                    Err(CommandParseError::UnexpectedArgument(spec.name))
                ),
                CommandArguments::Optional(_) => {}
            }
        }
        assert_eq!(parse_colon_command("  "), Err(CommandParseError::Empty));
        assert_eq!(
            parse_colon_command("not-a-command"),
            Err(CommandParseError::Unknown("not-a-command".to_owned()))
        );
    }

    #[test]
    fn quoted_paths_are_balanced_unescaped_and_kept_as_one_argument() {
        assert_eq!(
            parse_colon_command(r#"open "folder with spaces/żółw.txt""#),
            Ok(CommandInvocation::new(
                CommandId::Colon(ColonCommand::Open),
                InvocationParameters::Path(PathBuf::from("folder with spaces/żółw.txt"))
            ))
        );
        assert_eq!(
            parse_colon_command(r#"write 'folder with spaces/note.txt'"#),
            Ok(CommandInvocation::save(Some(PathBuf::from(
                "folder with spaces/note.txt"
            ))))
        );
        assert_eq!(
            parse_colon_command(r#"open "folder/quote\"name.txt""#),
            Ok(CommandInvocation::new(
                CommandId::Colon(ColonCommand::Open),
                InvocationParameters::Path(PathBuf::from("folder/quote\"name.txt"))
            ))
        );
        assert_eq!(
            parse_colon_command(r#"open "unterminated"#),
            Err(CommandParseError::UnbalancedPathQuote("open"))
        );
    }

    #[test]
    fn pane_resize_commands_parse_signed_cell_counts() {
        for (source, expected) in [
            ("resize-right + 12", 12),
            ("resize-left -3", -3),
            ("resize-top +1", 1),
            ("resize-bottom - 7", -7),
        ] {
            assert_eq!(
                parse_colon_command(source).unwrap().parameters(),
                &InvocationParameters::PaneResize(expected)
            );
        }
        for invalid in [
            "resize-right 3",
            "resize-left +0",
            "resize-top --2",
            "resize-bottom + 40000",
        ] {
            assert!(matches!(
                parse_colon_command(invalid),
                Err(CommandParseError::InvalidArgument { .. })
            ));
        }
    }

    #[test]
    fn session_commands_parse_optional_workspaces_and_typed_renames() {
        assert_eq!(
            parse_colon_command("session-start").unwrap().parameters(),
            &InvocationParameters::OptionalPath(None)
        );
        assert_eq!(
            parse_colon_command("session-stop api")
                .unwrap()
                .parameters(),
            &InvocationParameters::OptionalPath(Some(PathBuf::from("api")))
        );
        assert_eq!(
            parse_colon_command("session-rename api Backend API")
                .unwrap()
                .parameters(),
            &InvocationParameters::SessionRename {
                workspace: PathBuf::from("api"),
                name: "Backend API".to_owned(),
            }
        );
        assert_eq!(
            parse_colon_command(r#"session-rename "old session" New Name"#)
                .unwrap()
                .parameters(),
            &InvocationParameters::SessionRename {
                workspace: PathBuf::from("old session"),
                name: "New Name".to_owned(),
            }
        );
        assert!(matches!(
            parse_colon_command("session-rename only-one-part"),
            Err(CommandParseError::InvalidArgument {
                command: "session-rename",
                expected: "WORKSPACE NAME",
                ..
            })
        ));
        assert!(matches!(
            parse_colon_command(r#"session-rename "old session"New Name"#),
            Err(CommandParseError::InvalidArgument {
                command: "session-rename",
                expected: "WORKSPACE NAME",
                ..
            })
        ));
    }

    #[test]
    fn shared_commands_have_one_canonical_invocation_shape() {
        let direct_save = CommandInvocation::save(None);
        let editor_save =
            CommandInvocation::editor(EditorCommand::Save, CommandExecutionContext::default())
                .unwrap();
        assert_eq!(parse_colon_command("write"), Ok(direct_save.clone()));
        assert_eq!(editor_save, direct_save);
        assert_eq!(
            parse_colon_command("vsplit folder/note.txt"),
            Ok(CommandInvocation::split_vertical(Some(PathBuf::from(
                "folder/note.txt"
            ))))
        );
        assert_eq!(
            parse_colon_command("explorer folder"),
            Ok(CommandInvocation::open_explorer(Some(PathBuf::from(
                "folder"
            ))))
        );
    }

    #[test]
    fn short_write_and_scratch_buffer_spellings_resolve_to_the_shared_identity() {
        for name in ["buffer-new", "new"] {
            assert_eq!(
                parse_colon_command(name),
                Ok(CommandInvocation::editor(
                    EditorCommand::NewBuffer,
                    CommandExecutionContext::default()
                )
                .unwrap()),
                ":{name} must open a scratch buffer"
            );
            assert_eq!(resolve_command(name).unwrap().name, "buffer-new");
        }
        for name in ["write", "w", "save"] {
            assert_eq!(
                parse_colon_command(name),
                Ok(CommandInvocation::save(None)),
                ":{name} must write the active buffer"
            );
            assert_eq!(resolve_command(name).unwrap().name, "write");
        }
        for name in ["write!", "w!", "save!"] {
            assert_eq!(
                parse_colon_command(&format!("{name} note.txt")),
                Ok(CommandInvocation::new(
                    CommandId::Editor(EditorCommand::ForceSave),
                    InvocationParameters::OptionalPath(Some(PathBuf::from("note.txt")))
                )),
                ":{name} must force-write the given path"
            );
            assert_eq!(resolve_command(name).unwrap().name, "write!");
        }
    }

    #[test]
    fn execution_context_resolves_character_operands_and_command_counts() {
        let three = std::num::NonZeroUsize::new(3).unwrap();
        let one = std::num::NonZeroUsize::MIN;
        let find = CommandExecutionContext::resolved(one, Some('x'));
        assert!(CommandInvocation::editor(EditorCommand::FindNextChar, find).is_ok());
        assert_eq!(
            CommandInvocation::editor(
                EditorCommand::FindTillPreviousChar,
                CommandExecutionContext::resolved(three, Some('x'))
            ),
            Err(CommandInvocationError::CountNotSupported(
                EditorCommand::FindTillPreviousChar
            ))
        );

        let line = CommandExecutionContext::resolved(three, None);
        assert!(CommandInvocation::editor(EditorCommand::MoveFileStart, line).is_ok());
        assert!(CommandInvocation::editor(EditorCommand::MoveFileEnd, line).is_ok());
        assert_eq!(
            CommandInvocation::editor(
                EditorCommand::FindPreviousChar,
                CommandExecutionContext::default()
            ),
            Err(CommandInvocationError::MissingCharacter(
                EditorCommand::FindPreviousChar
            ))
        );
        assert_eq!(
            CommandInvocation::editor(EditorCommand::Save, line),
            Err(CommandInvocationError::CountNotSupported(
                EditorCommand::Save
            ))
        );
        assert_eq!(
            CommandInvocation::editor(
                EditorCommand::MoveRight,
                CommandExecutionContext::resolved(three, Some('x'))
            ),
            Err(CommandInvocationError::UnexpectedCharacter(
                EditorCommand::MoveRight
            ))
        );
    }

    #[test]
    fn help_invocations_distinguish_context_from_manual_topics() {
        let help = parse_colon_command("help").unwrap();
        let question_mark = parse_colon_command("?").unwrap();
        assert_eq!(help, question_mark, ":help and :? must be exact aliases");
        assert_eq!(
            help.parameters(),
            &InvocationParameters::Help(HelpInvocation::Manual(None))
        );
        assert_eq!(
            parse_colon_command("help regex").unwrap().parameters(),
            &InvocationParameters::Help(HelpInvocation::Manual(Some("regex".to_owned())))
        );
        assert_eq!(
            CommandInvocation::help(HelpInvocation::ActiveView).parameters(),
            &InvocationParameters::Help(HelpInvocation::ActiveView)
        );
        assert_eq!(
            CommandInvocation::editor(EditorCommand::ShowHelp, CommandExecutionContext::default()),
            Err(CommandInvocationError::HelpOriginRequired)
        );
    }

    #[test]
    fn grammar_command_parses_typed_choices_and_rejects_unknown_names() {
        assert_eq!(
            parse_colon_command("grammar helix").unwrap().parameters(),
            &InvocationParameters::Grammar(Some(GrammarKind::Runyte))
        );
        assert_eq!(
            parse_colon_command("grammar").unwrap().parameters(),
            &InvocationParameters::Grammar(None)
        );
        assert!(matches!(
            parse_colon_command("grammar vim"),
            Err(CommandParseError::InvalidArgument {
                command: "grammar",
                ..
            })
        ));
        assert!(matches!(
            parse_colon_command("grammar emacs"),
            Err(CommandParseError::InvalidArgument {
                command: "grammar",
                ..
            })
        ));
    }
}
