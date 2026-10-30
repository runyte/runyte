// SPDX-License-Identifier: MPL-2.0
use super::{Action, Owner};
use crate::input::{KeyCode, KeyStroke, Modifiers};

pub(super) fn legacy_action(owner: Owner, key: KeyStroke) -> Option<Action> {
    use Action::*;
    let plain = key.modifiers.is_empty();
    let control = key.modifiers == Modifiers::CONTROL;
    match key.code {
        KeyCode::Escape => Some(Cancel),
        KeyCode::Char('c') if control => Some(Cancel),
        KeyCode::Enter if plain => Some(Accept),
        _ if owner == Owner::ProviderOverwrite => None,
        KeyCode::Down | KeyCode::Tab if plain => Some(Next),
        KeyCode::Char('n') if control => Some(Next),
        KeyCode::BackTab => Some(Previous),
        KeyCode::Up if plain => Some(Previous),
        KeyCode::Char('p') if control => Some(Previous),
        _ => None,
    }
}

pub(super) fn labels(action: Action) -> &'static [&'static str] {
    match action {
        Action::Accept => &["Enter"],
        Action::Cancel => &["Esc", "Ctrl-c"],
        Action::Next => &["Down", "Tab", "Ctrl-n"],
        Action::Previous => &["Up", "BackTab", "Ctrl-p"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::ui::Bindings;

    #[test]
    fn cached_provider_defaults_preserve_raw_modifier_masks() {
        let bindings = Bindings::default();
        for owner in [Owner::ProviderReload, Owner::ProviderOverwrite] {
            for bits in 0..64 {
                let modifiers = Modifiers::from_bits(bits).unwrap();
                for code in [
                    KeyCode::Enter,
                    KeyCode::Escape,
                    KeyCode::Up,
                    KeyCode::Down,
                    KeyCode::Tab,
                    KeyCode::BackTab,
                    KeyCode::Char('c'),
                    KeyCode::Char('n'),
                    KeyCode::Char('p'),
                ] {
                    let key = KeyStroke::new(code, modifiers);
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
