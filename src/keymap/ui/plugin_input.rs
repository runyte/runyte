// SPDX-License-Identifier: MPL-2.0
use super::{Action, Owner};
use crate::input::{KeyCode, KeyStroke, Modifiers};

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    use Action::*;
    let plain = key.modifiers.is_empty();
    let control = key.modifiers == Modifiers::CONTROL;
    if owner == Owner::PluginPathCompletion {
        return if plain {
            match key.code {
                KeyCode::Tab => Some(Accept),
                KeyCode::Enter => Some(AcceptOrContinue),
                KeyCode::Up => Some(Previous),
                KeyCode::Down => Some(Next),
                _ => None,
            }
        } else {
            None
        };
    }
    match key.code {
        KeyCode::Escape => Some(Cancel),
        KeyCode::Char('c') if control => Some(Cancel),
        KeyCode::Enter if plain => Some(Accept),
        _ if owner == Owner::PluginConfirmation => None,
        KeyCode::Up | KeyCode::BackTab => Some(Previous),
        KeyCode::Down => Some(Next),
        KeyCode::Char('n') if owner == Owner::PluginPicker && control => Some(Next),
        KeyCode::Char('p') if owner == Owner::PluginPicker && control => Some(Previous),
        KeyCode::Backspace if owner != Owner::PluginChoice => Some(Backspace),
        _ if owner == Owner::PluginPicker => None,
        KeyCode::Tab => Some(Next),
        KeyCode::Left if owner != Owner::PluginChoice || plain => Some(MoveLeft),
        KeyCode::Right if owner != Owner::PluginChoice || plain => Some(MoveRight),
        KeyCode::Char(' ') if owner == Owner::PluginChoice && plain => Some(ToggleSelection),
        _ if owner == Owner::PluginChoice => None,
        KeyCode::Delete => Some(Delete),
        KeyCode::Home => Some(MoveStart),
        KeyCode::End => Some(MoveEnd),
        _ => None,
    }
}

pub(super) fn labels(owner: Owner, action: Action) -> &'static [&'static str] {
    use Action::*;
    match (owner, action) {
        (Owner::PluginPathCompletion, Accept) => &["Tab"],
        (_, Accept | AcceptOrContinue) => &["Enter"],
        (_, Cancel) => &["Esc", "Ctrl-c"],
        (Owner::PluginPicker, Next) => &["Down", "Ctrl-n"],
        (Owner::PluginPicker, Previous) => &["Up", "BackTab", "Ctrl-p"],
        (Owner::PluginPathCompletion, Next) => &["Down"],
        (Owner::PluginPathCompletion, Previous) => &["Up"],
        (_, Next) => &["Tab", "Down"],
        (_, Previous) => &["Up", "BackTab"],
        (_, Backspace) => &["Backspace"],
        (_, Delete) => &["Delete"],
        (_, MoveLeft) => &["Left"],
        (_, MoveRight) => &["Right"],
        (_, MoveStart) => &["Home"],
        (_, MoveEnd) => &["End"],
        (_, ToggleSelection) => &["Space"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::ui::Bindings;
    #[test]
    fn plugin_defaults_keep_raw_modifier_policies_and_do_not_inherit_leader() {
        let bindings = Bindings::default();
        for owner in [
            Owner::PluginConfirmation,
            Owner::PluginPicker,
            Owner::PluginForm,
            Owner::PluginChoice,
            Owner::PluginPathCompletion,
        ] {
            for bits in 0..64 {
                for code in [
                    KeyCode::Enter,
                    KeyCode::Escape,
                    KeyCode::Tab,
                    KeyCode::BackTab,
                    KeyCode::Up,
                    KeyCode::Down,
                    KeyCode::Left,
                    KeyCode::Right,
                    KeyCode::Home,
                    KeyCode::End,
                    KeyCode::Backspace,
                    KeyCode::Delete,
                    KeyCode::Char(' '),
                    KeyCode::Char('c'),
                    KeyCode::Char('n'),
                    KeyCode::Char('p'),
                ] {
                    let key = KeyStroke::new(code, Modifiers::from_bits(bits).unwrap());
                    assert_eq!(
                        bindings.lookup(owner, key),
                        legacy_action(owner, key),
                        "{owner:?} {key:?}"
                    );
                }
            }
        }
    }
}
