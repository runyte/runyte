// SPDX-License-Identifier: MPL-2.0
use super::{Action, Bindings, Owner};
use crate::input::{KeyCode, KeyStroke, Modifiers};

/// Choice keys historically spend a pending review motion, including removed
/// aliases and modifier combinations with no action. Other keys try the narrow
/// editor motion registry before their UI fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Route {
    Owned(Option<Action>),
    ReviewThenFallback(Option<Action>),
}

impl Bindings {
    pub(crate) fn context_review_route(&self, key: KeyStroke) -> Route {
        let key = key.canonical_for_binding();
        let action = self.lookup(Owner::ContextReview, key);
        if action.is_some_and(|action| self.explicitly_configured(Owner::ContextReview, action))
            || matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Tab
                    | KeyCode::BackTab
                    | KeyCode::Enter
                    | KeyCode::Escape
            )
        {
            Route::Owned(action)
        } else {
            Route::ReviewThenFallback(action)
        }
    }
}

pub(super) fn legacy_action(key: KeyStroke) -> Option<Action> {
    use Action::*;
    if key.code == KeyCode::BackTab
        || (key.code == KeyCode::Tab && key.modifiers == Modifiers::SHIFT)
    {
        return Some(ToggleSelection);
    }
    if !key.modifiers.is_empty() {
        return None;
    }
    match key.code {
        KeyCode::Up => Some(Previous),
        KeyCode::Down => Some(Next),
        KeyCode::Tab => Some(ToggleSelection),
        KeyCode::Enter => Some(Accept),
        KeyCode::Escape => Some(Cancel),
        KeyCode::Home => Some(First),
        KeyCode::End => Some(Last),
        _ => None,
    }
}

pub(super) fn labels(action: Action) -> &'static [&'static str] {
    use Action::*;
    match action {
        Next => &["Down"],
        Previous => &["Up"],
        ToggleSelection => &["Tab", "Shift-Tab", "BackTab"],
        Accept => &["Enter"],
        Cancel => &["Esc"],
        First => &["Home"],
        Last => &["End"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_exact_choice_modifier_policies() {
        for bits in 0..64 {
            let modifiers = Modifiers::from_bits(bits).unwrap();
            for code in [
                KeyCode::BackTab,
                KeyCode::Tab,
                KeyCode::Enter,
                KeyCode::Escape,
                KeyCode::Up,
                KeyCode::Down,
                KeyCode::Home,
                KeyCode::End,
            ] {
                let key = KeyStroke::new(code, modifiers);
                assert_eq!(
                    Bindings::default().lookup(Owner::ContextReview, key),
                    legacy_action(key)
                );
            }
            assert_eq!(
                legacy_action(KeyStroke::new(KeyCode::BackTab, modifiers)),
                Some(Action::ToggleSelection)
            );
            assert_eq!(
                legacy_action(KeyStroke::new(KeyCode::Tab, modifiers)),
                (modifiers.is_empty() || modifiers == Modifiers::SHIFT)
                    .then_some(Action::ToggleSelection)
            );
            for (code, action) in [
                (KeyCode::Enter, Action::Accept),
                (KeyCode::Escape, Action::Cancel),
                (KeyCode::Up, Action::Previous),
                (KeyCode::Down, Action::Next),
                (KeyCode::Home, Action::First),
                (KeyCode::End, Action::Last),
            ] {
                assert_eq!(
                    legacy_action(KeyStroke::new(code, modifiers)),
                    modifiers.is_empty().then_some(action)
                );
            }
        }
        assert_eq!(legacy_action(KeyStroke::ctrl('c')), None);
        assert_eq!(legacy_action(KeyStroke::char(' ')), None);
    }
}
