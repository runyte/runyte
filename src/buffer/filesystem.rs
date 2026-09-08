// SPDX-License-Identifier: MPL-2.0
//! Immutable filesystem reconciliation inputs and prepared disk facts.
use super::*;

#[derive(Debug)]
pub(crate) struct FilesystemInput {
    pub index: usize,
    pub path: PathBuf,
    revision: u64,
    generation: u64,
    state: Option<DiskState>,
    directory: bool,
    dirty: bool,
}
#[derive(Debug)]
pub(crate) struct FilesystemUpdate {
    pub input: FilesystemInput,
    pub path: PathBuf,
    pub git_path: Option<PathBuf>,
    state: Option<DiskState>,
    directory: Option<(DirectoryBuffer, String)>,
    pub warning: Option<String>,
    deleted: bool,
}
impl FilesystemInput {
    pub fn prepare(
        self,
        path: PathBuf,
        deleted: bool,
        view: ListingView,
        remaining: &mut usize,
        workspace: &Path,
    ) -> FilesystemUpdate {
        let mut state = None;
        let mut directory = None;
        let mut warning = None;
        if deleted {
            warning = Some(format!("open buffer has stale path {}", path.display()));
        } else if self.directory {
            if self.dirty {
                warning = Some(format!(
                    "affected explorer {} kept unsaved edits; refresh it before saving",
                    path.display()
                ));
            } else {
                let prepared =
                    crate::fs_plan::DirectorySnapshot::read_bounded(&path, view.show_hidden, 1024)
                        .and_then(|snapshot| {
                            DirectoryBuffer::from_snapshot(path.clone(), view, snapshot)
                        });
                match prepared {
                    Ok((value, text))
                        if text.len().saturating_mul(8).saturating_add(1024 * 512)
                            <= *remaining =>
                    {
                        *remaining -= text.len() * 8 + 1024 * 512;
                        directory = Some((value, text));
                    }
                    Ok(_) => {
                        warning = Some(format!(
                            "explorer {} exceeds refresh payload budget; refresh it manually",
                            path.display()
                        ))
                    }
                    Err(error) => {
                        warning = Some(format!(
                            "could not refresh explorer {}: {error}",
                            path.display()
                        ))
                    }
                }
            }
        } else if path != self.path {
            state = DiskState::inspect(&path).ok().flatten().filter(|current| {
                self.state
                    .as_ref()
                    .is_some_and(|expected| current.matches_displaced(expected))
            });
            if self.state.is_some() && state.is_none() {
                warning = Some(format!(
                    "moved file {} needs disk reconciliation before saving",
                    path.display()
                ));
            }
        }
        let git_path = (!self.directory
            && !deleted
            && crate::path_safety::ensure_within_root(workspace, &path).is_ok())
        .then(|| crate::path_safety::path_identity(&path).ok())
        .flatten();
        FilesystemUpdate {
            git_path,
            input: self,
            path,
            state,
            directory,
            warning,
            deleted,
        }
    }
}
impl Buffer {
    pub(crate) fn filesystem_input(&self, index: usize) -> Option<FilesystemInput> {
        if !matches!(self.kind, BufferKind::File | BufferKind::Directory) {
            return None;
        }
        Some(FilesystemInput {
            index,
            path: self.path.clone()?,
            revision: self.revision(),
            generation: self.disk_generation,
            state: self.disk_state.clone(),
            directory: self.is_directory(),
            dirty: self.dirty,
        })
    }
    /// No filesystem access. Text edited during the operation remains authoritative.
    pub(crate) fn install_filesystem_update(&mut self, update: FilesystemUpdate) -> bool {
        if self.path.as_ref() != Some(&update.input.path)
            || self.disk_generation != update.input.generation
        {
            return false;
        }
        let moved = self.path.as_ref() != Some(&update.path);
        if moved {
            self.path = Some(update.path.clone());
            if let Some(directory) = &mut self.directory {
                directory.retarget_root(update.path);
            }
            if let Some(state) = update.state {
                self.disk_state = Some(state);
            }
        }
        self.disk_generation = self.disk_generation.wrapping_add(1);
        if self.is_directory() {
            if self.revision() == update.input.revision
                && !self.dirty
                && let Some((directory, text)) = update.directory
            {
                self.directory = Some(directory);
                self.text = Text::from_str(&text);
                self.undo.clear();
                self.redo.clear();
                self.undo_group = None;
                self.accept_current_as_listing_baseline();
            } else {
                self.external_status = ExternalFileStatus::Changed;
            }
        } else if update.deleted {
            self.external_status = ExternalFileStatus::Deleted;
        } else if update.warning.is_some() {
            self.external_status = ExternalFileStatus::Changed;
        } else {
            self.clear_external_file_state();
        }
        true
    }
}
