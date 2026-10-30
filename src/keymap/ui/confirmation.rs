// SPDX-License-Identifier: MPL-2.0
use super::{Action, Owner};
use crate::input::{KeyCode, KeyStroke, Modifiers};

pub(super) fn actions(owner: Owner) -> &'static [Action] {
    use Action::*;
    if owner == Owner::TextConfirmation {
        &[
            Accept, Cancel, Backspace, Delete, MoveLeft, MoveRight, MoveStart, MoveEnd,
        ]
    } else {
        &[Accept, Cancel]
    }
}

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    use Action::*;
    match key.code {
        KeyCode::Enter => Some(Accept),
        KeyCode::Escape => Some(Cancel),
        KeyCode::Char('c')
            if owner != Owner::StashConfirmation && key.modifiers.contains(Modifiers::CONTROL) =>
        {
            Some(Cancel)
        }
        KeyCode::Backspace if owner == Owner::TextConfirmation => Some(Backspace),
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
        Cancel if owner == Owner::StashConfirmation => &["Esc"],
        Cancel => &["Esc", "Ctrl-c"],
        Backspace => &["Backspace"],
        Delete => &["Delete"],
        MoveLeft => &["Left"],
        MoveRight => &["Right"],
        MoveStart => &["Home"],
        MoveEnd => &["End"],
        _ => &[],
    }
}
