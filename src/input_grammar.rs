// SPDX-License-Identifier: MPL-2.0

//! Presentation-neutral translation from owned input into editor intent.
//!
//! A grammar owns only interpretation state: prefixes, counts, and character
//! operands. Buffer state and presentation stay with the application, which
//! applies the resulting intent through shared editor commands.

use std::{fmt, num::NonZeroUsize};

use crate::{
    command::{
        CommandExecutionContext, CommandInvocation, CommandInvocationError, CommandUnavailable,
        EditorCommand, GrammarKind, HelpInvocation, Mode,
    },
    input::{InputEvent, KeyCode, KeyStroke, Modifiers},
    keymap::{BindingAvailability, BindingScope, BindingTarget, Key, KeySequence, Keymap, Lookup},
};

/// Read-only editor context needed to interpret one input event.
#[derive(Clone, Copy)]
pub struct GrammarContext<'a> {
    mode: Mode,
    scope: BindingScope,
    keymap: &'a Keymap,
    recording_macro: bool,
}

impl<'a> GrammarContext<'a> {
    pub const fn new(mode: Mode, scope: BindingScope, keymap: &'a Keymap) -> Self {
        Self {
            mode,
            scope,
            keymap,
            recording_macro: false,
        }
    }

    pub const fn mode(self) -> Mode {
        self.mode
    }

    pub const fn scope(self) -> BindingScope {
        self.scope
    }

    pub const fn keymap(self) -> &'a Keymap {
        self.keymap
    }

    pub const fn with_recording_macro(mut self, recording_macro: bool) -> Self {
        self.recording_macro = recording_macro;
        self
    }

    pub const fn recording_macro(self) -> bool {
        self.recording_macro
    }
}

/// Direction of a grammar-level line selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineDirection {
    Down,
    Up,
}

/// Buffer-dependent range operation requested by an input grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeIntent {
    SelectLine {
        direction: LineDirection,
        count: NonZeroUsize,
    },
}

/// Presentation-neutral state change for status and error surfaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GrammarNotice {
    PendingSequence(KeySequence),
    Count(usize),
    SequenceCancelled,
    NoBinding(KeySequence),
    AwaitingCharacter(EditorCommand),
    /// The first operand of a two-character command has arrived and the
    /// second has not.
    AwaitingSecondCharacter {
        command: EditorCommand,
        first: char,
    },
    CharacterInputCancelled,
    ExpectedCharacter,
    InvalidRegister {
        register: char,
        macros_only: bool,
    },
    UnavailableBinding {
        target: BindingTarget,
        availability: BindingAvailability,
    },
}

/// Semantic work emitted by an input grammar.
#[derive(Clone, Eq, PartialEq)]
pub enum EditorIntent {
    Command(CommandInvocation),
    InsertText(String),
    Range(RangeIntent),
    Notice(GrammarNotice),
}

impl fmt::Debug for EditorIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(invocation) => {
                formatter.debug_tuple("Command").field(invocation).finish()
            }
            Self::InsertText(text) => formatter
                .debug_struct("InsertText")
                .field("bytes", &text.len())
                .field("characters", &text.chars().count())
                .finish(),
            Self::Range(intent) => formatter.debug_tuple("Range").field(intent).finish(),
            Self::Notice(notice) => formatter.debug_tuple("Notice").field(notice).finish(),
        }
    }
}

/// State update that depends on the mode after emitted work is applied.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GrammarPostAction {
    #[default]
    None,
    RetainPrefixIfModal(KeyStroke),
}

/// Complete translation of one input event.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GrammarOutput {
    pub intents: Vec<EditorIntent>,
    pub reprocess: Option<InputEvent>,
    pub post_action: GrammarPostAction,
    /// The exact key spelling that resolved through the shared registry.
    /// Frontends can pair it with the target description after execution;
    /// semantic callers remain independent of interactive key bindings.
    pub resolved_binding: Option<(KeySequence, BindingTarget)>,
}

impl GrammarOutput {
    fn one(intent: EditorIntent) -> Self {
        Self {
            intents: vec![intent],
            ..Self::default()
        }
    }
}

/// Stateful, frontend-independent interpretation of editor input.
pub trait InputGrammar {
    fn translate(
        &mut self,
        input: InputEvent,
        context: GrammarContext<'_>,
    ) -> Result<GrammarOutput, CommandInvocationError>;

    /// Applies state whose meaning depends on the mode reached after the
    /// translated intents ran (currently Runyte's sticky `Z` prefix).
    fn complete(&mut self, action: GrammarPostAction, resulting_mode: Mode);

    fn pending_sequence(&self) -> &KeySequence;

    /// Numeric prefix typed so far, before the command key that consumes it.
    fn pending_count(&self) -> Option<usize>;

    /// Character-taking command whose operand has not arrived yet.
    fn awaiting_character(&self) -> Option<EditorCommand>;

    fn reset(&mut self);
}

/// Runyte's current Helix-style editing grammar.
#[derive(Clone, Debug, Default)]
pub struct RunyteGrammar {
    pending: KeySequence,
    count: Option<usize>,
    count_keys: KeySequence,
    awaiting_character: Option<(EditorCommand, usize)>,
    /// The first operand of an awaited two-character command, once typed.
    first_operand: Option<char>,
    awaiting_binding: Option<(KeySequence, BindingTarget)>,
}

/// The active, owned input interpreter. Unsupported grammar names are
/// rejected when parsing configuration or commands.
#[derive(Clone, Debug)]
pub enum ActiveGrammar {
    Runyte(RunyteGrammar),
}

impl Default for ActiveGrammar {
    fn default() -> Self {
        Self::Runyte(RunyteGrammar::default())
    }
}

impl ActiveGrammar {
    pub fn new(kind: GrammarKind) -> Self {
        match kind {
            GrammarKind::Runyte => Self::Runyte(RunyteGrammar::default()),
        }
    }

    /// Actual binding keys and any operand already entered. Presentation must
    /// not reconstruct these from defaults: configured bindings and counts
    /// are part of the gesture too.
    pub(crate) fn character_input_progress(&self) -> Option<(&KeySequence, bool)> {
        match self {
            Self::Runyte(grammar) => grammar
                .awaiting_binding
                .as_ref()
                .map(|(sequence, _)| (sequence, grammar.first_operand.is_some())),
        }
    }

    pub const fn kind(&self) -> GrammarKind {
        match self {
            Self::Runyte(_) => GrammarKind::Runyte,
        }
    }

    pub const fn preferred_mode(&self) -> Option<Mode> {
        match self {
            Self::Runyte(_) => None,
        }
    }
}

/// Runyte's compatibility policy for counts that repeat a key-bound command.
/// This is deliberately separate from semantic count acceptance: for example,
/// `42gg` addresses line 42 rather than executing `gg` 42 times.
const fn runyte_repeats_for_count(command: EditorCommand) -> bool {
    use EditorCommand as Command;
    matches!(
        command,
        Command::MoveLeft
            | Command::MoveRight
            | Command::MoveUp
            | Command::MoveDown
            | Command::MediaMoveUp
            | Command::MediaMoveDown
            | Command::MoveWordForward
            | Command::MoveWordBackward
            | Command::MoveWordEnd
            | Command::MoveLongWordForward
            | Command::MoveLongWordBackward
            | Command::MoveLongWordEnd
            | Command::PageUp
            | Command::PageDown
            | Command::HalfPageUp
            | Command::HalfPageDown
            | Command::ScrollViewDown
            | Command::ScrollViewUp
            | Command::SelectLine
            | Command::SelectLineUp
            | Command::PasteAfterOrReplace
            | Command::PasteAfter
            | Command::PasteBefore
            | Command::ClipboardPasteAfter
            | Command::ClipboardPasteBefore
            | Command::Undo
            | Command::Redo
            | Command::ReplayMacro
            | Command::ExpandSyntaxSelection
            | Command::ShrinkSyntaxSelection
            | Command::SelectSyntaxParent
            | Command::SelectSyntaxChild
            | Command::SelectPreviousSyntaxSibling
            | Command::SelectNextSyntaxSibling
            | Command::GotoPreviousSyntaxFunction
            | Command::GotoNextSyntaxFunction
            | Command::GotoPreviousSyntaxClass
            | Command::GotoNextSyntaxClass
            | Command::GotoPreviousSyntaxParameter
            | Command::GotoNextSyntaxParameter
    )
}

impl RunyteGrammar {
    /// Restores the grammar-owned count prefix to the registry sequence that
    /// resolved. The keymap owns the binding itself, while the grammar owns
    /// the decimal keys typed before it; completed-command feedback needs
    /// both to describe the actual gesture.
    fn resolved_sequence(count_keys: &KeySequence, binding: &KeySequence) -> KeySequence {
        if count_keys.is_empty() {
            return binding.clone();
        }
        KeySequence::new(
            count_keys
                .as_slice()
                .iter()
                .copied()
                .chain(binding.as_slice().iter().copied()),
        )
    }

    fn editor_binding_intent(
        &mut self,
        command: EditorCommand,
        availability: BindingAvailability,
    ) -> Result<EditorIntent, CommandInvocationError> {
        let count = self.count.take();
        match availability {
            BindingAvailability::Implemented => {
                let repetitions =
                    if count.is_some_and(|count| count > 1) && runyte_repeats_for_count(command) {
                        count.expect("checked count")
                    } else {
                        1
                    };
                if command.takes_character() {
                    self.awaiting_character = Some((command, repetitions));
                    return Ok(EditorIntent::Notice(GrammarNotice::AwaitingCharacter(
                        command,
                    )));
                }
                if command == EditorCommand::SelectLine || command == EditorCommand::SelectLineUp {
                    return Ok(EditorIntent::Range(RangeIntent::SelectLine {
                        direction: if command == EditorCommand::SelectLine {
                            LineDirection::Down
                        } else {
                            LineDirection::Up
                        },
                        count: NonZeroUsize::new(repetitions).expect("repetitions are non-zero"),
                    }));
                }
                let execution = count
                    .filter(|_| command.accepts_count())
                    .and_then(NonZeroUsize::new)
                    .map_or_else(CommandExecutionContext::default, |count| {
                        CommandExecutionContext::resolved(count, None)
                    });
                let invocation = if command == EditorCommand::ShowHelp {
                    CommandInvocation::help(HelpInvocation::ActiveView)
                } else {
                    CommandInvocation::editor(command, execution)?
                };
                Ok(EditorIntent::Command(invocation))
            }
            BindingAvailability::Planned(reason) => Ok(EditorIntent::Command(
                CommandInvocation::unavailable_editor(command, CommandUnavailable::Planned(reason)),
            )),
            BindingAvailability::Unsupported(reason) => Ok(EditorIntent::Command(
                CommandInvocation::unavailable_editor(
                    command,
                    CommandUnavailable::Unsupported(reason),
                ),
            )),
        }
    }

    fn binding_intent(
        &mut self,
        target: BindingTarget,
        availability: BindingAvailability,
    ) -> Result<EditorIntent, CommandInvocationError> {
        match target {
            BindingTarget::Editor(command) => self.editor_binding_intent(command, availability),
            BindingTarget::Colon(_) | BindingTarget::Plugin(_) if availability.is_implemented() => {
                self.count = None;
                Ok(EditorIntent::Command(target.invocation()?))
            }
            BindingTarget::Colon(_) | BindingTarget::Plugin(_) => {
                self.count = None;
                Ok(EditorIntent::Notice(GrammarNotice::UnavailableBinding {
                    target,
                    availability,
                }))
            }
        }
    }

    fn program_intents(
        &mut self,
        binding: &crate::keymap::Binding,
    ) -> Result<Vec<EditorIntent>, CommandInvocationError> {
        let count = self.count.take();
        let count_step = binding.program.count_step();
        binding
            .program
            .iter()
            .enumerate()
            .map(|(index, action)| {
                self.count = count.filter(|_| count_step == Some(index));
                if let crate::keymap::actions::Action::Invocation(invocation) = action
                    && let Some(count) = self.count.take()
                {
                    return Ok(EditorIntent::Command(CommandInvocation::from_parts(
                        invocation.id(),
                        invocation.parameters().clone(),
                        CommandExecutionContext::resolved(
                            NonZeroUsize::new(count).expect("positive count"),
                            None,
                        ),
                    )?));
                }
                use crate::keymap::actions::Action;
                match action {
                    Action::Editor(command) => {
                        self.editor_binding_intent(*command, binding.availability)
                    }
                    Action::Target(target) => self.binding_intent(*target, binding.availability),
                    Action::Invocation(invocation) => Ok(EditorIntent::Command(invocation.clone())),
                }
            })
            .collect()
    }

    fn translate_insert(
        &mut self,
        input: InputEvent,
        context: GrammarContext<'_>,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        if matches!(input, InputEvent::Pointer(_)) {
            return Ok(GrammarOutput::default());
        }
        let InputEvent::Key(key) = input else {
            self.pending.clear();
            let InputEvent::Text(text) = input else {
                unreachable!()
            };
            return Ok(GrammarOutput::one(EditorIntent::InsertText(text)));
        };
        let inside_prefix = !self.pending.is_empty();
        let mut candidate = self.pending.clone();
        candidate.push(key);
        match context
            .keymap()
            .lookup_in(context.mode(), context.scope(), &candidate)
        {
            Lookup::Exact(binding) => {
                let target = binding.program.last().target();
                self.pending.clear();
                let intents = self.program_intents(binding)?;
                if self.awaiting_character.is_some() {
                    self.awaiting_binding = Some((candidate, target));
                    Ok(GrammarOutput {
                        intents,
                        ..GrammarOutput::default()
                    })
                } else {
                    Ok(GrammarOutput {
                        intents,
                        resolved_binding: Some((candidate, target)),
                        ..GrammarOutput::default()
                    })
                }
            }
            Lookup::Prefix(_) | Lookup::ExactAndPrefix { .. } => {
                self.pending = candidate;
                Ok(GrammarOutput::one(EditorIntent::Notice(
                    GrammarNotice::PendingSequence(self.pending.clone()),
                )))
            }
            Lookup::NoMatch if inside_prefix => {
                let prefix = std::mem::take(&mut self.pending);
                let fallback =
                    match context
                        .keymap()
                        .lookup_in(context.mode(), context.scope(), &prefix)
                    {
                        Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                            Some(binding)
                        }
                        Lookup::NoMatch | Lookup::Prefix(_) => None,
                    };
                if let Some(binding) = fallback {
                    let target = binding.program.last().target();
                    let intents = self.program_intents(binding)?;
                    if self.awaiting_character.is_some() {
                        self.awaiting_binding = Some((prefix.clone(), target));
                    }
                    Ok(GrammarOutput {
                        intents,
                        reprocess: Some(InputEvent::Key(key)),
                        post_action: GrammarPostAction::None,
                        resolved_binding: self
                            .awaiting_character
                            .is_none()
                            .then_some((prefix, target)),
                    })
                } else {
                    Ok(GrammarOutput::one(EditorIntent::Notice(
                        GrammarNotice::NoBinding(candidate),
                    )))
                }
            }
            Lookup::NoMatch => {
                if let KeyCode::Char(character) = key.code
                    && !key
                        .modifiers
                        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
                {
                    Ok(GrammarOutput::one(EditorIntent::InsertText(
                        character.to_string(),
                    )))
                } else {
                    Ok(GrammarOutput::default())
                }
            }
        }
    }

    /// Translates one modal key.
    ///
    /// Recording reserves no key here. `Space m m` resolves to the same
    /// command whether or not a recording is running, and the editor decides
    /// there whether that starts one or ends the one already open.
    fn translate_modal(
        &mut self,
        key: KeyStroke,
        context: GrammarContext<'_>,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        if let Some((command, count)) = self.awaiting_character.take() {
            let first = self.first_operand.take();
            if context
                .keymap
                .ui()
                .lookup(crate::keymap::ui::Owner::CharacterInput, key)
                == Some(crate::keymap::ui::Action::Cancel)
            {
                self.awaiting_binding = None;
                return Ok(GrammarOutput::one(EditorIntent::Notice(
                    GrammarNotice::CharacterInputCancelled,
                )));
            }
            let KeyCode::Char(character) = key.code else {
                self.awaiting_binding = None;
                return Ok(GrammarOutput::one(EditorIntent::Notice(
                    GrammarNotice::ExpectedCharacter,
                )));
            };
            if command.takes_second_character() && first.is_none() {
                self.awaiting_character = Some((command, count));
                self.first_operand = Some(character);
                if let Some((sequence, _)) = self.awaiting_binding.as_mut() {
                    sequence.push(key);
                }
                return Ok(GrammarOutput::one(EditorIntent::Notice(
                    GrammarNotice::AwaitingSecondCharacter {
                        command,
                        first: character,
                    },
                )));
            }
            let resolved_binding = self.awaiting_binding.take().map(|(mut sequence, target)| {
                sequence.push(key);
                (sequence, target)
            });
            let execution = CommandExecutionContext::resolved(
                NonZeroUsize::new(count).expect("character repetition is non-zero"),
                Some(first.unwrap_or(character)),
            );
            let execution = match first {
                Some(_) => execution.with_second_character(character),
                None => execution,
            };
            return Ok(GrammarOutput {
                intents: vec![EditorIntent::Command(CommandInvocation::editor(
                    command, execution,
                )?)],
                resolved_binding,
                ..GrammarOutput::default()
            });
        }

        let prefix_action = context
            .keymap
            .ui()
            .lookup(crate::keymap::ui::Owner::GrammarPrefix, key);
        if prefix_action == Some(crate::keymap::ui::Action::Cancel) && !self.pending.is_empty() {
            self.pending.clear();
            self.count = None;
            self.count_keys.clear();
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::SequenceCancelled,
            )));
        }
        if prefix_action == Some(crate::keymap::ui::Action::Back) && !self.pending.is_empty() {
            self.pending.pop();
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                if self.pending.is_empty() {
                    GrammarNotice::SequenceCancelled
                } else {
                    GrammarNotice::PendingSequence(self.pending.clone())
                },
            )));
        }

        if self.pending.is_empty()
            && context.scope() != BindingScope::DirectoryTree
            && key.modifiers.is_empty()
            && let KeyCode::Char(digit @ '0'..='9') = key.code
            && (digit != '0' || self.count.is_some())
        {
            let digit = digit.to_digit(10).expect("matched a decimal digit") as usize;
            let count = self
                .count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit)
                .min(999_999);
            self.count = Some(count);
            self.count_keys.push(key);
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::Count(count),
            )));
        }

        let mut candidate = self.pending.clone();
        candidate.push(key);
        match context
            .keymap()
            .lookup_in(context.mode(), context.scope(), &candidate)
        {
            Lookup::NoMatch => {
                self.pending.clear();
                self.count = None;
                self.count_keys.clear();
                Ok(GrammarOutput::one(EditorIntent::Notice(
                    GrammarNotice::NoBinding(candidate),
                )))
            }
            Lookup::Prefix(_) | Lookup::ExactAndPrefix { .. } => {
                self.pending = candidate;
                Ok(GrammarOutput::one(EditorIntent::Notice(
                    GrammarNotice::PendingSequence(self.pending.clone()),
                )))
            }
            Lookup::Exact(binding) => {
                let target = binding.program.last().target();
                let count_keys = std::mem::take(&mut self.count_keys);
                let resolved_sequence = Self::resolved_sequence(&count_keys, &candidate);
                let sticky = !binding.configured_actions
                    && candidate
                        .as_slice()
                        .first()
                        .is_some_and(|key| *key == Key::char('Z'));
                self.pending.clear();
                let intents = self.program_intents(binding)?;
                if self.awaiting_character.is_some() {
                    self.awaiting_binding = Some((resolved_sequence.clone(), target));
                }
                Ok(GrammarOutput {
                    intents,
                    reprocess: None,
                    post_action: if sticky {
                        GrammarPostAction::RetainPrefixIfModal(Key::char('Z'))
                    } else {
                        GrammarPostAction::None
                    },
                    resolved_binding: self
                        .awaiting_character
                        .is_none()
                        .then_some((resolved_sequence, target)),
                })
            }
        }
    }
}

impl InputGrammar for RunyteGrammar {
    fn translate(
        &mut self,
        input: InputEvent,
        context: GrammarContext<'_>,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        match context.mode() {
            Mode::Insert | Mode::Replace => self.translate_insert(input, context),
            Mode::Normal | Mode::Select => match input {
                InputEvent::Key(key) => self.translate_modal(key, context),
                InputEvent::Text(_) | InputEvent::ClipboardPaste | InputEvent::Pointer(_) => {
                    Ok(GrammarOutput::default())
                }
            },
            Mode::Command | Mode::List => Ok(GrammarOutput::default()),
        }
    }

    fn complete(&mut self, action: GrammarPostAction, resulting_mode: Mode) {
        if let GrammarPostAction::RetainPrefixIfModal(key) = action
            && matches!(resulting_mode, Mode::Normal | Mode::Select)
        {
            self.pending.push(key);
        }
    }

    fn pending_sequence(&self) -> &KeySequence {
        &self.pending
    }

    fn pending_count(&self) -> Option<usize> {
        self.count
    }

    fn awaiting_character(&self) -> Option<EditorCommand> {
        self.awaiting_character.map(|(command, _)| command)
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.count = None;
        self.count_keys.clear();
        self.awaiting_character = None;
        self.first_operand = None;
        self.awaiting_binding = None;
    }
}

impl InputGrammar for ActiveGrammar {
    fn translate(
        &mut self,
        input: InputEvent,
        context: GrammarContext<'_>,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        match self {
            Self::Runyte(grammar) => grammar.translate(input, context),
        }
    }

    fn complete(&mut self, action: GrammarPostAction, resulting_mode: Mode) {
        match self {
            Self::Runyte(grammar) => grammar.complete(action, resulting_mode),
        }
    }

    fn pending_sequence(&self) -> &KeySequence {
        match self {
            Self::Runyte(grammar) => grammar.pending_sequence(),
        }
    }

    fn pending_count(&self) -> Option<usize> {
        match self {
            Self::Runyte(grammar) => grammar.pending_count(),
        }
    }

    fn awaiting_character(&self) -> Option<EditorCommand> {
        match self {
            Self::Runyte(grammar) => grammar.awaiting_character(),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Runyte(grammar) => grammar.reset(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::CommandId,
        input::{PointerButton, PointerEvent, PointerEventKind},
        keymap::default_keymap,
    };

    fn context(mode: Mode, scope: BindingScope) -> GrammarContext<'static> {
        GrammarContext::new(mode, scope, default_keymap())
    }

    fn translate_key(
        grammar: &mut RunyteGrammar,
        mode: Mode,
        scope: BindingScope,
        key: KeyStroke,
    ) -> GrammarOutput {
        grammar
            .translate(InputEvent::Key(key), context(mode, scope))
            .unwrap()
    }

    fn only_intent(output: GrammarOutput) -> EditorIntent {
        assert!(output.reprocess.is_none());
        assert_eq!(output.intents.len(), 1);
        output.intents.into_iter().next().unwrap()
    }

    #[test]
    fn character_input_cancellation_uses_effective_actions_and_keeps_operands_structural() {
        for source in [
            "{}",
            "ui: {character-input: {cancel: [F8]}}",
            "ui: {character-input: {cancel: []}}",
        ] {
            let compiled = crate::keymap::configured::compile(
                &serde_yaml::from_str(source).unwrap(),
                crate::keymap::default_keymap(),
            );
            assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
            for mode in [Mode::Normal, Mode::Select] {
                for sequence in ["2 f", "m r", "m r (", "Space m R"] {
                    for bits in 0..64 {
                        let modifiers = Modifiers::from_bits(bits).unwrap();
                        for code in [KeyCode::Escape, KeyCode::Function(8), KeyCode::Char('x')] {
                            let mut grammar = RunyteGrammar::default();
                            let context = || {
                                GrammarContext::new(mode, BindingScope::Global, &compiled.keymap)
                            };
                            for key in KeySequence::parse(sequence).unwrap().as_slice() {
                                grammar.translate(InputEvent::Key(*key), context()).unwrap();
                            }
                            assert!(grammar.awaiting_character().is_some(), "{sequence}");
                            let key = KeyStroke::new(code, modifiers);
                            let output =
                                grammar.translate(InputEvent::Key(key), context()).unwrap();
                            let intent = only_intent(output);
                            if code == KeyCode::Char('x') && sequence == "m r" {
                                assert!(matches!(
                                    intent,
                                    EditorIntent::Notice(GrammarNotice::AwaitingSecondCharacter {
                                        first: 'x',
                                        ..
                                    })
                                ));
                            } else if code == KeyCode::Char('x') {
                                assert!(
                                    matches!(intent, EditorIntent::Command(_)),
                                    "{sequence} {key:?}: {intent:?}"
                                );
                            } else {
                                let cancelled = source == "{}" && code == KeyCode::Escape
                                    || source.contains("[F8]")
                                        && key == KeyStroke::plain(KeyCode::Function(8));
                                assert_eq!(
                                    intent,
                                    EditorIntent::Notice(if cancelled {
                                        GrammarNotice::CharacterInputCancelled
                                    } else {
                                        GrammarNotice::ExpectedCharacter
                                    }),
                                    "{sequence} {source} {key:?}"
                                );
                            }
                            assert_eq!(
                                grammar.awaiting_character().is_some(),
                                code == KeyCode::Char('x') && sequence == "m r"
                            );
                        }
                    }
                }
            }
        }
        for alias in ["x", "Ctrl-c", "Alt-x", "Meta-x"] {
            let source = format!("ui: {{character-input: {{cancel: [{alias}]}}}}");
            let compiled = crate::keymap::configured::compile(
                &serde_yaml::from_str(&source).unwrap(),
                crate::keymap::default_keymap(),
            );
            assert_eq!(compiled.errors.len(), 1, "{alias}");
            assert_eq!(
                compiled.keymap.ui().lookup(
                    crate::keymap::ui::Owner::CharacterInput,
                    KeyStroke::plain(KeyCode::Escape)
                ),
                Some(crate::keymap::ui::Action::Cancel)
            );
        }
    }

    #[test]
    fn pointer_input_is_a_safe_noop_in_every_grammar_and_mode() {
        let pointer = InputEvent::Pointer(PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: 4,
            row: 2,
            modifiers: Modifiers::NONE,
        });
        for kind in GrammarKind::ALL {
            let mut grammar = ActiveGrammar::new(*kind);
            for mode in [Mode::Normal, Mode::Select, Mode::Insert, Mode::Command] {
                let output = grammar
                    .translate(pointer.clone(), context(mode, BindingScope::Global))
                    .unwrap();
                assert!(output.intents.is_empty(), "{kind:?} {mode:?}");
                assert!(output.reprocess.is_none(), "{kind:?} {mode:?}");
            }
        }
    }

    #[test]
    fn modal_keys_emit_semantic_commands_and_counts() {
        let mut grammar = RunyteGrammar::default();
        let intent = only_intent(translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('h'),
        ));
        let EditorIntent::Command(invocation) = intent else {
            panic!("expected command intent")
        };
        assert_eq!(invocation.id(), CommandId::Editor(EditorCommand::MoveLeft));
        assert_eq!(invocation.execution().count(), None);

        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::char('3'),
            )),
            EditorIntent::Notice(GrammarNotice::Count(3))
        );
        let EditorIntent::Command(invocation) = only_intent(translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('w'),
        )) else {
            panic!("expected command intent")
        };
        assert_eq!(
            invocation.id(),
            CommandId::Editor(EditorCommand::MoveWordForward)
        );
        assert_eq!(invocation.execution().count(), Some(3));
    }

    #[test]
    fn resolved_binding_spelling_keeps_the_typed_count_prefix() {
        let mut grammar = RunyteGrammar::default();
        translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('3'),
        );
        let counted_motion = translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('w'),
        );
        assert_eq!(
            counted_motion
                .resolved_binding
                .as_ref()
                .map(|(sequence, _)| sequence.to_string()),
            Some("3 w".to_owned())
        );

        translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('2'),
        );
        let awaiting = translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('f'),
        );
        assert!(awaiting.resolved_binding.is_none());
        let counted_character = translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('x'),
        );
        assert_eq!(
            counted_character
                .resolved_binding
                .as_ref()
                .map(|(sequence, _)| sequence.to_string()),
            Some("2 f x".to_owned())
        );

        let mut saturated = RunyteGrammar::default();
        for _ in 0..7 {
            translate_key(
                &mut saturated,
                Mode::Normal,
                BindingScope::Global,
                Key::char('9'),
            );
        }
        assert_eq!(saturated.pending_count(), Some(999_999));
        let resolved = translate_key(
            &mut saturated,
            Mode::Normal,
            BindingScope::Global,
            Key::char('l'),
        );
        assert_eq!(
            resolved
                .resolved_binding
                .as_ref()
                .map(|(sequence, _)| sequence.to_string()),
            Some("9 9 9 9 9 9 9 l".to_owned())
        );
    }

    #[test]
    fn semantic_count_acceptance_is_distinct_from_runyte_repetition() {
        assert!(EditorCommand::MoveFileStart.accepts_count());
        assert!(EditorCommand::MoveFileEnd.accepts_count());
        assert!(!runyte_repeats_for_count(EditorCommand::MoveFileStart));
        assert!(!runyte_repeats_for_count(EditorCommand::MoveFileEnd));

        for command in EditorCommand::ALL {
            assert!(
                !runyte_repeats_for_count(*command) || command.accepts_count(),
                "Runyte repetition must remain a subset of semantic count acceptance: {command:?}"
            );
        }
    }

    #[test]
    fn bare_zero_remains_a_binding_but_later_zero_extends_a_count() {
        let mut grammar = RunyteGrammar::default();
        let EditorIntent::Command(invocation) = only_intent(translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('0'),
        )) else {
            panic!("expected command intent")
        };
        assert_eq!(
            invocation.id(),
            CommandId::Editor(EditorCommand::MoveLineStart)
        );

        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::char('2'),
            )),
            EditorIntent::Notice(GrammarNotice::Count(2))
        );
        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::char('0'),
            )),
            EditorIntent::Notice(GrammarNotice::Count(20))
        );
    }

    #[test]
    fn line_keys_emit_explicit_directional_range_intents() {
        let mut grammar = RunyteGrammar::default();
        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::char('x'),
            )),
            EditorIntent::Range(RangeIntent::SelectLine {
                direction: LineDirection::Down,
                count: NonZeroUsize::MIN,
            })
        );

        translate_key(
            &mut grammar,
            Mode::Select,
            BindingScope::Global,
            Key::char('3'),
        );
        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Select,
                BindingScope::Global,
                Key::char('X'),
            )),
            EditorIntent::Command(
                CommandInvocation::editor(
                    EditorCommand::ExtendLineAbove,
                    CommandExecutionContext::resolved(NonZeroUsize::new(3).unwrap(), None),
                )
                .unwrap()
            )
        );
    }

    #[test]
    fn runyte_find_discards_a_count_before_resolving_its_character() {
        let mut grammar = RunyteGrammar::default();
        translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('3'),
        );
        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::char('f'),
            )),
            EditorIntent::Notice(GrammarNotice::AwaitingCharacter(
                EditorCommand::FindNextChar
            ))
        );
        let EditorIntent::Command(invocation) = only_intent(translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('λ'),
        )) else {
            panic!("expected command intent")
        };
        assert_eq!(
            invocation.id(),
            CommandId::Editor(EditorCommand::FindNextChar)
        );
        assert_eq!(invocation.execution().count(), Some(1));
        assert_eq!(invocation.execution().character(), Some('λ'));
    }

    #[test]
    fn runyte_surround_replace_takes_two_character_operands() {
        let mut grammar = RunyteGrammar::default();
        for key in [Key::char('m'), Key::char('r')] {
            translate_key(&mut grammar, Mode::Normal, BindingScope::Global, key);
        }
        assert_eq!(
            grammar.awaiting_character(),
            Some(EditorCommand::SurroundReplace)
        );
        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::char('('),
            )),
            EditorIntent::Notice(GrammarNotice::AwaitingSecondCharacter {
                command: EditorCommand::SurroundReplace,
                first: '(',
            })
        );
        assert_eq!(
            grammar.awaiting_character(),
            Some(EditorCommand::SurroundReplace),
            "the second operand is still a character, not a binding"
        );
        let output = translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('m'),
        );
        assert_eq!(
            output.resolved_binding,
            Some((
                KeySequence::from([
                    Key::char('m'),
                    Key::char('r'),
                    Key::char('('),
                    Key::char('m'),
                ]),
                BindingTarget::Editor(EditorCommand::SurroundReplace),
            ))
        );
        let EditorIntent::Command(invocation) = only_intent(output) else {
            panic!("expected command intent")
        };
        assert_eq!(
            invocation.id(),
            CommandId::Editor(EditorCommand::SurroundReplace)
        );
        assert_eq!(invocation.execution().character(), Some('('));
        assert_eq!(invocation.execution().second_character(), Some('m'));
        assert_eq!(grammar.awaiting_character(), None);

        // Escape between the operands abandons both, and the next keys are
        // bindings again rather than a replacement.
        for key in [Key::char('m'), Key::char('r'), Key::char('(')] {
            translate_key(&mut grammar, Mode::Normal, BindingScope::Global, key);
        }
        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::plain(KeyCode::Escape),
            )),
            EditorIntent::Notice(GrammarNotice::CharacterInputCancelled)
        );
        assert_eq!(grammar.awaiting_character(), None);
        for key in [Key::char('m'), Key::char('d')] {
            translate_key(&mut grammar, Mode::Normal, BindingScope::Global, key);
        }
        let EditorIntent::Command(invocation) = only_intent(translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('"'),
        )) else {
            panic!("expected command intent")
        };
        assert_eq!(
            invocation.id(),
            CommandId::Editor(EditorCommand::SurroundDelete)
        );
        assert_eq!(invocation.execution().character(), Some('"'));
        assert_eq!(invocation.execution().second_character(), None);
    }

    #[test]
    fn prefixes_cancel_backspace_and_report_invalid_sequences_as_data() {
        let mut grammar = RunyteGrammar::default();
        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::char('g'),
            )),
            EditorIntent::Notice(GrammarNotice::PendingSequence(KeySequence::from(
                Key::char('g')
            )))
        );
        assert_eq!(
            only_intent(translate_key(
                &mut grammar,
                Mode::Normal,
                BindingScope::Global,
                Key::plain(KeyCode::Backspace),
            )),
            EditorIntent::Notice(GrammarNotice::SequenceCancelled)
        );
        translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('g'),
        );
        let intent = only_intent(translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('!'),
        ));
        assert!(matches!(
            intent,
            EditorIntent::Notice(GrammarNotice::NoBinding(_))
        ));
        assert!(grammar.pending_sequence().is_empty());
    }

    #[test]
    fn sticky_z_is_retained_only_after_a_modal_result() {
        let mut grammar = RunyteGrammar::default();
        translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('Z'),
        );
        let output = translate_key(
            &mut grammar,
            Mode::Normal,
            BindingScope::Global,
            Key::char('j'),
        );
        assert_eq!(
            output.post_action,
            GrammarPostAction::RetainPrefixIfModal(Key::char('Z'))
        );
        grammar.complete(output.post_action, Mode::Normal);
        assert_eq!(grammar.pending_sequence().to_string(), "Z");
        grammar.reset();
        grammar.complete(
            GrammarPostAction::RetainPrefixIfModal(Key::char('Z')),
            Mode::Insert,
        );
        assert!(grammar.pending_sequence().is_empty());
        grammar.complete(
            GrammarPostAction::RetainPrefixIfModal(Key::char('Z')),
            Mode::Command,
        );
        assert!(grammar.pending_sequence().is_empty());
    }

    #[test]
    fn literal_text_is_one_redacted_intent() {
        let mut grammar = RunyteGrammar::default();
        let output = grammar
            .translate(
                InputEvent::Text("private λ text".to_owned()),
                context(Mode::Insert, BindingScope::Global),
            )
            .unwrap();
        assert_eq!(
            output.intents,
            vec![EditorIntent::InsertText("private λ text".to_owned())]
        );
        assert!(!format!("{:?}", output.intents[0]).contains("private"));
    }

    #[test]
    fn colon_binding_emits_the_same_typed_invocation_with_a_count_prefix() {
        let run = |grammar: &mut RunyteGrammar, keys: &[char]| {
            let mut output = GrammarOutput::default();
            for key in keys {
                output =
                    translate_key(grammar, Mode::Normal, BindingScope::Global, Key::char(*key));
            }
            output
        };

        let mut grammar = RunyteGrammar::default();
        let EditorIntent::Command(invocation) = only_intent(run(&mut grammar, &[' ', 'l', '?']))
        else {
            panic!("expected typed colon invocation")
        };
        assert_eq!(
            invocation.id(),
            crate::command::CommandId::Colon(crate::command::ColonCommand::LspStatus)
        );

        let mut counted = RunyteGrammar::default();
        run(&mut counted, &['2']);
        assert_eq!(
            only_intent(run(&mut counted, &[' ', 'l', '?'])),
            EditorIntent::Command(invocation)
        );
    }

    #[test]
    fn unavailable_colon_target_never_executes() {
        let target = BindingTarget::Colon(crate::command::ColonCommand::Format);
        let mut grammar = RunyteGrammar::default();
        assert_eq!(
            grammar
                .binding_intent(target, BindingAvailability::Planned("later"))
                .unwrap(),
            EditorIntent::Notice(GrammarNotice::UnavailableBinding {
                target,
                availability: BindingAvailability::Planned("later"),
            })
        );
    }

    #[test]
    fn grammar_module_has_no_frontend_or_application_dependency() {
        let source = include_str!("input_grammar.rs");
        for forbidden in [
            ["cross", "term"].concat(),
            ["rata", "tui"].concat(),
            ["crate::", "app"].concat(),
            ["A", "pp::"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "forbidden dependency {forbidden}"
            );
        }
    }
}
