// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::workspace::{PublicationKey, WorkspaceRow};

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

#[test]
fn native_manager_keeps_two_publications_and_captures_exact_menu_subject() {
    let project = temporary("native-manager-shared-project");
    let first = row(&project, b"first", "first");
    let second = row(&project, b"second", "second");
    let second_selection = second.selection();
    let mut app = manager(vec![first, second]);
    let picker = app.list.as_ref().unwrap();
    assert_eq!(picker.items.len(), 2);
    assert!(picker.title.contains("Tab actions"));
    assert!(!picker.title.contains("attach"));
    assert!(picker.primary_action.is_none());
    assert!(picker.items.iter().all(|item| !item.label.contains('*')));
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let menu = app.session_action_menu.as_ref().unwrap();
    assert_eq!(menu.selection, second_selection);
    assert_eq!(
        menu.actions,
        vec![
            SessionAction::Rename,
            SessionAction::Close,
            SessionAction::ForceClose
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
        vec![SessionAction::Rename]
    );

    let mut incompatible = row(&project, b"incompatible", "incompatible");
    incompatible.incompatible_protocol = Some(999);
    let mut app = manager(vec![incompatible]);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions,
        vec![SessionAction::ForceClose]
    );
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
