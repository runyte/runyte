// SPDX-License-Identifier: MPL-2.0

//! Contextual help for the view under the cursor.
//!
//! Editor key tables come from the keymap registry at render time, so help
//! follows dispatch and configured bindings. Command prompts handle their own
//! input outside that registry and are described as such.
//!
//! The rendered document carries semantic colour spans over plain text and is
//! opened as an ordinary read-only buffer. That is what makes help searchable,
//! scrollable, and splittable without this module knowing anything about
//! drawing.

use std::{fmt::Write as _, ops::Range};

use crate::{
    command::{EditorCommand, GrammarKind, Mode},
    help_document::{HelpDocument, HelpDocumentWriter, HelpRole},
    input::{KeyCode, Modifiers},
    keymap::{BindingScope, BindingTarget, Key, KeySequence, Keymap, Lookup},
};

/// Width of the key column in a rendered section.
const KEY_COLUMN: usize = 12;

/// The orientation text the help window opens with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpTopic {
    Text,
    Explorer,
    Config,
    Notifications,
    GitStatus,
    GitBranches,
    GitWorktrees,
    GitLog,
    GitBlame,
    GitStash,
    WorkspaceSearch,
    CommitMessage,
    Diff,
    Terminal,
}

impl HelpTopic {
    pub const ALL: &'static [Self] = &[
        Self::Text,
        Self::Explorer,
        Self::Config,
        Self::Notifications,
        Self::GitStatus,
        Self::GitBranches,
        Self::GitWorktrees,
        Self::GitLog,
        Self::GitBlame,
        Self::GitStash,
        Self::WorkspaceSearch,
        Self::CommitMessage,
        Self::Diff,
        Self::Terminal,
    ];

    /// The topic for a view.
    ///
    /// Deliberately not a function of the mode. Normal and Select share every
    /// binding, so splitting help along them produced two documents whose key
    /// tables were identical and whose prose each omitted half the answer.
    /// One document per buffer type describes both.
    pub fn for_context(scope: BindingScope) -> Self {
        match scope {
            BindingScope::Directory => Self::Explorer,
            BindingScope::Settings => Self::Config,
            BindingScope::GitStatus => Self::GitStatus,
            BindingScope::GitBranches => Self::GitBranches,
            BindingScope::GitWorktrees => Self::GitWorktrees,
            BindingScope::GitLog => Self::GitLog,
            BindingScope::GitBlame => Self::GitBlame,
            BindingScope::GitStash => Self::GitStash,
            BindingScope::WorkspaceSearch => Self::WorkspaceSearch,
            BindingScope::CommitMessage => Self::CommitMessage,
            BindingScope::Diff => Self::Diff,
            BindingScope::Terminal => Self::Terminal,
            // Help describes the view it was opened from. Opening it again
            // from inside itself falls back to the editor overview rather than
            // documenting the help buffer, which the `q` row already covers.
            //
            // A Markdown document is ordinary text with one key of its own, so
            // it reads the text overview too; the key table below that prose is
            // generated from the scope and carries the extra row.
            BindingScope::Help
            | BindingScope::Global
            | BindingScope::Markdown
            | BindingScope::Plugin(_) => Self::Text,
        }
    }

    /// The document's title line.
    ///
    /// Read-only-ness is part of the title rather than only a sentence in the
    /// prose, so it is stated in the same place for every buffer type and
    /// matches the `[RO]` the pane title and global status line carry.
    pub fn title_for(self, _grammar: GrammarKind, read_only: bool) -> String {
        let context = match self {
            Self::Text => "TEXT",
            Self::Explorer => "EXPLORER",
            Self::Config => "CONFIG",
            Self::Notifications => "NOTIFICATIONS",
            Self::GitStatus => "GIT STATUS",
            Self::GitBranches => "GIT BRANCHES",
            Self::GitWorktrees => "GIT WORKTREES",
            Self::GitLog => "GIT LOG",
            Self::GitBlame => "GIT BLAME",
            Self::GitStash => "GIT STASHES",
            Self::WorkspaceSearch => "WORKSPACE SEARCH",
            Self::CommitMessage => "COMMIT MESSAGE",
            Self::Diff => "DIFF",
            Self::Terminal => "TERMINAL",
        };
        let access = if read_only { " · Read-only" } else { "" };
        format!(" Help · RUNYTE · {context}{access} ")
    }

    /// Paragraphs shown before the action list. Each entry is wrapped by the
    /// renderer, so these strings describe ideas rather than terminal rows.
    pub fn overview_for(self, _grammar: GrammarKind) -> &'static [&'static str] {
        match self {
            Self::Text => &[
                "Runyte is a selection-first modal editor: move to select, then act. Every editing command works on whatever is selected, however many ranges that is.",
                "NORMAL mode replaces the selection as you move. v enters SELECT mode, where moving extends every selection instead; v or Escape returns.",
                "INSERT writes text at each caret. REPLACE overwrites forward and lets Backspace retrace the current overwrite. Their shared bindings and any mode-specific keys appear below.",
                "COMMAND owns the interaction-line prompt. Type to edit a command, or use its prompt controls listed below; it returns to editing when submitted or cancelled.",
                "Search selects every match at once, and highlights them while the pattern is still being typed. With two or more characters selected, s and / search only inside the selection, leaving a cursor on every match; n and N then select only one result and step through them.",
                "Press {prefix:Space} and pause to explore command groups without memorising the full keymap.",
                "{key:swap-window} exchanges this pane's complete content with the previously focused pane and follows it to its new position; {key:swap-window:compatibility} is the compatibility spelling.",
                "{binding:Space n} opens Navigator over open buffers and running terminals. Enter focuses a visible destination; Tab offers actions including bringing it into this pane. {binding:Ctrl-w p} returns to this pane's previous destination.",
                "An ordinary file changed outside Runyte keeps its in-memory text and gains [STALE]. {binding:Space b d} compares a fresh disk snapshot without discarding edits; {binding:Space r} reloads, asking first whenever the buffer is dirty.",
                "Below the editor area, the global status line reports editor state and unread notification counts. The interaction line below it is reserved for active prompts and the last action echo; :notifications or :not opens complete retained feedback.",
                ":service-health describes optional services right now, and :log-open opens the durable diagnostic log of the process that owns this workspace. Start Runyte with -v, -vv, or -vvv for more detail in it; :help diagnostics explains the rest.",
            ],
            Self::Explorer => &[
                "The explorer is an editable directory listing. Move and edit here just as you do in a text buffer.",
                "Tab t opens an integrated terminal in this pane, starting in the directory this explorer shows. The explorer stays available behind the terminal.",
                "Tab f opens the Finder rooted at the directory this explorer shows rather than at the project root, and lists dotfiles exactly when this listing does.",
                "Tab e opens the directory this explorer shows in the system file manager, preserving unapplied edits in Runyte.",
                "Edits do not touch the filesystem until you review and confirm the write plan.",
                "A symlink carries a muted → target hint that is not part of the text. Enter opens what the link points at; renaming and deleting stay with the link.",
                "An explorer whose directory changed outside Runyte gains [STALE] and keeps its rows, selections, and unsaved edits. {binding:Space r} re-reads the directory and clears it.",
                "{binding:Tab} offers what the explorer can be asked to show: dotfiles, file details, and the order rows are listed in. Each is a setting, so a choice is saved and every open explorer follows it. {binding:.} and {binding:?} toggle the first two directly.",
            ],
            Self::Config => &[
                "The config page is a read-only view of the setting registry. Search, select, split, and move through it like any other text buffer; Enter changes the setting on the current row.",
                "Edits made to the configuration file outside this page reach the running editor through :config-reload. Nothing is watched, an unusable file leaves the running configuration alone, and a setting read at startup is named as needing a restart rather than shown as already in effect.",
            ],
            Self::Notifications => &[
                "The notification center is a single read-only, searchable history page. Newest notifications appear first with their local timestamp, Runyte-assigned severity, source, and complete details.",
                "Opening :notifications or :not acknowledges everything currently retained. Later notifications stay unread and update the global status line without taking focus or replacing the interaction line's prompt or action echo.",
            ],
            Self::GitStatus => GIT_STATUS_OVERVIEW,
            Self::GitBranches => GIT_BRANCHES_OVERVIEW,
            Self::GitWorktrees => GIT_WORKTREES_OVERVIEW,
            Self::GitLog => GIT_LOG_OVERVIEW,
            Self::GitBlame => GIT_BLAME_OVERVIEW,
            Self::GitStash => GIT_STASH_OVERVIEW,
            Self::WorkspaceSearch => &[
                "Workspace search results are a retained query snapshot. Move, select, search, split, and copy from this buffer like any other read-only document.",
                "Enter opens the typed path and source range represented by the current result row. The clean result remains available while it is among the eight most recently active special buffers; run workspace search again for fresh results.",
            ],
            Self::CommitMessage => COMMIT_MESSAGE_OVERVIEW,
            Self::Diff => DIFF_OVERVIEW,
            Self::Terminal => TERMINAL_OVERVIEW,
        }
    }

    /// Rows written out verbatim between the overview and the key tables.
    ///
    /// Unlike `overview_for`, these strings *are* terminal rows: they are
    /// aligned against one another and must not be re-flowed, which is why
    /// they are a separate list rather than another paragraph. Reserved for a
    /// comparison prose genuinely loses — commands differing along two axes at
    /// once, where a sentence makes the reader hold one axis in their head
    /// while reading the other.
    pub fn table_for(self, _grammar: GrammarKind) -> &'static [&'static str] {
        match self {
            Self::GitStash => GIT_STASH_CREATION_TABLE,
            _ => &[],
        }
    }
}

/// Writing this buffer commits. That is the one thing about it a reader
/// cannot guess from anywhere else, so it comes first.
const COMMIT_MESSAGE_OVERVIEW: &[&str] = &[
    "This is the commit message. Writing it with `:write` or `:write-quit` makes the commit; there is no separate confirmation step and neither command exits Runyte from this special buffer.",
    "The commit takes the index — whatever the Staged section of the changed-file list showed, not what is on disk now.",
    "`:c` abandons an unchanged message; `:c!` discards an edited one. Closing the buffer without writing commits nothing and leaves the index untouched.",
];

/// The one thing a reader cannot guess and cannot recover from by guessing:
/// which keys get out. `Escape` belongs to whatever is running, so saying so
/// comes before anything else this view can do.
const TERMINAL_OVERVIEW: &[&str] = &[
    "This pane is running a program on a pseudoterminal. INSERT sends Escape, Ctrl-c, Ctrl-o, the space bar, and ordinary keys to it. Ctrl-\\ leaves input for live NORMAL; pressing it again captures review. {prefix:Ctrl-w} instead starts window commands directly; h/j/k/l and their control-key aliases move immediately. A live terminal destination enters INSERT, a reviewed terminal stays in NORMAL/review, and a document enters NORMAL; w uses the same destination behavior, x swaps complete pane contents, and v/s split into the working-directory explorer without capturing review.",
    "{binding:Ctrl-w n} opens Navigator directly from Terminal Insert; canceling resumes child input. Persistent sessions cycle with {binding:Shift-Left} and {binding:Shift-Right}; {binding:Ctrl-w a} returns to the last successful session. Overlays keep input ownership.",
    "Use EDITOR='runyte --wait' and VISUAL='runyte --wait' for external editing in this terminal's parent session. In a request buffer, :wq saves and returns, :q finishes a clean edit, and :q! cancels. In the shell, runyte -a opens the shell's exact directory in the outer TUI.",
    "Directional pane movement never starts or discards review. i resumes input, and canceling a {prefix:Ctrl-w} prefix leaves this terminal in INSERT.",
    "Live NORMAL keeps showing current output without sending keys to the child. A second Ctrl-\\ or the first review operation captures a stable snapshot; live output continues behind it. Move with ordinary motions, press v to extend, x/X to select and walk whole lines, or C/Alt-C to add carets below/above at the same occupied cell column. Escape cancels a selection made either way. y copies every selection to the unnamed register and {binding:Space c y} uses the system clipboard. Ctrl-u/Ctrl-d and Ctrl-b/Ctrl-f scroll; s and / search, n/N move among matches, and p sends clipboard text to the live child. u takes that paste back with one delete per character, while it is still the child's last input and did not end a line it has run.",
    "For real Runyte editing over what a terminal printed, copy its output into a buffer: that text is an ordinary read-only document where search, multiple selections, and yank all work.",
    "Composing goes the other way. Write the text in an ordinary buffer with every editing command available, then send the selection — or the whole buffer, with nothing selected — to a terminal as one bracketed paste. That is the only way modal editing can reach a program that owns its own input area.",
    "The pane's buffer is still there behind the terminal, and leaving the terminal shows it again without ending the program. Closing the pane or opening a file in it does the same; the session keeps running and the terminal list reaches it.",
];

/// Mouse input sits above the keymap and therefore cannot appear in the
/// registry-derived rows below. Keep the application-wide gestures together
/// here so every contextual help document describes them.
const MOUSE_OVERVIEW: &[&str] = &[
    "Left click focuses a pane and places its caret. Shift-click extends the current selection, and left-button drag selects text and enters SELECT mode. The wheel scrolls the pane under the pointer; dragging a shared pane border resizes the split.",
    "Right-clicking any current selection copies all current selections to the system clipboard, exactly like {binding:Space c y}, without moving or replacing them.",
    "A reviewed terminal accepts the same drag-selection and right-click copy gestures. A live terminal that requests SGR mouse input receives its mouse events instead. Runyte's mouse capture replaces the terminal's native text selection; set editor.mouse to false and restart when native selection is preferred.",
];

const HELP_TRAILER: &str = ":help opens the general Runyte manual; :help <topic> jumps to one of\n\
its sections. This contextual page remains available through {binding:Space ?}.\n\
:tutorial opens a guided two-pane introduction with disposable scratch text.\n";

const DIFF_OVERVIEW: &[&str] = &[
    "A unified diff, rendered read-only. Leading `+` and `-` belong to the patch rather than to the text, so nothing here can be edited into a different change.",
    "The staged view shows what a commit would take; the unstaged view shows what staging would add to it.",
    "In a per-file diff, `Tab s` stages the exact hunk and `Tab u` unstages it. Stale or unsupported patches are refused; use Lazygit for finer patch surgery.",
];

/// The menu of this view covers only the actions that take a row, so help that
/// described just them would leave someone believing the list is all there is.
/// The colon commands are the whole surface; the keys are shortcuts to part of
/// it.
const GIT_STASH_OVERVIEW: &[&str] = &[
    "Stashes are listed by stable object identity. Applying keeps the stash; dropping is a separate confirmed action.",
    "Every stash action is a colon command first, and this view's Tab menu offers the ones that take a row. `:git-stashes` opens or refreshes the list, and `:git-stash-apply` and `:git-stash-drop` act on the stash under the cursor — invoked from any other buffer they are refused, because there is no row to mean.",
    "Creating a stash has no key at all: the three commands below are the only way. Each takes a required name, asks for confirmation, and is refused while a file buffer in this repository has unsaved changes. An apply that conflicts keeps the stash and leaves the resolution to an external Git tool.",
];

/// The three creation commands differ along two axes at once — what goes into
/// the stash and what survives in the working tree — and `-tracked` and `-all`
/// differ only along the second. A sentence has to spend both axes on each
/// command in turn; a table lets the reader read down the column they care
/// about. `git-stash-tracked` is `--keep-index`, which is why it stashes the
/// same content as `git-stash-all` and yet leaves the staged changes standing.
const GIT_STASH_CREATION_TABLE: &[&str] = &[
    "Creating a stash",
    "  What each command puts in the stash, and what it leaves on disk.",
    "",
    "  Command               Stashes                     Leaves in the tree",
    "  --------------------  --------------------------  ----------------------",
    "  :git-stash-tracked    tracked worktree and index  staged changes, still",
    "                                                    staged, and untracked",
    "                                                    files",
    "  :git-stash-all        tracked worktree and index  untracked files",
    "  :git-stash-untracked  tracked worktree, index,    nothing",
    "                        and untracked files",
];

/// Renders the whole help document for one view.
///
/// Sections are ordered by how specific they are: what only this buffer
/// answers to, then everything it shares with every other buffer. A reader
/// who already knows the editor stops after the first section; a reader who
/// does not can keep going.
pub fn render(
    topic: HelpTopic,
    grammar: GrammarKind,
    scope: BindingScope,
    keymap: &Keymap,
    read_only: bool,
) -> String {
    render_document(topic, grammar, scope, keymap, read_only)
        .text()
        .to_owned()
}

pub(crate) fn render_document(
    topic: HelpTopic,
    grammar: GrammarKind,
    scope: BindingScope,
    keymap: &Keymap,
    read_only: bool,
) -> HelpDocument {
    render_document_with_descriptions(topic, grammar, scope, keymap, read_only, None, |_| None)
}

/// What a running plugin contributes to help for one of its views.
///
/// The topic is the plugin's own prose, if its current model names one. The
/// actions are what the view's Tab menu offers right now, read from the same
/// registry the menu reads, so this page cannot list an action the menu would
/// not.
pub(crate) struct PluginPage<'a> {
    pub application: &'a str,
    pub topic: Option<&'a crate::plugin::help::Topic>,
    pub actions: Vec<PluginAction<'a>>,
}

pub(crate) struct PluginAction<'a> {
    pub label: &'a str,
    pub description: &'a str,
    pub group: Option<&'a str>,
}

impl PluginPage<'_> {
    fn title(&self, read_only: bool) -> Option<String> {
        let topic = self.topic?;
        let access = if read_only { " · Read-only" } else { "" };
        Some(format!(
            " Help · {} · {}{access} ",
            self.application.to_uppercase(),
            topic.title.to_uppercase()
        ))
    }
}

pub(crate) fn render_document_with_descriptions(
    topic: HelpTopic,
    grammar: GrammarKind,
    scope: BindingScope,
    keymap: &Keymap,
    read_only: bool,
    plugin: Option<&PluginPage<'_>>,
    description: impl Fn(crate::keymap::BindingTarget) -> Option<String>,
) -> HelpDocument {
    // Normal and Select bind the same sequences to the same commands, so
    // either answers for both. `normal_and_select_bind_the_same_sequences` in
    // keymap.rs fails if that stops being true, since this would then be
    // quietly documenting half the keymap.
    let mode = Mode::Normal;
    let mut out = String::new();
    let title = plugin
        .and_then(|plugin| plugin.title(read_only))
        .unwrap_or_else(|| topic.title_for(grammar, read_only));
    let _ = writeln!(
        out,
        "{}\n",
        crate::key_spelling::escape_markers(title.trim())
    );

    // Character ranges of text a plugin wrote. They are escaped on the way in
    // so no marker can hide in them, and styled only by their own backticks.
    let mut authored: Vec<Range<usize>> = Vec::new();
    let mut headings: Vec<Range<usize>> = Vec::new();
    if let Some(topic) = plugin.and_then(|plugin| plugin.topic) {
        let from = out.chars().count();
        for paragraph in &topic.paragraphs {
            let _ = writeln!(out, "{}\n", crate::key_spelling::escape_markers(paragraph));
        }
        authored.push(from..out.chars().count());
    } else {
        for paragraph in topic.overview_for(grammar) {
            let _ = writeln!(out, "{paragraph}\n");
        }
    }
    // The overview is written for the keys as they ship. Single-key pane
    // movement is the one option that changes what a *running program* sees,
    // so the terminal page says so rather than leaving the reader to notice
    // that Ctrl-l stopped clearing their screen.
    if topic == HelpTopic::Terminal && fast_pane_keys_are_active(keymap) {
        let _ = writeln!(
            out,
            "editor.fast_pane_keys is on, so Ctrl-h/j/k/l also leave this pane and the\n             program never receives them.\n"
        );
    }
    let _ = writeln!(out, "{}", HELP_TRAILER);

    let _ = writeln!(out, "Mouse");
    for paragraph in MOUSE_OVERVIEW {
        let _ = writeln!(out, "{paragraph}\n");
    }

    // Written out as-is. These rows are already aligned against each other,
    // so anything that reflowed them would be destroying the only reason they
    // are not prose.
    let table = topic.table_for(grammar);
    if !table.is_empty() {
        for line in table {
            let _ = writeln!(out, "{line}");
        }
        out.push('\n');
    }

    // The title already states the fact, so this states the consequence. It
    // says "this view" rather than "this buffer" because the document is
    // about the buffer type it names, not about the help buffer showing it.
    if read_only {
        let _ = writeln!(
            out,
            "Text edits are refused in this view rather than silently ignored,\n\
             and keys that could only produce a refusal are left out below.\n"
        );
    }

    // Every key cell a generated table writes, in character offsets. Only
    // these positions are coloured from the registry; prose is marked from
    // the authored lists further down, because a key's raw text is an
    // ordinary English word far more often than it is a key mention.
    let mut key_cells: Vec<Range<usize>> = Vec::new();

    if let Some(plugin) = plugin.filter(|plugin| !plugin.actions.is_empty()) {
        let _ = writeln!(out, "Application actions");
        let _ = writeln!(
            out,
            "  {} opens the menu of these, as they were offered for the selection\n  help was opened from.\n",
            ACTION_MENU_KEY
        );
        // Sections follow their first action, as they do in the menu.
        let mut groups: Vec<Option<&str>> = Vec::new();
        for action in &plugin.actions {
            if !groups.contains(&action.group) {
                groups.push(action.group);
            }
        }
        for group in groups {
            let indent = if let Some(group) = group {
                let from = out.chars().count() + 2;
                let _ = writeln!(out, "  {}", crate::key_spelling::escape_markers(group));
                // Measured as written: escaping can lengthen the name.
                headings.push(from..out.chars().count() - 1);
                "    "
            } else {
                "  "
            };
            for action in plugin.actions.iter().filter(|action| action.group == group) {
                let from = out.chars().count();
                let _ = write!(
                    out,
                    "{indent}{}",
                    crate::key_spelling::escape_markers(action.label)
                );
                if action.description != action.label {
                    let _ = write!(
                        out,
                        " — {}",
                        crate::key_spelling::escape_markers(action.description)
                    );
                }
                out.push('\n');
                authored.push(from..out.chars().count());
            }
            out.push('\n');
        }
    }

    let _ = writeln!(out, "Normal and Select\n");
    let scoped = keymap.scoped_bindings(mode, scope).collect::<Vec<_>>();
    let actions = keymap.context_actions(scope).collect::<Vec<_>>();
    if !scoped.is_empty() || !actions.is_empty() {
        let _ = writeln!(out, "Buffer keys");
        let _ = writeln!(out, "  Only this view answers to these.\n");
        for binding in &scoped {
            let label = description(binding.target);
            let detail = platform_description(
                binding.target,
                label.as_deref().unwrap_or(&binding.description),
            );
            // A plugin wrote this description, and Runyte validates it for
            // length and control characters, not for braces.
            let plugin = matches!(binding.target, BindingTarget::Plugin(_));
            let detail = if plugin {
                std::borrow::Cow::Owned(crate::key_spelling::escape_markers(&detail))
            } else {
                detail
            };
            key_cells.push(row(&mut out, &binding.sequence.to_string(), &detail));
            if plugin {
                let end = out.chars().count() - 1;
                authored.push(end - detail.chars().count()..end);
            }
        }
        if !actions.is_empty() {
            if !scoped.is_empty() {
                out.push('\n');
            }
            let _ = writeln!(
                out,
                "  Tab opens the action menu. Its mnemonic keys are active only while\n  that menu is open.\n"
            );
            for action in actions {
                key_cells.push(row(
                    &mut out,
                    &format!("Tab {}", action.mnemonic.label()),
                    &platform_description(action.target, action.description),
                ));
            }
        }
        out.push('\n');
    }

    // Entry points are split by whether a key finishes on its own. A prefix
    // opens the hint popup and teaches the rest of itself; a leaf runs
    // immediately and so is the only kind of key nothing can advertise.
    let entries = keymap.entry_points(mode, scope);
    let prefixes = entries
        .iter()
        .filter(|entry| entry.prefix && !entry.scoped)
        .collect::<Vec<_>>();
    if !prefixes.is_empty() {
        let _ = writeln!(out, "Where to start");
        let _ = writeln!(
            out,
            "  Press one and pause: the hint popup lists what follows.\n"
        );
        for entry in prefixes {
            let label = entry.key.label();
            let cell = row(&mut out, &format!("{label} …"), entry.description);
            // The ellipsis says the key opens onto more; it is not part of
            // what anyone presses, so it stays outside the marked range.
            key_cells.push(cell.start..cell.start + label.chars().count());
        }
        out.push('\n');
    }

    let direct = entries
        .iter()
        .filter(|entry| !entry.prefix && !entry.scoped)
        .filter(|entry| !read_only || !hides_a_refusal(entry.key, mode, scope, keymap))
        .collect::<Vec<_>>();
    if !direct.is_empty() {
        let _ = writeln!(out, "Direct keys");
        let _ = writeln!(out, "  These act on the first press, in every view.\n");
        // Grouped by the shape of the key rather than listed as one run.
        // A chord is not findable among seventy letters: someone looking for
        // Ctrl-o is looking for a chord, not for the letter it happens to use.
        for (label, group) in KeyShape::ALL.iter().map(|shape| {
            (
                shape.label(),
                direct
                    .iter()
                    .filter(|entry| KeyShape::of(entry.key) == *shape)
                    .collect::<Vec<_>>(),
            )
        }) {
            if group.is_empty() {
                continue;
            }
            let _ = writeln!(out, "  {label}");
            for entry in group {
                let detail = match keymap.lookup_in(mode, scope, &KeySequence::from(entry.key)) {
                    Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                        platform_description(binding.target, entry.description)
                    }
                    _ => std::borrow::Cow::Borrowed(entry.description),
                };
                key_cells.push(row(&mut out, &help_key_label(entry.key), &detail));
            }
            out.push('\n');
        }
    }

    // Insert and Replace share most bindings. Compare what this scope actually
    // dispatches in each mode, so configured keys and scoped overrides stay in
    // the right section without a second key inventory.
    if read_only && topic != HelpTopic::Terminal {
        let _ = writeln!(
            out,
            "Insert and Replace\n  Unavailable in this read-only view.\n"
        );
    } else {
        let visible = |binding: &&crate::keymap::Binding| {
            topic != HelpTopic::Terminal || terminal_insert_admits(binding, keymap)
        };
        let insert = keymap
            .bindings_for_scope(Mode::Insert, scope)
            .filter(visible)
            .collect::<Vec<_>>();
        let replace = keymap
            .bindings_for_scope(Mode::Replace, scope)
            .filter(visible)
            .collect::<Vec<_>>();
        if topic == HelpTopic::Terminal {
            write_mode_bindings(
                &mut out,
                "Terminal Insert",
                &insert,
                &description,
                &mut key_cells,
                &mut authored,
            );
            let _ = writeln!(
                out,
                "  Ordinary keys go to the child program; the Runyte-owned exceptions are described above.\nReplace\n  Unavailable in a terminal view.\n"
            );
        } else {
            let same = |left: &&crate::keymap::Binding, right: &&crate::keymap::Binding| {
                left.sequence == right.sequence
                    && left.target == right.target
                    && left.description == right.description
            };
            let shared = insert
                .iter()
                .copied()
                .filter(|binding| replace.iter().any(|other| same(binding, other)))
                .collect::<Vec<_>>();
            let insert_only = insert
                .iter()
                .copied()
                .filter(|binding| !replace.iter().any(|other| same(binding, other)))
                .collect::<Vec<_>>();
            let replace_only = replace
                .iter()
                .copied()
                .filter(|binding| !insert.iter().any(|other| same(binding, other)))
                .collect::<Vec<_>>();
            write_mode_bindings(
                &mut out,
                "Insert and Replace",
                &shared,
                &description,
                &mut key_cells,
                &mut authored,
            );
            write_mode_bindings(
                &mut out,
                "Insert only",
                &insert_only,
                &description,
                &mut key_cells,
                &mut authored,
            );
            write_mode_bindings(
                &mut out,
                "Replace only",
                &replace_only,
                &description,
                &mut key_cells,
                &mut authored,
            );
        }
    }

    // Command input is owned by the prompt rather than Keymap dispatch. It
    // has no registry bindings to tabulate, and this heading makes that
    // boundary explicit instead of silently omitting the mode.
    let _ = writeln!(
        out,
        "Command\n  The command prompt handles text input directly. Type to edit; Enter first accepts a pending command or path completion, otherwise submits a nonempty command. Escape or Ctrl-c cancels. Tab completes; Up/Down or Shift-Tab choose suggestions, and Home/End jump to the first/last suggestion. Left/Right or Ctrl-b/f move the caret; Ctrl-a/e move to its ends and Alt-b/f move by word. Backspace or Ctrl-h deletes backward, Ctrl-d deletes forward, Ctrl-w deletes the previous word, and Ctrl-u/k deletes to the start/end. Ctrl-s saves the active file.\n"
    );

    let (resolved, offset_map) = crate::key_spelling::resolve_with_map(&out, keymap)
        .expect("help key markers must resolve against every built-in keymap");
    let mut document = HelpDocumentWriter::new();
    document.write_prose(&resolved.text);
    for range in resolved.substitutions {
        document.mark_range(range.start, range.end, HelpRole::KeyBinding);
    }

    for heading in [
        title.trim(),
        "Mouse",
        "Application actions",
        "Normal and Select",
        "Buffer keys",
        "Where to start",
        "Direct keys",
        "Letters and punctuation",
        "Ctrl chords",
        "Alt chords",
        "Arrows and named keys",
        "Insert and Replace",
        "Insert only",
        "Replace only",
        "Terminal Insert",
        "Replace",
        "Command",
        "Creating a stash",
    ] {
        document.mark_token_since(0, heading, HelpRole::Heading);
    }

    // Key execution and help styling obtain their spellings from the same
    // registry, but only where the registry's spelling is unambiguous.
    //
    // The key tables are that place: a cell there holds a key and nothing
    // else. Searching the whole document for a binding's raw text is not,
    // because most single-character bindings spell ordinary English —
    // limiting the search to keys the tables actually printed still left the
    // indefinite article "a", the sentence-opening "A", and the "Left" and
    // "Right" of "Left click" and "Right-clicking" wearing the key colour
    // throughout the prose.
    for cell in &key_cells {
        document.mark_range(
            offset_map[cell.start],
            offset_map[cell.end],
            HelpRole::KeyBinding,
        );
    }
    // A context action is spelled `Tab x`, which is never an English word, so
    // it is safe to mark wherever the prose names one.
    for action in keymap.context_actions(scope) {
        document.mark_token_since(
            0,
            &format!("Tab {}", action.mnemonic.label()),
            HelpRole::KeyBinding,
        );
    }
    // Prose key mentions are authored rather than derived, for the reason
    // above. A key named in a paragraph belongs on this list; one that only
    // appears in the tables does not need to be here.
    for key in [
        "Shift-click",
        "right-click",
        "left-button drag",
        "Ctrl-h/j/k/l",
        "Ctrl-u/Ctrl-d",
        "Ctrl-b/Ctrl-f",
        "h/j/k/l",
        "C/Alt-C",
        "Ctrl-\\",
        "Ctrl-c",
        "Ctrl-o",
        "Ctrl-n",
        "Ctrl-p",
        "Escape",
        "Enter",
        "NORMAL",
        "SELECT",
        "INSERT",
        "Tab",
        "x/X",
        "n/N",
        "v/s",
        "i",
        "v",
        "s",
        "w",
        "n",
        "N",
        "/",
        "y",
        "p",
        "u",
    ] {
        document.mark_token_since(0, key, HelpRole::KeyBinding);
    }

    for command in [
        ":git-stash-untracked",
        ":git-stash-tracked",
        ":git-stash-apply",
        ":git-stash-drop",
        ":git-stashes",
        ":git-stash-all",
        ":service-health",
        ":notifications",
        ":write-quit",
        ":tutorial sessions",
        ":tutorial",
        ":log-open",
        ":help diagnostics",
        ":help <topic>",
        ":help",
        ":not",
        ":write",
        ":c!",
        ":c",
    ] {
        document.mark_token_since(0, command, HelpRole::Command);
    }

    for literal in [
        "editor.fast_pane_keys",
        "editor.mouse",
        "--config PATH",
        "--keep-index",
        "[worktree: /local/path]",
        "[↑2 ↓1]",
        "[gone]",
        "[missing]",
        "[prunable]",
        "[RO]",
        "[STALE]",
        "NORMAL/review",
        "SGR",
    ] {
        document.mark_token_since(0, literal, HelpRole::Code);
    }
    for path in [
        "~/.config/runyte/config.yaml",
        "/local/path",
        "docs/lsp/",
        ".runyte/",
        ".runyte",
    ] {
        document.mark_token_since(0, path, HelpRole::FilePath);
    }

    // Last, so that nothing Runyte marks by searching the whole document for
    // its own words can land inside text a plugin wrote.
    for range in &authored {
        document.reset_to_prose(offset_map[range.start], offset_map[range.end]);
    }
    for range in &headings {
        document.mark_range(
            offset_map[range.start],
            offset_map[range.end],
            HelpRole::Heading,
        );
    }

    document.finish()
}

/// Opens a plugin view's action menu. Named here because the actions section
/// cites it, and the key table it sits beside is generated.
const ACTION_MENU_KEY: &str = "{literal-key:Tab}";

fn platform_description(target: BindingTarget, description: &str) -> std::borrow::Cow<'_, str> {
    match target.id().platform_unavailable() {
        Some(reason) => std::borrow::Cow::Owned(format!("{description} (unavailable: {reason})")),
        None => std::borrow::Cow::Borrowed(description),
    }
}

/// How a key is typed, which is how someone looks for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyShape {
    Plain,
    Control,
    Alt,
    Named,
}

impl KeyShape {
    const ALL: &'static [Self] = &[Self::Plain, Self::Control, Self::Alt, Self::Named];

    fn of(key: crate::keymap::Key) -> Self {
        if key.modifiers.contains(Modifiers::CONTROL) {
            Self::Control
        } else if key.modifiers.contains(Modifiers::ALT) {
            Self::Alt
        } else if matches!(key.code, KeyCode::Char(_)) {
            Self::Plain
        } else {
            Self::Named
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Plain => "Letters and punctuation",
            Self::Control => "Ctrl chords",
            Self::Alt => "Alt chords",
            Self::Named => "Arrows and named keys",
        }
    }
}

/// Whether a single-key binding would only ever report a read-only refusal.
fn hides_a_refusal(
    key: crate::keymap::Key,
    mode: Mode,
    scope: BindingScope,
    keymap: &Keymap,
) -> bool {
    keymap
        .bindings_for_scope(mode, scope)
        .filter(|binding| binding.sequence.as_slice() == [key])
        .any(|binding| match binding.target {
            crate::keymap::BindingTarget::Editor(command) => command.is_mutating(),
            crate::keymap::BindingTarget::Colon(_) => false,
            crate::keymap::BindingTarget::Plugin(_) => true,
        })
}

/// First keys allowed past `App::handle_key_stroke`'s Terminal Insert gate.
/// Its pane-key and persistent-session paths are conditional at runtime; help
/// includes them because the same registry entry is usable when enabled.
fn terminal_insert_admits(binding: &crate::keymap::Binding, keymap: &Keymap) -> bool {
    let Some(&first) = binding.sequence.as_slice().first() else {
        return false;
    };
    let first = first.canonical_for_binding();
    let terminal_escape = crate::app::is_terminal_normal_key(first);
    let window_prefix = first == keymap.window_prefix();
    let fast_pane_move =
        fast_pane_keys_are_active(keymap) && crate::keymap::is_fast_pane_key(first);
    let navigation = |binding: &crate::keymap::Binding| {
        matches!(
            binding.target,
            BindingTarget::Editor(
                EditorCommand::NextRunningSession
                    | EditorCommand::PreviousRunningSession
                    | EditorCommand::PreviousSession
            )
        )
    };
    let persistent_navigation = match keymap.lookup_in(
        Mode::Insert,
        BindingScope::Terminal,
        &KeySequence::from(first),
    ) {
        Lookup::Exact(binding) => navigation(binding),
        Lookup::Prefix(bindings) => bindings.iter().any(|binding| navigation(binding)),
        Lookup::ExactAndPrefix {
            exact,
            continuations,
        } => navigation(exact) || continuations.iter().any(|binding| navigation(binding)),
        Lookup::NoMatch => false,
    };
    terminal_escape || window_prefix || fast_pane_move || persistent_navigation
}

fn help_key_label(key: Key) -> String {
    let label = key.label();
    if key.code == KeyCode::BackTab {
        format!("{label} / Shift-Tab")
    } else if key.modifiers.is_empty() && matches!(key.code, KeyCode::Char('<' | '>')) {
        format!("{label} / Shift-{label}")
    } else {
        label
    }
}

fn write_mode_bindings(
    out: &mut String,
    heading: &str,
    bindings: &[&crate::keymap::Binding],
    description: &impl Fn(BindingTarget) -> Option<String>,
    key_cells: &mut Vec<Range<usize>>,
    authored: &mut Vec<Range<usize>>,
) {
    let _ = writeln!(out, "{heading}");
    if bindings.is_empty() {
        let _ = writeln!(out, "  No additional bindings.\n");
        return;
    }
    for binding in bindings {
        let label = if binding.sequence.len() == 1 {
            help_key_label(binding.sequence.as_slice()[0])
        } else {
            binding.sequence.to_string()
        };
        let configured = description(binding.target);
        let detail = platform_description(
            binding.target,
            configured
                .as_deref()
                .unwrap_or(binding.description.as_ref()),
        );
        let plugin = matches!(binding.target, BindingTarget::Plugin(_));
        let detail = if plugin {
            std::borrow::Cow::Owned(crate::key_spelling::escape_markers(&detail))
        } else {
            detail
        };
        key_cells.push(row(out, &label, &detail));
        if plugin {
            let end = out.chars().count() - 1;
            authored.push(end - detail.chars().count()..end);
        }
    }
    out.push('\n');
}

fn fast_pane_keys_are_active(keymap: &Keymap) -> bool {
    keymap.fast_pane_keys()
}

/// Writes one key-table row and reports where its key cell landed.
///
/// The returned character range is what lets the key column be coloured on
/// its own. In that column a bare letter is certainly a key; in the prose
/// above it, the same letter is usually an article or the start of a
/// sentence, so the two are marked by different means.
fn row(out: &mut String, keys: &str, description: &str) -> Range<usize> {
    let padding = KEY_COLUMN.saturating_sub(keys.chars().count());
    let from = out.chars().count() + 2;
    let _ = writeln!(out, "  {keys}{}{description}", " ".repeat(padding.max(1)));
    from..from + keys.chars().count()
}

/// The same words in both grammars: nothing here is a motion or an operator,
/// so neither grammar has anything of its own to say about it.
const GIT_STATUS_OVERVIEW: &[&str] = &[
    "The changed-file list groups every file by whether a commit would take it. Rows are files: select several and one key acts on all of them.",
    "`Tab s` stages the selected rows; `Tab S` stages every unstaged or untracked row. Staging records files as written on disk and moves the base that the gutter marks are measured against.",
    "Committing takes the index — exactly what the Staged section shows. Write the message buffer to commit, or close it with `:c` / `:c!` to abandon it.",
    "Discarding is the one action here that cannot be undone: the thrown-away content was never a commit, so nothing in Git will produce it again.",
    "`Tab p` and `Tab P` pull and push the branch this working tree is on. Both reach the network and hold the editor until the remote answers or two minutes pass; the push never forces.",
    "`Tab p` fast-forwards silently. When the branch and its upstream have both moved on there is no fast-forward, so it says how far apart they are and offers to replay your commits on top; Enter does it, Escape leaves the branch alone. A replay that hits a conflict undoes itself and changes nothing, and neither the pull nor the replay stashes uncommitted changes: a dirty worktree is refused up front.",
];

const GIT_BRANCHES_OVERVIEW: &[&str] = &[
    "The Local section comes first and marks the current branch with an asterisk. A `[worktree: /local/path]` note identifies every registered checkout. The Remote section lists locally cached remote-tracking refs; opening or refreshing this view does not fetch.",
    "A local branch that tracks a remote one carries its drift in brackets: `[↑2 ↓1]` is two commits it has that the upstream does not and one the upstream has that it does not, `[=]` is in step, and `[gone]` is an upstream that no longer exists. Each remote row names every local branch configured to track it, or says `[not tracked locally]`.",
    "Enter on a local row checks it out. On a remote row it checks out the one local branch tracking it, asks when several do, or creates a same-named local tracking branch when none does. A conflicting local name is presented for editing rather than silently reused.",
    "Checking out a branch is refused while the working tree, index, or an open file buffer has uncommitted changes. When any terminal session is still running, type the exact target branch name to acknowledge that its job will keep using the working directory while Git replaces files.",
    "`Tab n` starts a new branch at the selected local or remote row and switches to it. `Tab w` creates a worktree for the selected branch and attaches to it in persistent mode; an already checked-out local branch points to its existing worktree instead of being forced into a second one.",
    "`Tab D` reviews a selected local branch: Enter is enough when an upstream or another local branch retains its tip; otherwise type the exact branch name. Cached upstream state reflects the last fetch.",
    "A branch checked out in a registered worktree takes that worktree, and the persistent session on it, with it. One confirmation names all three levels and always asks for the exact branch name; accepting stops the session, removes the worktree, then deletes the branch, and a failure at any level stops there. More than one checkout, or a checkout at this Runyte root, is still refused.",
    "`Tab p` fast-forwards the current branch onto what it tracks. When the two have both moved on it offers instead to replay the local commits on top of the upstream's: Enter rebases, Escape leaves the branch as it is, and a conflict undoes the replay rather than leaving a tree to resolve here. In the branch list it refuses a row that is not the current branch. `Tab P` publishes the selected branch, setting an upstream the first time; it never forces.",
    "Both reach the network and hold the editor until the remote answers or two minutes pass. Nothing can prompt for a password while they run, so an authentication that needs one fails instead of hanging.",
];

const GIT_WORKTREES_OVERVIEW: &[&str] = &[
    "The worktree list shows every checkout registered with this repository. Paths are identities even when their display needs replacement characters.",
    "`detached` means HEAD points directly at a commit instead of a local branch. The checkout still works, but new commits do not advance a branch unless you create or switch to one.",
    "`missing` means Git still has this worktree registered, but its directory is absent from the filesystem.",
    "`prunable` means Git considers the registration stale and eligible for `git worktree prune`, usually because its administrative metadata or checkout path is gone. A row can be both missing and prunable.",
    "Enter opens the selected root as a separate workspace; it never retargets this workspace's buffers or language servers in place. Unsaved buffers refuse the switch.",
    "`Tab n` names a new branch at the selected checkout's tip, asks for its worktree destination, and attaches there in persistent mode. Creating a worktree for an existing branch belongs to that branch's `Tab w` action in the branch list.",
    "`Tab D` removes one ordinary worktree after confirmation, leaving its branch. It refuses Git changes and unsaved persistent-session buffers; unpublished tracked or unretained detached history needs the exact branch name or displayed path. Current, locked, bare, and unavailable worktrees are also refused.",
    "A clean session on the worktree is stopped and forgotten rather than refusing the removal. The confirmation names it and asks for the exact branch name for that reason alone. The session stops before Git is asked to remove the directory it owns, and the removal has to succeed before its record is forgotten, so a refusal at either point leaves everything below it alone. This happens in standalone mode too: stopping that host is part of the removal, not a `session` command.",
];

const GIT_LOG_OVERVIEW: &[&str] = &[
    "The log loads up to 10,000 commits per page in topological order. Rows keep their full commit object identity even though the display uses an abbreviation.",
    "Enter opens the selected commit's bounded metadata and patch. `Ctrl-n` and `Ctrl-p` load the next and previous object-cursor pages; {binding:Space g r} reconciles the view without taking the replace-character key.",
];

const GIT_BLAME_OVERVIEW: &[&str] = &[
    "Blame was computed from the live buffer text, so unsaved lines are shown as uncommitted instead of being attributed to older disk content.",
    "Enter opens the commit on a committed row. Uncommitted rows deliberately have no historical target.",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::default_keymap;

    #[test]
    fn authored_key_markers_are_complete_and_resolve_in_both_variants() {
        crate::key_spelling::assert_authored_template(HELP_TRAILER);
        for template in MOUSE_OVERVIEW {
            crate::key_spelling::assert_authored_template(template);
        }
        for topic in HelpTopic::ALL {
            for template in topic
                .overview_for(GrammarKind::Runyte)
                .iter()
                .chain(topic.table_for(GrammarKind::Runyte))
            {
                crate::key_spelling::assert_authored_template(template);
            }
        }
    }

    /// The buffer type picks the topic. Nothing else does, and in particular
    /// the mode does not: a text buffer answers with one document whether the
    /// reader is in NORMAL or SELECT.
    #[test]
    fn the_buffer_type_alone_selects_the_topic() {
        for (scope, expected) in [
            (BindingScope::Directory, HelpTopic::Explorer),
            (BindingScope::Settings, HelpTopic::Config),
            (BindingScope::GitStatus, HelpTopic::GitStatus),
            (BindingScope::GitBranches, HelpTopic::GitBranches),
            (BindingScope::GitWorktrees, HelpTopic::GitWorktrees),
            (BindingScope::GitLog, HelpTopic::GitLog),
            (BindingScope::GitBlame, HelpTopic::GitBlame),
            (BindingScope::CommitMessage, HelpTopic::CommitMessage),
            (BindingScope::Diff, HelpTopic::Diff),
            (BindingScope::Global, HelpTopic::Text),
            (BindingScope::Help, HelpTopic::Text),
        ] {
            assert_eq!(HelpTopic::for_context(scope), expected, "{scope:?}");
        }
    }

    /// One document has to answer for both modal modes, so the mode the
    /// reader is in must be described rather than assumed.
    #[test]
    fn the_text_topic_describes_both_modal_modes() {
        let prose = HelpTopic::Text
            .overview_for(GrammarKind::Runyte)
            .join(" ")
            .to_lowercase();
        assert!(prose.contains("normal"), "{prose}");
        assert!(prose.contains("select"), "{prose}");
    }

    #[test]
    fn contextual_help_covers_each_default_binding_in_its_mode_and_scope() {
        let keymap = default_keymap();
        for &scope in BindingScope::ALL {
            let topic = HelpTopic::for_context(scope);
            let read_only = !matches!(
                scope,
                BindingScope::Global
                    | BindingScope::Markdown
                    | BindingScope::Directory
                    | BindingScope::CommitMessage
            );
            let document = render(topic, GrammarKind::Runyte, scope, keymap, read_only);
            let insert_heading = if scope == BindingScope::Terminal {
                "Terminal Insert\n"
            } else {
                "Insert and Replace\n"
            };
            let modal = document
                .split_once("Normal and Select\n")
                .unwrap()
                .1
                .split_once(insert_heading)
                .unwrap()
                .0;
            let insert_and_replace = document.split_once(insert_heading).unwrap().1;
            for mode in [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace] {
                for binding in keymap.bindings_for_scope(mode, scope) {
                    if mode == Mode::Replace && scope == BindingScope::Terminal {
                        assert!(insert_and_replace.contains("Unavailable in a terminal view"));
                        continue;
                    }
                    if matches!(mode, Mode::Insert | Mode::Replace)
                        && read_only
                        && scope != BindingScope::Terminal
                    {
                        assert!(insert_and_replace.contains("Unavailable in this read-only view"));
                        continue;
                    }
                    if matches!(mode, Mode::Insert | Mode::Replace)
                        && scope == BindingScope::Terminal
                        && !terminal_insert_admits(binding, keymap)
                    {
                        continue;
                    }
                    if matches!(mode, Mode::Normal | Mode::Select)
                        && read_only
                        && binding.scope == BindingScope::Global
                        && binding.sequence.len() == 1
                        && hides_a_refusal(binding.sequence.as_slice()[0], mode, scope, keymap)
                    {
                        continue;
                    }
                    let label = if matches!(mode, Mode::Normal | Mode::Select)
                        && binding.scope == BindingScope::Global
                        && binding.sequence.len() > 1
                    {
                        binding.sequence.as_slice()[0].label()
                    } else if binding.sequence.len() == 1 {
                        help_key_label(binding.sequence.as_slice()[0])
                    } else {
                        binding.sequence.to_string()
                    };
                    let section = if matches!(mode, Mode::Normal | Mode::Select) {
                        modal
                    } else {
                        insert_and_replace
                    };
                    assert!(
                        section.lines().any(|line| {
                            line.trim_start().starts_with(&label) && line.starts_with("  ")
                        }),
                        "{scope:?} {mode:?} missing {label:?} ({:?})",
                        binding.target
                    );
                }
            }
            assert!(document.contains("Command\n"));
            assert_eq!(keymap.bindings_for_scope(Mode::Command, scope).count(), 0);
        }
    }

    #[test]
    fn insert_and_replace_share_rows_and_shifted_indent_is_searchable() {
        let document = render(
            HelpTopic::Text,
            GrammarKind::Runyte,
            BindingScope::Global,
            default_keymap(),
            false,
        );
        let shared = document
            .split_once("Insert and Replace\n")
            .unwrap()
            .1
            .split_once("Insert only\n")
            .unwrap()
            .0;
        assert!(shared.contains("Tab"));
        assert!(shared.contains("Shift-Tab"));
        assert!(shared.contains("Ctrl-x"));
        assert!(shared.contains("Ctrl-w"));
        assert_eq!(shared.matches("Ctrl-x").count(), 1);
        assert!(document.contains("< / Shift-<"));
        assert!(document.contains("> / Shift->"));
    }

    #[test]
    fn configured_insert_bindings_and_live_indent_descriptions_reach_help() {
        let section: serde_yaml::Value = serde_yaml::from_str("rebind:\n  Ctrl-x: F12\n").unwrap();
        let compiled = crate::keymap::configured::compile(&section, default_keymap());
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        let document = render(
            HelpTopic::Text,
            GrammarKind::Runyte,
            BindingScope::Global,
            &compiled.keymap,
            false,
        );
        let shared = document
            .split_once("Insert and Replace\n")
            .unwrap()
            .1
            .split_once("Insert only\n")
            .unwrap()
            .0;
        assert!(
            shared
                .lines()
                .any(|line| line.trim_start().starts_with("F12 "))
        );
        assert!(
            !shared
                .lines()
                .any(|line| line.trim_start().starts_with("Ctrl-x "))
        );

        let tabs = default_keymap().with_indent_style(crate::config::IndentStyle::Tabs);
        let document = render(
            HelpTopic::Text,
            GrammarKind::Runyte,
            BindingScope::Global,
            &tabs,
            false,
        );
        assert!(document.contains("Insert a tab (indent: tabs)"));
        assert!(document.contains("Insert spaces to the next tab stop (indent: tabs)"));
    }

    #[test]
    fn terminal_insert_help_lists_only_keys_admitted_past_the_child_gate() {
        for keymap in [
            std::sync::Arc::new(default_keymap().clone()),
            crate::keymap::keymap_for(true),
        ] {
            let document = render(
                HelpTopic::Terminal,
                GrammarKind::Runyte,
                BindingScope::Terminal,
                &keymap,
                true,
            );
            let rows = document
                .split_once("Terminal Insert\n")
                .unwrap()
                .1
                .split_once("\nReplace\n")
                .unwrap()
                .0;
            let has_row = |key: &str| {
                rows.lines()
                    .any(|line| line.starts_with("  ") && line.trim_start().starts_with(key))
            };
            for child_key in [
                "Escape ",
                "Left ",
                "Right ",
                "Home ",
                "End ",
                "PageUp ",
                "PageDown ",
                "Ctrl-x ",
                "Tab ",
                "Ctrl-v ",
                "Alt-v ",
            ] {
                assert!(!has_row(child_key), "child owns {child_key:?}");
            }
            assert!(has_row("Ctrl-\\ "));
            assert!(has_row("Ctrl-w n "));
            assert!(has_row("Shift-Left "));
            assert_eq!(has_row("Ctrl-h "), fast_pane_keys_are_active(&keymap));
        }
    }

    #[test]
    fn terminal_insert_help_follows_effective_first_key_admission() {
        // These direct registry edits exercise dispatchable maps even though
        // keys.rebind currently only accepts Space/Ctrl-w namespace sources.
        let cases = [
            (false, "Shift-Left", "F12 p", "F12 p "),
            (true, "Ctrl-h", "F12", "Ctrl-j "),
            (false, "Ctrl-4", "Ctrl-Alt-4 z", "Ctrl-Alt-4 z "),
        ];
        for (fast_pane_keys, source, target, expected) in cases {
            let built_in = crate::keymap::keymap_for(fast_pane_keys);
            let mut bindings = built_in.bindings().to_vec();
            let source = KeySequence::parse(source).unwrap();
            let binding = bindings
                .iter_mut()
                .find(|binding| binding.is_active_in(Mode::Insert) && binding.sequence == source)
                .unwrap();
            binding.sequence = KeySequence::parse(target).unwrap();
            Keymap::new(bindings.clone()).unwrap();
            let keymap = built_in.with_test_bindings(bindings);
            let document = render(
                HelpTopic::Terminal,
                GrammarKind::Runyte,
                BindingScope::Terminal,
                &keymap,
                true,
            );
            let rows = document
                .split_once("Terminal Insert\n")
                .unwrap()
                .1
                .split_once("\nReplace\n")
                .unwrap()
                .0;
            assert!(
                rows.lines()
                    .any(|line| line.trim_start().starts_with(expected)),
                "{source} -> {target}: missing {expected:?} from {rows}"
            );
        }

        let section: serde_yaml::Value = serde_yaml::from_str("{}").unwrap();
        let fast = crate::keymap::keymap_for(true);
        let compiled = crate::keymap::configured::compile(&section, &fast);
        assert!(compiled.errors.is_empty());
        assert!(compiled.keymap.fast_pane_keys());
    }

    #[test]
    fn the_terminal_topic_distinguishes_live_and_reviewed_focus_destinations() {
        let prose = HelpTopic::Terminal
            .overview_for(GrammarKind::Runyte)
            .join(" ");

        assert!(prose.contains("A live terminal destination enters INSERT"));
        assert!(prose.contains("a reviewed terminal stays in NORMAL/review"));
        assert!(prose.contains("never starts or discards review"));
    }

    #[test]
    fn every_contextual_document_describes_mouse_selection_and_clipboard_copy() {
        for topic in HelpTopic::ALL {
            let rendered = render(
                *topic,
                GrammarKind::Runyte,
                BindingScope::Global,
                default_keymap(),
                false,
            );

            for required in [
                "Mouse\n",
                "Shift-click extends the current selection",
                "left-button drag selects text",
                "copies all current selections to the system clipboard",
                "exactly like Space c y",
                "reviewed terminal",
                "editor.mouse to false",
            ] {
                assert!(
                    rendered.contains(required),
                    "{topic:?} is missing {required:?}"
                );
            }
        }
    }

    /// Every topic still has prose of its own. The key tables are derived, so
    /// this is the only part a new view can forget to write.
    #[test]
    fn every_topic_carries_its_own_prose() {
        for topic in HelpTopic::ALL {
            assert!(
                !topic.overview_for(GrammarKind::Runyte).is_empty(),
                "{topic:?}"
            );
        }
    }

    #[test]
    fn the_worktree_topic_explains_git_state_labels() {
        let prose = HelpTopic::GitWorktrees
            .overview_for(GrammarKind::Runyte)
            .join(" ");

        assert!(prose.contains("`detached` means HEAD points directly at a commit"));
        assert!(prose.contains("`missing` means Git still has this worktree registered"));
        assert!(prose.contains("`prunable` means Git considers the registration stale"));
        assert!(prose.contains("both missing and prunable"));
    }

    /// The changed-file list is the hardest case: read-only, a direct primary
    /// action, and a contextual menu whose mnemonics must not hide globals.
    #[test]
    fn a_scoped_read_only_view_documents_what_only_it_does() {
        let rendered = render(
            HelpTopic::GitStatus,
            GrammarKind::Runyte,
            BindingScope::GitStatus,
            default_keymap(),
            true,
        );

        assert!(rendered.starts_with("Help · RUNYTE · GIT STATUS · Read-only"));
        assert!(rendered.contains("Text edits are refused in this view"));

        for section in ["Buffer keys", "Where to start", "Direct keys"] {
            assert!(rendered.contains(section), "{section} missing");
        }
        assert!(!rendered.contains("Different here"), "{rendered}");

        assert!(rendered.contains("Tab opens the action menu"));
        assert!(rendered.contains("Tab s"));
        assert!(rendered.contains("Stage every file the selection covers"));

        // Search remains global in the view, while mutations that could only
        // refuse are omitted from its direct-key section.
        let direct = &rendered[rendered.find("Direct keys").expect("a direct section")..];
        assert!(direct.contains("Search for text, ignoring case"));
        assert!(!direct.contains("Delete the selection or character"));
        assert!(!direct.contains("Undo the last change"));
        assert!(!direct.contains("Replace the selection, or paste after the caret"));

        // Prefixes are named, and chords are grouped where someone would look.
        assert!(rendered.contains("Space …"));
        assert!(rendered.contains("Application commands"));
        assert!(rendered.contains("Ctrl chords"));
        assert!(rendered.contains("Ctrl-o"));
        assert!(rendered.contains("Ctrl-i"));
    }

    /// The stash list's own keys never create a stash, so its help is the one
    /// place the three creation commands are compared. They differ along two
    /// axes at once, and `-tracked` and `-all` differ along only the second,
    /// which is exactly what a reader gets wrong from prose.
    #[test]
    fn the_stash_document_compares_the_creation_commands() {
        let rendered = render(
            HelpTopic::GitStash,
            GrammarKind::Runyte,
            BindingScope::GitStash,
            default_keymap(),
            true,
        );

        let table = rendered
            .find("Creating a stash")
            .expect("the creation table");
        for command in [
            ":git-stash-tracked",
            ":git-stash-all",
            ":git-stash-untracked",
        ] {
            assert!(rendered.contains(command), "{command} missing");
        }
        // The two that stash identical content are told apart by what they
        // leave behind, which is the distinction prose kept losing.
        assert!(rendered.contains("staged changes, still"));

        // The row actions are named as commands too. This view's keys reach
        // only those, so naming just the keys would imply the list is the
        // whole stash surface.
        for command in [":git-stashes", ":git-stash-apply", ":git-stash-drop"] {
            assert!(rendered.contains(command), "{command} missing");
        }

        // It sits above the derived key tables: what this view cannot do for
        // you is worth more than the keys it can.
        let keys = rendered.find("Buffer keys").expect("a key section");
        assert!(table < keys, "the table belongs above the key tables");
    }

    /// The rows are aligned against one another rather than wrapped, so they
    /// have to fit a conventional terminal and stay in their columns.
    #[test]
    fn the_stash_table_stays_within_its_columns() {
        let rows = HelpTopic::GitStash.table_for(GrammarKind::Runyte);
        let separator = rows
            .iter()
            .position(|row| row.trim_start().starts_with("--"))
            .expect("a rule under the header");
        // The gap before the first rule is the block's own indent, not a
        // column boundary, so only the inner gaps count.
        let columns = rows[separator]
            .match_indices("  -")
            .filter(|(index, _)| *index > 0)
            .map(|(index, _)| index + 2)
            .collect::<Vec<_>>();
        assert_eq!(columns.len(), 2, "three columns have two inner starts");

        // The heading and its lead-in are prose above the grid; the column
        // rule is the first row that has to line up with anything.
        for row in &rows[separator - 1..] {
            assert!(row.chars().count() <= 78, "too wide for a terminal: {row}");
            // Every cell either starts exactly on its column or is blank
            // there, which is what makes the table readable down a column.
            for start in &columns {
                let Some(boundary) = row.get(start - 1..*start) else {
                    continue;
                };
                assert_eq!(boundary, " ", "column {start} is crowded: {row}");
            }
        }
    }

    /// Only a topic with a genuine two-axis comparison gets a table. The rest
    /// answer with prose, so an empty list is the right default rather than
    /// something each new topic has to remember to write.
    #[test]
    fn tables_are_the_exception_rather_than_the_rule() {
        let tabled = HelpTopic::ALL
            .iter()
            .filter(|topic| !topic.table_for(GrammarKind::Runyte).is_empty())
            .collect::<Vec<_>>();
        assert_eq!(tabled, vec![&HelpTopic::GitStash]);
    }

    /// Read-only-ness is stated in the title of every document, in the same
    /// place, rather than left to whichever paragraph happens to mention it.
    #[test]
    fn the_title_states_whether_the_view_can_be_edited() {
        for topic in HelpTopic::ALL {
            let read_only = topic.title_for(GrammarKind::Runyte, true);
            let editable = topic.title_for(GrammarKind::Runyte, false);
            assert!(
                read_only.trim().ends_with("· Read-only"),
                "{topic:?}: {read_only}"
            );
            assert!(!editable.contains("Read-only"), "{topic:?}: {editable}");
            // The buffer type is named the same way either way, so the
            // suffix is the only difference between them.
            assert_eq!(
                read_only.trim().trim_end_matches("· Read-only").trim(),
                editable.trim()
            );
        }
    }

    #[test]
    fn an_action_only_scope_has_one_gap_before_its_tab_explanation() {
        let rendered = render(
            HelpTopic::Diff,
            GrammarKind::Runyte,
            BindingScope::Diff,
            default_keymap(),
            true,
        );
        assert!(
            rendered.contains("  Only this view answers to these.\n\n  Tab opens the action menu.")
        );
        assert!(
            !rendered
                .contains("  Only this view answers to these.\n\n\n  Tab opens the action menu.")
        );
    }

    /// An editable buffer keeps the keys a read-only one drops.
    #[test]
    fn an_editable_view_keeps_its_editing_keys() {
        let rendered = render(
            HelpTopic::Text,
            GrammarKind::Runyte,
            BindingScope::Global,
            default_keymap(),
            false,
        );

        assert!(rendered.starts_with("Help · RUNYTE · TEXT\n"));
        assert!(!rendered.contains("Read-only"));
        assert!(!rendered.contains("Text edits are refused"));
        assert!(rendered.contains("Delete the selection or character"));
        assert!(rendered.contains("Undo the last change"));
        // Nothing is scope-specific in a plain text buffer, so neither
        // buffer-specific section appears at all.
        assert!(!rendered.contains("Buffer keys"));
        assert!(!rendered.contains("Different here"));
    }

    /// A read-only view's prose still contains "a" and possessive "'s" as
    /// ordinary English, even though `a` (append) and `s` (search) are real
    /// bound keys elsewhere in the keymap. Neither should pick up the
    /// keyword colour: `a` never appears in this view's own tables because
    /// it is refused here, and the `s` in a possessive is not a bare key
    /// mention at all.
    #[test]
    fn prose_articles_and_possessives_are_not_mistaken_for_keys() {
        let rendered = render_document(
            HelpTopic::GitStatus,
            GrammarKind::Runyte,
            BindingScope::GitStatus,
            default_keymap(),
            true,
        );
        let text = rendered.text();
        // Checks the scope of a match's own first character, so the needle
        // must start exactly on the letter under test.
        let scope_at = |needle: &str| {
            let byte = text
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle:?}"));
            let offset = text[..byte].chars().count();
            rendered
                .spans()
                .iter()
                .find(|span| span.from <= offset && span.to > offset)
                .map(|span| span.scope.name())
        };

        assert_eq!(scope_at("a commit would take it"), None);
        // The needle starts on the "s" itself — the letter right after the
        // apostrophe in "file's" — not on "file".
        assert_eq!(scope_at("s changes, after a confirmation"), None);
    }

    /// The registry's spelling of a key is only trustworthy in the key
    /// column. Everywhere else, `a` is an article, `A` opens a sentence, and
    /// `Left`/`Right` name mouse buttons — all of them bound keys, none of
    /// them key mentions. A writable text view is the strongest case: it
    /// prints every one of those keys in its own tables, so the earlier rule
    /// of "mark what the tables printed" would still colour all of them.
    #[test]
    fn prose_never_wears_a_key_colour_the_tables_alone_earned() {
        let rendered = render_document(
            HelpTopic::Text,
            GrammarKind::Runyte,
            BindingScope::Global,
            default_keymap(),
            false,
        );
        let text = rendered.text();
        let scope_at = |needle: &str| {
            let byte = text
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle:?}"));
            let offset = text[..byte].chars().count();
            rendered
                .spans()
                .iter()
                .find(|span| span.from <= offset && span.to > offset)
                .map(|span| span.scope.name())
        };

        assert_eq!(scope_at("a selection-first modal editor"), None);
        assert_eq!(scope_at("a cursor on every match"), None);
        assert_eq!(scope_at("A reviewed terminal accepts"), None);
        assert_eq!(scope_at("Left click focuses"), None);
        assert_eq!(scope_at("Right-clicking any current selection"), None);

        // The same letters keep the key colour where they are keys: in the
        // generated key column, and in the paragraphs that name them.
        assert_eq!(scope_at("a          "), Some("keyword"));
        assert_eq!(scope_at("v enters SELECT mode"), Some("keyword"));
        assert_eq!(scope_at("n and N then select"), Some("keyword"));
        assert_eq!(scope_at("N then select"), Some("keyword"));
        assert_eq!(scope_at("Space b d compares"), Some("keyword"));
        assert_eq!(scope_at("Space r reloads"), Some("keyword"));
    }

    /// A prefix row's ellipsis says the key opens onto more keys; it is not
    /// something anyone presses, so the key colour stops at the key.
    #[test]
    fn a_prefix_rows_ellipsis_stays_outside_its_key() {
        let rendered = render_document(
            HelpTopic::Text,
            GrammarKind::Runyte,
            BindingScope::Global,
            default_keymap(),
            true,
        );
        let text = rendered.text();
        let byte = text.find("Z …").expect("the Z prefix row");
        let offset = text[..byte].chars().count();
        let scope_at = |offset: usize| {
            rendered
                .spans()
                .iter()
                .find(|span| span.from <= offset && span.to > offset)
                .map(|span| span.scope.name())
        };

        assert_eq!(scope_at(offset), Some("keyword"));
        assert_eq!(scope_at(offset + 2), None);
    }

    #[test]
    fn contextual_schema_styles_registry_keys_and_authored_technical_text() {
        let rendered = render_document(
            HelpTopic::GitBranches,
            GrammarKind::Runyte,
            BindingScope::GitBranches,
            default_keymap(),
            true,
        );
        let scope_at = |needle: &str| {
            let byte = rendered
                .text()
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle:?}"));
            let offset = rendered.text()[..byte].chars().count();
            rendered
                .spans()
                .iter()
                .find(|span| span.from <= offset && span.to > offset)
                .map(|span| span.scope.name())
        };

        assert_eq!(
            scope_at("Help · RUNYTE · GIT BRANCHES"),
            Some("markup.heading")
        );
        assert_eq!(scope_at(":help"), Some("function"));
        assert_eq!(scope_at("Space ?"), Some("keyword"));
        assert_eq!(scope_at("/local/path"), Some("string"));
        assert_eq!(scope_at("editor.mouse"), Some("markup.raw"));
    }

    fn plugin_topic() -> crate::plugin::help::Topic {
        crate::plugin::help::Topic {
            id: "rows".into(),
            title: "Rows".into(),
            paragraphs: vec![
                "Rows shows one page of a table. Press `Tab` for paging; Enter opens a record.".into(),
                "{key:nope} and {{binding:x} are text, not markers. Mouse and Buffer keys are words here.".into(),
            ],
        }
    }

    fn plugin_document(page: &PluginPage<'_>) -> HelpDocument {
        render_document_with_descriptions(
            HelpTopic::Text,
            GrammarKind::Runyte,
            BindingScope::Plugin(0),
            default_keymap(),
            true,
            Some(page),
            |_| None,
        )
    }

    fn roles_at(document: &HelpDocument, needle: &str) -> Vec<&'static str> {
        let start = document.text()[..document.text().find(needle).unwrap()]
            .chars()
            .count();
        let end = start + needle.chars().count();
        document
            .spans()
            .iter()
            .filter(|span| span.from < end && span.to > start)
            .map(|span| span.scope.name())
            .collect()
    }

    /// A plugin topic replaces the overview, not the rest of the page: the
    /// trailer, mouse notes and generated tables still follow it.
    #[test]
    fn a_plugin_topic_replaces_the_overview_and_keeps_generated_sections() {
        let topic = plugin_topic();
        let page = PluginPage {
            application: "Database viewer",
            topic: Some(&topic),
            actions: vec![
                PluginAction {
                    label: "Refresh",
                    description: "Refresh",
                    group: None,
                },
                PluginAction {
                    label: "Next page",
                    description: "Show the next page of rows",
                    group: Some("Paging"),
                },
                PluginAction {
                    label: "Filter",
                    description: "Filter {key:rows}",
                    group: Some("Query {key:x}"),
                },
                PluginAction {
                    label: "Previous page",
                    description: "Show the previous page",
                    group: Some("Paging"),
                },
            ],
        };
        let document = plugin_document(&page);
        let text = document.text();
        assert!(text.starts_with("Help · DATABASE VIEWER · ROWS · Read-only\n"));
        assert!(text.contains("{key:nope} and {{binding:x} are text, not markers."));
        assert!(!text.contains("selection-first modal editor"));
        assert!(text.contains(":help opens the general Runyte manual"));
        assert!(text.contains("\nMouse\n"));
        // Ungrouped first, then sections in the order their first action has.
        let refresh = text.find("\n  Refresh\n").unwrap();
        let paging = text.find("\n  Paging\n").unwrap();
        let next = text
            .find("\n    Next page — Show the next page of rows\n")
            .unwrap();
        let previous = text
            .find("\n    Previous page — Show the previous page\n")
            .unwrap();
        let query = text
            .find("\n  Query {key:x}\n    Filter — Filter {key:rows}\n")
            .unwrap();
        assert!(refresh < paging && paging < next && next < previous && previous < query);
        assert!(text.find("Application actions").unwrap() < refresh);
        assert!(text.contains("  Tab opens the menu of these"));

        assert_eq!(
            roles_at(&document, "Help · DATABASE VIEWER"),
            ["markup.heading"]
        );
        assert_eq!(roles_at(&document, "Paging"), ["markup.heading"]);
        // The whole escaped group name, not one character short of it.
        let heading = document.text().find("\n  Query {key:x}\n").unwrap() + 3;
        let start = document.text()[..heading].chars().count();
        assert!(document.spans().iter().any(|span| span.from == start
            && span.to == start + "Query {key:x}".chars().count()
            && span.scope.name() == "markup.heading"));
        assert_eq!(
            roles_at(&document, "Application actions"),
            ["markup.heading"]
        );
        // Only the plugin's own backticks style its prose.
        assert_eq!(
            roles_at(&document, "`Tab`"),
            ["punctuation", "markup.raw", "punctuation"]
        );
        for word in ["Enter opens", "Mouse and Buffer keys", "Next page — "] {
            assert!(roles_at(&document, word).is_empty(), "{word}");
        }
    }

    #[test]
    fn a_plugin_view_without_a_topic_keeps_the_text_overview_and_lists_actions() {
        let page = PluginPage {
            application: "Tasks",
            topic: None,
            actions: vec![PluginAction {
                label: "toggle",
                description: "Toggle selected tasks",
                group: None,
            }],
        };
        let text = plugin_document(&page).text().to_owned();
        assert!(text.starts_with("Help · RUNYTE · TEXT · Read-only\n"));
        assert!(text.contains("selection-first modal editor"));
        assert!(text.contains("Application actions\n"));
        assert!(text.contains("\n  toggle — Toggle selected tasks\n"));

        let empty = PluginPage {
            application: "Tasks",
            topic: None,
            actions: vec![],
        };
        assert!(
            !plugin_document(&empty)
                .text()
                .contains("Application actions")
        );
    }
}
