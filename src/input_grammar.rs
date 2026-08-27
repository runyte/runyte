// SPDX-License-Identifier: MPL-2.0

//! Presentation-neutral translation from owned input into editor intent.
//!
//! A grammar owns only interpretation state: prefixes, counts, and character
//! operands. Buffer state and presentation stay with the application, which
//! keeps Runyte and Vim input behavior separate from shared editor commands.

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
    VimMotion {
        motion: VimMotion,
        count: NonZeroUsize,
        explicit_count: bool,
        extend: bool,
    },
    VimOperator {
        operator: VimOperator,
        target: VimRangeTarget,
        register: Option<char>,
    },
    VimVisualOperator {
        operator: VimOperator,
        register: Option<char>,
    },
    VimVisualLine {
        count: NonZeroUsize,
    },
    VimSyntaxSelection {
        object: VimTextObject,
        around: bool,
    },
    VimReplace {
        character: char,
    },
    VimRepeatSearch {
        previous: bool,
        count: NonZeroUsize,
    },
    VimSearchWord {
        previous: bool,
        count: NonZeroUsize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimOperator {
    Delete,
    Change,
    Yank,
    Indent,
    Unindent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimMotion {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordBackward,
    WordEnd,
    WordEndBackward,
    LongWordForward,
    LongWordBackward,
    LongWordEnd,
    LongWordEndBackward,
    LineStart,
    FirstNonWhitespace,
    LastNonWhitespace,
    LineEnd,
    FileStart,
    FileEnd,
    FindNext(char),
    FindPrevious(char),
    TillNext(char),
    TillPrevious(char),
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    WindowTop,
    WindowCenter,
    WindowBottom,
    MatchBracket,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimTextObject {
    Function,
    Class,
    Parameter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VimRangeTarget {
    Characters {
        count: NonZeroUsize,
    },
    Motion {
        motion: VimMotion,
        count: NonZeroUsize,
    },
    Line {
        direction: LineDirection,
        count: NonZeroUsize,
    },
    Syntax {
        object: VimTextObject,
        around: bool,
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
    CharacterInputCancelled,
    ExpectedCharacter,
    InvalidRegister {
        register: char,
        macros_only: bool,
    },
    CountNotSupported(BindingTarget),
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
    awaiting_binding: Option<(KeySequence, BindingTarget)>,
}

/// Deliberately bounded Vim-compatible interpreter. It owns counts,
/// operator-pending state, character operands, registers, and macro prefixes;
/// buffer-dependent range resolution remains in App.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct VimGrammar {
    pending: KeySequence,
    count: Option<usize>,
    operator: Option<(VimOperator, usize, bool)>,
    awaiting: Option<VimAwaiting>,
    register: Option<char>,
    visual_line: bool,
    last_find: Option<(VimMotionKind, char)>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VimHint {
    pub key: KeyStroke,
    pub description: &'static str,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VimHelpRow {
    pub sequence: &'static str,
    pub description: &'static str,
}

#[cfg(test)]
const VIM_HELP_ROWS: &[VimHelpRow] = &[
    VimHelpRow {
        sequence: "h j k l",
        description: "character and display-line motions",
    },
    VimHelpRow {
        sequence: "w b e W B E",
        description: "word motions",
    },
    VimHelpRow {
        sequence: "0 ^ $ g_",
        description: "line motions",
    },
    VimHelpRow {
        sequence: "[count]gg / [count]G",
        description: "first, last, or addressed line",
    },
    VimHelpRow {
        sequence: "f F t T {char} ; ,",
        description: "find/till character and repeat",
    },
    VimHelpRow {
        sequence: "C-b C-f C-u C-d",
        description: "page and half-page motions",
    },
    VimHelpRow {
        sequence: "H M L / zt zz zb",
        description: "viewport motion and alignment",
    },
    VimHelpRow {
        sequence: "%",
        description: "matching delimiter",
    },
    VimHelpRow {
        sequence: "/ ? n N * #",
        description: "search, word search, and repeat",
    },
    VimHelpRow {
        sequence: "C-o C-i Tab",
        description: "jump backward or forward",
    },
    VimHelpRow {
        sequence: "i a I A o O",
        description: "enter Insert mode",
    },
    VimHelpRow {
        sequence: "v V o Esc",
        description: "character or line Visual mode, flip, leave",
    },
    VimHelpRow {
        sequence: "d c y + motion",
        description: "delete, change, or yank",
    },
    VimHelpRow {
        sequence: "dd cc yy",
        description: "linewise operator",
    },
    VimHelpRow {
        sequence: "x r u C-r",
        description: "delete, replace, undo, redo",
    },
    VimHelpRow {
        sequence: "p P",
        description: "paste after or before",
    },
    VimHelpRow {
        sequence: "af if ac ic ap ip",
        description: "syntax text objects",
    },
    VimHelpRow {
        sequence: "\"{register}",
        description: "select edit/paste register",
    },
    VimHelpRow {
        sequence: "q{a-z} / q",
        description: "record / stop macro",
    },
    VimHelpRow {
        sequence: "[count]@{a-z}",
        description: "replay macro",
    },
    VimHelpRow {
        sequence: "Space",
        description: "Runyte application commands",
    },
    VimHelpRow {
        sequence: "C-w",
        description: "window compatibility prefix",
    },
    VimHelpRow {
        sequence: ":",
        description: "command palette",
    },
];

#[cfg(test)]
fn vim_help_rows() -> &'static [VimHelpRow] {
    VIM_HELP_ROWS
}

#[cfg(test)]
const VIM_OPERATOR_HINTS: &[VimHint] = &[
    VimHint {
        key: KeyStroke::char('h'),
        description: "operate left",
    },
    VimHint {
        key: KeyStroke::char('l'),
        description: "operate right",
    },
    VimHint {
        key: KeyStroke::char('w'),
        description: "operate to next word",
    },
    VimHint {
        key: KeyStroke::char('b'),
        description: "operate to previous word",
    },
    VimHint {
        key: KeyStroke::char('e'),
        description: "operate through word end",
    },
    VimHint {
        key: KeyStroke::char('W'),
        description: "operate to next WORD",
    },
    VimHint {
        key: KeyStroke::char('B'),
        description: "operate to previous WORD",
    },
    VimHint {
        key: KeyStroke::char('E'),
        description: "operate through WORD end",
    },
    VimHint {
        key: KeyStroke::char('0'),
        description: "operate to line start",
    },
    VimHint {
        key: KeyStroke::char('^'),
        description: "operate to first non-whitespace",
    },
    VimHint {
        key: KeyStroke::char('$'),
        description: "operate through line end",
    },
    VimHint {
        key: KeyStroke::char('f'),
        description: "operate through next character",
    },
    VimHint {
        key: KeyStroke::char('F'),
        description: "operate through previous character",
    },
    VimHint {
        key: KeyStroke::char('t'),
        description: "operate until next character",
    },
    VimHint {
        key: KeyStroke::char('T'),
        description: "operate until previous character",
    },
    VimHint {
        key: KeyStroke::char(';'),
        description: "operate through repeated find",
    },
    VimHint {
        key: KeyStroke::char(','),
        description: "operate through reversed find",
    },
    VimHint {
        key: KeyStroke::char('%'),
        description: "operate through matching delimiter",
    },
    VimHint {
        key: KeyStroke::char('G'),
        description: "operate through last or addressed line",
    },
    VimHint {
        key: KeyStroke::char('g'),
        description: "operator g motions",
    },
    VimHint {
        key: KeyStroke::char('j'),
        description: "operate through next line",
    },
    VimHint {
        key: KeyStroke::char('k'),
        description: "operate through previous line",
    },
    VimHint {
        key: KeyStroke::char('i'),
        description: "inside syntax object",
    },
    VimHint {
        key: KeyStroke::char('a'),
        description: "around syntax object",
    },
];

#[cfg(test)]
const VIM_TEXT_OBJECT_HINTS: &[VimHint] = &[
    VimHint {
        key: KeyStroke::char('f'),
        description: "function",
    },
    VimHint {
        key: KeyStroke::char('c'),
        description: "class-like item",
    },
    VimHint {
        key: KeyStroke::char('p'),
        description: "parameter",
    },
];

#[cfg(test)]
const VIM_G_HINTS: &[VimHint] = &[
    VimHint {
        key: KeyStroke::char('g'),
        description: "go to first line or counted line",
    },
    VimHint {
        key: KeyStroke::char('e'),
        description: "previous word end",
    },
    VimHint {
        key: KeyStroke::char('E'),
        description: "previous WORD end",
    },
    VimHint {
        key: KeyStroke::char('_'),
        description: "last non-whitespace on line",
    },
    VimHint {
        key: KeyStroke::char('d'),
        description: "go to definition",
    },
    VimHint {
        key: KeyStroke::char('D'),
        description: "go to declaration",
    },
    VimHint {
        key: KeyStroke::char('y'),
        description: "go to type definition",
    },
    VimHint {
        key: KeyStroke::char('i'),
        description: "go to implementation",
    },
    VimHint {
        key: KeyStroke::char('r'),
        description: "go to references",
    },
];

#[cfg(test)]
const VIM_Z_HINTS: &[VimHint] = &[
    VimHint {
        key: KeyStroke::char('t'),
        description: "align current line at top",
    },
    VimHint {
        key: KeyStroke::char('z'),
        description: "align current line at center",
    },
    VimHint {
        key: KeyStroke::char('b'),
        description: "align current line at bottom",
    },
];

#[cfg(test)]
const VIM_D_HINT: &[VimHint] = &[VimHint {
    key: KeyStroke::char('d'),
    description: "delete whole line",
}];
#[cfg(test)]
const VIM_C_HINT: &[VimHint] = &[VimHint {
    key: KeyStroke::char('c'),
    description: "change whole line",
}];
#[cfg(test)]
const VIM_Y_HINT: &[VimHint] = &[VimHint {
    key: KeyStroke::char('y'),
    description: "yank whole line",
}];

/// Grammar-owned continuations for Vim-only state. Space and Ctrl-w keep
/// using the canonical registry; this table prevents Runyte rows from being
/// shown while a Vim operator or text object is pending.
#[cfg(test)]
fn vim_pending_hints(pending: &KeySequence) -> Vec<VimHint> {
    let keys = pending.as_slice();
    if keys
        .last()
        .is_some_and(|key| matches!(key.code, KeyCode::Char('i' | 'a')))
        && (keys.len() == 1
            || keys
                .first()
                .is_some_and(|key| matches!(key.code, KeyCode::Char('d' | 'c' | 'y'))))
    {
        VIM_TEXT_OBJECT_HINTS.to_vec()
    } else if keys
        .first()
        .is_some_and(|key| matches!(key.code, KeyCode::Char('d' | 'c' | 'y' | '>' | '<')))
    {
        let mut hints = VIM_OPERATOR_HINTS.to_vec();
        hints.extend_from_slice(match keys.first().map(|key| key.code) {
            Some(KeyCode::Char('d')) => VIM_D_HINT,
            Some(KeyCode::Char('c')) => VIM_C_HINT,
            Some(KeyCode::Char('y')) => VIM_Y_HINT,
            _ => &[],
        });
        hints
    } else if keys == [KeyStroke::char('g')] {
        VIM_G_HINTS.to_vec()
    } else if keys == [KeyStroke::char('z')] {
        VIM_Z_HINTS.to_vec()
    } else {
        Vec::new()
    }
}

/// Whether a displayed Vim continuation executes without another key.
#[cfg(test)]
fn vim_hint_is_exact(pending: &KeySequence, key: KeyStroke) -> bool {
    let operator_pending = pending.len() == 1
        && pending
            .as_slice()
            .first()
            .is_some_and(|key| matches!(key.code, KeyCode::Char('d' | 'c' | 'y' | '>' | '<')));
    !operator_pending
        || !matches!(
            key.code,
            KeyCode::Char('f' | 'F' | 't' | 'T' | 'g' | 'i' | 'a')
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(test)]
enum VimAwaiting {
    MotionCharacter(VimMotionKind, usize, bool),
    Replace(usize),
    Register,
    RecordMacro,
    ReplayMacro(usize),
    TextObject(bool),
    /// A character operand for a command reached through the shared keymap
    /// rather than through Vim's own vocabulary.
    NamespaceCharacter(EditorCommand, usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(test)]
enum VimMotionKind {
    FindNext,
    FindPrevious,
    TillNext,
    TillPrevious,
}

/// The active, owned input interpreter. Unsupported configured kinds are
/// rejected at selection time rather than represented as a pretend grammar.
#[derive(Clone, Debug)]
pub enum ActiveGrammar {
    Runyte(RunyteGrammar),
    #[cfg(test)]
    Vim(VimGrammar),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrammarUnavailable(pub GrammarKind);

impl fmt::Display for GrammarUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} grammar has been removed; use runyte", self.0)
    }
}

impl std::error::Error for GrammarUnavailable {}

impl Default for ActiveGrammar {
    fn default() -> Self {
        Self::Runyte(RunyteGrammar::default())
    }
}

impl ActiveGrammar {
    pub fn new(kind: GrammarKind) -> Result<Self, GrammarUnavailable> {
        match kind {
            GrammarKind::Runyte => Ok(Self::Runyte(RunyteGrammar::default())),
            #[cfg(test)]
            GrammarKind::Vim => Ok(Self::Vim(VimGrammar::default())),
            #[cfg(not(test))]
            GrammarKind::Vim => Err(GrammarUnavailable(kind)),
        }
    }

    pub const fn kind(&self) -> GrammarKind {
        match self {
            Self::Runyte(_) => GrammarKind::Runyte,
            #[cfg(test)]
            Self::Vim(_) => GrammarKind::Vim,
        }
    }

    pub const fn preferred_mode(&self) -> Option<Mode> {
        match self {
            Self::Runyte(_) => None,
            #[cfg(test)]
            Self::Vim(_) => None,
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
            BindingTarget::Colon(_) if self.count.take().is_some() => Ok(EditorIntent::Notice(
                GrammarNotice::CountNotSupported(target),
            )),
            BindingTarget::Colon(_) if availability.is_implemented() => {
                Ok(EditorIntent::Command(target.invocation()?))
            }
            BindingTarget::Colon(_) => {
                Ok(EditorIntent::Notice(GrammarNotice::UnavailableBinding {
                    target,
                    availability,
                }))
            }
        }
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
            .lookup_in(Mode::Insert, context.scope(), &candidate)
        {
            Lookup::Exact(binding) => {
                let target = binding.target;
                let availability = binding.availability;
                self.pending.clear();
                let intent = self.binding_intent(target, availability)?;
                if self.awaiting_character.is_some() {
                    self.awaiting_binding = Some((candidate, target));
                    Ok(GrammarOutput::one(intent))
                } else {
                    Ok(GrammarOutput {
                        intents: vec![intent],
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
                        .lookup_in(Mode::Insert, context.scope(), &prefix)
                    {
                        Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                            Some((binding.target, binding.availability))
                        }
                        Lookup::NoMatch | Lookup::Prefix(_) => None,
                    };
                if let Some((target, availability)) = fallback {
                    let intent = self.binding_intent(target, availability)?;
                    if self.awaiting_character.is_some() {
                        self.awaiting_binding = Some((prefix.clone(), target));
                    }
                    Ok(GrammarOutput {
                        intents: vec![intent],
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
            if key.code == KeyCode::Escape {
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
            let resolved_binding = self.awaiting_binding.take().map(|(mut sequence, target)| {
                sequence.push(key);
                (sequence, target)
            });
            let execution = CommandExecutionContext::resolved(
                NonZeroUsize::new(count).expect("character repetition is non-zero"),
                Some(character),
            );
            return Ok(GrammarOutput {
                intents: vec![EditorIntent::Command(CommandInvocation::editor(
                    command, execution,
                )?)],
                resolved_binding,
                ..GrammarOutput::default()
            });
        }

        if key.code == KeyCode::Escape && !self.pending.is_empty() {
            self.pending.clear();
            self.count = None;
            self.count_keys.clear();
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::SequenceCancelled,
            )));
        }
        if key.code == KeyCode::Backspace && !self.pending.is_empty() {
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
                let target = binding.target;
                let availability = binding.availability;
                let count_keys = std::mem::take(&mut self.count_keys);
                let resolved_sequence = Self::resolved_sequence(&count_keys, &candidate);
                let sticky = candidate
                    .as_slice()
                    .first()
                    .is_some_and(|key| *key == Key::char('Z'));
                self.pending.clear();
                let intent = self.binding_intent(target, availability)?;
                if self.awaiting_character.is_some() {
                    self.awaiting_binding = Some((resolved_sequence.clone(), target));
                }
                Ok(GrammarOutput {
                    intents: vec![intent],
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
                InputEvent::Text(_) | InputEvent::Pointer(_) => Ok(GrammarOutput::default()),
            },
            Mode::Command => Ok(GrammarOutput::default()),
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
        self.awaiting_binding = None;
    }
}

#[cfg(test)]
fn direct_binding_intent(
    target: BindingTarget,
    availability: BindingAvailability,
) -> Result<EditorIntent, CommandInvocationError> {
    match (target, availability) {
        (BindingTarget::Editor(command), BindingAvailability::Planned(reason)) => {
            Ok(EditorIntent::Command(
                CommandInvocation::unavailable_editor(command, CommandUnavailable::Planned(reason)),
            ))
        }
        (BindingTarget::Editor(command), BindingAvailability::Unsupported(reason)) => Ok(
            EditorIntent::Command(CommandInvocation::unavailable_editor(
                command,
                CommandUnavailable::Unsupported(reason),
            )),
        ),
        (_, BindingAvailability::Planned(_) | BindingAvailability::Unsupported(_)) => {
            Ok(EditorIntent::Notice(GrammarNotice::UnavailableBinding {
                target,
                availability,
            }))
        }
        (BindingTarget::Editor(EditorCommand::ShowHelp), BindingAvailability::Implemented) => Ok(
            EditorIntent::Command(CommandInvocation::help(HelpInvocation::ActiveView)),
        ),
        (_, BindingAvailability::Implemented) => Ok(EditorIntent::Command(target.invocation()?)),
    }
}

#[cfg(test)]
impl VimGrammar {
    fn command(
        command: EditorCommand,
        count: usize,
        character: Option<char>,
    ) -> Result<EditorIntent, CommandInvocationError> {
        let execution = if count == 1 && character.is_none() {
            CommandExecutionContext::default()
        } else {
            CommandExecutionContext::resolved(
                NonZeroUsize::new(count.max(1)).expect("Vim count is non-zero"),
                character,
            )
        };
        Ok(EditorIntent::Command(CommandInvocation::editor(
            command, execution,
        )?))
    }

    fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1).max(1)
    }

    fn take_count_with_explicit(&mut self) -> (usize, bool) {
        let explicit = self.count.is_some();
        (self.take_count(), explicit)
    }

    fn count_not_supported(command: EditorCommand) -> GrammarOutput {
        GrammarOutput::one(EditorIntent::Notice(GrammarNotice::CountNotSupported(
            BindingTarget::Editor(command),
        )))
    }

    fn command_with_register(
        &mut self,
        command: EditorCommand,
        count: usize,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        let mut intents = Vec::new();
        if let Some(register) = self.register.take() {
            intents.push(Self::command(
                EditorCommand::SelectRegister,
                1,
                Some(register),
            )?);
        }
        intents.push(Self::command(command, count, None)?);
        Ok(GrammarOutput {
            intents,
            ..GrammarOutput::default()
        })
    }

    fn push_count(&mut self, digit: char) -> GrammarOutput {
        let digit = digit.to_digit(10).expect("matched Vim count digit") as usize;
        let count = self
            .count
            .unwrap_or(0)
            .saturating_mul(10)
            .saturating_add(digit)
            .min(999_999);
        self.count = Some(count);
        GrammarOutput::one(EditorIntent::Notice(GrammarNotice::Count(count)))
    }

    fn count_product(left: usize, right: usize) -> NonZeroUsize {
        NonZeroUsize::new(left.saturating_mul(right).clamp(1, 999_999))
            .expect("clamped Vim count is non-zero")
    }

    fn motion_key(key: KeyStroke) -> Option<VimMotion> {
        if !key.modifiers.is_empty() {
            return match (key.code, key.modifiers) {
                (KeyCode::Char('b'), Modifiers::CONTROL) => Some(VimMotion::PageUp),
                (KeyCode::Char('f'), Modifiers::CONTROL) => Some(VimMotion::PageDown),
                (KeyCode::Char('u'), Modifiers::CONTROL) => Some(VimMotion::HalfPageUp),
                (KeyCode::Char('d'), Modifiers::CONTROL) => Some(VimMotion::HalfPageDown),
                _ => None,
            };
        }
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => Some(VimMotion::Left),
            KeyCode::Char('j') | KeyCode::Down => Some(VimMotion::Down),
            KeyCode::Char('k') | KeyCode::Up => Some(VimMotion::Up),
            KeyCode::Char('l') | KeyCode::Right => Some(VimMotion::Right),
            KeyCode::Char('w') => Some(VimMotion::WordForward),
            KeyCode::Char('b') => Some(VimMotion::WordBackward),
            KeyCode::Char('e') => Some(VimMotion::WordEnd),
            KeyCode::Char('W') => Some(VimMotion::LongWordForward),
            KeyCode::Char('B') => Some(VimMotion::LongWordBackward),
            KeyCode::Char('E') => Some(VimMotion::LongWordEnd),
            KeyCode::Char('0') | KeyCode::Home => Some(VimMotion::LineStart),
            KeyCode::Char('^') => Some(VimMotion::FirstNonWhitespace),
            KeyCode::Char('$') | KeyCode::End => Some(VimMotion::LineEnd),
            KeyCode::Char('G') => Some(VimMotion::FileEnd),
            KeyCode::Char('H') => Some(VimMotion::WindowTop),
            KeyCode::Char('M') => Some(VimMotion::WindowCenter),
            KeyCode::Char('L') => Some(VimMotion::WindowBottom),
            KeyCode::Char('%') => Some(VimMotion::MatchBracket),
            KeyCode::PageUp => Some(VimMotion::PageUp),
            KeyCode::PageDown => Some(VimMotion::PageDown),
            _ => None,
        }
    }

    fn find_motion(kind: VimMotionKind, character: char) -> VimMotion {
        match kind {
            VimMotionKind::FindNext => VimMotion::FindNext(character),
            VimMotionKind::FindPrevious => VimMotion::FindPrevious(character),
            VimMotionKind::TillNext => VimMotion::TillNext(character),
            VimMotionKind::TillPrevious => VimMotion::TillPrevious(character),
        }
    }

    fn repeated_find(&self, opposite: bool) -> Option<VimMotion> {
        let (kind, character) = self.last_find?;
        let kind = if opposite {
            match kind {
                VimMotionKind::FindNext => VimMotionKind::FindPrevious,
                VimMotionKind::FindPrevious => VimMotionKind::FindNext,
                VimMotionKind::TillNext => VimMotionKind::TillPrevious,
                VimMotionKind::TillPrevious => VimMotionKind::TillNext,
            }
        } else {
            kind
        };
        Some(Self::find_motion(kind, character))
    }

    fn syntax_object(character: char) -> Option<VimTextObject> {
        match character {
            'f' => Some(VimTextObject::Function),
            'c' => Some(VimTextObject::Class),
            'p' => Some(VimTextObject::Parameter),
            _ => None,
        }
    }

    fn operator_for(key: KeyStroke) -> Option<VimOperator> {
        if !key.modifiers.is_empty() {
            return None;
        }
        match key.code {
            KeyCode::Char('d') => Some(VimOperator::Delete),
            KeyCode::Char('c') => Some(VimOperator::Change),
            KeyCode::Char('y') => Some(VimOperator::Yank),
            KeyCode::Char('>') => Some(VimOperator::Indent),
            KeyCode::Char('<') => Some(VimOperator::Unindent),
            _ => None,
        }
    }

    fn begin_operator(&mut self, operator: VimOperator, key: KeyStroke) -> GrammarOutput {
        let (count, explicit_count) = self.take_count_with_explicit();
        self.operator = Some((operator, count, explicit_count));
        self.pending.clear();
        self.pending.push(key);
        GrammarOutput::one(EditorIntent::Notice(GrammarNotice::PendingSequence(
            self.pending.clone(),
        )))
    }

    fn finish_motion(&mut self, motion: VimMotion, extend: bool) -> GrammarOutput {
        let (motion_count, explicit_count) = self.take_count_with_explicit();
        self.pending.clear();
        if let Some((operator, operator_count, _)) = self.operator.take() {
            GrammarOutput::one(EditorIntent::Range(RangeIntent::VimOperator {
                operator,
                target: VimRangeTarget::Motion {
                    motion,
                    count: Self::count_product(operator_count, motion_count),
                },
                register: self.register.take(),
            }))
        } else {
            self.register = None;
            GrammarOutput::one(EditorIntent::Range(RangeIntent::VimMotion {
                motion,
                count: NonZeroUsize::new(motion_count).expect("Vim count is non-zero"),
                explicit_count,
                extend,
            }))
        }
    }

    fn finish_line_operator(
        &mut self,
        direction: LineDirection,
        includes_motion_destination: bool,
    ) -> GrammarOutput {
        let motion_count = self.take_count();
        let (operator, operator_count, _) = self.operator.take().expect("operator is pending");
        self.pending.clear();
        GrammarOutput::one(EditorIntent::Range(RangeIntent::VimOperator {
            operator,
            target: VimRangeTarget::Line {
                direction,
                count: NonZeroUsize::new(
                    Self::count_product(operator_count, motion_count)
                        .get()
                        .saturating_add(usize::from(includes_motion_destination)),
                )
                .expect("Vim line count is non-zero"),
            },
            register: self.register.take(),
        }))
    }

    fn translate_awaiting(
        &mut self,
        key: KeyStroke,
        mode: Mode,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        let awaiting = self.awaiting.take().expect("awaiting Vim operand");
        if key.code == KeyCode::Escape {
            self.reset();
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::CharacterInputCancelled,
            )));
        }
        let KeyCode::Char(character) = key.code else {
            self.reset();
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::ExpectedCharacter,
            )));
        };
        let output = match awaiting {
            VimAwaiting::MotionCharacter(kind, count, extend) => {
                self.count = Some(count);
                self.last_find = Some((kind, character));
                let motion = Self::find_motion(kind, character);
                self.finish_motion(motion, extend)
            }
            VimAwaiting::Replace(count) => {
                debug_assert_eq!(count, 1, "Vim replace counts are rejected before awaiting");
                GrammarOutput::one(EditorIntent::Range(RangeIntent::VimReplace { character }))
            }
            VimAwaiting::Register => {
                if !matches!(character, 'a'..='z' | 'A'..='Z' | '"' | '_') {
                    self.reset();
                    return Ok(GrammarOutput::one(EditorIntent::Notice(
                        GrammarNotice::InvalidRegister {
                            register: character,
                            macros_only: false,
                        },
                    )));
                }
                self.register = Some(character);
                GrammarOutput::default()
            }
            VimAwaiting::RecordMacro => {
                if !character.is_ascii_lowercase() {
                    self.reset();
                    return Ok(GrammarOutput::one(EditorIntent::Notice(
                        GrammarNotice::InvalidRegister {
                            register: character,
                            macros_only: true,
                        },
                    )));
                }
                GrammarOutput::one(Self::command(
                    EditorCommand::RecordMacro,
                    1,
                    Some(character),
                )?)
            }
            VimAwaiting::ReplayMacro(count) => {
                if !character.is_ascii_lowercase() {
                    self.reset();
                    return Ok(GrammarOutput::one(EditorIntent::Notice(
                        GrammarNotice::InvalidRegister {
                            register: character,
                            macros_only: true,
                        },
                    )));
                }
                GrammarOutput::one(Self::command(
                    EditorCommand::ReplayMacro,
                    count,
                    Some(character),
                )?)
            }
            VimAwaiting::NamespaceCharacter(command, count) => {
                GrammarOutput::one(Self::command(command, count, Some(character))?)
            }
            VimAwaiting::TextObject(around) => {
                let Some(object) = Self::syntax_object(character) else {
                    self.reset();
                    return Ok(GrammarOutput::one(EditorIntent::Notice(
                        GrammarNotice::NoBinding(KeySequence::from(key)),
                    )));
                };
                self.pending.clear();
                if let Some((operator, _, operator_count_explicit)) = self.operator.take() {
                    let (_, object_count_explicit) = self.take_count_with_explicit();
                    if operator_count_explicit || object_count_explicit {
                        self.reset();
                        return Ok(GrammarOutput::one(EditorIntent::Notice(
                            GrammarNotice::CountNotSupported(BindingTarget::Editor(match object {
                                VimTextObject::Function => EditorCommand::SelectSyntaxFunction,
                                VimTextObject::Class => EditorCommand::SelectSyntaxClass,
                                VimTextObject::Parameter => EditorCommand::SelectSyntaxParameter,
                            })),
                        )));
                    }
                    GrammarOutput::one(EditorIntent::Range(RangeIntent::VimOperator {
                        operator,
                        target: VimRangeTarget::Syntax { object, around },
                        register: self.register.take(),
                    }))
                } else if mode == Mode::Select {
                    let command = match (object, around) {
                        (VimTextObject::Function, true) => EditorCommand::SelectSyntaxFunction,
                        (VimTextObject::Function, false) => {
                            EditorCommand::SelectInsideSyntaxFunction
                        }
                        (VimTextObject::Class, true) => EditorCommand::SelectSyntaxClass,
                        (VimTextObject::Class, false) => EditorCommand::SelectInsideSyntaxClass,
                        (VimTextObject::Parameter, true) => EditorCommand::SelectSyntaxParameter,
                        (VimTextObject::Parameter, false) => {
                            EditorCommand::SelectInsideSyntaxParameter
                        }
                    };
                    let (_, explicit_count) = self.take_count_with_explicit();
                    if explicit_count {
                        self.reset();
                        return Ok(GrammarOutput::one(EditorIntent::Notice(
                            GrammarNotice::CountNotSupported(BindingTarget::Editor(command)),
                        )));
                    }
                    self.register = None;
                    GrammarOutput::one(EditorIntent::Range(RangeIntent::VimSyntaxSelection {
                        object,
                        around,
                    }))
                } else {
                    GrammarOutput::default()
                }
            }
        };
        Ok(output)
    }

    fn translate_namespace(
        &mut self,
        key: KeyStroke,
        context: GrammarContext<'_>,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        if key.code == KeyCode::Escape {
            self.reset();
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::SequenceCancelled,
            )));
        }
        let mut candidate = self.pending.clone();
        candidate.push(key);
        let mut canonical = candidate.clone();
        if canonical.as_slice().first() == Some(&KeyStroke::ctrl('w')) {
            canonical = KeySequence::new(
                [KeyStroke::char(' '), KeyStroke::char('w')]
                    .into_iter()
                    .chain(candidate.as_slice().iter().skip(1).copied()),
            );
        }
        match context
            .keymap()
            .lookup_in(Mode::Normal, context.scope(), &canonical)
        {
            Lookup::NoMatch => {
                self.reset();
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
                self.pending.clear();
                // A namespace can end on a command that names its operand with
                // the next key, such as the macro registers. Vim waits for that
                // key here rather than dispatching an invocation the command
                // inventory would reject as incomplete.
                if let BindingTarget::Editor(command) = binding.target
                    && command.takes_character()
                    && binding.availability == BindingAvailability::Implemented
                {
                    let count = self
                        .count
                        .take()
                        .filter(|count| *count > 1 && command.accepts_count())
                        .unwrap_or(1);
                    self.awaiting = Some(VimAwaiting::NamespaceCharacter(command, count));
                    return Ok(GrammarOutput::one(EditorIntent::Notice(
                        GrammarNotice::AwaitingCharacter(command),
                    )));
                }
                self.count = None;
                Ok(GrammarOutput {
                    intents: vec![direct_binding_intent(binding.target, binding.availability)?],
                    ..GrammarOutput::default()
                })
            }
        }
    }

    /// A view-local exact binding outranks Vim's global interpretation. This
    /// keeps directory and other specialized buffers operable without making
    /// their keys part of the Vim grammar itself. Global Runyte bindings are
    /// deliberately ignored here and remain available only through Vim's
    /// explicit Space and Ctrl-w delegation paths.
    fn translate_scoped_exact(
        &mut self,
        key: KeyStroke,
        context: GrammarContext<'_>,
    ) -> Result<Option<GrammarOutput>, CommandInvocationError> {
        if context.scope() == BindingScope::Global {
            return Ok(None);
        }
        // Directory `r` conflicts with Vim's replace-character command. Keep
        // the established Vim meaning until the user chooses which scoped
        // behavior should win.
        if context.scope() == BindingScope::Directory && key == KeyStroke::char('r') {
            return Ok(None);
        }
        let sequence = KeySequence::from(key);
        let binding = match context
            .keymap()
            .lookup_in(context.mode(), context.scope(), &sequence)
        {
            Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. }
                if binding.scope != BindingScope::Global =>
            {
                binding
            }
            Lookup::NoMatch
            | Lookup::Prefix(_)
            | Lookup::Exact(_)
            | Lookup::ExactAndPrefix { .. } => return Ok(None),
        };
        if self.count.take().is_some() {
            self.register = None;
            return Ok(Some(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::CountNotSupported(binding.target),
            ))));
        }
        self.register = None;
        Ok(Some(GrammarOutput::one(direct_binding_intent(
            binding.target,
            binding.availability,
        )?)))
    }

    fn translate_operator(
        &mut self,
        key: KeyStroke,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        if key.code == KeyCode::Escape {
            self.reset();
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::SequenceCancelled,
            )));
        }
        if key.modifiers.is_empty()
            && let KeyCode::Char(digit @ '0'..='9') = key.code
            && (digit != '0' || self.count.is_some())
        {
            return Ok(self.push_count(digit));
        }
        if self.pending.as_slice().last() == Some(&KeyStroke::char('g')) {
            let motion = match (key.code, key.modifiers) {
                (KeyCode::Char('g'), modifiers) if modifiers.is_empty() => VimMotion::FileStart,
                (KeyCode::Char('e'), modifiers) if modifiers.is_empty() => {
                    VimMotion::WordEndBackward
                }
                (KeyCode::Char('E'), modifiers) if modifiers.is_empty() => {
                    VimMotion::LongWordEndBackward
                }
                (KeyCode::Char('_'), modifiers) if modifiers.is_empty() => {
                    VimMotion::LastNonWhitespace
                }
                _ => {
                    let mut sequence = std::mem::take(&mut self.pending);
                    sequence.push(key);
                    self.operator = None;
                    self.count = None;
                    self.register = None;
                    return Ok(GrammarOutput::one(EditorIntent::Notice(
                        GrammarNotice::NoBinding(sequence),
                    )));
                }
            };
            return Ok(self.finish_motion(motion, false));
        }
        let (operator, _, _) = self.operator.expect("operator is pending");
        if Self::operator_for(key) == Some(operator) {
            return Ok(self.finish_line_operator(LineDirection::Down, false));
        }
        if key.modifiers.is_empty()
            && let KeyCode::Char(character @ ('i' | 'a')) = key.code
        {
            self.pending.push(key);
            self.awaiting = Some(VimAwaiting::TextObject(character == 'a'));
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::PendingSequence(self.pending.clone()),
            )));
        }
        if matches!(key.code, KeyCode::Char('j')) && key.modifiers.is_empty() {
            return Ok(self.finish_line_operator(LineDirection::Down, true));
        }
        if matches!(key.code, KeyCode::Char('k')) && key.modifiers.is_empty() {
            return Ok(self.finish_line_operator(LineDirection::Up, true));
        }
        if key == KeyStroke::char('g') {
            self.pending.push(key);
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::PendingSequence(self.pending.clone()),
            )));
        }
        if key.modifiers.is_empty()
            && let KeyCode::Char(character @ ('f' | 'F' | 't' | 'T')) = key.code
        {
            let kind = match character {
                'f' => VimMotionKind::FindNext,
                'F' => VimMotionKind::FindPrevious,
                't' => VimMotionKind::TillNext,
                'T' => VimMotionKind::TillPrevious,
                _ => unreachable!(),
            };
            self.awaiting = Some(VimAwaiting::MotionCharacter(kind, self.take_count(), false));
            self.pending.push(key);
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::AwaitingCharacter(EditorCommand::FindNextChar),
            )));
        }
        if (key == KeyStroke::char(';') || key == KeyStroke::char(','))
            && let Some(motion) = self.repeated_find(key == KeyStroke::char(','))
        {
            return Ok(self.finish_motion(motion, false));
        }
        if let Some(motion) = Self::motion_key(key)
            && matches!(
                motion,
                VimMotion::Left
                    | VimMotion::Right
                    | VimMotion::WordForward
                    | VimMotion::WordBackward
                    | VimMotion::WordEnd
                    | VimMotion::LongWordForward
                    | VimMotion::LongWordBackward
                    | VimMotion::LongWordEnd
                    | VimMotion::WordEndBackward
                    | VimMotion::LongWordEndBackward
                    | VimMotion::LineStart
                    | VimMotion::FirstNonWhitespace
                    | VimMotion::LineEnd
                    | VimMotion::LastNonWhitespace
                    | VimMotion::FileEnd
                    | VimMotion::MatchBracket
            )
        {
            return Ok(self.finish_motion(motion, false));
        }
        let mut sequence = std::mem::take(&mut self.pending);
        sequence.push(key);
        self.operator = None;
        self.count = None;
        self.register = None;
        Ok(GrammarOutput::one(EditorIntent::Notice(
            GrammarNotice::NoBinding(sequence),
        )))
    }

    fn translate_modal(
        &mut self,
        key: KeyStroke,
        context: GrammarContext<'_>,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        // Terminal protocols normally encode an uppercase character in both
        // the character and the Shift modifier. Vim's modal vocabulary cares
        // about the character (`G` versus `g`), not the redundant Shift bit.
        // Insert mode intentionally keeps the raw key so typed text is
        // unchanged.
        let key = key.canonical_for_binding();
        if self.awaiting.is_some() {
            return self.translate_awaiting(key, context.mode());
        }
        if context.recording_macro()
            && context.mode() != Mode::Insert
            && key == KeyStroke::char('q')
        {
            self.reset();
            return Ok(GrammarOutput::one(Self::command(
                EditorCommand::StopMacroRecording,
                1,
                None,
            )?));
        }
        if self.operator.is_some() {
            return self.translate_operator(key);
        }
        if !self.pending.is_empty() {
            if self.pending == KeySequence::from(KeyStroke::char('g')) {
                let extend = context.mode() == Mode::Select;
                let motion = match key {
                    key if key == KeyStroke::char('g') => Some(VimMotion::FileStart),
                    key if key == KeyStroke::char('e') => Some(VimMotion::WordEndBackward),
                    key if key == KeyStroke::char('E') => Some(VimMotion::LongWordEndBackward),
                    key if key == KeyStroke::char('_') => Some(VimMotion::LastNonWhitespace),
                    _ => None,
                };
                if let Some(motion) = motion {
                    return Ok(self.finish_motion(motion, extend));
                }
                let command = match key {
                    key if key == KeyStroke::char('d') => Some(EditorCommand::GotoDefinition),
                    key if key == KeyStroke::char('D') => Some(EditorCommand::GotoDeclaration),
                    key if key == KeyStroke::char('y') => Some(EditorCommand::GotoTypeDefinition),
                    key if key == KeyStroke::char('i') => Some(EditorCommand::GotoImplementation),
                    key if key == KeyStroke::char('r') => Some(EditorCommand::GotoReferences),
                    _ => None,
                };
                if let Some(command) = command {
                    self.pending.clear();
                    self.register = None;
                    if self.count.take().is_some() {
                        return Ok(Self::count_not_supported(command));
                    }
                    return Ok(GrammarOutput::one(Self::command(command, 1, None)?));
                }
                let mut sequence = std::mem::take(&mut self.pending);
                sequence.push(key);
                self.count = None;
                self.register = None;
                return Ok(GrammarOutput::one(EditorIntent::Notice(
                    GrammarNotice::NoBinding(sequence),
                )));
            }
            if self.pending == KeySequence::from(KeyStroke::char('z')) {
                let command = match key {
                    key if key == KeyStroke::char('t') => Some(EditorCommand::AlignViewTop),
                    key if key == KeyStroke::char('z') => Some(EditorCommand::AlignViewCenter),
                    key if key == KeyStroke::char('b') => Some(EditorCommand::AlignViewBottom),
                    _ => None,
                };
                self.pending.clear();
                self.register = None;
                let Some(command) = command else {
                    self.count = None;
                    return Ok(GrammarOutput::one(EditorIntent::Notice(
                        GrammarNotice::NoBinding(KeySequence::new([KeyStroke::char('z'), key])),
                    )));
                };
                if self.count.take().is_some() {
                    return Ok(Self::count_not_supported(command));
                }
                return Ok(GrammarOutput::one(Self::command(command, 1, None)?));
            }
            return self.translate_namespace(key, context);
        }
        if let Some(output) = self.translate_scoped_exact(key, context)? {
            return Ok(output);
        }
        if key.code == KeyCode::Escape {
            self.reset();
            return Ok(GrammarOutput::one(Self::command(
                EditorCommand::EnterNormalMode,
                1,
                None,
            )?));
        }
        if key.modifiers.is_empty()
            && let KeyCode::Char(digit @ '0'..='9') = key.code
            && (digit != '0' || self.count.is_some())
        {
            return Ok(self.push_count(digit));
        }
        let extend = context.mode() == Mode::Select;
        if key == KeyStroke::char('g') {
            self.register = None;
            self.pending.push(key);
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::PendingSequence(self.pending.clone()),
            )));
        }
        if key == KeyStroke::char('z') {
            self.register = None;
            self.pending.push(key);
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::PendingSequence(self.pending.clone()),
            )));
        }
        if key == KeyStroke::char(' ') || key == KeyStroke::ctrl('w') {
            self.count = None;
            self.register = None;
            self.pending.push(key);
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::PendingSequence(self.pending.clone()),
            )));
        }
        if let Some(operator) = Self::operator_for(key) {
            if extend {
                self.count = None;
                return Ok(GrammarOutput::one(EditorIntent::Range(
                    RangeIntent::VimVisualOperator {
                        operator,
                        register: self.register.take(),
                    },
                )));
            }
            if matches!(
                operator,
                VimOperator::Delete | VimOperator::Change | VimOperator::Yank
            ) {
                return Ok(self.begin_operator(operator, key));
            }
            self.count = None;
            self.register = None;
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::NoBinding(KeySequence::from(key)),
            )));
        }
        if key.modifiers.is_empty()
            && let KeyCode::Char(character @ ('f' | 'F' | 't' | 'T')) = key.code
        {
            let kind = match character {
                'f' => VimMotionKind::FindNext,
                'F' => VimMotionKind::FindPrevious,
                't' => VimMotionKind::TillNext,
                'T' => VimMotionKind::TillPrevious,
                _ => unreachable!(),
            };
            self.awaiting = Some(VimAwaiting::MotionCharacter(
                kind,
                self.take_count(),
                extend,
            ));
            self.pending.push(key);
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::AwaitingCharacter(EditorCommand::FindNextChar),
            )));
        }
        if extend
            && key.modifiers.is_empty()
            && let KeyCode::Char(character @ ('i' | 'a')) = key.code
        {
            self.pending.push(key);
            self.awaiting = Some(VimAwaiting::TextObject(character == 'a'));
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::PendingSequence(self.pending.clone()),
            )));
        }
        if let Some(motion) = Self::motion_key(key) {
            return Ok(self.finish_motion(motion, extend));
        }
        if key == KeyStroke::char(';') || key == KeyStroke::char(',') {
            if let Some(motion) = self.repeated_find(key == KeyStroke::char(',')) {
                return Ok(self.finish_motion(motion, extend));
            }
            self.count = None;
            self.register = None;
            return Ok(GrammarOutput::one(EditorIntent::Notice(
                GrammarNotice::NoBinding(KeySequence::from(key)),
            )));
        }

        let (count, explicit_count) = self.take_count_with_explicit();
        let output = match (key.code, key.modifiers, context.mode()) {
            (KeyCode::Char('i'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.register = None;
                if explicit_count {
                    Self::count_not_supported(EditorCommand::EnterInsertMode)
                } else {
                    GrammarOutput::one(Self::command(EditorCommand::EnterInsertMode, 1, None)?)
                }
            }
            (KeyCode::Char('a'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.register = None;
                if explicit_count {
                    Self::count_not_supported(EditorCommand::AppendAfter)
                } else {
                    GrammarOutput::one(Self::command(EditorCommand::AppendAfter, 1, None)?)
                }
            }
            (KeyCode::Char('I'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.register = None;
                if explicit_count {
                    Self::count_not_supported(EditorCommand::InsertLineStart)
                } else {
                    GrammarOutput {
                        intents: vec![
                            EditorIntent::Range(RangeIntent::VimMotion {
                                motion: VimMotion::FirstNonWhitespace,
                                count: NonZeroUsize::MIN,
                                explicit_count: false,
                                extend: false,
                            }),
                            Self::command(EditorCommand::EnterInsertMode, 1, None)?,
                        ],
                        ..GrammarOutput::default()
                    }
                }
            }
            (KeyCode::Char('A'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.register = None;
                if explicit_count {
                    Self::count_not_supported(EditorCommand::InsertLineEnd)
                } else {
                    GrammarOutput::one(Self::command(EditorCommand::InsertLineEnd, 1, None)?)
                }
            }
            (KeyCode::Char('o'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.register = None;
                if explicit_count {
                    Self::count_not_supported(EditorCommand::OpenLineBelow)
                } else {
                    GrammarOutput::one(Self::command(EditorCommand::OpenLineBelow, 1, None)?)
                }
            }
            (KeyCode::Char('O'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.register = None;
                if explicit_count {
                    Self::count_not_supported(EditorCommand::OpenLineAbove)
                } else {
                    GrammarOutput::one(Self::command(EditorCommand::OpenLineAbove, 1, None)?)
                }
            }
            (KeyCode::Char('V'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.visual_line = true;
                GrammarOutput::one(EditorIntent::Range(RangeIntent::VimVisualLine {
                    count: NonZeroUsize::new(count).expect("Vim count is non-zero"),
                }))
            }
            (KeyCode::Char('V'), modifiers, Mode::Select) if modifiers.is_empty() => {
                if self.visual_line {
                    self.visual_line = false;
                    GrammarOutput::one(Self::command(EditorCommand::EnterNormalMode, 1, None)?)
                } else {
                    self.visual_line = true;
                    GrammarOutput::one(EditorIntent::Range(RangeIntent::VimVisualLine {
                        count: NonZeroUsize::new(count).expect("Vim count is non-zero"),
                    }))
                }
            }
            (KeyCode::Char('v'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.visual_line = false;
                GrammarOutput::one(Self::command(EditorCommand::EnterSelectMode, 1, None)?)
            }
            (KeyCode::Char('v'), modifiers, Mode::Select) if modifiers.is_empty() => {
                self.visual_line = false;
                GrammarOutput::one(Self::command(EditorCommand::EnterNormalMode, 1, None)?)
            }
            (KeyCode::Char('o'), modifiers, Mode::Select) if modifiers.is_empty() => {
                GrammarOutput::one(Self::command(EditorCommand::FlipSelection, 1, None)?)
            }
            (KeyCode::Char('x'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                GrammarOutput::one(EditorIntent::Range(RangeIntent::VimOperator {
                    operator: VimOperator::Delete,
                    target: VimRangeTarget::Characters {
                        count: NonZeroUsize::new(count).expect("Vim count is non-zero"),
                    },
                    register: self.register.take(),
                }))
            }
            (KeyCode::Char('x'), modifiers, Mode::Select) if modifiers.is_empty() => {
                GrammarOutput::one(EditorIntent::Range(RangeIntent::VimVisualOperator {
                    operator: VimOperator::Delete,
                    register: self.register.take(),
                }))
            }
            (KeyCode::Char('r'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.register = None;
                if explicit_count {
                    Self::count_not_supported(EditorCommand::ReplaceChar)
                } else {
                    self.awaiting = Some(VimAwaiting::Replace(count));
                    GrammarOutput::one(EditorIntent::Notice(GrammarNotice::AwaitingCharacter(
                        EditorCommand::ReplaceChar,
                    )))
                }
            }
            (KeyCode::Char('u'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                GrammarOutput::one(Self::command(EditorCommand::Undo, count, None)?)
            }
            (KeyCode::Char('r'), Modifiers::CONTROL, Mode::Normal) => {
                GrammarOutput::one(Self::command(EditorCommand::Redo, count, None)?)
            }
            (KeyCode::Char('p'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.command_with_register(EditorCommand::PasteAfter, count)?
            }
            (KeyCode::Char('P'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.command_with_register(EditorCommand::PasteBefore, count)?
            }
            (KeyCode::Char('/'), modifiers, _) if modifiers.is_empty() => {
                GrammarOutput::one(Self::command(EditorCommand::SearchForward, 1, None)?)
            }
            (KeyCode::Char('?'), modifiers, _) if modifiers.is_empty() => {
                GrammarOutput::one(Self::command(EditorCommand::SearchBackward, 1, None)?)
            }
            (KeyCode::Char('n'), modifiers, _) if modifiers.is_empty() => {
                GrammarOutput::one(EditorIntent::Range(RangeIntent::VimRepeatSearch {
                    previous: false,
                    count: NonZeroUsize::new(count).expect("Vim count is non-zero"),
                }))
            }
            (KeyCode::Char('N'), modifiers, _) if modifiers.is_empty() => {
                GrammarOutput::one(EditorIntent::Range(RangeIntent::VimRepeatSearch {
                    previous: true,
                    count: NonZeroUsize::new(count).expect("Vim count is non-zero"),
                }))
            }
            (KeyCode::Char('*'), modifiers, _) if modifiers.is_empty() => {
                GrammarOutput::one(EditorIntent::Range(RangeIntent::VimSearchWord {
                    previous: false,
                    count: NonZeroUsize::new(count).expect("Vim count is non-zero"),
                }))
            }
            (KeyCode::Char('#'), modifiers, _) if modifiers.is_empty() => {
                GrammarOutput::one(EditorIntent::Range(RangeIntent::VimSearchWord {
                    previous: true,
                    count: NonZeroUsize::new(count).expect("Vim count is non-zero"),
                }))
            }
            (KeyCode::Char('o'), Modifiers::CONTROL, _) => {
                GrammarOutput::one(Self::command(EditorCommand::JumpBackward, count, None)?)
            }
            (KeyCode::Char('i'), Modifiers::CONTROL, _) | (KeyCode::Tab, Modifiers::NONE, _) => {
                GrammarOutput::one(Self::command(EditorCommand::JumpForward, count, None)?)
            }
            (KeyCode::Char(':'), modifiers, _) if modifiers.is_empty() => {
                GrammarOutput::one(Self::command(EditorCommand::OpenCommandPalette, 1, None)?)
            }
            (KeyCode::Char('"'), modifiers, _) if modifiers.is_empty() => {
                if explicit_count {
                    self.register = None;
                    Self::count_not_supported(EditorCommand::SelectRegister)
                } else {
                    self.awaiting = Some(VimAwaiting::Register);
                    GrammarOutput::one(EditorIntent::Notice(GrammarNotice::AwaitingCharacter(
                        EditorCommand::SelectRegister,
                    )))
                }
            }
            (KeyCode::Char('q'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.register = None;
                if explicit_count {
                    Self::count_not_supported(EditorCommand::RecordMacro)
                } else {
                    self.awaiting = Some(VimAwaiting::RecordMacro);
                    GrammarOutput::one(EditorIntent::Notice(GrammarNotice::AwaitingCharacter(
                        EditorCommand::RecordMacro,
                    )))
                }
            }
            (KeyCode::Char('@'), modifiers, Mode::Normal) if modifiers.is_empty() => {
                self.awaiting = Some(VimAwaiting::ReplayMacro(count));
                GrammarOutput::one(EditorIntent::Notice(GrammarNotice::AwaitingCharacter(
                    EditorCommand::ReplayMacro,
                )))
            }
            _ => GrammarOutput::default(),
        };
        self.register = None;
        Ok(output)
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
            .lookup_in(Mode::Insert, context.scope(), &candidate)
        {
            Lookup::Exact(binding) => {
                self.pending.clear();
                Ok(GrammarOutput::one(direct_binding_intent(
                    binding.target,
                    binding.availability,
                )?))
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
                        .lookup_in(Mode::Insert, context.scope(), &prefix)
                    {
                        Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => {
                            Some((binding.target, binding.availability))
                        }
                        Lookup::NoMatch | Lookup::Prefix(_) => None,
                    };
                if let Some((target, availability)) = fallback {
                    Ok(GrammarOutput {
                        intents: vec![direct_binding_intent(target, availability)?],
                        reprocess: Some(InputEvent::Key(key)),
                        ..GrammarOutput::default()
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
}

#[cfg(test)]
impl InputGrammar for VimGrammar {
    fn translate(
        &mut self,
        input: InputEvent,
        context: GrammarContext<'_>,
    ) -> Result<GrammarOutput, CommandInvocationError> {
        match context.mode() {
            Mode::Insert | Mode::Replace => self.translate_insert(input, context),
            Mode::Normal | Mode::Select => match input {
                InputEvent::Key(key) => self.translate_modal(key, context),
                InputEvent::Text(_) | InputEvent::Pointer(_) => Ok(GrammarOutput::default()),
            },
            Mode::Command => Ok(GrammarOutput::default()),
        }
    }

    fn complete(&mut self, _action: GrammarPostAction, resulting_mode: Mode) {
        if resulting_mode != Mode::Select {
            self.visual_line = false;
        }
    }

    fn pending_sequence(&self) -> &KeySequence {
        &self.pending
    }

    fn pending_count(&self) -> Option<usize> {
        self.count
    }

    fn awaiting_character(&self) -> Option<EditorCommand> {
        self.awaiting.and_then(|awaiting| match awaiting {
            VimAwaiting::NamespaceCharacter(command, _) => Some(command),
            VimAwaiting::Replace(_) => Some(EditorCommand::ReplaceChar),
            VimAwaiting::MotionCharacter(_, _, _)
            | VimAwaiting::Register
            | VimAwaiting::RecordMacro
            | VimAwaiting::ReplayMacro(_) => None,
            VimAwaiting::TextObject(_) => None,
        })
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.count = None;
        self.operator = None;
        self.awaiting = None;
        self.register = None;
        self.visual_line = false;
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
            #[cfg(test)]
            Self::Vim(grammar) => grammar.translate(input, context),
        }
    }

    fn complete(&mut self, action: GrammarPostAction, resulting_mode: Mode) {
        match self {
            Self::Runyte(grammar) => grammar.complete(action, resulting_mode),
            #[cfg(test)]
            Self::Vim(grammar) => grammar.complete(action, resulting_mode),
        }
    }

    fn pending_sequence(&self) -> &KeySequence {
        match self {
            Self::Runyte(grammar) => grammar.pending_sequence(),
            #[cfg(test)]
            Self::Vim(grammar) => grammar.pending_sequence(),
        }
    }

    fn pending_count(&self) -> Option<usize> {
        match self {
            Self::Runyte(grammar) => grammar.pending_count(),
            #[cfg(test)]
            Self::Vim(grammar) => grammar.pending_count(),
        }
    }

    fn awaiting_character(&self) -> Option<EditorCommand> {
        match self {
            Self::Runyte(grammar) => grammar.awaiting_character(),
            #[cfg(test)]
            Self::Vim(grammar) => grammar.awaiting_character(),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Runyte(grammar) => grammar.reset(),
            #[cfg(test)]
            Self::Vim(grammar) => grammar.reset(),
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

    fn translate_vim(grammar: &mut VimGrammar, mode: Mode, key: KeyStroke) -> GrammarOutput {
        translate_vim_in(grammar, mode, BindingScope::Global, key)
    }

    fn translate_vim_in(
        grammar: &mut VimGrammar,
        mode: Mode,
        scope: BindingScope,
        key: KeyStroke,
    ) -> GrammarOutput {
        grammar
            .translate(InputEvent::Key(key), context(mode, scope))
            .unwrap()
    }

    fn translate_vim_recording(
        grammar: &mut VimGrammar,
        mode: Mode,
        key: KeyStroke,
    ) -> GrammarOutput {
        grammar
            .translate(
                InputEvent::Key(key),
                context(mode, BindingScope::Global).with_recording_macro(true),
            )
            .unwrap()
    }

    fn only_intent(output: GrammarOutput) -> EditorIntent {
        assert!(output.reprocess.is_none());
        assert_eq!(output.intents.len(), 1);
        output.intents.into_iter().next().unwrap()
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
            let mut grammar = ActiveGrammar::new(*kind).unwrap();
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
            EditorIntent::Range(RangeIntent::SelectLine {
                direction: LineDirection::Up,
                count: NonZeroUsize::new(3).unwrap(),
            })
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
    fn colon_binding_emits_typed_invocation_and_rejects_runyte_count_prefix() {
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
            EditorIntent::Notice(GrammarNotice::CountNotSupported(BindingTarget::Colon(
                crate::command::ColonCommand::LspStatus
            )))
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
    fn active_grammar_selects_every_implemented_kind() {
        assert_eq!(
            ActiveGrammar::new(GrammarKind::Vim).unwrap().kind(),
            GrammarKind::Vim
        );
    }

    #[test]
    fn vim_operator_state_multiplies_counts_and_accepts_core_compositions() {
        let mut grammar = VimGrammar::default();
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('2'));
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('d'));
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('3'));
        let intent = only_intent(translate_vim(
            &mut grammar,
            Mode::Normal,
            KeyStroke::char('w'),
        ));
        assert_eq!(
            intent,
            EditorIntent::Range(RangeIntent::VimOperator {
                operator: VimOperator::Delete,
                target: VimRangeTarget::Motion {
                    motion: VimMotion::WordForward,
                    count: NonZeroUsize::new(6).unwrap(),
                },
                register: None,
            })
        );

        let mut find = VimGrammar::default();
        translate_vim(&mut find, Mode::Normal, KeyStroke::char('d'));
        assert!(matches!(
            only_intent(translate_vim(&mut find, Mode::Normal, KeyStroke::char('f'))),
            EditorIntent::Notice(GrammarNotice::AwaitingCharacter(_))
        ));
        assert!(matches!(
            only_intent(translate_vim(&mut find, Mode::Normal, KeyStroke::char('x'))),
            EditorIntent::Range(RangeIntent::VimOperator {
                target: VimRangeTarget::Motion {
                    motion: VimMotion::FindNext('x'),
                    ..
                },
                ..
            })
        ));

        for sequence in [
            &['d', '%'][..],
            &['d', 'G'][..],
            &['d', 'g', 'g'][..],
            &['d', 'g', 'e'][..],
        ] {
            let mut grammar = VimGrammar::default();
            let mut output = GrammarOutput::default();
            for key in sequence {
                output = translate_vim(&mut grammar, Mode::Normal, KeyStroke::char(*key));
            }
            assert!(matches!(
                only_intent(output),
                EditorIntent::Range(RangeIntent::VimOperator { .. })
            ));
        }

        let mut unsupported = VimGrammar::default();
        assert!(matches!(
            only_intent(translate_vim(
                &mut unsupported,
                Mode::Normal,
                KeyStroke::char('>')
            )),
            EditorIntent::Notice(GrammarNotice::NoBinding(_))
        ));
    }

    #[test]
    fn vim_g_motions_and_semantic_routes_preserve_typed_identities() {
        for (keys, motion, count, explicit_count) in [
            (&['G'][..], VimMotion::FileEnd, 1, false),
            (&['1', 'G'][..], VimMotion::FileEnd, 1, true),
            (&['g', 'g'][..], VimMotion::FileStart, 1, false),
            (&['1', 'g', 'g'][..], VimMotion::FileStart, 1, true),
        ] {
            let mut grammar = VimGrammar::default();
            let mut output = GrammarOutput::default();
            for key in keys {
                output = translate_vim(&mut grammar, Mode::Normal, KeyStroke::char(*key));
            }
            assert_eq!(
                only_intent(output),
                EditorIntent::Range(RangeIntent::VimMotion {
                    motion,
                    count: NonZeroUsize::new(count).unwrap(),
                    explicit_count,
                    extend: false,
                })
            );
        }

        for (key, motion) in [
            ('e', VimMotion::WordEndBackward),
            ('E', VimMotion::LongWordEndBackward),
            ('_', VimMotion::LastNonWhitespace),
        ] {
            let mut grammar = VimGrammar::default();
            translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('g'));
            assert!(matches!(
                only_intent(translate_vim(
                    &mut grammar,
                    Mode::Normal,
                    KeyStroke::char(key)
                )),
                EditorIntent::Range(RangeIntent::VimMotion { motion: actual, .. }) if actual == motion
            ));
        }

        for (key, command) in [
            ('d', EditorCommand::GotoDefinition),
            ('D', EditorCommand::GotoDeclaration),
            ('y', EditorCommand::GotoTypeDefinition),
            ('i', EditorCommand::GotoImplementation),
            ('r', EditorCommand::GotoReferences),
        ] {
            let mut grammar = VimGrammar::default();
            translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('g'));
            let EditorIntent::Command(invocation) = only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::char(key),
            )) else {
                panic!("g{key} must produce a semantic command")
            };
            assert_eq!(invocation.id(), CommandId::Editor(command));
        }
    }

    #[test]
    fn vim_modal_keys_ignore_terminal_shift_and_exact_scoped_actions_win() {
        for (character, expected) in [('G', VimMotion::FileEnd), ('W', VimMotion::LongWordForward)]
        {
            let mut grammar = VimGrammar::default();
            assert_eq!(
                only_intent(translate_vim(
                    &mut grammar,
                    Mode::Normal,
                    KeyStroke::new(KeyCode::Char(character), Modifiers::SHIFT),
                )),
                EditorIntent::Range(RangeIntent::VimMotion {
                    motion: expected,
                    count: NonZeroUsize::MIN,
                    explicit_count: false,
                    extend: false,
                })
            );
        }

        let mut insert = VimGrammar::default();
        assert_eq!(
            only_intent(
                insert
                    .translate(
                        InputEvent::Key(KeyStroke::new(KeyCode::Char('G'), Modifiers::SHIFT,)),
                        context(Mode::Insert, BindingScope::Global),
                    )
                    .unwrap(),
            ),
            EditorIntent::InsertText("G".to_owned())
        );

        for (key, expected) in [
            (
                KeyStroke::plain(KeyCode::Enter),
                EditorCommand::OpenDirectoryEntry,
            ),
            (
                KeyStroke::plain(KeyCode::Backspace),
                EditorCommand::OpenParentDirectory,
            ),
            (KeyStroke::char('-'), EditorCommand::OpenParentDirectory),
        ] {
            let mut grammar = VimGrammar::default();
            let EditorIntent::Command(invocation) = only_intent(translate_vim_in(
                &mut grammar,
                Mode::Normal,
                BindingScope::Directory,
                key,
            )) else {
                panic!("directory key must remain a scoped semantic command")
            };
            assert_eq!(invocation.id(), CommandId::Editor(expected));
        }
    }

    #[test]
    fn vim_uppercase_v_enters_counted_linewise_visual_mode() {
        let mut grammar = VimGrammar::default();
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('3'));
        assert_eq!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::new(KeyCode::Char('V'), Modifiers::SHIFT),
            )),
            EditorIntent::Range(RangeIntent::VimVisualLine {
                count: NonZeroUsize::new(3).unwrap(),
            })
        );

        grammar.complete(GrammarPostAction::None, Mode::Select);
        let EditorIntent::Command(exit) = only_intent(translate_vim(
            &mut grammar,
            Mode::Select,
            KeyStroke::new(KeyCode::Char('V'), Modifiers::SHIFT),
        )) else {
            panic!("V in linewise Visual must leave Visual mode")
        };
        assert_eq!(exit.id(), CommandId::Editor(EditorCommand::EnterNormalMode));
    }

    #[test]
    fn vim_core_motion_repetition_search_and_jump_keys_are_owned_intents() {
        for (key, motion) in [
            (KeyStroke::plain(KeyCode::Left), VimMotion::Left),
            (KeyStroke::plain(KeyCode::Down), VimMotion::Down),
            (KeyStroke::plain(KeyCode::Home), VimMotion::LineStart),
            (KeyStroke::plain(KeyCode::End), VimMotion::LineEnd),
        ] {
            let mut grammar = VimGrammar::default();
            assert!(matches!(
                only_intent(translate_vim(&mut grammar, Mode::Normal, key)),
                EditorIntent::Range(RangeIntent::VimMotion { motion: actual, .. }) if actual == motion
            ));
        }

        let mut find = VimGrammar::default();
        translate_vim(&mut find, Mode::Normal, KeyStroke::char('f'));
        translate_vim(&mut find, Mode::Normal, KeyStroke::char('x'));
        assert!(matches!(
            only_intent(translate_vim(&mut find, Mode::Normal, KeyStroke::char(';'))),
            EditorIntent::Range(RangeIntent::VimMotion {
                motion: VimMotion::FindNext('x'),
                ..
            })
        ));
        assert!(matches!(
            only_intent(translate_vim(&mut find, Mode::Normal, KeyStroke::char(','))),
            EditorIntent::Range(RangeIntent::VimMotion {
                motion: VimMotion::FindPrevious('x'),
                ..
            })
        ));

        for (key, previous) in [('*', false), ('#', true)] {
            let mut grammar = VimGrammar::default();
            assert_eq!(
                only_intent(translate_vim(
                    &mut grammar,
                    Mode::Normal,
                    KeyStroke::char(key)
                )),
                EditorIntent::Range(RangeIntent::VimSearchWord {
                    previous,
                    count: NonZeroUsize::MIN,
                })
            );
        }

        for (key, expected) in [
            (KeyStroke::ctrl('o'), EditorCommand::JumpBackward),
            (KeyStroke::ctrl('i'), EditorCommand::JumpForward),
            (KeyStroke::plain(KeyCode::Tab), EditorCommand::JumpForward),
        ] {
            let mut grammar = VimGrammar::default();
            let EditorIntent::Command(invocation) =
                only_intent(translate_vim(&mut grammar, Mode::Normal, key))
            else {
                panic!("jump key must be semantic")
            };
            assert_eq!(invocation.id(), CommandId::Editor(expected));
        }
    }

    #[test]
    fn vim_insert_reuses_registry_and_counts_are_explicitly_rejected() {
        let mut grammar = VimGrammar::default();
        let EditorIntent::Command(save) = only_intent(translate_vim(
            &mut grammar,
            Mode::Insert,
            KeyStroke::ctrl('s'),
        )) else {
            panic!("Vim Insert must reuse the Insert registry")
        };
        assert_eq!(save.id(), CommandId::Editor(EditorCommand::Save));

        for key in ['i', 'a', 'I', 'A', 'o', 'O', 'r', '"', 'q'] {
            for digit in ['1', '2'] {
                let mut grammar = VimGrammar::default();
                translate_vim(&mut grammar, Mode::Normal, KeyStroke::char(digit));
                assert!(matches!(
                    only_intent(translate_vim(
                        &mut grammar,
                        Mode::Normal,
                        KeyStroke::char(key)
                    )),
                    EditorIntent::Notice(GrammarNotice::CountNotSupported(_))
                ));
            }
        }
    }

    #[test]
    fn vim_registers_are_deferred_and_grammar_hints_are_not_runyte_rows() {
        let mut grammar = VimGrammar::default();
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('"'));
        assert!(
            translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('a'))
                .intents
                .is_empty()
        );
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('d'));
        let EditorIntent::Range(RangeIntent::VimOperator { register, .. }) = only_intent(
            translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('w')),
        ) else {
            panic!("deferred register must travel with the operation")
        };
        assert_eq!(register, Some('a'));

        let hints = vim_pending_hints(&KeySequence::from(KeyStroke::char('d')));
        assert!(hints.iter().any(|hint| hint.key == KeyStroke::char('w')));
        assert!(hints.iter().any(|hint| hint.key == KeyStroke::char('E')));
        assert!(hints.iter().any(|hint| hint.key == KeyStroke::char('$')));
        assert!(hints.iter().any(|hint| hint.key == KeyStroke::char('d')));
        assert!(hints.iter().any(|hint| hint.key == KeyStroke::char('f')));
        assert!(hints.iter().any(|hint| hint.key == KeyStroke::char('%')));
        assert!(hints.iter().any(|hint| hint.key == KeyStroke::char('g')));
        for key in ['f', 'F', 't', 'T', 'g', 'i', 'a'] {
            assert!(!vim_hint_is_exact(
                &KeySequence::from(KeyStroke::char('d')),
                KeyStroke::char(key)
            ));
        }
        assert!(vim_hint_is_exact(
            &KeySequence::from(KeyStroke::char('d')),
            KeyStroke::char('w')
        ));
        assert!(vim_hint_is_exact(
            &KeySequence::from(KeyStroke::char('g')),
            KeyStroke::char('g')
        ));
    }

    #[test]
    fn vim_cancel_and_invalid_register_clear_deferred_state() {
        let mut grammar = VimGrammar::default();
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('"'));
        assert!(matches!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::char('1')
            )),
            EditorIntent::Notice(GrammarNotice::InvalidRegister {
                register: '1',
                macros_only: false
            })
        ));

        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('"'));
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('a'));
        let cancelled = only_intent(translate_vim(
            &mut grammar,
            Mode::Normal,
            KeyStroke::plain(KeyCode::Escape),
        ));
        assert!(matches!(cancelled, EditorIntent::Command(_)));

        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('d'));
        let EditorIntent::Range(RangeIntent::VimOperator { register, .. }) = only_intent(
            translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('w')),
        ) else {
            panic!("expected Vim operator")
        };
        assert_eq!(register, None);

        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('d'));
        assert!(matches!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::plain(KeyCode::Escape)
            )),
            EditorIntent::Notice(GrammarNotice::SequenceCancelled)
        ));
        assert!(grammar.pending_sequence().is_empty());
    }

    #[test]
    fn vim_await_character_and_syntax_states_are_typed_and_cancelable() {
        let mut grammar = VimGrammar::default();
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('2'));
        assert!(matches!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::char('f')
            )),
            EditorIntent::Notice(GrammarNotice::AwaitingCharacter(_))
        ));
        assert_eq!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::char('λ')
            )),
            EditorIntent::Range(RangeIntent::VimMotion {
                motion: VimMotion::FindNext('λ'),
                count: NonZeroUsize::new(2).unwrap(),
                explicit_count: true,
                extend: false,
            })
        );

        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('t'));
        assert!(matches!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::plain(KeyCode::Escape)
            )),
            EditorIntent::Notice(GrammarNotice::CharacterInputCancelled)
        ));

        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('d'));
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('a'));
        assert_eq!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::char('f')
            )),
            EditorIntent::Range(RangeIntent::VimOperator {
                operator: VimOperator::Delete,
                target: VimRangeTarget::Syntax {
                    object: VimTextObject::Function,
                    around: true,
                },
                register: None,
            })
        );

        translate_vim(&mut grammar, Mode::Select, KeyStroke::char('i'));
        assert_eq!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Select,
                KeyStroke::char('p')
            )),
            EditorIntent::Range(RangeIntent::VimSyntaxSelection {
                object: VimTextObject::Parameter,
                around: false,
            })
        );

        for sequence in [['1', 'd', 'i', 'c'], ['d', '1', 'i', 'c']] {
            let mut grammar = VimGrammar::default();
            let mut output = GrammarOutput::default();
            for key in sequence {
                output = translate_vim(&mut grammar, Mode::Normal, KeyStroke::char(key));
            }
            assert!(matches!(
                only_intent(output),
                EditorIntent::Notice(GrammarNotice::CountNotSupported(_))
            ));
        }
    }

    #[test]
    fn vim_space_and_ctrl_w_use_only_the_canonical_registry_namespaces() {
        let mut grammar = VimGrammar::default();
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char(' '));
        let EditorIntent::Command(open) = only_intent(translate_vim(
            &mut grammar,
            Mode::Normal,
            KeyStroke::char('e'),
        )) else {
            panic!("Space e must use the canonical application tree")
        };
        assert_eq!(open.id(), CommandId::Editor(EditorCommand::OpenExplorer));

        translate_vim(&mut grammar, Mode::Normal, KeyStroke::ctrl('w'));
        let EditorIntent::Command(focus) = only_intent(translate_vim(
            &mut grammar,
            Mode::Normal,
            KeyStroke::char('h'),
        )) else {
            panic!("Ctrl-w h must use the canonical window tree")
        };
        assert_eq!(
            focus.id(),
            CommandId::Editor(EditorCommand::FocusWindowLeft)
        );
    }

    #[test]
    fn vim_macro_state_validates_registers_counts_replay_and_stops_typed() {
        let mut grammar = VimGrammar::default();
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('q'));
        assert!(matches!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::char('A')
            )),
            EditorIntent::Notice(GrammarNotice::InvalidRegister {
                register: 'A',
                macros_only: true
            })
        ));

        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('q'));
        let EditorIntent::Command(record) = only_intent(translate_vim(
            &mut grammar,
            Mode::Normal,
            KeyStroke::char('a'),
        )) else {
            panic!("q{{a-z}} must start recording")
        };
        assert_eq!(record.id(), CommandId::Editor(EditorCommand::RecordMacro));
        assert_eq!(record.execution().character(), Some('a'));

        let EditorIntent::Command(stop) = only_intent(translate_vim_recording(
            &mut grammar,
            Mode::Normal,
            KeyStroke::char('q'),
        )) else {
            panic!("q while recording must be a typed stop command")
        };
        assert_eq!(
            stop.id(),
            CommandId::Editor(EditorCommand::StopMacroRecording)
        );

        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('3'));
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('@'));
        let EditorIntent::Command(replay) = only_intent(translate_vim(
            &mut grammar,
            Mode::Normal,
            KeyStroke::char('b'),
        )) else {
            panic!("counted @{{a-z}} must replay")
        };
        assert_eq!(replay.id(), CommandId::Editor(EditorCommand::ReplayMacro));
        assert_eq!(replay.execution().count(), Some(3));
        assert_eq!(replay.execution().character(), Some('b'));
    }

    #[test]
    fn vim_replace_and_large_search_counts_remain_single_typed_intents() {
        let mut grammar = VimGrammar::default();
        translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('r'));
        assert_eq!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::char('λ')
            )),
            EditorIntent::Range(RangeIntent::VimReplace { character: 'λ' })
        );

        for _ in 0..6 {
            translate_vim(&mut grammar, Mode::Normal, KeyStroke::char('9'));
        }
        assert_eq!(
            only_intent(translate_vim(
                &mut grammar,
                Mode::Normal,
                KeyStroke::char('n'),
            )),
            EditorIntent::Range(RangeIntent::VimRepeatSearch {
                previous: false,
                count: NonZeroUsize::new(999_999).unwrap(),
            })
        );
    }

    #[test]
    fn vim_help_rows_are_grammar_owned_and_cover_declared_namespaces() {
        let rows = vim_help_rows();
        assert!(rows.iter().any(|row| row.sequence == "d c y + motion"));
        assert!(rows.iter().any(|row| row.sequence == "Space"));
        assert!(rows.iter().any(|row| row.sequence == "C-w"));
        assert!(!rows.iter().any(|row| row.sequence == "g d"));
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
