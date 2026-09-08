// SPDX-License-Identifier: MPL-2.0

//! Explicit configured-plugin lifecycle. Retired owners stay fenced until every
//! producer and accepted asynchronous operation has actually settled.
use super::WorkspaceHost;
use crate::{
    app::plugin_workflows::Instance,
    plugin::{
        self, HostMessage, PluginConfig,
        manager::{Action, Entry, Intent, Phase},
    },
};
use std::collections::BTreeSet;

pub(super) struct Record {
    config: Option<PluginConfig>,
    pub entry: Entry,
    settled: bool,
    cleanup_failed: bool,
    failed: bool,
}
impl WorkspaceHost {
    pub(super) fn initialize_plugin_manager(&mut self) {
        if self.app.config.plugins.len() > plugin::manager::MAX_CONFIGS {
            self.report_host_error("At most 128 plugins may be configured");
            self.app.update_plugin_manager(vec![Entry {
                config_index: 0,
                configured_id: "configuration".into(),
                enabled: false,
                valid: false,
                phase: Phase::Failed,
                owner: None,
                granted: vec![],
                jobs: 0,
                activities: 0,
                helpers: 0,
                cleanup: 0,
                diagnostic: Some("At most 128 plugins may be configured".into()),
            }]);
            return;
        }
        let configs = &self.app.config.plugins;
        let excessive_enabled =
            configs.iter().filter(|config| config.enabled).count() > plugin::MAX_PLUGINS;
        let mut ids = std::collections::BTreeMap::new();
        for config in configs {
            *ids.entry(config.id.as_str()).or_insert(0usize) += 1;
        }
        self.plugin_manager = configs
            .iter()
            .enumerate()
            .map(|(index, config)| {
                let valid =
                    valid_config(config) && ids[config.id.as_str()] == 1 && !excessive_enabled;
                let diagnostic = if excessive_enabled {
                    Some("At most 8 plugins may be enabled")
                } else if ids[config.id.as_str()] != 1 {
                    Some("Duplicate configured plugin ID")
                } else if !valid {
                    Some("Invalid plugin configuration")
                } else {
                    None
                };
                Record {
                    config: (valid && config.enabled).then(|| config.clone()),
                    settled: true,
                    cleanup_failed: false,
                    failed: !valid,
                    entry: Entry {
                        config_index: index,
                        configured_id: if plugin::valid_name(&config.id) {
                            config.id.clone()
                        } else {
                            format!("Invalid entry {}", index + 1)
                        },
                        enabled: config.enabled,
                        valid,
                        phase: if !valid {
                            Phase::Failed
                        } else if config.enabled {
                            Phase::Stopped
                        } else {
                            Phase::Disabled
                        },
                        owner: None,
                        granted: vec![],
                        jobs: 0,
                        activities: 0,
                        helpers: 0,
                        cleanup: 0,
                        diagnostic: diagnostic.map(str::to_owned),
                    },
                }
            })
            .collect();
        for index in 0..self.plugin_manager.len() {
            if self.plugin_manager[index].entry.valid && self.plugin_manager[index].entry.enabled {
                self.launch_managed_plugin(index);
            }
        }
        self.publish_plugin_manager();
    }

    fn launch_managed_plugin(&mut self, index: usize) {
        if self.plugin_workers.len() >= plugin::MAX_PLUGINS {
            return;
        }
        let Some(events) = self.plugin_events_sender.clone() else {
            return;
        };
        let Some(next) = self.next_plugin_owner.checked_add(1) else {
            let record = &mut self.plugin_manager[index];
            record.entry.phase = Phase::Failed;
            record.entry.diagnostic = Some("Plugin owner IDs exhausted".into());
            record.failed = true;
            return;
        };
        let owner = self.next_plugin_owner;
        self.next_plugin_owner = next;
        let Some(config) = self.plugin_manager[index].config.clone() else {
            return;
        };
        let (worker, sender) =
            plugin::spawn(config.clone(), self.app.project_root.clone(), owner, events);
        let hello = if config.api == plugin::application::Api::Epoch2 {
            HostMessage::Application(plugin::application::HostMessage::Hello {
                version: plugin::application::VERSION,
                capabilities: plugin::application::CAPABILITIES.to_vec(),
                limits: Default::default(),
            })
        } else {
            HostMessage::Hello {
                version: plugin::VERSION,
            }
        };
        let hello_failed = sender.try_send(hello).is_err();
        self.plugin_workers.insert(owner, worker);
        self.app.plugins.instances.insert(
            owner,
            Instance {
                config,
                sender,
                registered: false,
                application: Default::default(),
                pending: None,
                issued: BTreeSet::new(),
                subscriptions: Default::default(),
                sequence: 0,
            },
        );
        let record = &mut self.plugin_manager[index];
        record.entry.owner = Some(owner);
        record.entry.phase = Phase::Starting;
        record.entry.diagnostic = None;
        record.entry.granted.clear();
        record.entry.cleanup = 0;
        record.entry.jobs = 0;
        record.entry.activities = 0;
        record.entry.helpers = 0;
        record.settled = false;
        record.cleanup_failed = false;
        record.failed = false;
        if hello_failed {
            self.stop_plugin(owner, "Plugin startup channel unavailable");
        }
    }

    pub(super) fn manager_stopping(&mut self, owner: usize, requested: bool) {
        if let Some(record) = self
            .plugin_manager
            .iter_mut()
            .find(|record| record.entry.owner == Some(owner))
        {
            if record.cleanup_failed {
                return;
            }
            if record.entry.phase != Phase::RestartPending {
                record.entry.phase = Phase::Stopping;
            }
            record.failed = !requested;
            record.entry.diagnostic = Some(
                if requested {
                    "Stopped by user"
                } else {
                    "Plugin stopped after a protocol or service failure"
                }
                .into(),
            );
        }
        if let Some(worker) = self.plugin_workers.get(&owner) {
            worker.stop();
        }
    }

    pub(super) fn manager_worker_stopped(
        &mut self,
        owner: usize,
        failure: Option<String>,
        reaped: bool,
    ) {
        if self.app.plugins.instances.contains_key(&owner) {
            self.stop_plugin(
                owner,
                failure.as_deref().unwrap_or("Plugin process stopped"),
            );
        }
        if reaped {
            self.plugin_workers.remove(&owner);
        }
        if let Some(record) = self
            .plugin_manager
            .iter_mut()
            .find(|record| record.entry.owner == Some(owner))
        {
            record.settled = reaped;
            record.cleanup_failed = !reaped;
            if let Some(failure) = failure {
                if record.entry.phase != Phase::RestartPending {
                    record.failed = true;
                }
                record.entry.diagnostic = Some(failure);
            }
            if !reaped {
                record.entry.phase = Phase::Failed;
                record.failed = true;
            }
        }
    }

    fn manager_action(&mut self, intent: Intent) {
        let Some(record) = self.plugin_manager.get_mut(intent.config_index) else {
            return;
        };
        if record.entry.owner != intent.expected_owner {
            self.app
                .plugin_manager_refused("Plugin changed; reopen its actions");
            return;
        }
        match intent.action {
            Action::Stop => {
                if record.entry.owner.is_none() {
                    if record.entry.valid && record.entry.enabled {
                        record.entry.phase = Phase::Stopped;
                    }
                    return;
                }
                if record.entry.phase == Phase::RestartPending {
                    record.entry.phase = Phase::Stopping;
                }
                if let Some(owner) = record.entry.owner {
                    self.stop_plugin(owner, "stopped by user");
                }
            }
            Action::Restart => {
                if !record.entry.valid || !record.entry.enabled || record.cleanup_failed {
                    self.app.plugin_manager_refused(
                        "This plugin cannot restart; check its configuration or cleanup status",
                    );
                    return;
                }
                record.entry.phase = Phase::RestartPending;
                record.failed = false;
                if let Some(owner) = record.entry.owner {
                    self.stop_plugin(owner, "stopped by user");
                }
            }
        }
    }

    pub(super) fn sync_plugin_manager(&mut self) {
        for intent in self.app.take_plugin_manager_intents() {
            self.manager_action(intent);
        }
        for index in 0..self.plugin_manager.len() {
            let owner = self.plugin_manager[index].entry.owner;
            if let Some(instance) = owner.and_then(|owner| self.app.plugins.instances.get(&owner)) {
                let entry = &mut self.plugin_manager[index].entry;
                if instance.registered {
                    entry.phase = Phase::Running;
                }
                entry.granted = instance.application.capabilities.iter().cloned().collect();
                entry.jobs = instance
                    .application
                    .jobs
                    .values()
                    .filter(|job| {
                        matches!(
                            job.state,
                            plugin::application::JobState::Running
                                | plugin::application::JobState::Cancelling
                        )
                    })
                    .count();
                entry.activities = instance.application.activities.len();
                entry.helpers = instance.application.processes.len();
                continue;
            }
            let cleanup = owner.map_or(0, |owner| self.plugin_owner_work(owner));
            let record = &mut self.plugin_manager[index];
            record.entry.jobs = 0;
            record.entry.activities = 0;
            record.entry.helpers = 0;
            record.entry.cleanup = cleanup + usize::from(!record.settled);
            if !record.settled || cleanup != 0 {
                continue;
            }
            if record.entry.phase == Phase::RestartPending {
                self.launch_managed_plugin(index);
            } else if record.entry.owner.is_some() {
                // Keep the last owner token even while stopped: a menu opened
                // before an intervening failed generation must not pass an ABA
                // comparison merely because both snapshots have no live worker.
                record.entry.phase = if record.failed {
                    Phase::Failed
                } else {
                    Phase::Stopped
                };
            }
        }
        self.publish_plugin_manager();
    }

    fn publish_plugin_manager(&mut self) {
        if !self.plugin_manager.is_empty() {
            self.app.update_plugin_manager(
                self.plugin_manager
                    .iter()
                    .map(|record| record.entry.clone())
                    .collect(),
            );
        }
    }
    fn plugin_owner_work(&self, owner: usize) -> usize {
        self.plugin_recoveries
            .values()
            .filter(|pending| pending.owner == owner)
            .count()
            + self
                .plugin_local_orphans
                .keys()
                .filter(|(id, _, _)| *id == owner)
                .count()
            + self
                .plugin_handoffs
                .keys()
                .filter(|(id, _, _)| *id == owner)
                .count()
            + self
                .plugin_state_requests
                .values()
                .filter(|pending| pending.owner == owner)
                .count()
            + self
                .plugin_processes
                .values()
                .filter(|process| process.owner == owner)
                .count()
            + self
                .document_saves
                .values()
                .filter(|pending| pending.owner == owner)
                .count()
            + usize::from(
                self.filesystem_apply
                    .as_ref()
                    .is_some_and(|pending| pending.owner == owner),
            )
            + self
                .provider_reads
                .values()
                .filter(|pending| pending.owner == owner || pending.requester == owner)
                .count()
            + self
                .provider_writes
                .values()
                .filter(|pending| pending.owner == owner || pending.requester == owner)
                .count()
    }
}
fn valid_config(config: &PluginConfig) -> bool {
    plugin::valid_name(&config.id)
        && config.executable.is_absolute()
        && config.args.len() <= 32
        && config.args.iter().map(String::len).sum::<usize>() <= 8192
        && config.bindings.len()
            <= if config.api == plugin::application::Api::Epoch2 {
                plugin::application::MAX_COMMANDS
            } else {
                plugin::MAX_COMMANDS
            }
        && config.capabilities.len() <= 32
        && config
            .capabilities
            .iter()
            .all(|cap| plugin::valid_name(cap))
}
