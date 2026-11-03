// SPDX-License-Identifier: MPL-2.0
//! One effective view for dispatch, discovery, and validation.

use super::{Binding, BindingScope, KeySequence, scope_includes};
use crate::command::Mode;
use std::collections::{HashMap, HashSet};

/// True is an explicit assignment, false an exact inherited-key suppression.
/// Keep identity rather than binding indices so plugin rebuilds retain it.
#[derive(Clone, Debug, Default)]
pub(super) struct Overrides(pub HashMap<(Mode, BindingScope), HashMap<KeySequence, bool>>);

impl Overrides {
    pub fn governing(
        &self,
        mode: Mode,
        scope: BindingScope,
    ) -> HashMap<&KeySequence, (BindingScope, bool)> {
        let mut result = HashMap::new();
        for &parent in BindingScope::ALL {
            if parent != scope
                && scope_includes(scope, parent)
                && let Some(entries) = self.0.get(&(mode, parent))
            {
                for (sequence, &assigned) in entries {
                    result.insert(sequence, (parent, assigned));
                }
            }
        }
        if let Some(entries) = self.0.get(&(mode, scope)) {
            for (sequence, &assigned) in entries {
                result.insert(sequence, (scope, assigned));
            }
        }
        result
    }

    pub fn affects(
        &self,
        mode: Mode,
        scope: BindingScope,
        sequence: &KeySequence,
        descendants: bool,
    ) -> bool {
        self.0.iter().any(|(&(entry_mode, owner), entries)| {
            entry_mode == mode
                && (owner == BindingScope::Global || scope_includes(scope, owner))
                && entries.keys().any(|changed| {
                    changed == sequence || (descendants && changed.starts_with(sequence))
                })
        })
    }

    pub fn contains(&self, mode: Mode, scope: BindingScope, sequence: &KeySequence) -> bool {
        self.0
            .get(&(mode, scope))
            .is_some_and(|entries| entries.contains_key(sequence))
    }
}

pub(super) fn indices(
    bindings: &[Binding],
    overrides: &Overrides,
    mode: Mode,
    scope: BindingScope,
) -> Vec<usize> {
    let governing = overrides.governing(mode, scope);
    let admitted = |binding: &Binding| {
        governing
            .get(&binding.sequence)
            .is_none_or(|&(owner, assigned)| assigned && owner == binding.scope)
    };
    let mut scoped = bindings
        .iter()
        .enumerate()
        .filter(|(_, binding)| {
            scope != BindingScope::Global
                && binding.is_active_in(mode)
                && scope_includes(scope, binding.scope)
                && admitted(binding)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    scoped.sort_by_key(|&index| bindings[index].role);
    let shadowed = scoped
        .iter()
        .map(|&index| &bindings[index].sequence)
        .collect::<HashSet<_>>();
    let mut global = bindings
        .iter()
        .enumerate()
        .filter(|(_, binding)| {
            binding.scope == BindingScope::Global
                && binding.visible_in(mode, scope)
                && admitted(binding)
                && !shadowed.contains(&binding.sequence)
                && !(scope == BindingScope::DirectoryTree
                    && shadowed
                        .iter()
                        .any(|sequence| binding.sequence.starts_with(sequence)))
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    global.sort_by_key(|&index| bindings[index].role);
    scoped.extend(global);
    scoped
}
