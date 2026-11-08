// SPDX-License-Identifier: MPL-2.0
use super::Action;
use crate::input::{KeyCode, KeyStroke, Modifiers};

pub(super) fn action(key: KeyStroke) -> Option<Action> {
    use Action::*;
    use KeyCode::*;
    let key = key.canonical_for_binding();
    if key.modifiers != Modifiers::NONE && key.modifiers != Modifiers::SHIFT {
        return None;
    }
    let shift = key.modifiers.contains(Modifiers::SHIFT);
    Some(match key.code {
        Escape if !shift => Cancel,
        Enter if !shift => Accept,
        Tab if !shift => Next,
        Tab | BackTab => Previous,
        Char('u') if !shift => Back,
        Left | Char('h' | 'H') if shift || key.code == Char('H') => ExtendLeft,
        Right | Char('l' | 'L') if shift || key.code == Char('L') => ExtendRight,
        Up | Char('k' | 'K') if shift || key.code == Char('K') => ExtendUp,
        Down | Char('j' | 'J') if shift || key.code == Char('J') => ExtendDown,
        Left | Char('h') => MoveLeft,
        Right | Char('l') => MoveRight,
        Up | Char('k') => MoveUp,
        Down | Char('j') => MoveDown,
        _ => return None,
    })
}

pub(super) fn labels(action: Action) -> &'static [&'static str] {
    use Action::*;
    match action {
        Next => &["Tab"],
        Previous => &["Shift-Tab"],
        Accept => &["Enter"],
        Cancel => &["Esc"],
        Back => &["u"],
        MoveLeft => &["h", "Left"],
        MoveRight => &["l", "Right"],
        MoveUp => &["k", "Up"],
        MoveDown => &["j", "Down"],
        ExtendLeft => &["H", "Shift-Left"],
        ExtendRight => &["L", "Shift-Right"],
        ExtendUp => &["K", "Shift-Up"],
        ExtendDown => &["J", "Shift-Down"],
        _ => &[],
    }
}
