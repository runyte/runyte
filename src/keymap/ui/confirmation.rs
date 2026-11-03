// SPDX-License-Identifier: MPL-2.0
use super::{Action, Owner};
use crate::input::{KeyCode, KeyStroke, Modifiers};

pub(super) fn actions(owner: Owner) -> &'static [Action] {
    use Action::*;
    if owner == Owner::FilesystemConfirmation {
        &[
            Accept, Cancel, Next, Previous, PageUp, PageDown, First, Last,
        ]
    } else if owner == Owner::TreeDestination {
        &[Accept, Cancel, Backspace]
    } else if owner == Owner::TreeDelete {
        &[Cancel]
    } else if owner == Owner::TextConfirmation {
        &[
            Accept, Cancel, Backspace, Delete, MoveLeft, MoveRight, MoveStart, MoveEnd,
        ]
    } else {
        &[Accept, Cancel]
    }
}

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    use Action::*;
    if owner == Owner::TreeDelete {
        return (!key
            .modifiers
            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
            && matches!(key.code, KeyCode::Enter | KeyCode::Escape))
        .then_some(Cancel);
    }
    if owner == Owner::FilesystemConfirmation {
        let control = key.modifiers.contains(Modifiers::CONTROL);
        let navigation = match (key.code, control) {
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => Some(Next),
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) => Some(Previous),
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => Some(PageDown),
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => Some(PageUp),
            (KeyCode::Home, _) => Some(First),
            (KeyCode::End, _) => Some(Last),
            _ => None,
        };
        if navigation.is_some() {
            return navigation;
        }
    }
    match key.code {
        KeyCode::Enter => Some(Accept),
        KeyCode::Escape => Some(Cancel),
        KeyCode::Char('c')
            if !matches!(owner, Owner::StashConfirmation | Owner::TreeDestination)
                && key.modifiers.contains(Modifiers::CONTROL) =>
        {
            Some(Cancel)
        }
        KeyCode::Backspace if matches!(owner, Owner::TextConfirmation | Owner::TreeDestination) => {
            Some(Backspace)
        }
        KeyCode::Delete if owner == Owner::TextConfirmation => Some(Delete),
        KeyCode::Left if owner == Owner::TextConfirmation => Some(MoveLeft),
        KeyCode::Right if owner == Owner::TextConfirmation => Some(MoveRight),
        KeyCode::Home if owner == Owner::TextConfirmation => Some(MoveStart),
        KeyCode::End if owner == Owner::TextConfirmation => Some(MoveEnd),
        _ => None,
    }
}

pub(super) fn labels(owner: Owner, action: Action) -> &'static [&'static str] {
    use Action::*;
    match action {
        Accept => &["Enter"],
        Cancel if matches!(owner, Owner::StashConfirmation | Owner::TreeDestination) => &["Esc"],
        Cancel if owner == Owner::TreeDelete => &["Enter", "Esc"],
        Cancel => &["Esc", "Ctrl-c"],
        Next => &["Down", "Ctrl-n"],
        Previous => &["Up", "Ctrl-p"],
        PageDown => &["PageDown", "Ctrl-d"],
        PageUp => &["PageUp", "Ctrl-u"],
        First => &["Home"],
        Last => &["End"],
        Backspace => &["Backspace"],
        Delete => &["Delete"],
        MoveLeft => &["Left"],
        MoveRight => &["Right"],
        MoveStart => &["Home"],
        MoveEnd => &["End"],
        _ => &[],
    }
}
