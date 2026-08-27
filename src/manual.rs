// SPDX-License-Identifier: MPL-2.0

//! The general Runyte manual opened by `:help`.
//!
//! Contextual help answers what the active view can do and derives its key
//! rows from the keymap registry. This module answers a different question:
//! how editor concepts fit together. It is one ordinary read-only document so
//! its own movement, search, splits, and jump history remain useful.

use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManualTopic {
    GettingStarted,
    Editing,
    Search,
    Regex,
    FilesAndBuffers,
    Workspace,
    Git,
    LanguageServers,
    Configuration,
    Commands,
}

impl ManualTopic {
    pub const ALL: &'static [Self] = &[
        Self::GettingStarted,
        Self::Editing,
        Self::Search,
        Self::Regex,
        Self::FilesAndBuffers,
        Self::Workspace,
        Self::Git,
        Self::LanguageServers,
        Self::Configuration,
        Self::Commands,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::GettingStarted => "getting-started",
            Self::Editing => "editing",
            Self::Search => "search",
            Self::Regex => "regex",
            Self::FilesAndBuffers => "files-and-buffers",
            Self::Workspace => "workspace",
            Self::Git => "git",
            Self::LanguageServers => "language-servers",
            Self::Configuration => "configuration",
            Self::Commands => "commands",
        }
    }

    const fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::GettingStarted => &["start", "getting-started"],
            Self::Editing => &["edit", "editing", "selections"],
            Self::Search => &["search"],
            Self::Regex => &["regex", "regexp", "regular-expressions"],
            Self::FilesAndBuffers => &["files", "buffers", "files-and-buffers", "panes"],
            Self::Workspace => &["workspace", "workspace-search"],
            Self::Git => &["git"],
            Self::LanguageServers => &["lsp", "language-server", "language-servers"],
            Self::Configuration => &["config", "configuration", "settings"],
            Self::Commands => &["command", "commands", "palette"],
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::GettingStarted => "Getting started",
            Self::Editing => "Editing and selections",
            Self::Search => "Search",
            Self::Regex => "Regular expressions",
            Self::FilesAndBuffers => "Files, buffers, and panes",
            Self::Workspace => "Workspace search",
            Self::Git => "Git",
            Self::LanguageServers => "Language servers",
            Self::Configuration => "Configuration",
            Self::Commands => "Commands",
        }
    }

    fn heading(self) -> String {
        self.title().to_uppercase()
    }

    const fn body(self) -> &'static str {
        match self {
            Self::GettingStarted => {
                "Runyte is a selection-first modal editor: establish one or more selections, then act on all of them. NORMAL movement replaces the current selection; v enters SELECT mode, where movement extends it. Escape returns to NORMAL mode.\n\nPress Space and pause to explore command groups. Space ? describes the view under the cursor and generates its key rows from the active keymap. The general manual you are reading is opened by :help, and :help <topic> jumps directly to a section.\n\nUse Space f to find project files, open buffers, and terminals, Space e to explore the active directory, Space b b to manage open buffers, and : to search every command by name, alias, category, or description."
            }
            Self::Editing => {
                "Every editing command operates on every selected range. A motion in NORMAL mode replaces each range; v enters SELECT mode so subsequent motions extend them. Multiple ranges are ordinary editor state rather than a special mode, so one change, delete, yank, or paste applies everywhere.\n\nSearch is the quickest way to create a useful multi-selection. An initial search selects every match; editing immediately changes them all. Press n or N first when the intended edit belongs to only one result.\n\nAll buffer mutations are transactions. Undo and redo therefore restore text and selections together rather than replaying direct writes."
            }
            Self::Search => {
                "Runyte has two search flavours. s searches for a case-insensitive literal and / interprets the prompt as a regular expression. Neither takes a namespaced spelling: two keys are already the short spelling. Write (?-i) in a regular expression when a search has to match case.\n\nA search selects every non-overlapping match at once. n and N reduce that result to one selection and cycle forward or backward through the remembered matches.\n\nWith at least two characters selected, a new search is confined to the selected spans. Successive searches therefore narrow. Collapse to a bare caret with ; before searching when the whole buffer should be considered again.\n\nSpace / widens the same letters to the whole project: Space / s and Space / / are the workspace versions of s and /, Space / g fuzzy-searches file contents, and Space / f opens the project finder. Tab switches that finder between files and open buffers plus terminals."
            }
            Self::Regex => {
                "Runyte passes regex queries directly to Rust's regex engine. The opening / is a Runyte key that opens the prompt, not a delimiter around a /pattern/flags expression. Write (?i)hello, not /hello/i, for a case-insensitive match.\n\nCommon syntax\n  .             any character except newline\n  [abc] [^abc]  character classes\n  x|y           alternation\n  (x) (?:x)     capturing and non-capturing groups\n  * + ? {n,m}   repetition; append ? for lazy repetition\n  ^ $ \\A \\z   line/input anchors\n  \\b \\B       word-boundary assertions\n  \\d \\s \\w   Unicode-aware shorthand classes\n  \\p{Greek}    Unicode property or script\n\nInline flags\n  (?i)  case-insensitive\n  (?m)  ^ and $ match line boundaries\n  (?s)  . also matches newline\n  (?R)  CRLF-aware line boundaries when multiline is enabled\n  (?U)  swap greedy and lazy repetition\n  (?u)  Unicode mode; enabled by default\n  (?x)  verbose mode with insignificant whitespace and # comments\n\nFlags may be scoped, as in (?i:hello), or disabled later, as in (?i)hello(?-i:WORLD). Slash-delimited expressions, trailing flags, look-around, and backreferences are not supported. Capturing groups are accepted, but Runyte selects only the complete match.\n\nBare / searches the complete buffer as one string, so (?s)foo.*bar or an explicit \\n may span lines. Space / / searches each workspace file one line at a time, so a workspace result never spans lines."
            }
            Self::FilesAndBuffers => {
                "A buffer owns text and editor history; a pane is one view onto a buffer. Splitting creates panes, while buffer switching retargets the active pane without duplicating the buffer. Space b b manages open buffers, previews their authoritative text with Ctrl-t, and opens contextual actions with Tab. Space w opens pane commands. :close / :c closes a buffer in place; :quit / :q closes a pane and a buffer displayed only there.\n\nSpace f opens the project finder in file mode; Tab switches to open buffers and terminals without clearing the query, and Ctrl-t toggles the selected file, buffer, or terminal preview in either mode. Explorer rows read [explorer] dirname plus their relative path. :file-picker-directory searches only files beside the active file or explorer. Space e opens the active directory as an editable explorer whose text becomes a reviewed filesystem plan only when written.\n\nRead-only generated pages, including this manual, remain ordinary searchable and splittable buffers. Runyte retains the two most recently active clean special buffers for Ctrl-o/Ctrl-i and Alt-o/Alt-i; activating a third retires the least recent detached one. A dirty special buffer remains discoverable. Empty clean scratch buffers still retire immediately after their last pane leaves. The general manual and contextual help both give q a scoped close binding."
            }
            Self::Workspace => {
                "Space / s searches workspace text as a case-insensitive literal and Space / / searches with a regular expression. Results open one retained [workspace search] special buffer.\n\nWorkspace results are a query-time snapshot. Enter follows the typed file, line, and column represented by a result row; the clean result buffer remains available while it is among the two most recently active special buffers. Unsaved open buffers are authoritative over their on-disk files, but rerunning the command is required to refresh the result set.\n\nWorkspace matching is line-scoped, reads UTF-8 text files no larger than 4 MiB, skips symlinks and internal directories, respects the hidden-file setting, and retains at most 10,000 results."
            }
            Self::Git => {
                "Space g opens Git navigation and refresh commands. The changed-file list, branches, worktrees, log, blame, stashes, and diffs are typed editor views rather than terminal output. Open Space ? in any of those views for its exact row actions and keys.\n\nGit reads and mutations run through Runyte's Git service boundary. Mutations are ordered per repository, and stale results are rejected rather than applied to newer editor or repository state. Cancellation stops waiting and reconciles state; it does not claim rollback.\n\nThe changed-file list distinguishes staged content from unstaged working-tree content. A commit takes the index shown by its Staged section, and writing the generated commit-message buffer performs the commit."
            }
            Self::LanguageServers => {
                "rust-analyzer is configured automatically. To add another language, first install its server executable so it is on PATH, then add the language directly below lsp in ~/.config/runyte/config.yaml (or the path passed with --config):\n\nlsp:\n  markdown:\n    command: marksman\n    args: [\"server\"]\n\nThe language key is Runyte's built-in language name. command is an executable name or absolute path and args is the argument list passed to it; Runyte launches the process directly, without a shell. The older lsp.servers.<language> spelling is still accepted, but servers is only a redundant compatibility wrapper. Server definitions are edited in YAML; Space o o can enable or disable LSP as a whole. Exit and reopen standalone Runyte after changing the file. A persistent session retains its host's loaded configuration, so use runyte --session-restart [WORKSPACE] and repeat any non-default --config PATH.\n\nLanguage-server commands live under Space l. Definitions, references, implementations, symbols, diagnostics, code actions, hover documentation, completion, formatting, rename, and signature help appear only when the active language and server provide them.\n\nUse :lsp-status to inspect servers that have started or failed, :lsp-restart [language] to restart one from the loaded configuration, and :service-health to check whether the active document has a configured and attached server. Launch failures appear in :lsp-status after the first attempt and in notifications. Source and package checkouts include copy-ready configurations under docs/lsp/."
            }
            Self::Configuration => {
                "Runyte reads YAML configuration from its platform configuration path. :settings opens the typed setting registry as a read-only buffer; Enter on a row opens the appropriate finite-choice or validated-value prompt. :theme opens theme choices directly.\n\nThe settings page distinguishes changes that apply immediately from those that require restart. Accepted changes are persisted without replacing unrelated YAML settings and comments.\n\nThe active editing grammar is Runyte. The accepted helix configuration spelling is only a compatibility alias for that same grammar, not a claim of complete Helix compatibility."
            }
            Self::Commands => {
                "Press : to open the command palette. It searches canonical names, aliases, usage, descriptions, and categories. Up and Down choose a row, Tab completes it, Enter runs it, and Escape closes the palette.\n\nCommands whose current service is unavailable remain discoverable with the reason they cannot run. Path-valued commands show bounded filesystem hints after their command name.\n\nUse :help <topic> to return to a manual section, :about for Runyte's compact front page, :notifications for retained feedback, and Space ? for keys and behavior specific to the active view."
            }
        }
    }

    pub fn resolve(value: &str) -> Option<Self> {
        let normalized = value.trim().to_lowercase().replace([' ', '_'], "-");
        Self::ALL
            .iter()
            .copied()
            .find(|topic| topic.aliases().contains(&normalized.as_str()))
    }
}

pub fn available_topics() -> String {
    ManualTopic::ALL
        .iter()
        .map(|topic| topic.slug())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn render() -> String {
    let mut out = String::from(
        "Help · RUNYTE\n\n\
         This is the general Runyte manual. Use Space ? for contextual help about\n\
         the active view and its registry-backed keys. Use :help <topic> to return\n\
         directly to one of the sections below.\n\n\
         Topics\n",
    );
    for topic in ManualTopic::ALL {
        let _ = writeln!(out, "  {:<20} :help {}", topic.title(), topic.slug());
    }
    for topic in ManualTopic::ALL {
        let _ = write!(out, "\n{}\n\n{}\n", topic.heading(), topic.body());
    }
    out
}

pub fn topic_offset(text: &str, topic: ManualTopic) -> usize {
    let needle = format!("\n{}\n", topic.heading());
    let byte = text
        .find(&needle)
        .map(|byte| byte + 1)
        .expect("every registered manual topic is rendered");
    text[..byte].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_is_indexed_resolvable_and_has_a_unique_heading() {
        let rendered = render();
        for topic in ManualTopic::ALL {
            assert_eq!(ManualTopic::resolve(topic.slug()), Some(*topic));
            assert!(rendered.contains(&format!(":help {}", topic.slug())));
            let heading = format!("\n{}\n", topic.heading());
            assert_eq!(rendered.matches(&heading).count(), 1, "{topic:?}");
            assert_eq!(
                rendered
                    .chars()
                    .skip(topic_offset(&rendered, *topic))
                    .take(topic.heading().chars().count())
                    .collect::<String>(),
                topic.heading()
            );
        }
    }

    #[test]
    fn regex_topic_explains_runyte_delimiters_flags_and_search_scope() {
        let rendered = render();
        let regex = rendered
            .chars()
            .skip(topic_offset(&rendered, ManualTopic::Regex))
            .collect::<String>();
        let regex = regex.split("\nFILES, BUFFERS, AND PANES\n").next().unwrap();
        for required in [
            "(?i)hello, not /hello/i",
            "look-around",
            "backreferences",
            "Bare / searches the complete buffer",
            "Space / / searches each workspace file one line at a time",
        ] {
            assert!(regex.contains(required), "missing {required:?}");
        }
    }

    #[test]
    fn language_server_topic_contains_a_complete_flat_setup_example() {
        let rendered = render();
        let lsp = rendered
            .chars()
            .skip(topic_offset(&rendered, ManualTopic::LanguageServers))
            .collect::<String>();
        let lsp = lsp.split("\nCONFIGURATION\n").next().unwrap();

        for required in [
            "rust-analyzer is configured automatically",
            "lsp:\n  markdown:\n    command: marksman\n    args: [\"server\"]",
            "older lsp.servers.<language> spelling is still accepted",
            ":lsp-status",
            ":lsp-restart [language]",
            ":service-health",
        ] {
            assert!(lsp.contains(required), "missing {required:?}");
        }
    }
}
