// SPDX-License-Identifier: MPL-2.0

use super::{App, CommandOutcome, Mode};
use crate::input_grammar::InputGrammar;
use crate::{
    command::{CommandId, CommandInvocation, InvocationParameters},
    keymap::{Binding, BindingTarget, KeySequence, Keymap},
    plugin::{self, HostMessage, PluginConfig, Selection},
};
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub(crate) struct RuntimeCommand {
    pub arguments: Vec<plugin::arguments::Argument>,
    pub id: u64,
    pub plugin: usize,
    pub name: String,
    pub usage: String,
    pub local: String,
    pub description: String,
    pub binding: Option<KeySequence>,
    pub context: plugin::application::CommandContext,
}

pub(crate) struct Pending {
    pub action: Option<u64>,
    pub token: String,
    pub buffer: usize,
    pub revision: u64,
    pub selections: Vec<Selection>,
}

pub(crate) struct Instance {
    pub config: PluginConfig,
    pub sender: plugin::Sender,
    pub registered: bool,
    pub application: plugin::application::Instance,
    pub pending: Option<Pending>,
    pub issued: BTreeSet<usize>,
    pub subscriptions: BTreeMap<usize, (u64, bool)>,
    pub sequence: u64,
}

#[derive(Default)]
pub(crate) struct Plugins {
    pub document_saves: BTreeSet<usize>,
    pub provider_save_intents: VecDeque<super::plugin_providers::ProviderSaveIntent>,
    pub filesystem_accepted: Option<crate::fs_plan::DeletionMode>,
    pub filesystem_applying: bool,
    pub orphaned_payload: usize,
    pub input: Option<super::plugin_interaction::Surface>,
    pub input_finished: Vec<(
        usize,
        plugin::application::CapturedContext,
        plugin::interaction::Submission,
    )>,
    pub filesystem_confirmation: Option<(usize, String, u64, usize)>,
    pub filesystem_finished: Vec<(usize, plugin::filesystem::Finished)>,
    pub commands: BTreeMap<u64, RuntimeCommand>,
    pub instances: BTreeMap<usize, Instance>,
    pub cancellations: BTreeSet<usize>,
    pub next_command: u64,
    pub next_invocation: u64,
    pub frontend_attached: bool,
    pub presentation_dirty: bool,
    pub attachment_generation: u64,
    pub foreground_generation: u64,
    pub presented_views: BTreeMap<usize, (usize, u64)>,
}

impl App {
    pub(crate) fn plugin_keymaps(
        &self,
        commands: &BTreeMap<u64, RuntimeCommand>,
    ) -> Result<[Arc<Keymap>; 2]> {
        let bindings = commands
            .values()
            .filter_map(|command| {
                command.binding.clone().map(|sequence| {
                    let mut binding = Binding::implemented(
                        &[Mode::Normal, Mode::Select],
                        sequence,
                        BindingTarget::Plugin(command.id),
                    );
                    if command.context == plugin::application::CommandContext::View {
                        binding.scope = crate::keymap::BindingScope::Plugin(command.plugin);
                    }
                    binding.description =
                        format!("{} — {}", command.name, command.description).into();
                    binding
                })
            })
            .collect::<Vec<_>>();
        let build = |fast: bool| {
            let base = self
                .configured_keymaps
                .as_ref()
                .map(|maps| Arc::clone(&maps[usize::from(fast)]))
                .unwrap_or_else(|| crate::keymap::keymap_for(fast));
            base.with_plugin_bindings(bindings.clone()).map(Arc::new)
        };
        Ok([build(false)?, build(true)?])
    }

    pub(crate) fn install_plugin_keymaps(&mut self, maps: [Arc<Keymap>; 2]) {
        self.keymap = Arc::clone(&maps[usize::from(self.config.editor.fast_pane_keys)]);
        self.configured_keymaps = Some(maps);
        self.grammar.reset();
    }

    pub fn parse_command(
        &self,
        text: &str,
    ) -> Result<CommandInvocation, crate::command::CommandParseError> {
        let text = text.trim();
        let (name, arguments) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        if let Some(command) = self.plugins.commands.values().find(|c| c.name == name) {
            plugin::arguments::parse(&command.arguments, arguments).map_err(|_| {
                crate::command::CommandParseError::InvalidArgument {
                    command: "plugin command",
                    value: arguments.chars().take(160).collect(),
                    expected: "declared arguments with balanced quotes",
                }
            })?;
            return Ok(CommandInvocation::from_parts(
                CommandId::Plugin(command.id),
                if arguments.is_empty() {
                    InvocationParameters::None
                } else {
                    InvocationParameters::OptionalText(Some(arguments.into()))
                },
                Default::default(),
            )
            .expect("plugin takes no arguments"));
        }
        crate::command::parse_colon_command(text)
    }

    pub(crate) fn invoke_plugin(&mut self, id: u64) -> Result<CommandOutcome> {
        self.invoke_plugin_arguments(id, "")
    }

    pub(crate) fn invoke_plugin_arguments(
        &mut self,
        id: u64,
        arguments: &str,
    ) -> Result<CommandOutcome> {
        if let Some(command) = self.plugins.commands.get(&id)
            && command.local == "stop"
        {
            self.plugins.cancellations.insert(command.plugin);
            self.status("Plugin stop requested");
            return Ok(CommandOutcome::Completed);
        }
        let result = self.submit_plugin(id, arguments);
        match result {
            Ok(token) => {
                self.status(format!("Plugin invocation {token} accepted"));
                Ok(CommandOutcome::AsynchronousRequest(Some(
                    self.status.clone(),
                )))
            }
            Err(error) => {
                self.action_failed(error.to_string());
                Ok(CommandOutcome::UserError(self.status.clone()))
            }
        }
    }

    fn submit_plugin(&mut self, id: u64, arguments: &str) -> Result<String> {
        let command = self
            .plugins
            .commands
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("plugin command is no longer registered"))?
            .clone();
        let arguments = plugin::arguments::parse(&command.arguments, arguments)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        if self.plugins.instances[&command.plugin].config.api == plugin::application::Api::Epoch2 {
            if command.context == plugin::application::CommandContext::Buffer {
                ensure!(
                    self.active_terminal().is_none()
                        && !self.host_buffer_is_closed(self.active().buffer),
                    "command requires a buffer"
                );
            }
            let capture = plugin::application::CapturedContext {
                foreground_allowed: true,
                action: self.active_action_id,
                pane: self.active_pane,
                buffer: self.active().buffer,
                terminal: self.active().terminal,
                attachment: self.plugins.attachment_generation,
                foreground: self.plugins.foreground_generation,
            };
            let view = self.plugins.instances[&command.plugin]
                .application
                .views
                .iter()
                .find(|(_, view)| {
                    view.buffer == self.active().buffer && self.active_terminal().is_none()
                });
            if command.context == plugin::application::CommandContext::View {
                ensure!(view.is_some(), "command requires an owned application view");
            }
            let (view_handle, model_revision, rows) = if let Some((handle, view)) = view {
                if command.context == plugin::application::CommandContext::View {
                    ensure!(
                        self.plugins.presented_views.get(&self.active_pane)
                            == Some(&(view.buffer, view.revision)),
                        "Application view changed; wait for refresh"
                    );
                }
                let mut rows = std::collections::BTreeSet::new();
                for range in self.active().selection.ranges() {
                    for row in self.buffers[view.buffer].offset_to_row(range.from())
                        ..=self.buffers[view.buffer].offset_to_row(
                            range.to().saturating_sub(usize::from(!range.is_empty())),
                        )
                    {
                        if let Some(row) = view.model.rows.get(row) {
                            rows.insert(row.id.clone());
                        }
                    }
                }
                (
                    Some(handle.clone()),
                    Some(format!("m:{}", view.revision)),
                    rows.into_iter().collect(),
                )
            } else {
                (None, None, Vec::new())
            };
            let selection_revision = format!("q:{}", self.plugin_selection_revision(capture.pane));
            let buffer_revision = capture
                .terminal
                .is_none()
                .then(|| format!("r:{}", self.buffers[capture.buffer].revision()));
            let instance = self.plugins.instances.get_mut(&command.plugin).unwrap();
            ensure!(
                instance.application.requests.len() + instance.application.provider_requests
                    < plugin::application::MAX_REQUESTS,
                "application request limit reached"
            );
            let pane = instance
                .application
                .pane_handle(capture.pane)
                .map_err(|error| anyhow::anyhow!(error.message))?;
            let buffer = if capture.terminal.is_none() {
                Some(
                    instance
                        .application
                        .buffer_handle(capture.buffer)
                        .map_err(|error| anyhow::anyhow!(error.message))?,
                )
            } else {
                None
            };
            self.plugins.next_invocation += 1;
            let token = format!("h:{}", self.plugins.next_invocation);
            instance.sender.try_send(HostMessage::Application(
                plugin::application::HostMessage::Request {
                    id: token.clone(),
                    method: "command.invoke",
                    params: plugin::application::Invocation {
                        arguments,
                        command: command.local,
                        context: command.context,
                        pane,
                        selection_revision,
                        buffer,
                        buffer_revision,
                        view: view_handle,
                        model_revision,
                        rows,
                    },
                },
            ))?;
            instance.sender.try_send(HostMessage::Deadline {
                token: token.clone(),
                after_ms: Some(10000),
            })?;
            instance.application.requests.insert(token.clone(), capture);
            return Ok(token);
        }
        ensure!(
            self.active_terminal().is_none(),
            "plugin commands require a document buffer"
        );
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
        ensure!(
            !self.host_buffer_is_closed(buffer_id) && !buffer.is_read_only(),
            "plugin commands require a live editable buffer"
        );
        ensure!(
            buffer.len_bytes() <= plugin::MAX_BYTES / 4,
            "plugin snapshot exceeds 256 KiB"
        );
        ensure!(
            self.active().selection.ranges().len() <= plugin::MAX_SELECTIONS,
            "too many plugin selections"
        );
        let selections = self
            .active()
            .selection
            .ranges()
            .iter()
            .zip(self.operative_spans())
            .map(|(range, (from, to))| Selection {
                anchor: range.anchor,
                head: range.head,
                from,
                to,
            })
            .collect::<Vec<_>>();
        ensure!(
            selections.windows(2).all(|pair| pair[0].to <= pair[1].from),
            "plugin selection spans overlap"
        );
        let revision = buffer.revision();
        let text = buffer.to_string();
        let primary = self.active().selection.primary_index();
        let instance = self
            .plugins
            .instances
            .get_mut(&command.plugin)
            .expect("registered instance");
        ensure!(instance.pending.is_none(), "plugin is busy");
        ensure!(
            instance.issued.contains(&buffer_id) || instance.issued.len() < 64,
            "plugin buffer handle limit reached; restart the host"
        );
        self.plugins.next_invocation += 1;
        let token = self.plugins.next_invocation.to_string();
        let message = HostMessage::Invoke {
            invocation: token.clone(),
            command: command.local,
            buffer: buffer_id.to_string(),
            revision: revision.to_string(),
            text,
            selections: selections.clone(),
            primary,
        };
        ensure!(
            serde_json::to_vec(&message)?.len() < plugin::MAX_BYTES,
            "encoded plugin snapshot exceeds limit"
        );
        instance
            .sender
            .try_send(message)
            .map_err(|_| anyhow::anyhow!("plugin is unavailable or its queue is full"))?;
        instance.issued.insert(buffer_id);
        instance.pending = Some(Pending {
            action: self.active_action_id,
            token: token.clone(),
            buffer: buffer_id,
            revision,
            selections,
        });
        Ok(token)
    }
}

impl App {
    pub(super) fn plugin_command_matches(&self, query: &str) -> Vec<super::CommandMatch<'_>> {
        self.plugins
            .commands
            .values()
            .filter(|c| {
                let haystack = format!("{} {} editing", c.name, c.description).to_lowercase();
                query
                    .split_whitespace()
                    .all(|term| haystack.contains(&term.to_lowercase()))
            })
            .map(|command| super::CommandMatch {
                spec: super::MatchedCommandSpec {
                    id: CommandId::Plugin(command.id),
                    name: &command.name,
                    aliases: &[],
                    usage: &command.usage,
                    description: &command.description,
                    arguments: if command.arguments.is_empty() {
                        crate::command::CommandArguments::None
                    } else {
                        crate::command::CommandArguments::Required(
                            crate::command::ArgumentKind::FreeText,
                        )
                    },
                },
                name: &command.name,
                category: crate::command::CommandCategory::Editing,
                availability: super::CommandAvailability::Available,
            })
            .collect()
    }
}

impl App {
    pub(crate) fn plugin_application_feedback(&mut self, action: Option<u64>, detail: &str) {
        if self.update_action_feedback(action, detail) {
            self.plugins.presentation_dirty = true;
        }
    }

    pub(crate) fn plugin_completion_feedback(
        &mut self,
        action: Option<u64>,
        token: &str,
        code: &str,
        message: &str,
    ) {
        if code == "applied" {
            let detail = format!("Plugin invocation {token} applied");
            self.update_action_feedback(action, &detail);
            self.status(detail);
        } else {
            self.mark_action_feedback_failed(action, message);
            let class = if matches!(code, "stale" | "closed" | "read_only") {
                super::FailureClass::Protective
            } else {
                super::FailureClass::Fault
            };
            self.failure_from(
                class,
                "Plugins",
                "Plugin result rejected",
                format!("Plugin invocation {token}: {message}"),
            );
        }
    }

    pub(crate) fn plugin_stopped_feedback(
        &mut self,
        action: Option<u64>,
        message: String,
        requested: bool,
    ) {
        self.mark_action_feedback_failed(action, &message);
        if requested {
            self.status(&message);
            self.info_from("Plugins", "Plugin stopped", message);
        } else {
            self.error_from("Plugins", "Plugin stopped", message);
        }
    }
}
