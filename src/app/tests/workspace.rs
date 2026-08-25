// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn session_attach_captures_the_editor_working_directory_for_relative_selectors() {
    let root = temporary("session-attach-working-directory");
    let editor_directory = root.join("nested");
    fs::create_dir_all(&editor_directory).unwrap();
    let editor_directory = editor_directory.canonicalize().unwrap();
    assert_ne!(std::env::current_dir().unwrap(), editor_directory);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.execute(crate::command::parse_named_command("cd", Some("nested")).unwrap())
        .unwrap();
    assert_eq!(app.working_directory, editor_directory);
    app.enable_persistent_session();

    app.execute(crate::command::parse_named_command("session-attach", Some("../project")).unwrap())
        .unwrap();

    assert_eq!(
        app.take_workspace_switch(),
        Some(WorkspaceSwitchRequest {
            selector: PathBuf::from("../project"),
            working_directory: editor_directory,
        })
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn standalone_can_list_sessions_but_cannot_attach_start_or_stop_them() {
    let root = temporary("standalone-session-list");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();

    app.execute_command("sl").unwrap();
    assert!(app.list.is_some(), "standalone listing was refused");
    assert_eq!(
        app.list.as_ref().map(|picker| picker.title.as_str()),
        Some("Sessions · Enter cannot attach in standalone mode · loading…")
    );
    let generation = app.workspace_generation;
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation,
        result: Ok(vec![WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("history".to_owned()),
            number: None,
            project_root: root.clone(),
            running: false,
            incompatible_protocol: None,
            unsaved_buffers: None,
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: None,
        }]),
    });
    let picker = app.list.as_ref().unwrap();
    assert_eq!(
        picker.items[0].detail,
        format!("{} · stopped", root.display())
    );
    assert_eq!(
        picker.title,
        "Sessions · Enter cannot attach in standalone mode · Tab actions"
    );
    assert_eq!(picker.primary_action, None);
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Sessions"))
        .expect("the session picker has a semantic overlay");
    assert_eq!(
        overlay.title,
        "Sessions · Enter cannot attach in standalone mode · Tab actions"
    );
    assert!(
        overlay
            .actions
            .iter()
            .all(|action| action.key_hint != "Enter")
    );

    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.status,
        "attaching sessions needs workspace.mode: persistent"
    );
    assert!(app.list.is_some(), "the refused row should remain visible");
    assert!(app.take_workspace_switch().is_none());

    app.execute_command(&format!("session-attach {}", root.display()))
        .unwrap();
    assert_eq!(
        app.status,
        "attaching sessions needs workspace.mode: persistent"
    );
    app.execute_command("session-start elsewhere").unwrap();
    assert_eq!(
        app.status,
        "starting sessions needs workspace.mode: persistent"
    );
    app.execute_command("session-stop").unwrap();
    assert_eq!(
        app.status,
        "stopping sessions needs workspace.mode: persistent"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn session_picker_keeps_filter_and_routes_enter_and_tab_by_workspace_identity() {
    let root = temporary("session-picker");
    let current = root.join("current");
    let stopped = root.join("stopped");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&stopped).unwrap();
    let current = current.canonicalize().unwrap();
    let stopped = stopped.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 4;
    let rows = vec![
        WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("current".to_owned()),
            number: None,
            project_root: current.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(2),
            pending_wait_requests: Some(0),
            live_terminals: Some(1),
            terminal_sessions: Some(1),
            interactive_attached: Some(true),
        },
        WorkspaceRow {
            id: "bbbbbbbbbbbbbbbb".to_owned(),
            name: Some("archive".to_owned()),
            number: None,
            project_root: stopped.clone(),
            running: false,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: Some(false),
        },
    ];
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 4,
        result: Ok(rows.clone()),
    });
    assert_eq!(
        app.list.as_ref().unwrap().items[0].detail,
        format!(
            "{} · running · unsaved 2 · terminals 1 · TUI attached",
            current.display()
        )
    );
    assert_eq!(
        app.list.as_ref().unwrap().primary_action.as_deref(),
        Some("attach")
    );
    app.list.as_mut().unwrap().filter = "archive".to_owned();
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 4,
        result: Ok(rows),
    });
    assert_eq!(app.list.as_ref().unwrap().filter, "archive");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(stopped.clone())
    );

    app.workspace_generation = 5;
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 5,
        result: Ok(vec![WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("current".to_owned()),
            number: None,
            project_root: current,
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(2),
            pending_wait_requests: Some(0),
            live_terminals: Some(0),
            terminal_sessions: Some(0),
            interactive_attached: Some(true),
        }]),
    });
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.session_action_menu.is_some());
    assert!(app.overlay_snapshots().iter().any(|overlay| {
        overlay.kind == crate::snapshot::OverlayKind::BufferActions
            && overlay.rows.iter().any(|row| row.label == "Close")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn session_picker_omits_counts_a_running_host_answers_with_zero() {
    let root = temporary("session-picker-zero-counts");
    let quiet = root.join("quiet");
    let exited = root.join("exited");
    fs::create_dir_all(&quiet).unwrap();
    fs::create_dir_all(&exited).unwrap();
    let quiet = quiet.canonicalize().unwrap();
    let exited = exited.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &quiet,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 6;
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 6,
        result: Ok(vec![
            WorkspaceRow {
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("quiet".to_owned()),
                number: None,
                project_root: quiet.clone(),
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: Some(0),
                live_terminals: Some(0),
                terminal_sessions: Some(0),
                interactive_attached: Some(true),
            },
            WorkspaceRow {
                id: "bbbbbbbbbbbbbbbb".to_owned(),
                name: Some("exited".to_owned()),
                number: None,
                project_root: exited.clone(),
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: Some(0),
                live_terminals: Some(0),
                terminal_sessions: Some(2),
                interactive_attached: Some(true),
            },
        ]),
    });
    // A host answering zero everywhere leaves the row reading as its path
    // and state; nothing is blank because the host failed to answer.
    assert_eq!(
        app.list.as_ref().unwrap().items[0].detail,
        format!("{} · running · TUI attached", quiet.display())
    );
    // Retained screens whose children have exited are still worth naming,
    // and they do not bring `terminals 0` back with them.
    assert_eq!(
        app.list.as_ref().unwrap().items[1].detail,
        format!(
            "{} · running · exited terminals 2 · TUI attached",
            exited.display()
        )
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn session_picker_marks_a_running_hosts_unanswered_health_as_unavailable() {
    let root = temporary("session-picker-health-unavailable");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 7;
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 7,
        result: Ok(vec![WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("unanswered".to_owned()),
            number: None,
            project_root: root.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: None,
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: None,
        }]),
    });

    assert_eq!(
        app.list.as_ref().unwrap().items[0].detail,
        format!("{} · running · health unavailable", root.display())
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn session_picker_rebuilds_with_the_selected_hosts_semantic_preview() {
    let root = temporary("session-picker-preview");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 3;
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 3,
        result: Ok(vec![WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("current".to_owned()),
            number: Some(1),
            project_root: root.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(1),
            pending_wait_requests: Some(0),
            live_terminals: Some(1),
            terminal_sessions: Some(1),
            interactive_attached: Some(false),
        }]),
    });

    // The isolated app has no socket service. Model the same result a
    // selected live host returns and verify that applying it preserves the
    // manager while replacing its loading/error text.
    app.workspace_previews.clear();
    app.workspace_preview_generation = 7;
    app.workspace_preview_target = Some(root.clone());
    app.apply_workspace_event(WorkspaceEvent::Previewed {
        generation: 7,
        path: root.clone(),
        result: Ok(SessionPreview {
            layout_panes: 2,
            panes: vec![crate::workspace::SessionPreviewPane {
                active: true,
                title: "[file] src/app.rs".to_owned(),
                kind: SessionPreviewPaneKind::Buffer {
                    dirty: true,
                    read_only: false,
                },
                start_line: Some(4381),
                lines: vec!["fn rebuild_workspace_picker(&mut self) {".to_owned()],
            }],
            omitted_panes: 0,
            other_resources: vec!["[terminal] cargo test".to_owned()],
            omitted_resources: 0,
        }),
    });

    let picker = app.list.as_ref().unwrap();
    assert!(picker.has_preview());
    assert_eq!(picker.preview_title(), Some("Session"));
    let preview = picker.selected_preview().unwrap();
    assert!(preview.contains("1 pane shown · 2 in layout"), "{preview}");
    assert!(preview.contains("● [file] src/app.rs [+]"), "{preview}");
    assert!(
        preview.contains("4381  fn rebuild_workspace_picker"),
        "{preview}"
    );
    assert!(
        preview.contains("Other: [terminal] cargo test"),
        "{preview}"
    );

    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Sessions"))
        .unwrap();
    assert_eq!(overlay.layout, crate::snapshot::OverlayLayout::Preview);
    assert!(overlay.show_preview);
    assert!(matches!(
        overlay.preview,
        Some(crate::snapshot::OverlayPreview::MatchedText { lines, .. })
            if lines.iter().any(|line| line.contains("src/app.rs"))
    ));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn the_session_list_marks_stopped_rows_dormant_without_hiding_or_reordering_them() {
    let root = temporary("session-dimming");
    let current = root.join("current");
    let stopped = root.join("stopped");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&stopped).unwrap();
    let current = current.canonicalize().unwrap();
    let stopped = stopped.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 2;
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 2,
        result: Ok(vec![
            WorkspaceRow {
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("current".to_owned()),
                number: None,
                project_root: current,
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(true),
            },
            WorkspaceRow {
                id: "bbbbbbbbbbbbbbbb".to_owned(),
                name: Some("archive".to_owned()),
                number: None,
                project_root: stopped,
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: None,
            },
        ]),
    });

    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Sessions"))
        .expect("the session list should be open");
    // Both rows are present and in the order the catalog gave them:
    // dimming is the only difference the stopped one carries.
    assert_eq!(overlay.rows.len(), 2);
    assert!(overlay.rows[0].label.contains("current"));
    assert!(!overlay.rows[0].dimmed);
    assert!(overlay.rows[1].label.contains("archive"));
    assert!(overlay.rows[1].dimmed);
    // Dormant is not unavailable: Enter still starts a stopped session.
    assert!(overlay.rows[1].available);
    fs::remove_dir_all(root).unwrap();
}

/// Two numbered sessions and one unnumbered, in a persistent workspace.
#[cfg(unix)]
fn numbered_sessions(label: &str) -> (App, PathBuf, Vec<PathBuf>) {
    let root = temporary(label);
    let roots = ["current", "second", "spare"]
        .iter()
        .map(|name| {
            let path = root.join(name);
            fs::create_dir_all(&path).unwrap();
            path.canonicalize().unwrap()
        })
        .collect::<Vec<_>>();
    let mut app = App::new_in_isolated_project(
        &roots[0],
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 3;
    let numbers = [Some(1), Some(2), None];
    let names = ["runyte", "runyte-2", "spare"];
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 3,
        result: Ok(roots
            .iter()
            .zip(numbers)
            .zip(names)
            .map(|((project_root, number), name)| WorkspaceRow {
                id: format!("{name}00000000000000"),
                name: Some(name.to_owned()),
                number,
                project_root: project_root.clone(),
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(false),
            })
            .collect()),
    });
    (app, root, roots)
}

#[cfg(unix)]
#[test]
fn a_digit_attaches_to_a_numbered_session_from_the_manager() {
    let (mut app, root, roots) = numbered_sessions("session-number-attach");

    press(&mut app, '2');
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(roots[1].clone()),
        "the digit reaches the session holding that number, not the second row"
    );
    assert!(app.list.is_none(), "attaching closes the manager");
    fs::remove_dir_all(root).unwrap();
}

/// The reason a digit is a shortcut only while nothing has been typed.
/// Runyte's own default names and this repository's own worktree path both
/// contain digits, so filtering for them has to keep working.
#[cfg(unix)]
#[test]
fn a_digit_typed_into_a_filter_is_filter_text_rather_than_a_shortcut() {
    let (mut app, root, _roots) = numbered_sessions("session-number-filter");

    for character in "runyte-".chars() {
        press(&mut app, character);
    }
    press(&mut app, '2');
    assert!(
        app.take_workspace_switch().is_none(),
        "a digit after other text must not attach"
    );
    let list = app.list.as_ref().unwrap();
    assert_eq!(list.filter, "runyte-2");
    assert_eq!(
        list.visible_indices().len(),
        1,
        "the filter still narrows to the digit-bearing name"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn clearing_the_filter_arms_the_digit_shortcut_again() {
    let (mut app, root, roots) = numbered_sessions("session-number-rearm");

    press(&mut app, 'r');
    press(&mut app, '1');
    assert!(app.take_workspace_switch().is_none());
    assert_eq!(app.list.as_ref().unwrap().filter, "r1");

    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert!(app.list.as_ref().unwrap().filter.is_empty());
    press(&mut app, '1');
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(roots[0].clone()),
        "an emptied filter is the state the shortcut is armed in"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_digit_no_session_holds_reports_that_rather_than_attaching() {
    let (mut app, root, _roots) = numbered_sessions("session-number-missing");

    press(&mut app, '7');
    assert!(app.take_workspace_switch().is_none());
    assert!(
        app.status.contains("no session is numbered 7"),
        "unexpected message: {}",
        app.status
    );
    assert!(app.status_error);
    assert!(app.list.is_some(), "a wrong digit leaves the manager open");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn the_manager_number_action_prompts_with_the_number_a_session_already_has() {
    let (mut app, root, roots) = numbered_sessions("session-number-prompt");

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    app.session_action_menu.as_mut().unwrap().selected = 2;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.prompt_kind, PromptKind::SessionNumber);
    assert_eq!(app.command, "1", "the prompt opens on the current number");
    assert_eq!(
        app.session_number_target.as_deref(),
        Some(roots[0].as_path())
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.session_number_target.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_session_number_answer_accepts_one_to_nine_and_an_empty_clearing() {
    assert_eq!(parse_session_number("3"), Ok(Some(3)));
    assert_eq!(parse_session_number(" 9 "), Ok(Some(9)));
    assert_eq!(
        parse_session_number(""),
        Ok(None),
        "empty clears the number"
    );
    assert_eq!(parse_session_number("   "), Ok(None));
    assert!(parse_session_number("0").is_err());
    assert!(parse_session_number("10").is_err());
    assert!(parse_session_number("one").is_err());
}

#[cfg(unix)]
#[test]
fn workspace_actions_match_the_selected_session_state() {
    let root = temporary("workspace-actions");
    let current = root.join("current");
    let stopped = root.join("stopped");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&stopped).unwrap();
    let current = current.canonicalize().unwrap();
    let stopped = stopped.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 9;
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 9,
        result: Ok(vec![
            WorkspaceRow {
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("current".to_owned()),
                number: None,
                project_root: current,
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(true),
            },
            WorkspaceRow {
                id: "bbbbbbbbbbbbbbbb".to_owned(),
                name: Some("archive".to_owned()),
                number: None,
                project_root: stopped.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(false),
            },
        ]),
    });

    // A running row can be stopped but not forgotten.
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions,
        vec![
            SessionAction::Open,
            SessionAction::Rename,
            SessionAction::Number,
            SessionAction::Close,
            SessionAction::ForceClose,
        ]
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    // A stopped row can be forgotten but neither closed nor force closed.
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions,
        vec![
            SessionAction::Open,
            SessionAction::Rename,
            SessionAction::Number,
            SessionAction::Forget,
        ]
    );
    let labels = app
        .overlay_snapshots()
        .iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::BufferActions)
        .map(|overlay| {
            overlay
                .rows
                .iter()
                .map(|row| row.label.clone())
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(labels, vec!["Open", "Rename", "Number", "Forget"]);

    // No session service is attached in an isolated project, so the
    // request cannot be served; what matters here is that Forget asks to
    // forget rather than to stop, and that the picker stays open.
    app.session_action_menu.as_mut().unwrap().selected = 3;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.take_workspace_switch().is_none());
    assert!(app.list.is_some());

    app.workspace_generation = 10;
    app.apply_workspace_event(WorkspaceEvent::Forgotten {
        generation: 10,
        path: stopped.clone(),
        result: Ok(true),
    });
    assert!(app.session_action_menu.is_none());

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    app.session_action_menu.as_mut().unwrap().selected = 1;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.prompt_kind, PromptKind::SessionRename);
    assert_eq!(app.command, "archive");
    assert_eq!(
        app.session_rename_target.as_deref(),
        Some(stopped.as_path())
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.session_rename_target.is_none());
    fs::remove_dir_all(root).unwrap();
}

/// The menu's action list is fixed when it opens, but a refresh can change
/// the row underneath it, so Enter re-reads the live state before acting.
/// Force close additionally takes two presses.
#[cfg(unix)]
#[test]
fn session_actions_confirm_force_close_and_recheck_state_at_enter() {
    let root = temporary("workspace-action-state");
    let current = root.join("current");
    let stopped = root.join("stopped");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&stopped).unwrap();
    let current = current.canonicalize().unwrap();
    let stopped = stopped.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();

    // An isolated project has no session service, so a request that gets
    // past the state guards fails with that error rather than being
    // served. That is what separates "the guard refused" from "the guard
    // let it through".
    let rows = |running: bool| {
        vec![
            WorkspaceRow {
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("current".to_owned()),
                number: None,
                project_root: current.clone(),
                running,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(true),
            },
            WorkspaceRow {
                id: "bbbbbbbbbbbbbbbb".to_owned(),
                name: Some("archive".to_owned()),
                number: None,
                project_root: stopped.clone(),
                running: !running,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(false),
            },
        ]
    };
    let refresh = |app: &mut App, running: bool| {
        app.workspace_generation = app.workspace_generation.wrapping_add(1).max(1);
        let generation = app.workspace_generation;
        app.apply_workspace_event(WorkspaceEvent::Refreshed {
            generation,
            result: Ok(rows(running)),
        });
    };

    refresh(&mut app, true);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions[4],
        SessionAction::ForceClose
    );

    // The first Enter only arms the confirmation and keeps the menu open.
    app.session_action_menu.as_mut().unwrap().selected = 4;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_action_menu.as_ref().unwrap().force_armed);
    assert!(!app.status_error);
    assert!(
        app.status.contains("press Enter again to confirm"),
        "{}",
        app.status
    );

    // Moving in either direction disarms it, so the confirmation is never
    // inherited by whatever action the selection lands on.
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    assert!(!app.session_action_menu.as_ref().unwrap().force_armed);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_action_menu.as_ref().unwrap().force_armed);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert!(!app.session_action_menu.as_ref().unwrap().force_armed);

    // Arming again and confirming reaches the request itself.
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().selected_action(),
        Some(SessionAction::ForceClose)
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_action_menu.as_ref().unwrap().force_armed);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.status, "session service is unavailable");

    // A refresh can stop the row while its menu is open. The menu still
    // lists Close, but Enter reads the row as it is now and refuses.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions[3],
        SessionAction::Close
    );
    app.session_action_menu.as_mut().unwrap().selected = 3;
    refresh(&mut app, false);
    assert!(app.session_action_menu.is_some());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.status, "this session is already stopped");

    // Force close is refused the same way, and never arms for a row that
    // is no longer running.
    app.session_action_menu.as_mut().unwrap().selected = 4;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.session_action_menu.as_ref().unwrap().force_armed);
    assert_eq!(app.status, "this session is already stopped");

    // The mirror image: a stopped row's menu offers Forget, but a refresh
    // starts the session before Enter lands.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions[3],
        SessionAction::Forget
    );
    app.session_action_menu.as_mut().unwrap().selected = 3;
    refresh(&mut app, true);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.status, "stop this session before forgetting it");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_switch_requests_are_platform_guarded_persistent_and_preserve_dirty_hosts() {
    let root = temporary("workspace-switch-scratch");
    let current = root.join("current");
    let destination = root.join("destination");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&destination).unwrap();
    let current = current.canonicalize().unwrap();
    let destination = destination.canonicalize().unwrap();
    let file = current.join("note.txt");
    fs::write(&file, "saved\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();

    // Standalone mode never leaves its in-process host, regardless of
    // whether the only edit is in the scratchpad.
    seed(&mut app, "a note to self");
    assert!(app.buffers[0].dirty);
    assert!(!app.request_workspace_switch_for_platform(destination.clone(), false));
    assert!(app.take_workspace_switch().is_none());
    assert_eq!(
        app.status,
        crate::service_health::PERSISTENT_SESSION_UNSUPPORTED_REASON
    );
    assert!(!app.request_workspace_switch_for_platform(destination.clone(), true));
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("workspace.mode: persistent"));

    app.open_file(file).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved "));
    app.enable_persistent_session();
    assert!(app.request_workspace_switch_for_platform(destination.clone(), true));
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(destination)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_shift_d_confirms_cancels_and_removes_only_the_typed_path() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("general-worktree-remove");
    let current = root.join("current");
    let linked = root.join("linked");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&linked).unwrap();
    let current = current.canonicalize().unwrap();
    let linked = linked.canonicalize().unwrap();
    let worktree = |path: PathBuf, branch: &str| Worktree {
        path,
        head: Some("0123456789abcdef".to_owned()),
        branch: Some(format!("refs/heads/{branch}")),
        detached: false,
        bare: false,
        locked: None,
        prunable: None,
        missing: false,
        common_dir: root.join("common"),
    };
    let rows = vec![
        worktree(current.clone(), "main"),
        worktree(linked.clone(), "feature"),
    ];
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&current))
            .with_branches(&["feature", "main"], "main")
            .with_branch_checkout("main", current.clone())
            .with_branch_checkout("feature", linked.clone())
            .with_worktrees(rows.clone()),
    );
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&current, ports).unwrap();
    app.open_git_worktrees_result(rows, true);
    let offset = app.active_buffer().line_to_offset(1);
    app.active_mut().replace_selection(Selection::point(offset));

    context_action(&mut app, 'D');
    assert_eq!(
        app.git_worktree_removal
            .as_ref()
            .map(|confirmation| confirmation.path.as_path()),
        Some(linked.as_path())
    );
    assert!(app.status.contains(&crate::git::display_path(&linked)));
    assert!(app.status.contains("Branch feature will remain"));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(provider.removed_worktrees().is_empty());
    assert!(linked.exists());

    context_action(&mut app, 'D');
    app.status("unrelated service feedback");
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Remove worktree");
    assert_eq!(overlay.actions[0].label, "remove worktree");
    let question = overlay
        .message
        .expect("worktree removal keeps its own confirmation question");
    assert!(question.contains(&crate::git::display_path(&linked)));
    assert!(question.contains("Branch feature will remain"));
    assert!(!question.contains("unrelated service feedback"));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(provider.removed_worktrees(), vec![linked.clone()]);
    assert!(
        provider
            .branches(&Repository::new(&current))
            .unwrap()
            .iter()
            .any(|branch| branch.name == "feature")
    );
    assert!(
        !app.active_buffer()
            .to_string()
            .contains(&crate::git::display_path(&linked))
    );
    assert!(app.status.contains("no branch was deleted"));

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn worktree_control_path_is_one_safe_row_and_confirmation_with_typed_identity() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("general-worktree-remove-control-path");
    let current = root.join("current");
    fs::create_dir_all(&current).unwrap();
    let current = current.canonicalize().unwrap();
    let linked = root.join(OsString::from_vec(b"linked-\n-\t-\\-\x1b-\xff".to_vec()));
    fs::create_dir_all(&linked).unwrap();
    let linked = linked.canonicalize().unwrap();
    let row = |path: PathBuf, branch: &str| Worktree {
        path,
        head: Some("0123456789abcdef".to_owned()),
        branch: Some(format!("refs/heads/{branch}")),
        detached: false,
        bare: false,
        locked: None,
        prunable: None,
        missing: false,
        common_dir: root.join("common"),
    };
    let rows = vec![row(current.clone(), "main"), row(linked.clone(), "feature")];
    let provider =
        Rc::new(MemoryGitProvider::new(Repository::new(&current)).with_worktrees(rows.clone()));
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&current, ports).unwrap();
    app.open_git_worktrees_result(rows, true);
    let text = app.active_buffer().to_string();
    let display = crate::git::display_path(&linked);
    assert!(
        display.ends_with("linked-\\n-\\t-\\-\\u{1b}-�"),
        "{display:?}"
    );
    assert_eq!(text.lines().count(), 2, "{text:?}");
    assert!(text.contains("\\n"), "{text:?}");
    assert!(text.contains("\\t"), "{text:?}");
    assert!(text.contains("\\u{1b}"), "{text:?}");
    let offset = app.active_buffer().line_to_offset(1);
    app.active_mut().replace_selection(Selection::point(offset));

    context_action(&mut app, 'D');
    let confirmation = app.git_worktree_removal.as_ref().unwrap();
    assert_eq!(confirmation.path, linked);
    let question = confirmation.message();
    assert!(
        question
            .chars()
            .all(|character| character == '\n' || !character.is_control())
    );
    assert_eq!(question.lines().count(), 3);
    assert!(question.contains("\\n"));
    assert!(question.contains("\\t"));
    assert!(question.contains("\\u{1b}"));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.status.chars().all(|character| !character.is_control()));
    assert!(app.status.contains("\\n"));
    assert!(provider.removed_worktrees().is_empty());

    context_action(&mut app, 'D');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(provider.removed_worktrees(), vec![linked]);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_removal_refuses_current_locked_bare_and_unavailable_rows_before_confirmation() {
    let root = temporary("general-worktree-remove-refusals");
    let current = root.join("current");
    let linked = root.join("linked");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&linked).unwrap();
    let current = current.canonicalize().unwrap();
    let linked = linked.canonicalize().unwrap();
    let row = |path: PathBuf| Worktree {
        path,
        head: Some("0123456789abcdef".to_owned()),
        branch: Some("refs/heads/feature".to_owned()),
        detached: false,
        bare: false,
        locked: None,
        prunable: None,
        missing: false,
        common_dir: root.join("common"),
    };
    let mut locked = row(linked.clone());
    locked.locked = Some("maintenance".to_owned());
    let mut bare = row(root.join("bare"));
    bare.bare = true;
    let mut missing = row(root.join("missing"));
    missing.missing = true;
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.open_git_worktrees_result(vec![row(current), locked, bare, missing], true);
    for (line, expected) in [
        (0, "workspace is using"),
        (1, "locked"),
        (2, "bare"),
        (3, "unavailable"),
    ] {
        let offset = app.active_buffer().line_to_offset(line);
        app.active_mut().replace_selection(Selection::point(offset));
        app.remove_selected_worktree();
        assert!(
            app.status_error && app.status.contains(expected),
            "{}",
            app.status
        );
        assert!(app.git_worktree_removal.is_none());
    }
    fs::remove_dir_all(root).unwrap();
}
