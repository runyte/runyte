// SPDX-License-Identifier: MPL-2.0
//! Shared semantic UI actions. Input owners resolve actions before executing
//! them; literal input and structural option identifiers are never rewritten.

use crate::input::{KeyCode, KeyStroke, Modifiers};
use serde_yaml::Value;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

#[cfg(test)]
#[path = "../tests/ui_bindings.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Owner {
    Completion,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    Next,
    Previous,
    Accept,
    Cancel,
    CancelOrContinue,
    DismissAndContinue,
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Previous => "previous",
            Self::Accept => "accept",
            Self::Cancel => "cancel",
            Self::CancelOrContinue => "cancel-or-continue",
            Self::DismissAndContinue => "dismiss-and-continue",
        }
    }
    const ALL: [Self; 6] = [
        Self::Next,
        Self::Previous,
        Self::Accept,
        Self::Cancel,
        Self::CancelOrContinue,
        Self::DismissAndContinue,
    ];
}

#[derive(Clone, Debug)]
pub struct Bindings {
    completion: Arc<HashMap<KeyStroke, Action>>,
    labels: Arc<HashMap<Action, Vec<String>>>,
}

impl Default for Bindings {
    fn default() -> Self {
        static DEFAULTS: LazyLock<Bindings> = LazyLock::new(Bindings::built_in);
        DEFAULTS.clone()
    }
}

impl Bindings {
    fn built_in() -> Self {
        use Action::*;
        let mut completion = HashMap::new();
        // Legacy completion defaults match non-character keys regardless of
        // modifiers and Ctrl-character keys with additional modifiers. Expand
        // these authored policies once; configured bindings match exactly.
        for bits in 0..64 {
            let modifiers = Modifiers::from_bits(bits).expect("six-bit modifiers");
            for (code, action) in [
                (KeyCode::Escape, CancelOrContinue),
                (KeyCode::Down, Next),
                (KeyCode::Up, Previous),
                (KeyCode::Tab, Accept),
                (KeyCode::Enter, DismissAndContinue),
            ] {
                completion.insert(
                    KeyStroke::new(code, modifiers).canonical_for_binding(),
                    action,
                );
            }
            if modifiers.contains(Modifiers::CONTROL) {
                for (character, action) in [('c', Cancel), ('n', Next), ('p', Previous)] {
                    completion.insert(
                        KeyStroke::new(KeyCode::Char(character), modifiers).canonical_for_binding(),
                        action,
                    );
                }
            }
        }
        let labels = [
            (Next, vec!["Down", "Ctrl-n"]),
            (Previous, vec!["Up", "Ctrl-p"]),
            (Accept, vec!["Tab"]),
            (Cancel, vec!["Ctrl-c"]),
            (CancelOrContinue, vec!["Esc"]),
            (DismissAndContinue, vec!["Enter"]),
        ]
        .into_iter()
        .map(|(a, labels)| (a, labels.into_iter().map(str::to_owned).collect()))
        .collect();
        Self {
            completion: Arc::new(completion),
            labels: Arc::new(labels),
        }
    }
}

impl Bindings {
    pub fn lookup(&self, owner: Owner, key: KeyStroke) -> Option<Action> {
        match owner {
            Owner::Completion => self.completion.get(&key.canonical_for_binding()).copied(),
        }
    }

    pub fn label(&self, owner: Owner, action: Action) -> String {
        match owner {
            Owner::Completion => self
                .labels
                .get(&action)
                .filter(|v| !v.is_empty())
                .map_or_else(|| "unbound".into(), |labels| labels.join("/")),
        }
    }
}

pub(super) fn compile(section: Option<&Value>, errors: &mut Vec<String>) -> Bindings {
    let mut result = Bindings::default();
    let Some(section) = section else {
        return result;
    };
    let Some(owners) = section.as_mapping() else {
        errors.push("ui rejected: expected owner mappings".into());
        return result;
    };
    if owners.len() > 32 {
        errors.push("ui rejected: too many owners".into());
        return result;
    }
    for (owner, mapping) in owners {
        if owner.as_str() != Some("completion") {
            errors.push("ui rejected: unknown owner (expected completion)".into());
            continue;
        }
        let parsed = (|| -> Result<Bindings, String> {
            let mapping = mapping.as_mapping().ok_or("expected action mappings")?;
            if mapping.len() > Action::ALL.len() {
                return Err("too many actions".into());
            }
            let mut assignments = HashMap::new();
            for (name, value) in mapping {
                let action = Action::ALL
                    .into_iter()
                    .find(|a| Some(a.name()) == name.as_str())
                    .ok_or("unknown action")?;
                let keys = value
                    .as_sequence()
                    .ok_or("expected a list of keys; use [] to unbind")?;
                if keys.len() > 32 {
                    return Err("at most 32 keys per action".into());
                }
                let mut parsed = Vec::new();
                for key in keys {
                    let source = key.as_str().ok_or("keys must be strings")?;
                    if source.len() > 64 {
                        return Err("key spelling exceeds 64 bytes".into());
                    }
                    let key = KeyStroke::parse(source)?.canonical_for_binding();
                    if parsed.contains(&key) {
                        return Err("duplicate normalized key".into());
                    }
                    parsed.push(key);
                }
                assignments.insert(action, parsed);
            }
            let mut candidate = Bindings::default();
            Arc::make_mut(&mut candidate.completion)
                .retain(|_, action| !assignments.contains_key(action));
            for (action, keys) in assignments {
                Arc::make_mut(&mut candidate.labels)
                    .insert(action, keys.iter().map(|key| key.label()).collect());
                for key in keys {
                    if Arc::make_mut(&mut candidate.completion)
                        .insert(key, action)
                        .is_some()
                    {
                        return Err("overlapping action bindings".into());
                    }
                }
            }
            Ok(candidate)
        })();
        match parsed {
            Ok(candidate) => result = candidate,
            Err(error) => errors.push(format!(
                "ui.completion rejected: {error}; defaults retained"
            )),
        }
    }
    result
}
