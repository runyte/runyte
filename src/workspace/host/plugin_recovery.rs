// SPDX-License-Identifier: MPL-2.0
use super::{WorkspaceHost, plugin_providers::ReadPurpose};
use crate::{
    app::plugin_recovery::ProviderReloadIntent,
    buffer::{
        PreparedProviderReload, ProviderReloadChoice, ProviderReloadGuard, ProviderReloadSource,
    },
    plugin::{self, application as api, provider as wire},
};
use api::{Error, ErrorCode as Code};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
const REMOTE_PREPARE_CHARGE: usize = 16 * 1024 * 1024;

pub(super) struct Pending {
    pub owner: usize,
    generation: String,
    intent: ProviderReloadIntent,
    guard: ProviderReloadGuard,
    source: Option<ProviderReloadSource>,
    prepared: Option<PreparedProviderReload>,
    requires_choice: bool,
    cancelled: Arc<AtomicBool>,
    worker: bool,
    retired: bool,
    orphaned: bool,
    cleanup_counted: bool,
    uncertainty_charge: bool,
    accepted_choice: ProviderReloadChoice,
    charge: usize,
    deadline: Instant,
}
impl Pending {
    pub(super) fn retains_uncertainty(&self, buffer: usize) -> bool {
        self.worker && self.uncertainty_charge && self.intent.buffer == buffer
    }
    pub(super) fn preparing_or_ready(&self) -> bool {
        self.worker || self.prepared.is_some()
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
fn conflict(message: &str) -> Error {
    Error::new(Code::ContextChanged, message)
}

impl WorkspaceHost {
    fn start_provider_reload(&mut self, intent: ProviderReloadIntent) -> Result<(), Error> {
        self.app
            .validate_provider_reload_intent(&intent)
            .map_err(|error| conflict(&error.to_string()))?;
        let local_bytes = self.app.buffers[intent.buffer].len_bytes();
        if local_bytes > wire::MAX_DOCUMENT_BYTES {
            return Err(Error::new(
                Code::LimitExceeded,
                "Local text exceeds the 8 MiB reload limit",
            ));
        }
        let prepare_charge = REMOTE_PREPARE_CHARGE + 2 * local_bytes;
        let owner = self
            .app
            .plugins
            .instances
            .iter()
            .find(|(_, instance)| {
                instance.registered
                    && instance.config.id == intent.identity.configured_plugin
                    && instance
                        .application
                        .providers
                        .contains_key(&intent.identity.provider)
            })
            .map(|(owner, _)| *owner)
            .ok_or_else(|| {
                Error::new(
                    Code::Unavailable,
                    "Resource provider is unavailable; restart it first",
                )
            })?;
        let read_charge = if self.provider_uncertain.contains_key(&intent.buffer) {
            0
        } else {
            wire::READ_CHARGE
        };
        // Maximum remote forward String/new rope: 16 MiB. The pinned local
        // source and inverse scale with the captured byte count. The separate
        // 16 MiB read/unknown reservation covers rope node/slack and metadata.
        // Reserve before pinning the local source; total prep stays <=32 MiB.
        self.reserve_application_payload(owner, prepare_charge + read_charge)?;
        let source = self.app.buffers[intent.buffer]
            .provider_reload_source()
            .map_err(|error| Error::new(Code::LimitExceeded, &error.to_string()))?;
        if source.requires_settlement() && !self.provider_uncertain.contains_key(&intent.buffer) {
            return Err(Error::new(
                Code::OutcomeUnknown,
                "Previous write settlement reference is unavailable",
            ));
        }
        let generation = self.app.plugins.instances[&owner]
            .application
            .generation
            .clone();
        let (result, _) = self.application_provider_read(
            owner,
            api::Request::ResourceOpen {
                plugin: intent.identity.configured_plugin.clone(),
                provider: intent.identity.provider.clone(),
                key: intent.identity.key.clone(),
                invocation: None,
            },
            ReadPurpose::Reload(intent.clone()),
        )?;
        let api::ResultValue::Job(job) = result else {
            unreachable!()
        };
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.retained_payload += prepare_charge;
        let deadline = state.deadlines[&job.job];
        self.plugin_recoveries.insert(
            job.job.clone(),
            Pending {
                owner,
                generation,
                guard: source.guard.clone(),
                requires_choice: source.requires_choice,
                source: Some(source),
                prepared: None,
                intent,
                cancelled: Arc::new(AtomicBool::new(false)),
                worker: false,
                retired: false,
                orphaned: false,
                cleanup_counted: false,
                uncertainty_charge: read_charge == 0,
                accepted_choice: ProviderReloadChoice::ReloadRemote,
                charge: prepare_charge,
                deadline,
            },
        );
        if self
            .application_job_event(owner, "job.changed", job.clone())
            .is_err()
        {
            self.stop_plugin(owner, "resource job consumer is too slow");
        }
        Ok(())
    }

    pub(super) fn prepare_provider_reload(
        &mut self,
        job: &str,
        remote: String,
        generation: String,
        version: String,
        settled: bool,
    ) -> Result<(), Error> {
        let pending = self
            .plugin_recoveries
            .get(job)
            .ok_or_else(|| conflict("Reload was cancelled"))?;
        self.validate_provider_recovery(pending, true)?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::new(Code::Unavailable, "Reload worker unavailable"))?;
        let sender = self
            .plugin_events_sender
            .clone()
            .ok_or_else(|| Error::new(Code::Unavailable, "Reload event channel unavailable"))?;
        let permit = self
            .plugin_local_slots
            .get_or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(16)))
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(Code::Busy, "Local preparation service is busy"))?;
        let pending = self.plugin_recoveries.get_mut(job).unwrap();
        let source = pending
            .source
            .take()
            .ok_or_else(|| conflict("Reload preparation already started"))?;
        pending.worker = true;
        let cancelled = pending.cancelled.clone();
        let owner = pending.owner;
        let job = job.to_owned();
        let permit = Arc::new(permit);
        let worker_permit = permit.clone();
        let result_generation = generation.clone();
        #[cfg(test)]
        let hook = self.plugin_recovery_hook.clone();
        let task = runtime.spawn_blocking(move || {
            let _permit = worker_permit;
            #[cfg(test)]
            if let Some(hook) = hook {
                hook();
            }
            source
                .prepare(remote, generation, version, settled, &cancelled)
                .map_err(|error| conflict(&error.to_string()))
        });
        runtime.spawn(async move {
            let result = task.await.unwrap_or_else(|_| {
                Err(Error::new(Code::Unavailable, "Reload preparation failed"))
            });
            let _ = sender
                .send(plugin::Event {
                    plugin: owner,
                    result: Ok(plugin::ClientMessage::ProviderReload(wire::ReloadEvent {
                        generation: result_generation,
                        job,
                        result,
                        _permit: permit,
                    })),
                })
                .await;
        });
        Ok(())
    }

    fn validate_provider_recovery(&self, pending: &Pending, foreground: bool) -> Result<(), Error> {
        if pending.retired || pending.cancelled.load(Ordering::Acquire) {
            return Err(Error::new(Code::Cancelled, "Remote reload cancelled"));
        }
        if Instant::now() >= pending.deadline {
            return Err(Error::new(Code::Timeout, "Remote reload timed out"));
        }
        if !self
            .app
            .plugins
            .instances
            .get(&pending.owner)
            .is_some_and(|instance| {
                instance.registered
                    && instance.application.generation == pending.generation
                    && instance.config.id == pending.intent.identity.configured_plugin
                    && instance
                        .application
                        .providers
                        .contains_key(&pending.intent.identity.provider)
            })
        {
            return Err(Error::new(
                Code::Unavailable,
                "Resource provider changed during reload",
            ));
        }
        if self.app.host_buffer_is_closed(pending.intent.buffer)
            || !pending
                .guard
                .matches(&self.app.buffers[pending.intent.buffer])
        {
            return Err(conflict("Document changed during reload"));
        }
        if foreground {
            self.app.plugin_foreground(&pending.intent.context)?;
        }
        Ok(())
    }

    pub(super) fn provider_reload_event(&mut self, owner: usize, event: wire::ReloadEvent) {
        let Some(pending) = self.plugin_recoveries.get_mut(&event.job) else {
            return;
        };
        if pending.owner != owner || pending.generation != event.generation {
            return;
        }
        pending.worker = false;
        if pending.retired {
            self.release_provider_recovery(&event.job);
            return;
        }
        let result = self
            .validate_provider_recovery(&self.plugin_recoveries[&event.job], true)
            .and(event.result);
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(error) => {
                self.finish_provider_read(&event.job, Err(error));
                return;
            }
        };
        let pending = &self.plugin_recoveries[&event.job];
        if pending.requires_choice {
            let intent = pending.intent.clone();
            let label = self.app.buffers[intent.buffer]
                .provider()
                .unwrap()
                .label
                .clone();
            self.plugin_recoveries.get_mut(&event.job).unwrap().prepared = Some(prepared);
            if let Err(error) = self
                .app
                .show_provider_reload(event.job.clone(), intent, label)
            {
                self.finish_provider_read(&event.job, Err(conflict(&error.to_string())));
            }
        } else {
            let buffer = pending.intent.buffer;
            let context = pending.intent.context.clone();
            let result = self
                .app
                .install_provider_reload(
                    buffer,
                    prepared,
                    ProviderReloadChoice::ReloadRemote,
                    &context,
                )
                .map(|_| buffer)
                .map_err(|error| conflict(&error.to_string()));
            self.finish_provider_read(&event.job, result);
        }
    }

    pub(super) fn sync_provider_recoveries(&mut self) {
        for intent in self.app.take_provider_reload_intents() {
            let buffer = intent.buffer;
            let action = intent.context.action;
            if let Err(error) = self.start_provider_reload(intent) {
                self.app.plugins.document_saves.remove(&buffer);
                self.app
                    .provider_reload_feedback(action, false, &error.message);
            }
        }
        self.app.sync_provider_reload();
        for decision in self.app.take_provider_reload_decisions() {
            let Some(pending) = self.plugin_recoveries.get(&decision.job) else {
                continue;
            };
            let buffer = pending.intent.buffer;
            let result = self
                .validate_provider_recovery(pending, false)
                .and_then(|_| {
                    let choice = decision
                        .choice
                        .ok_or_else(|| Error::new(Code::Cancelled, "Remote reload cancelled"))?;
                    let context = decision
                        .context
                        .as_ref()
                        .ok_or_else(|| conflict("Reload requires a fresh confirmation"))?;
                    let prepared = self
                        .plugin_recoveries
                        .get_mut(&decision.job)
                        .unwrap()
                        .prepared
                        .take()
                        .ok_or_else(|| conflict("Reload is not ready"))?;
                    self.plugin_recoveries
                        .get_mut(&decision.job)
                        .unwrap()
                        .accepted_choice = choice;
                    self.app
                        .install_provider_reload(buffer, prepared, choice, context)
                        .map(|_| buffer)
                        .map_err(|error| conflict(&error.to_string()))
                });
            self.finish_provider_read(&decision.job, result);
        }
        let stale: Vec<_> = self
            .plugin_recoveries
            .iter()
            .filter(|(_, pending)| !pending.retired)
            .filter_map(|(job, pending)| {
                self.validate_provider_recovery(pending, pending.prepared.is_none())
                    .err()
                    .map(|error| (job.clone(), error))
            })
            .collect();
        for (job, error) in stale {
            self.finish_provider_read(&job, Err(error));
        }
    }

    pub(super) fn retire_provider_recovery(&mut self, job: &str, error: Option<&Error>) {
        let Some(pending) = self.plugin_recoveries.get_mut(job) else {
            return;
        };
        if pending.retired {
            return;
        }
        pending.cancelled.store(true, Ordering::Release);
        pending.retired = true;
        self.app.clear_provider_reload(job);
        let action = pending.intent.context.action;
        match error {
            Some(error) if error.code == Code::Cancelled => {
                self.app
                    .provider_reload_feedback(action, true, "Remote reload cancelled")
            }
            Some(error) => self
                .app
                .provider_reload_feedback(action, false, &error.message),
            None => self.app.provider_reload_feedback(
                action,
                true,
                match pending.accepted_choice {
                    ProviderReloadChoice::ReloadRemote => "Reloaded remote text",
                    ProviderReloadChoice::KeepLocal => {
                        "Kept local edits and accepted the remote baseline"
                    }
                },
            ),
        }
        if pending.worker {
            pending.cleanup_counted = true;
            self.app.plugins.provider_reload_cleanup += 1;
            // Read completion/cancellation may remove its original ledger while
            // preparation still owns those bytes. Transfer before normal release.
            let charge = self.provider_reads.get(job).map_or(0, |read| read.charge);
            pending.charge += charge;
            if pending.orphaned {
                self.app.plugins.orphaned_payload += charge;
            } else if let Some(instance) = self.app.plugins.instances.get_mut(&pending.owner) {
                instance.application.retained_payload += charge;
            }
        } else {
            self.release_provider_recovery(job);
        }
    }
    fn release_provider_recovery(&mut self, job: &str) {
        let Some(pending) = self.plugin_recoveries.remove(job) else {
            return;
        };
        if pending.cleanup_counted {
            self.app.plugins.provider_reload_cleanup -= 1;
        }
        if pending.orphaned {
            self.app.plugins.orphaned_payload -= pending.charge;
        } else if let Some(instance) = self
            .app
            .plugins
            .instances
            .get_mut(&pending.owner)
            .filter(|instance| instance.application.generation == pending.generation)
        {
            instance.application.retained_payload -= pending.charge;
        }
        self.reconcile_provider_payload();
    }
    pub(super) fn stop_provider_recoveries(&mut self, owner: usize) {
        for pending in self
            .plugin_recoveries
            .values_mut()
            .filter(|pending| pending.owner == owner && !pending.orphaned)
        {
            pending.orphaned = true;
            pending.cancelled.store(true, Ordering::Release);
            self.app.plugins.orphaned_payload += pending.charge;
        }
    }
}
