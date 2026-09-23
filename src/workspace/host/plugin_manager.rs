// SPDX-License-Identifier: MPL-2.0

//! Explicit configured-plugin lifecycle. Retired owners stay fenced until every
//! producer and accepted asynchronous operation has actually settled.
use super::WorkspaceHost;
use crate::{
    app::plugin_workflows::Instance,
    notification::{NotificationDraft, NotificationSeverity},
    plugin::{
        self, HostMessage, PluginConfig,
        manager::{Action, Entry, Intent, Phase},
    },
};

pub(super) struct Record {
    /// The configuration this plugin is admitted and launched from, kept even
    /// while it is disabled or invalid so a reload can tell an entry that
    /// changed from one that only moved.
    configured: PluginConfig,
    /// Whether the configuration no longer describes this plugin. A retired
    /// record exists only to keep a generation's cleanup visible, and leaves
    /// the manager as soon as nothing of that generation is outstanding.
    retired: bool,
    /// A reloaded entry that could not reach this plugin yet, held until the
    /// work it would have interrupted is gone.
    pending: Option<Reloaded>,
    pub entry: Entry,
    settled: bool,
    cleanup_failed: bool,
    failed: bool,
}
impl WorkspaceHost {
    pub(super) fn retain_plugin_failure(&mut self, owner: usize, reason: &str) {
        if let Some(record) = self
            .plugin_manager
            .iter_mut()
            .find(|record| record.entry.owner == Some(owner))
        {
            record.entry.diagnostic = Some(
                reason
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(1024)
                    .collect(),
            );
        }
    }

    pub(super) fn initialize_plugin_manager(&mut self) {
        if self.refuse_excessive_plugin_configs() {
            return;
        }
        let configs = self.app.config.plugins.clone();
        let diagnostics = admission_diagnostics(&configs);
        self.plugin_manager = configs
            .iter()
            .zip(diagnostics)
            .enumerate()
            .map(|(index, (config, diagnostic))| Record::new(index, config, diagnostic))
            .collect();
        for index in 0..self.plugin_manager.len() {
            if self.plugin_manager[index].entry.valid && self.plugin_manager[index].entry.enabled {
                self.launch_managed_plugin(index);
            }
        }
        self.publish_plugin_manager();
    }

    /// Reports a configuration holding more plugin entries than the manager
    /// admits, leaving whatever is running alone.
    fn refuse_excessive_plugin_configs(&mut self) -> bool {
        if self.app.config.plugins.len() <= plugin::manager::MAX_CONFIGS {
            return false;
        }
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
            compatibility: String::new(),
        }]);
        true
    }

    fn launch_managed_plugin(&mut self, index: usize) {
        if self.plugin_workers.len() >= plugin::MAX_PLUGINS {
            return;
        }
        let record = &self.plugin_manager[index];
        if !(record.entry.valid && record.entry.enabled) {
            return;
        }
        let config = record.configured.clone();
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
        if let Err(reason) = config_admission(&config) {
            let record = &mut self.plugin_manager[index];
            record.entry.phase = Phase::Failed;
            record.entry.diagnostic = Some(reason);
            record.failed = true;
            return;
        }
        let (worker, sender) =
            plugin::spawn(config.clone(), self.app.project_root.clone(), owner, events);
        let hello = HostMessage::Application(plugin::application::HostMessage::Hello {
            version: plugin::application::VERSION,
            host_version: plugin::compatibility::HOST_VERSION,
            capabilities: plugin::application::host_capabilities().to_vec(),
            features: plugin::application::FEATURES.to_vec(),
            limits: Default::default(),
        });
        let hello_failed = sender.try_send(hello).is_err();
        self.plugin_workers.insert(owner, worker);
        self.app.plugins.instances.insert(
            owner,
            Instance {
                config,
                sender,
                registered: false,
                application: Default::default(),
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
            if requested || record.entry.diagnostic.is_none() {
                record.entry.diagnostic = Some(
                    if requested {
                        "Stopped by user"
                    } else {
                        "Plugin stopped after a protocol or service failure"
                    }
                    .into(),
                );
            }
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
                if record.entry.diagnostic.is_none()
                    || record.entry.diagnostic.as_deref()
                        == Some("Plugin stopped after a protocol or service failure")
                    || !reaped
                {
                    record.entry.diagnostic = Some(failure);
                }
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

    /// Brings the plugin manager into line with a reloaded configuration.
    ///
    /// Records are matched by configured ID rather than by position, so moving
    /// an entry in the file is not a change to the plugin it describes. An
    /// entry whose configuration is byte-for-byte what its running process was
    /// launched from is left completely alone: reloading because of an
    /// unrelated edit must not cost every plugin its state.
    ///
    /// Where a change does reach a running plugin, work in flight decides what
    /// happens to it. A plugin with no running job, activity, helper process,
    /// pending cleanup, or unsaved provider document is restarted immediately.
    /// One that holds any of those keeps running with what it was launched
    /// from, and the change waits on its record until that work is gone; see
    /// [`Self::settle_deferred_plugin_entry`]. `:plugin-restart` and
    /// `:plugin-stop` only bring that forward. Nothing is stopped to make a
    /// reload take effect sooner.
    fn reconcile_plugin_configuration(&mut self) {
        if self.app.config.plugins.len() > plugin::manager::MAX_CONFIGS {
            // Deliberately not the synthetic startup entry: the manager is
            // already published from real records, and replacing it with one
            // error row would leave every action index pointing at a plugin it
            // does not name.
            self.report_host_error(
                "At most 128 plugins may be configured; the running plugins are unchanged",
            );
            return;
        }
        let configs = self.app.config.plugins.clone();
        let diagnostics = admission_diagnostics(&configs);
        let mut previous = std::mem::take(&mut self.plugin_manager);
        let mut records: Vec<Record> = Vec::with_capacity(configs.len());
        // Each change paired with the record position it is to be applied to.
        let mut affected: Vec<(usize, Reloaded)> = Vec::new();

        for (index, (config, diagnostic)) in configs.iter().zip(diagnostics).enumerate() {
            let matched = plugin::valid_name(&config.id)
                .then(|| {
                    previous
                        .iter()
                        .position(|record| record.entry.configured_id == config.id)
                })
                .flatten();
            let Some(matched) = matched else {
                // An entry no running record answers for is entirely new, so
                // it takes the same path a changed one does and starts if the
                // configuration asks for it.
                affected.push((index, Reloaded::new(config, diagnostic.clone())));
                records.push(Record::new(index, config, diagnostic));
                continue;
            };
            let mut record = previous.remove(matched);
            let valid = diagnostic.is_none();
            record.entry.config_index = index;
            let returning = std::mem::replace(&mut record.retired, false);
            let unchanged = record.configured == *config && record.entry.valid == valid;
            // A record that was retired publishes an entry saying the file
            // disowned it, so matching it again has to undo that even when the
            // configuration text never changed. When the process it describes
            // is still running exactly this entry, only the claim is wrong:
            // repairing it in place is what keeps a remove-and-restore from
            // costing a plugin the generation it was already running.
            //
            // The parked change that retirement left is then the removal the
            // file has just taken back, and goes with it. What that removal
            // may have overwritten does not: an edit parked before it and
            // never delivered still has to reach the process, so the record
            // takes the ordinary path instead, which reports the wait as every
            // other deferral does. The generation's own launch configuration is
            // the authority on which of the two this is, and answering that
            // question at all needs a live instance, so a returning record with
            // nothing running never takes the shortcut.
            let delivered = !returning || self.plugin_generation_runs(record.entry.owner, config);
            if unchanged && delivered {
                if returning {
                    record.entry.enabled = config.enabled;
                    record.entry.diagnostic = diagnostic;
                    record.pending = None;
                }
                records.push(record);
                continue;
            }
            // The target configuration is adopted immediately even when the
            // change cannot reach the running process yet. Everything that
            // eventually starts this plugin reads it: the settle above, and
            // `:plugin-restart` when someone would rather not wait. Either way
            // the plugin comes back as the file saved it rather than as the
            // stale entry the reload found running.
            record.configured = config.clone();
            affected.push((records.len(), Reloaded::new(config, diagnostic)));
            records.push(record);
        }

        // An entry the file no longer has is gone once nothing of it is left.
        // One that still owns a generation stays visible until it settles,
        // because a manager that hid it would be claiming a cleanup finished
        // that has not.
        for mut record in previous {
            if record.entry.owner.is_none() {
                continue;
            }
            let index = records.len();
            record.retired = true;
            record.entry.config_index = index;
            affected.push((
                index,
                Reloaded {
                    id: record.entry.configured_id.clone(),
                    // Validity is the running generation's, not a judgement on
                    // an entry that no longer exists: it is what keeps
                    // `:plugin-stop` available for the process still running.
                    valid: record.entry.valid,
                    enabled: false,
                    compatibility: record.entry.compatibility.clone(),
                    diagnostic: Some("Removed from configuration".into()),
                },
            ));
            records.push(record);
        }

        self.plugin_manager = records;
        let mut applied = Vec::new();
        let mut deferred = Vec::new();
        for (index, reloaded) in affected {
            let id = reloaded.id.clone();
            if self.apply_reloaded_plugin_entry(index, reloaded) {
                applied.push(id);
            } else {
                deferred.push(id);
            }
        }
        self.report_plugin_reconciliation(&applied, &deferred);
    }

    /// Applies one reloaded entry. Returns whether the change reached it.
    ///
    /// A change that cannot reach a plugin yet is kept on its record rather
    /// than dropped, so the entry stops describing a generation that is gone
    /// as soon as the work protecting it does.
    fn apply_reloaded_plugin_entry(&mut self, index: usize, reloaded: Reloaded) -> bool {
        let wanted = reloaded.valid && reloaded.enabled;
        let record = &self.plugin_manager[index];
        let live = record
            .entry
            .owner
            .is_some_and(|owner| self.app.plugins.instances.contains_key(&owner));

        if live && self.plugin_reload_protected_work(index) != 0 {
            // `valid` stays the verdict on the generation that is actually
            // running. Stopping a plugin is refused for an entry marked
            // invalid, and stopping is exactly what someone needs for a plugin
            // whose new configuration this host cannot run, so adopting that
            // verdict early would strand the process this branch exists to
            // protect. Whether the file still enables the plugin is not in
            // doubt, so that is adopted at once: it is what decides between
            // restarting into the saved entry and stopping. A restart into an
            // inadmissible entry then fails at launch and says why, which is a
            // report rather than a plugin nothing can reach.
            let record = &mut self.plugin_manager[index];
            record.entry.enabled = reloaded.enabled;
            let reason = reloaded
                .diagnostic
                .as_deref()
                .unwrap_or("Configuration changed");
            let apply = if reloaded.enabled {
                ":plugin-restart"
            } else {
                ":plugin-stop"
            };
            record.entry.diagnostic =
                Some(format!("{reason}; work is in flight, {apply} applies it"));
            record.pending = Some(reloaded);
            return false;
        }

        let Reloaded {
            valid,
            enabled,
            compatibility,
            diagnostic,
            ..
        } = reloaded;
        let record = &mut self.plugin_manager[index];
        record.pending = None;
        record.entry.valid = valid;
        record.entry.enabled = enabled;
        record.entry.compatibility = compatibility;
        record.entry.diagnostic = diagnostic.clone();
        record.failed = !valid;

        if !live {
            let settled = record.settled;
            let cleanup_failed = record.cleanup_failed;
            let cleanup = record
                .entry
                .owner
                .map_or(0, |owner| self.plugin_owner_work(owner));
            let record = &mut self.plugin_manager[index];
            if wanted && !cleanup_failed {
                // A previous generation still shedding work cannot be replaced
                // yet. `RestartPending` is the established way to say "launch
                // once it settles", and the ordinary sync loop honours it.
                record.entry.phase = Phase::RestartPending;
                if settled && cleanup == 0 {
                    self.launch_managed_plugin(index);
                }
            } else {
                // A generation whose cleanup failed is not a candidate for
                // either state: it is still the failure the manager refuses to
                // restart, exactly as `:plugin-restart` refuses it.
                record.failed = record.failed || cleanup_failed;
                record.entry.phase = if record.failed {
                    Phase::Failed
                } else {
                    Phase::Disabled
                };
            }
            return true;
        }

        let owner = self.plugin_manager[index]
            .entry
            .owner
            .expect("a live plugin has an owner");
        if wanted {
            let record = &mut self.plugin_manager[index];
            record.entry.phase = Phase::RestartPending;
        }
        // The same deliberate stop `:plugin-stop` performs, rather than the
        // failure path: nothing went wrong, the entry describing it changed.
        self.stop_plugin(owner, "stopped by user");
        // `manager_stopping` treats a requested stop as a clean one, so the
        // reloaded entry's own verdict is restored over it.
        let record = &mut self.plugin_manager[index];
        record.failed = !valid;
        if !wanted {
            record.entry.diagnostic = diagnostic;
        }
        true
    }

    /// Whether the generation this record has running was launched from
    /// `config`, which is the only thing that can say what has actually
    /// reached the process rather than only the record describing it.
    fn plugin_generation_runs(&self, owner: Option<usize>, config: &PluginConfig) -> bool {
        owner.is_some_and(|owner| {
            self.app
                .plugins
                .instances
                .get(&owner)
                .is_some_and(|instance| instance.config == *config)
        })
    }

    /// Work a reloaded plugin would lose if it were restarted now.
    ///
    /// Counts what the plugin itself is doing, what the host still owes it,
    /// and the remote documents it is the only route to saving. An uncertain
    /// provider write is included because the outcome of the write it
    /// describes is exactly what a restart would make unknowable.
    ///
    /// The instance's own side is everything `stop_plugin` would cancel or
    /// orphan: a command invocation whose response has not arrived, accepted
    /// local filesystem work, view-model preparation, an issued staging
    /// handle, and an input surface the person is currently answering. A job
    /// and an activity lease are the two a plugin declares deliberately, but
    /// nothing says a plugin must declare one before doing something a restart
    /// would ruin.
    ///
    /// Only asked of a plugin with a live instance. `Record::settled` is about
    /// a previous generation being reaped, which is a separate question that
    /// the not-running path answers.
    fn plugin_reload_protected_work(&self, index: usize) -> usize {
        let record = &self.plugin_manager[index];
        let id = record.entry.configured_id.as_str();
        let owner = record.entry.owner.map_or(0, |owner| {
            self.plugin_owner_work(owner)
                + self
                    .app
                    .plugins
                    .instances
                    .get(&owner)
                    .map_or(0, |instance| {
                        instance
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
                            .count()
                            + instance.application.activities.len()
                            + instance.application.processes.len()
                            + instance.application.requests.len()
                            + instance.application.local_requests.len()
                            + instance.application.model_requests.len()
                            + instance.application.staging.len()
                            + instance.application.input_surfaces.len()
                    })
        });
        owner
            + self
                .app
                .buffers
                .iter()
                .filter(|buffer| {
                    buffer.provider().is_some_and(|document| {
                        document.identity.configured_plugin == id
                            && (buffer.holds_unsaved_work() || document.uncertain.is_some())
                    })
                })
                .count()
    }

    fn report_plugin_reconciliation(&mut self, applied: &[String], deferred: &[String]) {
        if !applied.is_empty() {
            self.app
                .push_background_notification(NotificationDraft::new(
                    NotificationSeverity::Info,
                    "Plugins",
                    "Configuration applied",
                    format!("configuration reload applied to: {}", applied.join(", ")),
                ));
        }
        if !deferred.is_empty() {
            self.app.push_background_notification(NotificationDraft::new(
                NotificationSeverity::Warning,
                "Plugins",
                "Configuration change pending",
                format!(
                    "{} kept running with the configuration {} started with because work is still in flight; the saved entry takes effect when that work finishes, and :plugin-restart or :plugin-stop applies it sooner",
                    deferred.join(", "),
                    if deferred.len() == 1 { "it" } else { "they" },
                ),
            ));
        }
        self.publish_plugin_manager();
    }

    pub(super) fn sync_plugin_manager(&mut self) {
        let had_records = !self.plugin_manager.is_empty();
        // Intents were captured against the entries the frontend was shown, so
        // they are applied before a reload renumbers anything.
        for intent in self.app.take_plugin_manager_intents() {
            self.manager_action(intent);
        }
        if std::mem::take(&mut self.app.plugins.configuration_reload) {
            self.reconcile_plugin_configuration();
        }
        for index in 0..self.plugin_manager.len() {
            if self.plugin_manager[index].pending.is_some() {
                self.settle_deferred_plugin_entry(index);
            }
            let owner = self.plugin_manager[index].entry.owner;
            if let Some(instance) = owner.and_then(|owner| self.app.plugins.instances.get(&owner)) {
                let entry = &mut self.plugin_manager[index].entry;
                if instance.registered {
                    entry.phase = Phase::Running;
                }
                entry.granted = instance.application.capabilities.iter().cloned().collect();
                if instance.registered {
                    entry.compatibility = format!(
                        "Host {}; {}; effective {}; features {}",
                        plugin::compatibility::HOST_VERSION,
                        plugin::VERSION,
                        instance.application.runyte,
                        if instance.application.features.is_empty() {
                            "none".into()
                        } else {
                            instance
                                .application
                                .features
                                .iter()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    );
                }
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
        self.prune_retired_plugins();
        if had_records && self.plugin_manager.is_empty() {
            // `publish_plugin_manager` never publishes an empty list, so that
            // a startup refusal's synthetic entry survives every later sync. A
            // configuration that removed its last plugin therefore has to
            // clear the manager explicitly.
            self.app.update_plugin_manager(Vec::new());
        } else {
            self.publish_plugin_manager();
        }
    }

    /// Delivers a change a reload had to park, once nothing is left to protect.
    ///
    /// A deferral is a wait, not a resting state. The reload was asked for and
    /// the only reason it did not reach this plugin was work in flight, so the
    /// moment that work is gone the saved entry takes effect on its own.
    /// `:plugin-restart` and `:plugin-stop` remain the way to have it sooner.
    fn settle_deferred_plugin_entry(&mut self, index: usize) {
        let live = self.plugin_manager[index]
            .entry
            .owner
            .is_some_and(|owner| self.app.plugins.instances.contains_key(&owner));
        if live && self.plugin_reload_protected_work(index) != 0 {
            return;
        }
        let Some(pending) = self.plugin_manager[index].pending.take() else {
            return;
        };
        let id = pending.id.clone();
        if self.apply_reloaded_plugin_entry(index, pending) {
            self.app
                .push_background_notification(NotificationDraft::new(
                    NotificationSeverity::Info,
                    "Plugins",
                    "Configuration applied",
                    format!("{id} finished its work and took the reloaded configuration"),
                ));
        }
    }

    /// Drops records the configuration no longer describes, once their
    /// generation has nothing outstanding.
    ///
    /// Retired records sit after every configured one, so removing them cannot
    /// renumber a configured entry. Intents captured against a retired entry
    /// still carry its owner, which is what refuses a stale one.
    fn prune_retired_plugins(&mut self) {
        if !self.plugin_manager.iter().any(|record| record.retired) {
            return;
        }
        let finished = self
            .plugin_manager
            .iter()
            .map(|record| {
                record.retired
                    && record.settled
                    && !record.cleanup_failed
                    && record.entry.owner.is_none_or(|owner| {
                        !self.app.plugins.instances.contains_key(&owner)
                            && !self.plugin_workers.contains_key(&owner)
                            && self.plugin_owner_work(owner) == 0
                    })
            })
            .collect::<Vec<_>>();
        let mut keep = finished.iter();
        self.plugin_manager
            .retain(|_| !keep.next().copied().unwrap_or(false));
        for (index, record) in self.plugin_manager.iter_mut().enumerate() {
            record.entry.config_index = index;
        }
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
impl Record {
    fn new(index: usize, config: &PluginConfig, diagnostic: Option<String>) -> Self {
        let valid = diagnostic.is_none();
        Self {
            configured: config.clone(),
            retired: false,
            pending: None,
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
                diagnostic,
                compatibility: compatibility_label(config),
            },
        }
    }
}

/// What a reloaded configuration says about one plugin, before any of it has
/// been applied to the record describing the plugin that is running.
///
/// Keeping this separate from the published [`Entry`] is what lets a change be
/// deferred: the entry keeps answering for the live generation while this
/// holds what the file now asks for.
///
/// It deliberately carries no record position. A parked change outlives the
/// reconciliation that produced it, and both pruning and a later reload can
/// renumber the manager, so a position captured here would be a stale index
/// waiting to be written through. Whoever applies one always knows where the
/// record is.
struct Reloaded {
    id: String,
    valid: bool,
    enabled: bool,
    compatibility: String,
    diagnostic: Option<String>,
}

impl Reloaded {
    fn new(config: &PluginConfig, diagnostic: Option<String>) -> Self {
        Self {
            id: config.id.clone(),
            valid: diagnostic.is_none(),
            enabled: config.enabled,
            compatibility: compatibility_label(config),
            diagnostic,
        }
    }
}

/// Why each configured entry cannot be admitted, in configuration order.
///
/// The set-wide refusals are decided here too, so that startup and a reload
/// judge a duplicate ID or one plugin too many the same way.
fn admission_diagnostics(configs: &[PluginConfig]) -> Vec<Option<String>> {
    let excessive_enabled =
        configs.iter().filter(|config| config.enabled).count() > plugin::MAX_PLUGINS;
    let mut ids = std::collections::BTreeMap::new();
    for config in configs {
        *ids.entry(config.id.as_str()).or_insert(0usize) += 1;
    }
    configs
        .iter()
        .map(|config| {
            if excessive_enabled {
                Some("At most 8 plugins may be enabled".to_owned())
            } else if ids[config.id.as_str()] != 1 {
                Some("Duplicate configured plugin ID".to_owned())
            } else {
                config_admission(config).err()
            }
        })
        .collect()
}

fn compatibility_label(config: &PluginConfig) -> String {
    plugin::compatibility::ReleaseRange::parse(&config.runyte).map_or_else(
        |_| String::new(),
        |range| {
            format!(
                "Host {}; configured {}",
                plugin::compatibility::HOST_VERSION,
                range.normalized()
            )
        },
    )
}

fn valid_config(config: &PluginConfig) -> bool {
    plugin::valid_name(&config.id)
        && config.executable.is_absolute()
        && config.args.len() <= 32
        && config.args.iter().map(String::len).sum::<usize>() <= 8192
        && config.bindings.len() <= plugin::application::MAX_COMMANDS
        && config.capabilities.len() <= 32
        && config
            .capabilities
            .iter()
            .all(|cap| plugin::valid_name(cap))
}

fn config_admission(config: &PluginConfig) -> Result<(), String> {
    if !valid_config(config) {
        return Err("Invalid plugin configuration".into());
    }
    if config.api != plugin::application::VERSION {
        return Err("Plugin requires explicit api: runyte-1; regenerate its configuration".into());
    }
    let range = plugin::compatibility::ReleaseRange::parse(&config.runyte).map_err(|_| {
        "Missing or invalid Runyte range; use runyte: \">=0.3.0, <0.4.0\"".to_owned()
    })?;
    let version = plugin::compatibility::Version::parse(plugin::compatibility::HOST_VERSION)
        .map_err(|error| error.to_string())?;
    if !range.contains(&version) {
        return Err(format!(
            "Host {} is outside configured Runyte range {}",
            plugin::compatibility::HOST_VERSION,
            range.normalized()
        ));
    }
    Ok(())
}
