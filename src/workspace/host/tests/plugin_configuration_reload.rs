// SPDX-License-Identifier: MPL-2.0

//! Configured plugin lifecycle after `:config-reload` replaced `plugins`.
//!
//! The editor half of a reload is covered beside `App`; these cover the half
//! only the host can answer, because only the host owns the processes and the
//! work in flight that restarting one would interrupt.

use super::*;
use crate::plugin::{
    application as api,
    manager::{Action, Intent, Phase},
    settings::Settings,
};
use std::time::Duration;

use super::manager::{configured, until};

/// What `:config-reload` leaves for the host to act on.
fn reload(host: &mut WorkspaceHost, plugins: Vec<PluginConfig>) {
    host.app.config.plugins = plugins;
    host.app.plugins.configuration_reload = true;
    host.sync_plugin_manager();
}

fn entry(host: &WorkspaceHost, index: usize) -> &crate::plugin::manager::Entry {
    &host.app.plugins.manager_entries[index]
}

/// Runs a lifecycle colon command exactly as a person would, and requires it
/// to be accepted rather than refused.
fn lifecycle(host: &mut WorkspaceHost, command: &str) {
    let invocation = crate::command::parse_colon_command(command).unwrap();
    let outcome = host.app.execute(invocation).unwrap();
    assert!(
        !matches!(
            outcome,
            CommandOutcome::UserError(_) | CommandOutcome::Unavailable(_)
        ),
        "{command} was refused: {outcome:?}"
    );
    host.sync_plugin_manager();
}

/// How many times a deferral has been reported to the notification centre.
fn pending_reports(host: &WorkspaceHost) -> usize {
    host.app
        .notifications()
        .entries()
        .iter()
        .filter(|retained| retained.title == "Configuration change pending")
        .map(|retained| retained.occurrences)
        .sum()
}

/// Gives a running plugin one job, so a reload has work it must not interrupt.
fn hold_job(host: &mut WorkspaceHost, owner: usize) {
    host.app
        .plugins
        .instances
        .get_mut(&owner)
        .unwrap()
        .application
        .jobs
        .insert(
            "j:1".into(),
            api::Job {
                message: None,
                job: "j:1".into(),
                title: "Indexing".into(),
                state: api::JobState::Running,
                progress: 0,
            },
        );
}

fn starts(root: &TestRuntimeRoot) -> usize {
    std::fs::read_to_string(root.join("manager.starts"))
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

#[tokio::test]
async fn an_unchanged_entry_keeps_the_generation_it_is_already_running() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();

    // An edit elsewhere in the file reloads the whole document, so an entry
    // that did not change must cost its plugin nothing at all.
    reload(&mut host, vec![original]);

    assert_eq!(entry(&host, 0).phase, Phase::Running);
    assert_eq!(entry(&host, 0).owner, Some(owner));
    assert_eq!(starts(&root), 1);
}

#[tokio::test]
async fn a_changed_entry_restarts_the_plugin_from_the_saved_configuration() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();

    let mut edited = original;
    edited.settings = Settings::from_value(serde_json::json!({"endpoint": "corrected"})).unwrap();
    reload(&mut host, vec![edited.clone()]);

    assert_eq!(entry(&host, 0).phase, Phase::RestartPending);
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let restarted = entry(&host, 0).owner.unwrap();
    assert!(restarted > owner);
    assert_eq!(starts(&root), 2);
    assert_eq!(
        host.app.plugins.instances[&restarted].config.settings, edited.settings,
        "the new generation runs the configuration that was saved"
    );
}

#[tokio::test]
async fn work_in_flight_keeps_the_running_configuration_until_an_explicit_restart() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();
    hold_job(&mut host, owner);

    let mut edited = original;
    edited.settings = Settings::from_value(serde_json::json!({"endpoint": "corrected"})).unwrap();
    reload(&mut host, vec![edited.clone()]);

    // The running job is untouched and so is the process behind it.
    assert_eq!(entry(&host, 0).phase, Phase::Running);
    assert_eq!(entry(&host, 0).owner, Some(owner));
    assert_eq!(starts(&root), 1);
    assert!(
        entry(&host, 0)
            .diagnostic
            .as_deref()
            .unwrap_or_default()
            .contains(":plugin-restart"),
        "{:?}",
        entry(&host, 0).diagnostic
    );
    assert!(
        host.app
            .notifications()
            .entries()
            .iter()
            .any(|retained| retained.title == "Configuration change pending"),
    );

    // The record still adopted the saved entry, which is what makes the
    // explicit restart start the corrected configuration rather than the one
    // the reload found running.
    host.app
        .plugins
        .instances
        .get_mut(&owner)
        .unwrap()
        .application
        .jobs
        .clear();
    host.app.plugins.manager_intents.push_back(Intent {
        config_index: 0,
        expected_owner: Some(owner),
        action: Action::Restart,
    });
    host.sync_plugin_manager();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let restarted = entry(&host, 0).owner.unwrap();
    assert_eq!(
        host.app.plugins.instances[&restarted].config.settings,
        edited.settings
    );
}

#[tokio::test]
async fn an_unsaved_provider_document_defers_the_change_that_would_strand_it() {
    let (root, mut host) = host();
    let mut original = configured(&root, "managed");
    original.capabilities.push("providers".into());
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();
    host.app
        .buffers
        .push(crate::buffer::Buffer::provider_document(
            crate::buffer::ProviderDocument {
                identity: crate::buffer::ProviderIdentity {
                    configured_plugin: "managed".into(),
                    provider: "remote".into(),
                    key: "k".into(),
                },
                label: "remote://k".into(),
                syntax_hint: None,
                version: "1".into(),
                generation: host.app.plugins.instances[&owner]
                    .application
                    .generation
                    .clone(),
                available: true,
                baseline_epoch: 0,
                uncertain: None,
            },
            "accepted".into(),
        ));
    let document = host.app.buffers.len() - 1;
    host.app.buffers[document].apply(&crate::text::Transaction::insert(0, "edited "));
    assert!(host.app.buffers[document].holds_unsaved_work());

    let mut edited = original;
    edited.settings = Settings::from_value(serde_json::json!({"endpoint": "corrected"})).unwrap();
    reload(&mut host, vec![edited]);

    assert_eq!(entry(&host, 0).phase, Phase::Running);
    assert_eq!(entry(&host, 0).owner, Some(owner));
    assert_eq!(starts(&root), 1);
}

#[tokio::test]
async fn a_removed_entry_is_stopped_and_leaves_the_manager_once_it_settles() {
    let (root, mut host) = host();
    host.app.config.plugins.push(configured(&root, "managed"));
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;

    reload(&mut host, Vec::new());

    // It stays visible while its cleanup is outstanding, because hiding it
    // would claim a cleanup finished that has not.
    assert_eq!(host.app.plugins.manager_entries.len(), 1);
    assert_eq!(entry(&host, 0).config_index, 0);
    assert!(!entry(&host, 0).enabled);
    until(&mut host, &mut events, |host| {
        host.plugin_workers.is_empty() && host.app.plugins.instances.is_empty()
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !host.plugin_manager.is_empty() {
            host.sync_plugin_manager();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        host.app.plugins.manager_entries.is_empty(),
        "the frontend's list is cleared too, not left showing a gone plugin"
    );
    assert_eq!(starts(&root), 1);
}

#[tokio::test]
async fn a_reload_starts_a_plugin_that_was_disabled_when_the_editor_started() {
    let (root, mut host) = host();
    let mut disabled = configured(&root, "managed");
    disabled.enabled = false;
    host.app.config.plugins.push(disabled.clone());
    let mut events = host.start_plugins().unwrap();
    assert_eq!(entry(&host, 0).phase, Phase::Disabled);
    assert_eq!(starts(&root), 0);

    let mut enabled = disabled;
    enabled.enabled = true;
    reload(&mut host, vec![enabled]);

    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    assert_eq!(starts(&root), 1);
}

#[tokio::test]
async fn a_disabled_entry_is_stopped_and_an_invalid_one_reports_why() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;

    let mut broken = original;
    broken.runyte = "not-a-range".into();
    reload(&mut host, vec![broken]);

    assert!(!entry(&host, 0).valid);
    assert!(
        entry(&host, 0)
            .diagnostic
            .as_deref()
            .unwrap_or_default()
            .contains("Runyte range"),
        "{:?}",
        entry(&host, 0).diagnostic
    );
    // Settles as `Failed`, exactly as an entry found unusable at startup does,
    // rather than as the clean stop the lifecycle call underneath it performs.
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Failed
    })
    .await;
    assert!(host.app.plugins.instances.is_empty());
    assert_eq!(starts(&root), 1);
}

#[tokio::test]
async fn entries_are_matched_by_identity_rather_than_position() {
    let (root, mut host) = host();
    let first = configured(&root, "first");
    let second = configured(&root, "second");
    host.app.config.plugins = vec![first.clone(), second.clone()];
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running && entry(host, 1).phase == Phase::Running
    })
    .await;
    let owners = [entry(&host, 0).owner, entry(&host, 1).owner];

    reload(&mut host, vec![second, first]);

    assert_eq!(entry(&host, 0).configured_id, "second");
    assert_eq!(entry(&host, 1).configured_id, "first");
    assert_eq!(entry(&host, 0).config_index, 0);
    assert_eq!(entry(&host, 1).config_index, 1);
    assert_eq!(entry(&host, 0).owner, owners[1]);
    assert_eq!(entry(&host, 1).owner, owners[0]);
    assert_eq!(starts(&root), 2, "reordering restarts nothing");
}

#[tokio::test]
async fn an_entry_the_file_did_not_have_before_starts_from_the_reload() {
    let (root, mut host) = host();
    let first = configured(&root, "first");
    host.app.config.plugins.push(first.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;

    let second = configured(&root, "second");
    reload(&mut host, vec![first, second]);

    until(&mut host, &mut events, |host| {
        host.app.plugins.manager_entries.len() == 2 && entry(host, 1).phase == Phase::Running
    })
    .await;
    assert_eq!(entry(&host, 1).configured_id, "second");
    assert_eq!(starts(&root), 2, "the entry that already ran is untouched");
}

/// A plugin holding work stays reachable by both lifecycle commands.
///
/// Deferring a change must not take the running plugin's own controls away:
/// the published entry keeps describing the generation that is running, so
/// `:plugin-stop` and `:plugin-restart` are decided from what that generation
/// is, not from an entry it was never started with.
#[tokio::test]
async fn a_deferred_entry_that_turned_invalid_can_still_be_stopped() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();
    hold_job(&mut host, owner);

    let mut broken = original;
    broken.runyte = "not-a-range".into();
    reload(&mut host, vec![broken]);

    assert_eq!(entry(&host, 0).phase, Phase::Running);
    assert!(
        entry(&host, 0).valid,
        "the entry answers for the generation that is running, which was admitted"
    );
    lifecycle(&mut host, "plugin-stop managed");
    until(&mut host, &mut events, |host| {
        host.app.plugins.instances.is_empty()
    })
    .await;
    assert_eq!(starts(&root), 1);
}

/// An entry removed while work was in flight, then restored unchanged.
///
/// The record survives the removal to keep its cleanup visible, so matching it
/// again has to re-apply even though its text never changed: the entry it is
/// still publishing says the configuration disowned it.
#[tokio::test]
async fn an_entry_removed_during_work_and_restored_becomes_enabled_again() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();
    hold_job(&mut host, owner);

    reload(&mut host, Vec::new());
    assert!(!entry(&host, 0).enabled);
    assert_eq!(entry(&host, 0).owner, Some(owner));
    assert_eq!(starts(&root), 1);

    reload(&mut host, vec![original]);

    assert!(
        entry(&host, 0).enabled,
        "the restored entry is enabled again rather than left dimmed forever"
    );
    assert_eq!(entry(&host, 0).phase, Phase::Running);
    assert_eq!(entry(&host, 0).owner, Some(owner));
    assert_eq!(
        entry(&host, 0).diagnostic,
        None,
        "the removal it was carrying is taken back with it"
    );
    assert_eq!(
        starts(&root),
        1,
        "the process it was holding never restarted"
    );

    // With the work finished, the ordinary restart applies the saved entry.
    host.app
        .plugins
        .instances
        .get_mut(&owner)
        .unwrap()
        .application
        .jobs
        .clear();
    lifecycle(&mut host, "plugin-restart managed");
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running && entry(host, 0).owner != Some(owner)
    })
    .await;
    assert_eq!(starts(&root), 2);
}

#[tokio::test]
async fn a_configuration_with_too_many_entries_leaves_the_running_plugins_alone() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();

    let mut excessive = vec![original];
    for index in 0..128 {
        let mut extra = crate::plugin::PluginConfig {
            enabled: false,
            ..excessive[0].clone()
        };
        extra.id = format!("filler-{index}");
        excessive.push(extra);
    }
    reload(&mut host, excessive);

    assert_eq!(host.app.plugins.manager_entries.len(), 1);
    assert_eq!(entry(&host, 0).configured_id, "managed");
    assert_eq!(entry(&host, 0).owner, Some(owner));
    assert_eq!(entry(&host, 0).phase, Phase::Running);
    assert_eq!(starts(&root), 1);
}

/// A deferral is a wait, not a resting state.
///
/// The reload was asked for; the only thing standing in its way was work that
/// has now finished, so the saved entry takes effect without a second command.
#[tokio::test]
async fn a_deferred_change_lands_by_itself_once_the_work_finishes() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();
    hold_job(&mut host, owner);

    let mut edited = original;
    edited.settings = Settings::from_value(serde_json::json!({"endpoint": "corrected"})).unwrap();
    reload(&mut host, vec![edited.clone()]);
    assert_eq!(entry(&host, 0).owner, Some(owner));
    assert_eq!(starts(&root), 1);

    host.app
        .plugins
        .instances
        .get_mut(&owner)
        .unwrap()
        .application
        .jobs
        .clear();
    host.sync_plugin_manager();

    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running && entry(host, 0).owner != Some(owner)
    })
    .await;
    assert_eq!(starts(&root), 2);
    let restarted = entry(&host, 0).owner.unwrap();
    assert_eq!(
        host.app.plugins.instances[&restarted].config.settings,
        edited.settings
    );
    assert!(
        host.app
            .notifications()
            .entries()
            .iter()
            .any(|retained| retained.body.contains("finished its work")),
    );
}

/// Stopping a deferred plugin still leaves its entry telling the truth.
///
/// The parked entry is applied to the record the moment the generation it was
/// protecting is gone, so the row does not rest on a verdict the file
/// contradicts.
#[tokio::test]
async fn stopping_a_deferred_plugin_adopts_the_verdict_the_file_now_carries() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();
    hold_job(&mut host, owner);

    let mut broken = original;
    broken.runyte = "not-a-range".into();
    reload(&mut host, vec![broken]);
    assert!(entry(&host, 0).valid, "the running generation was admitted");

    lifecycle(&mut host, "plugin-stop managed");
    until(&mut host, &mut events, |host| {
        host.app.plugins.instances.is_empty() && entry(host, 0).phase != Phase::Stopping
    })
    .await;

    assert!(
        !entry(&host, 0).valid,
        "the settled row names the entry the file now holds"
    );
    assert_eq!(entry(&host, 0).phase, Phase::Failed);
    assert!(
        entry(&host, 0)
            .diagnostic
            .as_deref()
            .unwrap_or_default()
            .contains("Runyte range"),
        "{:?}",
        entry(&host, 0).diagnostic
    );
    assert_eq!(starts(&root), 1);
}

/// A parked change survives the manager being renumbered under it.
///
/// Removing two plugins at once retires both; the idle one is pruned as soon
/// as it settles, which moves the busy one down a position. The change waiting
/// on the busy one has to reach the plugin it was made for, at wherever that
/// record now is.
#[tokio::test]
async fn a_parked_change_follows_its_record_when_the_manager_is_renumbered() {
    let (root, mut host) = host();
    let idle = configured(&root, "idle");
    let busy = configured(&root, "busy");
    host.app.config.plugins = vec![idle, busy];
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running && entry(host, 1).phase == Phase::Running
    })
    .await;
    let busy_owner = entry(&host, 1).owner.unwrap();
    hold_job(&mut host, busy_owner);

    reload(&mut host, Vec::new());
    assert_eq!(host.app.plugins.manager_entries.len(), 2);

    // The idle one drains and is pruned, moving the busy record to position 0.
    until(&mut host, &mut events, |host| {
        host.app.plugins.manager_entries.len() == 1
    })
    .await;
    assert_eq!(entry(&host, 0).configured_id, "busy");
    assert_eq!(entry(&host, 0).config_index, 0);

    host.app
        .plugins
        .instances
        .get_mut(&busy_owner)
        .unwrap()
        .application
        .jobs
        .clear();
    host.sync_plugin_manager();

    until(&mut host, &mut events, |host| {
        host.app.plugins.manager_entries.is_empty()
    })
    .await;
    assert_eq!(starts(&root), 2, "neither plugin was ever restarted");
}

/// A removal taken back must not take an undelivered edit with it.
///
/// Editing, removing and restoring an entry inside one work window parks three
/// changes over each other. The restore cancels the removal, but the edit it
/// overwrote still never reached the process, so it has to keep waiting.
#[tokio::test]
async fn a_removal_taken_back_leaves_an_undelivered_edit_still_waiting() {
    let (root, mut host) = host();
    let original = configured(&root, "managed");
    host.app.config.plugins.push(original.clone());
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running
    })
    .await;
    let owner = entry(&host, 0).owner.unwrap();
    hold_job(&mut host, owner);

    let mut edited = original;
    edited.settings = Settings::from_value(serde_json::json!({"endpoint": "corrected"})).unwrap();
    reload(&mut host, vec![edited.clone()]);
    reload(&mut host, Vec::new());
    // Identical notification bodies coalesce, so the count of times the wait
    // has been reported is what separates a restore that reported it from one
    // that silently repaired the row.
    let reported = pending_reports(&host);
    reload(&mut host, vec![edited.clone()]);

    assert!(entry(&host, 0).enabled);
    assert_eq!(entry(&host, 0).owner, Some(owner));
    assert_eq!(starts(&root), 1);
    // The wait is reported exactly as any other deferral is, rather than the
    // restore leaving the row looking as though nothing is outstanding.
    assert!(
        entry(&host, 0)
            .diagnostic
            .as_deref()
            .unwrap_or_default()
            .contains("work is in flight"),
        "{:?}",
        entry(&host, 0).diagnostic
    );
    assert_eq!(pending_reports(&host), reported + 1);

    host.app
        .plugins
        .instances
        .get_mut(&owner)
        .unwrap()
        .application
        .jobs
        .clear();
    host.sync_plugin_manager();

    until(&mut host, &mut events, |host| {
        entry(host, 0).phase == Phase::Running && entry(host, 0).owner != Some(owner)
    })
    .await;
    let restarted = entry(&host, 0).owner.unwrap();
    assert_eq!(
        host.app.plugins.instances[&restarted].config.settings, edited.settings,
        "the edit the removal overwrote still reaches the process"
    );
    assert_eq!(starts(&root), 2);
}

/// Active work is not only what a plugin declares as a job or a lease.
///
/// Everything `stop_plugin` cancels or orphans counts, because a restart would
/// destroy it just as surely as it would a job — and a plugin is under no
/// obligation to open a lease before doing any of it.
#[tokio::test]
async fn an_outstanding_request_defers_a_change_without_any_declared_work() {
    for outstanding in [
        Outstanding::Command,
        Outstanding::LocalFilesystem,
        Outstanding::InputSurface,
    ] {
        let (root, mut host) = host();
        let original = configured(&root, "managed");
        host.app.config.plugins.push(original.clone());
        let mut events = host.start_plugins().unwrap();
        until(&mut host, &mut events, |host| {
            entry(host, 0).phase == Phase::Running
        })
        .await;
        let owner = entry(&host, 0).owner.unwrap();
        assert_eq!(entry(&host, 0).jobs, 0);
        assert_eq!(entry(&host, 0).activities, 0);
        outstanding.hold(&mut host, owner);

        let mut edited = original;
        edited.settings =
            Settings::from_value(serde_json::json!({"endpoint": "corrected"})).unwrap();
        reload(&mut host, vec![edited]);

        assert_eq!(
            entry(&host, 0).owner,
            Some(owner),
            "{outstanding:?} was interrupted"
        );
        assert_eq!(entry(&host, 0).phase, Phase::Running);
        assert_eq!(starts(&root), 1, "{outstanding:?} was interrupted");
    }
}

/// One thing a plugin can have in flight without declaring a job or a lease.
#[derive(Clone, Copy, Debug)]
enum Outstanding {
    /// A command invocation whose response has not arrived.
    Command,
    /// Accepted local filesystem work.
    LocalFilesystem,
    /// A prompt the person is part-way through answering.
    InputSurface,
}

impl Outstanding {
    fn hold(self, host: &mut WorkspaceHost, owner: usize) {
        let application = &mut host
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        match self {
            Self::Command => {
                application.requests.insert(
                    "p:1".into(),
                    api::CapturedContext {
                        foreground_allowed: true,
                        native_handoff_allowed: false,
                        action: None,
                        pane: 0,
                        buffer: 0,
                        terminal: None,
                        attachment: 0,
                        foreground: 0,
                    },
                );
            }
            Self::LocalFilesystem => {
                application.local_requests.insert(
                    "p:1".into(),
                    crate::plugin::filesystem::Pending {
                        staging_job: None,
                        staging_handle: None,
                        cancelled: None,
                        creating: false,
                        invocation: None,
                        offset: 0,
                        limit: 0,
                        charge: 0,
                        expected_revision: None,
                    },
                );
            }
            Self::InputSurface => {
                application.input_surfaces.insert("s:1".into());
            }
        }
    }
}

/// A reload must not take a running plugin's keys away.
///
/// The compiled keymaps carry every live plugin binding, so a freshly compiled
/// `keys` section replaces them unless they are laid back over it. Dispatch,
/// help and the key hints all read that one keymap.
#[test]
fn a_reload_keeps_the_keys_of_the_plugins_that_are_running() {
    let (root, mut host) = self::host();
    let path = root.join("config.yaml");
    std::fs::write(&path, "editor:\n  tab_width: 4\n").unwrap();
    host.app.note_loaded_config(&path);

    let mut bound = config("case");
    bound.bindings.insert("upper".into(), "F12".into());
    let _receiver = instance(&mut host, 0, bound);
    register(&mut host, 0).unwrap();
    let bound_to = |host: &WorkspaceHost| {
        matches!(
            host.app.keymap().lookup(
                crate::command::Mode::Normal,
                &crate::keymap::KeySequence::parse("F12").unwrap()
            ),
            crate::keymap::Lookup::Exact(_)
        )
    };
    assert!(bound_to(&host));

    std::fs::write(&path, "editor:\n  tab_width: 8\n").unwrap();
    reload_command(&mut host);

    assert_eq!(host.app.config.editor.tab_width, 8);
    assert!(
        bound_to(&host),
        "the running plugin's key survived a reload of an unrelated setting"
    );
}

/// A `keys` section that collides with a live plugin binding is refused alone.
///
/// Applying it would leave dispatch holding a keymap the running plugins were
/// never validated against, which every later rebuild of their bindings
/// assumes cannot happen.
#[test]
fn a_keys_section_colliding_with_a_live_plugin_binding_is_refused_on_its_own() {
    let (root, mut host) = self::host();
    let path = root.join("config.yaml");
    std::fs::write(&path, "editor:\n  tab_width: 4\n").unwrap();
    host.app.note_loaded_config(&path);

    let mut bound = config("case");
    bound.bindings.insert("upper".into(), "F12".into());
    let _receiver = instance(&mut host, 0, bound);
    register(&mut host, 0).unwrap();

    std::fs::write(
        &path,
        // The left side names an existing default; the right side is where it
        // moves to. Open-explorer moves onto the key the plugin holds.
        "editor:\n  tab_width: 8\nkeys:\n  rebind:\n    Space e: F12\n",
    )
    .unwrap();
    reload_command(&mut host);

    assert_eq!(
        host.app.config.editor.tab_width, 8,
        "the rest of the file still applies"
    );
    assert!(
        host.app.notifications().entries().iter().any(|retained| {
            retained.title == "Key bindings"
                && retained.body.contains("the keys section was not applied")
        }),
        "the refusal is reported rather than silently dropped"
    );
    assert!(
        matches!(
            host.app.keymap().lookup(
                crate::command::Mode::Normal,
                &crate::keymap::KeySequence::parse("F12").unwrap()
            ),
            crate::keymap::Lookup::Exact(binding)
                if matches!(binding.target, crate::keymap::BindingTarget::Plugin(_))
        ),
        "the plugin keeps the key the refused section tried to take"
    );
    // The keymap dispatch holds is still one the live plugin is valid against,
    // which is what stopping it later depends on.
    host.stop_plugin(0, "stopped by user");
}

fn reload_command(host: &mut WorkspaceHost) {
    let invocation = crate::command::parse_colon_command("config-reload").unwrap();
    host.app.execute(invocation).unwrap();
}
