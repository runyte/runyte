// SPDX-License-Identifier: MPL-2.0

//! Captured-path history transactions. The caller runs these synchronous,
//! bounded lock/read/write operations on its background owner. No operation
//! acquires a publication/NameStore lock or grants process-control authority.

use super::{HistorySnapshot, RecentEntry, WorkspaceRow};
use crate::workspace::{
    catalog_values::merge_assigned_numbers,
    recent_history::{
        MAX_WORKSPACE_NUMBER, RECENT_LIMIT, RecordedWorkspace, lowest_free_workspace_number,
        unique_default_workspace_name, update_recents_if_changed,
    },
    windows_endpoint::CandidateOrigin,
    windows_location::ResolvedLayout,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Records an explicit visit using the already captured project identity.
/// Only this entry moves to the front; the bounded oldest tail may be evicted.
/// Activity is separate, and no project canonicalization or ambient lookup runs.
pub fn remember(layout: &ResolvedLayout) -> Result<Option<RecordedWorkspace>> {
    record(layout, Visit::Remember)
}

/// Ensures this exact entry exists without reordering an existing record or
/// claiming interactive activity. Other projects' names/numbers are untouched.
pub fn ensure_recorded(layout: &ResolvedLayout) -> Result<Option<RecordedWorkspace>> {
    record(layout, Visit::Ensure)
}

/// Records a successfully attached visit. The supplied captured timestamp can
/// never overwrite a later activity already committed by another attachment.
pub fn record_activity(
    layout: &ResolvedLayout,
    at_unix_seconds: u64,
) -> Result<Option<RecordedWorkspace>> {
    record(layout, Visit::Activity(at_unix_seconds))
}

#[derive(Clone, Copy)]
enum Visit {
    Ensure,
    Remember,
    Activity(u64),
}

fn record(layout: &ResolvedLayout, visit: Visit) -> Result<Option<RecordedWorkspace>> {
    let project = layout.project_root();
    let Some(cache) = layout.cache_root()? else {
        return Ok(None);
    };
    update_recents_if_changed(&cache.join("workspaces.json"), |entries| {
        unique_projects(entries)?;
        let existing = entries
            .iter()
            .position(|entry| entry.project_root == project);
        let mut entry = existing
            .map(|index| entries[index].clone())
            .unwrap_or_else(|| RecentEntry::new(project.to_owned(), None, None, None));
        if entry.name.is_none() {
            entry.name = Some(unique_default_workspace_name(project, entries));
        }
        if entry.number.is_none() && !entry.number_declined {
            entry.number = lowest_free_workspace_number(entries);
        }
        if let Visit::Activity(at) = visit {
            entry.last_active_unix_seconds = Some(
                entry
                    .last_active_unix_seconds
                    .map_or(at, |previous| previous.max(at)),
            );
        }
        let recorded = RecordedWorkspace {
            name: entry.name.clone().expect("assigned name"),
            number: entry.number,
        };
        if matches!(visit, Visit::Ensure)
            && let Some(index) = existing
        {
            entries[index] = entry;
        } else {
            if let Some(index) = existing {
                entries.remove(index);
            }
            entries.insert(0, entry);
            entries.truncate(RECENT_LIMIT);
        }
        Ok(Some(recorded))
    })
}

fn unique_projects(entries: &[RecentEntry]) -> Result<()> {
    let mut paths = BTreeSet::new();
    ensure!(
        entries
            .iter()
            .all(|entry| paths.insert(&entry.project_root)),
        "recent history contains duplicate project identity"
    );
    Ok(())
}

fn unique_numbers(entries: &[RecentEntry]) -> bool {
    let mut numbers = BTreeSet::new();
    entries
        .iter()
        .filter_map(|entry| entry.number)
        .all(|number| numbers.insert(number))
}

fn number_state(entry: &RecentEntry) -> (Option<u8>, bool, bool) {
    (entry.number, entry.number_declined, entry.number_pinned)
}

impl HistorySnapshot {
    fn eligible(&self, index: usize) -> bool {
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        if let Some(live_index) = entry.live_index {
            let publication = &self.live.entries()[live_index];
            self.live
                .entries()
                .iter()
                .filter(|candidate| candidate.row().project_root == entry.row.project_root)
                .count()
                == 1
                && publication.observations().iter().any(|candidate| {
                    matches!(
                        candidate.origin(),
                        CandidateOrigin::ConfiguredNamespace | CandidateOrigin::ConfiguredReady
                    )
                })
        } else {
            !entry.row.running
                && self
                    .live
                    .absent_projects()
                    .contains(&entry.row.project_root)
        }
    }

    fn checked_row(&self, index: usize) -> Result<&WorkspaceRow> {
        ensure!(
            self.eligible(index),
            "history action requires a unique configured observation"
        );
        Ok(&self.entries[index].row)
    }

    fn unchanged_index(&self, entries: &[RecentEntry], project: &Path) -> Result<usize> {
        let observed = self
            .remembered
            .iter()
            .find(|entry| entry.project_root == project)
            .context("workspace was not in the observed recent history")?;
        let index = entries
            .iter()
            .position(|entry| entry.project_root == project)
            .context("workspace history changed since observation")?;
        ensure!(
            &entries[index] == observed,
            "workspace history changed since observation"
        );
        Ok(index)
    }

    /// Persists decoration from this complete observation only. New/removed or
    /// concurrently renamed/renumbered entries win; activity and ordering never
    /// change. A conflicting number batch is left for a fresh observation.
    /// Missing-directory stopped records are not automatically pruned.
    /// A successful write does not rebase this snapshot. Obtain a fresh complete
    /// snapshot before subsequent explicit number/forget actions; those guards
    /// intentionally reject the earlier remembered values after persistence.
    pub fn persist(&self) -> Result<usize> {
        let rows = self
            .entries
            .iter()
            .enumerate()
            .filter(|(index, _)| self.eligible(*index))
            .map(|(_, entry)| entry.row.clone())
            .collect::<Vec<_>>();
        let Some(path) = self.history_path.as_deref().filter(|_| !rows.is_empty()) else {
            return Ok(0);
        };
        update_recents_if_changed(path, |entries| {
            unique_projects(entries)?;
            let before = entries.clone();
            // A newer visit can mean an observed stopped session restarted,
            // even if it retained the same digit. Require the whole observed
            // entry for number changes; activity itself is never written here.
            let unchanged = self
                .remembered
                .iter()
                .filter(|old| entries.contains(old))
                .cloned()
                .collect::<Vec<_>>();
            merge_assigned_numbers(entries, &unchanged, &rows);
            if !unique_numbers(entries) {
                for (entry, old) in entries.iter_mut().zip(&before) {
                    (entry.number, entry.number_declined, entry.number_pinned) = number_state(old);
                }
            }
            for entry in entries.iter_mut() {
                let Some(observed) = self
                    .remembered
                    .iter()
                    .find(|old| old.project_root == entry.project_root)
                else {
                    continue;
                };
                if entry.name != observed.name {
                    continue;
                }
                if let Some(name) = rows
                    .iter()
                    .find(|row| row.project_root == entry.project_root)
                    .and_then(|row| row.name.as_ref())
                {
                    entry.name = Some(name.clone());
                }
            }
            // Validate the complete name batch so swaps can converge, while
            // preserved concurrent/unobserved claims are never overwritten.
            let mut names = BTreeSet::new();
            if !entries
                .iter()
                .filter_map(|entry| entry.name.as_ref())
                .all(|name| names.insert(name))
            {
                for (entry, old) in entries.iter_mut().zip(&before) {
                    entry.name = old.name.clone();
                }
            }
            Ok(entries
                .iter()
                .zip(before)
                .filter(|(entry, old)| *entry != old)
                .count())
        })
    }

    /// Explicit live renumbering changes only snapshot-unchanged, unique scoped
    /// records. A taken digit swaps with another such live record; hidden,
    /// stopped, ambiguous or newly recorded holders are never displaced.
    pub fn set_number(&self, index: usize, number: Option<u8>) -> Result<Option<PathBuf>> {
        if let Some(number) = number {
            ensure!(
                (1..=MAX_WORKSPACE_NUMBER).contains(&number),
                "a session number must be between 1 and {MAX_WORKSPACE_NUMBER}"
            );
        }
        let row = self.checked_row(index)?;
        ensure!(row.running, "only a running session can receive a number");
        let path = self
            .history_path
            .as_deref()
            .context("session numbering requires an available history cache")?;
        update_recents_if_changed(path, |entries| {
            unique_projects(entries)?;
            let target = self.unchanged_index(entries, &row.project_root)?;
            let vacated = entries[target].number;
            let mut displaced = None;
            if let Some(number) = number
                && let Some(holder) = entries.iter().position(|entry| {
                    entry.number == Some(number) && entry.project_root != row.project_root
                })
            {
                let holder_path = entries[holder].project_root.clone();
                let holder_row = self
                    .entries
                    .iter()
                    .position(|entry| entry.row.project_root == holder_path)
                    .context("the requested number belongs to an unobserved workspace")?;
                ensure!(
                    self.checked_row(holder_row)?.running,
                    "the requested number belongs to a stopped workspace"
                );
                self.unchanged_index(entries, &holder_path)?;
                entries[holder].number = vacated;
                if vacated.is_none() {
                    entries[holder].number_pinned = false;
                }
                displaced = Some(holder_path);
            }
            entries[target].number = number;
            entries[target].number_declined = number.is_none();
            entries[target].number_pinned = number.is_some();
            ensure!(
                unique_numbers(entries),
                "session numbering changed since observation"
            );
            Ok(displaced)
        })
    }

    /// Forgets exactly one unchanged observed stopped row. This removes only
    /// its cache entry, never its directory, endpoint, name store or process.
    pub fn forget(&self, index: usize) -> Result<bool> {
        let row = self.checked_row(index)?;
        ensure!(!row.running, "forget requires an observed stopped session");
        let Some(path) = self.history_path.as_deref() else {
            return Ok(false);
        };
        update_recents_if_changed(path, |entries| {
            unique_projects(entries)?;
            let index = self.unchanged_index(entries, &row.project_root)?;
            entries.remove(index);
            Ok(true)
        })
    }

    /// Clears only unchanged stopped rows actually shown by this observation.
    /// Concurrent visits/edits, new entries and absent directories survive.
    pub fn clear_stopped(&self) -> Result<usize> {
        let stopped = self
            .entries
            .iter()
            .enumerate()
            .filter(|(index, entry)| !entry.row.running && self.eligible(*index))
            .filter_map(|(_, entry)| {
                self.remembered
                    .iter()
                    .find(|old| old.project_root == entry.row.project_root)
            })
            .collect::<Vec<_>>();
        let Some(path) = self.history_path.as_deref().filter(|_| !stopped.is_empty()) else {
            return Ok(0);
        };
        update_recents_if_changed(path, |entries| {
            unique_projects(entries)?;
            let before = entries.len();
            entries.retain(|entry| !stopped.contains(&entry));
            Ok(before - entries.len())
        })
    }
}

#[cfg(test)]
mod tests;
