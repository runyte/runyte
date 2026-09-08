// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::plugin::manager::{Action, Entry, Intent, Phase};

fn entry(index: usize, id: &str, phase: Phase, owner: Option<usize>) -> Entry {
    Entry {
        config_index: index,
        configured_id: id.into(),
        enabled: phase != Phase::Disabled,
        valid: true,
        phase,
        owner,
        granted: vec!["views".into(), "jobs".into()],
        jobs: 1,
        activities: 2,
        helpers: 3,
        cleanup: 0,
        diagnostic: None,
    }
}

#[test]
fn plugin_manager_refresh_preserves_filtered_identity_without_idle_or_hidden_redraw() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut entries = vec![
        entry(0, "alpha", Phase::Running, Some(1)),
        entry(1, "alpine", Phase::Failed, Some(2)),
        entry(2, "disabled", Phase::Disabled, None),
    ];
    assert!(app.update_plugin_manager(entries.clone()));
    assert!(!app.plugins.presentation_dirty);
    app.execute_command("plugins").unwrap();
    let list = app.list.as_mut().unwrap();
    assert_eq!(list.purpose, crate::picker::ListPurpose::Manager);
    list.filter = "al".into();
    list.selected = 1;
    list.show_preview = false;
    entries[1].phase = Phase::Running;
    entries[1].owner = Some(3);
    assert!(app.update_plugin_manager(entries.clone()));
    assert!(matches!(
        app.selected_list_action(),
        Some(ListAction::PluginEntry(1))
    ));
    assert_eq!(app.list.as_ref().unwrap().filter, "al");
    assert!(!app.list.as_ref().unwrap().show_preview);
    assert!(app.plugins.presentation_dirty);
    app.plugins.presentation_dirty = false;
    assert!(!app.update_plugin_manager(entries.clone()));
    assert!(!app.plugins.presentation_dirty);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    entries[0].jobs = 0;
    assert!(app.update_plugin_manager(entries));
    assert!(app.list.is_none());
    assert!(!app.plugins.presentation_dirty);
}

#[test]
fn plugin_manager_action_menu_rejects_replacement_and_back_retains_filter() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.update_plugin_manager(vec![entry(7, "remote", Phase::Running, Some(10))]);
    app.execute_command("plugins").unwrap();
    app.list.as_mut().unwrap().filter = "rem".into();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.plugin_manager_actions_open());
    assert!(matches!(
        app.selected_list_action(),
        Some(ListAction::PluginLifecycle(Intent {
            expected_owner: Some(10),
            action: Action::Stop,
            ..
        }))
    ));
    app.update_plugin_manager(vec![entry(7, "remote", Phase::Running, Some(11))]);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.take_plugin_manager_intents().is_empty());
    assert!(app.plugin_manager_actions_open());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(app.list.as_ref().unwrap().filter, "rem");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.take_plugin_manager_intents(),
        vec![Intent {
            config_index: 7,
            expected_owner: Some(11),
            action: Action::Stop
        }]
    );
    assert_eq!(app.list.as_ref().unwrap().title, "Plugins");
}

#[test]
fn plugin_manager_disabled_invalid_duplicate_and_overflow_rows_remain_inspectable() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut invalid = entry(usize::MAX, "bad\ncredential", Phase::Failed, None);
    invalid.valid = false;
    invalid.diagnostic = Some("Invalid configuration\n".repeat(100));
    app.update_plugin_manager(vec![
        entry(0, "disabled", Phase::Disabled, None),
        entry(1, "dup", Phase::Stopped, None),
        entry(2, "dup", Phase::Stopped, None),
        invalid,
    ]);
    assert!(
        app.plugin_manager_action_by_id("disabled", Action::Restart)
            .is_err()
    );
    assert!(
        app.plugin_manager_action_by_id("dup", Action::Stop)
            .is_err()
    );
    app.execute_command("plugins").unwrap();
    assert_eq!(app.list.as_ref().unwrap().items.len(), 4);
    assert_eq!(
        app.list.as_ref().unwrap().items[3].label,
        "Invalid plugin configuration"
    );
    let preview = app.list.as_ref().unwrap().items[3].preview().unwrap();
    assert!(!preview.contains("credential"));
    assert!(preview.contains("Grants: views, jobs"));
    assert!(preview.contains("Jobs: 1\nActivities: 2\nHelpers: 3\nCleanup: 0"));
    assert!(preview.len() < 600);
    app.open_plugin_manager_actions(0);
    assert!(!app.list_actions.iter().any(|action| matches!(
        action,
        ListAction::PluginLifecycle(Intent {
            action: Action::Restart,
            ..
        })
    )));
    app.return_to_plugin_manager();
    app.open_plugin_manager_actions(usize::MAX);
    assert!(matches!(
        app.list_actions.as_slice(),
        [ListAction::PluginManagerBack]
    ));
    assert!(app.take_plugin_manager_intents().is_empty());
}

#[test]
fn plugin_manager_commands_complete_only_unambiguous_configured_ids() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.update_plugin_manager(vec![
        entry(0, "remote", Phase::Failed, Some(4)),
        entry(1, "repl", Phase::Stopped, None),
        entry(2, "dup", Phase::Stopped, None),
        entry(3, "dup", Phase::Stopped, None),
    ]);
    app.mode = Mode::Command;
    app.prompt_kind = PromptKind::Command;
    app.command = "plugin-restart rem".into();
    app.command_cursor = app.command.chars().count();
    assert_eq!(
        app.matching_plugin_hints()
            .unwrap()
            .iter()
            .map(|entry| entry.configured_id.as_str())
            .collect::<Vec<_>>(),
        vec!["remote"]
    );
    let overlays = app.overlay_snapshots();
    let candidates = overlays
        .iter()
        .find(|overlay| overlay.title == "Choose plugin for :plugin-restart")
        .unwrap();
    assert_eq!(candidates.rows.len(), 1);
    assert_eq!(candidates.rows[0].label, "remote");
    assert_eq!(candidates.rows[0].detail, "Failed");
    app.complete_selected_command();
    assert_eq!(app.command, "plugin-restart remote");
    app.execute_command("plugin-restart remote").unwrap();
    app.execute_command("plugin-stop remote").unwrap();
    assert_eq!(
        app.take_plugin_manager_intents(),
        vec![Intent {
            config_index: 0,
            expected_owner: Some(4),
            action: Action::Stop
        }]
    );
    for command in [
        "plugin-stop",
        "plugin-restart",
        "plugin-stop ../remote",
        "plugin-stop remote extra",
    ] {
        assert!(parse_colon_command(command).is_err(), "{command}");
    }
    app.command = "plugin-stop d".into();
    app.command_cursor = app.command.chars().count();
    assert!(app.matching_plugin_hints().unwrap().is_empty());
}

#[test]
fn plugin_manager_cleanup_and_restart_protect_quit_but_running_plugins_do_not() {
    for phase in [Phase::Stopping, Phase::RestartPending, Phase::Failed] {
        let mut app = App::new(Config::default(), None).unwrap();
        let mut pending = entry(0, "remote", phase, Some(1));
        pending.jobs = 0;
        if phase == Phase::Failed {
            pending.cleanup = 1;
        }
        app.update_plugin_manager(vec![pending]);
        app.execute_command("q!").unwrap();
        assert!(!app.should_quit);
        assert_eq!(app.plugin_active_job_count(), 1);
        app.update_plugin_manager(vec![entry(0, "remote", Phase::Running, Some(2))]);
        assert_eq!(app.plugin_active_job_count(), 0);
        app.execute_command("q!").unwrap();
        assert!(app.should_quit);
    }
}

#[test]
fn plugin_manager_queued_actions_protect_before_host_sync_and_allow_persistent_detach() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.update_plugin_manager(vec![entry(0, "remote", Phase::Running, Some(1))]);
    app.execute_command("plugin-restart remote").unwrap();
    assert_eq!(app.plugin_active_job_count(), 1);
    app.execute_command("q!").unwrap();
    assert!(!app.should_quit);
    app.execute_command("plugin-stop remote").unwrap();
    assert_eq!(app.plugins.manager_intents.len(), 1);
    assert_eq!(app.plugins.manager_intents[0].action, Action::Stop);
    app.update_plugin_manager(vec![entry(0, "remote", Phase::Stopping, Some(1))]);
    assert_eq!(
        app.plugin_active_job_count(),
        1,
        "pending intent and published cleanup count once"
    );
    app.persistent_session = true;
    app.execute_command("detach").unwrap();
    assert!(app.should_quit);
    assert!(matches!(
        app.persistent_exit_request,
        Some(PersistentExitRequest::Detach)
    ));
    assert_eq!(app.plugins.manager_intents.len(), 1);
}
