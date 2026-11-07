// SPDX-License-Identifier: MPL-2.0

//! Captured private context-store provenance for editor buffers and scans.

use std::path::PathBuf;

use super::App;

const PRIVATE_IDENTITY_LIMIT: usize = 512;

#[cfg(any(unix, windows))]
#[derive(Default)]
pub(crate) struct ContextPrivacyInventory {
    depth: usize,
    identities: Option<Option<Vec<crate::buffer::FileIdentity>>>,
}

#[cfg(any(unix, windows))]
pub(crate) struct ContextPrivacyBatch(std::rc::Rc<std::cell::RefCell<ContextPrivacyInventory>>);

#[cfg(any(unix, windows))]
impl Drop for ContextPrivacyBatch {
    fn drop(&mut self) {
        let mut state = self.0.borrow_mut();
        state.depth -= 1;
        if state.depth == 0 {
            state.identities = None;
        }
    }
}

impl App {
    pub(crate) fn mark_context_buffer_private(&self, index: usize) {
        #[cfg(any(unix, windows))]
        {
            self.private_context_buffers.borrow_mut().insert(index);
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = index;
        }
    }
    #[cfg(any(unix, windows))]
    pub(crate) fn context_privacy_batch(&self) -> ContextPrivacyBatch {
        let mut state = self.private_context_inventory.borrow_mut();
        if state.depth == 0 {
            state.identities = None;
        }
        state.depth += 1;
        ContextPrivacyBatch(self.private_context_inventory.clone())
    }
    #[cfg(not(any(unix, windows)))]
    pub(crate) fn context_privacy_batch(&self) {}

    #[cfg(any(unix, windows))]
    fn private_file_identities(
        &self,
        policy: &crate::workspace::context::storage::ContextStorePolicy,
    ) -> Option<Vec<crate::buffer::FileIdentity>> {
        use crate::workspace::context::storage::Storage;
        let cached = {
            let state = self.private_context_inventory.borrow();
            if state.depth > 0 {
                state.identities.clone()
            } else {
                None
            }
        };
        let identities = match cached {
            Some(identities) => identities,
            None => {
                let found = match Storage::open_existing_with_policy(policy.clone()) {
                    Ok(store) => store.file_identities().ok(),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
                    Err(_) => None,
                };
                let mut state = self.private_context_inventory.borrow_mut();
                if state.depth > 0 {
                    state.identities = Some(found.clone());
                }
                found
            }
        };
        identities
    }

    #[cfg(any(unix, windows))]
    fn private_file_identity_matches(
        &self,
        policy: &crate::workspace::context::storage::ContextStorePolicy,
        accepted: Option<&crate::buffer::FileIdentity>,
        current: Option<&crate::buffer::FileIdentity>,
    ) -> bool {
        let Some(identities) = self.private_file_identities(policy) else {
            return true;
        };
        accepted.is_some_and(|identity| identities.contains(identity))
            || current.is_some_and(|identity| identities.contains(identity))
    }

    #[cfg(any(unix, windows))]
    fn record_confirmed_private_file_identities(&self, index: usize) {
        let Some(policy) = self.context_store_policy.as_ref() else {
            return;
        };
        let Some(root) = policy.private_root() else {
            return;
        };
        let Some(buffer) = self.buffers.get(index) else {
            return;
        };
        let opened = buffer.accepted_file_identity();
        let accepted = buffer.accepted_disk_file_identity();
        if opened.is_none() && accepted.is_none() {
            return;
        }
        let opened_path_proof = buffer
            .opened_path_identity()
            .is_some_and(|path| path.starts_with(root));
        let current = buffer.path.as_deref().and_then(|path| {
            crate::path_safety::open_regular_file(path, false)
                .ok()
                .and_then(|file| {
                    file.metadata().ok().and_then(|metadata| {
                        let identity = crate::buffer::file_identity(&file, &metadata).ok()??;
                        #[cfg(unix)]
                        let multiple = {
                            use std::os::unix::fs::MetadataExt;
                            metadata.nlink() > 1
                        };
                        #[cfg(windows)]
                        let multiple = crate::windows_fs::link_count(&file).is_ok_and(|n| n > 1);
                        Some((identity, multiple))
                    })
                })
        });
        let accepted_path_proof = buffer.path.as_deref().is_some_and(|path| {
            crate::path_safety::path_identity(path).is_ok_and(|identity| identity.starts_with(root))
                && current.as_ref().map(|(identity, _)| identity) == accepted
        });
        let needs_inventory = (buffer.was_multiply_linked()
            || buffer.accepted_disk_was_multiply_linked()
            || current.as_ref().is_some_and(|(_, multiple)| *multiple))
            && (!opened_path_proof || !accepted_path_proof);
        let inventory = needs_inventory
            .then(|| self.private_file_identities(policy))
            .flatten();
        let mut confirmed = Vec::new();
        if let Some(identity) = opened
            && (opened_path_proof || inventory.as_ref().is_some_and(|ids| ids.contains(identity)))
        {
            confirmed.push(identity.stable_bytes());
        }
        if let Some(identity) = accepted
            && (accepted_path_proof || inventory.as_ref().is_some_and(|ids| ids.contains(identity)))
        {
            confirmed.push(identity.stable_bytes());
        }
        let mut state = self.private_context_file_identities.borrow_mut();
        let exhausted = if let Some(known) = state.as_mut() {
            for identity in confirmed {
                known.insert(identity);
            }
            known.len() > PRIVATE_IDENTITY_LIMIT
        } else {
            false
        };
        if exhausted {
            *state = None;
        }
    }
    pub(crate) fn note_context_buffer_acceptance(&self, index: usize) {
        #[cfg(any(unix, windows))]
        self.record_confirmed_private_file_identities(index);
        let _ = self.context_buffer_is_private(index);
    }
    pub(crate) fn private_context_root(&self) -> Option<PathBuf> {
        #[cfg(any(unix, windows))]
        {
            self.context_store_policy
                .as_ref()
                .and_then(|policy| policy.private_root().map(|path| path.to_owned()))
        }
        #[cfg(not(any(unix, windows)))]
        {
            None
        }
    }

    pub(crate) fn private_context_buffer_exclusions(&self) -> (Vec<PathBuf>, Vec<[u8; 24]>, bool) {
        if self.private_context_root().is_none() {
            return (Vec::new(), Vec::new(), false);
        }
        let _batch = self.context_privacy_batch();
        for index in 0..self.buffers.len() {
            let _ = self.context_buffer_is_private(index);
        }
        #[cfg(any(unix, windows))]
        {
            let known = self.private_context_file_identities.borrow();
            match known.as_ref() {
                Some(identities) => (Vec::new(), identities.iter().copied().collect(), false),
                None => (Vec::new(), Vec::new(), true),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            (Vec::new(), Vec::new(), false)
        }
    }

    /// A context reader never receives private-store text, even when a file
    /// was opened through an alias and its current path has since changed.
    pub(crate) fn context_buffer_is_private(&self, index: usize) -> bool {
        #[cfg(any(unix, windows))]
        {
            if self.private_context_buffers.borrow().contains(&index) {
                return true;
            }
            let Some(buffer) = self.buffers.get(index) else {
                return false;
            };
            let private = self.context_buffer_value_is_private(buffer);
            if private {
                self.record_confirmed_private_file_identities(index);
                self.mark_context_buffer_private(index);
            }
            private
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = index;
            false
        }
    }

    pub(crate) fn context_buffer_value_is_private(&self, buffer: &crate::buffer::Buffer) -> bool {
        #[cfg(any(unix, windows))]
        {
            let Some(policy) = self.context_store_policy.as_ref() else {
                return false;
            };
            let Some(root) = policy.private_root() else {
                return false;
            };
            let paths = [buffer.path.as_deref(), buffer.media_path.as_deref()];
            let private_path = buffer
                .opened_path_identity()
                .is_some_and(|path| path.starts_with(root))
                || paths.into_iter().flatten().any(|path| {
                    path.starts_with(root)
                        || crate::path_safety::path_identity(path)
                            .is_ok_and(|identity| identity.starts_with(root))
                });
            if private_path {
                return true;
            }
            let candidate = buffer.was_multiply_linked()
                || buffer.path.as_deref().is_some_and(|path| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        std::fs::metadata(path).is_ok_and(|metadata| metadata.nlink() > 1)
                    }
                    #[cfg(windows)]
                    {
                        std::fs::File::open(path)
                            .ok()
                            .and_then(|file| crate::windows_fs::link_count(&file).ok())
                            .is_some_and(|count| count > 1)
                    }
                });
            if !candidate {
                return false;
            }
            // A failed or truncated inventory is ambiguous for this candidate.
            let current = buffer
                .path
                .as_deref()
                .and_then(|path| crate::path_safety::open_regular_file(path, false).ok())
                .and_then(|file| {
                    file.metadata().ok().and_then(|metadata| {
                        crate::buffer::file_identity(&file, &metadata)
                            .ok()
                            .flatten()
                    })
                });
            self.private_file_identity_matches(
                policy,
                buffer.accepted_file_identity(),
                current.as_ref(),
            )
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = buffer;
            false
        }
    }
}
