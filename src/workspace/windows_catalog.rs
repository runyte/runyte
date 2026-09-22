// SPDX-License-Identifier: MPL-2.0

//! Complete, read-only native live-host observations. Call from the catalog's
//! background owner: configured filesystem admission/scans are synchronous.
//! Discovery stays read-only; explicit history transactions are separate.
//! No stale deletion, service or CLI availability is enabled here.

pub use super::catalog_values::WorkspaceRow;
mod history;
use super::{
    windows_endpoint::{Candidate, EndpointMetadata, Inspection, Scan},
    windows_lifecycle::connect_control,
    windows_location::{KnownReadLocation, ResolvedLayout},
    windows_process_identity::PinnedProcess,
};
use crate::protocol::{ClientRequest, HostResponse};
use anyhow::{Context, Result, ensure};
pub use history::{
    HistoryEntry, HistorySnapshot, HistoryTarget, ensure_recorded, record_activity, remember,
    snapshot_with_history,
};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf, Prefix},
    sync::Arc,
    time::Duration,
};
use tokio::time::{Instant, timeout_at};

const MAX_KNOWN_PROJECTS: usize = super::recent_history::RECENT_LIMIT + 1;
const MAX_OBSERVATIONS: usize = 1024;
const MAX_HEALTH_BYTES: usize = 4 * 1024 * 1024;
const HOST_BUDGET: Duration = Duration::from_millis(500);
const SNAPSHOT_BUDGET: Duration = Duration::from_secs(2);

/// A display row plus its exact observation/proof. A workspace ID or path is
/// deliberately insufficient to identify this publication for control.
#[derive(Debug)]
pub struct CatalogEntry {
    row: WorkspaceRow,
    observations: Vec<Candidate>,
    peer: Arc<PinnedProcess>,
}

impl CatalogEntry {
    pub fn row(&self) -> &WorkspaceRow {
        &self.row
    }
    pub fn metadata(&self) -> &EndpointMetadata {
        self.observations[0].metadata()
    }
    pub fn observations(&self) -> &[Candidate] {
        &self.observations
    }
    pub fn peer(&self) -> &Arc<PinnedProcess> {
        &self.peer
    }
}

#[derive(Debug)]
pub struct CatalogSnapshot {
    entries: Vec<CatalogEntry>,
    // Retained observations only: callers must repeat exact stale proof under
    // the candidate's known locks before choosing to remove anything.
    stale: Vec<Candidate>,
    absent_projects: Vec<PathBuf>,
}

impl CatalogSnapshot {
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }
    pub fn stale_observations(&self) -> &[Candidate] {
        &self.stale
    }
    /// Explicitly configured projects with no live publication in this complete
    /// observation. No persisted recent-history entry is removed or changed.
    pub fn absent_projects(&self) -> &[PathBuf] {
        &self.absent_projects
    }

    pub fn select(
        &self,
        selector: &Path,
        working_directory: Option<&Path>,
    ) -> Result<Option<&CatalogEntry>> {
        let rows = self
            .entries
            .iter()
            .map(|entry| &entry.row)
            .collect::<Vec<_>>();
        let matches = select_indices(&rows, selector, working_directory);
        ensure!(
            matches.len() <= 1,
            "workspace selector {} matches multiple native publications; choose an unambiguous session name or namespace",
            selector.display()
        );
        Ok(matches.first().map(|index| &self.entries[*index]))
    }
}

/// Each known location must come from the same captured namespace selection.
/// Its ready file is observed explicitly, even when every registry row vanished.
/// Hidden inventory entries never supply these locations or namespace roots.
pub async fn snapshot(
    layout: &ResolvedLayout,
    known: &[ResolvedLayout],
    include_hidden: bool,
) -> Result<CatalogSnapshot> {
    ensure!(
        known.len() < MAX_KNOWN_PROJECTS,
        "native catalog known-project limit exceeded"
    );
    let known = known
        .iter()
        .map(ResolvedLayout::read_location)
        .collect::<Vec<_>>();
    snapshot_locations(layout, &known, include_hidden, true).await
}

async fn snapshot_locations(
    layout: &ResolvedLayout,
    known: &[KnownReadLocation],
    include_hidden: bool,
    allow_ready_cleanup: bool,
) -> Result<CatalogSnapshot> {
    ensure!(
        known.len() < MAX_KNOWN_PROJECTS,
        "native catalog known-project limit exceeded"
    );
    for project in known {
        ensure!(
            project.namespace_roots() == layout.namespace_roots(),
            "known project belongs to a different configured namespace"
        );
    }
    let view = layout.discovery_view(include_hidden)?;
    let mut observations = Vec::new();
    add_scan(&mut observations, view.scan_namespaces())?;
    if include_hidden {
        add_scan(&mut observations, view.scan_inventory())?;
    }
    let mut projects = BTreeMap::new();
    let current = layout.read_location();
    for project in std::iter::once(&current).chain(known) {
        let location = (
            project.project_root().to_owned(),
            project.endpoint_directory().to_owned(),
        );
        if projects.insert(location.clone(), ()).is_none() {
            let observed = if allow_ready_cleanup {
                view.observe_ready(&location.0, location.1)?
            } else {
                view.observe_snapshot_ready(project)?
            };
            if let Some(candidate) = observed {
                ensure!(
                    observations.len() < MAX_OBSERVATIONS,
                    "native catalog observation limit exceeded"
                );
                observations.push(candidate);
            }
        }
    }
    let known_projects = projects.into_keys().map(|(project, _)| project).collect();
    build_snapshot(
        observations,
        known_projects,
        Instant::now() + SNAPSHOT_BUDGET,
    )
    .await
}

fn add_scan(observations: &mut Vec<Candidate>, scan: Scan) -> Result<()> {
    ensure!(
        scan.limit.is_none(),
        "native catalog scan was truncated: {:?}",
        scan.limit
    );
    if let Some(issue) = scan.issues.into_iter().next() {
        return Err(issue.error)
            .context("native catalog could not completely read its configured registry");
    }
    ensure!(
        observations.len().saturating_add(scan.candidates.len()) <= MAX_OBSERVATIONS,
        "native catalog observation limit exceeded"
    );
    observations.extend(scan.candidates);
    Ok(())
}

#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct PublicationKey {
    project: Vec<u8>,
    pid: u32,
    creation: u64,
    incarnation: String,
    pipe: String,
}
impl From<&EndpointMetadata> for PublicationKey {
    fn from(metadata: &EndpointMetadata) -> Self {
        Self {
            project: metadata.project_root_bytes.clone(),
            pid: metadata.process.pid,
            creation: metadata.process.creation_time,
            incarnation: metadata.incarnation.clone(),
            pipe: metadata.address.as_str().to_owned(),
        }
    }
}

fn group(observations: Vec<Candidate>) -> Result<BTreeMap<PublicationKey, Vec<Candidate>>> {
    let mut grouped: BTreeMap<PublicationKey, Vec<Candidate>> = BTreeMap::new();
    for candidate in observations {
        let copies = grouped.entry(candidate.metadata().into()).or_default();
        if let Some(previous) = copies.first() {
            ensure!(
                previous.metadata() == candidate.metadata(),
                "native publication copies disagree; catalog observation is uncertain"
            );
        }
        copies.push(candidate);
    }
    Ok(grouped)
}

async fn build_snapshot(
    observations: Vec<Candidate>,
    known: Vec<PathBuf>,
    deadline: Instant,
) -> Result<CatalogSnapshot> {
    let mut result = CatalogSnapshot {
        entries: Vec::new(),
        stale: Vec::new(),
        absent_projects: Vec::new(),
    };
    let mut health_bytes = 0;
    for observations in group(observations)?.into_values() {
        let candidate = &observations[0];
        match candidate.inspect_process()? {
            Inspection::Stale(_) => {
                result.stale.extend(observations);
                continue;
            }
            Inspection::Present(_) => {}
        }
        ensure!(
            Instant::now() < deadline,
            "native catalog probe budget exhausted"
        );
        let deadline = deadline.min(Instant::now() + HOST_BUDGET);
        let (mut row, peer) = timeout_at(deadline, inspect(candidate, deadline, &mut health_bytes))
            .await
            .context("native catalog host observation timed out")??;
        ensure!(
            peer.is_alive()?,
            "native catalog host exited during observation"
        );
        // A metadata hint must not trigger UNC/device traversal merely to
        // decorate a listing. Native configured publications require local paths.
        let mut components = row.project_root.components();
        ensure!(
            matches!(components.next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
                && matches!(components.next(), Some(Component::RootDir)),
            "native catalog project is not a local filesystem path"
        );
        row.missing_directory = match row.project_root.metadata() {
            Ok(metadata) => !metadata.is_dir(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => {
                return Err(error).context("native catalog project directory is indeterminate");
            }
        };
        result.entries.push(CatalogEntry {
            row,
            observations,
            peer,
        });
    }
    for project in known {
        if !result
            .entries
            .iter()
            .any(|entry| entry.row.project_root == project)
            && !result.absent_projects.contains(&project)
        {
            result.absent_projects.push(project);
        }
    }
    result.entries.sort_by(|left, right| {
        left.row
            .project_root
            .cmp(&right.row.project_root)
            .then_with(|| {
                left.metadata()
                    .incarnation
                    .cmp(&right.metadata().incarnation)
            })
    });
    Ok(result)
}

async fn inspect(
    candidate: &Candidate,
    deadline: Instant,
    health_bytes: &mut usize,
) -> Result<(WorkspaceRow, Arc<PinnedProcess>)> {
    let metadata = candidate.metadata();
    let mut row = row(metadata)?;
    if metadata.protocol != crate::protocol::VERSION {
        let proof = candidate.authenticate(deadline).await?;
        return Ok((row, proof.peer().clone()));
    }
    let mut client = connect_control(metadata).await?;
    client.send(&ClientRequest::Health).await?;
    let response = client
        .recv()
        .await?
        .context("native catalog host closed before health")?;
    apply_health(
        &mut row,
        response,
        client.peer().identity().pid,
        health_bytes,
    )?;
    Ok((row, client.peer().clone()))
}

fn row(metadata: &EndpointMetadata) -> Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        id: metadata.id.clone(),
        name: metadata.name.clone(),
        project_root: metadata.project_root()?,
        running: true,
        incompatible_protocol: (metadata.protocol != crate::protocol::VERSION)
            .then_some(metadata.protocol),
        number: None,
        last_active_unix_seconds: None,
        unread_terminals: None,
        terminal_bell: None,
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
    })
}

fn apply_health(
    row: &mut WorkspaceRow,
    response: HostResponse,
    expected_pid: u32,
    used: &mut usize,
) -> Result<()> {
    let HostResponse::Health {
        protocol,
        pid,
        interactive_attached,
        unsaved_buffers,
        open_buffers,
        pending_wait_requests,
        plugin_jobs,
        activity_leases,
        activities,
        live_terminals,
        terminal_sessions,
        terminal_line_activity_unix_seconds,
        unread_terminals,
        terminal_bell,
    } = response
    else {
        anyhow::bail!("native catalog received an unexpected health response");
    };
    ensure!(
        protocol == crate::protocol::VERSION && pid == expected_pid,
        "native health identity disagrees with its authenticated peer"
    );
    let mut bytes = activities
        .len()
        .checked_mul(std::mem::size_of::<
            crate::service_health::ActivityLeaseHealth,
        >())
        .context("native catalog health size overflow")?;
    for activity in &activities {
        bytes = bytes
            .checked_add(activity.owner.len())
            .and_then(|value| value.checked_add(activity.title.len()))
            .context("native catalog health size overflow")?;
    }
    *used = used
        .checked_add(bytes)
        .context("native catalog health size overflow")?;
    ensure!(
        *used <= MAX_HEALTH_BYTES,
        "native catalog aggregate health limit exceeded"
    );
    row.interactive_attached = Some(interactive_attached);
    row.unsaved_buffers = Some(unsaved_buffers);
    row.open_buffers = Some(open_buffers);
    row.pending_wait_requests = Some(pending_wait_requests);
    row.plugin_jobs = Some(plugin_jobs);
    row.activity_leases = Some(activity_leases);
    row.activities = activities.into_iter().map(Into::into).collect();
    row.live_terminals = Some(live_terminals);
    row.terminal_sessions = Some(terminal_sessions);
    row.terminal_line_activity_unix_seconds = terminal_line_activity_unix_seconds;
    row.unread_terminals = Some(unread_terminals);
    row.terminal_bell = Some(terminal_bell);
    Ok(())
}

fn select_indices(
    rows: &[&WorkspaceRow],
    selector: &Path,
    working_directory: Option<&Path>,
) -> Vec<usize> {
    let text = selector.to_str();
    let lower = text.map(str::to_ascii_lowercase);
    let supplied = if selector.is_absolute() {
        Some(selector.to_path_buf())
    } else {
        working_directory.map(|cwd| cwd.join(selector))
    };
    let directory = supplied.map(selector_directory);
    let mut matches = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            lower.as_ref().is_some_and(|id| row.id == *id)
                || text.is_some_and(|name| row.name.as_deref() == Some(name))
                || directory.as_ref() == Some(&row.project_root)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matches.is_empty()
        && let Some(prefix) = lower
            .as_deref()
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        matches.extend(
            rows.iter()
                .enumerate()
                .filter(|(_, row)| row.id.starts_with(prefix))
                .map(|(index, _)| index),
        );
    }
    matches
}

// A live host can outlast its project directory. Resolve its still-existing
// parent so an ordinary drive path keeps matching the captured verbatim path,
// without changing the absent components' case or native character units.
fn selector_directory(path: PathBuf) -> PathBuf {
    let mut missing = Vec::new();
    let mut ancestor = path.as_path();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for component in missing.into_iter().rev() {
                    resolved.push(component);
                }
                return resolved;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let (Some(name), Some(parent)) = (ancestor.file_name(), ancestor.parent()) else {
                    return path;
                };
                missing.push(name.to_owned());
                ancestor = parent;
            }
            Err(_) => return path,
        }
    }
}

#[cfg(test)]
mod tests;
