// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::plugin::{
    application as api,
    manager::{Action, Intent, Phase},
    state,
};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

pub(super) fn configured(root: &TestRuntimeRoot, id: &str) -> PluginConfig {
    let program = root.join(format!("manager-{id}"));
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &program,
    )
    .unwrap();
    std::fs::write(root.join(format!("manager-{id}.behavior")),
        "printf '%s\\n' \"$$\" >> manager.starts\nread -r hello\nprintf '%s\\n' '{\"type\":\"register\",\"version\":\"runyte-experimental-1\",\"commands\":[{\"name\":\"upper\",\"description\":\"Uppercase selections\"}]}'\nwhile read -r line; do :; done\n").unwrap();
    let mut config = config(id);
    config.executable = program;
    config
}
pub(super) async fn until(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    ready: impl Fn(&WorkspaceHost) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready(host) {
            host.handle_plugin_event(events.recv().await.unwrap());
        }
    })
    .await
    .unwrap();
}
pub(super) fn action(host: &mut WorkspaceHost, index: usize, action: Action) {
    host.app.plugins.manager_intents.push_back(Intent {
        config_index: index,
        expected_owner: host.app.plugins.manager_entries[index].owner,
        action,
    });
    host.sync_plugin_manager();
}
fn phase(host: &WorkspaceHost) -> Phase {
    host.app.plugins.manager_entries[0].phase
}

#[tokio::test]
async fn manager_restart_waits_for_final_fifo_then_rejects_all_old_owner_events() {
    let (root, mut host) = host();
    host.app.config.plugins.push(configured(&root, "managed"));
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| phase(host) == Phase::Running).await;
    let old = host.app.plugins.manager_entries[0].owner.unwrap();
    let old_generation = host.app.plugins.instances[&old]
        .application
        .generation
        .clone();
    let old_command = host
        .app
        .plugins
        .commands
        .values()
        .find(|command| command.local == "upper")
        .unwrap()
        .id;
    action(&mut host, 0, Action::Restart);
    assert_eq!(phase(&host), Phase::RestartPending);
    assert_eq!(host.plugin_workers.len(), 1);
    assert!(host.app.plugins.instances.is_empty());
    assert!(host.protected_state().plugin_jobs > 0);
    until(&mut host, &mut events, |host| phase(host) == Phase::Running).await;
    let new = host.app.plugins.manager_entries[0].owner.unwrap();
    assert!(new > old);
    assert_ne!(
        host.app.plugins.instances[&new].application.generation,
        old_generation
    );
    assert!(!host.app.plugins.commands.contains_key(&old_command));
    assert_eq!(host.app.plugins.manager_entries[0].cleanup, 0);
    for result in [
        Err("old decoder failure containing credential-canary".into()),
        Ok(ClientMessage::Deadline {
            token: "old-deadline".into(),
        }),
        Ok(ClientMessage::Register {
            version: plugin::VERSION.into(),
            commands: vec![],
        }),
    ] {
        host.handle_plugin_event(Event {
            plugin: old,
            result,
        });
        assert!(host.app.plugins.instances.contains_key(&new));
        assert_eq!(phase(&host), Phase::Running);
    }
    assert!(
        !host.app.plugins.manager_entries[0]
            .diagnostic
            .as_deref()
            .unwrap_or("")
            .contains("credential-canary")
    );
    action(&mut host, 0, Action::Stop);
    until(&mut host, &mut events, |host| phase(host) == Phase::Stopped).await;
    assert_eq!(host.app.plugins.manager_entries[0].owner, Some(new));
    assert_eq!(host.protected_state().plugin_jobs, 0);
}

struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test]
async fn manager_restart_waits_for_state_orphan_and_stop_cancels_the_pending_restart() {
    state_restart(true).await;
}

#[tokio::test]
async fn manager_state_completion_advances_pending_restart_without_another_input() {
    state_restart(false).await;
}

async fn state_restart(cancel: bool) {
    let (root, mut host) = host();
    host.app.config.plugins.push(configured(&root, "managed"));
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| phase(host) == Phase::Running).await;
    let owner = host.app.plugins.manager_entries[0].owner.unwrap();
    host.app
        .plugins
        .instances
        .get_mut(&owner)
        .unwrap()
        .application
        .capabilities
        .insert("state".into());
    let gate = Arc::new(Gate {
        open: Mutex::new(false),
        changed: Condvar::new(),
    });
    let release = Release(gate.clone());
    let (started, ready) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started));
    host.plugin_state_hook = Some(Arc::new(move |phase| {
        if phase == state::Checkpoint::BeforeMutation {
            if let Some(started) = started.lock().unwrap().take() {
                let _ = started.send(());
            }
            let mut open = gate.open.lock().unwrap();
            while !*open {
                open = gate.changed.wait(open).unwrap();
            }
        }
        Ok(())
    }));
    host.application_state_request(
        owner,
        "p:1",
        api::Request::StateSet {
            expected_revision: "s:missing".into(),
            document: state::Document {
                version: 1,
                data: serde_json::value::RawValue::from_string("1".into()).unwrap(),
            },
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    action(&mut host, 0, Action::Restart);
    until(&mut host, &mut events, |host| {
        host.plugin_workers.is_empty()
    })
    .await;
    assert_eq!(phase(&host), Phase::RestartPending);
    assert_eq!(host.app.plugins.manager_entries[0].cleanup, 1);
    assert!(host.app.plugins.instances.is_empty());
    assert!(host.protected_state().plugin_jobs > 0);
    if cancel {
        action(&mut host, 0, Action::Stop);
        assert_eq!(phase(&host), Phase::Stopping);
    }
    release.0.release();
    let expected = if cancel {
        Phase::Stopped
    } else {
        Phase::Running
    };
    until(&mut host, &mut events, |host| phase(host) == expected).await;
    if cancel {
        assert!(host.app.plugins.instances.is_empty());
    } else {
        assert!(host.app.plugins.manager_entries[0].owner.unwrap() > owner);
        assert_eq!(host.app.plugins.instances.len(), 1);
        assert_eq!(host.app.plugins.manager_entries[0].cleanup, 0);
    }
    assert_eq!(
        std::fs::read_to_string(root.join("manager.starts"))
            .unwrap()
            .lines()
            .count(),
        if cancel { 1 } else { 2 }
    );
    assert!(
        !host
            .app
            .state_root
            .join("plugins/managed/state.json")
            .exists()
    );
    assert_eq!(host.app.plugins.orphaned_payload, 0);
}

#[tokio::test]
async fn manager_failed_generation_never_restarts_automatically_or_accepts_stale_none_action() {
    let (_root, mut host) = host();
    host.app.config.plugins.push(config("broken"));
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| phase(host) == Phase::Failed).await;
    let old = host.app.plugins.manager_entries[0].owner;
    assert!(old.is_some());
    assert!(host.plugin_workers.is_empty());
    for _ in 0..3 {
        host.sync_plugin_manager();
    }
    assert_eq!(host.app.plugins.manager_entries[0].owner, old);
    host.app.plugins.manager_intents.push_back(Intent {
        config_index: 0,
        expected_owner: None,
        action: Action::Restart,
    });
    host.sync_plugin_manager();
    assert!(host.plugin_workers.is_empty());
    assert!(host.app.status.contains("changed"));
    action(&mut host, 0, Action::Restart);
    until(&mut host, &mut events, |host| phase(host) == Phase::Failed).await;
    assert!(host.app.plugins.manager_entries[0].owner > old);
}

#[tokio::test]
async fn manager_config_admission_bounds_disabled_entries_without_hiding_enabled_tail() {
    let (root, mut host) = host();
    for index in 0..127 {
        let mut config = config(&format!("off-{index}"));
        config.enabled = false;
        host.app.config.plugins.push(config);
    }
    host.app.config.plugins.push(configured(&root, "tail"));
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| {
        host.app.plugins.manager_entries[127].phase == Phase::Running
    })
    .await;
    assert_eq!(host.app.plugins.manager_entries.len(), 128);
    assert!(
        host.app.plugins.manager_entries[..127]
            .iter()
            .all(|entry| entry.phase == Phase::Disabled)
    );
    assert_eq!(host.plugin_workers.len(), 1);
    let (_other, mut excessive) = self::host();
    for index in 0..129 {
        let mut config = config(&format!("off-{index}"));
        config.enabled = false;
        excessive.app.config.plugins.push(config);
    }
    assert!(excessive.start_plugins().is_none());
    assert!(excessive.plugin_events_sender.is_none());
    assert!(excessive.plugin_workers.is_empty());
    assert_eq!(excessive.app.plugins.manager_entries.len(), 1);
    assert!(
        excessive.app.plugins.manager_entries[0]
            .diagnostic
            .as_ref()
            .unwrap()
            .contains("128")
    );
    let (_other, mut duplicate) = self::host();
    duplicate.app.config.plugins = vec![config("same"), config("same")];
    let _events = duplicate.start_plugins();
    assert!(duplicate.plugin_workers.is_empty());
    assert!(
        duplicate
            .app
            .plugins
            .manager_entries
            .iter()
            .all(|entry| !entry.valid)
    );
}

#[tokio::test]
async fn manager_unverified_reap_keeps_producer_reservation_and_refuses_restart() {
    let (_root, mut host) = host();
    host.app.config.plugins.push(config("broken"));
    let _events = host.start_plugins().unwrap();
    let owner = host.app.plugins.manager_entries[0].owner.unwrap();
    host.handle_plugin_event(Event {
        plugin: owner,
        result: Ok(ClientMessage::WorkerStopped {
            failure: Some("Plugin process cleanup failed".into()),
            reaped: false,
        }),
    });
    assert_eq!(phase(&host), Phase::Failed);
    assert_eq!(host.app.plugins.manager_entries[0].cleanup, 1);
    assert_eq!(host.plugin_workers.len(), 1);
    action(&mut host, 0, Action::Restart);
    assert_eq!(phase(&host), Phase::Failed);
    assert_eq!(host.app.plugins.manager_entries[0].owner, Some(owner));
    assert!(host.protected_state().plugin_jobs > 0);
}

#[tokio::test]
async fn manager_recovery_snapshot_survives_restart_without_blocking_it() {
    let (root, mut host) = host();
    host.app.config.plugins.push(configured(&root, "managed"));
    let mut events = host.start_plugins().unwrap();
    until(&mut host, &mut events, |host| phase(host) == Phase::Running).await;
    let owner = host.app.plugins.manager_entries[0].owner.unwrap();
    host.app.buffers[0] = crate::buffer::Buffer::provider_document(
        crate::buffer::ProviderDocument {
            identity: crate::buffer::ProviderIdentity {
                configured_plugin: "managed".into(),
                provider: "remote".into(),
                key: "notes".into(),
            },
            label: "Notes".into(),
            syntax_hint: None,
            version: "v1".into(),
            generation: host.app.plugins.instances[&owner]
                .application
                .generation
                .clone(),
            available: true,
            baseline_epoch: 0,
            uncertain: Some(crate::buffer::ProviderUncertain {
                text: crate::text::Text::from_str("pending upload"),
                version: "v1".into(),
            }),
        },
        "pending upload".into(),
    );
    let charge = plugin::provider::READ_CHARGE;
    host.provider_uncertain
        .insert(0, ("managed".into(), charge, "old-writer".into()));
    host.app.plugins.orphaned_payload += charge;
    action(&mut host, 0, Action::Restart);
    until(&mut host, &mut events, |host| phase(host) == Phase::Running).await;
    assert_eq!(host.provider_uncertain[&0].1, charge);
    assert!(host.app.buffers[0].dirty);
    assert!(!host.app.buffers[0].provider().unwrap().available);
    assert!(host.app.buffers[0].provider().unwrap().uncertain.is_some());
    assert_eq!(host.app.plugins.orphaned_payload, charge);
    assert_eq!(host.app.plugins.manager_entries[0].cleanup, 0);
}
