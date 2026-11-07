// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    project_root::{CreatedBy, VersionControl, WorkspaceOrigin},
    test_support::TestRuntimeRoot,
};
use ratatui::{Terminal, backend::TestBackend};
use unicode_width::UnicodeWidthStr;

fn fixture() -> (App, TestRuntimeRoot) {
    let root = TestRuntimeRoot::new("workspace-echo").unwrap();
    let mut app = App::new_in_isolated_project(
        root.path(),
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.home_directory = Some(root.path().to_owned());
    app.project_root = root.join("project");
    app.state_root = app.project_root.join(".runyte");
    app.note_loaded_config(&root.join("config.yaml"));
    (app, root)
}

#[test]
fn workspace_opening_echo_preserves_every_discovery_origin_and_created_event() {
    for (origin, expected) in [
        (
            WorkspaceOrigin::VersionControl(VersionControl::Git),
            "Workspace ~/project (Git root)",
        ),
        (
            WorkspaceOrigin::VersionControl(VersionControl::Jujutsu),
            "Workspace ~/project (Jujutsu root)",
        ),
        (
            WorkspaceOrigin::VersionControl(VersionControl::Subversion),
            "Workspace ~/project (Subversion root)",
        ),
        (
            WorkspaceOrigin::Home,
            "Workspace ~/project (home workspace)",
        ),
        (
            WorkspaceOrigin::StateDirectory,
            "Workspace ~/project (.runyte)",
        ),
        (
            WorkspaceOrigin::Created(CreatedBy::Adoption),
            "Workspace ~/project (created, state in ~/project/.runyte)",
        ),
        (
            WorkspaceOrigin::Created(CreatedBy::Init),
            "Workspace ~/project (created, state in ~/project/.runyte)",
        ),
        (
            WorkspaceOrigin::Created(CreatedBy::AttachDirectory),
            "Workspace ~/project (created, state in ~/project/.runyte)",
        ),
    ] {
        let (mut app, _root) = fixture();
        app.note_workspace_origin(origin);
        app.finish_workspace_startup();
        let expected = expected.replace(
            "project/.runyte",
            &format!("project{}.runyte", std::path::MAIN_SEPARATOR),
        );
        assert_eq!(app.displayed_status_message(), expected);
        assert!(!app.displayed_status_message_is_error());
        if matches!(origin, WorkspaceOrigin::Created(_)) {
            assert!(
                app.notifications()
                    .entries()
                    .iter()
                    .any(|notice| notice.title == "Created a workspace")
            );
        }
        press(&mut app, 'g');
        press(&mut app, 'l');
        assert_eq!(app.displayed_status_message(), "g l (Move to line end)");
    }
}

#[test]
fn startup_notice_owns_the_line_through_finalization_and_deliberate_attachment() {
    for after_finalization in [false, true] {
        let (mut app, _root) = fixture();
        app.persistent_session = true;
        app.note_workspace_origin(WorkspaceOrigin::StateDirectory);
        if after_finalization {
            app.finish_workspace_startup();
        }
        let config: Config = serde_yaml::from_str("workspace:\n  mode: persistent\n").unwrap();
        app.note_config_deprecations(&config);
        app.finish_workspace_startup();
        app.note_workspace_opened();
        assert_eq!(
            app.displayed_status_message(),
            "workspace.mode is deprecated; replace it with mode: mux"
        );
        // A later ordinary notification retains its usual behavior once a
        // physical action has replaced the opening feedback.
        press(&mut app, 'g');
        press(&mut app, 'l');
        app.push_notification(NotificationDraft::new(
            NotificationSeverity::Warning,
            "Background",
            "Ordinary warning",
            "must not replace the action",
        ));
        assert_eq!(app.displayed_status_message(), "g l (Move to line end)");
    }
    let (mut app, _root) = fixture();
    app.note_workspace_origin(WorkspaceOrigin::StateDirectory);
    app.report_host_error("startup service failed");
    app.finish_workspace_startup();
    assert_eq!(app.displayed_status_message(), "startup service failed");
    assert!(app.displayed_status_message_is_error());
}

#[test]
fn editor_mode_and_existing_interaction_owners_suppress_workspace_echo() {
    let (mut app, _root) = fixture();
    app.enter_editor_mode();
    app.note_workspace_origin(WorkspaceOrigin::StateDirectory);
    app.finish_workspace_startup();
    app.note_workspace_opened();
    assert!(app.displayed_status_message().is_empty());
    let (mut app, _root) = fixture();
    app.note_workspace_origin(WorkspaceOrigin::StateDirectory);
    press(&mut app, 'g');
    let before = app.displayed_status_message().to_owned();
    app.finish_workspace_startup();
    assert_eq!(app.displayed_status_message(), before);
    assert_eq!(app.live_pending_display().as_deref(), Some("g …"));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, ':');
    app.note_workspace_opened();
    assert_eq!(app.mode, Mode::Command);
    assert!(!app.displayed_status_message().starts_with("Workspace "));
}

#[test]
fn mux_opening_echo_keeps_reason_and_session_visible_with_long_unicode_paths() {
    for origin in [
        WorkspaceOrigin::VersionControl(VersionControl::Git),
        WorkspaceOrigin::Created(CreatedBy::AttachDirectory),
    ] {
        let (mut app, root) = fixture();
        app.project_root = root.join("界á👩‍💻".repeat(80)).join("project");
        app.persistent_session = true;
        app.note_workspace_origin(origin);
        app.finish_workspace_startup();
        app.note_workspace_opened();
        let started = app.displayed_status_message();
        assert!(started.width() <= 80, "{started}");
        assert!(started.ends_with(", started a session"));
        assert!(
            started.contains(if matches!(origin, WorkspaceOrigin::Created(_)) {
                "(created, state in "
            } else {
                "(Git root)"
            })
        );
        app.note_workspace_opened();
        let attached = app.displayed_status_message();
        assert!(attached.width() <= 80, "{attached}");
        assert!(attached.ends_with(", attached to the running session"));
    }
}

#[test]
fn workspace_echo_is_identical_in_tui_native_and_wire_roundtrip_snapshots() {
    let (mut app, _root) = fixture();
    app.note_workspace_origin(WorkspaceOrigin::VersionControl(VersionControl::Git));
    app.finish_workspace_startup();
    let mut host = crate::workspace::WorkspaceHost::new(app);
    let geometry = crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 80, 8));
    let snapshot = host.prepare_frame(geometry);
    assert_eq!(
        snapshot.editor.status.interaction_line,
        "Workspace ~/project (Git root)"
    );
    let wire: crate::protocol::HostFrame = snapshot.clone().into();
    let wire: crate::protocol::HostFrame =
        serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap();
    let decoded: crate::workspace::HostFrame = wire.try_into().unwrap();
    let mut tui = Terminal::new(TestBackend::new(80, 8)).unwrap();
    tui.draw(|frame| {
        crate::ui::render(
            frame,
            host.app(),
            &snapshot.editor,
            &crate::key_hints::KeyHintState::default(),
            crate::ui::TerminalColorDepth::TrueColor,
        )
    })
    .unwrap();
    let mut native = Terminal::new(TestBackend::new(80, 8)).unwrap();
    native
        .draw(|frame| {
            crate::ui::render_native_frame(
                frame,
                &decoded,
                crate::ui::SessionMode::Standalone,
                crate::ui::TerminalColorDepth::TrueColor,
            )
        })
        .unwrap();
    let line = |terminal: &Terminal<TestBackend>| {
        (0..80)
            .map(|x| terminal.backend().buffer()[(x, 7)].symbol())
            .collect::<String>()
    };
    assert_eq!(line(&tui).trim_end(), "Workspace ~/project (Git root)");
    assert_eq!(line(&tui), line(&native));
}

#[test]
fn configuration_errors_from_editor_construction_keep_the_interaction_line() {
    let root = TestRuntimeRoot::new("startup-config-error").unwrap();
    let config: Config =
        serde_yaml::from_str("keys:\n  bind:\n    normal:\n      F5: no-such-command\n").unwrap();
    let mut trace = StartupTrace::new();
    let mut app = App::new_with_boundaries(
        config,
        Vec::new(),
        root.path().to_owned(),
        &mut trace,
        root.path().to_owned(),
        ProgramCache::default(),
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
        false,
        false,
    )
    .unwrap();
    let error = app.displayed_status_message().to_owned();
    assert!(app.displayed_status_message_is_error());
    assert!(error.contains("no-such-command"), "{error}");
    app.note_workspace_origin(WorkspaceOrigin::StateDirectory);
    app.finish_workspace_startup();
    assert_eq!(app.displayed_status_message(), error);
}

#[test]
fn configured_state_marker_is_named_in_the_workspace_echo() {
    let (mut app, _root) = fixture();
    app.config.workspace.state = PathBuf::from(".project-state");
    app.note_workspace_origin(WorkspaceOrigin::StateDirectory);
    app.finish_workspace_startup();
    assert_eq!(
        app.displayed_status_message(),
        "Workspace ~/project (.project-state)"
    );
    app.config.workspace.state = PathBuf::from("very-long-custom-state-marker-name");
    app.persistent_session = true;
    app.note_workspace_opened();
    assert!(app.displayed_status_message().width() <= 80);
    assert!(
        app.displayed_status_message()
            .contains("state-marker-name)")
    );
    assert!(app.displayed_status_message().contains("…"));
    assert!(!app.displayed_status_message().contains(".runyte"));
}

#[test]
fn created_origin_belongs_only_to_the_initial_persistent_opening() {
    for created in [
        CreatedBy::Adoption,
        CreatedBy::Init,
        CreatedBy::AttachDirectory,
    ] {
        let (mut app, _root) = fixture();
        app.persistent_session = true;
        app.note_workspace_origin(WorkspaceOrigin::Created(created));
        app.note_persistent_launch(true);
        app.finish_workspace_startup();
        app.note_workspace_opened();
        assert!(
            app.displayed_status_message()
                .contains("(created, state in ")
        );
        assert!(
            app.displayed_status_message()
                .ends_with(", started a session")
        );
        for _ in 0..2 {
            app.note_workspace_opened();
            assert_eq!(
                app.displayed_status_message(),
                "Workspace ~/project (.runyte), attached to the running session"
            );
        }
        assert_eq!(
            app.notifications()
                .entries()
                .iter()
                .filter(|notice| notice.title == "Created a workspace")
                .count(),
            1
        );
    }
}

#[test]
fn home_policy_is_captured_for_finder_and_search_without_changing_editor_mode() {
    use crate::{file_picker::ScanScope, workspace_state::StartupSafety};
    let fixture = TestRuntimeRoot::new("home-app").unwrap();
    let home = fixture.create_private_dir("home").unwrap();
    let config = Config {
        startup_safety: Some(Arc::new(StartupSafety::capture_with_homes(
            &[],
            || Some(fixture.join("state/runyte")),
            std::slice::from_ref(&home),
        ))),
        ..Config::default()
    };
    let mut app = App::new_in_project(config, None, &home).unwrap();
    assert!(app.home_workspace);
    assert!(!app.is_editor_mode());
    let excluded = cfg!(target_os = "macos").then(|| home.join("Library"));
    assert_eq!(
        app.ignoring_scan_scope(home.clone()),
        ScanScope::home(Some(home.clone()), excluded.clone())
    );
    assert_eq!(app.unfiltered_scan_scope(), ScanScope::home(None, excluded));
    let (service, requests) = crate::workspace_search::WorkspaceSearchService::controlled();
    app.attach_workspace_search(service);
    app.open_global_search("needle", SearchMode::Sensitive);
    let request = requests.recv().unwrap();
    assert!(request.contained);
    assert_eq!(request.excluded, app.home_scan_exclusion());
    app.home_directory = Some(fixture.join("different-environment-home"));
    app.note_workspace_origin(WorkspaceOrigin::Home);
    app.finish_workspace_startup();
    assert!(!app.displayed_status_message().contains("Workspace ~"));
    app.persistent_session = true;
    app.note_workspace_opened();
    app.note_workspace_opened();
    assert!(app.displayed_status_message().contains("home workspace"));
    assert!(
        app.displayed_status_message()
            .contains("attached to the running session")
    );
    assert!(!home.join(".runyte").exists());
    std::fs::create_dir(home.join(".git")).unwrap();
    std::fs::write(home.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let config = Config {
        startup_safety: Some(Arc::new(StartupSafety::capture_with_homes(
            &[],
            || Some(fixture.join("state/runyte")),
            std::slice::from_ref(&home),
        ))),
        ..Config::default()
    };
    let checkout = App::new_in_project(config, None, &home).unwrap();
    assert!(!checkout.home_workspace);
    assert_eq!(
        checkout.ignoring_scan_scope(home.clone()),
        ScanScope::ignoring(&home)
    );
    assert_eq!(checkout.state_root, app.state_root);
}

#[cfg(unix)]
#[test]
fn trusted_home_store_is_hidden_and_private_hardlink_provenance_survives_unlink() {
    use crate::{
        file_picker::{ScanScope, scan_files},
        workspace::context::storage::{ContextStorePolicy, Storage},
        workspace_state::StartupSafety,
    };
    let fixture = TestRuntimeRoot::new("home-context-private").unwrap();
    let home = fixture.create_private_dir("home").unwrap();
    let root = home.join(".cache/runyte/context");
    let store = Storage::new_with_policy(ContextStorePolicy::captured(
        root.clone(),
        std::slice::from_ref(&home),
        true,
    ))
    .unwrap();
    store.identity("agent").unwrap();
    let private_record = store.root().join("identity.json");
    let private_alias = home.join("private-alias.txt");
    std::fs::hard_link(&private_record, &private_alias).unwrap();
    let ordinary = home.join("ordinary.txt");
    std::fs::write(&ordinary, "public needle").unwrap();
    let ordinary_alias = home.join("ordinary-alias.txt");
    std::fs::hard_link(&ordinary, &ordinary_alias).unwrap();
    let config = || Config {
        startup_safety: Some(Arc::new(
            StartupSafety::capture_with_homes(
                &[],
                || Some(fixture.join("state/runyte")),
                std::slice::from_ref(&home),
            )
            .with_context_store(root.clone(), true),
        )),
        ..Config::default()
    };
    let mut private_app =
        App::new_in_project(config(), Some(private_alias.clone()), &home).unwrap();
    let public_app = App::new_in_project(config(), Some(ordinary_alias), &home).unwrap();
    assert!(!public_app.context_buffer_is_private(0));
    let scope = public_app.unfiltered_scan_scope();
    assert!(matches!(scope, ScanScope::Protected { .. }));
    let (entries, _, _) = scan_files(&home, &scope, &public_app.state_root, true).unwrap();
    assert!(entries.iter().any(|entry| entry.path == ordinary));
    assert!(!entries.iter().any(|entry| entry.path == private_alias));
    assert!(!entries.iter().any(|entry| entry.path.starts_with(&root)));
    assert!(scan_files(&root, &scope, &public_app.state_root, true).is_err());
    std::fs::remove_file(&private_record).unwrap();
    assert!(private_app.context_buffer_is_private(0));
    let scope = private_app.unfiltered_scan_scope();
    let (entries, _, _) = scan_files(&home, &scope, &private_app.state_root, true).unwrap();
    assert!(!entries.iter().any(|entry| entry.path == private_alias));
    let renamed_alias = home.join("renamed-private-alias.txt");
    std::fs::rename(&private_alias, &renamed_alias).unwrap();
    let scope = private_app.unfiltered_scan_scope();
    let (entries, _, _) = scan_files(&home, &scope, &private_app.state_root, true).unwrap();
    assert!(!entries.iter().any(|entry| entry.path == renamed_alias));
    // A reload accepts text from a second private inode into the same slot.
    // Both observed private identities survive removal of their store links.
    let second_record = store.root().join("second-record");
    std::fs::write(&second_record, "credential B").unwrap();
    let second_alias = home.join("second-private-alias.txt");
    std::fs::hard_link(&second_record, &second_alias).unwrap();
    private_app.buffers[0].path = Some(second_alias.clone());
    private_app.discard_buffer_changes(0).unwrap();
    std::fs::remove_file(&second_record).unwrap();
    let renamed_second_alias = home.join("renamed-second-private-alias.txt");
    std::fs::rename(&second_alias, &renamed_second_alias).unwrap();
    std::fs::write(&second_alias, "public credential C").unwrap();
    let scope = private_app.unfiltered_scan_scope();
    let (entries, _, _) = scan_files(&home, &scope, &private_app.state_root, true).unwrap();
    assert!(!entries.iter().any(|entry| entry.path == renamed_alias));
    assert!(
        !entries
            .iter()
            .any(|entry| entry.path == renamed_second_alias)
    );
    assert!(entries.iter().any(|entry| entry.path == second_alias));
    let (service, requests) = crate::workspace_search::WorkspaceSearchService::controlled();
    private_app.attach_workspace_search(service);
    private_app.open_global_search("credential", SearchMode::Sensitive);
    let request = requests.recv().unwrap();
    assert!(request.open_buffers.is_empty());
    assert!(request.private_paths.is_empty());
    assert_eq!(request.private_identities.len(), 2);
    let (matches, _) = crate::workspace_search::perform(request, || false)
        .unwrap()
        .unwrap();
    assert!(!matches.iter().any(|result| result.path == renamed_alias));
    assert!(
        !matches
            .iter()
            .any(|result| result.path == renamed_second_alias)
    );
    assert!(matches.iter().any(|result| result.path == second_alias));
}

#[cfg(unix)]
#[test]
fn ordinary_hardlink_stays_visible_until_a_private_store_link_appears() {
    use crate::{
        workspace::context::storage::{ContextStorePolicy, Storage},
        workspace_state::StartupSafety,
    };
    let fixture = TestRuntimeRoot::new("home-context-late-store").unwrap();
    let home = fixture.create_private_dir("home").unwrap();
    let root = home.join(".cache/runyte/context");
    let ordinary = home.join("ordinary.txt");
    std::fs::write(&ordinary, "public until linked privately").unwrap();
    let alias = home.join("ordinary-alias.txt");
    std::fs::hard_link(&ordinary, &alias).unwrap();
    let safety = StartupSafety::capture_with_homes(
        &[],
        || Some(fixture.join("state/runyte")),
        std::slice::from_ref(&home),
    )
    .with_context_store(root.clone(), true);
    let config = Config {
        startup_safety: Some(Arc::new(safety)),
        ..Config::default()
    };
    let app = App::new_in_project(config, Some(alias), &home).unwrap();
    assert!(!app.context_buffer_is_private(0));
    let store = Storage::new_with_policy(ContextStorePolicy::captured(
        root,
        std::slice::from_ref(&home),
        true,
    ))
    .unwrap();
    let private_link = store.root().join("new-record");
    std::fs::hard_link(&ordinary, &private_link).unwrap();
    assert!(app.context_buffer_is_private(0));
    std::fs::remove_file(private_link).unwrap();
    assert!(app.context_buffer_is_private(0));
}

#[cfg(unix)]
#[test]
fn explicit_context_store_remains_private_without_home_admission() {
    use crate::{workspace::context::storage::Storage, workspace_state::StartupSafety};
    let fixture = TestRuntimeRoot::new("explicit-context-private").unwrap();
    let home = fixture.create_private_dir("home").unwrap();
    let project = fixture.create_private_dir("project").unwrap();
    let root = home.join("private-context");
    let store = Storage::new(root.clone()).unwrap();
    store.identity("agent").unwrap();
    let safety = StartupSafety::capture_with_homes(
        &[],
        || Some(fixture.join("state/runyte")),
        std::slice::from_ref(&home),
    )
    .with_context_store(root, false);
    let config = Config {
        startup_safety: Some(Arc::new(safety)),
        ..Config::default()
    };
    let app =
        App::new_in_project(config, Some(store.root().join("identity.json")), &project).unwrap();
    assert!(app.context_buffer_is_private(0));
}
