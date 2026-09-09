// SPDX-License-Identifier: MPL-2.0

use super::{App, ListAction, Mode, PromptKind};
use crate::{
    picker::{ListPicker, PickerItem},
    plugin::{
        self,
        manager::{Action, Entry, Intent, Phase},
    },
};
use anyhow::{Result, ensure};

pub(super) struct ManagerReturn {
    filter: String,
    selected: Option<usize>,
    preview: bool,
}

fn label(entry: &Entry) -> String {
    if plugin::valid_name(&entry.configured_id) {
        entry.configured_id.clone()
    } else {
        entry.config_index.checked_add(1).map_or_else(
            || "Invalid plugin configuration".into(),
            |index| format!("Invalid plugin #{index}"),
        )
    }
}

fn details(entry: &Entry) -> String {
    // Only host-authored categories belong in diagnostic. Never include process
    // arguments, settings, stderr, or plugin-authored error messages here.
    let diagnostic = entry.diagnostic.as_deref().unwrap_or("None");
    let diagnostic: String = diagnostic
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect();
    format!(
        "Plugin: {}\nState: {}\nEnabled: {}\nGeneration: {}\nGrants: {}\nJobs: {}\nActivities: {}\nHelpers: {}\nCleanup: {}\nDiagnostic: {}",
        label(entry),
        entry.phase.label(),
        entry.enabled,
        entry
            .owner
            .map_or_else(|| "None".into(), |id| id.to_string()),
        if entry.granted.is_empty() {
            "None".into()
        } else {
            entry.granted.join(", ")
        },
        entry.jobs,
        entry.activities,
        entry.helpers,
        entry.cleanup,
        diagnostic,
    )
}

impl App {
    pub(crate) fn plugin_manager_refused(&mut self, message: &str) {
        self.action_failed(message);
    }

    pub(crate) fn update_plugin_manager(&mut self, entries: Vec<Entry>) -> bool {
        if self.plugins.manager_entries == entries {
            return false;
        }
        self.plugins.manager_entries = entries;
        if self
            .list
            .as_ref()
            .is_some_and(|list| list.title == "Plugins")
        {
            let saved = self.plugin_manager_position();
            self.rebuild_plugin_manager(saved);
            self.plugins.presentation_dirty = true;
        }
        // Action choices keep their captured generation until explicitly left.
        // A changed owner is refused at activation, never silently retargeted.
        true
    }

    pub(crate) fn take_plugin_manager_intents(&mut self) -> Vec<Intent> {
        self.plugins.manager_intents.drain(..).collect()
    }

    fn plugin_manager_position(&self) -> ManagerReturn {
        ManagerReturn {
            filter: self
                .list
                .as_ref()
                .map_or_else(String::new, |list| list.filter.clone()),
            selected: match self.selected_list_action() {
                Some(ListAction::PluginEntry(index)) => Some(index),
                _ => None,
            },
            preview: self.list.as_ref().is_some_and(|list| list.show_preview),
        }
    }

    fn rebuild_plugin_manager(&mut self, saved: ManagerReturn) {
        let mut items = Vec::new();
        self.list_actions.clear();
        for entry in &self.plugins.manager_entries {
            items.push(
                PickerItem::new(label(entry), entry.phase.label(), items.len())
                    .with_preview(details(entry))
                    .dimmed(!entry.enabled || !entry.valid),
            );
            self.list_actions
                .push(ListAction::PluginEntry(entry.config_index));
        }
        let mut list = ListPicker::new("Plugins", items)
            .with_preview("Plugin")
            .as_manager("actions", "Tab", "actions");
        list.filter = saved.filter;
        list.show_preview = saved.preview;
        let visible = list.visible_indices();
        list.selected = saved.selected.and_then(|selected| visible.iter().position(|index| {
            matches!(self.list_actions.get(list.items[*index].index), Some(ListAction::PluginEntry(id)) if *id == selected)
        })).unwrap_or(0);
        self.list = Some(list);
    }

    pub(super) fn open_plugin_manager(&mut self) {
        self.plugins.manager_return = None;
        self.rebuild_plugin_manager(ManagerReturn {
            filter: String::new(),
            selected: None,
            preview: true,
        });
    }

    pub(super) fn plugin_manager_actions_open(&self) -> bool {
        self.plugins.manager_return.is_some()
            && self
                .list
                .as_ref()
                .is_some_and(|list| list.title == "Plugin actions")
    }

    pub(super) fn return_to_plugin_manager(&mut self) {
        if let Some(saved) = self.plugins.manager_return.take() {
            self.rebuild_plugin_manager(saved);
        }
    }

    pub(super) fn open_plugin_manager_actions(&mut self, index: usize) {
        let Some(entry) = self
            .plugins
            .manager_entries
            .iter()
            .find(|entry| entry.config_index == index)
        else {
            self.action_failed("plugin configuration changed; reopen :plugins");
            return;
        };
        let entry = entry.clone();
        self.plugins.manager_return = Some(self.plugin_manager_position());
        let mut items = Vec::new();
        let mut actions = Vec::new();
        for (action, title, explanation) in [
            (
                Action::Stop,
                "Stop",
                "Stop this generation and cancel a pending restart",
            ),
            (
                Action::Restart,
                "Restart",
                "Wait for cleanup, then start a fresh generation",
            ),
        ] {
            if !entry.valid || (action == Action::Restart && !entry.enabled) {
                continue;
            }
            items.push(
                PickerItem::new(title, explanation, items.len()).with_preview(details(&entry)),
            );
            actions.push(ListAction::PluginLifecycle(Intent {
                config_index: index,
                expected_owner: entry.owner,
                action,
            }));
        }
        items.push(PickerItem::new(
            "Back",
            "Return to configured plugins",
            items.len(),
        ));
        actions.push(ListAction::PluginManagerBack);
        self.list_actions = actions;
        self.list = Some(
            ListPicker::new("Plugin actions", items)
                .with_preview("Plugin")
                .as_choice("choose"),
        );
    }

    pub(super) fn queue_plugin_manager_action(&mut self, intent: Intent) -> Result<()> {
        let entry = self
            .plugins
            .manager_entries
            .iter()
            .find(|entry| entry.config_index == intent.config_index)
            .ok_or_else(|| anyhow::anyhow!("plugin configuration changed; reopen :plugins"))?;
        ensure!(
            entry.owner == intent.expected_owner,
            "plugin generation changed; reopen :plugins"
        );
        ensure!(
            entry.valid,
            "plugin configuration is invalid; correct the configuration first"
        );
        if intent.action == Action::Restart {
            ensure!(
                entry.enabled,
                "plugin is disabled; enable it in the configuration first"
            );
            ensure!(
                !matches!(entry.phase, Phase::Starting | Phase::RestartPending),
                "plugin start is already pending"
            );
        }
        if let Some(existing) = self
            .plugins
            .manager_intents
            .iter_mut()
            .find(|old| old.config_index == intent.config_index)
        {
            *existing = intent;
        } else {
            ensure!(
                self.plugins.manager_intents.len() < plugin::manager::MAX_CONFIGS,
                "plugin action queue is full"
            );
            self.plugins.manager_intents.push_back(intent);
        }
        self.status("Plugin action requested");
        Ok(())
    }

    pub(super) fn plugin_manager_action_by_id(&mut self, id: &str, action: Action) -> Result<()> {
        ensure!(plugin::valid_name(id), "expected a configured plugin ID");
        let mut matches = self
            .plugins
            .manager_entries
            .iter()
            .filter(|entry| entry.configured_id == id);
        let entry = matches
            .next()
            .ok_or_else(|| anyhow::anyhow!("configured plugin ID not found"))?;
        ensure!(
            matches.next().is_none(),
            "plugin ID is ambiguous; correct duplicate configuration entries"
        );
        self.queue_plugin_manager_action(Intent {
            config_index: entry.config_index,
            expected_owner: entry.owner,
            action,
        })
    }

    pub(crate) fn plugin_id_completion_open(&self) -> bool {
        self.mode == Mode::Command
            && self.prompt_kind == PromptKind::Command
            && self.command_cursor == self.command.chars().count()
            && self
                .command
                .split_once(char::is_whitespace)
                .is_some_and(|(name, _)| matches!(name, "plugin-stop" | "plugin-restart"))
    }

    pub(super) fn matching_plugin_hints(&self) -> Option<Vec<&Entry>> {
        if !self.plugin_id_completion_open() {
            return None;
        }
        let (_, prefix) = self.command.split_once(char::is_whitespace)?;
        let prefix = prefix.trim_start();
        Some(
            self.plugins
                .manager_entries
                .iter()
                .filter(|entry| {
                    plugin::valid_name(&entry.configured_id)
                        && entry.configured_id.starts_with(prefix)
                        && self
                            .plugins
                            .manager_entries
                            .iter()
                            .filter(|other| other.configured_id == entry.configured_id)
                            .count()
                            == 1
                })
                .collect(),
        )
    }
}
