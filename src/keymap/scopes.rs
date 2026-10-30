// SPDX-License-Identifier: MPL-2.0
//! Atomic configuration of the inherited editor/list scope graph.

use super::{
    Binding, BindingAvailability, BindingScope, BindingTarget, Key, KeySequence, Keymap, actions,
    scope_includes, validate,
};
use crate::{
    command::Mode,
    input::{KeyCode, Modifiers},
};
use serde_yaml::Value;
use std::collections::HashSet;

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

pub(super) fn apply(section: Option<&Value>, base: Keymap, errors: &mut Vec<String>) -> Keymap {
    let Some(section) = section else {
        return base;
    };
    match compile(section, &base) {
        Ok(candidate) => candidate,
        Err(reason) => {
            errors.push(format!(
                "scopes rejected: all scoped overrides rolled back: {reason}"
            ));
            base
        }
    }
}

fn compile(section: &Value, base: &Keymap) -> Result<Keymap, String> {
    let scopes = section.as_mapping().ok_or("expected scope mappings")?;
    if scopes.len() > BindingScope::ALL.len() {
        return Err("too many scopes".into());
    }
    let mut candidate = base.clone();
    let mut assignments = Vec::new();
    let mut total = 0;
    for (name, modes) in scopes {
        let scope = BindingScope::ALL
            .iter()
            .copied()
            .find(|scope| Some(scope.name()) == name.as_str())
            .ok_or("unknown scope")?;
        let modes = modes.as_mapping().ok_or("expected mode mappings")?;
        if modes.len() > 5 {
            return Err("too many modes".into());
        }
        for (name, mapping) in modes {
            let mode = match name.as_str() {
                Some("normal") => Mode::Normal,
                Some("select") => Mode::Select,
                Some("insert") => Mode::Insert,
                Some("replace") => Mode::Replace,
                Some("list") => Mode::List,
                _ => return Err("unknown scope mode".into()),
            };
            if (scope == BindingScope::Global && mode != Mode::List)
                || (scope == BindingScope::SessionManager && mode != Mode::List)
                || (scope != BindingScope::Global
                    && scope != BindingScope::SessionManager
                    && mode == Mode::List)
            {
                return Err("global and session-manager support list; editor scopes support editor modes (use keys.bind for global editor bindings)".into());
            }
            if (scope.is_merge_review() && mode != Mode::Normal)
                || (scope == BindingScope::Terminal && mode == Mode::Replace)
            {
                return Err(
                    "mode belongs to an input owner awaiting scoped dispatch migration".into(),
                );
            }
            let mapping = mapping.as_mapping().ok_or("expected key mappings")?;
            total += mapping.len();
            if total > actions::MAX_BINDINGS {
                return Err("too many scoped assignments".into());
            }
            let mut seen = HashSet::new();
            for (source, value) in mapping {
                let source = source
                    .as_str()
                    .filter(|source| source.len() <= 512)
                    .ok_or("key must be a string of at most 512 bytes")?;
                let tokens = source.split_whitespace().collect::<Vec<_>>();
                if tokens.is_empty()
                    || tokens.len() > 8
                    || (mode == Mode::List && tokens.len() != 1)
                {
                    return Err("expected 1 through 8 keys, or one key in list mode".into());
                }
                let sequence = KeySequence::new(
                    tokens
                        .into_iter()
                        .map(|token| match token {
                            "Leader" => Ok(base.leader()),
                            "Window" => Ok(base.window_prefix()),
                            _ => Key::parse(token),
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| "invalid scoped key")?,
                );
                // These handlers still own structural input before/after the
                // registry. Reject misleading assignments and removals until
                // their action-owning surfaces are migrated.
                let first = sequence.as_slice()[0];
                if matches!(mode, Mode::Normal | Mode::Select)
                    && !scope.is_merge_review()
                    && scope != BindingScope::DirectoryTree
                    && first.modifiers.is_empty()
                    && matches!(first.code, KeyCode::Char('1'..='9'))
                {
                    return Err("leading counts remain structural input".into());
                }

                if ((mode == Mode::List
                    || (scope.is_merge_review() && scope != BindingScope::GitMergeInput))
                    && sequence.as_slice().contains(&base.leader()))
                    || sequence.as_slice().iter().skip(1).any(|key| {
                        if scope.is_merge_review() {
                            base.ui()
                                .lookup(super::ui::Owner::MergePrefix, *key)
                                .is_some()
                        } else {
                            matches!(key.code, KeyCode::Escape | KeyCode::Backspace)
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
                        && first.modifiers.is_empty())
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
                if !seen.insert(sequence.clone()) {
                    return Err("duplicate normalized scoped key".into());
                }
                let target = if value.is_null() {
                    None
                } else {
                    let name = value
                        .as_str()
                        .filter(|name| name.len() <= 128)
                        .ok_or("expected an action name or null")?;
                    Some(admit(scope, mode, name).ok_or_else(|| {
                        format!(
                            "action is unavailable in {} {}",
                            scope.name(),
                            actions::mode_name(mode)
                        )
                    })?)
                };
                candidate
                    .overrides
                    .0
                    .entry((mode, scope))
                    .or_default()
                    .insert(sequence.clone(), target.is_some());
                if let Some(target) = target {
                    assignments.push(Binding::implemented_in(
                        actions::mode_slice(mode),
                        scope,
                        sequence,
                        target,
                    ));
                }
            }
        }
    }
    // Two inherited parents have equal precedence. Require an explicit child
    // decision, even when one parent removes the key and the other assigns it.
    for &scope in BindingScope::ALL {
        for mode in [
            Mode::Normal,
            Mode::Select,
            Mode::Insert,
            Mode::Replace,
            Mode::List,
        ] {
            let mut inherited = HashSet::new();
            for &parent in BindingScope::ALL {
                if parent != scope
                    && scope_includes(scope, parent)
                    && let Some(entries) = candidate.overrides.0.get(&(mode, parent))
                {
                    for sequence in entries.keys() {
                        if !inherited.insert(sequence)
                            && !candidate.overrides.contains(mode, scope, sequence)
                        {
                            return Err(format!(
                                "conflicting inherited overrides in {} {}",
                                scope.name(),
                                actions::mode_name(mode)
                            ));
                        }
                    }
                }
            }
        }
    }
    candidate.bindings = candidate
        .bindings
        .into_iter()
        .flat_map(|binding| {
            let retained = binding
                .modes
                .iter()
                .copied()
                .filter(|&mode| {
                    !candidate
                        .overrides
                        .contains(mode, binding.scope, &binding.sequence)
                })
                .collect::<Vec<_>>();
            if retained.len() == binding.modes.len() {
                return vec![binding];
            }
            retained
                .into_iter()
                .map(|mode| {
                    let mut copy = binding.clone();
                    copy.modes = actions::mode_slice(mode);
                    copy
                })
                .collect::<Vec<_>>()
        })
        .collect();
    candidate.bindings.extend(assignments);
    let violations = validate::with_overrides(
        &candidate.bindings,
        &candidate.namespaces,
        &candidate.context_actions,
        &candidate.overrides,
    );
    if let Some(violation) = violations.first() {
        return Err(violation.message.clone());
    }
    candidate.rebuild_lookup_index();
    Ok(candidate)
}

/// Admit only actions already supported by this owner and mode. A scoped
/// spelling must not grant a surface another owner's internal transitions.
fn admit(scope: BindingScope, mode: Mode, name: &str) -> Option<BindingTarget> {
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
                    .map(|binding| (binding.target.name(), binding.target))
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
                actions.retain(|_, target| match target {
                    BindingTarget::Editor(command) => {
                        !crate::command::INTERNAL_EDITOR_COMMANDS.contains(command)
                            && !crate::command::GRAMMAR_ONLY_EDITOR_COMMANDS.contains(command)
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
        "\n## Scoped actions\n\nUse one action name or null in `keys.scopes.<scope>.<mode>`. Ordinary editor\nscopes also support the applicable global actions above. These additional\nactions belong to the indicated owner; raw UI input still has its own scope.\n\n",
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
                .map(|binding| binding.target)
                .chain(defaults.context_actions(scope).map(|action| action.target));
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
