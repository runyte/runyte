// SPDX-License-Identifier: MPL-2.0
//! Shared semantic UI actions. Owners execute actions without rewriting input;
//! literal input and structural option identifiers retain their own meaning.

mod picker;
mod prompt;

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
    ContextActions,
    Path,
    PathActions,
    ProgramActions,
    PluginManagerActions,
    BufferActions,
    TerminalActions,
    SessionActions,
    CommandPrompt,
    TextPrompt,
    PathPrompt,
    ProgramPrompt,
    FilePicker,
    Finder,
}

impl Owner {
    const ALL: [Self; 15] = [
        Self::Completion,
        Self::ContextActions,
        Self::Path,
        Self::PathActions,
        Self::ProgramActions,
        Self::PluginManagerActions,
        Self::BufferActions,
        Self::TerminalActions,
        Self::SessionActions,
        Self::CommandPrompt,
        Self::TextPrompt,
        Self::PathPrompt,
        Self::ProgramPrompt,
        Self::FilePicker,
        Self::Finder,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Completion => "completion",
            Self::ContextActions => "context-actions",
            Self::Path => "path",
            Self::PathActions => "path-actions",
            Self::ProgramActions => "program-actions",
            Self::PluginManagerActions => "plugin-manager-actions",
            Self::BufferActions => "buffer-actions",
            Self::TerminalActions => "terminal-actions",
            Self::SessionActions => "session-actions",
            Self::CommandPrompt => "command-prompt",
            Self::TextPrompt => "text-prompt",
            Self::PathPrompt => "path-prompt",
            Self::ProgramPrompt => "program-prompt",
            Self::FilePicker => "file-picker",
            Self::Finder => "finder",
        }
    }
    pub fn is_list_menu(self) -> bool {
        matches!(
            self,
            Self::BufferActions | Self::TerminalActions | Self::SessionActions
        )
    }
    pub fn is_picker(self) -> bool {
        matches!(self, Self::FilePicker | Self::Finder)
    }
    pub fn is_prompt(self) -> bool {
        matches!(
            self,
            Self::CommandPrompt | Self::TextPrompt | Self::PathPrompt | Self::ProgramPrompt
        )
    }
    fn inherits_leader(self) -> bool {
        self != Self::Completion && !self.is_prompt()
    }
    fn actions(self) -> &'static [Action] {
        use Action::*;
        match self {
            Self::Completion => &[
                Next,
                Previous,
                Accept,
                Cancel,
                CancelOrContinue,
                DismissAndContinue,
            ],
            Self::ContextActions | Self::PathActions | Self::ProgramActions => {
                &[Next, Previous, Accept, Cancel]
            }
            Self::BufferActions | Self::TerminalActions | Self::SessionActions => {
                &[Next, Previous, Accept, Back, Cancel]
            }
            Self::Path => &[OpenActions, Cancel],
            Self::PluginManagerActions => &[Cancel],
            Self::FilePicker | Self::Finder => picker::actions(self),
            Self::CommandPrompt | Self::TextPrompt | Self::PathPrompt | Self::ProgramPrompt => {
                prompt::actions(self)
            }
        }
    }
    fn structural(self, key: KeyStroke) -> bool {
        match self {
            Self::ContextActions => {
                key.modifiers.is_empty()
                    && matches!(key.code, KeyCode::Char(c) if c.is_ascii_alphabetic() && !matches!(c, 'j' | 'k'))
            }
            // This menu historically matches its choice character regardless
            // of modifiers. Preserve that broader structural input policy.
            Self::PathActions => matches!(key.code, KeyCode::Char('r' | 's')),
            Self::Completion
            | Self::Path
            | Self::ProgramActions
            | Self::BufferActions
            | Self::TerminalActions => false,
            Self::SessionActions => matches!(key.code, KeyCode::Char(c) if c.is_control()),
            Self::FilePicker | Self::Finder | Self::PluginManagerActions => {
                matches!(key.code, KeyCode::Char(c) if c.is_control())
                    || matches!(key.code, KeyCode::Char(_))
                        && !key
                            .modifiers
                            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
            }
            Self::CommandPrompt | Self::TextPrompt | Self::PathPrompt | Self::ProgramPrompt => {
                matches!(key.code, KeyCode::Char(c) if c.is_control())
                    || matches!(key.code, KeyCode::Char(_))
                        && !key
                            .modifiers
                            .intersects(Modifiers::CONTROL | Modifiers::ALT)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    Next,
    Previous,
    Accept,
    Cancel,
    CancelOrContinue,
    DismissAndContinue,
    OpenActions,
    Back,
    Save,
    MoveLeft,
    MoveRight,
    MoveStart,
    MoveEnd,
    WordLeft,
    WordRight,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteToStart,
    DeleteToEnd,
    Backspace,
    Delete,
    First,
    Last,
    Complete,
    PageUp,
    PageDown,
    SplitVertical,
    SplitHorizontal,
    TogglePreview,
    ToggleMode,
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
            Self::OpenActions => "open-actions",
            Self::Back => "back",
            Self::Save => "save",
            Self::MoveLeft => "move-left",
            Self::MoveRight => "move-right",
            Self::MoveStart => "move-start",
            Self::MoveEnd => "move-end",
            Self::WordLeft => "word-left",
            Self::WordRight => "word-right",
            Self::DeleteBackward => "delete-backward",
            Self::DeleteForward => "delete-forward",
            Self::DeleteWordBackward => "delete-word-backward",
            Self::DeleteToStart => "delete-to-start",
            Self::DeleteToEnd => "delete-to-end",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::First => "first",
            Self::Last => "last",
            Self::Complete => "complete",
            Self::PageUp => "page-up",
            Self::PageDown => "page-down",
            Self::SplitVertical => "split-vertical",
            Self::SplitHorizontal => "split-horizontal",
            Self::TogglePreview => "toggle-preview",
            Self::ToggleMode => "toggle-mode",
        }
    }
}

#[derive(Clone, Debug, Default)]
struct OwnerBindings {
    keys: Arc<HashMap<KeyStroke, Action>>,
    labels: Arc<HashMap<Action, Vec<String>>>,
    configured: std::collections::HashSet<Action>,
    without_leader: Option<Box<OwnerBindings>>,
}

#[derive(Clone, Debug)]
pub struct Bindings {
    owners: HashMap<Owner, OwnerBindings>,
}

impl Default for Bindings {
    fn default() -> Self {
        static DEFAULTS: LazyLock<Bindings> =
            LazyLock::new(|| Bindings::with_leader(KeyStroke::char(' ')));
        DEFAULTS.clone()
    }
}

impl Bindings {
    fn legacy() -> &'static Self {
        static DEFAULTS: LazyLock<Bindings> = LazyLock::new(|| {
            let owners = Owner::ALL
                .into_iter()
                .map(|owner| {
                    let mut keys = HashMap::new();
                    for bits in 0..64 {
                        let modifiers = Modifiers::from_bits(bits).expect("six-bit modifiers");
                        for code in [
                            KeyCode::Escape,
                            KeyCode::Down,
                            KeyCode::Up,
                            KeyCode::Tab,
                            KeyCode::BackTab,
                            KeyCode::Enter,
                            KeyCode::Char('c'),
                            KeyCode::Char('n'),
                            KeyCode::Char('p'),
                            KeyCode::Char('j'),
                            KeyCode::Char('k'),
                            KeyCode::PageUp,
                            KeyCode::PageDown,
                            KeyCode::Char('v'),
                            KeyCode::Char('t'),
                            KeyCode::Backspace,
                            KeyCode::Delete,
                            KeyCode::Home,
                            KeyCode::End,
                            KeyCode::Left,
                            KeyCode::Right,
                            KeyCode::Char('s'),
                            KeyCode::Char('b'),
                            KeyCode::Char('f'),
                            KeyCode::Char('a'),
                            KeyCode::Char('e'),
                            KeyCode::Char('w'),
                            KeyCode::Char('u'),
                            KeyCode::Char('h'),
                            KeyCode::Char('d'),
                        ] {
                            let key = KeyStroke::new(code, modifiers);
                            if let Some(action) = legacy_action(owner, key) {
                                keys.insert(key.canonical_for_binding(), action);
                            }
                        }
                    }
                    let labels = owner
                        .actions()
                        .iter()
                        .map(|&action| {
                            (
                                action,
                                legacy_labels(owner, action)
                                    .iter()
                                    .map(|s| (*s).to_owned())
                                    .collect(),
                            )
                        })
                        .collect();
                    (
                        owner,
                        OwnerBindings {
                            keys: Arc::new(keys),
                            labels: Arc::new(labels),
                            configured: Default::default(),
                            without_leader: None,
                        },
                    )
                })
                .collect();
            Bindings { owners }
        });
        &DEFAULTS
    }
    fn with_leader(leader: KeyStroke) -> Self {
        let mut defaults = Self::legacy().clone();
        for (&owner, bindings) in &mut defaults.owners {
            if owner.inherits_leader() {
                bindings.add_leader(leader);
            }
        }
        defaults
    }
    pub fn explicitly_configured(&self, owner: Owner, action: Action) -> bool {
        self.owners[&owner].configured.contains(&action)
    }
    pub fn lookup(&self, owner: Owner, key: KeyStroke) -> Option<Action> {
        self.lookup_with_leader(owner, key, true)
    }
    pub fn lookup_with_leader(
        &self,
        owner: Owner,
        key: KeyStroke,
        leader_active: bool,
    ) -> Option<Action> {
        self.owner_bindings(owner, leader_active)
            .keys
            .get(&key.canonical_for_binding())
            .copied()
    }
    pub fn label(&self, owner: Owner, action: Action) -> String {
        self.label_with_leader(owner, action, true)
    }
    pub fn label_with_leader(&self, owner: Owner, action: Action, leader_active: bool) -> String {
        self.owner_bindings(owner, leader_active)
            .labels
            .get(&action)
            .filter(|keys| !keys.is_empty())
            .map_or_else(|| "unbound".into(), |keys| keys.join("/"))
    }
    pub fn dismissal_label(&self, owner: Owner) -> String {
        let actions: &[Action] = if owner.is_list_menu() {
            &[Action::Back, Action::Cancel]
        } else {
            &[Action::Cancel]
        };
        let labels = actions
            .iter()
            .map(|&action| self.label(owner, action))
            .filter(|label| label != "unbound")
            .collect::<Vec<_>>();
        if labels.is_empty() {
            "unbound".to_owned()
        } else {
            labels.join("/")
        }
    }
    fn owner_bindings(&self, owner: Owner, leader_active: bool) -> &OwnerBindings {
        let bindings = &self.owners[&owner];
        if !leader_active {
            bindings.without_leader.as_deref().unwrap_or(bindings)
        } else {
            bindings
        }
    }
}

impl OwnerBindings {
    fn add_leader(&mut self, leader: KeyStroke) {
        self.without_leader = Some(Box::new(self.clone()));
        let leader = leader.canonical_for_binding();
        // The old frontend rewrote the effective leader to Escape before
        // owner dispatch. Preserve that precedence for inherited cancel only.
        Arc::make_mut(&mut self.keys).insert(leader, Action::Cancel);
        let labels = Arc::make_mut(&mut self.labels);
        for (&action, keys) in labels.iter_mut() {
            keys.retain(|source| {
                KeyStroke::parse(source)
                    .is_ok_and(|key| self.keys.get(&key.canonical_for_binding()) == Some(&action))
            });
        }
        // A default spelling can stand for a legacy modifier family. If
        // leader masks its usual spelling, advertise a surviving exact key
        // rather than claiming that the entire action is unbound.
        for (&action, labels) in labels.iter_mut() {
            if labels.is_empty() {
                let mut alternatives = self
                    .keys
                    .iter()
                    .filter(|(_, found)| **found == action)
                    .map(|(key, _)| key.label())
                    .filter(|label| {
                        KeyStroke::parse(label).is_ok_and(|key| {
                            self.keys.get(&key.canonical_for_binding()) == Some(&action)
                        })
                    })
                    .collect::<Vec<_>>();
                alternatives.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
                if let Some(label) = alternatives.into_iter().next() {
                    labels.push(label);
                }
            }
        }
        let cancel = labels.entry(Action::Cancel).or_default();
        if !cancel.iter().any(|source| {
            KeyStroke::parse(source).is_ok_and(|key| key.canonical_for_binding() == leader)
        }) {
            cancel.push(leader.label());
        }
    }
}

fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    use Action::*;
    if owner.is_prompt() {
        return prompt::legacy_action(owner, key);
    }
    if owner.is_picker() {
        return picker::legacy_action(owner, key);
    }
    let control = key.modifiers.contains(Modifiers::CONTROL);
    if owner == Owner::Completion {
        return match (key.code, control) {
            (KeyCode::Escape, _) => Some(CancelOrContinue),
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => Some(Next),
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) => Some(Previous),
            (KeyCode::Tab, _) => Some(Accept),
            (KeyCode::Enter, _) => Some(DismissAndContinue),
            (KeyCode::Char('c'), true) => Some(Cancel),
            _ => None,
        };
    }
    if owner.is_list_menu() && key.code == KeyCode::Tab {
        return Some(Back);
    }
    if owner == Owner::ProgramActions || owner.is_list_menu() {
        return match (key.code, control) {
            (KeyCode::Escape | KeyCode::Tab, _) | (KeyCode::Char('c'), true) => Some(Cancel),
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => Some(Next),
            (KeyCode::Up | KeyCode::BackTab, _) | (KeyCode::Char('p'), true) => Some(Previous),
            (KeyCode::Enter, _) => Some(Accept),
            _ => None,
        };
    }
    if owner == Owner::PluginManagerActions {
        return match (key.code, control) {
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) => Some(Cancel),
            _ => None,
        };
    }
    if owner == Owner::Path {
        return match (key.code, control) {
            (KeyCode::Tab, _) => Some(OpenActions),
            (KeyCode::Escape, _) | (KeyCode::Char('c'), true) => Some(Cancel),
            _ => None,
        };
    }
    match (key.code, control) {
        (KeyCode::Escape | KeyCode::Tab, _) | (KeyCode::Char('c'), true) => Some(Cancel),
        (KeyCode::Down | KeyCode::Char('j'), false) => Some(Next),
        (KeyCode::Up | KeyCode::Char('k'), false) | (KeyCode::BackTab, _) => Some(Previous),
        (KeyCode::Enter, false) => Some(Accept),
        _ => None,
    }
}

fn legacy_labels(owner: Owner, action: Action) -> &'static [&'static str] {
    if owner.is_prompt() {
        return prompt::labels(owner, action);
    }
    if owner.is_picker() {
        return picker::labels(owner, action);
    }
    use Action::*;
    if owner.is_list_menu() {
        return match action {
            Next => &["Down", "Ctrl-n"],
            Previous => &["Up", "Ctrl-p", "BackTab"],
            Accept => &["Enter"],
            Back => &["Tab"],
            Cancel => &["Esc", "Ctrl-c"],
            _ => &[],
        };
    }
    match (owner, action) {
        (Owner::Completion, Next) => &["Down", "Ctrl-n"],
        (Owner::Completion, Previous) => &["Up", "Ctrl-p"],
        (Owner::Completion, Accept) => &["Tab"],
        (Owner::Completion, Cancel) => &["Ctrl-c"],
        (Owner::Completion, CancelOrContinue) => &["Esc"],
        (Owner::Completion, DismissAndContinue) => &["Enter"],
        (Owner::ProgramActions, Next) => &["Down", "Ctrl-n"],
        (Owner::ProgramActions, Previous) => &["Up", "Ctrl-p", "BackTab"],
        (_, Next) => &["Down", "j"],
        (_, Previous) => &["Up", "k", "BackTab"],
        (_, Accept) => &["Enter"],
        (Owner::Path | Owner::PluginManagerActions, Cancel) => &["Esc", "Ctrl-c"],
        (_, Cancel) => &["Tab", "Esc", "Ctrl-c"],
        (_, OpenActions) => &["Tab"],
        _ => &[],
    }
}

pub(super) fn compile(
    section: Option<&Value>,
    leader: KeyStroke,
    errors: &mut Vec<String>,
) -> Bindings {
    let mut result = if leader == KeyStroke::char(' ') {
        Bindings::default()
    } else {
        Bindings::with_leader(leader)
    };
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
    for (name, mapping) in owners {
        let Some(owner) = Owner::ALL
            .into_iter()
            .find(|owner| Some(owner.name()) == name.as_str())
        else {
            errors.push("ui rejected: unknown owner".into());
            continue;
        };
        let parsed = (|| -> Result<OwnerBindings, String> {
            let mapping = mapping.as_mapping().ok_or("expected action mappings")?;
            if mapping.len() > owner.actions().len() {
                return Err("too many actions".into());
            }
            let mut assignments = HashMap::new();
            for (name, value) in mapping {
                let action = owner
                    .actions()
                    .iter()
                    .copied()
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
                    if owner.structural(key) {
                        return Err("key belongs to structural text or a menu choice".into());
                    }
                    if parsed.contains(&key) {
                        return Err("duplicate normalized key".into());
                    }
                    parsed.push(key);
                }
                assignments.insert(action, parsed);
            }
            // Start without the leader overlay: removing/reassigning cancel
            // restores any legacy movement/choice that leader used to cover.
            let mut candidate = Bindings::legacy().owners[&owner].clone();
            Arc::make_mut(&mut candidate.keys)
                .retain(|_, action| !assignments.contains_key(action));
            let inherited_leader =
                owner.inherits_leader() && !assignments.contains_key(&Action::Cancel);
            if inherited_leader
                && assignments
                    .values()
                    .flatten()
                    .any(|key| *key == leader.canonical_for_binding())
            {
                return Err("overlapping action bindings".into());
            }
            candidate.configured = assignments.keys().copied().collect();
            for (action, keys) in assignments {
                Arc::make_mut(&mut candidate.labels)
                    .insert(action, keys.iter().map(|key| key.label()).collect());
                for key in keys {
                    if Arc::make_mut(&mut candidate.keys)
                        .insert(key, action)
                        .is_some()
                    {
                        return Err("overlapping action bindings".into());
                    }
                }
            }
            if inherited_leader {
                candidate.add_leader(leader);
            }
            Ok(candidate)
        })();
        match parsed {
            Ok(candidate) => {
                result.owners.insert(owner, candidate);
            }
            Err(error) => errors.push(format!(
                "ui.{} rejected: {error}; defaults retained",
                owner.name()
            )),
        }
    }
    result
}
