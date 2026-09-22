// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::workspace::{
    recent_history::unique_default_workspace_name,
    windows_endpoint::{NameStore, StoppedNameEdit, StoppedNameSelection},
};

#[derive(Debug)]
struct NameObservation {
    state: PathBuf,
    stored: Option<String>,
}

#[derive(Debug)]
pub(super) struct StoppedNames {
    scope: DiscoveryScope,
    configured_state: PathBuf,
    observations: BTreeMap<PathBuf, NameObservation>,
}

pub(super) fn observe(
    scope: &DiscoveryScope,
    configured_state: &Path,
    remembered: &[RecentEntry],
    live: &CatalogSnapshot,
) -> Result<StoppedNames> {
    let mut observations = BTreeMap::new();
    for entry in remembered {
        // A hidden live publication never supplies local stored-name authority.
        if live
            .entries()
            .iter()
            .any(|live| live.row().project_root == entry.project_root)
        {
            continue;
        }
        ensure!(
            live.absent_projects().contains(&entry.project_root),
            "stored name requires a complete configured vacancy observation"
        );
        let state = crate::project_root::resolve_state_root(&entry.project_root, configured_state);
        scope.known_read_location(&entry.project_root, &state)?;
        let stored =
            NameStore::read_existing(&state, &crate::workspace::workspace_id(&entry.project_root))?;
        observations.insert(
            entry.project_root.clone(),
            NameObservation { state, stored },
        );
    }
    Ok(StoppedNames {
        scope: scope.clone(),
        configured_state: configured_state.to_owned(),
        observations,
    })
}

fn scoped(entry: &CatalogEntry) -> bool {
    entry.observations().iter().any(|candidate| {
        matches!(
            candidate.origin(),
            CandidateOrigin::ConfiguredNamespace | CandidateOrigin::ConfiguredReady
        )
    })
}

pub(super) fn decorate(names: &mut [RecentEntry], live: &CatalogSnapshot, stored: &StoppedNames) {
    for entry in names.iter_mut() {
        if let Some(observed) = stored.observations.get(&entry.project_root) {
            if let Some(name) = &observed.stored {
                entry.name = Some(name.clone());
            }
        } else if let Some(host) = live
            .entries()
            .iter()
            .find(|host| host.row().project_root == entry.project_root && scoped(host))
        {
            entry.name = host.row().name.clone();
        }
    }
    // Reserve authoritative configured live names even when their history cache
    // is absent/stale. Inventory-only foreign names reserve nothing locally.
    let mut reservations = names.to_vec();
    for host in live.entries().iter().filter(|host| scoped(host)) {
        if host.row().name.is_some() {
            reservations.push(RecentEntry::new(
                host.row().project_root.clone(),
                host.row().name.clone(),
                None,
                None,
            ));
        }
    }
    for entry in names.iter_mut() {
        if entry.name.is_none() && stored.observations.contains_key(&entry.project_root) {
            let name = unique_default_workspace_name(&entry.project_root, &reservations);
            entry.name = Some(name);
            reservations.push(entry.clone());
        }
    }
}

impl HistorySnapshot {
    /// Creates an owned stopped-name operation from this exact configured
    /// selection. This is an explicit mutating action; listing never opens a
    /// writable NameStore. The future worker retains this owner on cancellation.
    pub fn stopped_name_editor(&self, index: usize) -> Result<StoppedNameEdit> {
        let entry = self
            .entries
            .get(index)
            .context("session selection is unavailable")?;
        ensure!(
            !entry.row.running && entry.live_index.is_none(),
            "live names must be changed through their authenticated host"
        );
        let observed = self
            .names
            .observations
            .get(&entry.row.project_root)
            .context("session has no configured stopped-name observation")?;
        let history_path = self
            .history_path
            .clone()
            .context("stopped rename requires configured recent history")?;
        let selected = self
            .remembered
            .iter()
            .find(|old| old.project_root == entry.row.project_root)
            .context("selected workspace was not in recent history")?;
        let layout = ResolvedLayout::from_scope(
            self.names.scope.clone(),
            &entry.row.project_root,
            observed.state.clone(),
        )?;
        ensure!(
            crate::native_path::encode_path(layout.project_root())
                == crate::native_path::encode_path(&entry.row.project_root),
            "selected project identity changed"
        );
        let location = layout.publication_location()?;
        let store = NameStore::open(&observed.state)?;
        Ok(StoppedNameEdit::new(StoppedNameSelection {
            location,
            store,
            scope: self.names.scope.clone(),
            configured_state: self.names.configured_state.clone(),
            history_path,
            expected_name: observed.stored.clone(),
            cached_name: selected.name.clone(),
        }))
    }
}

#[cfg(test)]
mod tests;
