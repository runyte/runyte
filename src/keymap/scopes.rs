// SPDX-License-Identifier: MPL-2.0
//! View names and compatibility action admission during the mode migration.

use super::{BindingAvailability, BindingScope, BindingTarget, actions};
use crate::command::Mode;

impl BindingScope {
    pub fn name(self) -> &'static str {
        match self {
            Self::Plugin(_) => "plugin",
            Self::Global => "global",
            Self::Media => "media",
            Self::PdfPages => "pdf-pages",
            Self::Directory => "directory",
            Self::DirectoryTree => "directory-tree",
            Self::Settings => "settings",
            Self::GitStatus => "git-status",
            Self::GitBranches => "git-branches",
            Self::GitConflicts => "git-conflicts",
            Self::GitConflictFile => "git-conflict-file",
            Self::GitConflictMarkdown => "git-conflict-markdown",
            Self::GitMergeReview => "git-merge-review",
            Self::GitMergeDetail => "git-merge-detail",
            Self::GitMergeInput => "git-merge-input",
            Self::GitWorktrees => "git-worktrees",
            Self::GitComparison => "git-comparison",
            Self::GitRevisionDiff => "git-revision-diff",
            Self::GitLog => "git-log",
            Self::GitNetwork => "git-network",
            Self::GitBlame => "git-blame",
            Self::GitStash => "git-stash",
            Self::WorkspaceSearch => "workspace-search",
            Self::Help => "help",
            Self::CommitMessage => "commit-message",
            Self::Diff => "diff",
            Self::Markdown => "markdown",
            Self::Terminal => "terminal",
            Self::SessionManager => "session-manager",
        }
    }
}

pub(crate) fn admit(scope: BindingScope, mode: Mode, name: &str) -> Option<BindingTarget> {
    use std::{collections::HashMap, sync::LazyLock};
    type Inventory = HashMap<(BindingScope, Mode), HashMap<&'static str, BindingTarget>>;
    static INVENTORY: LazyLock<Inventory> = LazyLock::new(|| {
        let defaults = super::default_keymap();
        let mut inventory = HashMap::new();
        for &scope in BindingScope::ALL {
            for mode in [
                Mode::Normal,
                Mode::Select,
                Mode::Insert,
                Mode::Replace,
                Mode::List,
            ] {
                let mut actions = defaults
                    .bindings_for_scope(mode, scope)
                    .filter(|binding| binding.availability == BindingAvailability::Implemented)
                    .filter(|binding| {
                        scope != BindingScope::Terminal
                            || mode != Mode::Insert
                            || binding.sequence.as_slice().first().is_some_and(|&first| {
                                defaults.terminal_input_route(first, true, false)
                                    == super::terminal_input::Route::EditorGrammar
                            })
                    })
                    .flat_map(|binding| {
                        std::iter::once(binding.target)
                            .chain(binding.program.iter().map(|action| action.target()))
                            .map(|target| (target.name(), target))
                    })
                    .collect::<HashMap<_, _>>();
                if matches!(mode, Mode::Normal | Mode::Select) {
                    actions.extend(
                        defaults
                            .context_actions(scope)
                            .map(|action| (action.target.name(), action.target)),
                    );
                }
                if !scope.owns_modal_input()
                    && mode != Mode::List
                    && !(scope == BindingScope::Terminal && mode == Mode::Insert)
                {
                    for &command in crate::command::EditorCommand::ALL {
                        if super::actions::editor_modes(command).contains(&mode) {
                            actions
                                .entry(command.metadata().name)
                                .or_insert(BindingTarget::Editor(command));
                        }
                    }
                }
                if mode == Mode::List {
                    let command = crate::command::EditorCommand::ListClearFilter;
                    actions.insert(command.metadata().name, BindingTarget::Editor(command));
                }
                // Inherited document defaults are not terminal/tree capabilities.
                // Runtime refusal and configured admission must agree even when
                // a refused document key remains inherited.
                if matches!(scope, BindingScope::Terminal | BindingScope::DirectoryTree) {
                    actions.retain(|_, target| super::admission::admits(scope, *target));
                }
                actions.retain(|_, target| match target {
                    BindingTarget::Editor(command) => {
                        !crate::command::INTERNAL_EDITOR_COMMANDS.contains(command)
                    }
                    _ => true,
                });
                inventory.insert((scope, mode), actions);
            }
        }
        inventory
    });
    INVENTORY.get(&(scope, mode))?.get(name).copied()
}

/// Additional owner-specific action names, generated from admission metadata.
pub(super) fn reference() -> String {
    use std::{collections::BTreeMap, fmt::Write};
    let mut out = String::from(
        "\n## Scoped actions\n\nUse an action name or null in `keys.bind.<mode>`. Named modes also support\nthe applicable shared actions above. These additional actions belong to the\nindicated buffer owner; raw UI input remains under `keys.ui`.\n\n",
    );
    let defaults = super::default_keymap();
    for &scope in BindingScope::ALL {
        let mut extra = BTreeMap::<&str, Vec<&str>>::new();
        for mode in [
            Mode::Normal,
            Mode::Select,
            Mode::Insert,
            Mode::Replace,
            Mode::List,
        ] {
            if ((scope == BindingScope::Global || scope == BindingScope::SessionManager)
                != (mode == Mode::List))
                || (scope.is_merge_review() && mode != Mode::Normal)
                || (scope == BindingScope::Terminal && mode == Mode::Replace)
            {
                continue;
            }
            let targets = defaults
                .bindings_for_scope(mode, scope)
                .flat_map(|binding| {
                    std::iter::once(binding.target)
                        .chain(binding.program.iter().map(|action| action.target()))
                })
                .chain(defaults.context_actions(scope).map(|action| action.target))
                .chain((mode == Mode::List).then_some(BindingTarget::Editor(
                    crate::command::EditorCommand::ListClearFilter,
                )));
            for target in targets {
                if admit(scope, mode, target.name()).is_none() {
                    continue;
                }
                if matches!(target, BindingTarget::Editor(command) if actions::editor_modes(command).contains(&mode))
                    || matches!(target, BindingTarget::Colon(_))
                {
                    continue;
                }
                let modes = extra.entry(target.name()).or_default();
                if !modes.contains(&actions::mode_name(mode)) {
                    modes.push(actions::mode_name(mode));
                }
            }
        }
        if extra.is_empty() {
            continue;
        }
        let _ = writeln!(out, "### {}\n", scope.name());
        for (name, modes) in extra {
            let _ = writeln!(out, "- `{name}`: {}", modes.join(", "));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[path = "../tests/scoped_bindings.rs"]
mod tests;

pub(super) fn validate_keys(
    scope: BindingScope,
    mode: Mode,
    sequence: &super::KeySequence,
    value: &serde_yaml::Value,
    base: &super::Keymap,
) -> Result<(), String> {
    use crate::input::{KeyCode, Modifiers};
    if mode == Mode::List && sequence.len() != 1 {
        return Err("list bindings require one key".into());
    }
    let first = sequence.as_slice()[0];
    if scope == BindingScope::Global
        && !value.is_null()
        && sequence.len() == 1
        && (first == base.window_prefix()
            || (matches!(mode, Mode::Normal | Mode::Select) && first == base.leader()))
    {
        return Err("key is reserved for the leader or window prefix".into());
    }

    if matches!(mode, Mode::Normal | Mode::Select)
        && !scope.is_merge_review()
        && scope != BindingScope::DirectoryTree
        && first.modifiers.is_empty()
        && matches!(first.code, KeyCode::Char('1'..='9'))
    {
        return Err("leading counts remain structural input".into());
    }

    if ((mode == Mode::List || (scope.is_merge_review() && scope != BindingScope::GitMergeInput))
        && sequence.as_slice().contains(&base.leader()))
        || sequence.as_slice().iter().skip(1).any(|key| {
            if scope.is_merge_review() {
                base.ui()
                    .lookup(super::ui::Owner::MergePrefix, *key)
                    .is_some()
            } else {
                !value.is_null()
                    && if matches!(mode, Mode::Normal | Mode::Select) {
                        base.ui()
                            .lookup(super::ui::Owner::GrammarPrefix, *key)
                            .is_some()
                    } else {
                        matches!(key.code, KeyCode::Escape | KeyCode::Backspace)
                    }
            }
        })
    {
        return Err("key is owned by overlay dismissal or prefix cancellation".into());
    }

    if (mode == Mode::List
                    && (matches!(first.code, KeyCode::Char('1'..='9' | ' '))
                        && !first
                            .modifiers
                            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)))
                    || (matches!(mode, Mode::Normal | Mode::Select)
                        && !scope.is_merge_review()
                        && scope != BindingScope::DirectoryTree
                        && first.code == KeyCode::Tab
                        && first.modifiers.is_empty()
                        && base.ui().lookup(super::ui::Owner::ContextLauncher, first).is_some())
                    || (scope == BindingScope::GitMergeInput
                        && sequence.as_slice().iter().any(|key| {
                            base.ui().lookup(super::ui::Owner::MergeAcknowledgment, *key).is_some()
                                || matches!(key.code, KeyCode::Char(c) if c.is_control() || key.modifiers.is_empty() || key.modifiers == Modifiers::SHIFT)
                        }))
                {
                    return Err(
                        "key is owned by structural input or an unmigrated UI action".into(),
                    );
                }
    Ok(())
}
