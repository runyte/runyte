// SPDX-License-Identifier: MPL-2.0

//! Recent-history decoration and guarded native cache transactions.
//! Display identity never replaces the live publication's retained proof.

use super::{CatalogEntry, CatalogSnapshot, WorkspaceRow, select_indices, snapshot_locations};
use crate::workspace::{
    catalog_values::{apply_recent_activity, apply_recent_names, assign_running_workspace_numbers},
    recent_history::{RecentEntry, assign_missing_default_workspace_names, read_recents},
    windows_endpoint::CandidateOrigin,
    windows_location::{DiscoveryScope, KnownReadLocation, ResolvedLayout},
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

mod transactions;
pub use transactions::{ensure_recorded, record_activity, remember};

#[derive(Debug)]
pub struct HistoryEntry {
    row: WorkspaceRow,
    live_index: Option<usize>,
}

impl HistoryEntry {
    pub fn row(&self) -> &WorkspaceRow {
        &self.row
    }
}

/// Live authority remains borrowed from this snapshot's exact publication.
/// A stopped history row supplies no process or publication capability.
#[derive(Debug)]
pub enum HistoryTarget<'a> {
    Live {
        row: &'a WorkspaceRow,
        publication: &'a CatalogEntry,
    },
    Stopped {
        row: &'a WorkspaceRow,
    },
}

#[derive(Debug)]
pub struct HistorySnapshot {
    live: CatalogSnapshot,
    entries: Vec<HistoryEntry>,
    remembered: Vec<RecentEntry>,
    history_path: Option<PathBuf>,
}

impl HistorySnapshot {
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }
    pub fn live(&self) -> &CatalogSnapshot {
        &self.live
    }
    pub fn remembered(&self) -> &[RecentEntry] {
        &self.remembered
    }

    pub fn target(&self, index: usize) -> Option<HistoryTarget<'_>> {
        let entry = self.entries.get(index)?;
        Some(match entry.live_index {
            Some(index) => HistoryTarget::Live {
                row: &entry.row,
                publication: &self.live.entries()[index],
            },
            None => HistoryTarget::Stopped { row: &entry.row },
        })
    }

    pub fn select(
        &self,
        selector: &Path,
        working_directory: Option<&Path>,
    ) -> Result<Option<HistoryTarget<'_>>> {
        let rows = self
            .entries
            .iter()
            .map(|entry| &entry.row)
            .collect::<Vec<_>>();
        let matches = select_indices(&rows, selector, working_directory);
        ensure!(
            matches.len() <= 1,
            "workspace selector {} matches multiple native publications or history entries",
            selector.display()
        );
        Ok(matches.first().and_then(|index| self.target(*index)))
    }
}

/// Uses the already selected cache/runtime roots. `configured_state` is an
/// explicit captured configuration value: relative paths resolve separately
/// for each remembered project, while an absolute path is the caller's shared
/// state choice. Neither cache preparation nor a history write occurs here.
/// Filesystem work belongs on the catalog background owner, not the editor loop.
pub async fn snapshot_with_history(
    layout: &ResolvedLayout,
    configured_state: &Path,
    include_hidden: bool,
) -> Result<HistorySnapshot> {
    let current = layout.read_location();
    snapshot_with_history_in_scope(
        layout.discovery_scope(),
        Some(&current),
        configured_state,
        include_hidden,
    )
    .await
}

/// Selector-only discovery passes no current ready location. The remembered
/// paths and history cache still come from this explicitly captured scope.
pub async fn snapshot_with_history_in_scope(
    scope: &DiscoveryScope,
    current: Option<&KnownReadLocation>,
    configured_state: &Path,
    include_hidden: bool,
) -> Result<HistorySnapshot> {
    let history_path = scope
        .cache_root()?
        .map(|cache| cache.join("workspaces.json"));
    let remembered = read_recents(history_path.as_deref())?;
    let mut present = BTreeMap::new();
    let mut known = Vec::with_capacity(remembered.len());
    for entry in &remembered {
        ensure!(
            !present.contains_key(&entry.project_root),
            "recent history contains duplicate project identity"
        );
        let state = crate::project_root::resolve_state_root(&entry.project_root, configured_state);
        known.push(scope.known_read_location(&entry.project_root, &state)?);
        let exists = match entry.project_root.metadata() {
            Ok(metadata) => metadata.is_dir(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).context("remembered project directory is indeterminate");
            }
        };
        present.insert(entry.project_root.clone(), exists);
    }
    // No partial result or write survives a failed exact ready/peer observation.
    let live = snapshot_locations(scope, current, &known, include_hidden, false).await?;
    merge(live, remembered, present, history_path)
}

fn merge(
    live: CatalogSnapshot,
    remembered: Vec<RecentEntry>,
    present: BTreeMap<PathBuf, bool>,
    history_path: Option<PathBuf>,
) -> Result<HistorySnapshot> {
    let mut names = remembered.clone();
    assign_missing_default_workspace_names(&mut names);
    let mut publications = BTreeMap::<&Path, usize>::new();
    for entry in live.entries() {
        *publications.entry(&entry.row().project_root).or_default() += 1;
    }
    let mut entries = Vec::with_capacity(live.entries().len() + remembered.len());
    let mut eligible = Vec::new();
    for (index, entry) in live.entries().iter().enumerate() {
        let unique = publications[entry.row().project_root.as_path()] == 1;
        let scoped = entry.observations().iter().any(|candidate| {
            matches!(
                candidate.origin(),
                CandidateOrigin::ConfiguredNamespace | CandidateOrigin::ConfiguredReady
            )
        });
        let mut row = entry.row().clone();
        // No path-based digit or invented history name can select a foreign
        // namespace or one of several publications sharing that path.
        row.number = None;
        row.last_active_unix_seconds = None;
        if unique && scoped {
            eligible.push(entries.len());
        }
        entries.push(HistoryEntry {
            row,
            live_index: Some(index),
        });
    }
    for entry in &names {
        if publications.contains_key(entry.project_root.as_path()) || !present[&entry.project_root]
        {
            continue;
        }
        ensure!(
            live.absent_projects().contains(&entry.project_root),
            "remembered project lacks a complete exact ready observation"
        );
        eligible.push(entries.len());
        entries.push(HistoryEntry {
            row: stopped(entry),
            live_index: None,
        });
    }
    // Shared Unix presentation/history rules operate only on eligible unique
    // projects. Keep original entry indices as the authority mapping.
    eligible.sort_by_key(|index| {
        let path = &entries[*index].row.project_root;
        (
            remembered
                .iter()
                .position(|entry| &entry.project_root == path)
                .unwrap_or(usize::MAX),
            path.clone(),
        )
    });
    let mut rows = eligible
        .iter()
        .map(|index| entries[*index].row.clone())
        .collect::<Vec<_>>();
    apply_recent_names(&mut rows, &names);
    apply_recent_activity(&mut rows, &names);
    assign_running_workspace_numbers(&mut rows, &names);
    for (index, row) in eligible.into_iter().zip(rows) {
        entries[index].row = row;
    }
    entries.sort_by_cached_key(|entry| {
        let row = &entry.row;
        (
            row.number.is_none(),
            row.number.unwrap_or(0),
            row.last_active_unix_seconds.is_none(),
            row.last_active_unix_seconds.unwrap_or(0),
            row.project_root.clone(),
            entry.live_index,
        )
    });
    Ok(HistorySnapshot {
        live,
        entries,
        remembered,
        history_path,
    })
}

fn stopped(entry: &RecentEntry) -> WorkspaceRow {
    WorkspaceRow {
        unread_terminals: None,
        terminal_bell: None,
        id: crate::workspace::workspace_id(&entry.project_root),
        name: entry.name.clone(),
        number: None,
        last_active_unix_seconds: entry.last_active_unix_seconds,
        project_root: entry.project_root.clone(),
        running: false,
        incompatible_protocol: None,
        unsaved_buffers: None,
        open_buffers: None,
        pending_wait_requests: None,
        plugin_jobs: None,
        activity_leases: None,
        activities: Vec::new(),
        live_terminals: None,
        terminal_sessions: None,
        terminal_line_activity_unix_seconds: None,
        interactive_attached: None,
        git: None,
        missing_directory: false,
    }
}

#[cfg(test)]
mod tests;
