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
    pub manager_entries: Vec<plugin::manager::Entry>,
    pub manager_intents: VecDeque<plugin::manager::Intent>,
    pub(super) manager_return: Option<super::plugin_manager::ManagerReturn>,
    pub state_orphans: usize,
    pub document_saves: BTreeSet<usize>,
    pub provider_save_intents: VecDeque<super::plugin_providers::ProviderSaveIntent>,
    pub provider_inspect_intents: VecDeque<super::plugin_providers::ProviderInspectIntent>,
    pub provider_reload_cleanup: usize,
    pub provider_reload_intents: VecDeque<super::plugin_recovery::ProviderReloadIntent>,
    pub provider_reload: Option<super::plugin_recovery::ProviderReload>,
    pub provider_reload_decision: Option<super::plugin_recovery::ProviderReloadDecision>,
    pub provider_overwrite: Option<super::plugin_provider_overwrite::ProviderOverwrite>,
    pub provider_overwrite_decision:
        Option<super::plugin_provider_overwrite::ProviderOverwriteDecision>,
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
    pub viewport_watches: BTreeSet<(usize, String, usize)>,
    pub viewport_cache: BTreeMap<(usize, String, usize), plugin::observation::Snapshot>,
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
        if matches!(text.split_whitespace().next(), Some("pipe" | "|")) {
            return crate::command::parse_colon_command(text);
        }
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
            let owner = command.plugin;
            if let Some(entry) = self
                .plugins
                .manager_entries
                .iter()
                .find(|entry| entry.owner == Some(owner))
            {
                self.queue_plugin_manager_action(plugin::manager::Intent {
                    config_index: entry.config_index,
                    expected_owner: Some(owner),
                    action: plugin::manager::Action::Stop,
                })?;
            } else {
                self.plugins.cancellations.insert(owner);
            }
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
        self.plugin_command_preflight(&command)?;
        let arguments = plugin::arguments::parse(&command.arguments, arguments)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        if self.plugins.instances[&command.plugin].config.api == plugin::application::Api::Epoch2 {
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
            let (view_handle, model_revision, query_revision, rows) =
                if let Some((handle, view)) = view {
                    let query_pending = view.query.as_ref().is_some_and(|query| query.pending);
                    let primary = command.context == plugin::application::CommandContext::View
                        && self.plugins.instances[&command.plugin]
                            .application
                            .primary_commands
                            .contains(&command.local);
                    let mut rows = std::collections::BTreeSet::new();
                    for range in self
                        .active()
                        .selection
                        .ranges()
                        .iter()
                        .filter(|_| !query_pending)
                    {
                        for row in self.buffers[view.buffer].offset_to_row(range.from())
                            ..=self.buffers[view.buffer].offset_to_row(
                                range.to().saturating_sub(usize::from(!range.is_empty())),
                            )
                        {
                            if let Some(Some(index)) = view.projection.line_rows.get(row)
                                && let Some(projected) = view.projection.rows.get(*index)
                            {
                                rows.insert(projected.id.clone());
                            }
                        }
                    }
                    if primary {
                        ensure!(
                            !rows.is_empty(),
                            "Select an application row before invoking its primary action"
                        );
                    }
                    (
                        Some(handle.clone()),
                        Some(format!("m:{}", view.revision)),
                        view.query
                            .as_ref()
                            .map(|query| format!("qv:{}", query.revision)),
                        rows.into_iter().collect(),
                    )
                } else {
                    (None, None, None, Vec::new())
                };
            let selection_revision = format!("q:{}", self.plugin_selection_revision(capture.pane));
            let buffer_revision = capture
                .terminal
                .is_none()
                .then(|| format!("r:{}", self.buffers[capture.buffer].revision()));
            let instance = self.plugins.instances.get_mut(&command.plugin).unwrap();
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
            let accepted_action = if command.context == plugin::application::CommandContext::View {
                let handle = view_handle.as_ref().expect("owned view command");
                let action = plugin::observation::Action {
                    id: format!(
                        "a:{}",
                        instance.application.views[handle]
                            .accepted_actions
                            .checked_add(1)
                            .ok_or_else(|| anyhow::anyhow!("View action identity exhausted"))?
                    ),
                    request: token.clone(),
                    command: command.local.clone(),
                    pane: pane.clone(),
                    model_revision: model_revision.clone().expect("owned view model"),
                    query_revision: query_revision.clone(),
                    selection_revision: selection_revision.clone(),
                    selected_count: rows.len(),
                };
                let source = plugin::observation::Source::ViewActions {
                    view: handle.clone(),
                };
                instance
                    .application
                    .observations
                    .preflight_action(&source, &action)
                    .map_err(|error| anyhow::anyhow!(error.message))?;
                Some((source, action))
            } else {
                None
            };
            let callback = HostMessage::Application(plugin::application::HostMessage::Request {
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
                    query_revision,
                    rows,
                },
            });
            ensure!(
                plugin::Sender::message_fits(&callback)?,
                "Application action is too large; select fewer rows"
            );
            if let Err(error) = instance.sender.try_send(callback) {
                self.plugins.cancellations.insert(command.plugin);
                return Err(error);
            }
            if let Err(error) = instance.sender.try_send(HostMessage::Deadline {
                token: token.clone(),
                after_ms: Some(10000),
            }) {
                // The callback may already be visible to the child; retiring its
                // owner prevents an untracked callback from gaining authority.
                self.plugins.cancellations.insert(command.plugin);
                return Err(error);
            }
            instance.application.requests.insert(token.clone(), capture);
            if let Some((source, action)) = accepted_action {
                let plugin::observation::Source::ViewActions { view } = &source else {
                    unreachable!()
                };
                instance
                    .application
                    .views
                    .get_mut(view)
                    .unwrap()
                    .accepted_actions += 1;
                if let Err(error) = instance
                    .application
                    .observations
                    .record_action(&source, action)
                {
                    self.plugins.cancellations.insert(command.plugin);
                    return Err(anyhow::anyhow!(error.message));
                }
            }
            return Ok(token);
        }
        let buffer_id = self.active().buffer;
        let buffer = &self.buffers[buffer_id];
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
    /// Shared discovery/admission checks use metadata only. Snapshot encoding,
    /// selected-row capture and queue admission remain execution-time checks.
    fn plugin_command_preflight(&self, command: &RuntimeCommand) -> Result<()> {
        // Native Stop must remain available while work is busy or cancelling.
        if command.local == "stop" {
            return Ok(());
        }
        let instance = self
            .plugins
            .instances
            .get(&command.plugin)
            .ok_or_else(|| anyhow::anyhow!("plugin command is no longer registered"))?;
        ensure!(
            !self.plugins.cancellations.contains(&command.plugin),
            "Plugin is stopping"
        );
        if instance.config.api == plugin::application::Api::Epoch2 {
            use plugin::application::CommandContext;
            if command.context == CommandContext::Buffer {
                ensure!(
                    self.active_terminal().is_none()
                        && !self.host_buffer_is_closed(self.active().buffer),
                    "command requires a buffer"
                );
            }
            if command.context == CommandContext::View {
                let view = instance
                    .application
                    .views
                    .values()
                    .find(|view| {
                        view.buffer == self.active().buffer && self.active_terminal().is_none()
                    })
                    .ok_or_else(|| anyhow::anyhow!("command requires an owned application view"))?;
                ensure!(
                    view.model.actions.is_empty() || view.model.actions.contains(&command.local),
                    "Application view does not offer this action"
                );
                ensure!(
                    self.plugins.presented_views.get(&self.active_pane)
                        == Some(&(view.buffer, view.revision)),
                    "Application view changed; wait for refresh"
                );
                ensure!(
                    !view.query.as_ref().is_some_and(|query| query.pending)
                        || !instance
                            .application
                            .primary_commands
                            .contains(&command.local),
                    "Application query pending; wait for matching results"
                );
            }
            ensure!(
                instance.application.requests.len() + instance.application.provider_requests
                    < plugin::application::MAX_REQUESTS,
                "application request limit reached"
            );
        } else {
            ensure!(
                self.active_terminal().is_none(),
                "plugin commands require a document buffer"
            );
            let buffer = self.active_buffer();
            ensure!(
                !self.host_buffer_is_closed(self.active().buffer) && !buffer.is_read_only(),
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
            ensure!(instance.pending.is_none(), "plugin is busy");
        }
        Ok(())
    }

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
                availability: match self.plugin_command_preflight(command) {
                    Ok(()) => super::CommandAvailability::Available,
                    Err(error) => super::CommandAvailability::Unavailable(error.to_string()),
                },
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
