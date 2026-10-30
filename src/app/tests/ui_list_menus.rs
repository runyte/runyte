// SPDX-License-Identifier: MPL-2.0
use super::*;

fn configure(app: &mut App, owner: &str, accept: &str) {
    let source = format!(
        "ui: {{{owner}: {{accept: [{accept}], next: [F6], previous: [F5], back: [F7], cancel: [F10]}}}}"
    );
    let compiled =
        crate::keymap::configured::compile(&serde_yaml::from_str(&source).unwrap(), &app.keymap);
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    app.keymap = compiled.keymap;
}

#[test]
fn configured_buffer_menu_actions_preserve_target_and_stale_back_cancel_distinction() {
    let mut app = App::new(Config::default(), None).unwrap();
    let target = app.active().buffer;
    app.insert_text("retained target");
    app.open_scratch_buffer();
    assert_ne!(app.active().buffer, target);
    configure(&mut app, "buffer-actions", "F8");
    app.open_navigator();
    app.buffer_action_menu = Some(BufferActionMenu {
        buffer: target,
        actions: vec![BufferAction::BringHere, BufferAction::Discard],
        selected: 0,
    });
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.buffer_action_menu.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    assert_eq!(app.buffer_action_menu.as_ref().unwrap().selected, 1);
    key(&mut app, KeyCode::Function(5), Modifiers::NONE);
    assert_eq!(app.buffer_action_menu.as_ref().unwrap().selected, 0);
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::BufferActions)
        .unwrap();
    assert!(snapshot.actions.iter().any(|a| a.key_hint == "F8"));
    app.navigator_selection_lost = true;
    app.handle_list_key(KeyStroke::parse("F7").unwrap())
        .unwrap();
    assert!(app.navigator_selection_lost && app.buffer_action_menu.is_some());
    app.handle_list_key(KeyStroke::parse("F8").unwrap())
        .unwrap();
    assert!(app.navigator_selection_lost && app.buffer_action_menu.is_some());
    app.handle_list_key(KeyStroke::parse("F10").unwrap())
        .unwrap();
    assert!(!app.navigator_selection_lost && app.buffer_action_menu.is_none());
    assert!(app.list.is_some());
    app.buffer_action_menu = Some(BufferActionMenu {
        buffer: target,
        actions: vec![BufferAction::BringHere],
        selected: 0,
    });
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.list.is_none() && app.buffer_action_menu.is_none());
    assert_eq!(app.active().buffer, target);
}

#[test]
#[cfg(unix)]
fn configured_terminal_menu_arming_disarming_and_stale_target_follow_actions() {
    for accept in ["F8", "Esc"] {
        let root = temporary("configured-terminal-menu");
        fs::create_dir_all(&root).unwrap();
        let mut app = App::new_in_isolated_project(
            &root,
            HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
                String::new(),
            ))))),
        )
        .unwrap();
        app.config.editor.auto_close_terminal = false;
        app.open_terminal(Some(terminal_fixture_command()));
        let id = app.active_terminal().unwrap();
        app.open_terminal_list();
        configure(&mut app, "terminal-actions", accept);
        app.terminal_action_menu = Some(TerminalActionMenu {
            id,
            actions: vec![TerminalAction::ForceKill, TerminalAction::Rename],
            selected: 0,
            close_armed: false,
        });
        let accept_key = KeyStroke::parse(accept).unwrap();
        app.handle_input(InputEvent::Key(accept_key)).unwrap();
        assert!(app.terminal_action_menu.as_ref().unwrap().close_armed);
        assert!(app.status.contains(&format!("press {accept} again")));
        assert!(app.input_requires_presented_approval(&InputEvent::Key(accept_key)));
        assert!(
            !app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(
                KeyCode::Enter
            )))
        );
        let confirmation = app
            .overlay_snapshots()
            .into_iter()
            .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
            .unwrap();
        assert!(confirmation.actions.iter().any(|a| a.key_hint == accept));
        let compiled = crate::keymap::configured::compile(&serde_yaml::from_str(&format!("ui: {{terminal-actions: {{accept: [{accept}], next: [F6], previous: [F5], back: [F7], cancel: []}}}}" )).unwrap(), &app.keymap);
        assert!(compiled.errors.is_empty());
        app.keymap = compiled.keymap;
        let confirmation = app
            .overlay_snapshots()
            .into_iter()
            .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
            .unwrap();
        assert!(
            confirmation
                .message
                .as_ref()
                .unwrap()
                .contains("F7 cancels.")
        );
        assert!(!confirmation.message.as_ref().unwrap().contains("unbound"));
        assert!(
            confirmation
                .actions
                .iter()
                .any(|a| a.label == "cancel" && a.key_hint == "F7")
        );
        app.buffer_discard_confirmation = Some(app.active().buffer);
        assert!(
            app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(
                KeyCode::Enter
            )))
        );
        app.buffer_discard_confirmation = None;
        key(&mut app, KeyCode::Down, Modifiers::NONE);
        assert!(
            app.terminal_action_menu.as_ref().unwrap().close_armed,
            "removed navigation is inert"
        );
        key(&mut app, KeyCode::Function(6), Modifiers::NONE);
        assert!(!app.terminal_action_menu.as_ref().unwrap().close_armed);
        key(&mut app, KeyCode::Function(5), Modifiers::NONE);
        app.handle_input(InputEvent::Key(accept_key)).unwrap();
        assert!(app.terminal_action_menu.as_ref().unwrap().close_armed);
        app.apply_terminal_output(TerminalOutput::Exited { id, code: Some(0) });
        app.handle_input(InputEvent::Key(accept_key)).unwrap();
        assert!(!app.terminal_action_menu.as_ref().unwrap().close_armed);
        assert!(app.status.contains("no longer running"));
        key(&mut app, KeyCode::Function(7), Modifiers::NONE);
        assert!(app.terminal_action_menu.is_none() && app.list.is_some());
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[cfg(unix)]
fn configured_hidden_terminal_menu_close_arms_and_accepts_only_effective_key() {
    let root = temporary("configured-hidden-terminal-close");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.open_terminal(Some(terminal_fixture_command()));
    let id = app.active_terminal().unwrap();
    app.open_scratch_buffer();
    assert_ne!(app.active_terminal(), Some(id));
    app.open_terminal_list();
    configure(&mut app, "terminal-actions", "F8");
    app.terminal_action_menu = Some(TerminalActionMenu {
        id,
        actions: vec![TerminalAction::Close],
        selected: 0,
        close_armed: false,
    });
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.terminal_action_menu.as_ref().unwrap().close_armed);
    assert!(app.status.contains("press F8 again"));
    assert!(
        app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::parse("F8").unwrap()))
    );
    assert!(
        !app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.terminals.get(id).is_some());
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.terminals.get(id).is_none());
    assert!(app.terminal_action_menu.is_none());
    drop(app);
    fs::remove_dir_all(root).unwrap();
}
