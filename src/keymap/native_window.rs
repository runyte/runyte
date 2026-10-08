// SPDX-License-Identifier: MPL-2.0

//! Window-local controls, consumed before editor/terminal dispatch. These do
//! not travel to the workspace host or participate in editor macros/remapping.
//! Help and native dispatch read the same entries.

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
            KeyStroke::ctrl('V'),
            #[cfg(target_os = "macos")]
            KeyStroke::new(
                crate::input::KeyCode::Char('v'),
                crate::input::Modifiers::SUPER,
            ),
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
