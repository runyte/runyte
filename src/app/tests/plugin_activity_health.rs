// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::plugin_workflows::Instance,
    plugin::{self, activity, application as api},
    service_health::ActivityLeaseState,
};

fn with_activity(state: activity::State) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    let (sender, _receiver) = tokio::sync::mpsc::channel(32);
    let mut application = api::Instance::default();
    application.activities.insert(
        "a:test:1".into(),
        activity::Lease {
            info: activity::Info {
                lease: "a:test:1".into(),
                title: "Watch remote changes".into(),
                state,
                duration_seconds: 600,
            },
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(600),
        },
    );
    app.plugins.instances.insert(
        7,
        Instance {
            config: plugin::PluginConfig {
                settings: Default::default(),
                id: "remote-watch".into(),
                api: api::Api::Epoch2,
                capabilities: vec!["activity".into()],
                enabled: true,
                executable: "/nonexistent/plugin".into(),
                args: vec![],
                bindings: Default::default(),
            },
            sender: plugin::Sender::new(sender),
            registered: true,
            application,
            pending: None,
            issued: Default::default(),
            subscriptions: Default::default(),
            sequence: 0,
        },
    );
    app
}

#[test]
fn activity_health_protects_every_global_quit_and_force_discard_spelling() {
    for state in [activity::State::Active, activity::State::Cancelling] {
        for persistent in [false, true] {
            for command in [
                "quit",
                "q",
                "quit!",
                "q!",
                "quit-all",
                "qa",
                "quit-all!",
                "qa!",
                "quit-here",
                "qh",
                "quit-here!",
                "qh!",
            ] {
                let mut app = with_activity(state);
                app.persistent_session = persistent;
                app.quit_directory_handoff = true;
                app.execute_command(command).unwrap();
                assert!(
                    !app.should_quit,
                    "{command}, {state:?}, persistent={persistent}"
                );
                assert!(app.persistent_exit_request.is_none());
                assert!(app.quit_directory.is_none());
                assert!(app.status.contains("activity leases"), "{}", app.status);
                assert!(
                    app.status.contains("stop its owner in :plugins"),
                    "{}",
                    app.status
                );
                assert_eq!(app.status.contains(":detach"), persistent, "{}", app.status);
                app.plugins
                    .instances
                    .get_mut(&7)
                    .unwrap()
                    .application
                    .activities
                    .clear();
                app.execute_command("qa!").unwrap();
                assert!(app.should_quit);
            }
        }
    }
}

#[test]
fn activity_health_preserves_detach_and_reports_configured_owner_without_countdown() {
    let mut app = with_activity(activity::State::Active);
    let health = app.plugin_activity_health();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].owner, "remote-watch");
    assert_eq!(health[0].title, "Watch remote changes");
    assert_eq!(health[0].state, ActivityLeaseState::Active);
    let entry = app
        .service_health_snapshot()
        .entries
        .into_iter()
        .find(|entry| entry.service == "plugin activity")
        .unwrap();
    assert_eq!(entry.detail, "remote-watch · Watch remote changes · active");
    app.plugins
        .instances
        .get_mut(&7)
        .unwrap()
        .application
        .activities
        .get_mut("a:test:1")
        .unwrap()
        .info
        .state = activity::State::Cancelling;
    assert_eq!(
        app.plugin_activity_health()[0].state,
        ActivityLeaseState::Cancelling
    );
    let entry = app
        .service_health_snapshot()
        .entries
        .into_iter()
        .find(|entry| entry.service == "plugin activity")
        .unwrap();
    assert_eq!(
        entry.detail,
        "remote-watch · Watch remote changes · cancelling"
    );
    app.persistent_session = true;
    app.execute_command("detach").unwrap();
    assert!(app.should_quit);
    assert!(matches!(
        app.persistent_exit_request,
        Some(PersistentExitRequest::Detach)
    ));
    assert_eq!(app.plugin_activity_count(), 1);
}

#[test]
fn active_plugin_jobs_share_the_standalone_quit_guard() {
    let mut app = with_activity(activity::State::Active);
    let state = &mut app.plugins.instances.get_mut(&7).unwrap().application;
    state.activities.clear();
    state.jobs.insert(
        "j:1".into(),
        api::Job {
            job: "j:1".into(),
            title: "Download".into(),
            state: api::JobState::Cancelling,
            progress: 50,
        },
    );
    app.execute_command("qa!").unwrap();
    assert!(!app.should_quit);
    assert!(app.status.contains("1 plugin jobs"));
    assert!(app.status.contains(":plugins"));
    assert!(!app.status.contains(":detach"));
    app.plugins
        .instances
        .get_mut(&7)
        .unwrap()
        .application
        .jobs
        .get_mut("j:1")
        .unwrap()
        .state = api::JobState::Cancelled;
    app.execute_command("qa!").unwrap();
    assert!(app.should_quit);
}

#[cfg(unix)]
#[test]
fn activity_health_session_status_and_preview_keep_protected_owners_visible() {
    let mut row = WorkspaceRow {
        id: "test".into(),
        name: None,
        number: None,
        last_active_unix_seconds: None,
        project_root: PathBuf::from("/tmp/activity-health"),
        running: true,
        incompatible_protocol: None,
        unsaved_buffers: Some(0),
        open_buffers: Some(1),
        pending_wait_requests: Some(0),
        plugin_jobs: Some(2),
        activity_leases: Some(1),
        activities: with_activity(activity::State::Active).plugin_activity_health(),
        live_terminals: Some(1),
        terminal_sessions: Some(1),
        terminal_line_activity_unix_seconds: Some(0),
        unread_terminals: Some(0),
        terminal_bell: Some(false),
        interactive_attached: Some(false),
        git: None,
        missing_directory: false,
    };
    assert_eq!(terminal_output_status(&row, 1000), "ACTIVE");
    let preview = session_picker_preview(&row, None, false, "now");
    assert!(preview.contains("Plugin jobs  2"), "{preview}");
    assert!(preview.contains("Activities  1"), "{preview}");
    assert!(preview.contains("remote-watch · Watch remote changes · active"));
    row.activities[0].state = ActivityLeaseState::Cancelling;
    assert_eq!(terminal_output_status(&row, 1000), "CANCELLING");
    row.activities.clear();
    row.activity_leases = Some(0);
    assert_eq!(terminal_output_status(&row, 1000), "WORKING");
    row.plugin_jobs = Some(0);
    assert_eq!(terminal_output_status(&row, 1000), "QUIET");
    let mut app = with_activity(activity::State::Active);
    let mut working = row.clone();
    working.plugin_jobs = Some(1);
    let mut cancelling = row.clone();
    cancelling.activity_leases = Some(1);
    cancelling.activities = app.plugin_activity_health();
    cancelling.activities[0].state = ActivityLeaseState::Cancelling;
    app.workspace_rows = vec![row.clone(), working, cancelling];
    app.rebuild_workspace_picker();
    let picker = app.list.as_ref().unwrap();
    let trailing = picker
        .items
        .iter()
        .map(|item| item.trailing_detail.as_str())
        .collect::<Vec<_>>();
    assert!(trailing[0].ends_with("QUIET     "));
    assert!(trailing[1].ends_with("WORKING   "));
    assert!(trailing[2].ends_with("CANCELLING"));
    let widths = trailing
        .iter()
        .map(|text| unicode_width::UnicodeWidthStr::width(*text))
        .collect::<Vec<_>>();
    assert!(widths.iter().all(|width| *width == widths[0]));
    assert!(
        !app.refresh_workspace_activity_at(1000),
        "unchanged status must not trigger redraw"
    );
    app.workspace_rows[2].activities.clear();
    app.workspace_rows[2].activity_leases = Some(0);
    assert!(app.refresh_workspace_activity_at(1000));
    let picker = app.list.as_ref().unwrap();
    assert!(picker.items[0].trailing_detail.ends_with("QUIET  "));
    assert!(picker.items[1].trailing_detail.ends_with("WORKING"));
    assert!(!app.refresh_workspace_activity_at(1000));
    row.incompatible_protocol = Some(51);
    assert_eq!(terminal_output_status(&row, 1000), "");
}

#[test]
fn pending_plugin_state_storage_protects_quit_until_the_worker_settles() {
    let mut app = with_activity(activity::State::Active);
    let state = &mut app.plugins.instances.get_mut(&7).unwrap().application;
    state.activities.clear();
    state.state_pending = true;
    app.execute_command("qa!").unwrap();
    assert!(!app.should_quit);
    assert_eq!(app.plugin_active_job_count(), 1);
    app.plugins
        .instances
        .get_mut(&7)
        .unwrap()
        .application
        .state_pending = false;
    app.plugins.instances.remove(&7);
    app.plugins.state_orphans = 1;
    app.execute_command("qa!").unwrap();
    assert!(!app.should_quit);
    assert_eq!(app.plugin_active_job_count(), 1);
    app.plugins.state_orphans = 0;
    app.execute_command("qa!").unwrap();
    assert!(app.should_quit);
}
