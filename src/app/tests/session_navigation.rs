// SPDX-License-Identifier: MPL-2.0

use super::*;

fn selected_destination(app: &App) -> OpenDestination {
    match app.selected_list_action().expect("selected destination") {
        ListAction::Destination(destination) => destination,
        other => panic!("unexpected action {other:?}"),
    }
}

#[test]
fn navigator_fuzzy_matches_open_names_and_restores_opening_order() {
    let directory = temporary("navigator-open-names");
    fs::create_dir_all(directory.join("src")).unwrap();
    let parser = directory.join("src/parser.rs");
    fs::write(&parser, "secret_document_content\n").unwrap();
    fs::write(directory.join("unopened.rs"), "unopened").unwrap();
    let mut app = App::new(Config::default(), Some(parser)).unwrap();
    let parser = app.active().buffer;
    app.execute_command("buffer-new").unwrap();
    let scratch = app.active().buffer;
    seed(&mut app, "pathless");
    press(&mut app, ' ');
    press(&mut app, 'n');
    assert!(app.navigator_open());
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(scratch));
    let original = app
        .list
        .as_ref()
        .unwrap()
        .items
        .iter()
        .map(|item| item.index)
        .collect::<Vec<_>>();
    for character in "sprs".chars() {
        press(&mut app, character);
    }
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(parser));
    let picker = app.list.as_ref().unwrap();
    assert!(
        !picker
            .item_label_emphasis(picker.selected_item().unwrap())
            .is_empty()
    );
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Navigator —"))
        .unwrap();
    assert!(
        overlay.show_preview,
        "every destination list opens with its preview"
    );
    let row = &overlay.rows[overlay.selected.unwrap()];
    assert_eq!(
        row.emphasis
            .iter()
            .map(|position| row.label.chars().nth(*position).unwrap())
            .collect::<String>(),
        "sprs",
        "the rendered picker highlights fuzzy path matches"
    );
    key(&mut app, KeyCode::Char('t'), Modifiers::CONTROL);
    assert!(!app.list.as_ref().unwrap().show_preview);
    key(&mut app, KeyCode::Char('t'), Modifiers::CONTROL);
    let picker = app.list.as_ref().unwrap();
    assert!(picker.show_preview);
    assert!(
        picker
            .selected_preview()
            .unwrap()
            .contains("secret_document_content")
    );
    assert_eq!(picker.filter, "sprs");
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(parser));
    app.refresh_navigator();
    assert!(app.list.as_ref().unwrap().show_preview);
    key(&mut app, KeyCode::Char('t'), Modifiers::CONTROL);
    assert!(!app.list.as_ref().unwrap().show_preview);
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert_eq!(
        app.list
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|item| item.index)
            .collect::<Vec<_>>(),
        original
    );
    for character in "secret_document_content".chars() {
        press(&mut app, character);
    }
    assert!(app.list.as_ref().unwrap().selected_item().is_none());
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    for character in "unopened".chars() {
        press(&mut app, character);
    }
    assert!(app.list.as_ref().unwrap().selected_item().is_none());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.active().buffer, scratch);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn navigator_save_refreshes_metadata_and_preserves_order_actions_and_selection() {
    let directory = temporary("navigator-save-refresh");
    fs::create_dir_all(&directory).unwrap();
    for name in ["alpha.txt", "beta.txt", "gamma.txt"] {
        fs::write(directory.join(name), "original").unwrap();
    }
    let mut app = App::new(Config::default(), Some(directory.join("alpha.txt"))).unwrap();
    app.open_file(directory.join("beta.txt")).unwrap();
    let beta = app.active().buffer;
    app.apply_to_buffer(beta, &Transaction::insert(0, "edited "));
    app.open_file(directory.join("gamma.txt")).unwrap();
    app.open_navigator();
    let original_order = app
        .list
        .as_ref()
        .unwrap()
        .items
        .iter()
        .map(|item| item.index)
        .collect::<Vec<_>>();
    let actions = |app: &App| {
        app.list_actions
            .iter()
            .map(|action| match action {
                ListAction::Destination(destination) => *destination,
                other => panic!("unexpected action {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    let original_actions = actions(&app);
    for character in "txt".chars() {
        press(&mut app, character);
    }
    let picker = app.list.as_ref().unwrap();
    let beta_row = picker.visible_indices().iter().position(|index| {
        matches!(app.list_actions[picker.items[*index].index], ListAction::Destination(OpenDestination::Buffer(buffer)) if buffer == beta)
    }).unwrap();
    for _ in 0..beta_row {
        key(&mut app, KeyCode::Down, Modifiers::NONE);
    }
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(beta));
    assert!(
        app.list
            .as_ref()
            .unwrap()
            .selected_item()
            .unwrap()
            .trailing_detail
            .contains("[+]")
    );
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let menu = app.buffer_action_menu.as_mut().unwrap();
    menu.selected = menu
        .actions
        .iter()
        .position(|action| *action == BufferAction::Save)
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.buffers[beta].dirty);
    assert_eq!(
        fs::read_to_string(directory.join("beta.txt")).unwrap(),
        "edited original"
    );
    assert!(app.navigator_open());
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(beta));
    let picker = app.list.as_ref().unwrap();
    assert!(
        !picker
            .selected_item()
            .unwrap()
            .trailing_detail
            .contains("[+]")
    );
    assert_eq!(picker.filter, "txt");
    assert_eq!(
        picker
            .items
            .iter()
            .map(|item| item.index)
            .collect::<Vec<_>>(),
        original_order
    );
    assert_eq!(actions(&app), original_actions);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn navigator_printable_navigation_letters_filter_and_empty_space_dismisses() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_navigator();
    for character in "jkq".chars() {
        press(&mut app, character);
    }
    assert_eq!(app.list.as_ref().unwrap().filter, "jkq");
    press(&mut app, ' ');
    assert_eq!(app.list.as_ref().unwrap().filter, "jkq ");
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    press(&mut app, ' ');
    assert!(app.list.is_none());
}

#[test]
fn navigator_exact_basename_beats_recent_prefix_and_scattered_matches() {
    let directory = temporary("navigator-match-preference");
    fs::create_dir_all(&directory).unwrap();
    let names = [
        "parser",
        "parser_extra",
        "padded_archived_reference_source_extra_record",
    ];
    for name in names {
        fs::write(directory.join(name), "contents\n").unwrap();
    }
    let mut app = App::new(Config::default(), Some(directory.join(names[0]))).unwrap();
    let exact = app.active().buffer;
    app.open_file(directory.join(names[1])).unwrap();
    let prefix = app.active().buffer;
    app.open_file(directory.join(names[2])).unwrap();
    app.open_navigator();
    for character in "parser".chars() {
        press(&mut app, character);
    }
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(exact));
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(prefix));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn navigator_focuses_visible_buffer_and_bring_here_preserves_layout() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "first");
    let first = app.active().buffer;
    let first_pane = app.active_pane;
    app.split(Axis::Vertical, None).unwrap();
    let second_pane = app.active_pane;
    app.execute_command("buffer-new").unwrap();
    let second = app.active().buffer;
    assert!(app.visit_open_destination(OpenDestination::Buffer(first)));
    assert_eq!(app.active_pane, first_pane);
    assert_eq!(app.panes[&second_pane].buffer, second);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.visit_destination(OpenDestination::Buffer(second), true));
    assert_eq!(app.active_pane, first_pane);
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.panes[&first_pane].buffer, second);
    assert_eq!(app.panes[&second_pane].buffer, second);
    app.open_navigator();
    assert_eq!(
        app.open_destination_inventory()
            .iter()
            .filter(|item| item.destination == OpenDestination::Buffer(second))
            .count(),
        1
    );
    assert!(app.visit_open_destination(OpenDestination::Buffer(second)));
    assert_eq!(app.active_pane, first_pane, "active shared view wins");
}

#[cfg(unix)]
#[test]
fn navigator_terminal_prefix_remapping_cancel_and_focus_preserve_input_context() {
    let config = Config {
        keys: Some(serde_yaml::from_str("leader: Ctrl-x\nwindow: Ctrl-a\n").unwrap()),
        ..Config::default()
    };
    let mut app = App::new(config, None).unwrap();
    seed(&mut app, "document");
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    assert_eq!(app.mode, Mode::Insert);
    key(&mut app, KeyCode::Char('a'), Modifiers::CONTROL);
    press(&mut app, 'n');
    assert!(app.navigator_open());
    assert!(!app.terminals.get(terminal).unwrap().reviewing());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(app.active_terminal(), Some(terminal));
    assert!(!app.terminals.get(terminal).unwrap().reviewing());
    app.leave_terminal();
    key(&mut app, KeyCode::Char('x'), Modifiers::CONTROL);
    press(&mut app, 'n');
    assert!(app.navigator_open());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.terminals.get_mut(terminal).unwrap().begin_review();
    assert!(app.visit_open_destination(OpenDestination::Terminal(terminal)));
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.terminals.get(terminal).unwrap().reviewing());
}

#[cfg(unix)]
#[test]
fn navigator_finds_a_terminal_by_its_launch_command_without_showing_it() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.terminals
        .get_mut(terminal)
        .unwrap()
        .rename(Some("agent".to_owned()))
        .unwrap();
    app.open_navigator();
    for character in "cat".chars() {
        press(&mut app, character);
    }
    assert_eq!(
        selected_destination(&app),
        OpenDestination::Terminal(terminal)
    );
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Navigator —"))
        .unwrap();
    assert!(
        overlay.show_preview,
        "every destination list opens with its preview"
    );
    // The launch program moved to the preview; the row names the terminal.
    let row = &overlay.rows[overlay.selected.unwrap()];
    assert!(row.label.contains("agent"));
    assert!(!row.label.contains("cat"));
    assert!(row.detail.is_empty());
    assert!(overlay.preview.is_some());
}

#[cfg(unix)]
#[test]
fn navigator_refreshes_terminal_names_and_searchable_titles() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.open_navigator();
    let selected_index = app.list.as_ref().unwrap().selected_item().unwrap().index;
    app.terminals
        .get_mut(terminal)
        .unwrap()
        .rename(Some("renamed-agent".to_owned()))
        .unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: b"\x1b]2;updated-job-title\x07".to_vec(),
    });
    app.refresh_navigator();
    let item = app.list.as_ref().unwrap().selected_item().unwrap();
    assert_eq!(item.index, selected_index);
    assert!(item.label.contains("renamed-agent"));
    for character in "renamed-agent".chars() {
        press(&mut app, character);
    }
    assert_eq!(
        selected_destination(&app),
        OpenDestination::Terminal(terminal)
    );
    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    for character in "updated-job-title".chars() {
        press(&mut app, character);
    }
    assert_eq!(
        selected_destination(&app),
        OpenDestination::Terminal(terminal)
    );
}

#[cfg(unix)]
#[test]
fn navigator_removes_exited_terminal_without_changing_surviving_selection_identity() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "retained document");
    let buffer = app.active().buffer;
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.visit_open_destination(OpenDestination::Buffer(buffer));
    app.open_navigator();
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(buffer));
    app.apply_terminal_output(TerminalOutput::Exited {
        id: terminal,
        code: Some(0),
    });
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(buffer));
    assert!(!app.list.as_ref().unwrap().items.iter().any(|item| matches!(app.list_actions.get(item.index), Some(ListAction::Destination(OpenDestination::Terminal(id))) if *id == terminal)));
    assert!(
        app.terminals.get(terminal).is_some(),
        "exited output is retained for manager review"
    );
    app.closed_buffers.insert(buffer);
    app.refresh_navigator();
    assert!(app.list.as_ref().unwrap().items.is_empty());
}

#[cfg(unix)]
#[test]
fn navigator_selected_terminal_exit_requires_a_new_selection_before_accepting() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "retained document");
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.open_navigator();
    assert_eq!(
        selected_destination(&app),
        OpenDestination::Terminal(terminal)
    );
    app.apply_terminal_output(TerminalOutput::Exited {
        id: terminal,
        code: Some(0),
    });
    let before = app.active().destination();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(
        app.navigator_open(),
        "an Enter aimed at the retired identity cannot accept its replacement"
    );
    assert_eq!(app.active().destination(), before);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.buffer_action_menu.is_none());
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.navigator_open());
    assert!(app.active_terminal().is_none());
}

#[cfg(unix)]
#[test]
fn previous_destination_alternates_document_and_terminal_within_active_pane() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "document");
    let buffer = app.active().buffer;
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.previous_destination();
    assert_eq!(app.active_terminal(), None);
    assert_eq!(app.active().buffer, buffer);
    app.previous_destination();
    assert_eq!(app.active_terminal(), Some(terminal));
    app.leave_terminal();
    app.previous_destination();
    assert_eq!(
        app.active_terminal(),
        Some(terminal),
        "explicit terminal leave records its destination"
    );
}

#[cfg(unix)]
#[test]
fn navigator_visible_terminal_focus_and_explicit_bring_keep_one_pty_view() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "first document");
    let first = app.active_pane;
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.split(Axis::Vertical, None).unwrap();
    let second = app.active_pane;
    assert_ne!(first, second);
    app.visit_open_destination(OpenDestination::Terminal(terminal));
    assert_eq!(app.active_pane, first);
    assert_eq!(app.mode, Mode::Insert);
    assert_eq!(
        app.panes
            .values()
            .filter(|pane| pane.terminal == Some(terminal))
            .count(),
        1
    );
    app.activate_pane(second);
    app.visit_destination(OpenDestination::Terminal(terminal), true);
    assert_eq!(app.active_pane, second);
    assert_eq!(app.panes[&first].terminal, None);
    assert_eq!(app.panes[&second].terminal, Some(terminal));
    assert_eq!(app.panes.len(), 2);
}

#[cfg(unix)]
fn parent_wait_fixture(label: &str) -> (App, PathBuf, PathBuf, usize, TerminalId) {
    let directory = temporary(label);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("prompt.txt");
    fs::write(&path, "original\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let buffer = app.active().buffer;
    app.open_terminal_at(Some("/bin/cat".to_owned()), directory.clone());
    let terminal = app.active_terminal().unwrap();
    app.begin_parent_wait(terminal, &[buffer]).unwrap();
    app.set_parent_wait_buffers(HashMap::from([(buffer, 1)]));
    (app, directory, path, buffer, terminal)
}

#[cfg(unix)]
#[test]
fn parent_wait_save_and_return_routes_preserve_pane_and_live_terminal() {
    for (label, commands) in [
        ("parent-wait-wq", vec!["wq"]),
        ("parent-wait-wbc", vec!["wbc"]),
        ("parent-wait-w-q", vec!["w", "q"]),
    ] {
        let (mut app, directory, path, buffer, terminal) = parent_wait_fixture(label);
        app.apply_to_buffer(buffer, &Transaction::insert(0, "edited "));
        let pane = app.active_pane;
        for command in commands {
            app.execute_command(command).unwrap();
        }
        assert_eq!(fs::read_to_string(path).unwrap(), "edited original\n");
        assert_eq!(app.take_parent_wait_actions(), vec![(buffer, false)]);
        assert_eq!(app.active_pane, pane);
        assert_eq!(app.panes.len(), 1);
        assert_eq!(app.active_terminal(), Some(terminal));
        assert!(app.terminals.get(terminal).unwrap().live());
        assert!(!app.should_quit);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn parent_wait_write_alone_and_later_dirty_quit_keep_caller_waiting() {
    let (mut app, directory, _, buffer, terminal) =
        parent_wait_fixture("parent-wait-dirty-after-write");
    app.apply_to_buffer(buffer, &Transaction::insert(0, "saved "));
    app.execute_command("w").unwrap();
    assert!(app.take_parent_wait_actions().is_empty());
    app.apply_to_buffer(buffer, &Transaction::insert(0, "unsaved "));
    app.execute_command("q").unwrap();
    assert!(app.take_parent_wait_actions().is_empty());
    assert!(app.parent_wait_buffers.contains_key(&buffer));
    assert_eq!(app.active_terminal(), None);
    assert!(app.status.contains("unsaved changes"));
    app.execute_command("q!").unwrap();
    assert_eq!(app.take_parent_wait_actions(), vec![(buffer, true)]);
    assert_eq!(app.active_terminal(), Some(terminal));
    assert_eq!(app.buffers[buffer].to_string(), "saved original\n");
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn parent_wait_completion_after_navigation_returns_to_origin_and_preserves_shared_views() {
    let (mut app, directory, _, buffer, terminal) =
        parent_wait_fixture("parent-wait-navigation-return");
    let origin = app.active_pane;
    app.execute_command("buffer-new").unwrap();
    app.visit_open_destination(OpenDestination::Buffer(buffer));
    app.split(Axis::Vertical, None).unwrap();
    let shared = app.active_pane;
    app.execute_command("q").unwrap();
    assert_eq!(app.take_parent_wait_actions(), vec![(buffer, false)]);
    assert_eq!(app.active_pane, origin);
    assert_eq!(app.active_terminal(), Some(terminal));
    assert_eq!(app.panes[&shared].buffer, buffer);
    assert_eq!(app.panes.len(), 2);
    assert!(!app.closed_buffers.contains(&buffer));
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn parent_wait_shared_dirty_buffer_refuses_forced_cancellation() {
    let (mut app, directory, _, buffer, _) = parent_wait_fixture("parent-wait-shared-cancel");
    app.set_parent_wait_buffers(HashMap::from([(buffer, 2)]));
    app.apply_to_buffer(buffer, &Transaction::insert(0, "pending "));
    app.execute_command("q!").unwrap();
    assert!(app.take_parent_wait_actions().is_empty());
    assert_eq!(app.parent_wait_buffers[&buffer], 2);
    assert!(app.buffers[buffer].dirty);
    assert_eq!(app.buffers[buffer].to_string(), "pending original\n");
    assert!(app.status.contains("another pending request"));
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn terminal_manager_bulk_cleanup_uses_current_identities_beyond_the_filter() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "underlying document");
    app.open_terminal(Some("/bin/cat".to_owned()));
    let exited = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Exited {
        id: exited,
        code: Some(0),
    });
    app.split(Axis::Vertical, None).unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let late_exit = app.active_terminal().unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let live = app.active_terminal().unwrap();
    app.terminals
        .get_mut(live)
        .unwrap()
        .rename(Some("keep-running".to_owned()))
        .unwrap();
    app.open_terminal_list();
    for character in "keep-running".chars() {
        press(&mut app, character);
    }
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let menu = app.terminal_action_menu.as_mut().unwrap();
    menu.selected = menu
        .actions
        .iter()
        .position(|action| *action == TerminalAction::CloseExited)
        .unwrap();
    app.apply_terminal_output(TerminalOutput::Exited {
        id: late_exit,
        code: Some(0),
    });
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.terminals.get(exited).is_none());
    assert!(app.terminals.get(late_exit).is_none());
    assert!(app.terminals.get(live).unwrap().live());
    assert_eq!(app.terminals.len(), 1);
    assert!(
        app.panes
            .values()
            .all(|pane| pane.terminal != Some(exited) && pane.terminal != Some(late_exit))
    );
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.list.as_ref().unwrap().filter, "keep-running");
    assert!(app.list.as_ref().unwrap().selected_item().is_some());
    assert!(app.status.contains("closed 2 exited terminals"));
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let menu = app.terminal_action_menu.as_mut().unwrap();
    menu.selected = menu
        .actions
        .iter()
        .position(|action| *action == TerminalAction::CloseExited)
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status.contains("no exited terminals"));
    assert!(app.terminals.get(live).unwrap().live());
}

#[cfg(unix)]
#[test]
fn terminal_manager_cleaning_last_exited_entry_keeps_empty_manager_and_disabled_action() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let terminal = app.active_terminal().unwrap();
    app.apply_terminal_output(TerminalOutput::Exited {
        id: terminal,
        code: Some(0),
    });
    app.open_terminal_list();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let menu = app.terminal_action_menu.as_mut().unwrap();
    menu.selected = menu
        .actions
        .iter()
        .position(|action| *action == TerminalAction::CloseExited)
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.terminals.is_empty());
    let picker = app
        .list
        .as_ref()
        .expect("empty terminal manager remains open");
    assert!(picker.title.starts_with("Terminals — "));
    assert!(picker.selected_item().is_none());
    assert_eq!(app.panes.len(), 1);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title == "Terminal actions")
        .unwrap();
    let cleanup = overlay
        .rows
        .iter()
        .find(|row| row.label == "Close all exited terminals")
        .unwrap();
    assert!(!cleanup.available);
    assert_eq!(cleanup.detail, "No exited terminals to close");
    let menu = app.terminal_action_menu.as_mut().unwrap();
    menu.selected = menu
        .actions
        .iter()
        .position(|action| *action == TerminalAction::CloseExited)
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.terminals.is_empty());
    assert!(app.list.is_some());
    assert!(app.status.contains("no exited terminals"));
}

#[cfg(unix)]
#[test]
fn parent_wait_failed_save_does_not_finish_the_request() {
    let (mut app, directory, path, buffer, _) = parent_wait_fixture("parent-wait-save-failure");
    app.apply_to_buffer(buffer, &Transaction::insert(0, "edited "));
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    app.execute_command("wq").unwrap();
    assert!(app.take_parent_wait_actions().is_empty());
    assert!(app.parent_wait_buffers.contains_key(&buffer));
    assert!(app.buffers[buffer].dirty);
    assert!(app.active_terminal().is_none());
    assert!(!app.should_quit);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn navigator_aligns_types_titles_and_flags_in_terminal_cells() {
    use unicode_width::UnicodeWidthStr as _;
    let directory = temporary("navigator-columns");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("界.rs"), "source").unwrap();
    let mut app = App::new(Config::default(), Some(directory.join("界.rs"))).unwrap();
    app.apply_to_buffer(app.active().buffer, &Transaction::insert(0, "edited"));
    app.execute_command("help").unwrap();
    app.open_navigator();
    let items = &app.list.as_ref().unwrap().items;
    let file = items
        .iter()
        .find(|item| item.label[2..].starts_with("[file]"))
        .unwrap();
    let help = items
        .iter()
        .find(|item| item.label[2..].starts_with("[help]"))
        .unwrap();
    // A double-width name pads by cells, so both rows end in the same column
    // and their STATE runs line up after them.
    assert_eq!(file.label.width(), help.label.width());
    assert_eq!(file.trailing_detail.width(), help.trailing_detail.width());
    assert_eq!(file.trailing_detail.trim_end(), "[+]");
    assert!(help.trailing_detail.trim_end().ends_with("[RO]"));
    // NAME starts two columns after the widest TYPE on both rows, and that
    // is where an overlong name is shortened.
    let name = file.elide_from.unwrap();
    assert_eq!(help.elide_from, Some(name));
    assert_eq!(
        file.label.chars().take(name).collect::<String>().width(),
        2 + "[file]".width().max("[help]".width()) + 2
    );
    assert!(
        file.label
            .chars()
            .skip(name)
            .collect::<String>()
            .trim_end()
            .ends_with("界.rs")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn destination_elision_starts_at_the_name_after_a_unicode_type() {
    use unicode_width::UnicodeWidthStr as _;
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers.push(Buffer::virtual_text(
        "[界界e\u{301} output]",
        "captured output",
    ));
    app.open_buffer_picker();
    let items = &app.list.as_ref().unwrap().items;
    let generated = items
        .iter()
        .find(|item| item.label.contains("[界界e\u{301} output]"))
        .unwrap();
    let scratch = items
        .iter()
        .find(|item| item.label.contains("[scratch]"))
        .unwrap();
    let prefix = |item: &PickerItem| {
        item.label
            .chars()
            .take(item.elide_from.unwrap())
            .collect::<String>()
    };
    assert_eq!(prefix(generated).width(), prefix(scratch).width());
    assert_eq!(
        generated
            .label
            .chars()
            .skip(generated.elide_from.unwrap())
            .collect::<String>()
            .trim_end(),
        "界界e\u{301} output"
    );
    assert_eq!(
        scratch
            .label
            .chars()
            .skip(scratch.elide_from.unwrap())
            .collect::<String>()
            .trim_end(),
        "scratch"
    );
}

fn destinations(app: &App) -> Vec<OpenDestination> {
    let picker = app.list.as_ref().unwrap();
    picker
        .visible_indices()
        .iter()
        .map(
            |index| match &app.list_actions[picker.items[*index].index] {
                ListAction::Destination(destination) => *destination,
                other => panic!("unexpected action {other:?}"),
            },
        )
        .collect()
}

/// The Navigator, the buffer list, and the terminal list are one list over
/// three scopes: the same order, the same key legend, and the same Enter.
#[cfg(unix)]
#[test]
fn every_destination_list_orders_by_recent_activation_and_names_its_keys() {
    let directory = temporary("destination-list-order");
    fs::create_dir_all(&directory).unwrap();
    for name in ["alpha.txt", "beta.txt"] {
        fs::write(directory.join(name), name).unwrap();
    }
    let mut app = App::new(Config::default(), Some(directory.join("alpha.txt"))).unwrap();
    let alpha = app.active().buffer;
    app.open_file(directory.join("beta.txt")).unwrap();
    let beta = app.active().buffer;
    app.open_terminal(Some("/bin/cat".to_owned()));
    let first = app.active_terminal().unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let second = app.active_terminal().unwrap();
    app.leave_terminal();
    // Activation order, most recent last: second, beta, first, alpha.
    for destination in [
        OpenDestination::Terminal(second),
        OpenDestination::Buffer(beta),
        OpenDestination::Terminal(first),
        OpenDestination::Buffer(alpha),
    ] {
        assert!(app.visit_open_destination(destination));
        app.leave_terminal();
    }

    app.open_buffer_picker();
    let buffers = destinations(&app);
    assert_eq!(
        &buffers[..2],
        [
            OpenDestination::Buffer(alpha),
            OpenDestination::Buffer(beta)
        ]
    );
    assert!(
        buffers
            .iter()
            .all(|destination| matches!(destination, OpenDestination::Buffer(_)))
    );
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Buffers — "))
        .unwrap();
    assert!(overlay.show_preview);
    assert_eq!(
        overlay
            .actions
            .iter()
            .map(|action| format!("{} {}", action.key_hint, action.label))
            .collect::<Vec<_>>(),
        ["Enter visit", "Tab actions", "Esc close"]
    );
    assert!(
        overlay
            .legend
            .iter()
            .any(|action| action.key_hint == "Ctrl-t" && action.label == "preview")
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    // The exited terminal sinks below the running one in the terminal list,
    // and leaves the Navigator.
    let cleanup = terminal_cleanup(&app, first);
    app.apply_terminal_output(TerminalOutput::Exited {
        id: first,
        code: Some(0),
    });
    cleanup();
    app.open_terminal_list();
    assert_eq!(
        destinations(&app),
        [
            OpenDestination::Terminal(second),
            OpenDestination::Terminal(first)
        ]
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.open_navigator();
    let all = destinations(&app);
    assert!(!all.contains(&OpenDestination::Terminal(first)));
    let position = |destination| all.iter().position(|d| *d == destination).unwrap();
    assert!(
        position(OpenDestination::Buffer(beta)) < position(OpenDestination::Terminal(second)),
        "beta was activated after the second terminal"
    );
    close_test_terminals(&mut app);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_buffer_list_visits_a_buffer_where_a_pane_already_shows_it() {
    let directory = temporary("buffer-list-visit");
    fs::create_dir_all(&directory).unwrap();
    for name in ["left.txt", "right.txt"] {
        fs::write(directory.join(name), name).unwrap();
    }
    let mut app = App::new(Config::default(), Some(directory.join("left.txt"))).unwrap();
    let left_pane = app.active_pane;
    app.split(Axis::Vertical, Some(directory.join("right.txt")))
        .unwrap();
    let right_pane = app.active_pane;
    let right = app.active().buffer;
    app.activate_pane(left_pane);
    let left = app.active().buffer;

    // Fuzzy, like the Navigator: the letters need not be contiguous.
    app.open_buffer_picker();
    for character in "rgt".chars() {
        press(&mut app, character);
    }
    assert_eq!(selected_destination(&app), OpenDestination::Buffer(right));
    assert!(
        app.list
            .as_ref()
            .unwrap()
            .selected_item()
            .unwrap()
            .label
            .starts_with("* [file]"),
        "a buffer a pane shows is marked"
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(app.list.is_none());
    assert_eq!(app.active_pane, right_pane);
    assert_eq!(app.panes[&left_pane].buffer, left);
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn the_terminal_list_shows_an_exited_terminal_and_finds_terminals_by_id() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_terminal(Some("/bin/cat".to_owned()));
    let exited = app.active_terminal().unwrap();
    app.leave_terminal();
    // A second, running terminal the ID filter has to rule out.
    app.open_terminal(Some("/bin/cat".to_owned()));
    app.leave_terminal();
    let cleanup = terminal_cleanup(&app, exited);
    app.apply_terminal_output(TerminalOutput::Exited {
        id: exited,
        code: Some(0),
    });
    cleanup();

    // The ID left the rows but still filters; the pane title carries it.
    app.open_terminal_list();
    for character in format!("#{exited}").chars() {
        press(&mut app, character);
    }
    assert_eq!(destinations(&app), [OpenDestination::Terminal(exited)]);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active_terminal(), Some(exited));

    let view = app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 80,
            height: 24,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
    let title = app.snapshot(&view).panes[0].title.name.clone();
    assert!(
        title.starts_with(&format!("[terminal #{exited}] ")),
        "{title}"
    );
    assert!(title.ends_with("[exited]"), "{title}");
    close_test_terminals(&mut app);
}
