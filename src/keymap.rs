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

pub mod actions;
pub mod admission;
pub mod configured;
mod effective;
pub mod mode_graph;
mod named;
pub mod native_preview;
pub mod native_window;
pub mod pointer;
pub mod pointer_actions;
pub mod presets;
pub mod program;
mod scopes;
pub(crate) use scopes::admit as admits_scoped_action;
pub mod terminal_input;
pub mod ui;
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
    Plugin(u64),
    Editor(EditorCommand),
    Colon(ColonCommand),
}

impl BindingTarget {
    pub const fn id(self) -> CommandId {
        match self {
            Self::Plugin(id) => CommandId::Plugin(id),
            Self::Editor(command) => CommandId::Editor(command),
            Self::Colon(command) => CommandId::Colon(command),
        }
    }

    pub fn description(self) -> &'static str {
        self.id()
            .descriptor()
            .map(|d| d.metadata.description)
            .unwrap_or("Run plugin command")
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Plugin(_) => "plugin",
            Self::Editor(command) => command.metadata().name,
            Self::Colon(command) => {
                crate::command::command_spec(CommandId::Colon(command))
                    .expect("every bindable colon identity belongs to the command inventory")
                    .name
            }
        }
    }

    pub fn invocation(self) -> Result<CommandInvocation, CommandInvocationError> {
        match self {
            Self::Plugin(id) => CommandInvocation::from_parts(
                CommandId::Plugin(id),
                crate::command::InvocationParameters::None,
                Default::default(),
            ),
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
    /// Runtime application scope, validated together with authored scopes.
    Plugin(usize),
    #[default]
    Global,
    /// A native image/PDF surface owns its pixel selection and view controls.
    Media,
    /// Read-only PDF page rows with ordinary text navigation.
    PdfPages,
    Directory,
    DirectoryTree,
    Settings,
    GitStatus,
    GitBranches,
    GitConflicts,
    GitConflictFile,
    GitConflictMarkdown,
    GitMergeReview,
    GitMergeDetail,
    GitMergeInput,
    GitWorktrees,
    GitComparison,
    GitRevisionDiff,
    GitLog,
    GitNetwork,
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
    /// The session manager overlay. Its chords are [`Mode::List`] bindings
    /// layered on the generic list keys, not on the editor's.
    SessionManager,
}

impl BindingScope {
    /// Exhaustive scope inventory used by registry invariants.
    pub const ALL: &'static [Self] = &[
        Self::Global,
        Self::Media,
        Self::PdfPages,
        Self::Directory,
        Self::DirectoryTree,
        Self::Settings,
        Self::GitStatus,
        Self::GitBranches,
        Self::GitConflicts,
        Self::GitConflictFile,
        Self::GitConflictMarkdown,
        Self::GitMergeReview,
        Self::GitMergeDetail,
        Self::GitMergeInput,
        Self::Terminal,
        Self::GitWorktrees,
        Self::GitComparison,
        Self::GitRevisionDiff,
        Self::GitLog,
        Self::GitNetwork,
        Self::GitBlame,
        Self::GitStash,
        Self::WorkspaceSearch,
        Self::Help,
        Self::CommitMessage,
        Self::Diff,
        Self::Markdown,
        Self::SessionManager,
    ];

    pub(crate) const fn is_merge_review(self) -> bool {
        matches!(
            self,
            Self::GitMergeReview | Self::GitMergeDetail | Self::GitMergeInput
        )
    }
    pub(crate) const fn owns_modal_input(self) -> bool {
        self.is_merge_review()
            || matches!(
                self,
                Self::DirectoryTree | Self::Media | Self::PdfPages | Self::Terminal
            )
    }

    /// Whether the scope belongs to a generated view rather than to a document
    /// someone opened.
    ///
    /// `Markdown` is not one: it covers every Markdown file as well as the page
    /// rendered from it, so counting it would claim an ordinary document as a
    /// view of the editor's own.
    pub const fn is_special_buffer_scope(self) -> bool {
        !matches!(
            self,
            Self::Global
                | Self::Terminal
                | Self::Markdown
                | Self::SessionManager
                | Self::DirectoryTree
                | Self::GitConflictFile
                | Self::GitConflictMarkdown
                | Self::GitMergeReview
                | Self::GitMergeDetail
                | Self::GitMergeInput
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    /// Owning named table. Scope and editing modes below are compatibility
    /// metadata for action admission and view presentation, not lookup layers.
    pub binding_mode: Option<std::sync::Arc<str>>,
    pub modes: &'static [Mode],
    pub scope: BindingScope,
    pub sequence: KeySequence,
    pub target: BindingTarget,
    /// Nonempty semantic program shared by built-in and configured bindings.
    pub program: program::Program,
    /// Assigned through keys.bind (distinct from remapping or scoped assignments).
    pub configured_actions: bool,
    /// Accepted configuration paths, relative to keys; empty for built-ins.
    pub source_paths: Vec<Vec<String>>,
    pub description: std::borrow::Cow<'static, str>,
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
    /// Preserve authored labels; unnamed programs expose their ordered steps.
    pub fn discovery_description(&self) -> std::borrow::Cow<'static, str> {
        if !self.description.is_empty() {
            self.description.clone()
        } else if self.is_plain_action() {
            self.target.description().into()
        } else {
            self.program
                .iter()
                .map(|a| a.target().name())
                .collect::<Vec<_>>()
                .join(" → ")
                .into()
        }
    }

    /// Capability needed by a configured action or the ordinary target.
    pub fn action_capability(&self) -> Option<CommandCapability> {
        self.program
            .iter()
            .find_map(|action| action.target().id().capability())
            .or_else(|| self.target.id().capability())
    }

    /// Whether this binding invokes exactly its target, without arguments.
    pub fn is_plain_action(&self) -> bool {
        match self.program.steps() {
            [program::Step::Action(actions::Action::Target(target))] => self.target == *target,
            [program::Step::Action(actions::Action::Editor(command))] => {
                self.target == BindingTarget::Editor(*command)
            }
            [program::Step::Action(actions::Action::Invocation(invocation))] => self
                .target
                .invocation()
                .as_ref()
                .is_ok_and(|default| default == invocation),
            _ => false,
        }
    }

    pub fn implemented(
        modes: &'static [Mode],
        sequence: impl Into<KeySequence>,
        target: impl Into<BindingTarget>,
    ) -> Self {
        let target = target.into();
        Self {
            binding_mode: None,
            modes,
            scope: BindingScope::Global,
            sequence: sequence.into(),
            target,
            program: program::Program::single(target),
            configured_actions: false,
            source_paths: Vec::new(),
            description: target.description().into(),
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
            binding_mode: None,
            modes,
            scope,
            sequence: sequence.into(),
            target,
            program: program::Program::single(target),
            configured_actions: false,
            source_paths: Vec::new(),
            description: target.description().into(),
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

    /// Custom editing bindings do not take input away from a terminal child,
    /// including suffixes of the editor-owned window prefix.
    pub fn visible_in(&self, mode: Mode, scope: BindingScope) -> bool {
        self.is_active_in(mode)
            && !(scope.is_merge_review() && self.scope == BindingScope::Global)
            && !(scope == BindingScope::Terminal
                && matches!(mode, Mode::Insert | Mode::Replace)
                && self.configured_actions)
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
            binding_mode: None,
            modes,
            scope: BindingScope::Global,
            sequence: sequence.into(),
            target,
            program: program::Program::single(target),
            configured_actions: false,
            source_paths: Vec::new(),
            description: target.description().into(),
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
            binding_mode: None,
            modes,
            scope: BindingScope::Global,
            sequence: sequence.into(),
            target,
            program: program::Program::single(target),
            configured_actions: false,
            source_paths: Vec::new(),
            description: target.description().into(),
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
pub struct EntryPoint<'a> {
    pub key: Key,
    /// What the key does, or what its namespace holds.
    pub description: &'a str,
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
    pointer: pointer_actions::Bindings,
    preview: native_preview::Cache,
    ui: ui::Bindings,
    overrides: effective::Overrides,
    terminal_input: terminal_input::Index,
    bindings: Vec<Binding>,
    lookup_index: Arc<HashMap<LookupContext, Arc<HashMap<KeySequence, IndexedLookup>>>>,
    fast_pane_keys: bool,
    namespaces: Vec<BindingNamespace>,
    context_actions: Vec<ContextAction>,
    leader: KeyStroke,
    window: KeyStroke,
    default_spellings: HashMap<KeySequence, KeySequence>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum LookupContext {
    Named(mode_graph::ModeId),
    Legacy(Mode, BindingScope),
}

#[derive(Clone, Debug, Default)]
struct IndexedLookup {
    exact: Option<usize>,
    continuations: Vec<usize>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(Vec::new()).expect("an empty keymap is valid")
    }
}

impl Keymap {
    pub fn pointer(&self) -> &pointer_actions::Bindings {
        &self.pointer
    }

    pub fn pane_gestures(&self) -> pointer::PaneGestures {
        self.pointer.pane_gestures()
    }

    pub fn ui(&self) -> &ui::Bindings {
        &self.ui
    }
    pub(crate) fn fast_pane_keys(&self) -> bool {
        self.fast_pane_keys
    }

    pub(crate) fn with_fast_pane_keys_enabled(mut self, enabled: bool) -> Self {
        self.fast_pane_keys = enabled;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_fixture_bindings(mut self, bindings: Vec<Binding>) -> Self {
        self.bindings = bindings;
        self.rebuild_lookup_index();
        self
    }

    #[cfg(test)]
    pub(crate) fn named_fixture(bindings: Vec<Binding>) -> Self {
        let mut map = default_keymap().clone();
        map.bindings = bindings
            .into_iter()
            .map(|mut binding| {
                let id = map
                    .named_mode_for(binding.modes[0], binding.scope)
                    .expect("fixture role");
                binding.binding_mode =
                    Some(map.mode_graph().unwrap().get(id).unwrap().name().into());
                binding
            })
            .collect();
        map.namespaces.clear();
        map.context_actions.clear();
        map.rebuild_lookup_index();
        map
    }

    #[cfg(test)]
    pub(crate) fn with_test_bindings(&self, bindings: Vec<Binding>) -> Self {
        let mut keymap = self.clone();
        keymap.bindings = bindings;
        keymap.overrides.1 = None;
        keymap.rebuild_lookup_index();
        keymap
    }

    /// Resolve indent-key descriptions from the live setting in the same
    /// registry used by dispatch, help, and key hints.
    pub(crate) fn with_indent_overrides(mut self, enabled: bool) -> Self {
        if enabled {
            for binding in &mut self.bindings {
                if binding.configured_actions {
                    continue;
                }
                binding.description = match binding.target {
                    BindingTarget::Editor(EditorCommand::InsertTab) => {
                        "Insert this buffer's configured indentation".into()
                    }
                    BindingTarget::Editor(EditorCommand::InsertLiteralTab) => {
                        "Insert the other indentation style for this buffer".into()
                    }
                    _ => continue,
                };
            }
        }
        self
    }

    pub(crate) fn with_indent_style(&self, style: crate::config::IndentStyle) -> Self {
        let mut keymap = self.clone();
        for binding in &mut keymap.bindings {
            if binding.configured_actions {
                continue;
            }
            binding.description = match (binding.target, style) {
                (
                    BindingTarget::Editor(EditorCommand::InsertTab),
                    crate::config::IndentStyle::Spaces,
                ) => "Insert spaces to the next tab stop (indent: spaces)".into(),
                (
                    BindingTarget::Editor(EditorCommand::InsertTab),
                    crate::config::IndentStyle::Tabs,
                ) => "Insert a tab (indent: tabs)".into(),
                (
                    BindingTarget::Editor(EditorCommand::InsertLiteralTab),
                    crate::config::IndentStyle::Spaces,
                ) => "Insert a tab (indent: spaces)".into(),
                (
                    BindingTarget::Editor(EditorCommand::InsertLiteralTab),
                    crate::config::IndentStyle::Tabs,
                ) => "Insert spaces to the next tab stop (indent: tabs)".into(),
                _ => continue,
            };
        }
        keymap
    }

    pub(crate) fn with_plugin_bindings(&self, bindings: Vec<Binding>) -> anyhow::Result<Self> {
        for binding in &bindings {
            anyhow::ensure!(
                binding
                    .sequence
                    .as_slice()
                    .iter()
                    .enumerate()
                    .all(|(index, key)| {
                        !(index == 0
                            && key.modifiers.is_empty()
                            && matches!(key.code, KeyCode::Char('1'..='9')))
                            && !(index > 0
                                && binding
                                    .modes
                                    .iter()
                                    .any(|mode| matches!(mode, Mode::Normal | Mode::Select))
                                && self.ui().lookup(ui::Owner::GrammarPrefix, *key).is_some())
                    }),
                "plugin binding {} uses a grammar-reserved count or cancellation key",
                binding.sequence
            );
        }
        let mut candidate = self.clone();
        candidate
            .bindings
            .retain(|b| !matches!(b.target, BindingTarget::Plugin(_)));
        let bindings = named::plugin_bindings(&mut candidate, bindings)?;
        candidate.bindings.extend(bindings);
        let violations = validate::with_input_policy(
            &candidate.bindings,
            &candidate.namespaces,
            &candidate.context_actions,
            &candidate.overrides,
            Some(candidate.ui()),
        );
        anyhow::ensure!(
            violations.is_empty(),
            "plugin binding collision: {}",
            violations
                .first()
                .map(|v| v.message.as_str())
                .unwrap_or_default()
        );
        candidate.rebuild_lookup_index();
        Ok(candidate)
    }

    pub fn new(bindings: Vec<Binding>) -> Result<Self, DuplicateBinding> {
        Self::new_with_overrides(bindings, effective::Overrides::default())
    }

    fn new_with_overrides(
        bindings: Vec<Binding>,
        overrides: effective::Overrides,
    ) -> Result<Self, DuplicateBinding> {
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
            pointer: pointer_actions::Bindings::default(),
            preview: native_preview::Cache::default(),
            ui: ui::Bindings::default(),
            overrides,
            terminal_input: terminal_input::Index::default(),
            bindings,
            lookup_index: Arc::new(HashMap::new()),
            fast_pane_keys: false,
            namespaces: Vec::new(),
            context_actions: Vec::new(),
            leader: Key::char(' '),
            window: Key::ctrl('w'),
            default_spellings: HashMap::new(),
        };
        keymap.record_identity_spellings();
        keymap.rebuild_lookup_index();
        Ok(keymap)
    }

    fn rebuild_lookup_index(&mut self) {
        self.preview = native_preview::Cache::default();
        if let Some(tables) = &self.overrides.1 {
            let mut index = HashMap::new();
            let mut shared = HashMap::new();
            for (mode, effective) in tables.all_indices(&self.bindings) {
                if let Some(sequences) = shared.get(&effective) {
                    index.insert(LookupContext::Named(mode), Arc::clone(sequences));
                    continue;
                }
                let mut sequences: HashMap<KeySequence, IndexedLookup> = HashMap::new();
                for &binding_index in &effective {
                    let binding = &self.bindings[binding_index];
                    for prefix_len in 0..binding.sequence.len() {
                        sequences
                            .entry(KeySequence(
                                binding.sequence.as_slice()[..prefix_len].to_vec(),
                            ))
                            .or_default()
                            .continuations
                            .push(binding_index);
                    }
                    sequences.entry(binding.sequence.clone()).or_default().exact =
                        Some(binding_index);
                }
                let sequences = Arc::new(sequences);
                shared.insert(effective, Arc::clone(&sequences));
                index.insert(LookupContext::Named(mode), sequences);
            }
            self.lookup_index = Arc::new(index);
            self.terminal_input = terminal_input::Index::build(self);
            return;
        }
        // Keep positions into the binding registry so cloned maps can update
        // descriptions without leaving lookup, help, and hints out of sync.
        let mut scopes = BindingScope::ALL.to_vec();
        for binding in &self.bindings {
            if !scopes.contains(&binding.scope) {
                scopes.push(binding.scope);
            }
        }

        let mut index = HashMap::new();
        for mode in [
            Mode::Normal,
            Mode::Insert,
            Mode::Replace,
            Mode::Select,
            Mode::Command,
            Mode::List,
        ] {
            for &scope in &scopes {
                if self.overrides.1.is_none()
                    && scope != BindingScope::Global
                    && !self.bindings.iter().any(|binding| {
                        scope_includes(scope, binding.scope) && binding.is_active_in(mode)
                    })
                    && self.overrides.governing(mode, scope).is_empty()
                {
                    continue;
                }
                let effective = effective::indices(&self.bindings, &self.overrides, mode, scope);

                let mut sequences: HashMap<KeySequence, IndexedLookup> = HashMap::new();
                for binding_index in effective {
                    let binding = &self.bindings[binding_index];
                    for prefix_len in 0..binding.sequence.len() {
                        sequences
                            .entry(KeySequence(
                                binding.sequence.as_slice()[..prefix_len].to_vec(),
                            ))
                            .or_default()
                            .continuations
                            .push(binding_index);
                    }
                    sequences.entry(binding.sequence.clone()).or_default().exact =
                        Some(binding_index);
                }
                index.insert(LookupContext::Legacy(mode, scope), Arc::new(sequences));
            }
        }
        self.lookup_index = Arc::new(index);
        self.terminal_input = terminal_input::Index::build(self);
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
        self.terminal_input = terminal_input::Index::build(&self);
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
            .filter(move |action| scope_includes(scope, action.scope))
    }

    pub fn all_context_actions(&self) -> &[ContextAction] {
        &self.context_actions
    }

    pub fn bindings_for_mode(&self, mode: Mode) -> impl Iterator<Item = &Binding> {
        self.bindings_for_scope(mode, BindingScope::Global)
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
                    && binding.is_plain_action()
                    && matches!(binding.availability, BindingAvailability::Implemented)
            })
            .map(|binding| &binding.sequence)
    }

    pub fn bindings_for_scope(
        &self,
        mode: Mode,
        scope: BindingScope,
    ) -> impl Iterator<Item = &Binding> {
        effective::indices(&self.bindings, &self.overrides, mode, scope)
            .into_iter()
            .map(|index| &self.bindings[index])
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
            .flat_map(|scoped| {
                self.bindings
                    .iter()
                    .filter(move |binding| {
                        binding.visible_in(mode, scope)
                            && binding.scope == BindingScope::Global
                            && (binding.sequence == scoped.sequence
                                || (scope == BindingScope::DirectoryTree
                                    && binding.sequence.starts_with(&scoped.sequence)))
                    })
                    .map(move |global| (scoped, global))
            })
            .collect()
    }

    /// Every first key that starts a binding in this context.
    ///
    /// Scope-specific keys come first, then the global ones, each group in
    /// key order.
    pub fn entry_points(&self, mode: Mode, scope: BindingScope) -> Vec<EntryPoint<'_>> {
        let mut entries: Vec<EntryPoint<'_>> = Vec::new();
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
                    entry.description = &binding.description;
                } else {
                    entry.prefix = true;
                }
                entry.scoped &= scoped;
            } else {
                entries.push(EntryPoint {
                    key,
                    description: if leaf { &binding.description } else { "" },
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
                && ((!scope.is_merge_review() && namespace.scope == BindingScope::Global)
                    || scope_includes(scope, namespace.scope))
                && self.has_prefix_in(mode, scope, &namespace.sequence)
        })
    }

    /// What one key does in an open list with `scope`.
    ///
    /// A list reads one key at a time: it has no prefixes, counts, or
    /// operands, so it needs no input grammar between the key and the
    /// registry.
    pub fn list_binding(&self, scope: BindingScope, key: Key) -> Option<&Binding> {
        match self.lookup_in(Mode::List, scope, &KeySequence::from(key)) {
            Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => Some(binding),
            Lookup::Prefix(_) | Lookup::NoMatch => None,
        }
    }

    pub(crate) fn list_key_is_configured(&self, scope: BindingScope, key: Key) -> bool {
        self.overrides
            .affects(Mode::List, scope, &KeySequence::from(key), false)
    }

    pub fn lookup(&self, mode: Mode, sequence: &KeySequence) -> Lookup<'_> {
        self.lookup_in(mode, BindingScope::Global, sequence)
    }

    /// Clipboard copy bindings visible in Terminal Insert, including
    /// configured replacements. Ordinary Ctrl-c remains child input.
    pub fn terminal_clipboard_key(&self, key: Key) -> bool {
        let copy = |binding: &Binding| {
            !binding.configured_actions
                && binding.target == BindingTarget::Editor(EditorCommand::ClipboardYank)
        };
        match self.lookup_in(
            Mode::Insert,
            BindingScope::Terminal,
            &KeySequence::from(key),
        ) {
            Lookup::Exact(binding) => copy(binding),
            Lookup::Prefix(bindings) => bindings.iter().any(|binding| copy(binding)),
            Lookup::ExactAndPrefix {
                exact,
                continuations,
            } => copy(exact) || continuations.iter().any(|binding| copy(binding)),
            Lookup::NoMatch => false,
        }
    }

    pub fn mode_graph(&self) -> Option<&mode_graph::ModeGraph> {
        self.overrides.1.as_ref().map(|tables| &tables.graph)
    }

    pub fn named_mode_for(&self, mode: Mode, scope: BindingScope) -> Option<mode_graph::ModeId> {
        self.overrides.1.as_ref()?.mode_for(mode, scope)
    }

    pub fn bindings_for_named_mode(
        &self,
        mode: mode_graph::ModeId,
    ) -> impl Iterator<Item = &Binding> {
        self.overrides
            .1
            .as_ref()
            .map(|tables| tables.indices(&self.bindings, mode))
            .unwrap_or_default()
            .into_iter()
            .map(|index| &self.bindings[index])
    }

    pub fn lookup_named(&self, mode: mode_graph::ModeId, sequence: &KeySequence) -> Lookup<'_> {
        self.lookup_entry(
            self.lookup_index
                .get(&LookupContext::Named(mode))
                .and_then(|entries| entries.get(sequence)),
        )
    }

    fn has_prefix_in(&self, mode: Mode, scope: BindingScope, sequence: &KeySequence) -> bool {
        matches!(self.lookup_in(mode, scope, sequence), Lookup::Prefix(_))
    }

    pub fn lookup_in(&self, mode: Mode, scope: BindingScope, sequence: &KeySequence) -> Lookup<'_> {
        if self.overrides.1.is_some() {
            return self
                .named_mode_for(mode, scope)
                .map(|id| self.lookup_named(id, sequence))
                .unwrap_or(Lookup::NoMatch);
        }
        // Scopes without bindings in this mode inherit the global index.
        let entry = self
            .lookup_index
            .get(&LookupContext::Legacy(mode, scope))
            .or_else(|| {
                (!scope.is_merge_review())
                    .then(|| {
                        self.lookup_index
                            .get(&LookupContext::Legacy(mode, BindingScope::Global))
                    })
                    .flatten()
            })
            .and_then(|sequences| sequences.get(sequence));
        self.lookup_entry(entry)
    }

    fn lookup_entry(&self, entry: Option<&IndexedLookup>) -> Lookup<'_> {
        let Some(entry) = entry else {
            return Lookup::NoMatch;
        };
        let exact = entry.exact.map(|index| &self.bindings[index]);
        let continuations = entry
            .continuations
            .iter()
            .map(|&index| &self.bindings[index])
            .collect::<Vec<_>>();
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

/// Whether a keystroke leaves Insert mode instead of reaching a terminal.
///
/// `Escape` cannot be it: `vim` and `htop` inside the pane need it, and every
/// agent uses it too. `Ctrl-\\` begins the staged Normal/review transition;
/// `Ctrl-w` begins the Insert-mode pane-navigation namespace instead.
pub fn is_terminal_normal_key(key: Key) -> bool {
    // Two spellings of Ctrl-\\. A terminal implementing the kitty keyboard
    // protocol reports `Ctrl-\` as the character it is; a legacy one has only
    // the control byte `0x1c`, which Crossterm decodes as `Ctrl-4` because
    // that is what the historical table says. Runyte asks for the enhanced
    // protocol but cannot require it — it is off on macOS entirely — so the
    // escape hatch has to answer to both or it would be unreachable on the
    // terminals that need it most.
    key.modifiers.contains(Modifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('\\') | KeyCode::Char('4'))
}

/// Whether `key` is one of the single-key pane moves.
///
/// Says nothing about whether the option is on: that is the caller's
/// configuration to read, not the keymap's.
pub fn is_fast_pane_key(key: Key) -> bool {
    let key = key.canonical_for_binding();
    ['h', 'j', 'k', 'l']
        .into_iter()
        .any(|character| key == Key::ctrl(character))
}

fn scope_includes(active: BindingScope, binding: BindingScope) -> bool {
    active == binding
        || (active == BindingScope::GitConflictMarkdown
            && matches!(
                binding,
                BindingScope::Markdown | BindingScope::GitConflictFile
            ))
}

static DEFAULT_KEYMAP: LazyLock<Arc<Keymap>> = LazyLock::new(|| Arc::new(presets::load(false)));
static FAST_PANE_KEYMAP: LazyLock<Arc<Keymap>> = LazyLock::new(|| Arc::new(presets::load(true)));

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

    fn effective_lookup<'a>(bindings: &[&'a Binding], sequence: &KeySequence) -> Lookup<'a> {
        let mut exact = None;
        let mut continuations = Vec::new();
        for &binding in bindings {
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

    fn assert_index_matches_effective_bindings(keymap: &Keymap, extra_scopes: &[BindingScope]) {
        let mut scopes = BindingScope::ALL.to_vec();
        scopes.extend_from_slice(extra_scopes);
        let mut sequences = vec![KeySequence::default(), KeySequence::from(Key::char('!'))];
        for binding in keymap.bindings() {
            for len in 1..=binding.sequence.len() {
                sequences.push(KeySequence(binding.sequence.as_slice()[..len].to_vec()));
            }
        }
        let mut seen = std::collections::HashSet::new();
        sequences.retain(|sequence| seen.insert(sequence.clone()));
        for mode in [
            Mode::Normal,
            Mode::Insert,
            Mode::Replace,
            Mode::Select,
            Mode::Command,
            Mode::List,
        ] {
            for &scope in &scopes {
                let effective = keymap.bindings_for_scope(mode, scope).collect::<Vec<_>>();
                for sequence in &sequences {
                    assert_eq!(
                        keymap.lookup_in(mode, scope, sequence),
                        effective_lookup(&effective, sequence),
                        "{mode:?} {scope:?} {sequence}"
                    );
                }
            }
        }
    }

    #[test]
    fn indexed_lookup_matches_effective_bindings_across_registry_changes() {
        assert_index_matches_effective_bindings(default_keymap(), &[]);
        assert_index_matches_effective_bindings(&keymap_for(true), &[]);

        let section: serde_yaml::Value = serde_yaml::from_str(
            "leader: Ctrl-x\nrebind:\n  Space g: Leader G\n  Space e: Space\n",
        )
        .unwrap();
        let compiled = configured::compile(&section, default_keymap());
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        assert_index_matches_effective_bindings(&compiled.keymap, &[]);

        let invalid: serde_yaml::Value =
            serde_yaml::from_str("rebind:\n  Space e: Space\n  Space g: Space\n").unwrap();
        let rolled_back = configured::compile(&invalid, default_keymap());
        assert!(!rolled_back.errors.is_empty());
        assert_index_matches_effective_bindings(&rolled_back.keymap, &[]);

        let plugin = |sequence: &str, id, scope| {
            Binding::implemented_in(
                MODAL,
                scope,
                KeySequence::parse(sequence).unwrap(),
                BindingTarget::Plugin(id),
            )
        };
        let scoped = BindingScope::Plugin(7);
        let first = default_keymap()
            .with_plugin_bindings(vec![
                plugin("Space = a", 1, BindingScope::Global),
                plugin("F12", 2, scoped),
            ])
            .unwrap();
        assert_index_matches_effective_bindings(&first, &[scoped, BindingScope::Plugin(8)]);
        let refreshed = first
            .with_plugin_bindings(vec![plugin("F11", 4, scoped)])
            .unwrap();
        assert!(
            matches!(refreshed.lookup_in(Mode::Normal, scoped, &KeySequence::parse("F11").unwrap()), Lookup::Exact(binding) if binding.target == BindingTarget::Plugin(4))
        );
        assert!(matches!(
            refreshed.lookup_in(
                Mode::Normal,
                BindingScope::Plugin(8),
                &KeySequence::parse("F11").unwrap()
            ),
            Lookup::NoMatch
        ));
        assert!(matches!(
            refreshed.lookup_in(Mode::Normal, scoped, &KeySequence::parse("F12").unwrap()),
            Lookup::NoMatch
        ));
        let replaced = first
            .with_plugin_bindings(vec![plugin("Space = b", 3, BindingScope::Global)])
            .unwrap();
        assert_index_matches_effective_bindings(&replaced, &[scoped]);
        assert!(matches!(
            replaced.lookup(Mode::Normal, &KeySequence::parse("Space = a").unwrap()),
            Lookup::NoMatch
        ));
    }

    #[test]
    fn indexed_lookup_preserves_ambiguity_role_order_and_scoped_shadowing() {
        let bindings = vec![
            Binding::implemented(MODAL, [Key::char('a')], EditorCommand::MoveFileStart)
                .with_role(BindingRole::Fast),
            Binding::implemented(
                MODAL,
                [Key::char('a'), Key::char('b')],
                EditorCommand::MoveFileEnd,
            )
            .with_role(BindingRole::Compatibility),
            Binding::implemented_in(
                MODAL,
                BindingScope::Markdown,
                [Key::char('a')],
                EditorCommand::ToggleMarkdownRender,
            ),
            Binding::implemented_in(
                MODAL,
                BindingScope::Markdown,
                [Key::char('a'), Key::char('c')],
                EditorCommand::MoveFileStart,
            ),
        ];
        let keymap = Keymap::new(bindings).unwrap();
        assert_index_matches_effective_bindings(&keymap, &[]);
        let Lookup::ExactAndPrefix {
            exact,
            continuations,
        } = keymap.lookup_in(
            Mode::Normal,
            BindingScope::Markdown,
            &KeySequence::from(Key::char('a')),
        )
        else {
            panic!("expected a scoped exact binding and two continuations");
        };
        assert_eq!(exact.scope, BindingScope::Markdown);
        assert_eq!(continuations[0].scope, BindingScope::Markdown);
        assert_eq!(continuations[1].scope, BindingScope::Global);
    }

    #[test]
    fn indent_command_names_are_not_key_config_rebinding_aliases() {
        let section: serde_yaml::Value = serde_yaml::from_str(
            "rebind:\n  insert-tab: F12\n  insert-literal-tab: F11\n  insert-indent: F10\n",
        )
        .unwrap();
        let compiled = configured::compile(&section, default_keymap());
        assert_eq!(compiled.errors.len(), 3);
        assert!(
            compiled
                .errors
                .iter()
                .all(|error| error.contains("rebind") && error.contains("rejected"))
        );
        assert!(matches!(
            compiled
                .keymap
                .lookup(Mode::Insert, &KeySequence::from(Key::plain(KeyCode::Tab))),
            Lookup::Exact(binding)
                if binding.target == BindingTarget::Editor(EditorCommand::InsertTab)
        ));
    }

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
                                binding.description.clone(),
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
        let guide = crate::manual::USER_GUIDE;
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
            "docs/user-guide/ Editing table is missing direct modal bindings: {}",
            missing.join(", ")
        );

        let missing = missing_from(Mode::Insert, insert);
        assert!(
            missing.is_empty(),
            "docs/user-guide/ Insert and Replace modes section is missing direct bindings: {}",
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

    /// Buffer scopes may add behavior but never replace a global key. The
    /// sidebar is a separate focus surface and owns navigation outright.
    #[test]
    fn document_mode_overrides_are_limited_to_directory_paste() {
        for (keymap_name, keymap) in built_in_keymaps() {
            for mode in [Mode::Normal, Mode::Select] {
                for &scope in BindingScope::ALL {
                    if scope.owns_modal_input() {
                        continue;
                    }
                    let overrides = keymap.shadowed_bindings(mode, scope);
                    if scope == BindingScope::Directory {
                        assert_eq!(overrides.len(), 1);
                        let (directory, inherited) = &overrides[0];
                        assert_eq!(directory.sequence.to_string(), "p");
                        assert_eq!(
                            directory.target,
                            BindingTarget::Editor(EditorCommand::DirectoryPaste)
                        );
                        assert_eq!(
                            inherited.target,
                            BindingTarget::Editor(EditorCommand::PasteAfterOrReplace)
                        );
                    } else {
                        assert!(overrides.is_empty(), "{keymap_name} {scope:?} {mode:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn tree_shadowed_bindings_include_global_continuations() {
        let keymap = default_keymap();
        let hidden = keymap.shadowed_bindings(Mode::Normal, BindingScope::DirectoryTree);
        assert!(hidden.iter().any(|(scoped, global)| {
            scoped.sequence.to_string() == "m" && global.sequence.to_string() == "m i w"
        }));
        let effective = keymap
            .bindings_for_scope(Mode::Normal, BindingScope::DirectoryTree)
            .map(|binding| binding.sequence.to_string())
            .collect::<Vec<_>>();
        assert!(!effective.contains(&"m i w".to_owned()));
        assert!(
            keymap
                .bindings_for_scope(Mode::Normal, BindingScope::Global)
                .any(|binding| binding.sequence.to_string() == "m i w")
        );
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

    /// A list's filter owns every key that types something, and a list is not
    /// a buffer: it answers its own keys and none of the editor's.
    #[test]
    fn list_keys_leave_typing_to_the_filter_and_editor_keys_to_the_editor() {
        for (keymap_name, keymap) in built_in_keymaps() {
            for binding in keymap
                .bindings()
                .iter()
                .filter(|b| b.is_active_in(Mode::List))
            {
                assert_eq!(binding.modes, [Mode::List], "{keymap_name} {binding:?}");
                assert_eq!(binding.sequence.len(), 1, "{keymap_name} {binding:?}");
                let key = binding.sequence.as_slice()[0];
                assert!(
                    !matches!(key.code, KeyCode::Char(_))
                        || key.modifiers.contains(Modifiers::CONTROL),
                    "{keymap_name} list binding {} would take filter text",
                    key.label()
                );
            }
            // Pane movement and saving are editor keys, not list keys.
            assert!(
                keymap
                    .list_binding(BindingScope::Global, Key::ctrl('w'))
                    .is_none()
            );
            assert!(
                keymap
                    .list_binding(BindingScope::Global, Key::ctrl('s'))
                    .is_none()
            );
            assert!(
                keymap
                    .list_binding(BindingScope::Global, Key::char('j'))
                    .is_none()
            );
        }

        let keymap = default_keymap();
        let target = |scope, key| keymap.list_binding(scope, key).map(|b| b.target);
        assert_eq!(
            target(BindingScope::Global, Key::ctrl('n')),
            Some(EditorCommand::ListNext.into())
        );
        // The manager's chords are its own, and it still has the list's.
        assert_eq!(target(BindingScope::Global, Key::ctrl('o')), None);
        assert_eq!(
            target(BindingScope::SessionManager, Key::ctrl('o')),
            Some(EditorCommand::OpenSessionDirectory.into())
        );
        assert_eq!(
            target(BindingScope::SessionManager, Key::ctrl('g')),
            Some(ColonCommand::GitWorktrees.into())
        );
        assert_eq!(
            target(BindingScope::SessionManager, Key::ctrl('t')),
            Some(EditorCommand::ListTogglePreview.into())
        );
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
            special, 18,
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
                ("C".to_owned(), "git-merge-continue", ActionContext::Buffer),
                ("A".to_owned(), "git-merge-abort", ActionContext::Buffer),
                ("d".to_owned(), "git-diff-side-by-side", ActionContext::Row),
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
            BindingScope::GitConflicts,
            BindingScope::GitConflictFile,
            BindingScope::GitConflictMarkdown,
            BindingScope::GitWorktrees,
            BindingScope::GitStash,
            BindingScope::GitNetwork,
            BindingScope::Diff,
            BindingScope::Directory,
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
                ("m".to_owned(), "merge"),
                ("f".to_owned(), "fetch"),
                ("d".to_owned(), "compare"),
                ("n".to_owned(), "create"),
                ("w".to_owned(), "worktree"),
                ("D".to_owned(), "delete"),
                ("p".to_owned(), "pull"),
                ("P".to_owned(), "push"),
            ]
        );
        assert_eq!(
            named(BindingScope::GitNetwork),
            vec![
                ("a".to_owned(), "all"),
                ("h".to_owned(), "head"),
                ("r".to_owned(), "ref"),
            ]
        );
        assert_eq!(
            named(BindingScope::GitWorktrees),
            vec![
                ("d".to_owned(), "compare"),
                ("n".to_owned(), "branch"),
                ("D".to_owned(), "remove"),
            ]
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
    fn help_prefix_and_removed_short_split_keep_their_namespaces() {
        let keymap = default_keymap();
        assert!(matches!(
            keymap.lookup(Mode::Normal, &KeySequence::parse("Space v").unwrap()),
            Lookup::NoMatch
        ));
        assert!(matches!(
            keymap.lookup(Mode::Normal, &KeySequence::parse("Space h").unwrap()),
            Lookup::Prefix(_)
        ));
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
            let sequence =
                KeySequence::from([Key::char('m'), Key::char(part), Key::char(delimiter)]);
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
