// SPDX-License-Identifier: MPL-2.0

//! Window-local controls, consumed before editor/terminal dispatch. Their
//! effective projection travels with the painted frame; dispatch and help read
//! the same shared UI actions. They do not become editor macro input.

use crate::input::KeyStroke;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    FontSize(i8),
    PasteText,
}

pub struct Binding {
    pub keys: &'static [KeyStroke],
    pub description: &'static str,
    pub action: Action,
}

pub const BINDINGS: &[Binding] = &[
    Binding {
        keys: &[KeyStroke::ctrl('+'), KeyStroke::ctrl('=')],
        description: "Increase window font size (until the window closes)",
        action: Action::FontSize(1),
    },
    Binding {
        keys: &[KeyStroke::ctrl('-')],
        description: "Decrease window font size (until the window closes)",
        action: Action::FontSize(-1),
    },
    Binding {
        keys: &[
            #[cfg(target_os = "macos")]
            KeyStroke::new(
                crate::input::KeyCode::Char('v'),
                crate::input::Modifiers::SUPER,
            ),
            KeyStroke::ctrl('V'),
        ],
        description: "Paste system clipboard text at the cursor",
        action: Action::PasteText,
    },
];

pub fn lookup(key: KeyStroke) -> Option<&'static Binding> {
    let key = key.canonical_for_binding();
    BINDINGS.iter().find(|binding| binding.keys.contains(&key))
}

pub fn help() -> String {
    let mut text = String::from("\nNative window\n\n");
    for binding in BINDINGS {
        let keys = binding
            .keys
            .iter()
            .map(|key| key.label())
            .collect::<Vec<_>>()
            .join(" / ");
        text.push_str(&format!("  {keys}  {}\n", binding.description));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{KeyCode, Modifiers};

    #[test]
    fn command_paste_is_macos_only_and_preserves_other_keys() {
        assert_eq!(
            lookup(KeyStroke::new(KeyCode::Char('v'), Modifiers::SUPER))
                .map(|binding| binding.action),
            cfg!(target_os = "macos").then_some(Action::PasteText)
        );
        for key in [
            KeyStroke::new(KeyCode::Char('y'), Modifiers::SUPER),
            KeyStroke::ctrl('Y'),
            KeyStroke::ctrl('y'),
            KeyStroke::ctrl('v'),
            KeyStroke::char('v'),
        ] {
            assert!(lookup(key).is_none());
        }
        assert_eq!(
            lookup(KeyStroke::ctrl('V')).unwrap().action,
            Action::PasteText
        );
    }

    #[test]
    fn window_font_controls_share_their_help_and_preserve_other_input() {
        for binding in BINDINGS {
            for key in binding.keys {
                assert_eq!(lookup(*key).unwrap().action, binding.action);
                assert!(help().contains(&key.label()));
            }
            assert!(help().contains(binding.description));
        }
        assert_eq!(
            lookup(KeyStroke::new(
                KeyCode::Char('+'),
                Modifiers::CONTROL | Modifiers::SHIFT
            ))
            .unwrap()
            .action,
            Action::FontSize(1)
        );
        for key in [
            KeyStroke::char('+'),
            KeyStroke::char('-'),
            KeyStroke::ctrl('g'),
            KeyStroke::alt('-'),
        ] {
            assert!(lookup(key).is_none());
        }
    }
}

/// Immutable bounded projection carried by a painted frontend frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bindings {
    entries: std::sync::Arc<[(KeyStroke, Action)]>,
    authored: u8,
    guard: u8,
}
impl Default for Bindings {
    fn default() -> Self {
        Self::from_entries(
            BINDINGS
                .iter()
                .flat_map(|binding| binding.keys.iter().map(|key| (*key, binding.action)))
                .collect(),
        )
        .expect("factory window bindings")
    }
}
impl Bindings {
    pub const MAX_ENTRIES: usize = 96;
    pub const GUARD_NONE: u8 = 0;
    pub const GUARD_CHARACTER: u8 = 1;
    pub const GUARD_TEXT: u8 = 2;
    pub const GUARD_PATH_CHOICE: u8 = 3;
    pub fn from_entries(mut entries: Vec<(KeyStroke, Action)>) -> Result<Self, String> {
        if entries.len() > Self::MAX_ENTRIES {
            return Err("too many window bindings".into());
        }
        let mut seen = std::collections::HashSet::new();
        for (key, action) in &entries {
            if *key != key.canonical_for_binding() || !seen.insert(*key) {
                return Err("duplicate or noncanonical window binding".into());
            }
            if matches!(key.code, crate::input::KeyCode::Char(c) if c.is_control() || !key.modifiers.intersects(crate::input::Modifiers::CONTROL | crate::input::Modifiers::ALT | crate::input::Modifiers::SUPER))
            {
                return Err("window binding owns structural text".into());
            }
            if !matches!(action, Action::FontSize(-1 | 1) | Action::PasteText) {
                return Err("invalid window action".into());
            }
        }
        entries.sort_by_key(|(key, _)| key.label());
        Ok(Self {
            entries: entries.into(),
            authored: 0,
            guard: 0,
        })
    }
    /// Provenance belongs to the captured configuration; eligibility belongs to
    /// the latest acknowledged editor state. Keep them separate across reloads.
    pub fn with_policy(mut self, authored: u8, guard: u8) -> Result<Self, String> {
        if authored > 7 || guard > 3 {
            return Err("invalid window input policy".into());
        }
        self.authored = authored;
        self.guard = guard;
        Ok(self)
    }
    pub fn has_authored_characters(&self) -> bool {
        self.entries.iter().any(|(key, action)| {
            let bit = match action {
                Action::FontSize(1) => 1,
                Action::FontSize(-1) => 2,
                Action::PasteText => 4,
                _ => 0,
            };
            matches!(key.code, crate::input::KeyCode::Char(_)) && self.authored & bit != 0
        })
    }
    pub fn authored(&self) -> u8 {
        self.authored
    }
    pub fn guard(&self) -> u8 {
        self.guard
    }
    pub fn with_current_guard(&self, current: &Self) -> Self {
        let mut bindings = self.clone();
        bindings.guard = current.guard;
        bindings
    }
    fn eligible(&self, key: KeyStroke, action: Action) -> bool {
        use crate::input::{KeyCode, Modifiers};
        let bit = match action {
            Action::FontSize(1) => 1,
            Action::FontSize(-1) => 2,
            Action::PasteText => 4,
            _ => 0,
        };
        self.authored & bit == 0
            || !matches!(key.code, KeyCode::Char(c) if match self.guard {
                Self::GUARD_CHARACTER => true,
                Self::GUARD_TEXT => !key.modifiers.intersects(Modifiers::CONTROL | Modifiers::ALT),
                Self::GUARD_PATH_CHOICE => matches!(c, 'r' | 's'),
                _ => false,
            })
    }
    pub fn entries(&self) -> &[(KeyStroke, Action)] {
        &self.entries
    }
    pub fn lookup(&self, key: KeyStroke) -> Option<Action> {
        let key = key.canonical_for_binding();
        self.entries
            .iter()
            .find(|(candidate, action)| *candidate == key && self.eligible(key, *action))
            .map(|(_, action)| *action)
    }
    pub fn help(&self) -> String {
        let mut text = String::from("\nNative window\n\n");
        for binding in BINDINGS {
            let keys = self
                .entries
                .iter()
                .filter(|(key, action)| *action == binding.action && self.eligible(*key, *action))
                .map(|(key, _)| key.label())
                .collect::<Vec<_>>();
            let keys = if keys.is_empty() {
                "unbound".into()
            } else {
                keys.join(" / ")
            };
            text.push_str(&format!("  {keys}  {}\n", binding.description));
        }
        text
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use crate::input::{KeyCode, Modifiers};

    #[test]
    fn window_projection_preserves_factory_modifier_matrix() {
        let ui = crate::keymap::ui::Bindings::default();
        for bits in 0..64 {
            for code in [
                KeyCode::Char('+'),
                KeyCode::Char('='),
                KeyCode::Char('-'),
                KeyCode::Char('v'),
                KeyCode::Char('V'),
                KeyCode::Enter,
            ] {
                let key = KeyStroke::new(code, Modifiers::from_bits(bits).unwrap());
                assert_eq!(
                    ui.native_window().lookup(key),
                    lookup(key).map(|binding| binding.action),
                    "{key:?}"
                );
            }
        }
    }

    #[test]
    fn window_projection_replaces_initialized_cache_and_removes_all_aliases() {
        let base = crate::keymap::default_keymap();
        assert_eq!(
            base.ui().native_window().lookup(KeyStroke::ctrl('+')),
            Some(Action::FontSize(1))
        );
        let compiled = crate::keymap::configured::compile(&serde_yaml::from_str("ui: {native-window: {font-increase: [F8], font-decrease: [], paste-text: [Alt-v]}}").unwrap(), base);
        assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
        let bindings = compiled.keymap.ui().native_window();
        assert_eq!(
            bindings.lookup(KeyStroke::plain(KeyCode::Function(8))),
            Some(Action::FontSize(1))
        );
        assert_eq!(
            bindings.lookup(KeyStroke::parse("Alt-v").unwrap()),
            Some(Action::PasteText)
        );
        for key in [
            KeyStroke::ctrl('+'),
            KeyStroke::ctrl('='),
            KeyStroke::ctrl('-'),
            KeyStroke::ctrl('V'),
        ] {
            assert!(bindings.lookup(key).is_none());
        }
        assert!(bindings.help().contains("F8"));
        assert!(bindings.help().contains("unbound"));
        assert!(
            base.ui()
                .native_window()
                .lookup(KeyStroke::ctrl('+'))
                .is_some()
        );
    }

    #[test]
    fn window_projection_rejects_structural_keys_duplicates_and_invalid_actions() {
        for entries in [
            vec![(KeyStroke::char('x'), Action::FontSize(1))],
            vec![(KeyStroke::ctrl('x'), Action::FontSize(2))],
            vec![(KeyStroke::ctrl('x'), Action::FontSize(1)); 2],
            vec![(KeyStroke::ctrl('x'), Action::FontSize(1)); Bindings::MAX_ENTRIES + 1],
            vec![(
                KeyStroke::new(KeyCode::Char('x'), Modifiers::CONTROL | Modifiers::SHIFT),
                Action::FontSize(1),
            )],
        ] {
            assert!(Bindings::from_entries(entries).is_err());
        }
        assert!(Bindings::default().with_policy(8, 0).is_err());
        assert!(Bindings::default().with_policy(0, 4).is_err());
        let compiled = crate::keymap::configured::compile(
            &serde_yaml::from_str("ui: {native-window: {font-increase: [x]}}").unwrap(),
            crate::keymap::default_keymap(),
        );
        assert_eq!(compiled.errors.len(), 1);
        assert!(
            compiled
                .keymap
                .ui()
                .native_window()
                .lookup(KeyStroke::ctrl('+'))
                .is_some()
        );
    }
}
