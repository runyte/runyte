// SPDX-License-Identifier: MPL-2.0

//! Descriptive action contracts shared by configuration and discovery.
//!
//! Stage 1 preserves the executor: effects describe possible work, not a new
//! scheduling rule. In particular, a bundled action can still change modes.
//! Scoped admission is independent of global binding modes until stage 4.

mod editor;
mod views;

use super::{
    COMMANDS, ColonCommand, CommandCategory, CommandContext, CommandId, CommandMetadata,
    CommandSpec, EditorCommand, Mode,
};
use crate::keymap::{BindingScope, BindingTarget, admission};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionEffect {
    Selection,
    Text,
    Mode,
    View,
    Pane,
    File,
    Operand,
    Prompt,
    Background,
    Register,
    SearchState,
    Configuration,
    Macro,
    History,
    TerminalInput,
    Application,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountBehavior {
    /// No semantic count; invocation validation still accepts an identity count of one.
    Reject,
    /// Repeat the operation (possibly combined into one transaction).
    Repeat,
    /// Interpret an address or a selection extent; not repeated invocation.
    Interpret,
}

impl CountBehavior {
    pub const fn accepts(self) -> bool {
        !matches!(self, Self::Reject)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperandArity {
    Zero,
    One,
    Two,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferMutation {
    Unchanged,
    /// May reach a text transaction, including after an operand, prompt or
    /// asynchronous result. This is not a writable-buffer admission check.
    MayMutate,
}

/// Temporary compatibility policy, removed in stage 3 when the executor and
/// validator switch to the effect-derived program continuation rule together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacySequencePosition {
    Continue,
    Last,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionPolicy {
    /// Only `keys.bind` admission. Empty for scope-only/internal actions.
    /// Never combine this with the views as a Cartesian admission table.
    pub global_binding_modes: &'static [Mode],
    /// Possible semantic effects across supported views, including bundled
    /// steps and owned continuations, but not unrelated later picker actions
    /// or ubiquitous normalization/redraw bookkeeping. Not a scheduling rule;
    /// view descriptions explain differences between implementations. Generic
    /// dispatchers such as ListAccept describe only dispatch itself: callers
    /// must consult the selected action for its effects.
    pub effects: &'static [ActionEffect],
    /// Semantic count support; the physical input grammar has its own policy.
    pub count: CountBehavior,
    /// Interactive character operands, independent of palette arguments.
    pub operands: OperandArity,
    pub buffer_mutation: BufferMutation,
    /// The existing read-only text-buffer gate, after specialized interception.
    pub requires_writable_buffer: bool,
    pub legacy_sequence_position: LegacySequencePosition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewBehavior {
    pub scope: BindingScope,
    pub description: &'static str,
}

/// What an accepted palette argument supplies today. Character operands are
/// deliberately not supplied this way until the argument stage of the plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentDelivery {
    Invocation,
    ReplacesPrompt,
}

#[derive(Clone, Copy, Debug)]
pub struct ActionArgument {
    pub syntax: super::CommandArguments,
    pub usage: &'static str,
    pub delivery: ArgumentDelivery,
}

#[derive(Clone, Copy, Debug)]
pub struct ActionDescriptor {
    pub id: CommandId,
    /// Canonical action name, prose and capability (not palette aliases).
    pub metadata: CommandMetadata,
    pub category: CommandCategory,
    pub policy: ActionPolicy,
    /// The existing palette spelling, aliases, argument format and requirement.
    /// `usage` records formats more precise than Path/FreeText. Presence of
    /// interactive operands never grants a new palette argument form.
    pub palette: Option<&'static CommandSpec>,
    /// Palette discovery context only; keys still reach their workflow guards.
    pub palette_context: Option<CommandContext>,
    /// Overrides to ordinary action prose for specialized view implementations.
    pub view_overrides: &'static [ViewBehavior],
}

impl ActionDescriptor {
    pub fn argument(self) -> Option<ActionArgument> {
        let spec = self.palette?;
        spec.arguments
            .accepts_arguments()
            .then_some(ActionArgument {
                syntax: spec.arguments,
                usage: spec.usage,
                delivery: if matches!(
                    self.id,
                    CommandId::Editor(
                        EditorCommand::OpenPathFilePicker | EditorCommand::RenameTerminal
                    )
                ) {
                    ArgumentDelivery::ReplacesPrompt
                } else {
                    ArgumentDelivery::Invocation
                },
            })
    }

    /// Description in an admitted view, using the existing scope gate. This
    /// does not grant a binding in any mode or bypass runtime context checks.
    pub fn view(self, scope: BindingScope) -> Option<ViewBehavior> {
        let target = match self.id {
            CommandId::Editor(c) => BindingTarget::Editor(c),
            CommandId::Colon(c) => BindingTarget::Colon(c),
            CommandId::Plugin(_) => return None,
        };
        admission::admits(scope, target).then(|| {
            self.view_overrides
                .iter()
                .copied()
                .find(|v| v.scope == scope)
                .unwrap_or(ViewBehavior {
                    scope,
                    description: self.metadata.description,
                })
        })
    }
}

impl EditorCommand {
    pub const fn action_policy(self) -> ActionPolicy {
        editor::editor_policy(self)
    }

    pub fn descriptor(self) -> ActionDescriptor {
        let id = CommandId::Editor(self);
        ActionDescriptor {
            id,
            metadata: self.metadata(),
            category: self.category(),
            policy: self.action_policy(),
            palette: palette(id),
            palette_context: id.context(),
            view_overrides: views::editor_views(self),
        }
    }
}

impl ColonCommand {
    pub fn descriptor(self) -> ActionDescriptor {
        let id = CommandId::Colon(self);
        let spec = palette(id).expect("every colon action has a palette specification");
        ActionDescriptor {
            id,
            metadata: CommandMetadata {
                name: spec.name,
                description: spec.description,
                capability: id.capability(),
            },
            category: self.category(),
            policy: self.action_policy(),
            palette: Some(spec),
            palette_context: id.context(),
            view_overrides: &[],
        }
    }

    pub const fn action_policy(self) -> ActionPolicy {
        use ActionEffect::*;
        use ColonCommand as C;
        let effects: &[ActionEffect] = match self {
            C::ChangeDirectory => &[File, Configuration, Prompt, Background, View, Application],
            C::InitializeWorkspace => &[File, Configuration, Background, View, Application],
            C::CloseBuffer | C::ForceCloseBuffer => &[View],
            C::DescribeKey => &[Prompt, View, Mode],
            C::ActionBrowser => &[View, Mode],
            C::CopyBinding => &[Register],
            C::DiffDisk => &[File, View],
            C::DiffRemote => &[Background, View],
            C::DiffOff | C::DiffThis => &[View],
            C::Format => &[Background, Text],
            C::GitCompare
            | C::GitDiff
            | C::GitDiffSideBySide
            | C::GitIndex
            | C::GitRefresh
            | C::GitSearchCommits => &[Background, View],
            C::GitBranches
            | C::GitBlame
            | C::GitBlameFile
            | C::GitLog
            | C::GitNetwork
            | C::GitNetworkHead
            | C::GitNetworkAll
            | C::GitNetworkRef
            | C::GitStatus
            | C::GitStashes
            | C::GitWorktrees => &[Background, View, Mode],
            C::GitNetworkChooseRef => &[View],
            C::GitCommit => &[Prompt, Background, File, View, Mode],
            C::GitDiscard
            | C::GitStashTracked
            | C::GitStashAll
            | C::GitStashUntracked
            | C::GitStashApply
            | C::GitStashDrop => &[Prompt, Background, File],
            C::GitStage
            | C::GitStageHunk
            | C::GitUnstageHunk
            | C::GitStageLines
            | C::GitUnstage => &[Background, File],
            C::GitCancel | C::PipeCancel | C::PluginStop | C::PluginRestart | C::LspRestart => {
                &[Background]
            }
            C::ConfigReload => &[Configuration, Background],
            C::Grammar => &[Configuration, Mode, Selection],
            C::WriteConfig | C::ForceWriteConfig => &[File],
            C::AgentChannel | C::LspStatus => &[Background, View],
            C::Notifications => &[View, Mode],
            C::LogOpen | C::ServiceHealth | C::Plugins | C::Path => &[View],
            C::LspTrust | C::ContextAccess => &[Prompt, Configuration, Background],
            C::Open => &[File, View, Prompt, Background],
            C::Quit | C::ForceQuit => &[Pane, View, Application],
            C::Detach
            | C::QuitAll
            | C::ForceQuitAll
            | C::TerminateQuitAll
            | C::QuitHere
            | C::ForceQuitHere => &[Application],
            C::Reload => &[Prompt, Background, File, Text],
            C::Resize => &[Pane, Prompt, Mode],
            C::Pipe => &[Background, Text],
            C::WriteQuit => &[File, Text, View, Application, Prompt, Background],
            C::WriteBufferClose => &[File, Text, View, Prompt, Background],
            C::SessionAttach | C::SessionList => &[Background, View],
            C::SessionStop | C::SessionClean => &[Background],
            C::SessionRename => &[Background, Configuration],
        };
        ActionPolicy {
            global_binding_modes: &[super::Mode::Normal, super::Mode::Select],
            effects,
            count: CountBehavior::Reject,
            operands: OperandArity::Zero,
            buffer_mutation: if matches!(
                self,
                C::Format | C::Pipe | C::Reload | C::WriteQuit | C::WriteBufferClose
            ) {
                BufferMutation::MayMutate
            } else {
                BufferMutation::Unchanged
            },
            requires_writable_buffer: false,
            legacy_sequence_position: LegacySequencePosition::Last,
        }
    }
}

impl CommandId {
    /// Plugins have runtime metadata and are not part of the built-in catalog.
    pub fn descriptor(self) -> Option<ActionDescriptor> {
        match self {
            Self::Editor(c) => Some(c.descriptor()),
            Self::Colon(c) => Some(c.descriptor()),
            Self::Plugin(_) => None,
        }
    }
}

fn palette(id: CommandId) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|spec| spec.id == id)
}
