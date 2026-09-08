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
        let (events, receiver) = tokio::sync::mpsc::channel(32);
        for (id, config) in configs.into_iter().enumerate() {
            if !plugin::valid_name(&config.id)
                || !names.insert(config.id.clone())
                || !config.executable.is_absolute()
                || config.args.len() > 32
                || config.args.iter().map(String::len).sum::<usize>() > 8192
                || config.bindings.len() > plugin::MAX_COMMANDS
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
                .try_send(HostMessage::Hello {
                    version: plugin::VERSION,
                })
                .expect("new queue");
            self.plugin_workers.insert(id, worker);
            self.app.plugins.instances.insert(
                id,
                Instance {
                    config,
                    sender,
                    registered: false,
                    pending: None,
                    issued: BTreeSet::new(),
                    subscriptions: Default::default(),
                    sequence: 0,
                },
            );
        }
        Some(receiver)
    }

    pub fn handle_plugin_event(&mut self, event: Event) {
        // Observation delivery or a queued stop can retire this same instance.
        // Check membership after that boundary before looking up its state.
        self.sync_plugin_observers();
        if !self.app.plugins.instances.contains_key(&event.plugin) {
            return;
        }
        let result = event
            .result
            .map_err(anyhow::Error::msg)
            .and_then(|message| self.plugin_message(event.plugin, message));
        if let Err(error) = result {
            self.stop_plugin(event.plugin, &error.to_string());
        }
    }

    /// Administrative cancellation also removes commands and subscriptions.
    pub fn stop_plugin(&mut self, id: usize, reason: &str) {
        let Some(instance) = self.app.plugins.instances.remove(&id) else {
            return;
        };
        self.plugin_workers.remove(&id);
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

    fn plugin_send(&mut self, id: usize, message: HostMessage) -> Result<()> {
        self.app
            .plugins
            .instances
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("plugin stopped"))?
            .sender
            .try_send(message)
            .map_err(|_| anyhow::anyhow!("plugin outbound queue full or closed"))
    }

    fn plugin_message(&mut self, id: usize, message: ClientMessage) -> Result<()> {
        if let ClientMessage::Register { version, commands } = message {
            let instance = &self.app.plugins.instances[&id];
            ensure!(!instance.registered, "plugin already registered");
            ensure!(version == plugin::VERSION, "unsupported plugin API version");
            ensure!(
                !commands.is_empty() && commands.len() <= plugin::MAX_COMMANDS,
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
                        id: command_id,
                        plugin: id,
                        name: name.clone(),
                        local: registration.name,
                        description: registration.description,
                        binding,
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
                    id: command_id,
                    plugin: id,
                    name: stop_name,
                    local: "stop".to_owned(),
                    description: "Stop plugin and cancel pending work".to_owned(),
                    binding: None,
                },
            );
            let maps = self.app.plugin_keymaps(&candidate)?;
            self.app.plugins.commands = candidate;
            self.app.install_plugin_keymaps(maps);
            self.app.plugins.instances.get_mut(&id).unwrap().registered = true;
            return self.plugin_send(
                id,
                HostMessage::Registered {
                    commands: full_names,
                },
            );
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
