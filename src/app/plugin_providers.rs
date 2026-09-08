// SPDX-License-Identifier: MPL-2.0

use super::App;
use crate::plugin::application::CapturedContext;

const SAVE_INTENT_LIMIT: usize = 4;

#[derive(Clone, Copy, Debug)]
pub(crate) enum ProviderSaveClose {
    QuitView,
    CloseBuffer,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProviderSaveContinuation {
    action: ProviderSaveClose,
    pane: usize,
    buffer: usize,
    attachment: u64,
    foreground: u64,
}

pub(crate) struct ProviderSaveIntent {
    pub buffer: usize,
    /// Admission must reject edits made after the native save command.
    pub expected_revision: u64,
    pub context: CapturedContext,
    pub continuation: Option<ProviderSaveContinuation>,
}

#[derive(Clone)]
pub(crate) struct ProviderInspectIntent {
    pub buffer: usize,
    pub expected_revision: u64,
    pub context: CapturedContext,
}

impl App {
    pub(super) fn queue_provider_inspection(&mut self) -> anyhow::Result<()> {
        let buffer = self.active().buffer;
        self.validate_provider_inspection(buffer)?;
        anyhow::ensure!(
            self.active_terminal().is_none(),
            ":diff-remote requires a provider document"
        );
        anyhow::ensure!(
            self.plugins.provider_inspect_intents.len() < 4,
            "Remote inspection queue is full"
        );
        anyhow::ensure!(
            !self
                .plugins
                .provider_inspect_intents
                .iter()
                .any(|intent| intent.buffer == buffer),
            "Remote inspection is already queued"
        );
        let context = CapturedContext {
            foreground_allowed: true,
            action: self.active_action_id,
            pane: self.active_pane,
            buffer,
            terminal: self.active().terminal,
            attachment: self.plugins.attachment_generation,
            foreground: self.plugins.foreground_generation,
        };
        self.plugin_foreground(&context)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        self.plugins
            .provider_inspect_intents
            .push_back(ProviderInspectIntent {
                buffer,
                expected_revision: self.buffers[buffer].revision(),
                context,
            });
        self.status("Reading remote version for comparison");
        Ok(())
    }

    pub(crate) fn take_provider_inspect_intents(
        &mut self,
    ) -> std::collections::VecDeque<ProviderInspectIntent> {
        std::mem::take(&mut self.plugins.provider_inspect_intents)
    }

    pub(crate) fn provider_inspection_snapshot(&self, source: usize) -> Option<usize> {
        self.buffers.iter().enumerate().find_map(|(index, buffer)| {
            (!self.host_buffer_is_closed(index)
                && matches!(
                    buffer.generated_view_identity(),
                    Some(crate::buffer::GeneratedViewIdentity::ProviderSnapshot { source_buffer, .. })
                        if *source_buffer == source
                ))
            .then_some(index)
        })
    }

    fn validate_provider_inspection(&self, source: usize) -> anyhow::Result<()> {
        anyhow::ensure!(
            source < self.buffers.len()
                && !self.host_buffer_is_closed(source)
                && self.buffers[source].provider().is_some(),
            ":diff-remote requires a live provider document"
        );
        anyhow::ensure!(
            self.maximized.is_none(),
            "Leave the maximized view before comparing"
        );
        anyhow::ensure!(
            self.buffers[source].len_bytes() <= crate::diff_view::MAX_DIFF_BYTES,
            "The Runyte buffer is too large to compare"
        );
        let snapshot = self.provider_inspection_snapshot(source);
        anyhow::ensure!(
            self.diffs.iter().all(|diff| {
                if diff.has_buffer(source) || snapshot.is_some_and(|id| diff.has_buffer(id)) {
                    diff.has_buffer(source)
                        && snapshot.is_some_and(|id| diff.has_buffer(id))
                        && [crate::diff::Side::Left, crate::diff::Side::Right]
                            .into_iter()
                            .all(|side| {
                                let side = diff.side(side);
                                self.panes.get(&side.pane).is_some_and(|pane| {
                                    pane.terminal.is_none() && pane.buffer == side.buffer
                                })
                            })
                } else {
                    true
                }
            }),
            "This buffer is already being compared; :diff-off closes it"
        );
        Ok(())
    }

    pub(crate) fn publish_provider_inspection(
        &mut self,
        intent: ProviderInspectIntent,
        generation: String,
        version: String,
        text: String,
    ) -> anyhow::Result<usize> {
        use crate::{
            buffer::{Buffer, GeneratedViewIdentity},
            diff::Side,
            diff_view::{DiffSession, MAX_DIFF_BYTES},
        };
        self.plugin_foreground(&intent.context)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        anyhow::ensure!(
            intent.context.buffer == intent.buffer && intent.context.terminal.is_none(),
            "Inspection target changed"
        );
        self.validate_provider_inspection(intent.buffer)?;
        anyhow::ensure!(
            self.buffers[intent.buffer].revision() == intent.expected_revision,
            "Document changed during remote inspection"
        );
        anyhow::ensure!(
            !self.has_input_overlay() && self.mode != super::Mode::Command,
            "An input surface already owns the frontend"
        );
        anyhow::ensure!(
            text.len() <= MAX_DIFF_BYTES,
            "The remote version is too large to compare"
        );
        let source = intent.buffer;
        let label = self.buffers[source].provider().unwrap().label.clone();
        let snapshot = Buffer::virtual_text_identified(
            GeneratedViewIdentity::ProviderSnapshot {
                source_buffer: source,
                generation,
                version,
            },
            format!("[remote snapshot] {label}"),
            &text,
        );
        let existing = self.provider_inspection_snapshot(source);
        let comparison = self.diffs.iter().position(|diff| diff.has_buffer(source));
        let index = existing.unwrap_or_else(|| {
            let index = self.buffers.len();
            // Install only after all admission checks; a failed layout retires it.
            self.buffers.push(Buffer::scratch());
            self.syntax.push(None);
            index
        });
        let sides = if let Some(comparison) = comparison {
            Some((
                self.diffs[comparison].side(Side::Left),
                self.diffs[comparison].side(Side::Right),
            ))
        } else {
            self.diff_sides(index, source)
        };
        let Some((left, right)) = sides else {
            if existing.is_none() {
                self.close_buffer(index);
            }
            anyhow::bail!("Comparing needs room for two panes");
        };
        self.buffers[index] = snapshot;
        self.retire_syntax(index);
        self.normalize_buffer(index);
        for side in [left, right] {
            if let Some(pane) = self.panes.get_mut(&side.pane) {
                pane.folds.clear();
                pane.preserve_scroll = false;
            }
        }
        let left_text = self.buffers[left.buffer].to_string();
        let right_text = self.buffers[right.buffer].to_string();
        let session = DiffSession::new(left, right, &left_text, &right_text);
        let equal = session.alignment().is_equal();
        if let Some(comparison) = comparison {
            self.diffs[comparison] = session;
        } else {
            self.diffs.push(session);
        }
        self.status(if equal {
            format!("{label} and its remote version are identical")
        } else {
            format!("Comparing the remote version with {label}")
        });
        self.plugins.presentation_dirty = true;
        Ok(index)
    }

    pub(super) fn queue_provider_save(&mut self, buffer: usize, close: Option<ProviderSaveClose>) {
        if self.plugins.filesystem_applying || self.document_mutation_pending(buffer) {
            self.action_warning(
                "Save pending",
                "A captured document revision is still being written",
            );
            return;
        }
        if self.plugins.provider_save_intents.len() >= SAVE_INTENT_LIMIT {
            self.action_warning("Save refused", "Provider save queue is full");
            return;
        }
        if let Err(error) = self.buffers[buffer].prepare_provider_save() {
            self.action_warning("Save refused", error.to_string());
            return;
        }
        let continuation = close.map(|action| ProviderSaveContinuation {
            action,
            pane: self.active_pane,
            buffer,
            attachment: self.plugins.attachment_generation,
            foreground: self.plugins.foreground_generation,
        });
        let context = CapturedContext {
            foreground_allowed: self.recording_macro.is_none() && self.macro_replay.is_none(),
            action: self.active_action_id,
            pane: self.active_pane,
            buffer,
            terminal: self.active().terminal,
            attachment: self.plugins.attachment_generation,
            foreground: self.plugins.foreground_generation,
        };
        // Protect even clean documents between native command dispatch and host
        // admission. The host releases this guard on refusal or completion.
        self.plugins.document_saves.insert(buffer);
        self.plugins
            .provider_save_intents
            .push_back(ProviderSaveIntent {
                buffer,
                expected_revision: self.buffers[buffer].revision(),
                context,
                continuation,
            });
        self.status("Provider save pending");
    }

    pub(crate) fn take_provider_save_intents(
        &mut self,
    ) -> std::collections::VecDeque<ProviderSaveIntent> {
        std::mem::take(&mut self.plugins.provider_save_intents)
    }

    /// Called only after confirmed save acceptance and release of its guard.
    pub(crate) fn finish_provider_save_continuation(
        &mut self,
        continuation: Option<ProviderSaveContinuation>,
    ) {
        let Some(continuation) = continuation else {
            return;
        };
        if self.plugins.attachment_generation != continuation.attachment
            || self.plugins.foreground_generation != continuation.foreground
            || self.active_pane != continuation.pane
            || self.active().buffer != continuation.buffer
            || self.active_terminal().is_some()
            || self.host_buffer_is_closed(continuation.buffer)
            || self.buffers[continuation.buffer].dirty
            || self.document_mutation_pending(continuation.buffer)
        {
            return;
        }
        match continuation.action {
            ProviderSaveClose::QuitView => self.request_view_quit(false),
            ProviderSaveClose::CloseBuffer => {
                if !self.complete_parent_wait(false) {
                    self.close_active_buffer(false);
                }
            }
        }
    }

    pub(crate) fn refresh_provider_save_continuation(
        &self,
        continuation: Option<ProviderSaveContinuation>,
        context: &CapturedContext,
    ) -> Option<ProviderSaveContinuation> {
        let mut continuation = continuation?;
        if self.plugin_foreground(context).is_err()
            || context.terminal.is_some()
            || continuation.pane != context.pane
            || continuation.buffer != context.buffer
            || continuation.attachment != context.attachment
        {
            return None;
        }
        continuation.foreground = context.foreground;
        Some(continuation)
    }
}

impl App {
    pub(crate) fn provider_save_feedback(&mut self, success: bool, message: &str) {
        if success {
            self.status(message);
        } else {
            self.action_warning("Remote save", message);
        }
    }
}
