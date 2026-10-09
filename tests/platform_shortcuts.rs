// SPDX-License-Identifier: MPL-2.0

use runyte::{
    command::{EditorCommand, GrammarKind, Mode},
    help::{self, HelpTopic},
    input::{KeyCode, KeyStroke, Modifiers},
    key_hints::{KeyHintState, key_hint_keys},
    keymap::{
        BindingScope, BindingTarget, KeySequence, Keymap, Lookup, default_keymap, native_window,
    },
};

#[test]
fn command_and_super_spellings_share_identity_without_remapping_control() {
    for character in ['c', 'v'] {
        let command = KeyStroke::parse(&format!("Cmd-{character}")).unwrap();
        let super_key = KeyStroke::parse(&format!("Super-{character}")).unwrap();
        assert_eq!(command, super_key);
        assert_eq!(command.modifiers, Modifiers::SUPER);
        assert_ne!(command, KeyStroke::ctrl(character));
        assert_eq!(KeyStroke::parse(&command.label()).unwrap(), command);
    }
    assert!(KeyStroke::parse("Cmd-Super-c").is_err());
    assert!(KeyStroke::parse("Super-Cmd-c").is_err());
}

#[test]
fn command_copy_uses_platform_spelling_in_registry_help_and_hints() {
    // Exercise Command bindings even on Linux, where they may be configured
    // explicitly but are not built-in defaults.
    let mut binding = default_keymap()
        .bindings()
        .iter()
        .find(|binding| binding.sequence == KeySequence::from(KeyStroke::ctrl('C')))
        .unwrap()
        .clone();
    binding.sequence = KeySequence::parse("Cmd-c").unwrap();
    let mut bindings = default_keymap().bindings().to_vec();
    if !cfg!(target_os = "macos") {
        bindings.push(binding);
    }
    let keymap = Keymap::with_namespaces(bindings, default_keymap().namespaces().to_vec()).unwrap();
    let Lookup::Exact(binding) = keymap.lookup_in(
        Mode::Normal,
        BindingScope::Global,
        &KeySequence::parse("Super-c").unwrap(),
    ) else {
        panic!("Command copy must resolve through the registry");
    };
    assert_eq!(
        binding.target,
        BindingTarget::Editor(EditorCommand::ClipboardYank)
    );
    let label = if cfg!(target_os = "macos") {
        "Cmd-c"
    } else {
        "Super-c"
    };
    assert_eq!(binding.sequence.to_string(), label);
    let help = help::render(
        HelpTopic::for_context(BindingScope::Global),
        GrammarKind::Runyte,
        BindingScope::Global,
        &keymap,
        false,
    );
    assert!(help.contains(label));
    let mut hints = KeyHintState::default();
    hints.push(KeyStroke::parse("Cmd-c").unwrap());
    let rows = hints.rows(&keymap, Mode::Normal);
    assert!(rows.iter().any(|row| key_hint_keys(row).contains(label)));
}

#[test]
fn native_clipboard_defaults_keep_control_c_for_the_terminal() {
    let keymap = default_keymap();
    assert!(!keymap.terminal_clipboard_key(KeyStroke::ctrl('c')));
    assert!(keymap.terminal_clipboard_key(KeyStroke::ctrl('C')));
    assert_eq!(
        keymap.terminal_clipboard_key(KeyStroke::parse("Cmd-c").unwrap()),
        cfg!(target_os = "macos")
    );
    let paste = native_window::lookup(KeyStroke::ctrl('V')).unwrap();
    assert_eq!(paste.action, native_window::Action::PasteText);
    let command_paste = native_window::lookup(KeyStroke::parse("Cmd-v").unwrap());
    if cfg!(target_os = "macos") {
        let command_paste = command_paste.unwrap();
        assert_eq!(command_paste.action, paste.action);
        assert_eq!(
            command_paste.keys[0],
            KeyStroke::new(KeyCode::Char('v'), Modifiers::SUPER)
        );
        assert!(native_window::help().contains("Cmd-v / Ctrl-V"));
    } else {
        assert!(command_paste.is_none());
    }
    assert!(native_window::lookup(KeyStroke::ctrl('c')).is_none());
    assert!(native_window::lookup(KeyStroke::ctrl('v')).is_none());
}
