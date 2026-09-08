// SPDX-License-Identifier: MPL-2.0

//! Host-owned approval for one captured best-effort remote overwrite.

use super::App;
use crate::{
    input::{InputEvent, KeyCode, Modifiers},
    plugin::application::CapturedContext,
};

pub(crate) struct ProviderOverwrite {
    job: String,
    buffer: usize,
    expected_revision: u64,
    context: CapturedContext,
    label: String,
    atomic_replace: bool,
}

impl ProviderOverwrite {
    pub(super) fn message(&self) -> String {
        let replacement = if self.atomic_replace {
            "Replacement is atomic, but changes made remotely since the saved version was read can still be overwritten."
        } else {
            "Replacement is not atomic. Changes made remotely since the saved version was read can be overwritten, and a failed write can leave partial content."
        };
        format!(
            "Overwrite {} with the captured Runyte text?\nThis provider cannot guarantee conditional writes.\n{replacement}\nEnter confirms.\nEscape cancels.",
            self.label
        )
    }
}

pub(crate) struct ProviderOverwriteDecision {
    pub job: String,
    /// Present only after a physical, unmodified Enter accepted the surface.
    pub context: Option<CapturedContext>,
}

impl App {
    pub(crate) fn show_provider_overwrite(
        &mut self,
        job: String,
        buffer: usize,
        expected_revision: u64,
        context: CapturedContext,
        label: String,
        atomic_replace: bool,
    ) -> anyhow::Result<()> {
        self.sync_provider_overwrite();
        self.plugin_foreground(&context)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        anyhow::ensure!(
            self.recording_macro.is_none() && self.macro_replay.is_none(),
            "Remote overwrite confirmation requires input outside a macro"
        );
        anyhow::ensure!(
            context.buffer == buffer
                && context.terminal.is_none()
                && buffer < self.buffers.len()
                && !self.host_buffer_is_closed(buffer)
                && self.buffers[buffer].provider().is_some()
                && self.buffers[buffer].revision() == expected_revision,
            "Remote overwrite target changed"
        );
        anyhow::ensure!(
            !self.plugin_has_input_surface() && self.plugins.provider_overwrite_decision.is_none(),
            "An input surface or previous overwrite decision is pending"
        );
        anyhow::ensure!(
            crate::plugin::provider::safe_text(&job, 256)
                && crate::plugin::provider::safe_text(&label, 160),
            "Invalid remote overwrite label"
        );
        self.plugins.provider_overwrite = Some(ProviderOverwrite {
            job,
            buffer,
            expected_revision,
            context,
            label,
            atomic_replace,
        });
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        self.plugins.presentation_dirty = true;
        self.status("Review the remote overwrite before continuing");
        Ok(())
    }

    pub(crate) fn sync_provider_overwrite(&mut self) {
        let Some(surface) = self.plugins.provider_overwrite.take() else {
            return;
        };
        // Taking our own surface out makes the shared input-owner check include
        // every competing native/plugin surface without treating us as a conflict.
        let valid = self.recording_macro.is_none()
            && self.macro_replay.is_none()
            && self.plugin_foreground(&surface.context).is_ok()
            && surface.context.terminal.is_none()
            && surface.buffer < self.buffers.len()
            && !self.host_buffer_is_closed(surface.buffer)
            && self.buffers[surface.buffer].provider().is_some()
            && self.buffers[surface.buffer].revision() == surface.expected_revision
            && !self.has_input_overlay()
            && self.mode != super::Mode::Command;
        if valid {
            self.plugins.provider_overwrite = Some(surface);
        } else {
            self.plugins.provider_overwrite_decision = Some(ProviderOverwriteDecision {
                job: surface.job,
                context: None,
            });
            self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            self.plugins.presentation_dirty = true;
        }
    }

    pub(crate) fn clear_provider_overwrite(&mut self, job: &str) {
        if self
            .plugins
            .provider_overwrite
            .as_ref()
            .is_some_and(|surface| surface.job == job)
        {
            self.plugins.provider_overwrite = None;
            self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            self.plugins.presentation_dirty = true;
        }
        if self
            .plugins
            .provider_overwrite_decision
            .as_ref()
            .is_some_and(|decision| decision.job == job)
        {
            self.plugins.provider_overwrite_decision = None;
        }
    }

    pub(crate) fn take_provider_overwrite_decisions(&mut self) -> Vec<ProviderOverwriteDecision> {
        self.plugins
            .provider_overwrite_decision
            .take()
            .into_iter()
            .collect()
    }

    /// Only the physical frontend input entry point calls this method.
    pub(super) fn handle_provider_overwrite_input(&mut self, input: InputEvent) {
        let Some(mut surface) = self.plugins.provider_overwrite.take() else {
            return;
        };
        let accepted = matches!(&input, InputEvent::Key(key) if key.code == KeyCode::Enter && key.modifiers.is_empty());
        let cancelled = matches!(&input, InputEvent::Key(key) if key.code == KeyCode::Escape || (key.code == KeyCode::Char('c') && key.modifiers == Modifiers::CONTROL) || key.canonical_for_binding() == self.keymap.leader());
        // Input consumed by the surface continues its context. A semantic
        // command bypassing this owner changes the generation and cancels it.
        surface.context.foreground = self.plugins.foreground_generation;
        if accepted || cancelled {
            surface.context.action = self.active_action_id;
            self.plugins.provider_overwrite_decision = Some(ProviderOverwriteDecision {
                job: surface.job,
                context: accepted.then_some(surface.context),
            });
            self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
            self.plugins.presentation_dirty = true;
        } else {
            self.plugins.provider_overwrite = Some(surface);
        }
    }
}
