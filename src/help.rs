// SPDX-License-Identifier: MPL-2.0

//! Contextual help for the view under the cursor.
//!
//! Only the prose lives here. Every key this module names is read out of the
//! keymap registry at render time, so help cannot drift from what the keys
//! actually do — a hand-maintained key table is a second source of truth, and
//! the one that goes stale.
//!
//! The rendered document is plain text because it is opened as an ordinary
//! read-only buffer. That is what makes help searchable, scrollable, and
//! splittable without this module knowing anything about drawing.

use std::fmt::Write as _;

use crate::{
    command::{EditorCommand, GrammarKind, Mode},
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
            BindingScope::Help | BindingScope::Global => Self::Text,
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
                "Search selects every match at once. With two or more characters selected, s and / search only inside the selection, leaving a cursor on every match; n and N then select only one result and step through them.",
                "Press Space and pause to explore command groups without memorising the full keymap.",
                "An ordinary file changed outside Runyte keeps its in-memory text and gains [STALE]. Space b d compares a fresh disk snapshot without discarding edits; Space r reloads, asking first whenever the buffer is dirty.",
                "Below the editor area, the global status line reports editor state and unread notification counts. The interaction line below it is reserved for active prompts and the last action echo; :notifications or :not opens complete retained feedback.",
            ],
            Self::Explorer => &[
                "The explorer is an editable directory listing. Move and edit here just as you do in a text buffer.",
                "Edits do not touch the filesystem until you review and confirm the write plan.",
                "A symlink carries a muted → target hint that is not part of the text. Enter opens what the link points at; renaming and deleting stay with the link.",
            ],
            Self::Config => &[
                "The config page is a read-only view of the setting registry. Search, select, split, and move through it like any other text buffer; Enter changes the setting on the current row.",
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
                "Enter opens the typed path and source range represented by the current result row. The clean result remains available while it is among the two most recently active special buffers; run workspace search again for fresh results.",
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
    "This pane is running a program on a pseudoterminal. INSERT sends Escape, Ctrl-c, Ctrl-o, Space, and ordinary keys to it. Ctrl-\\ leaves input for live NORMAL; pressing it again captures review. Ctrl-w instead starts window commands directly; h/j/k/l and their arrow/control aliases move immediately. A live terminal destination enters INSERT, a reviewed terminal stays in NORMAL/review, and a document enters NORMAL; w uses the same destination behavior and v/s split without capturing review.",
    "Directional pane movement never starts or discards review. i resumes input, and canceling a Ctrl-w prefix leaves this terminal in INSERT.",
    "Live NORMAL keeps showing current output without sending keys to the child. A second Ctrl-\\ or the first review operation captures a stable snapshot; live output continues behind it. Move with ordinary motions, press v to extend, x/X to select and walk whole lines, or C/Alt-C to add carets below/above at the same occupied cell column. Escape cancels a selection made either way. y copies every selection to the unnamed register and Space c y uses the system clipboard. Ctrl-u/Ctrl-d and Ctrl-b/Ctrl-f scroll; s and / search, n/N move among matches, and p sends clipboard text to the live child. u takes that paste back with one delete per character, while it is still the child's last input and did not end a line it has run.",
    "For real Runyte editing over what a terminal printed, copy its output into a buffer: that text is an ordinary read-only document where search, multiple selections, and yank all work.",
    "Composing goes the other way. Write the text in an ordinary buffer with every editing command available, then send the selection — or the whole buffer, with nothing selected — to a terminal as one bracketed paste. That is the only way modal editing can reach a program that owns its own input area.",
    "The pane's buffer is still there behind the terminal, and leaving the terminal shows it again without ending the program. Closing the pane or opening a file in it does the same; the session keeps running and the terminal list reaches it.",
];

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
    // Normal and Select bind the same sequences to the same commands, so
    // either answers for both. `normal_and_select_bind_the_same_sequences` in
    // keymap.rs fails if that stops being true, since this would then be
    // quietly documenting half the keymap.
    let mode = Mode::Normal;
    let mut out = String::new();
    let _ = writeln!(out, "{}\n", topic.title_for(grammar, read_only).trim());

    for paragraph in topic.overview_for(grammar) {
        let _ = writeln!(out, "{paragraph}\n");
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
    let _ = writeln!(
        out,
        ":help opens the general Runyte manual; :help <topic> jumps to one of\n\
         its sections. This contextual page remains available through Space ?.\n"
    );

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

    let scoped = keymap.scoped_bindings(mode, scope).collect::<Vec<_>>();
    let actions = keymap.context_actions(scope).collect::<Vec<_>>();
    if !scoped.is_empty() || !actions.is_empty() {
        let _ = writeln!(out, "Buffer keys");
        let _ = writeln!(out, "  Only this view answers to these.\n");
        for binding in &scoped {
            row(&mut out, &binding.sequence.to_string(), binding.description);
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
                row(
                    &mut out,
                    &format!("Tab {}", action.mnemonic.label()),
                    action.description,
                );
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
            row(
                &mut out,
                &format!("{} …", entry.key.label()),
                entry.description,
            );
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
                row(&mut out, entry.key.label().as_str(), entry.description);
            }
            out.push('\n');
        }
    }

    out
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
        })
}

/// Whether the registry in force binds the bare pane moves.
///
/// Asked of the keymap rather than of configuration, because the keymap is
/// what this page is describing.
fn fast_pane_keys_are_active(keymap: &Keymap) -> bool {
    matches!(
        keymap.lookup_in(
            Mode::Insert,
            BindingScope::Global,
            &KeySequence::from(Key::ctrl('h')),
        ),
        Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::FocusWindowLeft)
    )
}

fn row(out: &mut String, keys: &str, description: &str) {
    let padding = KEY_COLUMN.saturating_sub(keys.chars().count());
    let _ = writeln!(out, "  {keys}{}{description}", " ".repeat(padding.max(1)));
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
    "The branch list shows local branches, with the current branch marked by an asterisk. A `[worktree: /local/path]` note identifies every registered checkout of a branch.",
    "A branch that tracks a remote one carries its drift in brackets: `[↑2 ↓1]` is two commits it has that the upstream does not and one the upstream has that it does not, `[=]` is in step, and `[gone]` is an upstream that no longer exists. A branch tracking nothing says nothing.",
    "Checking out a branch is refused while the working tree, index, or an open file buffer has uncommitted changes. When any terminal session is still running, type the exact target branch name to acknowledge that its job will keep using the working directory while Git replaces files.",
    "`Tab n` starts a new branch at the selected one and switches to it, under the same rules, including exact-name confirmation for a live terminal. `Tab D` reviews the selected branch: Enter is enough when an upstream or another local branch retains its tip; otherwise type the exact branch name. Cached upstream state reflects the last fetch.",
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
    "`Tab n` creates a checkout of the selected branch at an explicit destination. `Tab N` first names a new branch, then asks for its destination.",
    "`Tab D` removes one ordinary worktree after confirmation, leaving its branch. It refuses Git changes and unsaved persistent-session buffers; unpublished tracked or unretained detached history needs the exact branch name or displayed path. Current, locked, bare, and unavailable worktrees are also refused.",
    "A clean session on the worktree is stopped and forgotten rather than refusing the removal. The confirmation names it and asks for the exact branch name for that reason alone. The session stops before Git is asked to remove the directory it owns, and the removal has to succeed before its record is forgotten, so a refusal at either point leaves everything below it alone. This happens in standalone mode too: stopping that host is part of the removal, not a `session` command.",
];

const GIT_LOG_OVERVIEW: &[&str] = &[
    "The log loads up to 10,000 commits per page in topological order. Rows keep their full commit object identity even though the display uses an abbreviation.",
    "Enter opens the selected commit's bounded metadata and patch. `Ctrl-n` and `Ctrl-p` load the next and previous object-cursor pages; `Space g r` reconciles the view without taking the replace-character key.",
];

const GIT_BLAME_OVERVIEW: &[&str] = &[
    "Blame was computed from the live buffer text, so unsaved lines are shown as uncommitted instead of being attributed to older disk content.",
    "Enter opens the commit on a committed row. Uncommitted rows deliberately have no historical target.",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::default_keymap;

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
    fn the_terminal_topic_distinguishes_live_and_reviewed_focus_destinations() {
        let prose = HelpTopic::Terminal
            .overview_for(GrammarKind::Runyte)
            .join(" ");

        assert!(prose.contains("A live terminal destination enters INSERT"));
        assert!(prose.contains("a reviewed terminal stays in NORMAL/review"));
        assert!(prose.contains("never starts or discards review"));
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
        assert!(!direct.contains("Paste after the selection"));

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
}
