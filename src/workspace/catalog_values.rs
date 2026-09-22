// SPDX-License-Identifier: MPL-2.0

//! Owned catalog values and platform-neutral presentation/history semantics.
//! No discovery, filesystem access, transport, or publication selection lives
//! here. A row's project path is not authority to control a native publication.

use super::{
    SessionPreview,
    recent_history::{MAX_WORKSPACE_NUMBER, RecentEntry},
};
use crate::git::WorkspaceGitFacts;
use anyhow::Result;
use std::path::PathBuf;

/// The number of workspace-ID characters a listing shows by default.
///
/// A workspace ID is already a truncation of a hash of its project root, and
/// every selector that takes one resolves a prefix, so the full string is only
/// ever read to be shortened again by whoever types it. Six hex digits tell
/// apart far more workspaces than one person keeps, and they cost the listing
/// twenty-six fewer columns on a narrow terminal. Git abbreviates object IDs
/// for the same reason, and Runyte's own Git log already follows it.
pub const ABBREVIATED_WORKSPACE_ID: usize = 6;

/// The narrowest ID prefix that still tells `ids` apart.
///
/// Never below `ABBREVIATED_WORKSPACE_ID`, and above it only when two listed
/// workspaces genuinely share that many characters, so every ID a listing
/// prints stays a selector that resolves to exactly the row it was read from.
///
/// That holds for the listing it was computed from. A later command resolves
/// against whatever is registered then, so a workspace first recorded after
/// the listing was read could in principle share an abbreviation with a row it
/// showed. Doing so needs two project roots whose hashes agree over this many
/// hex digits, and both prefix resolvers answer more than one match by
/// reporting the selector as ambiguous, so the cost of the collision is an
/// error rather than reaching the wrong workspace.
pub fn abbreviated_id_width<'a>(ids: impl IntoIterator<Item = &'a str>) -> usize {
    let ids = ids.into_iter().collect::<Vec<_>>();
    let longest = ids.iter().map(|id| id.len()).max().unwrap_or(0);
    let mut prefixes = Vec::with_capacity(ids.len());
    for width in ABBREVIATED_WORKSPACE_ID..longest {
        prefixes.clear();
        prefixes.extend(ids.iter().map(|id| &id[..width.min(id.len())]));
        prefixes.sort_unstable();
        let before = prefixes.len();
        prefixes.dedup();
        if prefixes.len() == before {
            return width;
        }
    }
    longest.max(ABBREVIATED_WORKSPACE_ID)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRow {
    pub unread_terminals: Option<usize>,
    pub terminal_bell: Option<bool>,
    pub id: String,
    pub name: Option<String>,
    /// The digit that selects this workspace in the session manager, when it
    /// has one. Per-user history rather than host state: a running host does
    /// not answer it, and the same project numbered on one machine is
    /// unnumbered on another.
    pub number: Option<u8>,
    /// The latest wall-clock second at which this workspace was visited.
    /// Per-user history like `number`, and absent for catalog entries written
    /// before activity timestamps were introduced until they are visited.
    pub last_active_unix_seconds: Option<u64>,
    pub project_root: PathBuf,
    pub running: bool,
    /// The protocol of a running host this build cannot speak to. Such a
    /// workspace can be listed and stopped but never attached to, and it is
    /// worth naming rather than hiding: a host left over from another version
    /// keeps holding the endpoint every client resolves, so a workspace that
    /// looked stopped was the reason attaching to it failed.
    pub incompatible_protocol: Option<u32>,
    pub unsaved_buffers: Option<usize>,
    /// Every buffer the host holds open, unsaved or not.
    pub open_buffers: Option<usize>,
    pub pending_wait_requests: Option<usize>,
    pub plugin_jobs: Option<usize>,
    pub activity_leases: Option<usize>,
    pub activities: Vec<crate::service_health::ActivityLeaseHealth>,
    pub live_terminals: Option<usize>,
    pub terminal_sessions: Option<usize>,
    /// Latest creation/completed-line baseline among the host's live terminal
    /// sessions. Host-owned and never persisted in recent history.
    pub terminal_line_activity_unix_seconds: Option<u64>,
    pub interactive_attached: Option<bool>,
    /// What this workspace's own directory says about its Git checkout, when
    /// it is one. Read from files rather than answered by the host, because a
    /// stopped session has no host and its branch is worth listing anyway.
    pub git: Option<WorkspaceGitFacts>,
    /// Whether the project root has gone from disk while a host still runs in
    /// it. Such a row keeps its number and its place so it can be found and
    /// closed, rather than quietly becoming an unnumbered mystery.
    pub missing_directory: bool,
}

impl WorkspaceRow {
    /// The one wording for a workspace's state, so the CLI listing and the
    /// editor's picker cannot describe the same row differently.
    pub fn state_label(&self) -> String {
        match (self.running, self.incompatible_protocol) {
            (true, Some(protocol)) => format!("running (protocol {protocol})"),
            (true, None) => "running".to_owned(),
            (false, _) => "stopped".to_owned(),
        }
    }

    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            self.project_root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| self.project_root.display().to_string())
        })
    }
}

#[derive(Debug)]
pub enum WorkspaceEvent {
    DirectoryWorktrees {
        generation: u64,
        result: Result<Vec<PathBuf>, String>,
    },
    Inventory {
        generation: u64,
        path: PathBuf,
        result: Result<DestinationInventory, String>,
    },
    Observed {
        result: Result<Vec<WorkspaceRow>, String>,
    },
    Refreshed {
        generation: u64,
        result: Result<Vec<WorkspaceRow>, String>,
    },
    /// A silent manager-owned refresh used only while the overlay remains
    /// open, so host terminal activity can cross its display threshold.
    Polled {
        result: Result<Vec<WorkspaceRow>, String>,
    },
    Inspected {
        generation: u64,
        path: PathBuf,
        result: Box<Result<Option<WorkspaceRow>, String>>,
    },
    Previewed {
        generation: u64,
        path: PathBuf,
        result: Result<SessionPreview, String>,
    },
    Stopped {
        generation: u64,
        selector: PathBuf,
        result: Result<(), String>,
    },
    /// A workspace was dropped from the visited history. `recorded` is whether
    /// there was an entry to drop, so a row that came from the running registry
    /// rather than from history can say so instead of claiming a removal.
    Forgotten {
        generation: u64,
        path: PathBuf,
        result: Result<bool, String>,
    },
    Renamed {
        generation: u64,
        path: PathBuf,
        name: String,
        result: Result<(), String>,
    },
    /// A workspace's number changed. `displaced` names the workspace that gave
    /// the number up, when assigning it swapped a pair, so the editor can say
    /// where the old shortcut went.
    Numbered {
        generation: u64,
        path: PathBuf,
        number: Option<u8>,
        result: Result<Option<PathBuf>, String>,
    },
}

#[derive(Clone, Debug)]
pub struct DestinationInventory {
    pub incarnation: String,
    pub entries: Vec<crate::protocol::OpenDestinationEntry>,
    pub truncated: bool,
}

/// Supplies catalog names to running hosts which have never been explicitly
/// renamed. An explicit host name remains authoritative and is merged back
/// into recents after inspection.
#[cfg(any(unix, test))]
pub(super) fn apply_recent_names(rows: &mut [WorkspaceRow], recent_entries: &[RecentEntry]) {
    for row in rows.iter_mut().filter(|row| row.name.is_none()) {
        row.name = recent_entries
            .iter()
            .find(|entry| entry.project_root == row.project_root)
            .and_then(|entry| entry.name.clone());
    }
}

/// Gives every running session a digit, and no stopped or declining one.
///
/// The digit is a shortcut that attaches, so it belongs to a session somebody
/// can reach right now: a stopped session releases the one it held rather than
/// reserving one of the nine against the sessions that are actually up.
/// Explicitly renumbered running sessions reserve their chosen digits first;
/// automatic assignments then compact into the lowest remaining digits while
/// preserving their previous relative order. A new fourth running session is
/// therefore 4 even when stopped history used to hold that number.
///
/// A workspace whose record declines a digit is left alone. Taking one away is
/// a decision, and handing the row the lowest free digit on the next listing
/// would undo it before it could be seen.
///
/// A number is per-user history rather than host state, so a running host never
/// answers one and the catalog is the only place a preference can come from.
pub(super) fn assign_running_workspace_numbers(
    rows: &mut [WorkspaceRow],
    recent_entries: &[RecentEntry],
) {
    let record = |row: &WorkspaceRow| {
        recent_entries
            .iter()
            .find(|entry| entry.project_root == row.project_root)
    };
    let mut taken = [false; MAX_WORKSPACE_NUMBER as usize];
    let mut wanted = Vec::with_capacity(rows.len());
    for row in rows.iter_mut() {
        row.number = None;
        let entry = record(row);
        let declined = entry.is_some_and(|entry| entry.number_declined);
        wanted.push(row.running && !declined);
        if !row.running || declined {
            continue;
        }
        let preferred = entry
            .filter(|entry| entry.number_pinned)
            .and_then(|entry| entry.number)
            .filter(|number| {
                (1..=MAX_WORKSPACE_NUMBER).contains(number) && !taken[usize::from(number - 1)]
            });
        if let Some(number) = preferred {
            taken[usize::from(number - 1)] = true;
            row.number = Some(number);
        }
    }
    let mut automatic = rows
        .iter()
        .enumerate()
        .filter(|(index, row)| wanted[*index] && row.number.is_none())
        .map(|(index, row)| {
            let previous = record(row).and_then(|entry| entry.number);
            (index, previous)
        })
        .collect::<Vec<_>>();
    automatic.sort_by_key(|(index, previous)| (previous.is_none(), previous.unwrap_or(0), *index));
    for (index, _) in automatic {
        let row = &mut rows[index];
        if row.number.is_some() {
            continue;
        }
        let free = (1..=MAX_WORKSPACE_NUMBER).find(|candidate| !taken[usize::from(candidate - 1)]);
        if let Some(number) = free {
            taken[usize::from(number - 1)] = true;
            row.number = Some(number);
        }
    }
}

/// Puts the rows in the order the manager and `--session-list` show them.
///
/// A digit pins its session: numbered sessions lead the listing in digit order,
/// so the shortcut also says where the row is and the top of the list stops
/// reshuffling as sessions are visited. Everything else follows by the same
/// value the `Last active` column shows, least recently visited first, with a
/// session nothing has ever attached to last: `-` there is unknown rather than
/// old. Rows that tie in all of it are ordered by path so a listing does not
/// move between two refreshes that found the same sessions.
pub(super) fn order_workspace_rows(rows: &mut [WorkspaceRow]) {
    rows.sort_by_cached_key(|row| {
        (
            row.number.is_none(),
            row.number.unwrap_or(0),
            row.last_active_unix_seconds.is_none(),
            row.last_active_unix_seconds.unwrap_or(0),
            row.project_root.clone(),
        )
    });
}

/// Supplies the per-user visit time to every row, including running hosts.
///
/// Hosts do not own this value: it describes when this client was in the
/// workspace, so the recent-workspace catalog remains its single source.
pub(super) fn apply_recent_activity(rows: &mut [WorkspaceRow], recent_entries: &[RecentEntry]) {
    for row in rows.iter_mut() {
        row.last_active_unix_seconds = recent_entries
            .iter()
            .find(|entry| entry.project_root == row.project_root)
            .and_then(|entry| entry.last_active_unix_seconds);
    }
}

pub(super) fn validate_destination_inventory(
    incarnation: String,
    entries: Vec<crate::protocol::OpenDestinationEntry>,
    truncated: bool,
) -> Result<DestinationInventory> {
    use crate::protocol::{MAX_DESTINATION_LABEL_BYTES, MAX_DESTINATIONS, OpenDestination};
    anyhow::ensure!(
        incarnation.len() == 64 && incarnation.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid host incarnation in destination inventory"
    );
    anyhow::ensure!(
        entries.len() <= MAX_DESTINATIONS,
        "destination inventory exceeds its entry limit"
    );
    let mut identities = std::collections::BTreeSet::new();
    for entry in &entries {
        anyhow::ensure!(
            entry.label.len() <= MAX_DESTINATION_LABEL_BYTES
                && entry.detail.len() <= MAX_DESTINATION_LABEL_BYTES,
            "destination inventory label exceeds its byte limit"
        );
        let identity = match entry.destination {
            OpenDestination::Buffer(id) => (0, id),
            OpenDestination::Terminal(id) => (1, id),
        };
        anyhow::ensure!(
            identity.1 > 0 && identities.insert(identity),
            "destination inventory contains an invalid or duplicate resource identity"
        );
    }
    Ok(DestinationInventory {
        incarnation,
        entries,
        truncated,
    })
}

/// Applies names learned from running hosts to the current recents catalog.
///
/// The rows may have taken several control timeouts to inspect. Re-reading
/// under the writer lock is therefore essential: their original recents
/// snapshot is stale by construction and must not restore old ordering or
/// discard a workspace recorded while inspection was in flight. A name is
/// updated only when the current value still matches that snapshot, so an
/// inspection result cannot overwrite a newer name from another refresh.
/// Records the digit each running session was just given and clears numbering
/// state from stopped sessions.
///
/// The listing decides the numbers, so this is where automatic assignments
/// catch up after a gap closes. A stopped session retains neither an assigned
/// digit, an explicit pin, nor an explicit decision to stay unnumbered; if it
/// starts again, it joins the running sessions as a new automatic assignment.
/// Writing the answer back is also what lets the status line name this
/// session's digit without listing every workspace.
///
/// A record that changed while the listing was being gathered is not written,
/// for the same reason a concurrently renamed one is not: the person who
/// changed it answered more recently than this refresh read.
pub(super) fn merge_assigned_numbers(
    paths: &mut [RecentEntry],
    snapshot: &[RecentEntry],
    rows: &[WorkspaceRow],
) {
    for entry in paths {
        // These digits were decided against the catalog as the refresh found
        // it. A record that has changed since then holds somebody else's newer
        // answer -- a renumber, or a digit taken away, from another process --
        // so it is left alone rather than overwritten from a stale read. The
        // next listing resolves the numbers against what that answer left.
        let Some(snapshot_entry) = snapshot
            .iter()
            .find(|candidate| candidate.project_root == entry.project_root)
        else {
            continue;
        };
        if snapshot_entry.number != entry.number
            || snapshot_entry.number_declined != entry.number_declined
            || snapshot_entry.number_pinned != entry.number_pinned
        {
            continue;
        }
        let Some(row) = rows
            .iter()
            .find(|row| row.project_root == entry.project_root)
        else {
            continue;
        };
        if !row.running {
            entry.number = None;
            entry.number_declined = false;
            entry.number_pinned = false;
        } else if !entry.number_declined {
            entry.number = row.number;
            if row.number.is_none() {
                entry.number_pinned = false;
            }
        }
    }
}

#[cfg(test)]
mod tests;
