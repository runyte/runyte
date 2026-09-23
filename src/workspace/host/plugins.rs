// SPDX-License-Identifier: MPL-2.0

use super::WorkspaceHost;
#[cfg(all(test, unix))]
use crate::app::plugin_workflows::Instance;
use crate::{
    app::plugin_workflows::RuntimeCommand,
    keymap::KeySequence,
    plugin::{self, ClientMessage, Event, HostMessage},
};
use anyhow::{Result, ensure};
use std::collections::BTreeSet;

impl WorkspaceHost {
    /// Prepare the entire registry before publishing any negotiated state.
    pub(super) fn register_plugin_commands(
        &mut self,
        id: usize,
        commands: Vec<plugin::application::Registration>,
        capabilities: BTreeSet<String>,
        features: BTreeSet<String>,
        runyte: String,
    ) -> Result<()> {
        let instance = &self.app.plugins.instances[&id];
        ensure!(!instance.registered, "plugin already registered");
        ensure!(
            !commands.is_empty() && commands.len() <= plugin::MAX_COMMANDS,
            "invalid plugin command count"
        );
        let mut candidate = self.app.plugins.commands.clone();
        let mut names = BTreeSet::new();
        let mut full_names = Vec::new();
        let mut next_command = self.app.plugins.next_command;
        let mut groups = BTreeSet::new();
        for registration in &commands {
            let default_binding = if let Some(binding) = &registration.default_binding {
                ensure!(
                    features.contains(plugin::application::VIEW_DEFAULT_BINDINGS)
                        && registration.context == plugin::application::CommandContext::View,
                    "Default bindings require view-default-bindings and a view command"
                );
                ensure!(binding.len() <= 128, "Default binding is too long");
                let sequence = KeySequence::parse(binding).map_err(anyhow::Error::msg)?;
                ensure!(sequence.len() <= 8, "Default binding exceeds 8 keys");
                Some(sequence)
            } else {
                None
            };
            if let Some(presentation) = &registration.presentation {
                ensure!(
                    features.contains(plugin::application::VIEW_ACTION_PRESENTATION),
                    "Command presentation requires view-action-presentation"
                );
                presentation
                    .validate()
                    .map_err(|error| anyhow::anyhow!(error.message))?;
                if let Some(group) = &presentation.group {
                    groups.insert(group);
                }
                ensure!(groups.len() <= 16, "Too many command presentation groups");
            }
            ensure!(
                registration.alias.as_deref().is_none_or(plugin::valid_name),
                "invalid plugin command alias"
            );
            ensure!(
                plugin::valid_name(&registration.name)
                    && registration.name != "stop"
                    && names.insert(registration.name.clone()),
                "invalid or duplicate plugin command"
            );
            ensure!(
                !registration.description.is_empty()
                    && registration.description.len() <= 160
                    && !registration.description.chars().any(char::is_control),
                "invalid plugin description"
            );
            let name = format!("plugin.{}.{}", instance.config.id, registration.name);
            ensure!(
                !candidate.values().any(|c| c.name == name)
                    && crate::command::resolve_command(&name).is_none(),
                "plugin command collision"
            );
            let binding = instance
                .config
                .bindings
                .get(&registration.name)
                .map(|s| KeySequence::parse(s))
                .or_else(|| default_binding.map(Ok))
                .or_else(|| registration.primary.then(|| KeySequence::parse("Enter")))
                .transpose()
                .map_err(anyhow::Error::msg)?;
            ensure!(
                binding.as_ref().is_none_or(|b| b.len() <= 8),
                "plugin binding exceeds 8 keys"
            );
            next_command = next_command
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Plugin command identities exhausted"))?;
            candidate.insert(
                next_command,
                RuntimeCommand {
                    presentation: registration.presentation.clone(),
                    alias: registration.alias.clone(),
                    alias_name: None,
                    arguments: registration.arguments.clone(),
                    id: next_command,
                    plugin: id,
                    usage: format!(
                        "{}{}",
                        name,
                        registration
                            .arguments
                            .iter()
                            .map(|arg| format!(" <{}>", arg.name))
                            .collect::<String>()
                    ),
                    name: name.clone(),
                    local: registration.name.clone(),
                    description: registration.description.clone(),
                    binding,
                    context: registration.context,
                },
            );
            full_names.push(name);
        }
        ensure!(
            instance
                .config
                .bindings
                .keys()
                .all(|key| names.contains(key)),
            "binding refers to unregistered plugin command"
        );
        next_command = next_command
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Plugin command identities exhausted"))?;
        let stop_name = format!("plugin.{}.stop", instance.config.id);
        candidate.insert(
            next_command,
            RuntimeCommand {
                presentation: None,
                alias: None,
                alias_name: None,
                arguments: vec![],
                id: next_command,
                plugin: id,
                usage: stop_name.clone(),
                name: stop_name,
                local: "stop".into(),
                description: "Stop plugin and cancel pending work".into(),
                binding: None,
                context: plugin::application::CommandContext::Workspace,
            },
        );
        let conflicts = crate::app::plugin_workflows::resolve_aliases(&mut candidate);
        let maps = self.app.plugin_keymaps(&candidate)?;
        let presentation_bytes = candidate
            .values()
            .filter(|command| command.plugin == id)
            .filter_map(|command| command.presentation.as_ref())
            .map(plugin::presentation::Presentation::payload_bytes)
            .sum();
        self.reserve_application_payload(id, presentation_bytes)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        // Admission cannot await, so this ack and the following state commit are
        // one host turn. Queue failure leaves the entire registry unchanged.
        instance.sender.try_send(HostMessage::Application(
            plugin::application::HostMessage::Registered {
                commands: full_names,
                capabilities: capabilities.clone(),
                features: features.clone(),
                runyte: runyte.clone(),
                limits: plugin::application::Limits::for_features(&features),
            },
        ))?;
        let instance = self.app.plugins.instances.get_mut(&id).unwrap();
        instance.application.retained_payload += presentation_bytes;
        instance.application.capabilities = capabilities;
        instance.application.features = features;
        instance.application.runyte = runyte;
        instance.application.command_contexts = commands
            .iter()
            .map(|c| (c.name.clone(), c.context))
            .collect();
        instance.application.command_arguments = commands
            .iter()
            .map(|c| (c.name.clone(), c.arguments.clone()))
            .collect();
        instance.application.primary_commands = commands
            .iter()
            .filter(|c| c.primary)
            .map(|c| c.name.clone())
            .collect();
        instance.registered = true;
        self.app.plugins.next_command = next_command;
        self.app.plugins.commands = candidate;
        self.app.install_plugin_keymaps(maps);
        self.app.plugin_alias_conflicts(conflicts);
        Ok(())
    }

    /// Cancels every worker before waiting, and keeps the runtime alive until
    /// all owned plugin children have been reaped. No new input is dispatched.
    pub async fn shutdown_plugins(&mut self) -> Result<()> {
        self.shutdown_pipe().await;
        let workers = std::mem::take(&mut self.plugin_workers);
        for worker in workers.values() {
            worker.stop();
        }
        let results =
            futures_util::future::join_all(workers.into_values().map(plugin::Worker::wait_stopped))
                .await;
        for result in results {
            result?;
        }
        Ok(())
    }

    pub fn plugin_presentation_pending(&self) -> bool {
        self.app.plugins.presentation_dirty
    }
    pub fn take_plugin_presentation_change(&mut self) -> bool {
        std::mem::take(&mut self.app.plugins.presentation_dirty)
    }

    /// Called once by host service startup; never by an attached frontend.
    pub fn start_plugins(&mut self) -> Option<tokio::sync::mpsc::Receiver<Event>> {
        if self.plugins_started {
            return None;
        }
        self.plugins_started = true;
        // Created whatever the configuration enables. A reload can enable a
        // plugin that startup found disabled, and a channel that only exists
        // when one was enabled at startup would leave that plugin with nothing
        // to deliver its events on. An empty queue costs one allocation and no
        // wakeups.
        let (events, receiver) = tokio::sync::mpsc::channel(plugin::EVENT_CAPACITY);
        self.plugin_events_sender = Some(events);
        self.initialize_plugin_manager();
        Some(receiver)
    }

    pub fn handle_plugin_event(&mut self, event: Event) -> bool {
        let changed = self.handle_plugin_event_inner(event);
        self.sync_plugin_manager();
        changed || self.plugin_presentation_pending()
    }

    fn handle_plugin_event_inner(&mut self, event: Event) -> bool {
        if let Ok(ClientMessage::ProviderReload(reload)) = event.result {
            self.provider_reload_event(event.plugin, reload);
            self.sync_application_observers();
            return self.plugin_presentation_pending();
        }
        if let Ok(ClientMessage::WorkerStopped { failure, reaped }) = event.result {
            self.manager_worker_stopped(event.plugin, failure, reaped);
            return self.plugin_presentation_pending();
        }
        if let Ok(ClientMessage::FilesystemApplied { job, result, .. }) = event.result {
            self.complete_plugin_filesystem_apply(job, result);
            return true;
        }
        if let Ok(ClientMessage::DocumentSaved { job, result, .. }) = event.result {
            self.complete_document_save(job, result);
            return true;
        }
        let presentation = |host: &Self| {
            let visible = host
                .app
                .panes
                .iter()
                .filter(|(_, pane)| pane.terminal.is_none())
                .map(|(&pane_id, pane)| {
                    let buffer = &host.app.buffers[pane.buffer];
                    (
                        pane_id,
                        pane.buffer,
                        buffer.revision(),
                        host.app.plugin_selection_revision(pane_id),
                        buffer.display_name(),
                    )
                })
                .collect::<Vec<_>>();
            (
                host.app.active_pane,
                visible,
                host.app.status.clone(),
                host.app.plugins.commands.len(),
            )
        };
        let before = presentation(self);
        // Observation delivery or a queued stop can retire this same instance.
        // Check membership after that boundary before looking up its state.
        self.sync_plugin_observers();
        if let Ok(ClientMessage::State(state)) = event.result {
            if let Err(error) = self.application_state_event(event.plugin, state)
                && self.app.plugins.instances.contains_key(&event.plugin)
            {
                self.stop_plugin(event.plugin, &error.to_string());
            }
            self.sync_application_observers();
            return before != presentation(self);
        }
        if let Ok(ClientMessage::Handoff(handoff)) = event.result {
            if let Err(error) = self.application_handoff_event(event.plugin, handoff)
                && self.app.plugins.instances.contains_key(&event.plugin)
            {
                self.stop_plugin(event.plugin, &error.to_string());
            }
            self.sync_application_observers();
            return before != presentation(self);
        }
        if let Ok(ClientMessage::Process(process)) = event.result {
            if let Err(error) = self.application_process_event(event.plugin, process)
                && self.app.plugins.instances.contains_key(&event.plugin)
            {
                self.stop_plugin(event.plugin, &error.to_string());
            }
            self.sync_application_observers();
            return before != presentation(self);
        }
        if let Ok(
            ClientMessage::Local {
                generation,
                request,
                ..
            }
            | ClientMessage::ModelPrepared {
                generation,
                request,
                ..
            },
        ) = &event.result
            && let Some(charge) = self.plugin_local_orphans.remove(&(
                event.plugin,
                generation.clone(),
                request.clone(),
            ))
        {
            self.app.plugins.orphaned_payload -= charge;
        }
        if !self.app.plugins.instances.contains_key(&event.plugin) {
            return before != presentation(self);
        }
        let result = event
            .result
            .map_err(anyhow::Error::msg)
            .and_then(|message| self.plugin_message(event.plugin, message));
        if let Err(error) = result {
            let reason = if self
                .app
                .plugins
                .instances
                .get(&event.plugin)
                .is_some_and(|instance| !instance.registered)
            {
                let rejection = error
                    .downcast_ref::<plugin::application::RegistrationFailure>()
                    .cloned()
                    .unwrap_or_else(|| plugin::application::RegistrationFailure {
                        code: "invalid_registration",
                        message:
                            "Invalid plugin registration; check commands, settings and bindings"
                                .into(),
                    });
                if let Some(worker) = self.plugin_workers.get(&event.plugin) {
                    worker.reject(rejection.clone());
                }
                rejection.message
            } else {
                error.to_string()
            };
            self.retain_plugin_failure(event.plugin, &reason);
            self.stop_plugin(event.plugin, &reason);
        }
        // Asynchronous validation can complete input or release a queued retry.
        // Progress both before returning to an otherwise idle frontend.
        self.sync_plugin_inputs();
        self.sync_plugin_views();
        self.sync_application_observers();
        before != presentation(self)
    }

    /// Administrative cancellation also removes commands and subscriptions.
    pub fn stop_plugin(&mut self, id: usize, reason: &str) {
        self.manager_stopping(id, reason == "stopped by user");
        self.stop_plugin_state(id);
        self.stop_plugin_handoffs(id);
        self.stop_plugin_processes(id);
        self.stop_provider_recoveries(id);
        self.stop_provider_reads(id);
        self.stop_provider_writes(id);
        self.orphan_document_saves(id);
        self.orphan_plugin_filesystem_apply(id);
        self.app.cancel_plugin_input(id, None);
        self.app.cancel_plugin_filesystem(id, None);
        let Some(instance) = self.app.plugins.instances.remove(&id) else {
            return;
        };
        self.refresh_plugin_viewport_watches(None);
        for (request, pending) in &instance.application.model_requests {
            pending
                .cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.plugin_local_orphans.insert(
                (id, instance.application.generation.clone(), request.clone()),
                pending.charge + pending.source_charge,
            );
            self.app.plugins.orphaned_payload += pending.charge + pending.source_charge;
        }
        for (request, pending) in &instance.application.local_requests {
            if let Some(cancelled) = &pending.cancelled {
                cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            self.plugin_local_orphans.insert(
                (id, instance.application.generation.clone(), request.clone()),
                pending.charge,
            );
            self.app.plugins.orphaned_payload += pending.charge;
        }
        for issued in instance.application.staging.values() {
            issued
                .cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.app.plugins.presentation_dirty = true;
        for view in instance.application.views.values() {
            if let crate::buffer::BufferKind::Virtual { name, .. } =
                &mut self.app.buffers[view.buffer].kind
            {
                name.push_str(" [unavailable]");
            }
        }
        self.app
            .plugins
            .commands
            .retain(|_, command| command.plugin != id);
        let conflicts =
            crate::app::plugin_workflows::resolve_aliases(&mut self.app.plugins.commands);
        self.app.plugin_alias_conflicts(conflicts);
        let maps = self
            .app
            .plugin_keymaps(&self.app.plugins.commands)
            .expect("removing plugin bindings preserves validity");
        self.app.install_plugin_keymaps(maps);
        self.app.plugin_stopped_feedback(
            None,
            format!("Plugin {} stopped: {reason}", instance.config.id),
            reason == "stopped by user",
        );
    }

    pub(super) fn plugin_send(&mut self, id: usize, message: HostMessage) -> Result<()> {
        let instance = self
            .app
            .plugins
            .instances
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("plugin stopped"))?;
        if let HostMessage::Deadline { token, after_ms } = &message {
            if let Some(ms) = after_ms {
                instance.application.deadlines.insert(
                    token.clone(),
                    std::time::Instant::now() + std::time::Duration::from_millis(*ms),
                );
            } else {
                instance.application.deadlines.remove(token);
            }
        }
        instance
            .sender
            .try_send(message)
            .map_err(|_| anyhow::anyhow!("plugin outbound queue full or closed"))
    }

    pub(super) fn plugin_message(&mut self, id: usize, message: ClientMessage) -> Result<()> {
        match message {
            ClientMessage::ProviderReload(reload) => {
                self.provider_reload_event(id, reload);
                Ok(())
            }
            ClientMessage::WorkerStopped { failure, reaped } => {
                self.manager_worker_stopped(id, failure, reaped);
                Ok(())
            }
            ClientMessage::State(event) => self.application_state_event(id, event),
            ClientMessage::Handoff(event) => self.application_handoff_event(id, event),
            ClientMessage::Process(event) => self.application_process_event(id, event),
            ClientMessage::ModelPrepared {
                generation,
                request,
                result,
                ..
            } => self.application_model_result(id, generation, request, result),
            ClientMessage::Local {
                generation,
                request,
                result,
                ..
            } => self.application_local_result(id, generation, request, result),
            ClientMessage::OutputReady { _notification } => {
                self.sync_application_observers();
                drop(_notification);
                Ok(())
            }
            ClientMessage::Queued { message, .. } => self.plugin_message(id, *message),
            ClientMessage::Application(message) => self.application_message(id, message),
            ClientMessage::Unsupported { id: request } => {
                self.application_request_id(id, &request)?;
                self.application_send(
                    id,
                    plugin::application::HostMessage::Response {
                        id: request,
                        outcome: plugin::application::Response::Failure {
                            error: plugin::application::Error::new(
                                plugin::application::ErrorCode::Unsupported,
                                "Unsupported method",
                            ),
                        },
                    },
                )
            }
            ClientMessage::Deadline { token } => self.application_deadline(id, token),
            ClientMessage::FilesystemApplied { .. } | ClientMessage::DocumentSaved { .. } => {
                anyhow::bail!("unexpected internal plugin event")
            }
        }
    }

    /// Observation checkpoints coalesce changes within a host turn, including
    /// undo/reload paths. No scan or wakeup exists when there are no subscribers.
    pub fn sync_plugin_observers(&mut self) {
        self.sync_pipe();
        self.sync_plugin_handoffs();
        self.sync_provider_writes();
        self.sync_provider_inspections();
        self.sync_provider_recoveries();
        self.sync_plugin_inputs();
        self.sync_plugin_filesystem();
        self.sync_plugin_views();
        for id in std::mem::take(&mut self.app.plugins.cancellations) {
            self.stop_plugin(id, "stopped by user");
        }
        self.sync_application_observers();
        self.sync_plugin_manager();
    }
}

#[cfg(all(test, unix))]
#[path = "tests/plugins.rs"]
mod tests;

impl WorkspaceHost {
    fn sync_plugin_views(&mut self) {
        let snapshots = self
            .app
            .plugins
            .instances
            .iter()
            .flat_map(|(&owner, instance)| {
                instance
                    .application
                    .snapshots
                    .iter()
                    .filter(|(_, snapshot)| self.app.host_buffer_is_closed(snapshot.buffer))
                    .map(move |(handle, _)| (owner, handle.clone()))
            })
            .collect::<Vec<_>>();
        for (owner, token) in snapshots {
            let Some(instance) = self.app.plugins.instances.get_mut(&owner) else {
                continue;
            };
            if let Some(snapshot) = instance.application.snapshots.remove(&token) {
                instance.application.retained_payload -= snapshot.text.len_bytes();
            }
            if self
                .plugin_send(
                    owner,
                    HostMessage::Deadline {
                        token,
                        after_ms: None,
                    },
                )
                .is_err()
            {
                self.stop_plugin(owner, "event consumer is too slow");
            }
        }
        let closed = self
            .app
            .plugins
            .instances
            .iter()
            .flat_map(|(&owner, instance)| {
                instance
                    .application
                    .views
                    .iter()
                    .filter(|(_, view)| self.app.host_buffer_is_closed(view.buffer))
                    .map(move |(handle, _)| (owner, handle.clone()))
            })
            .collect::<Vec<_>>();
        for (owner, view) in closed {
            if !self.app.plugins.instances.contains_key(&owner) {
                continue;
            }
            if self.retire_view_models(owner, &view).is_err() {
                self.stop_plugin(owner, "view lifecycle consumer is too slow");
                continue;
            }
            let Some(instance) = self.app.plugins.instances.get_mut(&owner) else {
                continue;
            };
            if let Some(view) = instance.application.views.remove(&view) {
                instance.application.retained_payload -= view.charge;
            }
            instance.application.sequence += 1;
            let sequence = format!("e:{}", instance.application.sequence);
            if self
                .application_send(
                    owner,
                    plugin::application::HostMessage::Event {
                        sequence,
                        event: "view.closed",
                        data: plugin::application::EventData::ViewClosed { view },
                    },
                )
                .is_err()
            {
                self.stop_plugin(owner, "event consumer is too slow");
            }
        }
    }
}
