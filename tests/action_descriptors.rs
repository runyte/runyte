// SPDX-License-Identifier: MPL-2.0

use runyte::command::EditorCommand;
use runyte::keymap::{BindingTarget, actions};

// Every built-in identity follows the same effect rule, including colon-only
// actions. Operands are resumable; prompts and deferred work own the tail.
#[test]
fn continuation_follows_effects_for_every_builtin_action() {
    use runyte::command::{
        ColonCommand, CommandId, InvocationParameters, descriptor::ActionEffect,
    };
    for id in EditorCommand::ALL
        .iter()
        .copied()
        .map(CommandId::Editor)
        .chain(ColonCommand::ALL.iter().copied().map(CommandId::Colon))
    {
        let d = id.descriptor().unwrap();
        let expected = !d
            .policy
            .effects
            .iter()
            .any(|effect| matches!(effect, ActionEffect::Prompt | ActionEffect::Background));
        assert_eq!(
            d.can_continue(&InvocationParameters::None),
            expected,
            "{id:?}"
        );
    }
    assert!(!actions::can_continue(BindingTarget::Plugin(42)));
}

#[test]
fn default_global_editor_bindings_use_descriptor_admitted_modes() {
    use runyte::command::Mode;
    use runyte::keymap::{BindingAvailability, BindingScope, default_keymap};

    for binding in default_keymap().bindings() {
        if binding.scope != BindingScope::Global
            || binding.availability != BindingAvailability::Implemented
        {
            continue;
        }
        let BindingTarget::Editor(command) = binding.target else {
            continue;
        };
        for mode in [Mode::Normal, Mode::Select, Mode::Insert, Mode::Replace] {
            if binding.is_active_in(mode) {
                assert!(
                    command.action_policy().global_binding_modes.contains(&mode),
                    "{command:?} has an implemented default global binding in {mode:?}, but its descriptor does not admit that mode"
                );
            }
        }
    }
}

#[test]
fn every_builtin_identity_has_a_complete_descriptor() {
    use runyte::command::{COMMANDS, ColonCommand, CommandId};
    use runyte::keymap::{BindingScope, admission};
    use std::collections::HashSet;
    let mut identities = HashSet::new();
    let mut names = HashSet::new();
    for id in EditorCommand::ALL
        .iter()
        .copied()
        .map(CommandId::Editor)
        .chain(ColonCommand::ALL.iter().copied().map(CommandId::Colon))
    {
        let d = id.descriptor().expect("built-in descriptor");
        assert!(identities.insert(d.id));
        assert_eq!(d.id, id);
        assert!(!d.metadata.name.is_empty());
        assert!(
            names.insert(d.metadata.name),
            "duplicate action name {id:?}"
        );
        assert!(!d.metadata.description.is_empty());
        assert!(!d.policy.effects.is_empty(), "{id:?}");
        assert_eq!(d.metadata.capability, id.capability());
        assert_eq!(d.category, id.category());
        assert_eq!(d.palette_context, id.context());
        let specs: Vec<_> = COMMANDS.iter().filter(|s| s.id == id).collect();
        assert!(specs.len() <= 1, "duplicate palette identity {id:?}");
        assert_eq!(d.palette.map(|p| p.name), specs.first().map(|s| s.name));
        for &scope in BindingScope::ALL {
            let target = match id {
                CommandId::Editor(c) => BindingTarget::Editor(c),
                CommandId::Colon(c) => BindingTarget::Colon(c),
                _ => unreachable!(),
            };
            assert_eq!(
                d.view(scope).is_some(),
                admission::admits(scope, target),
                "{id:?} {scope:?}"
            );
            if let Some(v) = d.view(scope) {
                assert!(!v.description.is_empty());
            }
        }
        let mut scopes = HashSet::new();
        for v in d.view_overrides {
            assert!(scopes.insert(v.scope), "duplicate view override {id:?}");
            assert!(!v.description.is_empty());
        }
    }
    assert_eq!(
        identities.len(),
        EditorCommand::ALL.len() + ColonCommand::ALL.len()
    );
    assert!(CommandId::Plugin(42).descriptor().is_none());
}

#[test]
fn descriptors_distinguish_effects_from_writable_admission() {
    use EditorCommand as C;
    use runyte::command::{ColonCommand, descriptor::*};
    assert_eq!(
        C::EnterInsertMode.action_policy().buffer_mutation,
        BufferMutation::Unchanged
    );
    assert!(C::EnterInsertMode.is_mutating());
    assert_eq!(
        C::AlignSelections.action_policy().buffer_mutation,
        BufferMutation::MayMutate
    );
    assert!(!C::AlignSelections.is_mutating());
    assert!(actions::can_continue(BindingTarget::Editor(
        C::EnterInsertMode
    )));
    // A split can also open a path, which has prompt/background effects.
    assert!(!actions::can_continue(BindingTarget::Editor(
        C::SplitVertical
    )));
    assert!(
        C::EnterInsertMode
            .action_policy()
            .effects
            .contains(&ActionEffect::Mode)
    );
    assert!(
        C::SplitVertical
            .action_policy()
            .effects
            .contains(&ActionEffect::Pane)
    );
    assert_eq!(
        ColonCommand::Pipe.action_policy().buffer_mutation,
        BufferMutation::MayMutate
    );
    assert!(!actions::can_continue(BindingTarget::Colon(
        ColonCommand::Pipe
    )));
    assert_eq!(
        C::MoveFileStart.action_policy().count,
        CountBehavior::Interpret
    );
    assert_eq!(
        C::PasteAfter.action_policy().count,
        CountBehavior::Interpret
    );
    assert_eq!(C::Yank.action_policy().count, CountBehavior::Reject);
}

#[test]
fn palette_arguments_and_interactive_operands_remain_separate() {
    use EditorCommand as C;
    use runyte::command::{ColonCommand, CommandArguments, descriptor::*};
    assert_eq!(
        C::SurroundReplace.action_policy().operands,
        OperandArity::Two
    );
    assert!(C::SurroundReplace.descriptor().argument().is_none());
    assert_eq!(C::FindNextChar.action_policy().operands, OperandArity::One);
    assert!(C::FindNextChar.descriptor().argument().is_none());
    let save = C::Save.descriptor();
    assert_eq!(save.metadata.name, "save");
    assert_eq!(save.palette.unwrap().name, "write");
    assert!(matches!(
        save.argument().unwrap().syntax,
        CommandArguments::Optional(_)
    ));
    assert_eq!(
        C::RenameTerminal.descriptor().argument().unwrap().delivery,
        ArgumentDelivery::ReplacesPrompt
    );
    let pipe = ColonCommand::Pipe.descriptor();
    assert!(matches!(
        pipe.argument().unwrap().syntax,
        CommandArguments::Required(_)
    ));
    assert_eq!(
        pipe.argument().unwrap().delivery,
        ArgumentDelivery::Invocation
    );
    assert!(pipe.policy.effects.contains(&ActionEffect::Background));
}

#[test]
fn configuration_rejects_blocking_effects_and_retains_mode_boundaries() {
    use runyte::keymap::{configured, default_keymap};
    for source in [
        "bind: {normal: {F8: [split-vertical, move-left]}}",
        "bind: {normal: {F8: [{command: pipe, argument: sort}, move-left]}}",
        "bind: {normal: {F8: list-next}}",
        "bind: {normal: {F8: commit-undo-checkpoint}}",
        "bind: {insert: {F8: git-status}}",
        "bind: {normal: {F8: {command: surround-replace, argument: ab}}}",
    ] {
        let value = serde_yaml::from_str(source).unwrap();
        let compiled = configured::compile(&value, default_keymap());
        assert!(
            !compiled.errors.is_empty(),
            "unexpected admission: {source}"
        );
    }
}

#[test]
fn differing_view_implementations_are_described_without_changing_admission() {
    use EditorCommand as C;
    use runyte::keymap::BindingScope as S;
    let yank = C::Yank.descriptor();
    assert!(
        yank.view(S::Terminal)
            .unwrap()
            .description
            .contains("retain")
    );
    assert!(
        yank.view(S::Directory)
            .unwrap()
            .description
            .contains("copy payload")
    );
    assert!(yank.view(S::Media).is_none());
    for c in [C::PasteAfter, C::PasteBefore] {
        assert!(c.descriptor().view(S::Terminal).is_none());
    }
    for c in [C::TerminalPaste, C::TerminalClipboardPasteText] {
        assert!(c.descriptor().view(S::Terminal).is_some());
        assert!(
            !c.action_policy()
                .effects
                .contains(&runyte::command::descriptor::ActionEffect::Mode)
        );
    }
    assert!(
        C::MoveUp
            .descriptor()
            .view(S::Media)
            .unwrap()
            .description
            .contains("PDF page")
    );
    assert!(
        C::CutSelection
            .descriptor()
            .view(S::Directory)
            .unwrap()
            .description
            .contains("move payload")
    );
}

#[test]
fn descriptive_effects_include_bundled_steps_and_actual_input_delivery() {
    use EditorCommand as C;
    use runyte::command::descriptor::*;
    for c in [
        C::SelectLine,
        C::SelectLineUp,
        C::ExtendLineAbove,
        C::ExtendLineBelow,
    ] {
        assert_eq!(c.action_policy().count, CountBehavior::Repeat);
    }
    for c in [
        C::ExpandSyntaxSelection,
        C::FlipSelection,
        C::SelectSyntaxParent,
        C::SelectInsideWord,
        C::SelectAroundBraces,
    ] {
        assert!(
            !c.action_policy().effects.contains(&ActionEffect::Mode),
            "{c:?}"
        );
    }
    for c in [C::GlobalSearch, C::GlobalSearchRegex] {
        assert!(
            c.action_policy()
                .effects
                .contains(&ActionEffect::Background)
        );
        assert!(c.action_policy().effects.contains(&ActionEffect::View));
    }
    assert_eq!(
        C::Diagnostics.action_policy().effects,
        &[ActionEffect::View]
    );
    let send = C::SendToTerminal.descriptor();
    assert!(!send.policy.effects.contains(&ActionEffect::Prompt));
    assert_eq!(
        send.argument().unwrap().delivery,
        ArgumentDelivery::Invocation
    );
}

#[test]
fn semantic_count_and_operand_validation_keeps_its_existing_boundaries() {
    use EditorCommand as C;
    use runyte::command::{CommandExecutionContext as E, CommandInvocation as I};
    use std::num::NonZeroUsize as N;
    let one = E::resolved(N::new(1).unwrap(), None);
    let two = E::resolved(N::new(2).unwrap(), None);
    assert!(I::editor(C::Yank, one).is_ok());
    assert!(I::editor(C::Yank, two).is_err());
    assert!(I::editor(C::SelectLine, two).is_ok());
    let bare = I::editor(C::MoveFileEnd, E::default()).unwrap();
    let addressed = I::editor(C::MoveFileEnd, one).unwrap();
    assert_eq!(bare.execution().count(), None);
    assert_eq!(addressed.execution().count(), Some(1));
    let operand = E::resolved(N::new(1).unwrap(), Some('('));
    assert!(I::editor(C::SurroundReplace, operand).is_err());
    assert!(I::editor(C::SurroundReplace, operand.with_second_character('[')).is_ok());
    assert!(I::editor(C::FindNextChar, operand.with_second_character('[')).is_err());
}

#[test]
fn effects_distinguish_synchronous_views_from_owned_async_and_prompt_work() {
    use runyte::command::{ColonCommand as C, descriptor::ActionEffect as E};
    assert_eq!(C::DiffDisk.action_policy().effects, &[E::File, E::View]);
    assert!(
        C::DiffRemote
            .action_policy()
            .effects
            .contains(&E::Background)
    );
    assert_eq!(C::GitNetworkChooseRef.action_policy().effects, &[E::View]);
    assert_eq!(
        C::GitSearchCommits.action_policy().effects,
        &[E::Background, E::View]
    );
    for c in [
        C::Reload,
        C::SessionList,
        C::LspStatus,
        C::AgentChannel,
        C::ConfigReload,
    ] {
        assert!(c.action_policy().effects.contains(&E::Background), "{c:?}");
    }
    for c in [C::SessionStop, C::SessionClean] {
        assert!(!c.action_policy().effects.contains(&E::Prompt), "{c:?}");
    }
    assert!(C::Grammar.action_policy().effects.contains(&E::Mode));
    assert!(C::Resize.action_policy().effects.contains(&E::Prompt));
    assert!(
        C::WriteQuit
            .action_policy()
            .effects
            .contains(&E::Application)
    );
    assert!(
        !C::WriteBufferClose
            .action_policy()
            .effects
            .contains(&E::Application)
    );
    for c in [C::WriteQuit, C::WriteBufferClose] {
        assert!(c.action_policy().effects.contains(&E::Prompt));
        assert!(c.action_policy().effects.contains(&E::Background));
    }
    for c in [
        EditorCommand::TerminalPaste,
        EditorCommand::TerminalClipboardPasteText,
        EditorCommand::TerminalUndoPaste,
    ] {
        assert!(c.action_policy().effects.contains(&E::TerminalInput));
    }
    assert_eq!(
        EditorCommand::ClipboardPaste.action_policy().effects,
        &[E::Text, E::Selection]
    );
    assert!(
        EditorCommand::DirectoryTreeSearch
            .action_policy()
            .effects
            .contains(&E::SearchState)
    );
    for c in [EditorCommand::MoveLeft, EditorCommand::MoveFileEnd] {
        assert!(c.action_policy().effects.contains(&E::View));
    }
    for c in [
        EditorCommand::OpenNavigator,
        EditorCommand::OpenBufferPicker,
    ] {
        assert_eq!(c.action_policy().effects, &[E::View]);
    }
}

#[test]
fn owned_continuations_and_specialized_mode_transitions_are_described() {
    use runyte::command::{ColonCommand, EditorCommand as C, descriptor::ActionEffect as E};
    for c in [
        C::SelectionUndo,
        C::SelectionRedo,
        C::ListCloseTarget,
        C::ToggleZen,
        C::ToggleFullscreen,
        C::ShowAbout,
        C::CopyTerminalOutput,
        C::ToggleMarkdownRender,
        C::GlobalSearch,
        C::GlobalSearchRegex,
        C::JumpBackward,
        C::JumpForward,
        C::JumpBackwardBuffer,
        C::JumpForwardBuffer,
        C::PreviousDestination,
        C::SwapWindow,
        C::FocusWindowLeft,
        C::SplitVertical,
        C::ShowHelp,
        C::TriggerCompletion,
        C::DirectoryTreeOpen,
        C::MergeReviewApprove,
        C::OpenGitConflicts,
    ] {
        assert!(c.action_policy().effects.contains(&E::Mode), "{c:?}");
    }
    for c in [
        C::OpenExplorerSession,
        C::DirectoryTreeHorizontal,
        C::NextRunningSession,
    ] {
        assert!(c.action_policy().effects.contains(&E::Background), "{c:?}");
    }
    let close = C::ListCloseTarget.action_policy();
    assert_eq!(
        close.buffer_mutation,
        runyte::command::descriptor::BufferMutation::MayMutate
    );
    for effect in [E::Prompt, E::Background, E::Text, E::File] {
        assert!(close.effects.contains(&effect));
    }
    assert_eq!(
        C::PreviousGitNetworkPage.action_policy().effects,
        &[E::View, E::Mode]
    );
    assert!(
        ColonCommand::Notifications
            .action_policy()
            .effects
            .contains(&E::Mode)
    );
    assert!(
        C::ToggleSoftWrap
            .action_policy()
            .effects
            .contains(&E::Configuration)
    );
}

#[test]
fn explicit_plugin_invocations_are_always_last() {
    use runyte::command::{
        CommandExecutionContext, CommandId, CommandInvocation, InvocationParameters,
    };
    let invocation = CommandInvocation::from_parts(
        CommandId::Plugin(42),
        InvocationParameters::None,
        CommandExecutionContext::default(),
    )
    .unwrap();
    assert!(!actions::Action::Invocation(invocation).can_continue());
}

#[test]
fn generated_page_destinations_describe_owned_mode_transitions() {
    use runyte::command::{ColonCommand as C, descriptor::ActionEffect as E};
    for command in [
        C::AgentChannel,
        C::LogOpen,
        C::GitCompare,
        C::GitDiff,
        C::GitIndex,
    ] {
        assert!(
            command.action_policy().effects.contains(&E::Mode),
            "{command:?}"
        );
    }
}
