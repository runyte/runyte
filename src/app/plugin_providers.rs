// SPDX-License-Identifier: MPL-2.0

use super::App;

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
    pub continuation: Option<ProviderSaveContinuation>,
}

impl App {
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
        // Protect even clean documents between native command dispatch and host
        // admission. The host releases this guard on refusal or completion.
        self.plugins.document_saves.insert(buffer);
        self.plugins
            .provider_save_intents
            .push_back(ProviderSaveIntent {
                buffer,
                expected_revision: self.buffers[buffer].revision(),
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
