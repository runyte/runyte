// SPDX-License-Identifier: MPL-2.0

//! `:config-reload` against an edited configuration file.
//!
//! These drive the editor directly rather than a host, because every effect a
//! reload has on editor state — settings, theme, bindings, and the language
//! server definitions the editor forwards — belongs to `App`. Configured
//! plugin lifecycle is the host's half and is covered beside it.

use super::*;
use crate::config::WorkspaceMode;
use crate::keymap::Key;
use crate::lsp::LspCommand;

/// A unique temporary configuration path per test.
fn config_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    temporary_directory().join(format!(
        "runyte-config-reload-{}-{nanos}-{name}",
        std::process::id()
    ))
}

/// An editor started from `source`, with that file as its loaded config.
fn editor(name: &str, source: &str) -> (App, PathBuf) {
    let path = config_path(name);
    fs::write(&path, source).unwrap();
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);
    (app, path)
}

#[test]
fn an_edited_setting_takes_effect_without_restarting() {
    let (mut app, path) = editor("tab-width.yaml", "editor:\n  tab_width: 4\n");
    assert_eq!(app.config.editor.tab_width, 4);

    fs::write(&path, "editor:\n  tab_width: 8\n  soft_wrap: true\n").unwrap();
    app.execute_command("config-reload").unwrap();

    assert_eq!(app.config.editor.tab_width, 8);
    assert!(app.config.editor.soft_wrap);
    assert_eq!(app.persisted_config.editor.tab_width, 8);
    assert!(!app.status_error, "{}", app.status);
    assert!(app.status.contains("reloaded config"), "{}", app.status);
    fs::remove_file(path).unwrap();
}

#[test]
fn an_invalid_replacement_leaves_the_running_configuration_untouched() {
    let (mut app, path) = editor("invalid.yaml", "editor:\n  tab_width: 4\n");

    fs::write(&path, "editor:\n  tab_width: 99\n").unwrap();
    app.execute_command("config-reload").unwrap();

    assert_eq!(app.config.editor.tab_width, 4);
    assert_eq!(app.persisted_config.editor.tab_width, 4);
    assert!(app.status_error);
    assert!(
        app.status.contains("tab_width must be between 1 and 16"),
        "{}",
        app.status
    );
    assert!(
        app.status.contains("running configuration is unchanged"),
        "{}",
        app.status
    );

    fs::write(&path, "editor:\n  tab_width: [not, a, number]\n").unwrap();
    app.execute_command("config-reload").unwrap();
    assert_eq!(app.config.editor.tab_width, 4);
    assert!(app.status_error);
    fs::remove_file(path).unwrap();
}

#[test]
fn a_theme_that_cannot_be_resolved_keeps_the_one_on_screen() {
    let (mut app, path) = editor("theme.yaml", "theme: gruvbox\neditor:\n  tab_width: 4\n");
    assert_eq!(app.theme_name, "gruvbox");

    fs::write(&path, "theme: no-such-theme\neditor:\n  tab_width: 8\n").unwrap();
    app.execute_command("config-reload").unwrap();

    // The unrelated setting still lands; only the theme is held back, and the
    // name that could not be built remains what the file says is saved.
    assert_eq!(app.theme_name, "gruvbox");
    assert_eq!(app.config.editor.tab_width, 8);
    assert_eq!(app.persisted_config.theme.as_deref(), Some("no-such-theme"));
    assert!(
        app.status.contains("theme unavailable; keeping gruvbox"),
        "{}",
        app.status
    );
    assert!(
        app.notifications()
            .entries()
            .iter()
            .any(|entry| entry.title == "Theme unavailable")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn a_resolvable_theme_is_adopted_by_the_editor_and_its_terminals() {
    let (mut app, path) = editor("theme-ok.yaml", "theme: gruvbox\n");
    assert_eq!(app.theme_name, "gruvbox");

    fs::write(&path, "theme: matrix\n").unwrap();
    app.execute_command("config-reload").unwrap();

    assert_eq!(app.theme_name, "matrix");
    assert_eq!(
        app.theme.background,
        app.config.resolve_theme("matrix").unwrap().background
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn a_setting_read_at_startup_stays_effective_and_is_reported_as_saved() {
    let (mut app, path) = editor("startup-bound.yaml", "workspace:\n  mode: standalone\n");
    assert_eq!(app.config.workspace.mode, WorkspaceMode::Standalone);

    fs::write(
        &path,
        "workspace:\n  mode: persistent\neditor:\n  mouse: false\n",
    )
    .unwrap();
    app.execute_command("config-reload").unwrap();

    // Effective values are what this process is actually doing.
    assert_eq!(app.config.workspace.mode, WorkspaceMode::Standalone);
    assert!(app.config.editor.mouse);
    // Saved values are what the file now says, so the settings page offers them.
    assert_eq!(
        app.persisted_config.workspace.mode,
        WorkspaceMode::Persistent
    );
    assert!(!app.persisted_config.editor.mouse);
    assert!(!app.status_error, "{}", app.status);
    assert!(
        app.status
            .contains("restart required for editor.mouse, workspace.mode"),
        "{}",
        app.status
    );
    assert!(
        app.notifications()
            .entries()
            .iter()
            .any(|entry| entry.title == "Restart required"),
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn a_changed_keys_section_reaches_dispatch_help_and_hints_together() {
    let (mut app, path) = editor("keys.yaml", "editor:\n  tab_width: 4\n");
    assert_eq!(app.keymap().leader(), Key::char(' '));

    fs::write(&path, "keys:\n  leader: Ctrl-x\n").unwrap();
    app.execute_command("config-reload").unwrap();

    assert_eq!(app.keymap().leader(), Key::ctrl('x'));
    // Help and key hints read the same keymap dispatch does, so an actionable
    // message spells the reloaded binding rather than the built-in one.
    assert!(
        app.key_text(crate::key_spelling::actionable::HELP)
            .contains("Ctrl-x"),
        "{}",
        app.key_text(crate::key_spelling::actionable::HELP)
    );

    // Removing the section returns dispatch to the built-in registry.
    fs::write(&path, "editor:\n  tab_width: 4\n").unwrap();
    app.execute_command("config-reload").unwrap();
    assert_eq!(app.keymap().leader(), Key::char(' '));
    fs::remove_file(path).unwrap();
}

#[test]
fn rejected_key_entries_are_counted_without_failing_the_reload() {
    let (mut app, path) = editor("keys-bad.yaml", "editor:\n  tab_width: 4\n");

    fs::write(&path, "keys:\n  rebind:\n    Space e: Space\n").unwrap();
    app.execute_command("config-reload").unwrap();

    assert!(!app.status_error, "{}", app.status);
    assert!(
        app.status.contains("key binding entries rejected"),
        "{}",
        app.status
    );
    fs::remove_file(path).unwrap();
}

/// Every reload hands the manager the definitions the file now holds.
///
/// It is sent unconditionally rather than only when the editor thinks they
/// changed: the manager is the side that compares them and restarts only what
/// differs (`tests/lsp_client.rs` covers that), and an unconditional send means
/// running the command again is all it takes to recover from a dropped one.
#[test]
fn every_reload_hands_the_language_server_manager_the_saved_definitions() {
    let (mut app, path) = editor(
        "lsp.yaml",
        "lsp:\n  markdown:\n    command: marksman\n    args: [\"server\"]\n",
    );
    let (handle, mut queue) = crate::lsp::command_channel();
    app.attach_lsp(handle);

    fs::write(
        &path,
        "editor:\n  tab_width: 8\nlsp:\n  markdown:\n    command: other-server\n",
    )
    .unwrap();
    app.execute_command("config-reload").unwrap();

    let command = queue.try_recv().expect("reconfiguration is forwarded");
    let LspCommand::Reconfigure(config) = command else {
        panic!("expected a reconfiguration");
    };
    assert_eq!(
        config.servers["markdown"].command,
        PathBuf::from("other-server")
    );
    assert!(config.servers["markdown"].args.is_empty());
    // The built-in servers a load merges in travel with it.
    assert!(config.servers.contains_key("rust"));
    assert!(!app.status_error, "{}", app.status);
    fs::remove_file(path).unwrap();
}

/// A manager that cannot take the message says so rather than leaving the
/// editor and the servers quietly describing different configurations.
#[test]
fn a_refused_reconfiguration_is_reported_instead_of_being_dropped() {
    let (mut app, path) = editor("lsp-full.yaml", "editor:\n  tab_width: 4\n");
    let (handle, queue) = crate::lsp::command_channel();
    app.attach_lsp(handle);
    drop(queue);

    fs::write(&path, "editor:\n  tab_width: 8\n").unwrap();
    app.execute_command("config-reload").unwrap();

    assert_eq!(app.config.editor.tab_width, 8, "the reload still lands");
    assert!(
        app.status.contains("language servers not reconfigured"),
        "{}",
        app.status
    );
    assert!(
        app.notifications()
            .entries()
            .iter()
            .any(|entry| entry.title == "Language servers not reconfigured")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn a_deleted_configuration_file_is_refused_rather_than_reset_to_defaults() {
    let (mut app, path) = editor("deleted.yaml", "editor:\n  tab_width: 4\n");

    fs::remove_file(&path).unwrap();
    app.execute_command("config-reload").unwrap();

    assert_eq!(app.config.editor.tab_width, 4);
    assert!(app.status_error);
    assert!(app.status.contains("no longer exists"), "{}", app.status);
}

#[test]
fn a_configuration_file_created_after_startup_reloads_from_it() {
    let path = config_path("created.yaml");
    let (config, _) = Config::load(Some(&path)).unwrap();
    let mut app = App::new(config, None).unwrap();
    app.note_loaded_config(&path);

    app.execute_command("config-reload").unwrap();
    assert!(!app.status_error, "{}", app.status);
    assert!(
        app.status.contains("no configuration file at"),
        "{}",
        app.status
    );

    fs::write(&path, "editor:\n  tab_width: 7\n").unwrap();
    app.execute_command("config-reload").unwrap();
    assert_eq!(app.config.editor.tab_width, 7);
    fs::remove_file(path).unwrap();
}

#[test]
fn reloading_without_a_loaded_configuration_path_reports_why() {
    let mut app = App::new(Config::default(), None).unwrap();

    app.execute_command("config-reload").unwrap();

    assert!(app.status_error);
    assert!(
        app.status.contains("no configuration file is loaded"),
        "{}",
        app.status
    );
}

#[test]
fn a_reloaded_notification_limit_reaches_the_retained_history() {
    let (mut app, path) = editor(
        "notifications.yaml",
        "notifications:\n  history_limit: 50\n",
    );

    fs::write(&path, "notifications:\n  history_limit: 10\n").unwrap();
    app.execute_command("config-reload").unwrap();

    assert_eq!(app.config.notifications.history_limit, 10);
    for index in 0..14 {
        app.push_notification(crate::notification::NotificationDraft::new(
            crate::notification::NotificationSeverity::Info,
            "Test",
            format!("Entry {index}"),
            "body",
        ));
    }
    assert_eq!(app.notifications().entries().len(), 10);
    fs::remove_file(path).unwrap();
}

#[test]
fn every_startup_bound_setting_is_kept_and_named_on_its_own() {
    /// The key, a file asking for a different value, and what proves the
    /// running value survived the reload.
    struct Kept {
        key: &'static str,
        source: &'static str,
        effective: fn(&App) -> bool,
    }
    let kept = [
        Kept {
            key: "lsp.enable",
            source: "lsp:\n  enable: false\n",
            effective: |app| app.config.lsp.enable,
        },
        Kept {
            key: "workspace.state",
            source: "workspace:\n  state: .elsewhere\n",
            effective: |app| app.config.workspace.state.as_os_str() == ".runyte",
        },
        Kept {
            key: "workspace.state_anchor",
            source: "workspace:\n  state_anchor: profile\n",
            effective: |app| app.config.workspace.state_anchor.is_none(),
        },
    ];
    for Kept {
        key,
        source,
        effective,
    } in kept
    {
        let (mut app, path) = editor("startup-bound-one.yaml", "editor:\n  tab_width: 4\n");
        fs::write(&path, source).unwrap();
        app.execute_command("config-reload").unwrap();

        assert!(effective(&app), "{key} was adopted instead of held back");
        assert!(!app.status_error, "{}", app.status);
        assert!(
            app.status.contains(&format!("restart required for {key}")),
            "{key}: {}",
            app.status
        );
        // One key reads as one key, not as a list of one.
        let restart = app
            .notifications()
            .entries()
            .iter()
            .find(|entry| entry.title == "Restart required")
            .expect("the retained report names it too");
        assert!(
            restart.body.contains(&format!("{key} is read at startup")),
            "{key}: {}",
            restart.body
        );
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn an_explorer_setting_reaches_open_explorers_and_nothing_else_disturbs_them() {
    let directory = config_path("explorer-root");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("visible.txt"), "").unwrap();
    fs::write(directory.join(".hidden"), "").unwrap();

    let (mut app, path) = editor("explorer.yaml", "editor:\n  tab_width: 4\n");
    app.project_root.clone_from(&directory);
    app.working_directory.clone_from(&directory);
    app.execute_command("explorer").unwrap();
    assert!(app.active_buffer().is_directory());
    let listing = |app: &App| app.active_buffer().to_string();
    assert!(!listing(&app).contains(".hidden"));
    let before = listing(&app);

    // An edit that says nothing about explorers leaves the listing untouched.
    fs::write(&path, "editor:\n  tab_width: 8\n").unwrap();
    app.execute_command("config-reload").unwrap();
    assert_eq!(listing(&app), before);

    fs::write(
        &path,
        "editor:\n  tab_width: 8\n  show_hidden_files: true\n",
    )
    .unwrap();
    app.execute_command("config-reload").unwrap();
    assert!(listing(&app).contains(".hidden"), "{}", listing(&app));

    fs::remove_file(path).unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn an_unchanged_plugins_section_asks_the_host_for_nothing() {
    let entry = "plugins:\n  - id: demo\n    enabled: false\n    executable: /opt/demo\n";
    let (mut app, path) = editor("plugins.yaml", entry);
    assert!(!app.plugins.configuration_reload);

    fs::write(&path, format!("editor:\n  tab_width: 8\n{entry}")).unwrap();
    app.execute_command("config-reload").unwrap();
    assert!(
        !app.plugins.configuration_reload,
        "an unrelated edit must not put every configured plugin through reconciliation"
    );

    fs::write(
        &path,
        "plugins:\n  - id: demo\n    enabled: true\n    executable: /opt/demo\n",
    )
    .unwrap();
    app.execute_command("config-reload").unwrap();
    assert!(app.plugins.configuration_reload);
    fs::remove_file(path).unwrap();
}
