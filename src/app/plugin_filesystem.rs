// SPDX-License-Identifier: MPL-2.0

use super::{App, FsConfirmation, Mode};
use crate::{fs_plan::FsPlan, plugin::filesystem::Finished};

impl App {
    pub(crate) fn plugin_has_input_surface(&self) -> bool {
        self.mode == Mode::Command
            || self.has_input_overlay()
            || self.macro_replay.is_some()
            || self.recording_macro.is_some()
    }

    pub(crate) fn present_plugin_filesystem(&mut self, owner: usize, handle: String, plan: FsPlan) {
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        let buffer = self.active().buffer;
        self.plugins.filesystem_confirmation =
            Some((owner, handle, self.confirmation_revision, buffer));
        self.fs_confirmation = Some(FsConfirmation {
            buffer,
            plan,
            selected: 0,
        });
        self.plugins.presentation_dirty = true;
        self.status("Review application filesystem changes before applying");
    }

    pub(crate) fn cancel_plugin_filesystem(&mut self, owner: usize, handle: Option<&str>) {
        if self
            .plugins
            .filesystem_confirmation
            .as_ref()
            .is_some_and(|(id, plan, _, _)| {
                *id == owner && handle.is_none_or(|handle| handle == plan)
            })
        {
            self.plugins.filesystem_accepted = None;
            let (_, plan, revision, _) = self.plugins.filesystem_confirmation.take().unwrap();
            if self.confirmation_revision == revision {
                self.fs_confirmation = None;
            }
            self.plugins.filesystem_finished.push((
                owner,
                Finished {
                    plan,
                    state: "cancelled",
                    applied: 0,
                    recovery: false,
                },
            ));
            self.plugins.presentation_dirty = true;
        }
    }

    pub(crate) fn sync_plugin_filesystem_confirmation(&mut self) {
        if let Some((owner, _, revision, buffer)) = self.plugins.filesystem_confirmation.as_ref()
            && (self.fs_confirmation.is_none()
                || *revision != self.confirmation_revision
                || !self.plugins.frontend_attached
                || self.host_buffer_is_closed(*buffer))
        {
            self.cancel_plugin_filesystem(*owner, None);
        }
    }
}

impl App {
    pub(crate) fn plugin_filesystem_start_failed(&mut self, message: String) {
        self.action_warning("Filesystem changes pending", message);
    }
    pub(crate) fn document_mutation_pending(&self, buffer: usize) -> bool {
        (self.plugins.filesystem_applying
            && self.buffers.get(buffer).is_some_and(|buffer| {
                matches!(
                    buffer.kind,
                    crate::buffer::BufferKind::File | crate::buffer::BufferKind::Directory
                )
            }))
            || self.plugins.document_saves.contains(&buffer)
    }
    pub(crate) fn plugin_filesystem_confirmation_matches(&self, revision: u64) -> bool {
        revision == self.confirmation_revision && self.fs_confirmation.is_some()
    }
    pub(crate) fn plugin_filesystem_inputs(
        &self,
    ) -> anyhow::Result<Vec<crate::buffer::FilesystemInput>> {
        let inputs = self
            .buffers
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.host_buffer_is_closed(*index))
            .filter_map(|(index, buffer)| buffer.filesystem_input(index))
            .take(129)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            inputs.len() <= 128,
            "Too many open filesystem documents for asynchronous reconciliation"
        );
        Ok(inputs)
    }
    pub(crate) fn plugin_trash_backend(&self) -> std::sync::Arc<dyn crate::fs_plan::TrashBackend> {
        self.ports.trash.clone()
    }
    pub(crate) fn plugin_listing_view(&self) -> crate::directory_buffer::ListingView {
        self.listing_view()
    }

    pub(crate) fn run_plugin_filesystem(
        plan: FsPlan,
        deletion: crate::fs_plan::DeletionMode,
        trash: std::sync::Arc<dyn crate::fs_plan::TrashBackend>,
        inputs: Vec<crate::buffer::FilesystemInput>,
        view: crate::directory_buffer::ListingView,
        phase: std::sync::Arc<std::sync::atomic::AtomicU8>,
        workspace: std::path::PathBuf,
    ) -> crate::plugin::filesystem::Applied {
        use crate::fs_plan::{EntryKind, FsOperation};
        if phase
            .compare_exchange(
                0,
                1,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return crate::plugin::filesystem::Applied {
                result: Ok(Default::default()),
                updates: vec![],
                cancelled_before_start: true,
            };
        }
        let result = plan.apply_bounded(
            deletion,
            trash.as_ref(),
            crate::plugin::filesystem::MAX_ENTRIES,
        );
        let report = match &result {
            Ok(report) => report,
            Err(error) => &error.report,
        };
        let root = plan.root();
        let mut remaining = 4 * 1024 * 1024;
        let mut updates = Vec::new();
        for input in inputs {
            let mapped = super::mapped_applied_path(root, &input.path, &report.applied)
                .unwrap_or_else(|| input.path.clone());
            let deleted = report.applied.iter().any(|operation| match operation {
                FsOperation::Delete { path, kind } => {
                    let path = super::resolved_operation_path(root, path);
                    mapped == path || (*kind == EntryKind::Directory && mapped.starts_with(&path))
                }
                _ => false,
            });
            let affected = mapped != input.path
                || deleted
                || report.applied.iter().any(|operation| {
                    let paths = match operation {
                        FsOperation::Create { path, .. } | FsOperation::Delete { path, .. } => {
                            vec![path]
                        }
                        FsOperation::Rename { from, to, .. }
                        | FsOperation::Move { from, to, .. }
                        | FsOperation::Copy { from, to, .. } => vec![from, to],
                    };
                    paths.into_iter().any(|path| {
                        super::resolved_operation_path(root, path)
                            .parent()
                            .is_some_and(|parent| parent.starts_with(&mapped))
                    })
                });
            if affected {
                updates.push(input.prepare(mapped, deleted, view, &mut remaining, &workspace));
            }
        }
        phase.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::plugin::filesystem::Applied {
            result,
            updates,
            cancelled_before_start: false,
        }
    }

    pub(crate) fn finish_plugin_filesystem_apply(
        &mut self,
        plan: String,
        result: Result<crate::plugin::filesystem::Applied, String>,
    ) -> (Finished, crate::plugin::application::JobState) {
        use crate::plugin::application::JobState;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                for index in 0..self.buffers.len() {
                    if !self.host_buffer_is_closed(index)
                        && matches!(
                            self.buffers[index].kind,
                            crate::buffer::BufferKind::File | crate::buffer::BufferKind::Directory
                        )
                    {
                        self.buffers[index].mark_write_uncertain();
                    }
                }
                self.error_from("Filesystem", "Filesystem outcome unknown", format!("{error}; inspect the workspace before retrying; a mutation may have committed"));
                return (
                    Finished {
                        plan,
                        state: "outcome_unknown",
                        applied: 0,
                        recovery: true,
                    },
                    JobState::OutcomeUnknown,
                );
            }
        };
        if result.cancelled_before_start {
            return (
                Finished {
                    plan,
                    state: "cancelled",
                    applied: 0,
                    recovery: false,
                },
                JobState::Cancelled,
            );
        }
        let mut warnings = Vec::new();
        let mut git_paths = Vec::new();
        for update in result.updates {
            let index = update.input.index;
            if self.host_buffer_is_closed(index) {
                continue;
            }
            if let Some(warning) = &update.warning {
                warnings.push(warning.clone());
            }
            let moved = update.input.path != update.path;
            let previous = update.input.path.clone();
            let git_path = update.git_path.clone();
            let directory = self.buffers[index].is_directory();
            if directory && self.buffers[index].dirty {
                warnings.push(format!(
                    "explorer {} kept unsaved edits; refresh it before saving",
                    update.path.display()
                ));
            }
            if self.buffers[index].install_filesystem_update(update) {
                if moved {
                    self.git.forget(&previous);
                    if !directory {
                        git_paths.extend(git_path);
                    }
                    self.reparse_whole(index);
                    self.lsp_touch(index);
                }
                if directory && !self.buffers[index].dirty {
                    self.forget_directory_view(index);
                    self.forget_directory_jumps(index);
                    self.clear_syntax_history(index);
                    self.retire_syntax(index);
                    self.normalize_buffer(index);
                }
            } else {
                warnings.push(format!(
                    "document {} changed identity; inspect filesystem state",
                    previous.display()
                ));
            }
        }
        self.reconcile_plugin_filesystem_git(git_paths);
        warnings.sort();
        warnings.dedup();
        let warning = warnings.join(" · ");
        match result.result {
            Ok(report) => {
                let count = report.applied.len();
                let recovery = !report.recovery.is_empty();
                let message = format!(
                    "Applied {count} filesystem operations. {} {warning}",
                    report.recovery_summary()
                );
                if recovery || !warning.is_empty() {
                    self.action_warning("Filesystem changes applied", message);
                } else {
                    self.status(message);
                }
                (
                    Finished {
                        plan,
                        state: "succeeded",
                        applied: count,
                        recovery,
                    },
                    JobState::Succeeded,
                )
            }
            Err(error) => {
                let count = error.report.applied.len();
                let recovery = !error.report.recovery.is_empty();
                self.error_from(
                    "Filesystem",
                    "Filesystem plan failed",
                    format!("{error} · {warning}"),
                );
                (
                    Finished {
                        plan,
                        state: "failed",
                        applied: count,
                        recovery,
                    },
                    JobState::Failed,
                )
            }
        }
    }
}
