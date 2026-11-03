// SPDX-License-Identifier: MPL-2.0
use super::{Action, Owner};
use crate::input::{KeyCode, KeyStroke, Modifiers};

pub(super) fn actions(owner: Owner) -> &'static [Action] {
    use Action::*;
    if owner == Owner::SessionDirectory {
        &[
            Next,
            Previous,
            PageUp,
            PageDown,
            First,
            Last,
            Accept,
            EnterDirectory,
            Cancel,
            Backspace,
        ]
    } else {
        &[
            Next, Previous, PageUp, PageDown, First, Last, Accept, Cancel, Backspace,
        ]
    }
}

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    use Action::*;
    let control = key.modifiers.contains(Modifiers::CONTROL);
    match (key.code, control) {
        (KeyCode::Escape, _) | (KeyCode::Char('c'), true) => Some(Cancel),
        (KeyCode::Up | KeyCode::BackTab, _) | (KeyCode::Char('p'), true) => Some(Previous),
        (KeyCode::Down, _) | (KeyCode::Char('n'), true) => Some(Next),
        (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => Some(PageUp),
        (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => Some(PageDown),
        (KeyCode::Home, _) => Some(First),
        (KeyCode::End, _) => Some(Last),
        (KeyCode::Enter, _) => Some(Accept),
        (KeyCode::Tab, _) if owner == Owner::SessionDirectory => Some(EnterDirectory),
        (KeyCode::Backspace, _) => Some(Backspace),
        _ => None,
    }
}

pub(super) fn labels(action: Action) -> &'static [&'static str] {
    use Action::*;
    match action {
        Next => &["Down", "Ctrl-n"],
        Previous => &["Up", "Ctrl-p", "BackTab"],
        PageUp => &["PageUp", "Ctrl-u"],
        PageDown => &["PageDown", "Ctrl-d"],
        First => &["Home"],
        Last => &["End"],
        Accept => &["Enter"],
        EnterDirectory => &["Tab"],
        Cancel => &["Esc", "Ctrl-c"],
        Backspace => &["Backspace"],
        _ => &[],
    }
}
