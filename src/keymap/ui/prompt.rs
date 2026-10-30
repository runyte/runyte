// SPDX-License-Identifier: MPL-2.0
//! Prompt-specific defaults, including the original modifier precedence.
use super::{Action, Owner};
use crate::input::{KeyCode, KeyStroke, Modifiers};

pub(super) fn actions(owner: Owner) -> &'static [Action] {
    use Action::*;
    match owner {
        Owner::CommandPrompt => &[
            Accept,
            Cancel,
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
            Next,
            Previous,
            First,
            Last,
            Complete,
        ],
        Owner::PathPrompt => &[
            Accept,
            Cancel,
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
            Next,
            Previous,
            Complete,
        ],
        Owner::ProgramPrompt => &[
            Accept,
            Cancel,
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
            Next,
            Previous,
            OpenActions,
        ],
        Owner::TextPrompt => &[
            Accept,
            Cancel,
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
        ],
        _ => &[],
    }
}

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    use Action::*;
    if key.modifiers.contains(Modifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(Cancel),
            KeyCode::Char('s') => Some(Save),
            KeyCode::Char('b') => Some(MoveLeft),
            KeyCode::Char('f') => Some(MoveRight),
            KeyCode::Char('a') => Some(MoveStart),
            KeyCode::Char('e') => Some(MoveEnd),
            KeyCode::Char('w') => Some(DeleteWordBackward),
            KeyCode::Char('u') => Some(DeleteToStart),
            KeyCode::Char('k') => Some(DeleteToEnd),
            KeyCode::Char('h') => Some(DeleteBackward),
            KeyCode::Char('d') => Some(DeleteForward),
            _ => None,
        };
    }
    if key.modifiers.contains(Modifiers::ALT) {
        return match key.code {
            KeyCode::Char('b') => Some(WordLeft),
            KeyCode::Char('f') => Some(WordRight),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Escape => Some(Cancel),
        KeyCode::Enter => Some(Accept),
        KeyCode::Backspace => Some(Backspace),
        KeyCode::Delete if owner != Owner::CommandPrompt => Some(Delete),
        KeyCode::Left => Some(MoveLeft),
        KeyCode::Right => Some(MoveRight),
        KeyCode::Home => Some(if owner == Owner::CommandPrompt {
            First
        } else {
            MoveStart
        }),
        KeyCode::End => Some(if owner == Owner::CommandPrompt {
            Last
        } else {
            MoveEnd
        }),
        KeyCode::Up | KeyCode::BackTab if owner != Owner::TextPrompt => Some(Previous),
        KeyCode::Down if owner != Owner::TextPrompt => Some(Next),
        KeyCode::Tab if owner == Owner::ProgramPrompt => Some(OpenActions),
        KeyCode::Tab if owner != Owner::TextPrompt => Some(Complete),
        _ => None,
    }
}

pub(super) fn labels(owner: Owner, action: Action) -> &'static [&'static str] {
    use Action::*;
    match action {
        Accept => &["Enter"],
        Cancel => &["Esc", "Ctrl-c"],
        Save => &["Ctrl-s"],
        MoveLeft => &["Left", "Ctrl-b"],
        MoveRight => &["Right", "Ctrl-f"],
        MoveStart if owner == Owner::CommandPrompt => &["Ctrl-a"],
        MoveStart => &["Home", "Ctrl-a"],
        MoveEnd if owner == Owner::CommandPrompt => &["Ctrl-e"],
        MoveEnd => &["End", "Ctrl-e"],
        WordLeft => &["Alt-b"],
        WordRight => &["Alt-f"],
        DeleteBackward => &["Ctrl-h"],
        DeleteForward => &["Ctrl-d"],
        DeleteWordBackward => &["Ctrl-w"],
        DeleteToStart => &["Ctrl-u"],
        DeleteToEnd => &["Ctrl-k"],
        Backspace => &["Backspace"],
        Delete => &["Delete"],
        Next => &["Down"],
        Previous => &["Up", "BackTab"],
        First => &["Home"],
        Last => &["End"],
        Complete | OpenActions => &["Tab"],
        _ => &[],
    }
}
