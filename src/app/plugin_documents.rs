// SPDX-License-Identifier: MPL-2.0
use super::App;
use crate::{buffer::SavedDocument, plugin::application::JobState};
impl App {
    pub(crate) fn plugin_document_path_is_open(&self, path: &std::path::Path) -> bool {
        self.buffers
            .iter()
            .enumerate()
            .any(|(id, buffer)| !self.host_buffer_is_closed(id) && buffer.owns_path_identity(path))
    }
    pub(crate) fn finish_plugin_document_save(
        &mut self,
        buffer: usize,
        cancelled: bool,
        result: Result<Option<SavedDocument>, String>,
    ) -> JobState {
        let warning = result
            .as_ref()
            .ok()
            .and_then(|saved| saved.as_ref())
            .and_then(|saved| saved.warning.clone());
        match result {
            Ok(None) => JobState::Cancelled,
            Ok(Some(saved)) if !cancelled => {
                let accepted = !self.host_buffer_is_closed(buffer)
                    && self.buffers[buffer].accept_document_save(saved);
                if let Some(path) = self.buffers[buffer].path.clone() {
                    self.reconcile_git_after_file_write(&path);
                }
                if !accepted {
                    self.buffers[buffer].mark_write_uncertain();
                }
                if warning.is_some() || !accepted {
                    self.action_warning(
                        "Document write completed",
                        warning.as_deref().unwrap_or("Verify disk state before closing; the saved baseline could not be accepted"),
                    );
                } else if !self.buffers[buffer].dirty {
                    self.lsp_save(buffer);
                }
                JobState::Succeeded
            }
            Ok(Some(_)) | Err(_) if cancelled => {
                self.buffers[buffer].mark_write_uncertain();
                self.action_warning(
                    "Document write outcome unknown",
                    warning.map_or_else(
                        || "The write may have committed; inspect disk and reload before retrying".to_owned(),
                        |warning| format!("The write may have committed; inspect disk and reload before retrying. {warning}"),
                    ),
                );
                JobState::OutcomeUnknown
            }
            Err(error) => {
                self.error_from("Document", "Document save failed", error);
                JobState::Failed
            }
            Ok(Some(_)) => unreachable!(),
        }
    }
}
