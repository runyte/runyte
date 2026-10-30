// SPDX-License-Identifier: MPL-2.0
//! Shared ownership of the first key typed into a terminal in Insert mode.

use super::{BindingScope, BindingTarget, Key, Keymap, effective};
use crate::command::{EditorCommand, Mode};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Child,
    EditorGrammar,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Index {
    claims: HashMap<Key, Claim>,
    suppressed: std::collections::HashSet<Key>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Claim {
    unconditional: bool,
    persistent: bool,
    fast_pane: bool,
}

impl Index {
    pub(super) fn build(map: &Keymap) -> Self {
        let mut index = Self::default();
        let overrides = map
            .overrides
            .governing(Mode::Insert, BindingScope::Terminal);
        for (sequence, &(_, assigned)) in &overrides {
            if !assigned && sequence.len() == 1 {
                index.suppressed.insert(sequence.as_slice()[0]);
            }
        }
        for position in effective::indices(
            &map.bindings,
            &map.overrides,
            Mode::Insert,
            BindingScope::Terminal,
        ) {
            let binding = &map.bindings[position];
            let Some(&first) = binding.sequence.as_slice().first() else {
                continue;
            };
            let explicit = overrides
                .get(&binding.sequence)
                .is_some_and(|(_, assigned)| *assigned);
            let claim = index.claims.entry(first).or_default();
            // An explicit terminal assignment owns its prefix even when its
            // command will report that a persistent session is unavailable.
            claim.unconditional |= explicit
                || first == map.window_prefix()
                || super::is_terminal_normal_key(first)
                || (binding.actions.is_empty()
                    && binding.target == BindingTarget::Editor(EditorCommand::ClipboardYank));
            claim.persistent |= matches!(
                binding.target,
                BindingTarget::Editor(
                    EditorCommand::NextRunningSession
                        | EditorCommand::PreviousRunningSession
                        | EditorCommand::PreviousSession
                )
            );
            claim.fast_pane |= super::is_fast_pane_key(first);
        }
        index
    }
}

impl Keymap {
    /// Resolve first-key ownership without allocating. Pending sequences are
    /// already owned by the editor grammar and do not pass through this gate.
    pub fn terminal_input_route(&self, key: Key, persistent: bool, fast_pane: bool) -> Route {
        let key = key.canonical_for_binding();
        if self.terminal_input.claims.get(&key).is_some_and(|claim| {
            claim.unconditional
                || (persistent && claim.persistent)
                || (fast_pane && claim.fast_pane)
        }) {
            return Route::EditorGrammar;
        }
        // Legacy policies capture these physical keys even if keys.rebind
        // removed their exact command. Scoped null explicitly opts out. A
        // surviving longer binding above still owns its prefix.
        if !self.terminal_input.suppressed.contains(&key)
            && (super::is_terminal_normal_key(key)
                || key == self.window_prefix()
                || (fast_pane && super::is_fast_pane_key(key)))
        {
            Route::EditorGrammar
        } else {
            Route::Child
        }
    }
}
