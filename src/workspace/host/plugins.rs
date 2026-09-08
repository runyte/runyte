// SPDX-License-Identifier: MPL-2.0

use super::{BufferId, BufferRevision, WorkspaceHost};
use crate::{
    app::plugin_workflows::{Instance, RuntimeCommand},
    keymap::KeySequence,
    plugin::{self, ClientMessage, Event, HostMessage},
    text::{Change, Transaction},
};
use anyhow::{Result, ensure};
use std::collections::BTreeSet;

impl WorkspaceHost {
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
        let configs = self
            .app
            .config
            .plugins
            .iter()
            .filter(|c| c.enabled)
            .cloned()
            .collect::<Vec<_>>();
        if configs.is_empty() {
            return None;
        }
        if configs.len() > plugin::MAX_PLUGINS {
            self.report_host_error("At most 8 plugins may be enabled".to_owned());
            return None;
        }
        let mut names = BTreeSet::new();
        let (events, receiver) = tokio::sync::mpsc::channel(136);
        self.plugin_events_sender = Some(events.clone());
        for (id, config) in configs.into_iter().enumerate() {
            if !plugin::valid_name(&config.id)
                || !names.insert(config.id.clone())
                || !config.executable.is_absolute()
                || config.args.len() > 32
                || config.args.iter().map(String::len).sum::<usize>() > 8192
                || config.bindings.len()
                    > if config.api == plugin::application::Api::Epoch2 {
                        plugin::application::MAX_COMMANDS
                    } else {
                        plugin::MAX_COMMANDS
                    }
                || config.capabilities.len() > 32
                || config
                    .capabilities
                    .iter()
                    .any(|cap| !plugin::valid_name(cap))
            {
                self.report_host_error(format!(
                    "Invalid or duplicate plugin configuration: {}",
                    config.id
                ));
                continue;
            }
            let (worker, sender) = plugin::spawn(
                config.clone(),
                self.app.project_root.clone(),
                id,
                events.clone(),
            );
            sender
                .try_send(if config.api == plugin::application::Api::Epoch2 {
                    HostMessage::Application(plugin::application::HostMessage::Hello {
                        version: plugin::application::VERSION,
                        capabilities: plugin::application::CAPABILITIES.to_vec(),
                        limits: Default::default(),
                    })
                } else {
                    HostMessage::Hello {
                        version: plugin::VERSION,
                    }
                })
                .expect("new queue");
            self.plugin_workers.insert(id, worker);
            self.app.plugins.instances.insert(
                id,
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
        }
        Some(receiver)
    }

    pub fn handle_plugin_event(&mut self, event: Event) -> bool {
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
        if let Ok(ClientMessage::Local {
            generation,
            request,
            ..
        }) = &event.result
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
            self.stop_plugin(event.plugin, &error.to_string());
        }
        self.sync_plugin_views();
        before != presentation(self)
    }

    /// Administrative cancellation also removes commands and subscriptions.
    pub fn stop_plugin(&mut self, id: usize, reason: &str) {
        self.stop_provider_reads(id);
        self.stop_provider_writes(id);
        self.orphan_document_saves(id);
        self.orphan_plugin_filesystem_apply(id);
        self.app.cancel_plugin_input(id, None);
        self.app.cancel_plugin_filesystem(id, None);
        let Some(instance) = self.app.plugins.instances.remove(&id) else {
            return;
        };
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
        self.plugin_workers.remove(&id);
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
        let maps = self
            .app
            .plugin_keymaps(&self.app.plugins.commands)
            .expect("removing plugin bindings preserves validity");
        self.app.install_plugin_keymaps(maps);
        let action = instance.pending.as_ref().and_then(|pending| pending.action);
        let pending = instance
            .pending
            .map(|p| format!("; invocation {} cancelled", p.token))
            .unwrap_or_default();
        self.app.plugin_stopped_feedback(
            action,
            format!("Plugin {} stopped: {reason}{pending}", instance.config.id),
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
            ClientMessage::Local {
                generation,
                request,
                result,
                ..
            } => {
                return self.application_local_result(id, generation, request, result);
            }
            ClientMessage::Queued { message, .. } => return self.plugin_message(id, *message),
            ClientMessage::Application(message) => {
                ensure!(
                    self.app.plugins.instances[&id].config.api == plugin::application::Api::Epoch2,
                    "wrong API epoch"
                );
                return self.application_message(id, message);
            }
            ClientMessage::Unsupported { id: request } => {
                self.application_request_id(id, &request)?;
                return self.application_send(
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
                );
            }
            ClientMessage::Deadline { token } => return self.application_deadline(id, token),
            _ => {}
        }
        if let ClientMessage::Register { version, commands } = message {
            let instance = &self.app.plugins.instances[&id];
            ensure!(!instance.registered, "plugin already registered");
            ensure!(version == plugin::VERSION, "unsupported plugin API version");
            ensure!(
                !commands.is_empty()
                    && commands.len()
                        <= if instance.config.api == plugin::application::Api::Epoch2 {
                            plugin::application::MAX_COMMANDS
                        } else {
                            plugin::MAX_COMMANDS
                        },
                "invalid plugin command count"
            );
            let mut candidate = self.app.plugins.commands.clone();
            let mut names = BTreeSet::new();
            let mut full_names = Vec::new();
            for registration in commands {
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
                    .or_else(|| {
                        instance
                            .application
                            .primary_commands
                            .contains(&registration.name)
                            .then(|| KeySequence::parse("Enter"))
                    })
                    .transpose()
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    binding.as_ref().is_none_or(|b| b.len() <= 8),
                    "plugin binding exceeds 8 keys"
                );
                self.app.plugins.next_command += 1;
                let command_id = self.app.plugins.next_command;
                candidate.insert(
                    command_id,
                    RuntimeCommand {
                        arguments: instance
                            .application
                            .command_arguments
                            .get(&registration.name)
                            .cloned()
                            .unwrap_or_default(),
                        id: command_id,
                        plugin: id,
                        usage: format!(
                            "{}{}",
                            name,
                            instance
                                .application
                                .command_arguments
                                .get(&registration.name)
                                .into_iter()
                                .flatten()
                                .map(|arg| format!(" <{}>", arg.name))
                                .collect::<String>()
                        ),
                        name: name.clone(),
                        local: registration.name.clone(),
                        description: registration.description,
                        binding,
                        context: instance
                            .application
                            .command_contexts
                            .get(&registration.name)
                            .copied()
                            .unwrap_or(plugin::application::CommandContext::Buffer),
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
            self.app.plugins.next_command += 1;
            let command_id = self.app.plugins.next_command;
            let stop_name = format!("plugin.{}.stop", instance.config.id);
            candidate.insert(
                command_id,
                RuntimeCommand {
                    arguments: vec![],
                    id: command_id,
                    plugin: id,
                    usage: stop_name.clone(),
                    name: stop_name,
                    local: "stop".to_owned(),
                    description: "Stop plugin and cancel pending work".to_owned(),
                    binding: None,
                    context: plugin::application::CommandContext::Workspace,
                },
            );
            let maps = self.app.plugin_keymaps(&candidate)?;
            self.app.plugins.commands = candidate;
            self.app.install_plugin_keymaps(maps);
            self.app.plugins.instances.get_mut(&id).unwrap().registered = true;
            let instance = &self.app.plugins.instances[&id];
            let message = if instance.config.api == plugin::application::Api::Epoch2 {
                HostMessage::Application(plugin::application::HostMessage::Registered {
                    commands: full_names,
                    capabilities: instance.application.capabilities.clone(),
                    limits: Default::default(),
                })
            } else {
                HostMessage::Registered {
                    commands: full_names,
                }
            };
            return self.plugin_send(id, message);
        }
        ensure!(
            self.app.plugins.instances[&id].registered,
            "plugin must register first"
        );
        match message {
            ClientMessage::Replace {
                invocation,
                replacements,
            } => {
                let pending = self.take_plugin_pending(id, &invocation)?;
                let result = self.apply_plugin_replacements(&pending, replacements);
                let (status, revision, message) = match result {
                    Ok(revision) => ("applied", Some(revision.to_string()), String::new()),
                    Err((code, message)) => (code, None, message),
                };
                self.app
                    .plugin_completion_feedback(pending.action, &invocation, status, &message);
                self.plugin_send(
                    id,
                    HostMessage::Complete {
                        invocation,
                        status,
                        revision,
                        message,
                    },
                )?;
                self.sync_plugin_observers();
            }
            ClientMessage::Fail {
                invocation,
                message,
            } => {
                ensure!(
                    message.len() <= 1024 && !message.chars().any(char::is_control),
                    "invalid plugin failure message"
                );
                let pending = self.take_plugin_pending(id, &invocation)?;
                self.app.plugin_completion_feedback(
                    pending.action,
                    &invocation,
                    "failed",
                    &message,
                );
                self.plugin_send(
                    id,
                    HostMessage::Complete {
                        invocation,
                        status: "failed",
                        revision: None,
                        message,
                    },
                )?;
            }
            ClientMessage::Subscribe { request, buffer } => {
                ensure!(request.len() <= 64, "request ID exceeds limit");
                let Some(index) = buffer
                    .parse::<usize>()
                    .ok()
                    .filter(|i| self.app.plugins.instances[&id].issued.contains(i))
                else {
                    return self.plugin_send(
                        id,
                        HostMessage::Error {
                            request,
                            code: "unknown_buffer",
                        },
                    );
                };
                if self.app.host_buffer_is_closed(index) {
                    return self.plugin_send(
                        id,
                        HostMessage::Error {
                            request,
                            code: "closed",
                        },
                    );
                }
                let revision = self.app.buffers[index].revision();
                self.app
                    .plugins
                    .instances
                    .get_mut(&id)
                    .unwrap()
                    .subscriptions
                    .insert(index, (revision, false));
                self.plugin_send(
                    id,
                    HostMessage::Subscribed {
                        request,
                        buffer,
                        revision: revision.to_string(),
                    },
                )?;
            }
            ClientMessage::Unsubscribe { request, buffer } => {
                ensure!(
                    request.len() <= 64 && buffer.len() <= 64,
                    "request ID exceeds limit"
                );
                if let Ok(index) = buffer.parse::<usize>() {
                    self.app
                        .plugins
                        .instances
                        .get_mut(&id)
                        .unwrap()
                        .subscriptions
                        .remove(&index);
                }
                self.plugin_send(id, HostMessage::Unsubscribed { request, buffer })?;
            }
            ClientMessage::Register { .. } => unreachable!(),
            ClientMessage::Queued { .. }
            | ClientMessage::Local { .. }
            | ClientMessage::Application(_)
            | ClientMessage::Unsupported { .. }
            | ClientMessage::Deadline { .. }
            | ClientMessage::FilesystemApplied { .. }
            | ClientMessage::DocumentSaved { .. } => anyhow::bail!("wrong API epoch"),
        }
        Ok(())
    }

    fn take_plugin_pending(
        &mut self,
        id: usize,
        token: &str,
    ) -> Result<crate::app::plugin_workflows::Pending> {
        let instance = self.app.plugins.instances.get_mut(&id).unwrap();
        ensure!(
            instance.pending.as_ref().is_some_and(|p| p.token == token),
            "unknown invocation result"
        );
        Ok(instance.pending.take().unwrap())
    }

    fn apply_plugin_replacements(
        &mut self,
        pending: &crate::app::plugin_workflows::Pending,
        replacements: Vec<String>,
    ) -> Result<u64, (&'static str, String)> {
        let fail = |code, message: &str| (code, message.to_owned());
        if self.app.host_buffer_is_closed(pending.buffer) {
            return Err(fail("closed", "invoking buffer is closed"));
        }
        let buffer = &self.app.buffers[pending.buffer];
        if buffer.revision() != pending.revision {
            return Err(fail("stale", "invoking buffer changed"));
        }
        if buffer.is_read_only() {
            return Err(fail("read_only", "invoking buffer is read-only"));
        }
        if replacements.len() != pending.selections.len()
            || replacements.iter().map(String::len).sum::<usize>() > plugin::MAX_BYTES / 2
        {
            return Err(fail(
                "invalid_replacements",
                "replacement count or size is invalid",
            ));
        }
        if pending
            .selections
            .iter()
            .any(|s| s.from > s.to || s.to > buffer.len_chars())
            || pending.selections.windows(2).any(|s| s[0].to > s[1].from)
        {
            return Err(fail(
                "invalid_replacements",
                "replacement ranges are invalid or overlap",
            ));
        }
        let changes = pending
            .selections
            .iter()
            .zip(replacements)
            .map(|(s, text)| Change::new(s.from, s.to, text))
            .collect();
        let transaction = Transaction::new(changes);
        if transaction.is_empty() {
            return Ok(pending.revision);
        }
        self.app.buffers[pending.buffer].commit_undo_group();
        self.apply_expected_transaction(
            BufferId::from_index(pending.buffer),
            BufferRevision::from_raw(pending.revision),
            transaction,
        )
        .map(|r| r.get())
        .map_err(|error| ("refused", error.to_string()))
    }

    /// Observation checkpoints coalesce changes within a host turn, including
    /// undo/reload paths. No scan or wakeup exists when there are no subscribers.
    pub fn sync_plugin_observers(&mut self) {
        self.sync_provider_writes();
        self.sync_provider_inspections();
        self.sync_plugin_inputs();
        self.sync_plugin_filesystem();
        self.sync_plugin_views();
        for id in std::mem::take(&mut self.app.plugins.cancellations) {
            self.stop_plugin(id, "stopped by user");
        }
        let observations = self
            .app
            .plugins
            .instances
            .values()
            .flat_map(|i| i.subscriptions.keys())
            .map(|&index| {
                (
                    index,
                    (
                        self.app.buffers[index].revision(),
                        self.app.host_buffer_is_closed(index),
                    ),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut failed = Vec::new();
        for (&id, instance) in &mut self.app.plugins.instances {
            for (&index, observed) in &mut instance.subscriptions {
                let current = observations[&index];
                if current == *observed {
                    continue;
                }
                *observed = current;
                instance.sequence += 1;
                if instance
                    .sender
                    .try_send(HostMessage::BufferState {
                        sequence: instance.sequence.to_string(),
                        buffer: index.to_string(),
                        revision: current.0.to_string(),
                        closed: current.1,
                    })
                    .is_err()
                {
                    failed.push(id);
                    break;
                }
            }
        }
        for id in failed {
            self.stop_plugin(id, "event consumer is too slow");
        }
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
            let Some(instance) = self.app.plugins.instances.get_mut(&owner) else {
                continue;
            };
            if let Some(view) = instance.application.views.remove(&view) {
                instance.application.retained_payload -= view.model.payload_bytes() * 2;
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
