// SPDX-License-Identifier: MPL-2.0

//! Native decisions for a host-owned, immutable remote reload candidate.

use super::App;
use crate::{
    buffer::{PreparedProviderReload, ProviderIdentity, ProviderReloadChoice},
    input::{InputEvent, KeyCode, Modifiers},
    plugin::application::CapturedContext,
};
use anyhow::{Result, ensure};

#[derive(Clone)]
pub(crate) struct ProviderReloadIntent {
    pub buffer: usize,
    pub expected_revision: u64,
    pub context: CapturedContext,
    pub identity: ProviderIdentity,
    pub generation: String,
    pub epoch: u64,
}

pub(crate) struct ProviderReload {
    job: String,
    intent: ProviderReloadIntent,
    pub(super) label: String,
    pub(super) selected: usize,
}

pub(crate) struct ProviderReloadDecision {
    pub job: String,
    pub choice: Option<ProviderReloadChoice>,
    /// Only physical, unmodified Enter can provide this fresh context.
    pub context: Option<CapturedContext>,
}

impl App {
    pub(crate) fn provider_reload_feedback(
        &mut self,
        action: Option<u64>,
        success: bool,
        message: &str,
    ) {
        if success {
            self.status(message);
        } else {
            self.action_warning("Remote reload", message);
        }
        let detail = if success {
            message.to_owned()
        } else {
            format!("Remote reload failed: {message}")
        };
        if self.update_action_feedback(action, &detail)
            && let Some(feedback) = self.action_feedback.as_mut()
        {
            feedback.is_error = !success;
        }
        // Terminal outcomes also change status when their initiating action has
        // already left the echo slot. They still need a visible frame.
        self.plugins.presentation_dirty = true;
    }

    pub(super) fn queue_provider_reload(&mut self) -> Result<()> {
        let buffer = self.active().buffer;
        ensure!(
            self.active().terminal.is_none(),
            "Reload requires a provider document"
        );
        ensure!(
            !self.document_mutation_pending(buffer),
            "Document operation is pending"
        );
        ensure!(
            self.plugins.provider_reload_intents.len() < 4,
            "Remote reload queue is full"
        );
        ensure!(
            self.recording_macro.is_none() && self.macro_replay.is_none(),
            "Remote reload requires input outside a macro"
        );
        let document = self.buffers[buffer]
            .provider()
            .ok_or_else(|| anyhow::anyhow!("Reload requires a provider document"))?;
        ensure!(
            self.buffers[buffer].len_bytes() <= 8 * 1024 * 1024,
            "Document exceeds the 8 MiB reload limit"
        );
        let intent = ProviderReloadIntent {
            buffer,
            expected_revision: self.buffers[buffer].revision(),
            context: CapturedContext {
                foreground_allowed: true,
                action: self.active_action_id,
                pane: self.active_pane,
                buffer,
                terminal: None,
                attachment: self.plugins.attachment_generation,
                foreground: self.plugins.foreground_generation,
            },
            identity: document.identity.clone(),
            generation: document.generation.clone(),
            epoch: document.baseline_epoch,
        };
        self.validate_provider_reload_intent(&intent)?;
        // Reserve before returning: a batch cannot queue a save/close/second
        // reload before the host admits this read. Host settles every outcome.
        self.plugins.document_saves.insert(buffer);
        self.plugins.provider_reload_intents.push_back(intent);
        self.status("Reading remote text for reload");
        Ok(())
    }

    pub(crate) fn take_provider_reload_intents(
        &mut self,
    ) -> std::collections::VecDeque<ProviderReloadIntent> {
        std::mem::take(&mut self.plugins.provider_reload_intents)
    }

    pub(crate) fn validate_provider_reload_intent(
        &self,
        intent: &ProviderReloadIntent,
    ) -> Result<()> {
        self.plugin_foreground(&intent.context)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        ensure!(
            intent.context.buffer == intent.buffer && intent.context.terminal.is_none(),
            "Remote reload target changed"
        );
        ensure!(
            self.buffers.get(intent.buffer).is_some() && !self.host_buffer_is_closed(intent.buffer),
            "Remote reload document closed"
        );
        let buffer = &self.buffers[intent.buffer];
        ensure!(
            buffer.revision() == intent.expected_revision
                && buffer.provider().is_some_and(|document| {
                    document.identity == intent.identity
                        && document.generation == intent.generation
                        && document.baseline_epoch == intent.epoch
                }),
            "Document or resource binding changed during reload"
        );
        Ok(())
    }

    pub(crate) fn show_provider_reload(
        &mut self,
        job: String,
        intent: ProviderReloadIntent,
        label: String,
    ) -> Result<()> {
        self.sync_provider_reload();
        self.validate_provider_reload_intent(&intent)?;
        ensure!(
            !self.plugin_has_input_surface() && self.plugins.provider_reload_decision.is_none(),
            "An input surface or previous reload decision is pending"
        );
        ensure!(
            crate::plugin::provider::safe_text(&job, 256)
                && crate::plugin::provider::safe_text(&label, 160),
            "Invalid remote reload label"
        );
        self.plugins.provider_reload = Some(ProviderReload {
            job,
            intent,
            label,
            selected: 2,
        });
        self.provider_reload_presentation_changed();
        self.status("Choose how to adopt the remote text");
        Ok(())
    }

    fn provider_reload_presentation_changed(&mut self) {
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        self.plugins.presentation_dirty = true;
    }

    pub(crate) fn sync_provider_reload(&mut self) {
        let Some(surface) = self.plugins.provider_reload.take() else {
            return;
        };
        if self
            .validate_provider_reload_intent(&surface.intent)
            .is_ok()
            && !self.plugin_has_input_surface()
        {
            self.plugins.provider_reload = Some(surface);
        } else {
            self.plugins.provider_reload_decision = Some(ProviderReloadDecision {
                job: surface.job,
                choice: None,
                context: None,
            });
            self.provider_reload_presentation_changed();
        }
    }

    pub(crate) fn clear_provider_reload(&mut self, job: &str) {
        if self
            .plugins
            .provider_reload
            .as_ref()
            .is_some_and(|surface| surface.job == job)
        {
            self.plugins.provider_reload = None;
            self.provider_reload_presentation_changed();
        }
        if self
            .plugins
            .provider_reload_decision
            .as_ref()
            .is_some_and(|decision| decision.job == job)
        {
            self.plugins.provider_reload_decision = None;
        }
    }

    pub(crate) fn take_provider_reload_decisions(&mut self) -> Vec<ProviderReloadDecision> {
        self.plugins
            .provider_reload_decision
            .take()
            .into_iter()
            .collect()
    }

    pub(super) fn handle_provider_reload_input(&mut self, input: InputEvent) {
        let Some(mut surface) = self.plugins.provider_reload.take() else {
            return;
        };
        let previous_selection = surface.selected;
        let mut finish = None;
        if let InputEvent::Key(key) = input {
            let control = key.modifiers == Modifiers::CONTROL;
            if key.code == KeyCode::Escape
                || (control && key.code == KeyCode::Char('c'))
                || key.canonical_for_binding() == self.keymap.leader()
            {
                finish = Some(None);
            } else if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                finish = Some(match surface.selected {
                    0 => Some(ProviderReloadChoice::ReloadRemote),
                    1 => Some(ProviderReloadChoice::KeepLocal),
                    _ => None,
                });
            } else if (key.modifiers.is_empty() && matches!(key.code, KeyCode::Down | KeyCode::Tab))
                || (control && key.code == KeyCode::Char('n'))
            {
                surface.selected = (surface.selected + 1) % 3;
            } else if matches!(key.code, KeyCode::BackTab)
                || (key.modifiers.is_empty() && key.code == KeyCode::Up)
                || (control && key.code == KeyCode::Char('p'))
            {
                surface.selected = (surface.selected + 2) % 3;
            }
        }
        surface.intent.context.foreground = self.plugins.foreground_generation;
        let changed = finish.is_some() || surface.selected != previous_selection;
        if let Some(choice) = finish {
            self.plugins.provider_reload_decision = Some(ProviderReloadDecision {
                job: surface.job,
                context: choice.as_ref().map(|_| surface.intent.context),
                choice,
            });
        } else {
            self.plugins.provider_reload = Some(surface);
        }
        if changed {
            self.provider_reload_presentation_changed();
        }
    }

    pub(crate) fn install_provider_reload(
        &mut self,
        buffer: usize,
        prepared: PreparedProviderReload,
        choice: ProviderReloadChoice,
        context: &CapturedContext,
    ) -> Result<()> {
        self.plugin_foreground(context)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        ensure!(
            context.buffer == buffer
                && context.terminal.is_none()
                && self.buffers.get(buffer).is_some()
                && !self.host_buffer_is_closed(buffer),
            "Remote reload target changed"
        );
        ensure!(
            !self.plugin_has_input_surface(),
            "An input surface owns the frontend"
        );
        let applied = self.buffers[buffer].accept_provider_reload(prepared, choice)?;
        if let Some(transaction) = applied.transaction {
            // A replacement is already prepared. Schedule whole syntax instead
            // of rebuilding an incremental parse input on the editor loop.
            self.invalidate_partial_guards(buffer);
            self.word_index_notify_update(buffer);
            let deferred = self.defer_syntax;
            self.defer_syntax = true;
            self.reparse_whole(buffer);
            self.defer_syntax = deferred;
            self.map_transaction_views(buffer, std::slice::from_ref(&transaction));
            self.normalize_buffer(buffer);
        }
        self.plugins.presentation_dirty = true;
        self.status(match choice {
            ProviderReloadChoice::ReloadRemote => "Reloaded remote text",
            ProviderReloadChoice::KeepLocal => "Kept local edits and accepted the remote baseline",
        });
        Ok(())
    }
}
