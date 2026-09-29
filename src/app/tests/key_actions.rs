// SPDX-License-Identifier: MPL-2.0
use super::*;

fn configured(source: &str) -> App {
    let config = Config {
        keys: Some(serde_yaml::from_str(source).unwrap()),
        ..Config::default()
    };
    App::new(config, None).unwrap()
}

#[test]
fn action_binding_sequences_execute_and_group_selection_history() {
    let mut app = configured(
        "bind:\n  normal:\n    F6: [move-right, move-right]\n    F7: [select-all, yank]\n    F8: [select-all, enter-insert-mode]\n",
    );
    seed(&mut app, "abc def");
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 2);
    key(&mut app, KeyCode::Char('u'), Modifiers::ALT);
    assert_eq!(cursor(&app).col, 0);
    key(&mut app, KeyCode::Function(7), Modifiers::NONE);
    assert_eq!(app.registers[&'"'].text, "abc def");
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
}

#[test]
fn action_binding_counts_character_arguments_and_runtime_errors() {
    let mut app = configured(
        "bind:\n  normal:\n    F1: move-right\n    F2: [move-right, move-right]\n    F3: [move-right, find-next-char]\n    F4: [remove-primary-selection, move-right]\n",
    );
    seed(&mut app, "abc def");
    press(&mut app, '3');
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 3);
    press(&mut app, '2');
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 3);
    assert!(app.status_error);
    assert!(app.status.contains("multiple actions"));
    key(&mut app, KeyCode::Function(4), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 3);
    assert!(app.status_error);
    set_cursor(&mut app, 0, 0);
    key(&mut app, KeyCode::Function(3), Modifiers::NONE);
    assert_eq!(cursor(&app).col, 1);
    press(&mut app, 'e');
    assert_eq!(cursor(&app).col, 5);
}

#[test]
fn action_bindings_preserve_insert_replace_modes_and_render_full_help() {
    let mut app = configured(
        "bind:\n  normal:\n    F1: [select-all, yank]\n    F2: {command: help, argument: key-actions}\n  insert:\n    F3: insert-newline\n  replace:\n    F3: enter-normal-mode\n",
    );
    seed(&mut app, "abc");
    press(&mut app, 'i');
    key(&mut app, KeyCode::Function(3), Modifiers::NONE);
    assert_eq!(app.active_buffer().text().to_string(), "\nabc");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'R');
    key(&mut app, KeyCode::Function(3), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Normal);
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .contains("select-line-up")
    );
    app.execute_editor_command(EditorCommand::ShowHelp).unwrap();
    assert!(
        app.active_buffer()
            .text()
            .to_string()
            .contains("select-all → yank")
    );
}

#[test]
fn action_binding_overwrites_do_not_advertise_old_keys_in_teaching_text() {
    let mut app = configured(
        "bind:\n  normal:\n    Space e: yank\n    F1: open-explorer\n  select:\n    Space e: yank\n    F1: open-explorer\n",
    );
    app.execute_command("about").unwrap();
    let body = app.active_buffer().text().to_string();
    assert!(body.contains("F1"));
    assert!(!body.contains("Space e"));
}

#[test]
fn action_bindings_replay_once_per_recorded_key_and_keep_prompts_owned() {
    let mut app = configured(
        "bind:\n  normal:\n    F1: [move-right, move-right]\n    F2: [move-right, search]\n",
    );
    seed(&mut app, "abcdef");
    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    for stroke in [' ', 'm', 'm'] {
        press(&mut app, stroke);
    }
    assert_eq!(app.macros[&DEFAULT_MACRO_REGISTER].len(), 1);
    for stroke in [' ', 'm', 'r'] {
        press(&mut app, stroke);
    }
    finish_macro_replay(&mut app);
    assert_eq!(cursor(&app).col, 4);
    key(&mut app, KeyCode::Function(2), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Command);
    key(&mut app, KeyCode::Function(1), Modifiers::NONE);
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(cursor(&app).col, 5);
}
