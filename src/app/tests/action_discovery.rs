// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::command::discovery;

fn app() -> App {
    App::new(Config::default(), None).unwrap()
}
fn inspect(app: &mut App, keys: &[char]) {
    app.execute_command(discovery::DESCRIBE_KEY).unwrap();
    for &c in keys {
        press(app, c);
    }
}

#[test]
fn describing_edit_quit_and_operand_keys_never_executes_them() {
    for keys in [vec!['d'], vec!['p'], vec!['f'], vec![' ', 'q']] {
        let mut app = app();
        seed(&mut app, "unchanged");
        let buffer = app.active().buffer;
        inspect(&mut app, &keys);
        assert_eq!(app.buffers[buffer].text().to_string(), "unchanged");
        assert!(!app.should_quit);
        assert!(
            app.active_buffer()
                .text()
                .to_string()
                .contains("Inspection did not execute")
        );
        assert!(app.grammar.pending_sequence().is_empty());
    }
}

#[test]
fn describe_prefix_escape_paste_and_changed_context_cancel_without_edits() {
    let mut app = app();
    seed(&mut app, "abc");
    inspect(&mut app, &[' ']);
    assert!(app.describe_key.is_some());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.describe_key.is_none());
    assert_eq!(app.active_buffer().text().to_string(), "abc");
    inspect(&mut app, &[]);
    app.handle_input(InputEvent::Text("pasted text".into()))
        .unwrap();
    assert_eq!(app.active_buffer().text().to_string(), "abc");
    inspect(&mut app, &[]);
    app.mode = Mode::Select;
    press(&mut app, 'd');
    assert!(app.describe_key.is_none());
    assert_eq!(app.active_buffer().text().to_string(), "abc");
}

#[test]
fn inspected_keys_are_not_recorded_in_a_macro() {
    let mut app = app();
    app.start_describe_key();
    app.recording_macro = Some('a');
    let before = app.macros.get(&'a').cloned();
    press(&mut app, 'd');
    assert_eq!(app.macros.get(&'a').cloned(), before);
    assert!(app.macro_staging.is_empty());
}

#[test]
fn palette_discovery_remembers_insert_mode_and_terminal_child_routing() {
    let mut app = app();
    let id = app.terminals.insert_test_session(80, 24);
    app.active_mut().terminal = Some(id);
    app.mode = Mode::Insert;
    app.open_prompt_with_value(PromptKind::Command, discovery::DESCRIBE_KEY.into());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
    press(&mut app, 'x');
    let text = app.active_buffer().text().to_string();
    assert!(text.contains("Mode: insert"), "{text}");
    assert!(text.contains("Nothing was sent"), "{text}");
}

#[test]
fn action_browser_inspects_without_running_and_filters_using_scoped_admission() {
    let mut app = app();
    seed(&mut app, "abc");
    let buffer = app.active().buffer;
    app.execute_command(discovery::ACTION_BROWSER).unwrap();
    let list = app.list.as_mut().unwrap();
    for c in "delete-selection".chars() {
        list.push_filter(c);
    }
    assert!(list.selected_preview().is_some());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.buffers[buffer].text().to_string(), "abc");
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .contains("Character operands")
    );
    assert!(!discovery::admitted_here(
        CommandId::Editor(EditorCommand::DeleteSelection),
        Mode::Insert,
        BindingScope::Terminal
    ));
    assert!(discovery::admitted_here(
        CommandId::Editor(EditorCommand::DirectoryTreeOpen),
        Mode::Normal,
        BindingScope::DirectoryTree
    ));
}

#[test]
fn describe_exact_prefix_waits_for_longer_chord_and_context_changes_cancel() {
    use crate::keymap::{Binding, Keymap};
    let mut app = app();
    app.keymap = std::sync::Arc::new(
        Keymap::new(vec![
            Binding::implemented(
                &[Mode::Normal],
                KeySequence::parse("z").unwrap(),
                EditorCommand::DeleteSelection,
            ),
            Binding::implemented(
                &[Mode::Normal],
                KeySequence::parse("z z").unwrap(),
                EditorCommand::Yank,
            ),
        ])
        .unwrap(),
    );
    seed(&mut app, "abc");
    let buffer = app.active().buffer;
    inspect(&mut app, &['z']);
    assert!(app.describe_key.is_some());
    assert_eq!(app.active().selection, Selection::point(0));
    press(&mut app, 'z');
    assert!(app.describe_key.is_none());
    assert_eq!(app.buffers[buffer].text().to_string(), "abc");
    assert!(app.active_buffer().text().to_string().contains("yank"));
    app.start_describe_key();
    app.keymap = std::sync::Arc::new(crate::keymap::default_keymap().clone());
    press(&mut app, 'd');
    assert!(app.describe_key.is_none());
}

#[test]
fn describe_tree_key_focuses_result_but_cancel_preserves_tree_focus() {
    let mut app = app();
    app.directory_tree.focused = true;
    inspect(&mut app, &[]);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.directory_tree.focused);
    inspect(&mut app, &['j']);
    assert!(!app.directory_tree.focused);
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .contains("directory-tree")
    );
}

#[test]
fn copy_inspected_binding_uses_clipboard_and_new_capture_clears_it() {
    let mut app = app();
    let clipboard = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    app.set_system_clipboard(Box::new(MemoryClipboard(clipboard.clone())));
    inspect(&mut app, &['d']);
    app.execute_command(discovery::COPY_BINDING).unwrap();
    assert!(clipboard.lock().unwrap().contains("delete-selection"));
    app.start_describe_key();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.described_binding.is_none());
}

#[test]
fn plugin_binding_is_identified_without_claiming_builtin_source_or_yaml() {
    use crate::keymap::{Binding, BindingTarget, Keymap};
    let mut app = app();
    app.keymap = std::sync::Arc::new(
        Keymap::new(vec![Binding::implemented(
            &[Mode::Normal],
            KeySequence::parse("x").unwrap(),
            BindingTarget::Plugin(42),
        )])
        .unwrap(),
    );
    inspect(&mut app, &['x']);
    let text = app.active_buffer().text().to_string();
    assert!(text.contains("runtime plugin action 42"), "{text}");
    assert!(!text.contains("Source: built-in"));
    assert!(app.described_binding.is_none());
}
