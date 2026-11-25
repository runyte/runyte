// SPDX-License-Identifier: MPL-2.0
use super::*;

fn choose(app: &mut App, count: usize) {
    for _ in 0..count {
        key(app, KeyCode::Down, Modifiers::NONE);
    }
    key(app, KeyCode::Enter, Modifiers::NONE);
}

#[test]
fn list_close_dirty_cancel_discard_and_captured_target() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.insert_text("unsaved");
    let target = app.active().buffer;
    app.open_buffer_picker();
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert_eq!(app.list_close.as_ref().unwrap().selected, 0);
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.closed_buffers.contains(&target));
    assert!(app.list_close.is_none());
    app.request_buffer_close(target);
    app.open_scratch_buffer();
    let other = app.active().buffer;
    choose(&mut app, 2);
    assert!(app.closed_buffers.contains(&target));
    assert!(!app.closed_buffers.contains(&other));
}

#[test]
fn list_close_changed_or_disappearing_buffer_never_discards() {
    for disappear in [false, true] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.insert_text("before");
        let id = app.active().buffer;
        app.request_buffer_close(id);
        if disappear {
            app.close_buffer_discarding(id);
        } else {
            app.insert_text("after");
        }
        choose(&mut app, 2);
        assert_eq!(app.closed_buffers.contains(&id), disappear);
        assert!(app.status.contains("changed or closed"));
    }
}

#[test]
fn list_close_saves_file_and_untitled_buffer_only_after_success() {
    let root = temporary("list-close-save");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("saved.txt");
    fs::write(&path, "before").unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_file(path.clone()).unwrap();
    app.insert_text("after");
    let id = app.active().buffer;
    let expected = app.buffers[id].text().to_string();
    app.request_buffer_close(id);
    choose(&mut app, 1);
    assert!(app.closed_buffers.contains(&id));
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    app.open_scratch_buffer();
    app.insert_text("scratch");
    let id = app.active().buffer;
    let revision = app.buffers[id].revision();
    app.request_buffer_close(id);
    choose(&mut app, 1);
    assert!(matches!(app.prompt_kind, PromptKind::SaveClosePath { .. }));
    assert!(!app.closed_buffers.contains(&id));
    let _ = app.save_close_path(id, revision, root.to_str().unwrap());
    assert!(!app.closed_buffers.contains(&id));
    app.save_close_path(id, revision, root.join("scratch.txt").to_str().unwrap())
        .unwrap();
    assert!(app.closed_buffers.contains(&id));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn list_close_remaps_and_filter_clearing_is_opt_in() {
    let mut app = App::new(Config::default(), None).unwrap();
    let source = serde_yaml::from_str(
        "scopes: {global: {list: {Delete: null, F8: list-close-target, F9: list-clear-filter}}}",
    )
    .unwrap();
    let result = crate::keymap::configured::compile(&source, &app.keymap);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    app.keymap = result.keymap;
    app.open_buffer_picker();
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert!(app.list_close.is_none());
    app.list.as_mut().unwrap().push_filter('x');
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert_eq!(app.list.as_ref().unwrap().filter, "x");
    key(&mut app, KeyCode::Function(9), Modifiers::NONE);
    assert!(app.list.as_ref().unwrap().filter.is_empty());
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.list_close.is_some());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.list_close.is_none());
}

#[test]
fn list_close_confirmation_requires_fresh_input_and_teaches_remapping() {
    let mut app = App::new(Config::default(), None).unwrap();
    let id = app.active().buffer;
    let source =
        serde_yaml::from_str("ui: {list-close: {accept: [F8], next: [F6], cancel: [F10]}}")
            .unwrap();
    let result = crate::keymap::configured::compile(&source, &app.keymap);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    app.keymap = result.keymap;
    app.request_buffer_close(id);
    key(&mut app, KeyCode::Function(6), Modifiers::NONE);
    let input = InputEvent::Key(KeyStroke::plain(KeyCode::Function(8)));
    assert!(app.input_requires_presented_approval(&input));
    app.handle_repeated_input(input.clone()).unwrap();
    assert!(!app.closed_buffers.contains(&id));
    app.handle_replayed_input(input).unwrap();
    assert!(!app.closed_buffers.contains(&id));
    let overlay = app.overlay_snapshots().pop().unwrap();
    assert!(overlay.actions.iter().any(|action| action.key_hint == "F8"));
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.closed_buffers.contains(&id));
}

#[test]
fn list_close_terminal_unknown_cancel_confirm_and_exited_removal() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.config.editor.auto_close_terminal = false;
    let id = app.terminals.insert_test_session(80, 24);
    app.open_terminal_list();
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert!(app.list_close.is_some());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.terminals.get(id).is_some());
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    choose(&mut app, 1);
    assert!(app.terminals.get(id).is_none());
    let id = app.terminals.insert_test_session(80, 24);
    app.apply_terminal_output(TerminalOutput::Exited { id, code: Some(0) });
    app.open_terminal_list();
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert!(app.list_close.is_none());
    assert!(app.terminals.get(id).is_none());
}

#[test]
fn list_close_terminal_input_invalidates_confirmation() {
    let mut app = App::new(Config::default(), None).unwrap();
    let id = app.terminals.insert_test_session(80, 24);
    app.request_terminal_close(id);
    app.terminals.get_mut(id).unwrap().send_text("new work");
    choose(&mut app, 1);
    assert!(app.terminals.get(id).is_some());
    assert!(app.status.contains("changed"));
}

#[test]
fn list_close_special_buffers_do_not_offer_ordinary_file_save() {
    let mut app = App::new(Config::default(), None).unwrap();
    let id = app.active().buffer;
    app.insert_text("unsaved special text");
    for kind in [BufferKind::Directory, BufferKind::CommitMessage] {
        app.buffers[id].kind = kind;
        app.request_buffer_close(id);
        assert_eq!(
            app.list_close.as_ref().unwrap().labels(),
            &["Cancel", "Discard changes and close"]
        );
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        assert!(app.buffers[id].dirty);
        assert!(!app.closed_buffers.contains(&id));
    }
}
