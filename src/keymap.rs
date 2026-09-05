// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, LazyLock},
};

use crate::{
    command::{
        COMMANDS, ColonCommand, CommandCapability, CommandId, CommandInvocation,
        CommandInvocationError, EditorCommand, Mode,
    },
    input::{KeyCode, KeyStroke, Modifiers},
};

pub mod configured;
pub mod validate;

/// Compatibility name for a keymap key while callers migrate to the owned
/// input vocabulary.
pub type Key = KeyStroke;

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeySequence(Vec<Key>);

impl KeySequence {
    pub fn new(keys: impl IntoIterator<Item = Key>) -> Self {
        Self(
            keys.into_iter()
                .map(KeyStroke::canonical_for_binding)
                .collect(),
        )
    }

    pub fn as_slice(&self) -> &[Key] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, key: Key) {
        self.0.push(key.canonical_for_binding());
    }

    pub fn pop(&mut self) -> Option<Key> {
        self.0.pop()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn starts_with(&self, prefix: &Self) -> bool {
        self.0.starts_with(&prefix.0)
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let keys = value
            .split_whitespace()
            .map(KeyStroke::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if keys.is_empty() {
            return Err("a key sequence cannot be empty".to_owned());
        }
        Ok(Self::new(keys))
    }
}

impl<const N: usize> From<[Key; N]> for KeySequence {
    fn from(value: [Key; N]) -> Self {
        Self::new(value)
    }
}

impl From<Key> for KeySequence {
    fn from(value: Key) -> Self {
        Self::new([value])
    }
}

impl fmt::Display for KeySequence {
    /// Renders through `Formatter::pad` so callers can align key columns with
    /// width specifiers such as `{:<12}`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut label = String::new();
        for (index, key) in self.0.iter().enumerate() {
            if index > 0 {
                label.push(' ');
            }
            label.push_str(&key.label());
        }
        formatter.pad(&label)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingAvailability {
    Implemented,
    Planned(&'static str),
    Unsupported(&'static str),
}

/// How a binding participates in the default grammar's migration surface.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BindingRole {
    /// Canonical, discoverable binding in the nested Runyte namespace.
    #[default]
    Primary,
    /// Deliberately short binding retained for frequent use.
    Fast,
    /// Historical or platform alias retained for compatibility.
    Compatibility,
}

/// Semantic command identity reached by a key binding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BindingTarget {
    Editor(EditorCommand),
    Colon(ColonCommand),
}

impl BindingTarget {
    pub const fn id(self) -> CommandId {
        match self {
            Self::Editor(command) => CommandId::Editor(command),
            Self::Colon(command) => CommandId::Colon(command),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Editor(command) => command.metadata().description,
            Self::Colon(command) => {
                COMMANDS
                    .iter()
                    .find(|spec| spec.id == CommandId::Colon(command))
                    .expect("every bindable colon identity belongs to the command inventory")
                    .description
            }
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Editor(command) => command.metadata().name,
            Self::Colon(command) => {
                COMMANDS
                    .iter()
                    .find(|spec| spec.id == CommandId::Colon(command))
                    .expect("every bindable colon identity belongs to the command inventory")
                    .name
            }
        }
    }

    pub fn invocation(self) -> Result<CommandInvocation, CommandInvocationError> {
        match self {
            Self::Editor(command) => CommandInvocation::editor(command, Default::default()),
            Self::Colon(ColonCommand::Format) => CommandInvocation::from_parts(
                CommandId::Colon(ColonCommand::Format),
                crate::command::InvocationParameters::None,
                Default::default(),
            ),
            Self::Colon(ColonCommand::LspRestart) => CommandInvocation::lsp_restart(None),
            Self::Colon(ColonCommand::LspStatus) => Ok(CommandInvocation::lsp_status()),
            Self::Colon(ColonCommand::Reload) => CommandInvocation::from_parts(
                CommandId::Colon(ColonCommand::Reload),
                crate::command::InvocationParameters::None,
                Default::default(),
            ),
            Self::Colon(command) => CommandInvocation::from_parts(
                CommandId::Colon(command),
                crate::command::InvocationParameters::None,
                Default::default(),
            ),
        }
    }
}

impl From<EditorCommand> for BindingTarget {
    fn from(value: EditorCommand) -> Self {
        Self::Editor(value)
    }
}

impl From<ColonCommand> for BindingTarget {
    fn from(value: ColonCommand) -> Self {
        Self::Colon(value)
    }
}

impl BindingAvailability {
    pub const fn is_implemented(self) -> bool {
        matches!(self, Self::Implemented)
    }

    pub const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Implemented => None,
            Self::Planned(reason) | Self::Unsupported(reason) => Some(reason),
        }
    }
}

/// The buffer context in which a binding is active.
///
/// Global bindings remain available everywhere. The registry can detect a
/// scoped binding with the same sequence, but the built-in map forbids those
/// collisions and uses contextual actions instead. Every non-global variant
/// names one special-buffer role. Notifications and the about page are also
/// special buffers, but remain in the global scope until they gain actions of
/// their own.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum BindingScope {
    #[default]
    Global,
    Directory,
    Settings,
    GitStatus,
    GitBranches,
    GitWorktrees,
    GitLog,
    GitBlame,
    GitStash,
    WorkspaceSearch,
    Help,
    /// A commit message. Editable, but writing it commits rather than saving
    /// a file, which is the whole reason it needs its own help.
    CommitMessage,
    /// A read-only unified diff, including the staged-review buffer.
    Diff,
    /// A Markdown document and the page it renders to.
    ///
    /// Unlike every other scope here, this one is not a special buffer: an
    /// ordinary `.md` file keeps all of its editing keys and gains one more.
    /// The document and its rendered page share the scope because the key that
    /// opens the page is the key that leaves it.
    Markdown,
    /// A terminal's live/review surface. The buffer behind the pane must not
    /// lend it scoped bindings, while terminal-only escapes stay out of
    /// ordinary editor hints.
    Terminal,
}

impl BindingScope {
    /// Exhaustive scope inventory used by registry invariants.
    pub const ALL: &'static [Self] = &[
        Self::Global,
        Self::Directory,
        Self::Settings,
        Self::GitStatus,
        Self::GitBranches,
        Self::Terminal,
        Self::GitWorktrees,
        Self::GitLog,
        Self::GitBlame,
        Self::GitStash,
        Self::WorkspaceSearch,
        Self::Help,
        Self::CommitMessage,
        Self::Diff,
        Self::Markdown,
    ];

    /// Whether the scope belongs to a generated view rather than to a document
    /// someone opened.
    ///
    /// `Markdown` is not one: it covers every Markdown file as well as the page
    /// rendered from it, so counting it would claim an ordinary document as a
    /// view of the editor's own.
    pub const fn is_special_buffer_scope(self) -> bool {
        !matches!(self, Self::Global | Self::Terminal | Self::Markdown)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub modes: &'static [Mode],
    pub scope: BindingScope,
    pub sequence: KeySequence,
    pub target: BindingTarget,
    pub description: &'static str,
    pub availability: BindingAvailability,
    pub role: BindingRole,
    /// Another sequence reaching the same command, named here so discovery can
    /// mention it. Two spellings of a command are a keymap fact, but which one
    /// is worth advertising is editorial: only the alias someone would not
    /// otherwise find from this namespace belongs here.
    pub alias: Option<KeySequence>,
    /// Modes in which `alias` reaches the command when they differ from this
    /// binding's modes. `None` means the alias is active in `modes` too.
    pub alias_modes: Option<&'static [Mode]>,
}

impl Binding {
    pub fn implemented(
        modes: &'static [Mode],
        sequence: impl Into<KeySequence>,
        target: impl Into<BindingTarget>,
    ) -> Self {
        let target = target.into();
        Self {
            modes,
            scope: BindingScope::Global,
            sequence: sequence.into(),
            target,
            description: target.description(),
            availability: BindingAvailability::Implemented,
            role: BindingRole::Primary,
            alias: None,
            alias_modes: None,
        }
    }

    pub fn implemented_in(
        modes: &'static [Mode],
        scope: BindingScope,
        sequence: impl Into<KeySequence>,
        target: impl Into<BindingTarget>,
    ) -> Self {
        let target = target.into();
        Self {
            modes,
            scope,
            sequence: sequence.into(),
            target,
            description: target.description(),
            availability: BindingAvailability::Implemented,
            role: BindingRole::Primary,
            alias: None,
            alias_modes: None,
        }
    }

    pub const fn with_role(mut self, role: BindingRole) -> Self {
        self.role = role;
        self
    }

    /// Name a second sequence for the same command so key discovery can show
    /// it alongside this one. The keymap remains the authority on what the
    /// alias does: `aliases_reach_the_command_they_are_advertised_on` fails if
    /// the two ever drift apart.
    pub fn with_alias(mut self, alias: impl Into<KeySequence>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    /// Name an alias that reaches the same command from a different mode.
    pub fn with_alias_in(mut self, modes: &'static [Mode], alias: impl Into<KeySequence>) -> Self {
        self.alias = Some(alias.into());
        self.alias_modes = Some(modes);
        self
    }

    pub fn is_active_in(&self, mode: Mode) -> bool {
        self.modes.contains(&mode)
    }

    pub fn planned(
        modes: &'static [Mode],
        sequence: impl Into<KeySequence>,
        command: EditorCommand,
        reason: &'static str,
    ) -> Self {
        let target = BindingTarget::Editor(command);
        Self {
            modes,
            scope: BindingScope::Global,
            sequence: sequence.into(),
            target,
            description: target.description(),
            availability: BindingAvailability::Planned(reason),
            role: BindingRole::Primary,
            alias: None,
            alias_modes: None,
        }
    }

    pub fn unsupported(
        modes: &'static [Mode],
        sequence: impl Into<KeySequence>,
        command: EditorCommand,
        reason: &'static str,
    ) -> Self {
        let target = BindingTarget::Editor(command);
        Self {
            modes,
            scope: BindingScope::Global,
            sequence: sequence.into(),
            target,
            description: target.description(),
            availability: BindingAvailability::Unsupported(reason),
            role: BindingRole::Primary,
            alias: None,
            alias_modes: None,
        }
    }
}

/// A labelled, non-executable prefix shown as one nested hint row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingNamespace {
    pub modes: &'static [Mode],
    pub scope: BindingScope,
    pub sequence: KeySequence,
    pub description: &'static str,
    /// Live active-buffer capability required by this subtree's ordinary
    /// commands. The prefix remains navigable while unavailable so discovery
    /// can explain the state and expose recovery commands.
    pub capability: Option<CommandCapability>,
}

/// One action offered by the contextual menu for a buffer scope.
///
/// The mnemonic is local to the open menu, so it never participates in normal
/// key dispatch and cannot shadow a global binding. The keymap still owns the
/// command identity and description consumed by execution, help, and the menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextAction {
    pub scope: BindingScope,
    pub mnemonic: Key,
    /// One lower-case word naming what the action does, distinct from the
    /// sentence that explains it. The menu reads down this column, so two
    /// actions in a scope only share a word when they really do the same
    /// thing to different amounts of the buffer.
    pub name: &'static str,
    pub target: BindingTarget,
    pub description: &'static str,
    pub context: ActionContext,
}

impl ContextAction {
    pub fn row(
        scope: BindingScope,
        mnemonic: Key,
        name: &'static str,
        target: impl Into<BindingTarget>,
    ) -> Self {
        Self::new(scope, mnemonic, name, target, ActionContext::Row)
    }

    pub fn buffer(
        scope: BindingScope,
        mnemonic: Key,
        name: &'static str,
        target: impl Into<BindingTarget>,
    ) -> Self {
        Self::new(scope, mnemonic, name, target, ActionContext::Buffer)
    }

    fn new(
        scope: BindingScope,
        mnemonic: Key,
        name: &'static str,
        target: impl Into<BindingTarget>,
        context: ActionContext,
    ) -> Self {
        let target = target.into();
        Self {
            scope,
            mnemonic,
            name,
            target,
            description: target.description(),
            context,
        }
    }

    pub const fn with_description(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionContext {
    Row,
    Buffer,
}

impl BindingNamespace {
    pub fn global(
        modes: &'static [Mode],
        sequence: impl Into<KeySequence>,
        description: &'static str,
    ) -> Self {
        Self {
            modes,
            scope: BindingScope::Global,
            sequence: sequence.into(),
            description,
            capability: None,
        }
    }

    pub const fn with_capability(mut self, capability: CommandCapability) -> Self {
        self.capability = Some(capability);
        self
    }

    pub fn is_active_in(&self, mode: Mode) -> bool {
        self.modes.contains(&mode)
    }
}

/// One first key of the sequences active in a context.
///
/// Discovery through the hint popup only ever starts from a key someone
/// already knows to press. This is the set they could have pressed, so a view
/// can name its own starting points instead of assuming the reader has the
/// keymap memorised.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntryPoint {
    pub key: Key,
    /// What the key does, or what its namespace holds.
    pub description: &'static str,
    /// Whether more keys are expected. A prefix opens the hint popup; a leaf
    /// runs on the first press and never shows a hint at all.
    pub prefix: bool,
    /// Whether every binding under this key belongs to the active scope. A
    /// global prefix that merely has a scoped leaf hanging off it is not a
    /// buffer-specific key, so it does not count.
    pub scoped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Lookup<'a> {
    NoMatch,
    Exact(&'a Binding),
    Prefix(Vec<&'a Binding>),
    ExactAndPrefix {
        exact: &'a Binding,
        continuations: Vec<&'a Binding>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicateBinding {
    pub mode: Mode,
    pub scope: BindingScope,
    pub sequence: KeySequence,
}

impl fmt::Display for DuplicateBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "duplicate {} {:?} binding for {}",
            self.mode.label(),
            self.scope,
            self.sequence
        )
    }
}

impl std::error::Error for DuplicateBinding {}

#[derive(Clone, Debug)]
pub struct Keymap {
    bindings: Vec<Binding>,
    namespaces: Vec<BindingNamespace>,
    context_actions: Vec<ContextAction>,
    leader: KeyStroke,
    window: KeyStroke,
    default_spellings: HashMap<KeySequence, KeySequence>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(Vec::new()).expect("an empty keymap is valid")
    }
}

impl Keymap {
    pub fn new(bindings: Vec<Binding>) -> Result<Self, DuplicateBinding> {
        for (index, binding) in bindings.iter().enumerate() {
            for mode in binding.modes {
                if bindings[..index].iter().any(|other| {
                    other.is_active_in(*mode)
                        && other.scope == binding.scope
                        && other.sequence == binding.sequence
                }) {
                    return Err(DuplicateBinding {
                        mode: *mode,
                        scope: binding.scope,
                        sequence: binding.sequence.clone(),
                    });
                }
            }
        }
        let mut keymap = Self {
            bindings,
            namespaces: Vec::new(),
            context_actions: Vec::new(),
            leader: Key::char(' '),
            window: Key::ctrl('w'),
            default_spellings: HashMap::new(),
        };
        keymap.record_identity_spellings();
        Ok(keymap)
    }

    pub fn with_namespaces(
        bindings: Vec<Binding>,
        namespaces: Vec<BindingNamespace>,
    ) -> Result<Self, DuplicateBinding> {
        let mut keymap = Self::new(bindings)?;
        keymap.namespaces = namespaces;
        keymap.record_identity_spellings();
        Ok(keymap)
    }

    pub fn with_context_actions(mut self, actions: Vec<ContextAction>) -> Self {
        for (index, action) in actions.iter().enumerate() {
            assert!(
                matches!(
                    action.mnemonic,
                    KeyStroke {
                        code: KeyCode::Char(character),
                        modifiers: Modifiers::NONE,
                    } if character.is_ascii_alphabetic() && !matches!(character, 'j' | 'k')
                ),
                "contextual action mnemonics must be plain letters other than reserved j and k"
            );
            assert!(
                !actions[..index].iter().any(|other| {
                    other.scope == action.scope && other.mnemonic == action.mnemonic
                }),
                "duplicate {:?} action mnemonic {}",
                action.scope,
                action.mnemonic.label()
            );
        }
        self.context_actions = actions;
        self
    }

    pub(crate) fn with_spelling_metadata(
        mut self,
        leader: KeyStroke,
        window: KeyStroke,
        default_spellings: HashMap<KeySequence, KeySequence>,
    ) -> Self {
        self.leader = leader;
        self.window = window;
        self.default_spellings = default_spellings;
        self
    }

    fn record_identity_spellings(&mut self) {
        for sequence in self
            .bindings
            .iter()
            .flat_map(|binding| {
                [
                    &binding.sequence,
                    binding.alias.as_ref().unwrap_or(&binding.sequence),
                ]
            })
            .chain(self.namespaces.iter().map(|namespace| &namespace.sequence))
        {
            self.default_spellings
                .entry(sequence.clone())
                .or_insert_with(|| sequence.clone());
        }
    }

    pub const fn leader(&self) -> KeyStroke {
        self.leader
    }

    pub const fn window_prefix(&self) -> KeyStroke {
        self.window
    }

    pub fn spelling_for_default(&self, sequence: &KeySequence) -> Option<&KeySequence> {
        self.default_spellings.get(sequence)
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub fn namespaces(&self) -> &[BindingNamespace] {
        &self.namespaces
    }

    pub fn context_actions(&self, scope: BindingScope) -> impl Iterator<Item = &ContextAction> {
        self.context_actions
            .iter()
            .filter(move |action| action.scope == scope)
    }

    pub fn all_context_actions(&self) -> &[ContextAction] {
        &self.context_actions
    }

    pub fn bindings_for_mode(&self, mode: Mode) -> impl Iterator<Item = &Binding> {
        let mut bindings = self
            .bindings
            .iter()
            .filter(move |binding| {
                binding.is_active_in(mode) && binding.scope == BindingScope::Global
            })
            .collect::<Vec<_>>();
        bindings.sort_by_key(|binding| binding.role);
        bindings.into_iter()
    }

    /// The everywhere-available sequence reaching `target`, for messages that
    /// have to tell someone which keys open a view.
    ///
    /// Deliberately global-only. Such a message is shown outside the scope it
    /// talks about — a refusal in the wrong buffer is the whole reason to name
    /// the keys — so a scoped binding would advertise a key that does nothing
    /// where the reader is standing.
    pub fn global_sequence_for(&self, mode: Mode, target: BindingTarget) -> Option<&KeySequence> {
        self.bindings_for_mode(mode)
            .find(|binding| {
                binding.target == target
                    && matches!(binding.availability, BindingAvailability::Implemented)
            })
            .map(|binding| &binding.sequence)
    }

    pub fn bindings_for_scope(
        &self,
        mode: Mode,
        scope: BindingScope,
    ) -> impl Iterator<Item = &Binding> {
        let mut bindings = Vec::new();
        if scope != BindingScope::Global {
            let mut scoped = self
                .bindings
                .iter()
                .filter(|binding| {
                    binding.is_active_in(mode) && scope_includes(scope, binding.scope)
                })
                .collect::<Vec<_>>();
            scoped.sort_by_key(|binding| binding.role);
            bindings.extend(scoped);
        }
        let mut global = self
            .bindings
            .iter()
            .filter(|binding| {
                binding.is_active_in(mode)
                    && binding.scope == BindingScope::Global
                    && (scope == BindingScope::Global
                        || !self.bindings.iter().any(|scoped| {
                            scoped.is_active_in(mode)
                                && scope_includes(scope, scoped.scope)
                                && scoped.sequence == binding.sequence
                        }))
            })
            .collect::<Vec<_>>();
        global.sort_by_key(|binding| binding.role);
        bindings.extend(global);
        bindings.into_iter()
    }

    /// The bindings a scope contributes on top of the global keymap.
    ///
    /// These are what a view has to explain about itself: everything else it
    /// answers to is already true of every other buffer.
    pub fn scoped_bindings(
        &self,
        mode: Mode,
        scope: BindingScope,
    ) -> impl Iterator<Item = &Binding> {
        self.bindings_for_scope(mode, scope)
            .filter(|binding| binding.scope != BindingScope::Global)
    }

    /// Scoped bindings paired with the global binding each one hides.
    ///
    /// A key that means something else here is worse than a key that means
    /// nothing here: the reader gets a confident wrong answer. `bindings_for_scope`
    /// already drops the hidden binding from dispatch, so this is the only
    /// place the collision is still visible.
    pub fn shadowed_bindings(&self, mode: Mode, scope: BindingScope) -> Vec<(&Binding, &Binding)> {
        self.scoped_bindings(mode, scope)
            .filter_map(|scoped| {
                let global = self.bindings.iter().find(|binding| {
                    binding.is_active_in(mode)
                        && binding.scope == BindingScope::Global
                        && binding.sequence == scoped.sequence
                })?;
                Some((scoped, global))
            })
            .collect()
    }

    /// Every first key that starts a binding in this context.
    ///
    /// Scope-specific keys come first, then the global ones, each group in
    /// key order.
    pub fn entry_points(&self, mode: Mode, scope: BindingScope) -> Vec<EntryPoint> {
        let mut entries: Vec<EntryPoint> = Vec::new();
        for binding in self.bindings_for_scope(mode, scope) {
            let Some(key) = binding.sequence.as_slice().first().copied() else {
                continue;
            };
            let leaf = binding.sequence.len() == 1;
            let scoped = binding.scope != BindingScope::Global;
            if let Some(entry) = entries.iter_mut().find(|entry| entry.key == key) {
                // An exact single-key binding names the key; longer sequences
                // only establish that more keys are expected.
                if leaf {
                    entry.description = binding.description;
                } else {
                    entry.prefix = true;
                }
                entry.scoped &= scoped;
            } else {
                entries.push(EntryPoint {
                    key,
                    description: if leaf { binding.description } else { "" },
                    prefix: !leaf,
                    scoped,
                });
            }
        }

        // A prefix that is not also a command has no description of its own,
        // so it takes the name its namespace gives it.
        for entry in &mut entries {
            if entry.description.is_empty()
                && let Some(namespace) = self
                    .namespaces_for_scope(mode, scope)
                    .find(|namespace| namespace.sequence.as_slice() == [entry.key])
            {
                entry.description = namespace.description;
            }
        }

        entries.sort_by(|left, right| {
            right
                .scoped
                .cmp(&left.scoped)
                .then_with(|| left.key.label().cmp(&right.key.label()))
        });
        entries
    }

    pub fn namespaces_for_scope(
        &self,
        mode: Mode,
        scope: BindingScope,
    ) -> impl Iterator<Item = &BindingNamespace> {
        self.namespaces.iter().filter(move |namespace| {
            namespace.is_active_in(mode)
                && (namespace.scope == BindingScope::Global
                    || scope_includes(scope, namespace.scope))
        })
    }

    pub fn lookup(&self, mode: Mode, sequence: &KeySequence) -> Lookup<'_> {
        self.lookup_in(mode, BindingScope::Global, sequence)
    }

    pub fn lookup_in(&self, mode: Mode, scope: BindingScope, sequence: &KeySequence) -> Lookup<'_> {
        let mut exact = None;
        let mut continuations = Vec::new();
        for binding in self.bindings_for_scope(mode, scope) {
            if binding.sequence == *sequence {
                exact = Some(binding);
            } else if binding.sequence.starts_with(sequence) {
                continuations.push(binding);
            }
        }
        match (exact, continuations.is_empty()) {
            (None, true) => Lookup::NoMatch,
            (None, false) => Lookup::Prefix(continuations),
            (Some(exact), true) => Lookup::Exact(exact),
            (Some(exact), false) => Lookup::ExactAndPrefix {
                exact,
                continuations,
            },
        }
    }
}

const MODAL: &[Mode] = &[Mode::Normal, Mode::Select];
const INSERT: &[Mode] = &[Mode::Insert, Mode::Replace];

fn modal(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
    let sequence = sequence.into();
    let role = existing_binding_role(&sequence);
    Binding::implemented(MODAL, sequence, command).with_role(role)
}

fn primary_modal(sequence: impl Into<KeySequence>, target: impl Into<BindingTarget>) -> Binding {
    Binding::implemented(MODAL, sequence, target)
}

fn directory(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
    let sequence = sequence.into();
    let role = existing_binding_role(&sequence);
    Binding::implemented_in(MODAL, BindingScope::Directory, sequence, command).with_role(role)
}

fn markdown(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
    Binding::implemented_in(MODAL, BindingScope::Markdown, sequence, command)
}

fn git_status(sequence: impl Into<KeySequence>, target: impl Into<BindingTarget>) -> Binding {
    Binding::implemented_in(MODAL, BindingScope::GitStatus, sequence, target)
}

fn git_branches(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
    Binding::implemented_in(MODAL, BindingScope::GitBranches, sequence, command)
}

fn git_worktrees(sequence: impl Into<KeySequence>, target: impl Into<BindingTarget>) -> Binding {
    Binding::implemented_in(MODAL, BindingScope::GitWorktrees, sequence, target)
}

fn git_log(sequence: impl Into<KeySequence>, target: impl Into<BindingTarget>) -> Binding {
    Binding::implemented_in(MODAL, BindingScope::GitLog, sequence, target)
}

fn git_blame(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
    Binding::implemented_in(MODAL, BindingScope::GitBlame, sequence, command)
}

fn workspace_search(sequence: impl Into<KeySequence>, target: impl Into<BindingTarget>) -> Binding {
    Binding::implemented_in(MODAL, BindingScope::WorkspaceSearch, sequence, target)
}

/// `q` closes help, as it does in Vim and Helix.
///
/// It stays unbound everywhere else: `q` and `Q` say nothing about what they
/// do, which is why macros live under `Space m` instead. Help is read-only and
/// row-oriented, so the letter is free here and costs nothing.
fn help_scope(sequence: impl Into<KeySequence>, target: impl Into<BindingTarget>) -> Binding {
    Binding::implemented_in(MODAL, BindingScope::Help, sequence, target)
}

fn settings(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
    let sequence = sequence.into();
    let role = existing_binding_role(&sequence);
    Binding::implemented_in(MODAL, BindingScope::Settings, sequence, command).with_role(role)
}

fn insert(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
    let sequence = sequence.into();
    let role = existing_binding_role(&sequence);
    Binding::implemented(INSERT, sequence, command).with_role(role)
}

fn terminal_insert(sequence: impl Into<KeySequence>, command: EditorCommand) -> Binding {
    let sequence = sequence.into();
    let role = existing_binding_role(&sequence);
    Binding::implemented_in(INSERT, BindingScope::Terminal, sequence, command).with_role(role)
}

fn unsupported(
    sequence: impl Into<KeySequence>,
    command: EditorCommand,
    reason: &'static str,
) -> Binding {
    let sequence = sequence.into();
    let role = existing_binding_role(&sequence);
    Binding::unsupported(MODAL, sequence, command, reason).with_role(role)
}

fn existing_binding_role(sequence: &KeySequence) -> BindingRole {
    let keys = sequence.as_slice();
    if keys.len() > 1 && keys.first().is_some_and(|key| *key == Key::ctrl('w')) {
        return BindingRole::Compatibility;
    }
    if let [space, suffix] = keys
        && *space == Key::char(' ')
    {
        return match suffix.code {
            KeyCode::Char('e' | 'E' | 'f' | 'F' | 'b' | '/') if suffix.modifiers.is_empty() => {
                BindingRole::Fast
            }
            _ => BindingRole::Primary,
        };
    }
    BindingRole::Primary
}

fn built_in_bindings() -> Vec<Binding> {
    use EditorCommand as Command;

    vec![
        modal(Key::plain(KeyCode::Escape), Command::EnterNormalMode),
        modal(Key::ctrl('\\'), Command::EnterNormalMode),
        modal(Key::ctrl('4'), Command::EnterNormalMode).with_role(BindingRole::Compatibility),
        modal(Key::char(':'), Command::OpenCommandPalette),
        modal(Key::char('h'), Command::MoveLeft),
        modal(Key::plain(KeyCode::Left), Command::MoveLeft),
        modal(Key::char('j'), Command::MoveDown),
        modal(Key::plain(KeyCode::Down), Command::MoveDown),
        modal(Key::char('k'), Command::MoveUp),
        modal(Key::plain(KeyCode::Up), Command::MoveUp),
        modal(Key::char('l'), Command::MoveRight),
        modal(Key::plain(KeyCode::Right), Command::MoveRight),
        modal(Key::char('w'), Command::MoveWordForward),
        modal(Key::char('b'), Command::MoveWordBackward),
        modal(Key::char('e'), Command::MoveWordEnd),
        modal(Key::char('W'), Command::MoveLongWordForward),
        modal(Key::char('B'), Command::MoveLongWordBackward),
        modal(Key::char('E'), Command::MoveLongWordEnd),
        modal(Key::char('f'), Command::FindNextChar),
        modal(Key::char('F'), Command::FindPreviousChar),
        modal(Key::char('t'), Command::FindTillNextChar),
        modal(Key::char('T'), Command::FindTillPreviousChar),
        modal(Key::plain(KeyCode::Home), Command::MoveLineStart),
        modal(Key::plain(KeyCode::End), Command::MoveLineEnd),
        // Retained MVP aliases; Helix uses Home/End and goto mode.
        modal(Key::char('0'), Command::MoveLineStart),
        modal(Key::char('^'), Command::MoveFirstNonWhitespace),
        modal(Key::char('$'), Command::MoveLineEnd),
        modal(Key::ctrl('b'), Command::PageUp),
        modal(Key::plain(KeyCode::PageUp), Command::PageUp),
        modal(Key::ctrl('f'), Command::PageDown),
        modal(Key::plain(KeyCode::PageDown), Command::PageDown),
        modal(Key::ctrl('u'), Command::HalfPageUp),
        modal(Key::ctrl('d'), Command::HalfPageDown),
        modal(Key::char('i'), Command::EnterInsertMode),
        modal(Key::char('a'), Command::AppendAfter),
        modal(Key::char('I'), Command::InsertLineStart),
        modal(Key::char('A'), Command::InsertLineEnd),
        modal(Key::char('o'), Command::OpenLineBelow),
        modal(Key::char('O'), Command::OpenLineAbove),
        modal(Key::char('r'), Command::ReplaceChar),
        modal(Key::char('R'), Command::EnterReplaceMode),
        modal(Key::char('~'), Command::ToggleCase),
        modal(Key::char('u'), Command::Undo),
        modal(Key::char('U'), Command::Redo),
        modal(Key::char('y'), Command::Yank),
        modal(Key::char('Y'), Command::YankLine),
        modal(Key::char('p'), Command::PasteAfter),
        modal(Key::char('P'), Command::PasteBefore),
        modal(Key::char('>'), Command::Indent),
        modal(Key::char('<'), Command::Unindent),
        modal(Key::ctrl('c'), Command::ToggleComments),
        modal(Key::char('d'), Command::DeleteSelection),
        modal(Key::char('c'), Command::ChangeSelection),
        modal(Key::char('v'), Command::EnterSelectMode),
        modal(Key::char('x'), Command::SelectLine),
        modal(Key::char('X'), Command::SelectLineUp),
        modal(Key::char('%'), Command::SelectAll),
        modal(Key::char(';'), Command::CollapseSelection),
        modal(Key::alt(';'), Command::FlipSelection),
        modal(Key::char(','), Command::KeepPrimarySelection),
        modal(Key::alt(','), Command::RemovePrimarySelection),
        modal(Key::char('C'), Command::CopySelectionDown),
        modal(Key::alt('C'), Command::CopySelectionUp),
        modal(Key::char('V'), Command::CopySelectionDownPadded),
        modal(Key::alt('V'), Command::CopySelectionUpPadded),
        modal(Key::char(')'), Command::RotateSelectionForward),
        modal(Key::char('('), Command::RotateSelectionBackward),
        modal(Key::alt(')'), Command::RotateSelectionContentsForward),
        modal(Key::alt('('), Command::RotateSelectionContentsBackward),
        modal(Key::char('&'), Command::AlignSelections),
        modal(Key::char('_'), Command::TrimTrailingWhitespace),
        modal(Key::alt('_'), Command::TrimSelections),
        primary_modal(
            [Key::char(' '), Key::char('p'), Key::char('w')],
            Command::HardWrap,
        ),
        primary_modal(
            [Key::char(' '), Key::char('p'), Key::char('r')],
            Command::Reflow,
        ),
        primary_modal(
            [Key::char(' '), Key::char('p'), Key::char('s')],
            Command::ToggleSoftWrap,
        ),
        primary_modal(
            [Key::char(' '), Key::char('p'), Key::char('.')],
            Command::ToggleWhitespace,
        ),
        // Joining is the inverse of `Space p w`, so it lives beside it rather
        // than on Helix's `J`: the delimiter prompt is part of the command, and
        // a bare letter cannot say that it is coming.
        primary_modal(
            [Key::char(' '), Key::char('p'), Key::char('j')],
            Command::JoinSelections,
        ),
        // Table formatting is the third way of laying selected lines out, so it
        // joins wrapping and joining under `Space p` rather than opening a
        // namespace of its own for one command.
        primary_modal(
            [Key::char(' '), Key::char('p'), Key::char('t')],
            Command::FormatTable,
        ),
        // Searching this buffer is one key and no namespace: `s` and `/` are
        // the spellings someone who searches all day wants, and `Space /`
        // widens each of them to the whole project without respelling it.
        modal(Key::char('s'), Command::Search),
        modal(Key::char('/'), Command::SearchRegex),
        modal(Key::char('n'), Command::SearchNext),
        modal(Key::char('N'), Command::SearchPrevious),
        modal(Key::char('*'), Command::SearchSelection),
        // Ctrl-s was Runyte's original save shortcut. Helix assigns it to
        // save_selection; Runyte retains the global save compatibility key.
        modal(Key::ctrl('s'), Command::Save),
        // The key every other program pastes with, and the only way an image
        // reaches a document. It is bound in Insert as well as in the modal
        // modes because a picture is pasted while writing the paragraph that
        // introduces it, not after leaving it. Terminal Insert is deliberately
        // untouched, so Ctrl-v still reaches the child program running there.
        modal(Key::ctrl('v'), Command::ClipboardPaste),
        insert(Key::ctrl('v'), Command::ClipboardPaste),
        modal(Key::ctrl('o'), Command::JumpBackward),
        // Alt rather than Ctrl-Shift: without a disambiguating keyboard
        // protocol, Ctrl-O and Ctrl-o arrive as the same control byte.
        modal(Key::alt('o'), Command::JumpBackwardBuffer),
        modal(Key::alt('i'), Command::JumpForwardBuffer),
        // Unix terminals other than macOS can distinguish this from Tab while
        // REPORT_ALL_KEYS_AS_ESCAPE_CODES is active. On macOS and Windows the
        // two still arrive alike, so Tab owns contextual actions and the
        // forward jump has no reachable key there.
        modal(Key::ctrl('i'), Command::JumpForward),
        modal(Key::plain(KeyCode::Tab), Command::CodeAction),
        modal([Key::char('g'), Key::char('g')], Command::MoveFileStart),
        modal(Key::char('G'), Command::MoveFileEnd),
        modal([Key::char('g'), Key::char('e')], Command::MoveFileEnd),
        modal([Key::char('g'), Key::char('h')], Command::MoveLineStart),
        modal([Key::char('g'), Key::char('l')], Command::MoveLineEnd),
        modal(
            [Key::char('g'), Key::char('s')],
            Command::MoveFirstNonWhitespace,
        ),
        modal([Key::char('g'), Key::char('t')], Command::GotoWindowTop),
        modal([Key::char('g'), Key::char('c')], Command::GotoWindowCenter),
        modal([Key::char('g'), Key::char('b')], Command::GotoWindowBottom),
        // Vim's names for the same three positions. They cost nothing here
        // because Helix leaves H, M, and L unbound, and they read as motions
        // rather than as the z-family's view scrolling.
        modal(Key::char('H'), Command::GotoWindowTop),
        modal(Key::char('M'), Command::GotoWindowCenter),
        modal(Key::char('L'), Command::GotoWindowBottom),
        modal([Key::char('g'), Key::char('p')], Command::GotoNextParagraph),
        modal(
            [Key::char('g'), Key::char('P')],
            Command::GotoPreviousParagraph,
        ),
        modal([Key::char('g'), Key::char('f')], Command::GotoFile),
        modal([Key::char('g'), Key::char('w')], Command::GotoWord),
        modal([Key::char('g'), Key::char('d')], Command::GotoDefinition),
        modal([Key::char('g'), Key::char('D')], Command::GotoDeclaration),
        modal(
            [Key::char('g'), Key::char('y')],
            Command::GotoTypeDefinition,
        ),
        modal([Key::char('g'), Key::char('r')], Command::GotoReferences),
        modal(
            [Key::char('g'), Key::char('i')],
            Command::GotoImplementation,
        ),
        modal([Key::char('z'), Key::char('z')], Command::AlignViewCenter),
        modal([Key::char('z'), Key::char('c')], Command::AlignViewCenter),
        modal([Key::char('z'), Key::char('t')], Command::AlignViewTop),
        modal([Key::char('z'), Key::char('b')], Command::AlignViewBottom),
        modal([Key::char('z'), Key::char('m')], Command::AlignViewMiddle),
        modal([Key::char('z'), Key::char('j')], Command::ScrollViewDown),
        modal(
            [Key::char('z'), Key::plain(KeyCode::Down)],
            Command::ScrollViewDown,
        ),
        modal([Key::char('z'), Key::char('k')], Command::ScrollViewUp),
        modal(
            [Key::char('z'), Key::plain(KeyCode::Up)],
            Command::ScrollViewUp,
        ),
        modal([Key::char('z'), Key::ctrl('f')], Command::PageDown),
        modal(
            [Key::char('z'), Key::plain(KeyCode::PageDown)],
            Command::PageDown,
        ),
        modal([Key::char('z'), Key::ctrl('b')], Command::PageUp),
        modal(
            [Key::char('z'), Key::plain(KeyCode::PageUp)],
            Command::PageUp,
        ),
        modal([Key::char('z'), Key::ctrl('u')], Command::HalfPageUp),
        modal([Key::char('z'), Key::ctrl('d')], Command::HalfPageDown),
        modal([Key::char('Z'), Key::char('z')], Command::AlignViewCenter),
        modal([Key::char('Z'), Key::char('c')], Command::AlignViewCenter),
        modal([Key::char('Z'), Key::char('t')], Command::AlignViewTop),
        modal([Key::char('Z'), Key::char('b')], Command::AlignViewBottom),
        modal([Key::char('Z'), Key::char('m')], Command::AlignViewMiddle),
        modal([Key::char('Z'), Key::char('j')], Command::ScrollViewDown),
        modal(
            [Key::char('Z'), Key::plain(KeyCode::Down)],
            Command::ScrollViewDown,
        ),
        modal([Key::char('Z'), Key::char('k')], Command::ScrollViewUp),
        modal(
            [Key::char('Z'), Key::plain(KeyCode::Up)],
            Command::ScrollViewUp,
        ),
        modal([Key::char('Z'), Key::ctrl('f')], Command::PageDown),
        modal(
            [Key::char('Z'), Key::plain(KeyCode::PageDown)],
            Command::PageDown,
        ),
        modal([Key::char('Z'), Key::ctrl('b')], Command::PageUp),
        modal(
            [Key::char('Z'), Key::plain(KeyCode::PageUp)],
            Command::PageUp,
        ),
        modal([Key::char('Z'), Key::ctrl('u')], Command::HalfPageUp),
        modal([Key::char('Z'), Key::ctrl('d')], Command::HalfPageDown),
        modal([Key::char(' '), Key::char('e')], Command::OpenExplorer),
        modal(
            [Key::char(' '), Key::char('E')],
            Command::OpenWorkingDirectoryExplorer,
        ),
        primary_modal([Key::char(' '), Key::char(' ')], ColonCommand::SessionList),
        modal([Key::char(' '), Key::char('?')], Command::ShowHelp),
        // Buffers. `Space b b` repeats the namespace letter the way `Space m m`
        // does: the most-reached-for thing in a group is spelled with the group
        // itself rather than with a second letter to remember.
        primary_modal(
            [Key::char(' '), Key::char('b'), Key::char('b')],
            Command::OpenBufferPicker,
        ),
        primary_modal(
            [Key::char(' '), Key::char('b'), Key::char('c')],
            ColonCommand::CloseBuffer,
        ),
        primary_modal(
            [Key::char(' '), Key::char('b'), Key::char('d')],
            ColonCommand::DiffDisk,
        ),
        primary_modal(
            [Key::char(' '), Key::char('b'), Key::char('n')],
            Command::NewBuffer,
        ),
        // Terminals. `Space t t` repeats the namespace letter for the list,
        // the way `Space b b` does, because reaching a terminal already
        // running is the thing done most once more than one exists.
        primary_modal(
            [Key::char(' '), Key::char('t'), Key::char('t')],
            Command::OpenTerminalList,
        ),
        primary_modal(
            [Key::char(' '), Key::char('t'), Key::char('n')],
            Command::OpenTerminal,
        ),
        primary_modal(
            [Key::char(' '), Key::char('t'), Key::char('q')],
            Command::LeaveTerminal,
        ),
        primary_modal(
            [Key::char(' '), Key::char('t'), Key::char('r')],
            Command::RenameTerminal,
        ),
        primary_modal(
            [Key::char(' '), Key::char('t'), Key::char('y')],
            Command::CopyTerminalOutput,
        ),
        // The one command in this namespace meant for a pane that is *not* a
        // terminal: it is how text composed with the whole editor reaches a
        // program that owns its own input area.
        primary_modal(
            [Key::char(' '), Key::char('t'), Key::char('s')],
            Command::SendToTerminal,
        ),
        // Looking past this buffer lives under `Space /`. The sigil says search,
        // the prefix says the whole project rather than the file in front of
        // you, and the key after it is the one the bare key already spells:
        // `Space / s` mirrors `s` and `Space / /` mirrors `/`, so a flavour is
        // spelled once and widened by the prefix rather than respelled.
        primary_modal(
            [Key::char(' '), Key::char('/'), Key::char('s')],
            Command::GlobalSearch,
        ),
        primary_modal(
            [Key::char(' '), Key::char('/'), Key::char('/')],
            Command::GlobalSearchRegex,
        ),
        // The finder is `f` in every namespace it appears in, so it does not
        // compete with the sigil for a spelling. `Space f` is its short one.
        primary_modal(
            [Key::char(' '), Key::char('/'), Key::char('f')],
            Command::OpenFilePicker,
        )
        .with_alias([Key::char(' '), Key::char('f')]),
        modal([Key::char(' '), Key::char('f')], Command::OpenFilePicker),
        // The same finder over a wider scope: `a` drops the ignore files the
        // project states, `p` drops the project as the root as well.
        primary_modal(
            [Key::char(' '), Key::char('/'), Key::char('a')],
            Command::OpenAllFilesPicker,
        ),
        primary_modal(
            [Key::char(' '), Key::char('/'), Key::char('p')],
            Command::OpenPathFilePicker,
        ),
        primary_modal(
            [Key::char(' '), Key::char('s'), Key::char('e')],
            Command::SplitSelectionAtLineEnds,
        ),
        primary_modal(
            [Key::char(' '), Key::char('s'), Key::char('b')],
            Command::SplitSelectionAtLineStarts,
        ),
        primary_modal(
            [Key::char(' '), Key::char('s'), Key::char('c')],
            Command::KeepPrimarySelection,
        )
        .with_alias(Key::char(',')),
        // `&` is the Helix spelling and stays the fast one; someone who arrives
        // here through the namespace should still learn it exists.
        primary_modal(
            [Key::char(' '), Key::char('s'), Key::char('a')],
            Command::AlignSelections,
        )
        .with_alias(Key::char('&')),
        primary_modal(
            [Key::char(' '), Key::char('s'), Key::char('k')],
            Command::KeepMatchingSelections,
        ),
        primary_modal(
            [Key::char(' '), Key::char('s'), Key::char('r')],
            Command::RemoveMatchingSelections,
        ),
        primary_modal([Key::char(' '), Key::char('r')], ColonCommand::Reload),
        // Git views and refreshes. Mutations and commit preparation live in
        // the changed-file list's contextual action menu.
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('b')],
            ColonCommand::GitBranches,
        ),
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('g')],
            ColonCommand::GitStatus,
        ),
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('w')],
            ColonCommand::GitWorktrees,
        ),
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('l')],
            ColonCommand::GitLog,
        ),
        // `f` reads as the finder wherever it appears, and this is a finder
        // over commits rather than the buffer search `/` now spells. Commits
        // are the only Git corpus large enough to need one; branches and
        // stashes are lists you read.
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('f')],
            ColonCommand::GitSearchCommits,
        ),
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('B')],
            ColonCommand::GitBlameFile,
        ),
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('d')],
            ColonCommand::GitDiff,
        ),
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('D')],
            ColonCommand::GitDiffSideBySide,
        ),
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('r')],
            ColonCommand::GitRefresh,
        ),
        primary_modal(
            [Key::char(' '), Key::char('g'), Key::char('t')],
            ColonCommand::GitStashes,
        ),
        primary_modal(
            [Key::char(' '), Key::char('c'), Key::char('y')],
            Command::ClipboardYank,
        ),
        primary_modal(
            [Key::char(' '), Key::char('c'), Key::char('p')],
            Command::ClipboardPasteAfter,
        ),
        primary_modal(
            [Key::char(' '), Key::char('c'), Key::char('P')],
            Command::ClipboardPasteBefore,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('h')],
            Command::ShowDocumentation,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('c')],
            Command::TriggerCompletion,
        )
        .with_alias_in(INSERT, Key::ctrl('x')),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('s')],
            Command::DocumentSymbols,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('S')],
            Command::WorkspaceSymbols,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('d')],
            Command::Diagnostics,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('r')],
            Command::RenameSymbol,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('a')],
            Command::CodeAction,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('f')],
            ColonCommand::Format,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('R')],
            ColonCommand::LspRestart,
        ),
        primary_modal(
            [Key::char(' '), Key::char('l'), Key::char('?')],
            ColonCommand::LspStatus,
        ),
        primary_modal(
            [Key::char(' '), Key::char('o'), Key::char('o')],
            Command::OpenSettings,
        ),
        primary_modal(
            [Key::char(' '), Key::char('o'), Key::char('t')],
            Command::OpenThemeSettings,
        ),
        primary_modal(
            [Key::char(' '), Key::char('o'), Key::char('s')],
            ColonCommand::ServiceHealth,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('l'),
                Key::char('g'),
                Key::char('d'),
            ],
            Command::GotoDefinition,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('l'),
                Key::char('g'),
                Key::char('D'),
            ],
            Command::GotoDeclaration,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('l'),
                Key::char('g'),
                Key::char('y'),
            ],
            Command::GotoTypeDefinition,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('l'),
                Key::char('g'),
                Key::char('r'),
            ],
            Command::GotoReferences,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('l'),
                Key::char('g'),
                Key::char('i'),
            ],
            Command::GotoImplementation,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('e')],
            Command::ExpandSyntaxSelection,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('s')],
            Command::ShrinkSyntaxSelection,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('p')],
            Command::SelectSyntaxParent,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('c')],
            Command::SelectSyntaxChild,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('h')],
            Command::SelectPreviousSyntaxSibling,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('l')],
            Command::SelectNextSyntaxSibling,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('o')],
            Command::DocumentOutline,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('f')],
            Command::FoldAllSyntax,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('x')],
            Command::ToggleSyntaxFold,
        ),
        primary_modal(
            [Key::char(' '), Key::char('x'), Key::char('u')],
            Command::UnfoldAllSyntax,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('f'),
            ],
            Command::SelectSyntaxFunction,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('c'),
            ],
            Command::SelectSyntaxClass,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('p'),
            ],
            Command::SelectSyntaxParameter,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('f'),
            ],
            Command::SelectInsideSyntaxFunction,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('c'),
            ],
            Command::SelectInsideSyntaxClass,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('p'),
            ],
            Command::SelectInsideSyntaxParameter,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('('),
            ],
            Command::SelectAroundParentheses,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char(')'),
            ],
            Command::SelectAroundParentheses,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('('),
            ],
            Command::SelectInsideParentheses,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char(')'),
            ],
            Command::SelectInsideParentheses,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('['),
            ],
            Command::SelectAroundSquareBrackets,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char(']'),
            ],
            Command::SelectAroundSquareBrackets,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('['),
            ],
            Command::SelectInsideSquareBrackets,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char(']'),
            ],
            Command::SelectInsideSquareBrackets,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('{'),
            ],
            Command::SelectAroundBraces,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('}'),
            ],
            Command::SelectAroundBraces,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('{'),
            ],
            Command::SelectInsideBraces,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('}'),
            ],
            Command::SelectInsideBraces,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('<'),
            ],
            Command::SelectAroundAngleBrackets,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('>'),
            ],
            Command::SelectAroundAngleBrackets,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('<'),
            ],
            Command::SelectInsideAngleBrackets,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('>'),
            ],
            Command::SelectInsideAngleBrackets,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('"'),
            ],
            Command::SelectAroundDoubleQuotes,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('"'),
            ],
            Command::SelectInsideDoubleQuotes,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('\''),
            ],
            Command::SelectAroundSingleQuotes,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('\''),
            ],
            Command::SelectInsideSingleQuotes,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('`'),
            ],
            Command::SelectAroundBackticks,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('`'),
            ],
            Command::SelectInsideBackticks,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('a'),
                Key::char('m'),
            ],
            Command::SelectAroundClosestDelimiter,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('i'),
                Key::char('m'),
            ],
            Command::SelectInsideClosestDelimiter,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('['),
                Key::char('f'),
            ],
            Command::GotoPreviousSyntaxFunction,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('['),
                Key::char('c'),
            ],
            Command::GotoPreviousSyntaxClass,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char('['),
                Key::char('p'),
            ],
            Command::GotoPreviousSyntaxParameter,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char(']'),
                Key::char('f'),
            ],
            Command::GotoNextSyntaxFunction,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char(']'),
                Key::char('c'),
            ],
            Command::GotoNextSyntaxClass,
        ),
        primary_modal(
            [
                Key::char(' '),
                Key::char('x'),
                Key::char(']'),
                Key::char('p'),
            ],
            Command::GotoNextSyntaxParameter,
        ),
        modal([Key::ctrl('w'), Key::char('w')], Command::NextWindow),
        modal([Key::ctrl('w'), Key::ctrl('w')], Command::NextWindow),
        modal([Key::ctrl('w'), Key::char('x')], Command::SwapWindow),
        modal([Key::ctrl('w'), Key::char('v')], Command::SplitVertical),
        modal([Key::ctrl('w'), Key::ctrl('v')], Command::SplitVertical),
        modal([Key::ctrl('w'), Key::char('s')], Command::SplitHorizontal),
        modal([Key::ctrl('w'), Key::ctrl('s')], Command::SplitHorizontal),
        modal([Key::ctrl('w'), Key::char('c')], Command::CloseWindow),
        modal([Key::ctrl('w'), Key::char('o')], Command::OnlyWindow),
        modal([Key::ctrl('w'), Key::ctrl('o')], Command::OnlyWindow),
        modal([Key::ctrl('w'), Key::char('f')], Command::ToggleFullscreen),
        modal([Key::ctrl('w'), Key::char('z')], Command::ToggleZen),
        modal([Key::ctrl('w'), Key::char('t')], Command::OpenTerminal),
        modal([Key::ctrl('w'), Key::char('h')], Command::FocusWindowLeft),
        modal([Key::ctrl('w'), Key::ctrl('h')], Command::FocusWindowLeft),
        modal([Key::ctrl('w'), Key::char('j')], Command::FocusWindowDown),
        modal([Key::ctrl('w'), Key::ctrl('j')], Command::FocusWindowDown),
        modal([Key::ctrl('w'), Key::char('k')], Command::FocusWindowUp),
        modal([Key::ctrl('w'), Key::ctrl('k')], Command::FocusWindowUp),
        modal([Key::ctrl('w'), Key::char('l')], Command::FocusWindowRight),
        modal([Key::ctrl('w'), Key::ctrl('l')], Command::FocusWindowRight),
        insert([Key::ctrl('w'), Key::char('w')], Command::NextWindow),
        insert([Key::ctrl('w'), Key::ctrl('w')], Command::NextWindow),
        insert([Key::ctrl('w'), Key::char('x')], Command::SwapWindow),
        insert([Key::ctrl('w'), Key::char('h')], Command::FocusWindowLeft),
        insert([Key::ctrl('w'), Key::ctrl('h')], Command::FocusWindowLeft),
        insert([Key::ctrl('w'), Key::char('j')], Command::FocusWindowDown),
        insert([Key::ctrl('w'), Key::ctrl('j')], Command::FocusWindowDown),
        insert([Key::ctrl('w'), Key::char('k')], Command::FocusWindowUp),
        insert([Key::ctrl('w'), Key::ctrl('k')], Command::FocusWindowUp),
        insert([Key::ctrl('w'), Key::char('l')], Command::FocusWindowRight),
        insert([Key::ctrl('w'), Key::ctrl('l')], Command::FocusWindowRight),
        terminal_insert([Key::ctrl('w'), Key::char('v')], Command::SplitVertical),
        terminal_insert([Key::ctrl('w'), Key::ctrl('v')], Command::SplitVertical),
        terminal_insert([Key::ctrl('w'), Key::char('s')], Command::SplitHorizontal),
        terminal_insert([Key::ctrl('w'), Key::ctrl('s')], Command::SplitHorizontal),
        terminal_insert([Key::ctrl('w'), Key::char('f')], Command::ToggleFullscreen),
        terminal_insert([Key::ctrl('w'), Key::char('z')], Command::ToggleZen),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('w')],
            Command::NextWindow,
        ),
        primary_modal(
            [Key::char(' '), Key::char('w'), Key::char('x')],
            Command::SwapWindow,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('v')],
            Command::SplitVertical,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('s')],
            Command::SplitHorizontal,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('c')],
            Command::CloseWindow,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('o')],
            Command::OnlyWindow,
        ),
        // `=` says level rather than naming a direction, the way Vim's
        // `Ctrl-w =` does, and nothing else in the window namespace wants it.
        modal(
            [Key::char(' '), Key::char('w'), Key::char('=')],
            Command::EqualizeWindows,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('f')],
            Command::ToggleFullscreen,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('z')],
            Command::ToggleZen,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('h')],
            Command::FocusWindowLeft,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('j')],
            Command::FocusWindowDown,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('k')],
            Command::FocusWindowUp,
        ),
        modal(
            [Key::char(' '), Key::char('w'), Key::char('l')],
            Command::FocusWindowRight,
        ),
        modal(Key::char('"'), Command::SelectRegister),
        // Macros live in one namespace rather than on `Q` and `q`, which say
        // nothing about what they do. `Space m m` both starts and stops the
        // default recording, so nothing else has to be remembered to finish
        // one.
        modal(
            [Key::char(' '), Key::char('m'), Key::char('m')],
            Command::RecordDefaultMacro,
        ),
        modal(
            [Key::char(' '), Key::char('m'), Key::char('M')],
            Command::RecordMacro,
        ),
        modal(
            [Key::char(' '), Key::char('m'), Key::char('r')],
            Command::ReplayDefaultMacro,
        ),
        modal(
            [Key::char(' '), Key::char('m'), Key::char('R')],
            Command::ReplayMacro,
        ),
        modal(
            [Key::char(' '), Key::char('m'), Key::char('l')],
            Command::ListMacros,
        ),
        // Enter alone opens an entry. A directory buffer is an ordinary buffer,
        // so `e` has to stay the word-end motion here as it is everywhere else.
        directory(Key::plain(KeyCode::Enter), Command::OpenDirectoryEntry),
        directory(Key::plain(KeyCode::Backspace), Command::OpenParentDirectory),
        directory(Key::char('-'), Command::OpenParentDirectory),
        // The global dot now toggles visible whitespace. Keep dotfile
        // discovery on the same mnemonic behind Space in explorer buffers.
        directory(Key::char('.'), Command::ToggleHiddenFiles),
        directory(Key::char('?'), Command::ToggleDirectoryDetails),
        // The same question in a document: `?` asks a Markdown file to show
        // itself as it is meant to be read, and asks the page it opened to go
        // back to the source. Nothing else answers it, so the key stays free
        // everywhere it would have nothing to say.
        markdown(Key::char('?'), Command::ToggleMarkdownRender),
        // A directory buffer deliberately leaves the split sequences to the
        // global bindings: splitting an explorer shows the same listing twice,
        // exactly as splitting a file shows the same text twice.
        // Enter remains the direct primary action in row-oriented views. Every
        // other contextual operation lives in the Tab menu below, so normal
        // buffer bindings remain available here unchanged.
        help_scope(Key::char('q'), ColonCommand::CloseBuffer),
        git_status(Key::plain(KeyCode::Enter), ColonCommand::GitDiff),
        git_branches(Key::plain(KeyCode::Enter), Command::CheckoutBranch),
        git_worktrees(Key::plain(KeyCode::Enter), Command::OpenWorktree),
        git_log(Key::plain(KeyCode::Enter), Command::OpenGitCommit),
        // Paging lives on Ctrl chords so the Git log keeps every motion key.
        git_log(Key::ctrl('n'), Command::NextGitLogPage),
        git_log(Key::ctrl('p'), Command::PreviousGitLogPage),
        git_blame(Key::plain(KeyCode::Enter), Command::OpenGitCommit),
        workspace_search(
            Key::plain(KeyCode::Enter),
            Command::OpenWorkspaceSearchResult,
        ),
        settings(Key::plain(KeyCode::Enter), Command::ActivateSetting),
        unsupported(
            Key::char('|'),
            Command::ShellPipe,
            "shell pipes are not available",
        ),
        modal([Key::char('m'), Key::char('m')], Command::MatchBracket),
        insert(Key::plain(KeyCode::Escape), Command::EnterNormalMode),
        insert(Key::ctrl('\\'), Command::EnterNormalMode),
        insert(Key::ctrl('4'), Command::EnterNormalMode).with_role(BindingRole::Compatibility),
        insert(
            Key::new(KeyCode::Backspace, Modifiers::ALT),
            Command::DeleteWordBackward,
        ),
        insert(
            Key::new(KeyCode::Delete, Modifiers::ALT),
            Command::DeleteWordForward,
        ),
        insert(Key::ctrl('u'), Command::DeleteToLineStart),
        insert(Key::ctrl('k'), Command::DeleteToLineEnd),
        insert(Key::ctrl('c'), Command::ToggleComments),
        insert(Key::plain(KeyCode::Backspace), Command::DeleteCharBackward)
            .with_alias(Key::new(KeyCode::Backspace, Modifiers::SHIFT)),
        insert(
            Key::new(KeyCode::Backspace, Modifiers::SHIFT),
            Command::DeleteCharBackward,
        )
        .with_role(BindingRole::Compatibility),
        insert(Key::plain(KeyCode::Delete), Command::DeleteCharForward),
        insert(Key::ctrl('j'), Command::InsertNewline),
        insert(Key::plain(KeyCode::Enter), Command::InsertNewline),
        insert(Key::plain(KeyCode::Tab), Command::InsertTab),
        insert(Key::ctrl('x'), Command::TriggerCompletion),
        insert(
            Key::new(KeyCode::BackTab, Modifiers::SHIFT),
            Command::InsertLiteralTab,
        ),
        insert(Key::plain(KeyCode::Left), Command::MoveLeft),
        insert(Key::plain(KeyCode::Right), Command::MoveRight),
        insert(Key::plain(KeyCode::Up), Command::MoveUp),
        insert(Key::plain(KeyCode::Down), Command::MoveDown),
        insert(Key::plain(KeyCode::Home), Command::MoveLineStart),
        insert(Key::plain(KeyCode::End), Command::MoveLineEnd),
        insert(Key::plain(KeyCode::PageUp), Command::PageUp),
        insert(Key::plain(KeyCode::PageDown), Command::PageDown),
        // Preserve Runyte's original global shortcuts in Insert mode.
        insert(Key::ctrl('s'), Command::Save),
    ]
}

/// The four keys single-key pane movement claims, and where each one goes.
///
/// One list rather than four bindings written out, because the same four keys
/// have to be recognised twice: once here, to build the bindings, and once by
/// the terminal, which owns its keyboard and only lets a key through when
/// something above names it.
const FAST_PANE_MOVES: [(char, EditorCommand); 4] = [
    ('h', EditorCommand::FocusWindowLeft),
    ('j', EditorCommand::FocusWindowDown),
    ('k', EditorCommand::FocusWindowUp),
    ('l', EditorCommand::FocusWindowRight),
];

/// Whether `key` is one of the single-key pane moves.
///
/// Says nothing about whether the option is on: that is the caller's
/// configuration to read, not the keymap's.
pub fn is_fast_pane_key(key: Key) -> bool {
    let key = key.canonical_for_binding();
    FAST_PANE_MOVES
        .iter()
        .any(|(character, _)| key == Key::ctrl(*character))
}

/// `Ctrl-w h` and `Space w h` one keystroke shorter, for people who move
/// between panes the way tmux does.
///
/// Modal and Insert both, so the keys mean the same thing wherever the cursor
/// is; a pane move that worked only in NORMAL would still need an Escape
/// first, which is the keystroke the option exists to remove.
fn fast_pane_bindings() -> Vec<Binding> {
    FAST_PANE_MOVES
        .into_iter()
        .flat_map(|(character, command)| {
            [MODAL, INSERT].map(|modes| {
                Binding::implemented(modes, Key::ctrl(character), command)
                    .with_role(BindingRole::Fast)
                    .with_alias([Key::ctrl('w'), Key::char(character)])
            })
        })
        .collect()
}

/// Adds the fast pane keys, letting them win every key they collide with.
///
/// Shadowing rather than refusing to build: `Ctrl-j` and `Ctrl-k` already
/// insert a newline and delete to end of line in Insert mode, and someone who
/// turns this on has decided pane movement is worth more than those. Dropping
/// the loser here rather than in dispatch keeps one answer per key in the
/// registry, so help and the hint popup describe what the keys actually do.
fn with_fast_pane_keys(mut bindings: Vec<Binding>) -> Vec<Binding> {
    let fast = fast_pane_bindings();
    bindings.retain(|binding| {
        binding.scope != BindingScope::Global
            || !fast.iter().any(|replacement| {
                replacement.sequence == binding.sequence
                    && replacement
                        .modes
                        .iter()
                        .any(|mode| binding.is_active_in(*mode))
            })
    });
    bindings.extend(fast);
    bindings
}

fn scope_includes(active: BindingScope, binding: BindingScope) -> bool {
    active == binding
}

fn build_keymap(bindings: Vec<Binding>) -> Keymap {
    let namespaces = vec![
        // Top-level prefixes. These never reach the hint popup, which only
        // lists continuations of a sequence already begun, but they are what
        // a view names when it tells a reader where they can start.
        BindingNamespace::global(MODAL, Key::char(' '), "Application commands"),
        BindingNamespace::global(MODAL, Key::char('g'), "Goto"),
        BindingNamespace::global(MODAL, Key::char('m'), "Match"),
        BindingNamespace::global(MODAL, Key::char('z'), "View alignment"),
        BindingNamespace::global(MODAL, Key::char('Z'), "View alignment, staying open"),
        BindingNamespace::global(MODAL, Key::ctrl('w'), "Pane commands"),
        BindingNamespace::global(INSERT, Key::ctrl('w'), "Move between panes"),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('b')], "Buffers"),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('c')], "Clipboard"),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('g')], "Git")
            .with_capability(CommandCapability::GitProject),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('l')], "Language (LSP)")
            .with_capability(CommandCapability::LspDocument),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('m')], "Macros"),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('o')], "Configuration"),
        BindingNamespace::global(
            MODAL,
            [Key::char(' '), Key::char('p')],
            "Text layout and whitespace",
        ),
        BindingNamespace::global(
            MODAL,
            [Key::char(' '), Key::char('l'), Key::char('g')],
            "Language navigation",
        )
        .with_capability(CommandCapability::LspDocument),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('s')], "Selections"),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('t')], "Terminals"),
        BindingNamespace::global(
            MODAL,
            [Key::char(' '), Key::char('/')],
            "Look past this buffer",
        ),
        BindingNamespace::global(MODAL, [Key::char(' '), Key::char('w')], "Windows"),
        BindingNamespace::global(
            MODAL,
            [Key::char(' '), Key::char('x')],
            "Syntax (Tree-sitter)",
        )
        .with_capability(CommandCapability::Syntax),
        BindingNamespace::global(
            MODAL,
            [Key::char(' '), Key::char('x'), Key::char('a')],
            "Select around",
        )
        .with_capability(CommandCapability::Syntax),
        BindingNamespace::global(
            MODAL,
            [Key::char(' '), Key::char('x'), Key::char('i')],
            "Select inside",
        )
        .with_capability(CommandCapability::Syntax),
        BindingNamespace::global(
            MODAL,
            [Key::char(' '), Key::char('x'), Key::char('[')],
            "Previous syntax object",
        )
        .with_capability(CommandCapability::Syntax),
        BindingNamespace::global(
            MODAL,
            [Key::char(' '), Key::char('x'), Key::char(']')],
            "Next syntax object",
        )
        .with_capability(CommandCapability::Syntax),
    ];
    let actions = vec![
        // Row actions lead. Buffer-wide actions follow them, so the menu reads
        // from the object under the cursor out to the view that contains it.
        ContextAction::row(
            BindingScope::GitStatus,
            Key::char('s'),
            "stage",
            ColonCommand::GitStage,
        )
        .with_description("Stage every file the selection covers"),
        ContextAction::row(
            BindingScope::GitStatus,
            Key::char('u'),
            "unstage",
            ColonCommand::GitUnstage,
        )
        .with_description("Unstage every file the selection covers"),
        ContextAction::row(
            BindingScope::GitStatus,
            Key::char('D'),
            "discard",
            ColonCommand::GitDiscard,
        )
        .with_description("Discard every selected file's changes, after a confirmation"),
        ContextAction::row(
            BindingScope::GitStatus,
            Key::char('o'),
            "open",
            EditorCommand::OpenChangedFile,
        ),
        ContextAction::buffer(
            BindingScope::GitStatus,
            Key::char('S'),
            "stage",
            EditorCommand::StageAllChangedFiles,
        ),
        ContextAction::buffer(
            BindingScope::GitStatus,
            Key::char('c'),
            "commit",
            ColonCommand::GitCommit,
        ),
        ContextAction::buffer(
            BindingScope::GitStatus,
            Key::char('i'),
            "index",
            ColonCommand::GitIndex,
        ),
        ContextAction::buffer(
            BindingScope::GitStatus,
            Key::char('p'),
            "pull",
            EditorCommand::PullBranch,
        ),
        ContextAction::buffer(
            BindingScope::GitStatus,
            Key::char('P'),
            "push",
            EditorCommand::PushBranch,
        ),
        ContextAction::row(
            BindingScope::GitBranches,
            Key::char('n'),
            "create",
            EditorCommand::CreateBranch,
        ),
        ContextAction::row(
            BindingScope::GitBranches,
            Key::char('w'),
            "worktree",
            EditorCommand::CreateWorktree,
        ),
        ContextAction::row(
            BindingScope::GitBranches,
            Key::char('D'),
            "delete",
            EditorCommand::DeleteBranch,
        ),
        ContextAction::buffer(
            BindingScope::GitBranches,
            Key::char('p'),
            "pull",
            EditorCommand::PullBranch,
        ),
        ContextAction::row(
            BindingScope::GitBranches,
            Key::char('P'),
            "push",
            EditorCommand::PushBranch,
        ),
        ContextAction::row(
            BindingScope::GitWorktrees,
            Key::char('n'),
            "branch",
            EditorCommand::CreateNewWorktree,
        ),
        ContextAction::row(
            BindingScope::GitWorktrees,
            Key::char('D'),
            "remove",
            EditorCommand::RemoveWorktree,
        ),
        ContextAction::row(
            BindingScope::GitStash,
            Key::char('a'),
            "apply",
            ColonCommand::GitStashApply,
        ),
        ContextAction::row(
            BindingScope::GitStash,
            Key::char('D'),
            "drop",
            ColonCommand::GitStashDrop,
        ),
        ContextAction::row(
            BindingScope::Diff,
            Key::char('s'),
            "stage",
            ColonCommand::GitStageHunk,
        ),
        ContextAction::row(
            BindingScope::Diff,
            Key::char('u'),
            "unstage",
            ColonCommand::GitUnstageHunk,
        ),
    ];
    Keymap::with_namespaces(bindings, namespaces)
        .expect("the built-in keymap must not contain duplicate bindings")
        .with_context_actions(actions)
}

static DEFAULT_KEYMAP: LazyLock<Arc<Keymap>> =
    LazyLock::new(|| Arc::new(build_keymap(built_in_bindings())));

static FAST_PANE_KEYMAP: LazyLock<Arc<Keymap>> =
    LazyLock::new(|| Arc::new(build_keymap(with_fast_pane_keys(built_in_bindings()))));

pub fn default_keymap() -> &'static Keymap {
    &DEFAULT_KEYMAP
}

/// The keymap the editor answers to, given whether single-key pane movement
/// is configured on.
///
/// Two whole keymaps rather than an exception inside dispatch, because key
/// execution, help, and the hint popup all read the registry: an option only
/// dispatch knew about would move panes on a key that help still swore
/// deleted to end of line.
pub fn keymap_for(fast_pane_keys: bool) -> Arc<Keymap> {
    if fast_pane_keys {
        Arc::clone(&FAST_PANE_KEYMAP)
    } else {
        Arc::clone(&DEFAULT_KEYMAP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built_in_keymaps() -> [(&'static str, Arc<Keymap>); 2] {
        [
            ("default", keymap_for(false)),
            ("fast pane", keymap_for(true)),
        ]
    }

    /// Help renders one document per buffer type rather than one per mode,
    /// which is only honest while the two modal modes bind the same keys to
    /// the same commands. A Normal-only or Select-only binding would be
    /// documented in both or neither, so it has to be a deliberate change:
    /// add the mode back to `help::render` before making this pass again.
    #[test]
    fn normal_and_select_bind_the_same_sequences() {
        for (keymap_name, keymap) in built_in_keymaps() {
            for &scope in BindingScope::ALL {
                let semantics = |mode| {
                    let mut rows = keymap
                        .bindings_for_scope(mode, scope)
                        .map(|binding| {
                            (
                                binding.sequence.to_string(),
                                binding.target,
                                binding.description,
                                binding.availability,
                                binding.role,
                                binding.alias.clone(),
                                binding.alias_modes,
                            )
                        })
                        .collect::<Vec<_>>();
                    rows.sort_by(|left, right| left.0.cmp(&right.0));
                    rows
                };
                assert_eq!(
                    semantics(Mode::Normal),
                    semantics(Mode::Select),
                    "{keymap_name} keymap {scope:?} no longer exposes the same bindings in both modal modes"
                );
            }
        }
    }

    /// The user guide calls its Editing table the complete direct-binding
    /// inventory. Keep that claim tied to the registry: prose may group
    /// related keys into one row, but every one-key modal sequence must still
    /// be named exactly somewhere in the table.
    #[test]
    fn user_guide_covers_every_direct_editing_binding() {
        let guide =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/user-guide.md"))
                .expect("read the user guide");
        let editing = guide
            .split_once("### Editing")
            .expect("user guide has an Editing section")
            .1
            .split_once("<a id=\"insert-mode\"></a>")
            .expect("Editing section ends at the Insert-mode anchor")
            .0
            .replace("\\|", "|");

        let insert = guide
            .split_once("### Insert and Replace modes")
            .expect("user guide has an Insert and Replace modes section")
            .1
            .split_once("### Search")
            .expect("Insert and Replace modes section ends at Search")
            .0;

        let missing_from = |mode, section: &str| {
            default_keymap()
                .bindings_for_mode(mode)
                .filter(|binding| binding.sequence.len() == 1)
                .map(|binding| binding.sequence.to_string())
                .map(|label| {
                    if label == "Shift-BackTab" {
                        "Shift-Tab".to_owned()
                    } else {
                        label
                    }
                })
                .filter(|label| !section.contains(&format!("`{label}`")))
                .collect::<Vec<_>>()
        };

        let missing = missing_from(Mode::Normal, &editing);
        assert!(
            missing.is_empty(),
            "docs/user-guide.md Editing table is missing direct modal bindings: {}",
            missing.join(", ")
        );

        let missing = missing_from(Mode::Insert, insert);
        assert!(
            missing.is_empty(),
            "docs/user-guide.md Insert and Replace modes section is missing direct bindings: {}",
            missing.join(", ")
        );

        assert!(
            editing.contains("| `s` / `/` | Search with an escaped literal"),
            "the two buffer-search flavours must be documented on `s` and `/`"
        );
        assert!(
            !editing.contains("| `/` | Open the project finder"),
            "the retired direct `/` finder binding is still documented"
        );
    }

    /// Every top-level prefix can be named. A prefix with nothing to call
    /// itself is a key a reader can only find by accident, which is the gap
    /// entry points exist to close.
    #[test]
    fn every_entry_point_can_name_itself() {
        for (keymap_name, keymap) in built_in_keymaps() {
            for mode in [Mode::Normal, Mode::Select] {
                for &scope in BindingScope::ALL {
                    for entry in keymap.entry_points(mode, scope) {
                        assert!(
                            !entry.description.is_empty(),
                            "{keymap_name} keymap {scope:?} {mode:?} entry point {} has no description",
                            entry.key.label()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn entry_points_separate_scoped_keys_from_global_ones() {
        let entries = default_keymap().entry_points(Mode::Normal, BindingScope::GitStatus);
        let scoped = entries
            .iter()
            .take_while(|entry| entry.scoped)
            .map(|entry| entry.key.label())
            .collect::<Vec<_>>();
        assert_eq!(scoped, ["Enter"]);
        // Scope-specific keys sort ahead of every global one.
        assert!(entries.iter().skip(scoped.len()).all(|entry| !entry.scoped));

        // A prefix is reported as one, and takes its namespace's name.
        let space = entries
            .iter()
            .find(|entry| entry.key == Key::char(' '))
            .expect("Space starts bindings in every scope");
        assert!(space.prefix && !space.scoped);
        assert_eq!(space.description, "Application commands");

        // Space carries scoped leaves in no scope, but it is still global; a
        // key is buffer-specific only when everything under it is.
        let global = default_keymap().entry_points(Mode::Normal, BindingScope::Global);
        assert!(global.iter().all(|entry| !entry.scoped));
    }

    /// Scoped direct keys may add behavior, but never replace a key that means
    /// something globally. Contextual action mnemonics are isolated inside the
    /// Tab menu and therefore do not participate in this check.
    #[test]
    fn no_scoped_binding_shadows_a_global_binding() {
        for (keymap_name, keymap) in built_in_keymaps() {
            for mode in [Mode::Normal, Mode::Select] {
                for &scope in BindingScope::ALL {
                    assert!(
                        keymap.shadowed_bindings(mode, scope).is_empty(),
                        "{keymap_name} keymap {scope:?} shadows a global binding in {mode:?}"
                    );
                }
            }
        }
    }

    /// A namespace is presentation data for a real registry prefix, not an
    /// independent hint tree. Pin uniqueness, reachability, and the absence
    /// of exact-prefix ambiguity so discovery can neither invent a dead row
    /// nor hide an executable command behind one.
    #[test]
    fn every_namespace_is_unique_reachable_and_not_an_exact_binding() {
        for (keymap_name, keymap) in built_in_keymaps() {
            let mut seen = std::collections::HashSet::new();
            for namespace in keymap.namespaces() {
                assert!(!namespace.sequence.is_empty());
                for mode in namespace.modes {
                    assert!(
                        seen.insert((*mode, namespace.scope, namespace.sequence.clone())),
                        "duplicate {keymap_name} keymap {mode:?} {:?} namespace {}",
                        namespace.scope,
                        namespace.sequence
                    );
                    assert!(
                        matches!(
                            keymap.lookup_in(*mode, namespace.scope, &namespace.sequence),
                            Lookup::Prefix(ref bindings) if !bindings.is_empty()
                        ),
                        "dead or executable {keymap_name} keymap namespace {} in {mode:?} {:?}",
                        namespace.sequence,
                        namespace.scope
                    );
                }
            }
        }
    }

    #[test]
    fn built_in_bindings_have_no_exact_prefix_ambiguity() {
        for (keymap_name, keymap) in built_in_keymaps() {
            for binding in keymap.bindings() {
                for mode in binding.modes {
                    assert!(
                        !matches!(
                            keymap.lookup_in(*mode, binding.scope, &binding.sequence),
                            Lookup::ExactAndPrefix { .. }
                        ),
                        "{keymap_name} keymap {} is both executable and a prefix in {mode:?} {:?}",
                        binding.sequence,
                        binding.scope
                    );
                }
            }
        }
    }

    #[test]
    fn scoped_buffer_inventory_excludes_global_and_terminal_pane_scopes() {
        assert!(!BindingScope::Global.is_special_buffer_scope());
        assert!(!BindingScope::Terminal.is_special_buffer_scope());
        let special = BindingScope::ALL
            .iter()
            .copied()
            .filter(|scope| scope.is_special_buffer_scope())
            .count();
        assert_eq!(
            special, 12,
            "special-buffer scope inventory changed; update the UI vocabulary"
        );
    }

    #[test]
    fn contextual_actions_are_ordered_and_registry_backed() {
        let actions = default_keymap()
            .context_actions(BindingScope::GitStatus)
            .map(|action| {
                (
                    action.mnemonic.label(),
                    action.target.name(),
                    action.context,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actions,
            vec![
                ("s".to_owned(), "git-stage", ActionContext::Row),
                ("u".to_owned(), "git-unstage", ActionContext::Row),
                ("D".to_owned(), "git-discard", ActionContext::Row),
                ("o".to_owned(), "open-changed-file", ActionContext::Row),
                (
                    "S".to_owned(),
                    "stage-all-changed-files",
                    ActionContext::Buffer,
                ),
                ("c".to_owned(), "git-commit", ActionContext::Buffer),
                ("i".to_owned(), "git-index", ActionContext::Buffer),
                ("p".to_owned(), "pull-branch", ActionContext::Buffer),
                ("P".to_owned(), "push-branch", ActionContext::Buffer),
            ]
        );
        let branch_pull = default_keymap()
            .context_actions(BindingScope::GitBranches)
            .find(|action| action.mnemonic == Key::char('p'))
            .unwrap();
        assert_eq!(branch_pull.context, ActionContext::Buffer);
    }

    /// The menu's second column is data, not a derivation: it is written per
    /// action so the word can distinguish two actions the command names do
    /// not. Pinning it keeps the columns from drifting into a sentence, and
    /// keeps a scope from ending up with one word standing for two different
    /// actions.
    #[test]
    fn every_contextual_action_names_itself_in_one_word() {
        for scope in [
            BindingScope::GitStatus,
            BindingScope::GitBranches,
            BindingScope::GitWorktrees,
            BindingScope::GitStash,
            BindingScope::Diff,
        ] {
            for action in default_keymap().context_actions(scope) {
                assert!(
                    !action.name.is_empty()
                        && action
                            .name
                            .chars()
                            .all(|character| character.is_ascii_lowercase()),
                    "{scope:?} {} names itself {:?}",
                    action.mnemonic.label(),
                    action.name
                );
            }
        }

        let named = |scope| {
            default_keymap()
                .context_actions(scope)
                .map(|action| (action.mnemonic.label(), action.name))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            named(BindingScope::GitBranches),
            vec![
                ("n".to_owned(), "create"),
                ("w".to_owned(), "worktree"),
                ("D".to_owned(), "delete"),
                ("p".to_owned(), "pull"),
                ("P".to_owned(), "push"),
            ]
        );
        assert_eq!(
            named(BindingScope::GitWorktrees),
            vec![("n".to_owned(), "branch"), ("D".to_owned(), "remove"),]
        );
        assert_eq!(
            named(BindingScope::GitStash),
            vec![("a".to_owned(), "apply"), ("D".to_owned(), "drop")]
        );
        assert_eq!(
            named(BindingScope::Diff),
            vec![("s".to_owned(), "stage"), ("u".to_owned(), "unstage")]
        );
    }

    #[test]
    fn contextual_action_mnemonics_cannot_take_menu_controls() {
        for mnemonic in [
            Key::char('j'),
            Key::char('k'),
            Key::plain(KeyCode::Enter),
            Key::plain(KeyCode::Escape),
        ] {
            let result = std::panic::catch_unwind(|| {
                Keymap::new(Vec::new())
                    .unwrap()
                    .with_context_actions(vec![ContextAction::row(
                        BindingScope::Global,
                        mnemonic,
                        "move",
                        EditorCommand::MoveLeft,
                    )])
            });
            assert!(result.is_err(), "{} was accepted", mnemonic.label());
        }
    }

    #[test]
    fn scoped_bindings_exclude_everything_inherited_from_the_global_keymap() {
        let keymap = default_keymap();
        let scoped = keymap
            .scoped_bindings(Mode::Normal, BindingScope::GitStatus)
            .count();
        let all = keymap
            .bindings_for_scope(Mode::Normal, BindingScope::GitStatus)
            .count();
        assert_eq!(scoped, 1);
        assert!(all > scoped, "the global keymap still applies in a scope");
        assert!(
            keymap
                .scoped_bindings(Mode::Normal, BindingScope::Global)
                .next()
                .is_none(),
            "the global scope contributes nothing on top of itself"
        );
    }

    #[test]
    fn exact_prefix_and_missing_lookups_are_distinct() {
        let keymap = default_keymap();
        let goto = KeySequence::from(Key::char('g'));
        let goto_start = KeySequence::from([Key::char('g'), Key::char('g')]);

        assert!(matches!(
            keymap.lookup(Mode::Normal, &goto),
            Lookup::Prefix(bindings)
                if bindings.iter().any(|binding| binding.target == BindingTarget::Editor(EditorCommand::MoveFileStart))
                    && bindings.iter().any(|binding| binding.target == BindingTarget::Editor(EditorCommand::GotoDefinition))
        ));
        assert!(matches!(
            keymap.lookup(Mode::Normal, &goto_start),
            Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::MoveFileStart)
        ));
        assert!(matches!(
            keymap.lookup(Mode::Normal, &KeySequence::from(Key::char('!'))),
            Lookup::NoMatch
        ));
    }

    #[test]
    fn aliases_reach_the_command_they_are_advertised_on() {
        let keymap = default_keymap();
        let aliased = keymap
            .bindings()
            .iter()
            .filter(|binding| binding.alias.is_some())
            .count();
        assert!(aliased > 0, "no binding advertises an alias any more");

        for binding in keymap.bindings() {
            let Some(alias) = binding.alias.as_ref() else {
                assert!(
                    binding.alias_modes.is_none(),
                    "{} has alias modes without an alias",
                    binding.sequence
                );
                continue;
            };
            assert_ne!(
                *alias, binding.sequence,
                "{} advertises itself as its own alias",
                binding.sequence
            );
            for mode in binding.alias_modes.unwrap_or(binding.modes) {
                let resolved = match keymap.lookup_in(*mode, binding.scope, alias) {
                    Lookup::Exact(found) | Lookup::ExactAndPrefix { exact: found, .. } => {
                        found.target
                    }
                    _ => panic!(
                        "{alias}, advertised by {}, runs nothing in {}",
                        binding.sequence,
                        mode.label()
                    ),
                };
                assert_eq!(
                    resolved,
                    binding.target,
                    "{alias}, advertised by {}, runs a different command in {}",
                    binding.sequence,
                    mode.label()
                );
            }
        }
    }

    /// Vim's `H`, `M`, and `L` move the cursor within the viewport; the
    /// `z` family scrolls the viewport under a stationary cursor. Binding the
    /// former to the latter would be the easy mistake, so this pins each to
    /// the goto command its `g` sequence already runs.
    #[test]
    fn vim_window_letters_alias_the_goto_window_motions() {
        let keymap = default_keymap();
        let cases = [
            (Key::char('H'), Key::char('t'), EditorCommand::GotoWindowTop),
            (
                Key::char('M'),
                Key::char('c'),
                EditorCommand::GotoWindowCenter,
            ),
            (
                Key::char('L'),
                Key::char('b'),
                EditorCommand::GotoWindowBottom,
            ),
        ];
        for mode in [Mode::Normal, Mode::Select] {
            for (letter, suffix, command) in cases {
                let goto = KeySequence::from([Key::char('g'), suffix]);
                for sequence in [KeySequence::from(letter), goto] {
                    let resolved = match keymap.lookup_in(mode, BindingScope::Global, &sequence) {
                        Lookup::Exact(found) | Lookup::ExactAndPrefix { exact: found, .. } => {
                            found.target
                        }
                        _ => panic!("{sequence} runs nothing in {}", mode.label()),
                    };
                    assert_eq!(
                        resolved,
                        BindingTarget::from(command),
                        "{sequence} runs a different command in {}",
                        mode.label()
                    );
                }
            }
        }
    }

    #[test]
    fn built_in_minor_modes_are_queryable_as_prefixes() {
        let keymap = default_keymap();
        let cases = [
            KeySequence::from(Key::char('g')),
            KeySequence::from(Key::char(' ')),
            KeySequence::from(Key::ctrl('w')),
        ];

        for prefix in cases {
            assert!(matches!(
                keymap.lookup(Mode::Normal, &prefix),
                Lookup::Prefix(bindings) if !bindings.is_empty()
            ));
        }
    }

    #[test]
    fn space_space_is_the_exact_session_list_command_in_both_modal_modes() {
        let sequence = KeySequence::from([Key::char(' '), Key::char(' ')]);
        for mode in [Mode::Normal, Mode::Select] {
            assert!(matches!(
                default_keymap().lookup(mode, &sequence),
                Lookup::Exact(binding)
                    if binding.target == BindingTarget::Colon(ColonCommand::SessionList)
            ));
        }
    }

    /// The session manager moved off the shifted `W`, which is now free.
    #[test]
    fn space_shift_w_is_unbound() {
        let sequence = KeySequence::from([Key::char(' '), Key::char('W')]);
        for mode in [Mode::Normal, Mode::Select] {
            assert!(matches!(
                default_keymap().lookup(mode, &sequence),
                Lookup::NoMatch
            ));
        }
    }

    #[test]
    fn syntax_folding_uses_the_unshifted_namespace_bindings() {
        let keymap = default_keymap();
        for mode in [Mode::Normal, Mode::Select] {
            for (suffix, command) in [
                ('f', EditorCommand::FoldAllSyntax),
                ('x', EditorCommand::ToggleSyntaxFold),
                ('u', EditorCommand::UnfoldAllSyntax),
            ] {
                let sequence =
                    KeySequence::from([Key::char(' '), Key::char('x'), Key::char(suffix)]);
                assert!(matches!(
                    keymap.lookup(mode, &sequence),
                    Lookup::Exact(binding)
                        if binding.target == BindingTarget::Editor(command)
                ));
            }

            let removed = KeySequence::from([Key::char(' '), Key::char('x'), Key::char('F')]);
            assert!(matches!(keymap.lookup(mode, &removed), Lookup::NoMatch));
        }
    }

    #[test]
    fn removed_short_space_actions_stay_unbound_and_splits_keep_their_namespaces() {
        let keymap = default_keymap();
        for suffix in ['h', 'v'] {
            assert!(matches!(
                keymap.lookup(
                    Mode::Normal,
                    &KeySequence::from([Key::char(' '), Key::char(suffix)])
                ),
                Lookup::NoMatch
            ));
        }
        for (sequence, command) in [
            (
                KeySequence::from([Key::char(' '), Key::char('w'), Key::char('v')]),
                EditorCommand::SplitVertical,
            ),
            (
                KeySequence::from([Key::char(' '), Key::char('w'), Key::char('s')]),
                EditorCommand::SplitHorizontal,
            ),
            (
                KeySequence::from([Key::ctrl('w'), Key::char('v')]),
                EditorCommand::SplitVertical,
            ),
            (
                KeySequence::from([Key::ctrl('w'), Key::char('s')]),
                EditorCommand::SplitHorizontal,
            ),
        ] {
            assert!(matches!(
                keymap.lookup(Mode::Normal, &sequence),
                Lookup::Exact(binding)
                    if binding.target == BindingTarget::Editor(command)
            ));
        }
    }

    #[test]
    fn terminal_insert_ctrl_w_registers_terminal_window_actions() {
        let keymap = default_keymap();
        for (suffix, command) in [
            ('v', EditorCommand::SplitVertical),
            ('s', EditorCommand::SplitHorizontal),
            ('f', EditorCommand::ToggleFullscreen),
            ('z', EditorCommand::ToggleZen),
        ] {
            let sequence = KeySequence::from([Key::ctrl('w'), Key::char(suffix)]);
            assert!(matches!(
                keymap.lookup_in(Mode::Insert, BindingScope::Terminal, &sequence),
                Lookup::Exact(binding)
                    if binding.target == BindingTarget::Editor(command)
            ));
            assert!(matches!(
                keymap.lookup(Mode::Insert, &sequence),
                Lookup::NoMatch
            ));
        }
    }

    #[test]
    fn default_keymap_has_no_duplicate_sequences_per_mode() {
        let keymap = default_keymap();
        assert!(!keymap.bindings().is_empty());
        assert!(Keymap::new(keymap.bindings().to_vec()).is_ok());
    }

    #[test]
    fn delimiter_text_objects_bind_opening_closing_quote_and_closest_keys() {
        let cases = [
            ('a', '(', EditorCommand::SelectAroundParentheses),
            ('a', ')', EditorCommand::SelectAroundParentheses),
            ('i', '[', EditorCommand::SelectInsideSquareBrackets),
            ('i', ']', EditorCommand::SelectInsideSquareBrackets),
            ('a', '{', EditorCommand::SelectAroundBraces),
            ('i', '}', EditorCommand::SelectInsideBraces),
            ('a', '<', EditorCommand::SelectAroundAngleBrackets),
            ('i', '>', EditorCommand::SelectInsideAngleBrackets),
            ('a', '"', EditorCommand::SelectAroundDoubleQuotes),
            ('i', '\'', EditorCommand::SelectInsideSingleQuotes),
            ('a', '`', EditorCommand::SelectAroundBackticks),
            ('i', 'm', EditorCommand::SelectInsideClosestDelimiter),
        ];
        for (part, delimiter, command) in cases {
            let sequence = KeySequence::from([
                Key::char(' '),
                Key::char('x'),
                Key::char(part),
                Key::char(delimiter),
            ]);
            assert!(matches!(
                default_keymap().lookup(Mode::Normal, &sequence),
                Lookup::Exact(binding)
                    if binding.target == BindingTarget::Editor(command)
            ));
        }
    }

    #[test]
    fn directory_bindings_shadow_global_sequences_only_in_directory_scope() {
        let sequence = KeySequence::from(Key::char('r'));
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &sequence),
            Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::ReplaceChar)
        ));
        assert!(matches!(
            default_keymap().lookup_in(Mode::Normal, BindingScope::Directory, &sequence),
            Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::ReplaceChar)
        ));
    }

    #[test]
    fn refresh_uses_space_r_in_explorer_and_git_status() {
        let refresh = KeySequence::from([Key::char(' '), Key::char('r')]);
        for scope in [BindingScope::Directory, BindingScope::GitStatus] {
            assert!(matches!(
                default_keymap().lookup_in(Mode::Normal, scope, &refresh),
                Lookup::Exact(binding)
                    if binding.target == BindingTarget::Colon(ColonCommand::Reload)
            ));
            assert!(matches!(
                default_keymap().lookup_in(Mode::Normal, scope, &KeySequence::from(Key::char('r'))),
                Lookup::Exact(binding)
                    if binding.target == BindingTarget::Editor(EditorCommand::ReplaceChar)
            ));
        }
    }

    #[test]
    fn e_is_the_word_end_motion_inside_directory_buffers() {
        let sequence = KeySequence::from(Key::char('e'));
        assert!(matches!(
            default_keymap().lookup_in(Mode::Normal, BindingScope::Directory, &sequence),
            Lookup::Exact(binding) if binding.target == BindingTarget::Editor(EditorCommand::MoveWordEnd)
        ));
    }

    #[test]
    fn question_mark_toggles_details_only_inside_directory_buffers() {
        let sequence = KeySequence::from(Key::char('?'));
        assert!(matches!(
            default_keymap().lookup_in(Mode::Normal, BindingScope::Directory, &sequence),
            Lookup::Exact(binding)
                if binding.target
                    == BindingTarget::Editor(EditorCommand::ToggleDirectoryDetails)
        ));
        assert!(matches!(
            default_keymap().lookup(Mode::Normal, &sequence),
            Lookup::NoMatch
        ));
    }

    #[test]
    fn directory_bindings_are_prioritized_for_contextual_help() {
        let commands = default_keymap()
            .bindings_for_scope(Mode::Normal, BindingScope::Directory)
            .take(4)
            .map(|binding| binding.target)
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            [
                BindingTarget::Editor(EditorCommand::OpenDirectoryEntry),
                BindingTarget::Editor(EditorCommand::OpenParentDirectory),
                BindingTarget::Editor(EditorCommand::OpenParentDirectory),
                BindingTarget::Editor(EditorCommand::ToggleHiddenFiles),
            ]
        );
    }

    #[test]
    fn dot_keeps_the_explorer_action_and_whitespace_stays_in_its_namespace() {
        let dot = KeySequence::from(Key::char('.'));
        let space_p_dot = KeySequence::from([Key::char(' '), Key::char('p'), Key::char('.')]);
        assert!(matches!(
            default_keymap().lookup_in(Mode::Normal, BindingScope::Directory, &dot),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::ToggleHiddenFiles)
        ));
        assert!(matches!(
            default_keymap().lookup(Mode::Select, &dot),
            Lookup::NoMatch
        ));
        assert!(matches!(
            default_keymap().lookup_in(Mode::Normal, BindingScope::Directory, &space_p_dot),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::ToggleWhitespace)
        ));
        assert!(matches!(
            default_keymap().lookup_in(Mode::Insert, BindingScope::Directory, &dot),
            Lookup::NoMatch
        ));
    }

    #[test]
    fn exact_and_prefix_can_coexist() {
        const NORMAL_MODE: &[Mode] = &[Mode::Normal];
        let keymap = Keymap::new(vec![
            Binding::implemented(NORMAL_MODE, Key::char('g'), EditorCommand::MoveFileStart),
            Binding::implemented(
                NORMAL_MODE,
                [Key::char('g'), Key::char('e')],
                EditorCommand::MoveFileEnd,
            ),
        ])
        .unwrap();

        assert!(matches!(
            keymap.lookup(Mode::Normal, &KeySequence::from(Key::char('g'))),
            Lookup::ExactAndPrefix { continuations, .. } if continuations.len() == 1
        ));
    }

    #[test]
    fn character_shift_is_canonicalized() {
        let shifted = Key::new(KeyCode::Char('G'), Modifiers::SHIFT);
        assert_eq!(
            KeySequence::from(shifted),
            KeySequence::from(Key::char('G'))
        );
    }
}
