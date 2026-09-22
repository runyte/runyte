// SPDX-License-Identifier: MPL-2.0

//! One authoritative stopped-name record. The synchronous catalog worker must
//! keep this owner when a caller stops observing it; begun mutation and pending
//! recovery cannot be represented by a detached, discarded blocking task.

use super::*;
use crate::workspace::{
    recent_history::{
        RecentEntry, assign_missing_default_workspace_names, read_recents,
        update_recents_if_changed,
    },
    windows_location::DiscoveryScope,
};
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct StoppedNameSelection {
    pub location: EndpointLocation,
    pub store: NameStore,
    pub scope: DiscoveryScope,
    pub configured_state: PathBuf,
    pub history_path: PathBuf,
    pub expected_name: Option<String>,
    pub cached_name: Option<String>,
}

#[derive(Debug)]
pub struct StoppedNameCommit {
    pub name: String,
    /// The authoritative name committed. A regenerable history cache failure
    /// does not turn that success into a failed or rolled-back rename.
    pub cache_error: Option<String>,
}

/// At most one stored-name Change, including its bounded staging/restoration
/// handles, remains owned until retry succeeds or the owner shuts down.
pub struct StoppedNameEdit {
    selection: StoppedNameSelection,
    recovery: Option<Change>,
}

impl StoppedNameEdit {
    pub(crate) fn new(selection: StoppedNameSelection) -> Self {
        Self {
            selection,
            recovery: None,
        }
    }

    pub fn recovery_pending(&self) -> bool {
        self.recovery.is_some()
    }

    pub fn rename(&mut self, name: &str) -> Result<StoppedNameCommit> {
        self.rename_with(name, |_, _| Ok(()), || Ok(()))
    }

    fn rename_with(
        &mut self,
        name: &str,
        mut hook: impl FnMut(Step, &Path) -> io::Result<()>,
        cache_checkpoint: impl FnOnce() -> io::Result<()>,
    ) -> Result<StoppedNameCommit> {
        validate_name(name)?;
        ensure!(
            self.recovery.is_none(),
            "stopped name has unresolved update recovery"
        );
        let location = self.selection.location.clone();
        let store = self.selection.store.clone();
        let id = crate::workspace::workspace_id(&location.project);
        let _identity = location.registries.identity_locks(&id)?;
        let _registry = location.registries.registry_locks()?;
        vacant(&location)?;
        let scan = location.registries.scan_namespaces();
        ensure_scan_available(&scan, &id, name)?;

        // Other stores are read one at a time before holding our selected store
        // lock. Cooperating explicit namespace mutations share registry locks.
        let initial = read_recents(Some(&self.selection.history_path))?;
        let projects = project_set(&initial)?;
        ensure!(
            projects.contains(&location.project),
            "selected workspace was forgotten from history"
        );
        let mut publications = BTreeMap::new();
        for candidate in &scan.candidates {
            let metadata = candidate.metadata();
            if let Some(previous) =
                publications.insert(metadata.project_root_bytes.clone(), metadata)
            {
                ensure!(
                    previous == metadata,
                    "namespace publication copies disagree during name selection"
                );
            }
        }
        let published = publications
            .into_iter()
            .map(|(project, metadata)| (project, metadata.name.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut stored = BTreeMap::new();
        for entry in &initial {
            if entry.project_root == location.project
                || published.contains_key(&encode_path(&entry.project_root))
            {
                continue;
            }
            let state = crate::project_root::resolve_state_root(
                &entry.project_root,
                &self.selection.configured_state,
            );
            self.selection
                .scope
                .known_read_location(&entry.project_root, &state)?;
            stored.insert(
                entry.project_root.clone(),
                NameStore::read_existing(
                    &state,
                    &crate::workspace::workspace_id(&entry.project_root),
                )?,
            );
        }
        let _store = store.lock()?;
        let previous = store.read_unlocked(&id)?;
        ensure!(
            previous.as_ref().map(|(_, _, name)| name) == self.selection.expected_name.as_ref(),
            "stored session name changed since selection"
        );
        let history_path = self.selection.history_path.clone();
        let mut committed = false;
        let mut cache_error = None;
        let result = update_recents_if_changed(&history_path, |entries| {
            ensure!(
                project_set(entries)? == projects,
                "history membership changed during stopped rename"
            );
            let target = entries
                .iter()
                .position(|entry| entry.project_root == location.project)
                .context("selected workspace was forgotten from history")?;
            if self.selection.expected_name.is_none() {
                ensure!(
                    entries[target].name == self.selection.cached_name,
                    "fallback session name changed since selection"
                );
            }
            let mut effective = entries.clone();
            for entry in &mut effective {
                if let Some(name) = published.get(&encode_path(&entry.project_root)) {
                    entry.name = name.clone();
                } else if let Some(Some(name)) = stored.get(&entry.project_root) {
                    entry.name = Some(name.clone());
                }
            }
            if let Some(name) = &self.selection.expected_name {
                effective[target].name = Some(name.clone());
            }
            assign_missing_default_workspace_names(&mut effective);
            ensure!(
                effective
                    .iter()
                    .all(|entry| entry.project_root == location.project
                        || entry.name.as_deref() != Some(name)),
                "session name is already reserved in configured history"
            );
            let leaf = format!("{id}.json");
            self.recovery = Some(Change {
                directory: store.directory.clone(),
                path: store.path.join(&leaf),
                name: leaf,
                old: previous.map(|(file, bytes, _)| Snapshot { file, bytes }),
                bytes: serde_json::to_vec(name).map_err(io::Error::other)?,
                next: None,
                restore: None,
                touched: false,
                publication_index: None,
            });
            if let Err(primary) = self.apply(&mut hook) {
                return match self.rollback(&mut hook) {
                    Ok(()) => Err(primary.into()),
                    Err(rollback) => {
                        let message = format!(
                            "{primary}; stopped-name outcome is unknown; owned recovery remains: {rollback}"
                        );
                        Err(anyhow::Error::new(primary).context(message))
                    }
                };
            }
            self.recovery = None;
            self.selection.expected_name = Some(name.to_owned());
            committed = true;
            if let Err(error) = cache_checkpoint() {
                cache_error = Some(error.to_string());
            } else if entries[target].name == self.selection.cached_name {
                entries[target].name = Some(name.to_owned());
            }
            Ok(())
        });
        if !committed {
            result?;
        } else if let Err(error) = result {
            cache_error = Some(format!("{error:#}"));
        }
        // Reusing this owner never overwrites a newer cache value. A failed or
        // skipped cache update is reconciled by a fresh effective-name snapshot.
        Ok(StoppedNameCommit {
            name: name.to_owned(),
            cache_error,
        })
    }

    fn apply(&mut self, hook: &mut impl FnMut(Step, &Path) -> io::Result<()>) -> io::Result<()> {
        let change = self.recovery.as_mut().expect("owned before staging");
        let (name, file) = change.directory.create_owned_pending()?;
        change.next = Some(Pending { file, name });
        let pending = change.next.as_mut().unwrap();
        pending.file.write_all(&change.bytes)?;
        pending.file.sync_all()?;
        hook(Step::Staged, &change.path)?;
        change.verify_old()?;
        hook(Step::Removing, &change.path)?;
        change.touched = true;
        if let Some(old) = &change.old {
            change
                .directory
                .remove_owned(OsStr::new(&change.name), &old.file)?;
        }
        hook(Step::Removed, &change.path)?;
        let pending = change.next.as_ref().unwrap();
        change
            .directory
            .install_owned_pending(&pending.file, OsStr::new(&change.name))?;
        hook(Step::Installed, &change.path)?;
        change.directory.sync()?;
        hook(Step::Synced, &change.path)?;
        verify_at(
            &change.path,
            &pending.file,
            &change.bytes,
            MAX_STORED_NAME_BYTES,
        )
    }

    fn rollback(&mut self, hook: &mut impl FnMut(Step, &Path) -> io::Result<()>) -> io::Result<()> {
        let change = self.recovery.as_mut().expect("recovery present");
        hook(Step::RollingBack, &change.path)?;
        change.rollback()?;
        hook(Step::Restored, &change.path)?;
        self.recovery = None;
        Ok(())
    }

    /// A newly published host now owns live naming. Recovery never changes its
    /// stored name: reacquire the same ordered locks and prove vacancy again.
    pub fn retry_recovery(&mut self) -> io::Result<()> {
        if self.recovery.is_none() {
            return Ok(());
        }
        let location = self.selection.location.clone();
        let store = self.selection.store.clone();
        let id = crate::workspace::workspace_id(&location.project);
        let _identity = location.registries.identity_locks(&id)?;
        let _registry = location.registries.registry_locks()?;
        vacant(&location)?;
        let _store = store.lock()?;
        self.rollback(&mut |_, _| Ok(()))
    }
}

impl Drop for StoppedNameEdit {
    fn drop(&mut self) {
        // Matches Publication's final best-effort cleanup. Runtime cancellation
        // must retain the worker owner; Drop is not a persistent recovery journal.
        let _ = self.retry_recovery();
    }
}

fn project_set(entries: &[RecentEntry]) -> Result<BTreeSet<PathBuf>> {
    let projects = entries
        .iter()
        .map(|entry| entry.project_root.clone())
        .collect::<BTreeSet<_>>();
    ensure!(
        projects.len() == entries.len(),
        "recent history contains duplicate project identity"
    );
    Ok(projects)
}

fn vacant(location: &EndpointLocation) -> io::Result<()> {
    let scan = location.observe_registrations()?;
    if scan.limit.is_some() || !scan.issues.is_empty() {
        return Err(invalid("stopped-name vacancy is indeterminate"));
    }
    if !scan.candidates.is_empty() || location.observe_ready()?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "workspace publication is occupied",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
