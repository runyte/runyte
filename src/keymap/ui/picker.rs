// SPDX-License-Identifier: MPL-2.0
use super::{Action, Owner};
use crate::input::{KeyCode, KeyStroke, Modifiers};

pub(super) fn actions(owner: Owner) -> &'static [Action] {
    use Action::*;
    if owner == Owner::Finder {
        &[
            Accept,
            Cancel,
            Next,
            Previous,
            PageUp,
            PageDown,
            First,
            Last,
            MoveLeft,
            MoveRight,
            MoveStart,
            MoveEnd,
            Backspace,
            DeleteBackward,
            Delete,
            DeleteWordBackward,
            DeleteToEnd,
            SplitVertical,
            SplitHorizontal,
            TogglePreview,
            ToggleMode,
        ]
    } else {
        &[
            Accept,
            Cancel,
            Next,
            Previous,
            PageUp,
            PageDown,
            First,
            Last,
            MoveLeft,
            MoveRight,
            MoveStart,
            MoveEnd,
            Backspace,
            DeleteBackward,
            Delete,
            DeleteWordBackward,
            DeleteToEnd,
            SplitVertical,
            SplitHorizontal,
            TogglePreview,
        ]
    }
}

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    use Action::*;
    let control = key.modifiers.contains(Modifiers::CONTROL);
    match key.code {
        KeyCode::Escape => Some(Cancel),
        KeyCode::Char('c') if control => Some(Cancel),
        KeyCode::Char('p') if control => Some(Previous),
        KeyCode::Char('n') if control => Some(Next),
        KeyCode::Char('u') if control => Some(PageUp),
        KeyCode::Char('d') if control => Some(PageDown),
        KeyCode::Char('s') if control => Some(SplitHorizontal),
        KeyCode::Char('v') if control => Some(SplitVertical),
        KeyCode::Char('t') if control => Some(TogglePreview),
        KeyCode::Char('b') if control => Some(MoveLeft),
        KeyCode::Char('f') if control => Some(MoveRight),
        KeyCode::Char('h') if control => Some(DeleteBackward),
        KeyCode::Char('w') if control => Some(DeleteWordBackward),
        KeyCode::Char('k') if control => Some(DeleteToEnd),
        KeyCode::Char('a') if control => Some(MoveStart),
        KeyCode::Char('e') if control => Some(MoveEnd),
        KeyCode::Tab if owner == Owner::Finder && key.modifiers.is_empty() => Some(ToggleMode),
        KeyCode::Tab if owner == Owner::FilePicker => Some(Next),
        KeyCode::Down => Some(Next),
        KeyCode::Up | KeyCode::BackTab => Some(Previous),
        KeyCode::PageUp => Some(PageUp),
        KeyCode::PageDown => Some(PageDown),
        KeyCode::Home => Some(First),
        KeyCode::End => Some(Last),
        KeyCode::Left => Some(MoveLeft),
        KeyCode::Right => Some(MoveRight),
        KeyCode::Backspace => Some(Backspace),
        KeyCode::Delete => Some(Delete),
        KeyCode::Enter => Some(Accept),
        _ => None,
    }
}

pub(super) fn labels(owner: Owner, action: Action) -> &'static [&'static str] {
    use Action::*;
    match action {
        Accept => &["Enter"],
        Cancel => &["Esc", "Ctrl-c"],
        Next if owner == Owner::FilePicker => &["Down", "Tab", "Ctrl-n"],
        Next => &["Down", "Ctrl-n"],
        Previous => &["Up", "BackTab", "Ctrl-p"],
        PageUp => &["PageUp", "Ctrl-u"],
        PageDown => &["PageDown", "Ctrl-d"],
        First => &["Home"],
        Last => &["End"],
        MoveLeft => &["Left", "Ctrl-b"],
        MoveRight => &["Right", "Ctrl-f"],
        MoveStart => &["Ctrl-a"],
        MoveEnd => &["Ctrl-e"],
        Backspace => &["Backspace"],
        DeleteBackward => &["Ctrl-h"],
        Delete => &["Delete"],
        DeleteWordBackward => &["Ctrl-w"],
        DeleteToEnd => &["Ctrl-k"],
        SplitVertical => &["Ctrl-v"],
        SplitHorizontal => &["Ctrl-s"],
        TogglePreview => &["Ctrl-t"],
        ToggleMode => &["Tab"],
        _ => &[],
    }
}
