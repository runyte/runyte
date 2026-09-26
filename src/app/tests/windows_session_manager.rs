// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::workspace::{PublicationKey, WorkspaceRow, WorkspaceSelection};

fn row(project: &Path, tag: &[u8], name: &str) -> WorkspaceRow {
    WorkspaceRow {
        publication_key: Some(PublicationKey::for_test(tag)),
        unread_terminals: None,
        terminal_bell: None,
        id: "same-project-id".to_owned(),
        name: Some(name.to_owned()),
        number: None,
        last_active_unix_seconds: None,
        project_root: project.to_owned(),
        running: true,
        incompatible_protocol: None,
        unsaved_buffers: Some(0),
        open_buffers: Some(0),
        pending_wait_requests: Some(0),
        plugin_jobs: Some(0),
        activity_leases: Some(0),
        activities: Vec::new(),
        live_terminals: Some(0),
        terminal_sessions: Some(0),
        terminal_line_activity_unix_seconds: None,
        interactive_attached: Some(false),
        git: None,
        missing_directory: false,
    }
}

fn manager(rows: Vec<WorkspaceRow>) -> App {
    let root = temporary("native-manager-identity");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.workspace_generation = 1;
    app.list = Some(ListPicker::new("Sessions · loading…", Vec::new()));
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 1,
        result: Ok(rows),
    });
    app
}

fn persistent_manager(rows: Vec<WorkspaceRow>) -> App {
    let mut app = manager(rows);
    app.enable_persistent_session();
    app.rebuild_workspace_picker();
    app
}

#[test]
fn native_strip_keeps_same_project_publications_distinct_and_clicks_exact_row() {
    let project = temporary("native-strip-shared-project");
    let current = row(&project, b"current", "current");
    let other = row(&project, b"other", "other");
    let mut app = persistent_manager(vec![current.clone(), other.clone()]);
    app.list = None;
    app.current_native_publication = Some(current.selection());
    let strip = app.session_strip_snapshot().unwrap();
    assert_eq!(strip.entries.len(), 2);
    assert!(strip.entries[0].current);
    assert!(!strip.entries[1].current);

    let geometry = FrameGeometry {
        screen: Rect {
            x: 2,
            y: 3,
            width: 50,
            height: 12,
        },
        editor: Rect {
            x: 2,
            y: 3,
            width: 50,
            height: 10,
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    let view = app.prepare_view(geometry);
    let layout = view.session_strip.as_ref().unwrap().snapshot.layout(50);
    let click = PointerEvent {
        kind: PointerEventKind::Down(PointerButton::Left),
        column: 2 + layout.entries[1].cells.start as u16,
        row: 3,
        modifiers: Modifiers::NONE,
    };
    app.handle_pointer(click, &view).unwrap();
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(
        request.target,
        WorkspaceSwitchTarget::Selected(other.selection())
    );
    assert!(request.running_only);

    // The prepared frame owns its exact target even if a later catalog scan
    // publishes another host for the same directory and display name.
    app.workspace_rows[1] = row(&project, b"replacement", "other");
    app.handle_pointer(click, &view).unwrap();
    assert_eq!(
        app.take_workspace_switch().unwrap().target,
        WorkspaceSwitchTarget::Selected(other.selection())
    );

    let current_click = PointerEvent {
        column: 2 + layout.entries[0].cells.start as u16,
        ..click
    };
    app.handle_pointer(current_click, &view).unwrap();
    assert!(app.take_workspace_switch().is_none());
}

#[test]
fn native_strip_observation_keeps_rows_on_failure_and_respects_visibility() {
    let project = temporary("native-strip-observation");
    let current = row(&project, b"current", "current");
    let other = row(&project, b"other", "other");
    let mut app = persistent_manager(vec![current.clone(), other]);
    app.list = None;
    app.current_native_publication = Some(current.selection());
    app.apply_workspace_event(WorkspaceEvent::Observed {
        result: Err("unavailable".to_owned()),
    });
    let strip = app.session_strip_snapshot().unwrap();
    assert_eq!(strip.entries.len(), 2);
    assert!(strip.entries.iter().all(|entry| entry.health_unknown));

    app.config.workspace.session_strip = crate::config::SessionStripVisibility::Hidden;
    assert!(app.session_strip_snapshot().is_none());
    app.config.workspace.session_strip = crate::config::SessionStripVisibility::Always;
    app.maximized = Some(MaximizedPane {
        pane: app.active_pane,
        view: MaximizedView::Zen,
    });
    assert!(app.session_strip_snapshot().is_none());
    app.maximized = None;
    app.persistent_session = false;
    assert!(app.session_strip_snapshot().is_none());
}

#[test]
fn native_strip_observation_updates_closed_manager_and_fallback_target() {
    let mut app = persistent_manager(Vec::new());
    app.list = None;
    let current = row(&app.project_root, b"current", "current");
    let other = row(&app.project_root, b"other", "other");
    app.current_native_publication = Some(current.selection());
    assert!(app.session_strip_snapshot().is_none());

    let geometry = FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 12,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 10,
        },
        status: Rect::default(),
        message: Rect::default(),
    };
    app.config.workspace.session_strip = crate::config::SessionStripVisibility::Always;
    let view = app.prepare_view(geometry);
    assert_eq!(
        view.session_strip.as_ref().unwrap().targets,
        vec![current.selection()]
    );
    assert!(view.session_strip.as_ref().unwrap().snapshot.entries[0].current);

    app.config.workspace.session_strip = crate::config::SessionStripVisibility::Auto;
    app.apply_workspace_event(WorkspaceEvent::Observed {
        result: Ok(vec![other.clone()]),
    });
    assert_eq!(app.workspace_rows, vec![other.clone()]);
    let view = app.prepare_view(geometry);
    let strip = view.session_strip.unwrap();
    assert_eq!(strip.targets, vec![other.selection(), current.selection()]);
    assert_eq!(strip.snapshot.entries.len(), 2);
    assert!(!strip.snapshot.entries[0].current);
    assert!(strip.snapshot.entries[1].current);
}

#[test]
fn explorer_tab_s_queues_only_its_exact_directory_for_native_switching() {
    let project = temporary("native-explorer-session");
    let child = project.join("child");
    fs::create_dir_all(&child).unwrap();
    let mut app = App::new_in_isolated_project(
        &project,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.open_explorer(Some(child.clone())).unwrap();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Char('s'), Modifiers::NONE);
    let request = app
        .take_workspace_switch()
        .expect("Tab s selected directory");
    assert_eq!(request.target, WorkspaceSwitchTarget::UserSelector(child));
    assert!(!request.running_only);
    assert!(request.visit.is_none());
    assert!(!app.should_quit);
    assert_eq!(
        crate::command::CommandId::Editor(EditorCommand::OpenExplorerSession)
            .platform_unavailable(),
        None
    );
    assert_eq!(
        crate::command::CommandId::Colon(ColonCommand::SessionAttach).platform_unavailable(),
        None
    );
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn native_directory_chooser_accepts_windows_paths_and_queues_exact_directory() {
    use crate::command::{CommandExecutionContext, CommandInvocation};

    let root = temporary("native-directory-chooser");
    let child = root.join("child");
    fs::create_dir_all(&child).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.mode = Mode::Insert;
    let open = || {
        CommandInvocation::editor(
            EditorCommand::OpenSessionDirectory,
            CommandExecutionContext::default(),
        )
        .unwrap()
    };
    app.execute(open()).unwrap();
    assert!(app.session_directory_chooser_open());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.mode, Mode::Insert);
    assert!(app.take_workspace_switch().is_none());

    app.mode = Mode::Normal;
    key(&mut app, KeyCode::Char(':'), Modifiers::NONE);
    for character in "open-session-directory".chars() {
        key(&mut app, KeyCode::Char(character), Modifiers::NONE);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_directory_chooser_open());
    app.handle_input(InputEvent::Text(format!("{}\\", child.display())))
        .unwrap();
    assert!(app.list.as_ref().unwrap().title.contains("child"));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(request.target, WorkspaceSwitchTarget::UserSelector(child));
    assert!(!request.running_only);
    assert!(!app.should_quit);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_terminal_reported_directory_queues_only_validated_osc7_path() {
    let root = temporary("native-terminal-directory");
    let child = root.join("child");
    fs::create_dir_all(&child).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.open_terminal_at(Some(terminal_fixture_command()), root.clone());
    let terminal = app.active_terminal().unwrap();
    app.attach_terminal_reported_directory(terminal);
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("validated directory"));

    let url = url::Url::from_directory_path(&child).unwrap();
    app.apply_terminal_output(TerminalOutput::Bytes {
        id: terminal,
        bytes: format!("\x1b]7;{url}\x07").into_bytes(),
    });
    app.open_terminal_list();
    app.open_terminal_actions_for(terminal);
    let menu = app.terminal_action_menu.as_mut().unwrap();
    menu.selected = menu
        .actions
        .iter()
        .position(|action| *action == TerminalAction::OpenSessionDirectory)
        .unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let request = app.take_workspace_switch().unwrap();
    let WorkspaceSwitchTarget::UserSelector(path) = request.target else {
        panic!("terminal action must select its reported directory");
    };
    assert_eq!(path.canonicalize().unwrap(), child.canonicalize().unwrap());
    assert!(!request.running_only);
    assert!(!app.should_quit);
    close_test_terminal(&mut app, terminal);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_persistent_manager_visits_only_exact_running_selection() {
    let project = temporary("native-manager-visit");
    let first = row(&project, b"first", "first");
    let second = row(&project, b"second", "second");
    let expected = second.selection();
    let mut app = persistent_manager(vec![first, second]);
    assert_eq!(app.list.as_ref().unwrap().title, "Sessions");
    assert!(
        app.list_legend()
            .iter()
            .any(|action| action.key_hint == "1-9" && action.label == "attach")
    );
    assert_eq!(
        app.list.as_ref().unwrap().primary_action.as_deref(),
        Some("open")
    );
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(request.target, WorkspaceSwitchTarget::Selected(expected));
    assert!(request.running_only);
    assert!(request.visit.is_none());
    assert!(app.list.is_none());

    let mut app = persistent_manager(vec![row(&project, b"first", "first")]);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().selected_action(),
        Some(SessionAction::Open)
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.take_workspace_switch().is_some());
}

#[test]
fn native_numbered_shortcuts_and_manager_digits_select_exact_running_publications() {
    let project = temporary("native-numbered-navigation");
    let mut current = row(&project, b"current", "current");
    current.number = Some(1);
    let mut destination = row(&project, b"destination", "destination");
    destination.number = Some(2);
    let mut stopped = row(&project.join("stopped"), b"stopped", "stopped");
    stopped.running = false;
    stopped.publication_key = None;
    let mut app = persistent_manager(vec![current.clone(), destination.clone(), stopped]);
    app.current_native_publication = Some(current.selection());
    app.rebuild_workspace_picker();
    let labels = app
        .list
        .as_ref()
        .unwrap()
        .items
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();
    assert!(labels[0].starts_with("1 * current"));
    assert!(labels[1].starts_with("2   destination"));
    assert!(labels[2].starts_with("    stopped"));

    key(&mut app, KeyCode::Char('2'), Modifiers::NONE);
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(
        request.target,
        WorkspaceSwitchTarget::Selected(destination.selection())
    );
    assert!(request.running_only);
    assert!(!app.should_quit);

    app.rebuild_workspace_picker();
    key(&mut app, KeyCode::Char('x'), Modifiers::NONE);
    key(&mut app, KeyCode::Char('2'), Modifiers::NONE);
    assert_eq!(app.list.as_ref().unwrap().filter, "x2");
    assert!(app.take_workspace_switch().is_none());

    app.list = None;
    key(&mut app, KeyCode::Char(' '), Modifiers::NONE);
    key(&mut app, KeyCode::Char('2'), Modifiers::NONE);
    assert_eq!(
        app.take_workspace_switch().unwrap().target,
        WorkspaceSwitchTarget::Selected(destination.selection())
    );
    app.attach_numbered_session('1');
    assert!(app.take_workspace_switch().is_none());
    app.attach_numbered_session('3');
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("no session is numbered 3"));
    assert_eq!(
        crate::command::CommandId::Editor(EditorCommand::Session2).platform_unavailable(),
        None
    );
}

#[test]
fn native_cycle_and_previous_use_exact_identity_and_keep_current_attachment() {
    let project = temporary("native-cycle-navigation");
    let current = row(&project, b"current", "current");
    let other_publication = row(&project, b"other", "other");
    let third = row(&project.join("third"), b"third", "third");
    let mut app = persistent_manager(vec![
        current.clone(),
        other_publication.clone(),
        third.clone(),
    ]);
    app.list = None;
    app.current_native_publication = Some(current.selection());
    app.cycle_persistent_session(true);
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(
        request.target,
        WorkspaceSwitchTarget::Selected(other_publication.selection())
    );
    assert!(request.running_only);
    assert!(!app.should_quit);

    app.cycle_persistent_session(false);
    assert_eq!(
        app.take_workspace_switch().unwrap().target,
        WorkspaceSwitchTarget::Selected(third.selection())
    );
    key(&mut app, KeyCode::Right, Modifiers::SHIFT);
    assert_eq!(
        app.take_workspace_switch().unwrap().target,
        WorkspaceSwitchTarget::Selected(other_publication.selection())
    );
    app.workspace_rows = vec![current.clone()];
    app.cycle_persistent_session(true);
    assert!(app.take_workspace_switch().is_none());

    app.workspace_rows = vec![other_publication.clone()];
    app.cycle_persistent_session(true);
    assert_eq!(
        app.take_workspace_switch().unwrap().target,
        WorkspaceSwitchTarget::Selected(other_publication.selection())
    );
    key(&mut app, KeyCode::Char('w'), Modifiers::CONTROL);
    key(&mut app, KeyCode::Char('a'), Modifiers::NONE);
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(request.target, WorkspaceSwitchTarget::Previous);
    assert!(request.running_only);
    assert_eq!(
        crate::command::CommandId::Editor(EditorCommand::PreviousSession).platform_unavailable(),
        None
    );
    assert_eq!(
        crate::command::CommandId::Editor(EditorCommand::NextRunningSession).platform_unavailable(),
        None
    );
}

#[test]
fn native_session_attach_selector_uses_catalog_preparation_without_quitting_source() {
    let project = temporary("native-session-attach-selector");
    let mut app = persistent_manager(Vec::new());
    app.list = None;
    app.execute_colon_invocation_for_workspace_platform(
        ColonCommand::SessionAttach,
        InvocationParameters::Path(project.clone()),
        true,
    )
    .unwrap();
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(request.target, WorkspaceSwitchTarget::UserSelector(project));
    assert!(!request.running_only);
    assert!(!app.should_quit);
    assert_eq!(
        crate::command::CommandId::Colon(ColonCommand::SessionAttach).platform_unavailable(),
        None
    );
}

#[test]
fn native_catalog_number_updates_match_current_publication_not_project_root() {
    let project = temporary("native-number-exact-current");
    let mut other = row(&project, b"other", "other");
    other.number = Some(1);
    let mut current = row(&project, b"current", "current");
    current.number = Some(2);
    let mut app = persistent_manager(vec![other.clone(), current.clone()]);
    app.current_native_publication = Some(current.selection());
    app.list = None;
    app.apply_workspace_event(WorkspaceEvent::Observed {
        result: Ok(vec![other.clone(), current.clone()]),
    });
    assert_eq!(app.workspace_number, Some(2));
    app.workspace_generation = 2;
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 2,
        result: Ok(vec![other.clone(), current.clone()]),
    });
    assert_eq!(app.workspace_number, Some(2));
    app.rebuild_workspace_picker();
    app.apply_workspace_event(WorkspaceEvent::Polled {
        result: Ok(vec![other, current]),
    });
    assert_eq!(app.workspace_number, Some(2));
}

#[test]
fn native_pending_cycle_resumes_from_observation_and_reports_empty_or_failed_catalog() {
    let project = temporary("native-cycle-observation");
    let current = row(&project, b"current", "current");
    let destination = row(&project, b"destination", "destination");
    let mut app = persistent_manager(Vec::new());
    app.list = None;
    app.current_native_publication = Some(current.selection());
    app.session_navigation.pending_cycle = Some(true);
    app.apply_workspace_event(WorkspaceEvent::Observed {
        result: Ok(vec![current, destination.clone()]),
    });
    assert_eq!(
        app.take_workspace_switch().unwrap().target,
        WorkspaceSwitchTarget::Selected(destination.selection())
    );
    assert!(app.session_navigation.pending_cycle.is_none());

    app.session_navigation.pending_cycle = Some(true);
    app.apply_workspace_event(WorkspaceEvent::Observed {
        result: Ok(Vec::new()),
    });
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("no running sessions"));
    assert!(app.session_navigation.pending_cycle.is_none());

    app.session_navigation.pending_cycle = Some(false);
    app.apply_workspace_event(WorkspaceEvent::Observed {
        result: Err("catalog unavailable".to_owned()),
    });
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("catalog unavailable"));
    assert!(app.session_navigation.pending_cycle.is_none());
}

#[test]
fn native_destination_inventory_keeps_exact_selection_and_host_incarnation() {
    use crate::{
        protocol::{OpenDestination, OpenDestinationEntry},
        workspace::DestinationInventory,
    };

    let project = temporary("native-destination-inventory");
    let target = row(&project, b"target", "target");
    let current = row(&project, b"current", "current");
    let mut app = persistent_manager(vec![target.clone(), current.clone()]);
    app.current_native_publication = Some(current.selection());
    key(&mut app, KeyCode::Char('e'), Modifiers::CONTROL);
    assert!(app.session_inventory_open());
    let (generation, selection) = app.session_inventory_request().unwrap();
    assert_eq!(*selection, target.selection());
    let entry = OpenDestinationEntry {
        destination: OpenDestination::Buffer(1),
        label: "note.txt".to_owned(),
        detail: "buffer".to_owned(),
    };
    app.apply_workspace_event(WorkspaceEvent::Inventory {
        generation,
        path: project.clone(),
        selection: current.selection(),
        result: Ok(DestinationInventory {
            incarnation: "1".repeat(64),
            entries: vec![entry.clone()],
            truncated: false,
        }),
    });
    assert!(app.list.as_ref().unwrap().title.contains("unavailable"));
    app.apply_workspace_event(WorkspaceEvent::Inventory {
        generation,
        path: project,
        selection: target.selection(),
        result: Ok(DestinationInventory {
            incarnation: "2".repeat(64),
            entries: vec![entry],
            truncated: false,
        }),
    });
    assert_eq!(app.list.as_ref().unwrap().items[0].label, "note.txt");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(
        request.target,
        WorkspaceSwitchTarget::Selected(target.selection())
    );
    assert!(request.running_only);
    let visit = request.visit.unwrap();
    assert_eq!(visit.incarnation, "2".repeat(64));
    assert_eq!(visit.destination, super::OpenDestination::Buffer(0));
    assert!(!app.should_quit);
    assert!(!app.session_inventory_open());
}

#[test]
fn native_destination_inventory_refuses_replaced_row_and_restores_manager_safely() {
    use crate::{
        protocol::{OpenDestination, OpenDestinationEntry},
        workspace::DestinationInventory,
    };

    let project = temporary("native-destination-replaced");
    let target = row(&project, b"old", "old");
    let mut app = persistent_manager(vec![target.clone()]);
    app.current_native_publication = Some(target.selection());
    app.open_session_inventory();
    let (generation, selection) = app.session_inventory_request().unwrap();
    assert_eq!(*selection, target.selection());
    app.apply_workspace_event(WorkspaceEvent::Inventory {
        generation,
        path: project.clone(),
        selection: target.selection(),
        result: Ok(DestinationInventory {
            incarnation: "3".repeat(64),
            entries: vec![OpenDestinationEntry {
                destination: OpenDestination::Terminal(7),
                label: "shell".to_owned(),
                detail: "terminal".to_owned(),
            }],
            truncated: false,
        }),
    });
    app.workspace_rows[0] = row(&project, b"replacement", "replacement");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("selected session changed"));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(!app.session_inventory_open());
    assert!(app.session_manager_selection_lost);
    assert!(app.list.as_ref().unwrap().title.starts_with("Sessions"));
}

#[test]
fn native_manager_visit_refuses_stale_stopped_and_incompatible_rows() {
    let project = temporary("native-manager-visit-refusal");
    let old = row(&project, b"old", "old");
    let mut app = persistent_manager(vec![old]);
    app.apply_workspace_event(WorkspaceEvent::Polled {
        result: Ok(vec![row(&project, b"replacement", "replacement")]),
    });
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("selected session changed"));
    let replacement = app.workspace_rows[0].selection();
    app.visit_selected_native_session(replacement);
    assert!(app.take_workspace_switch().is_none());

    let mut stopped = row(&project, b"stopped", "stopped");
    stopped.running = false;
    stopped.publication_key = None;
    let mut app = persistent_manager(vec![stopped]);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(
        request.target,
        WorkspaceSwitchTarget::Selected(WorkspaceSelection::project_only(project.clone()))
    );
    assert!(!request.running_only);

    let mut incompatible = row(&project, b"incompatible", "incompatible");
    incompatible.incompatible_protocol = Some(999);
    let mut app = persistent_manager(vec![incompatible]);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("unsupported protocol"));
}

#[test]
fn native_manager_keeps_two_publications_and_captures_exact_menu_subject() {
    let project = temporary("native-manager-shared-project");
    let first = row(&project, b"first", "first");
    let second = row(&project, b"second", "second");
    let second_selection = second.selection();
    let mut app = manager(vec![first, second]);
    let picker = app.list.as_ref().unwrap();
    assert_eq!(picker.items.len(), 2);
    assert_eq!(
        picker.secondary_action,
        Some(("Tab".to_owned(), "actions".to_owned()))
    );
    assert!(!picker.title.contains("attach"));
    assert!(picker.primary_action.is_none());
    assert!(
        !app.list_legend()
            .iter()
            .any(|action| action.key_hint == "1-9")
    );
    assert!(picker.items.iter().all(|item| !item.label.contains('*')));
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let menu = app.session_action_menu.as_ref().unwrap();
    assert_eq!(menu.selection, second_selection);
    assert_eq!(
        menu.actions,
        vec![
            SessionAction::Rename,
            SessionAction::Number,
            SessionAction::Close,
            SessionAction::ForceClose,
            SessionAction::OpenDirectory,
            SessionAction::Destinations,
            SessionAction::Worktrees,
        ]
    );
}

#[test]
fn native_manager_refuses_stale_replacement_and_duplicate_full_identity() {
    let project = temporary("native-manager-replacement");
    let old = row(&project, b"old", "old");
    let mut app = manager(vec![old.clone()]);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().selection,
        old.selection()
    );
    app.apply_workspace_event(WorkspaceEvent::Polled {
        result: Ok(vec![row(&project, b"replacement", "replacement")]),
    });
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_action_menu.is_none());
    assert!(app.session_rename_target.is_none());
    assert!(app.session_manager_selection_lost);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.session_action_menu.is_none());
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.session_action_menu.is_some());

    let duplicate = row(&project, b"duplicate", "duplicate");
    let mut app = manager(vec![duplicate.clone(), duplicate]);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.session_action_menu.is_none());
}

#[test]
fn native_manager_enter_and_selectorless_stop_do_not_attach_or_choose_cwd() {
    let project = temporary("native-manager-enter");
    let mut app = manager(vec![row(&project, b"one", "one")]);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.list.is_some());
    assert!(app.take_workspace_switch().is_none());
    app.execute_colon_invocation_for_workspace_platform(
        ColonCommand::SessionStop,
        InvocationParameters::OptionalPath(None),
        false,
    )
    .unwrap();
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("explicit selector"));
}

#[test]
fn native_stopped_preview_and_incompatible_menu_make_no_attachment_claim() {
    let project = temporary("native-manager-stopped");
    let mut stopped = row(&project, b"stopped", "stopped");
    stopped.running = false;
    stopped.publication_key = None;
    let mut app = manager(vec![stopped]);
    let preview = app.list.as_ref().unwrap().selected_preview().unwrap();
    assert!(preview.contains("attachment is unavailable"));
    assert!(!preview.contains("starts the session"));
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions,
        vec![
            SessionAction::Rename,
            SessionAction::Forget,
            SessionAction::OpenDirectory,
            SessionAction::Worktrees,
        ]
    );

    let mut incompatible = row(&project, b"incompatible", "incompatible");
    incompatible.incompatible_protocol = Some(999);
    let mut app = manager(vec![incompatible]);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions,
        vec![
            SessionAction::ForceClose,
            SessionAction::OpenDirectory,
            SessionAction::Destinations,
            SessionAction::Worktrees,
        ]
    );
}

#[test]
fn native_persistent_stopped_row_opens_only_its_frozen_history_selection() {
    let project = temporary("native-manager-stopped-open");
    let mut stopped = row(&project, b"stopped", "stopped");
    stopped.running = false;
    stopped.publication_key = None;
    let selection = stopped.selection();
    let mut app = persistent_manager(vec![stopped.clone()]);
    let preview = app.list.as_ref().unwrap().selected_preview().unwrap();
    assert!(preview.contains("opening this row starts the session"));
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions,
        vec![
            SessionAction::Open,
            SessionAction::Rename,
            SessionAction::Forget,
            SessionAction::OpenDirectory,
            SessionAction::Worktrees,
        ]
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let request = app.take_workspace_switch().unwrap();
    assert_eq!(
        request.target,
        WorkspaceSwitchTarget::Selected(selection.clone())
    );
    assert!(!request.running_only);
    assert!(!app.should_quit);

    let mut app = persistent_manager(vec![stopped]);
    app.workspace_rows[0] = row(&project, b"replacement", "replacement");
    app.visit_selected_native_session(selection);
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("selected session changed"));
}

#[test]
fn native_manager_number_and_forget_actions_keep_frozen_row_identity() {
    let project = temporary("native-number-forget-menu");
    let live = row(&project, b"live", "live");
    let mut app = manager(vec![live.clone()]);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().selected_action(),
        Some(SessionAction::Number)
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.prompt_kind, PromptKind::SessionNumber);
    assert_eq!(app.session_number_target.as_ref(), Some(&live.selection()));
    app.workspace_rows[0] = row(&project, b"replacement", "replacement");
    key(&mut app, KeyCode::Char('2'), Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status.contains("selected session changed"));
    assert!(app.workspace_pending_selection.is_none());

    let mut stopped = row(&project, b"stopped", "stopped");
    stopped.running = false;
    stopped.publication_key = None;
    let mut app = manager(vec![stopped.clone()]);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().selected_action(),
        Some(SessionAction::Forget)
    );
    app.workspace_rows[0] = row(&project, b"new-live", "new-live");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status.contains("selected session changed"));
    assert!(app.workspace_pending_selection.is_none());
}

#[test]
fn native_warning_and_clean_completion_survive_intervening_refresh() {
    let project = temporary("native-manager-completion");
    let selected = row(&project, b"one", "one").selection();
    let mut app = manager(vec![row(&project, b"one", "one")]);
    app.workspace_pending_selection = Some((3, selected.clone()));
    app.workspace_generation = 4;
    app.apply_workspace_event(WorkspaceEvent::ControlWarnings {
        generation: 3,
        details: vec!["cache cleanup needs a retry".to_owned()],
        omitted: 0,
    });
    assert_eq!(app.workspace_pending_selection, Some((3, selected.clone())));
    assert!(
        app.notifications
            .entries()
            .iter()
            .any(|entry| entry.body.contains("cache cleanup"))
    );
    app.apply_workspace_event(WorkspaceEvent::Renamed {
        generation: 3,
        path: project,
        selection: Some(selected),
        name: "new".to_owned(),
        result: Ok(()),
    });
    assert!(app.workspace_pending_selection.is_none());

    app.workspace_pending_clean = Some(8);
    app.workspace_generation = 9;
    app.apply_workspace_event(WorkspaceEvent::Cleaned {
        generation: 8,
        result: Ok(1),
    });
    assert!(app.workspace_pending_clean.is_none());
}

#[test]
fn native_forget_completion_keeps_exact_pending_row_across_refresh() {
    let project = temporary("native-forget-pending-refresh");
    let mut stopped = row(&project, b"stopped", "stopped");
    stopped.running = false;
    stopped.publication_key = None;
    let selection = stopped.selection();
    let mut app = manager(vec![stopped]);
    app.workspace_pending_forget = Some((7, selection.clone()));
    app.workspace_generation = 8;
    app.apply_workspace_event(WorkspaceEvent::Forgotten {
        generation: 7,
        path: project.join("other"),
        result: Ok(true),
    });
    assert_eq!(app.workspace_pending_forget, Some((7, selection.clone())));
    app.apply_workspace_event(WorkspaceEvent::Forgotten {
        generation: 7,
        path: project.clone(),
        result: Err("history write failed".to_owned()),
    });
    assert!(app.workspace_pending_forget.is_none());
    assert!(app.status.contains("history write failed"));

    app.workspace_pending_forget = Some((9, selection));
    app.workspace_generation = 10;
    app.apply_workspace_event(WorkspaceEvent::Forgotten {
        generation: 9,
        path: project,
        result: Ok(true),
    });
    assert!(app.workspace_pending_forget.is_none());
    assert_eq!(app.workspace_generation, 11);
}
