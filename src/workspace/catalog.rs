// SPDX-License-Identifier: MPL-2.0

//! Asynchronous discovery and lifecycle work for switchable workspaces.
//!
//! Registry access and control connections may block or time out, so the
//! editor submits bounded requests and accepts owned completion events. The
//! picker never performs filesystem or transport work on the render path.

use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use crate::protocol::{ClientRequest, HostResponse};
use crate::{external_open, project_root};

use super::{
    SessionPreview,
    lifecycle::{
        HostStartup, connect_control, ensure_workspace_host, force_shutdown_host, rename_host,
        resolve_registered_host_from, resolve_workspace_endpoint, shutdown_host,
        terminate_incompatible_host,
    },
    transport::{
        LocalEndpoint, MAX_HOST_NAME_BYTES, RegisteredHost, decode_path, encode_path,
        registered_hosts_in, registry_roots, validate_host_name, workspace_id,
    },
};

const REQUEST_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 16;
const CONTROL_TIMEOUT: Duration = Duration::from_millis(500);
const RECENT_LIMIT: usize = 256;

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
    pub id: String,
    pub name: Option<String>,
    /// The digit that selects this workspace in the session manager, when it
    /// has one. Per-user history rather than host state: a running host does
    /// not answer it, and the same project numbered on one machine is
    /// unnumbered on another.
    pub number: Option<u8>,
    pub project_root: PathBuf,
    pub running: bool,
    /// The protocol of a running host this build cannot speak to. Such a
    /// workspace can be listed and stopped but never attached to, and it is
    /// worth naming rather than hiding: a host left over from another version
    /// keeps holding the endpoint every client resolves, so a workspace that
    /// looked stopped was the reason attaching to it failed.
    pub incompatible_protocol: Option<u32>,
    pub unsaved_buffers: Option<usize>,
    pub pending_wait_requests: Option<usize>,
    pub live_terminals: Option<usize>,
    pub terminal_sessions: Option<usize>,
    pub interactive_attached: Option<bool>,
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
enum WorkspaceRequest {
    Refresh {
        generation: u64,
    },
    Inspect {
        generation: u64,
        path: PathBuf,
    },
    Start {
        generation: u64,
        selector: PathBuf,
        working_directory: PathBuf,
    },
    Stop {
        generation: u64,
        selector: PathBuf,
        working_directory: PathBuf,
        force: bool,
    },
    Forget {
        generation: u64,
        path: PathBuf,
    },
    Rename {
        generation: u64,
        selector: PathBuf,
        working_directory: PathBuf,
        name: String,
    },
    Number {
        generation: u64,
        selector: PathBuf,
        working_directory: PathBuf,
        number: Option<u8>,
    },
}

#[derive(Debug)]
pub enum WorkspaceEvent {
    Refreshed {
        generation: u64,
        result: Result<Vec<WorkspaceRow>, String>,
    },
    Inspected {
        generation: u64,
        path: PathBuf,
        result: Result<Option<WorkspaceRow>, String>,
    },
    Previewed {
        generation: u64,
        path: PathBuf,
        result: Result<SessionPreview, String>,
    },
    Started {
        generation: u64,
        path: PathBuf,
        result: Result<(), String>,
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

#[derive(Clone)]
pub struct WorkspaceServiceHandle {
    requests: mpsc::Sender<WorkspaceRequest>,
    previews: watch::Sender<Option<WorkspacePreviewRequest>>,
}

#[derive(Clone, Debug)]
struct WorkspacePreviewRequest {
    generation: u64,
    path: PathBuf,
}

impl WorkspaceServiceHandle {
    pub fn try_refresh(&self, generation: u64) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Refresh { generation })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_inspect(&self, generation: u64, path: PathBuf) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Inspect { generation, path })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    /// Requests the selected session's live overview. A watch slot retains
    /// only the newest selection while an earlier host is answering, so fast
    /// picker movement cannot build a queue of stale socket round trips.
    pub fn try_preview(&self, generation: u64, path: PathBuf) -> Result<(), &'static str> {
        self.previews
            .send(Some(WorkspacePreviewRequest { generation, path }))
            .map_err(|_| "session preview service is unavailable")
    }

    pub fn try_stop(
        &self,
        generation: u64,
        selector: PathBuf,
        working_directory: PathBuf,
        force: bool,
    ) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Stop {
                generation,
                selector,
                working_directory,
                force,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_start(
        &self,
        generation: u64,
        selector: PathBuf,
        working_directory: PathBuf,
    ) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Start {
                generation,
                selector,
                working_directory,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_forget(&self, generation: u64, path: PathBuf) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Forget { generation, path })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_rename(
        &self,
        generation: u64,
        selector: PathBuf,
        working_directory: PathBuf,
        name: String,
    ) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Rename {
                generation,
                selector,
                working_directory,
                name,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_number(
        &self,
        generation: u64,
        selector: PathBuf,
        working_directory: PathBuf,
        number: Option<u8>,
    ) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Number {
                generation,
                selector,
                working_directory,
                number,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }
}

pub struct WorkspaceService;

impl WorkspaceService {
    pub fn spawn(
        executable: PathBuf,
        state: PathBuf,
        config: Option<PathBuf>,
    ) -> (WorkspaceServiceHandle, mpsc::Receiver<WorkspaceEvent>) {
        Self::spawn_with(
            registry_roots(),
            recent_file(),
            executable,
            state,
            config,
            None,
        )
    }

    fn spawn_with(
        roots: Vec<PathBuf>,
        recents: Option<PathBuf>,
        executable: PathBuf,
        state: PathBuf,
        config: Option<PathBuf>,
        runtime: Option<PathBuf>,
    ) -> (WorkspaceServiceHandle, mpsc::Receiver<WorkspaceEvent>) {
        let (request_tx, mut request_rx) = mpsc::channel(REQUEST_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
        let (preview_tx, mut preview_rx) = watch::channel(None::<WorkspacePreviewRequest>);
        let preview_events = event_tx.clone();
        let preview_state = state.clone();
        tokio::spawn(async move {
            while preview_rx.changed().await.is_ok() {
                let request = preview_rx.borrow_and_update().clone();
                let Some(WorkspacePreviewRequest { generation, path }) = request else {
                    continue;
                };
                let result = preview_session(&path, &preview_state)
                    .await
                    .map_err(|error| format!("{error:#}"));
                if preview_events
                    .send(WorkspaceEvent::Previewed {
                        generation,
                        path,
                        result,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            while let Some(request) = request_rx.recv().await {
                let event = match request {
                    WorkspaceRequest::Refresh { generation } => WorkspaceEvent::Refreshed {
                        generation,
                        result: refresh(&roots, recents.as_deref(), &state, runtime.as_deref())
                            .await
                            .map_err(|error| format!("{error:#}")),
                    },
                    WorkspaceRequest::Inspect { generation, path } => {
                        let result = inspect_workspace_target(&path, &state, runtime.as_deref())
                            .await
                            .map_err(|error| format!("{error:#}"));
                        WorkspaceEvent::Inspected {
                            generation,
                            path,
                            result,
                        }
                    }
                    WorkspaceRequest::Start {
                        generation,
                        selector,
                        working_directory,
                    } => {
                        let startup = HostStartup::new(executable.clone(), "started")
                            .with_config(config.as_deref());
                        let result = start(
                            &roots,
                            recents.as_deref(),
                            &selector,
                            &working_directory,
                            &state,
                            config.as_deref(),
                            startup,
                        )
                        .await
                        .map_err(|error| format!("{error:#}"));
                        WorkspaceEvent::Started {
                            generation,
                            path: selector,
                            result,
                        }
                    }
                    WorkspaceRequest::Stop {
                        generation,
                        selector,
                        working_directory,
                        force,
                    } => {
                        let result = stop(
                            &roots,
                            &selector,
                            &working_directory,
                            &state,
                            config.as_deref(),
                            force,
                        )
                        .await
                        .map_err(|error| format!("{error:#}"));
                        WorkspaceEvent::Stopped {
                            generation,
                            selector,
                            result,
                        }
                    }
                    WorkspaceRequest::Forget { generation, path } => {
                        let recents = recents.clone();
                        let target = path.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            forget_recent_workspace_in(recents.as_deref(), &target)
                        })
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|forgotten| forgotten.map_err(|error| format!("{error:#}")));
                        WorkspaceEvent::Forgotten {
                            generation,
                            path,
                            result,
                        }
                    }
                    WorkspaceRequest::Rename {
                        generation,
                        selector,
                        working_directory,
                        name,
                    } => {
                        let result = rename(
                            &roots,
                            recents.as_deref(),
                            &selector,
                            &working_directory,
                            &name,
                            &state,
                            config.as_deref(),
                        )
                        .await
                        .map_err(|error| format!("{error:#}"));
                        WorkspaceEvent::Renamed {
                            generation,
                            path: selector,
                            name,
                            result,
                        }
                    }
                    WorkspaceRequest::Number {
                        generation,
                        selector,
                        working_directory,
                        number,
                    } => {
                        let result = number_workspace(
                            &roots,
                            recents.as_deref(),
                            &selector,
                            &working_directory,
                            number,
                            &state,
                        )
                        .await
                        .map_err(|error| format!("{error:#}"));
                        WorkspaceEvent::Numbered {
                            generation,
                            path: selector,
                            number,
                            result,
                        }
                    }
                };
                if event_tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        (
            WorkspaceServiceHandle {
                requests: request_tx,
                previews: preview_tx,
            },
            event_rx,
        )
    }
}

/// Enumerates running and recently visited workspaces for non-editor clients
/// such as `--session-list`.
///
/// `state` is the configured workspace state directory. It is needed because a
/// running host is not always a registered one: reaching the endpoint a project
/// publishes means resolving that project's state root the same way a
/// connecting client does.
pub async fn known_workspaces(state: &Path) -> Result<Vec<WorkspaceRow>> {
    refresh(&registry_roots(), recent_file().as_deref(), state, None).await
}

/// Removes every stopped session from the visited history and returns the
/// number of history entries removed. Running sessions remain discoverable
/// through their endpoint registry even when they have no recent entry.
pub async fn clear_stopped_sessions(state: &Path) -> Result<usize> {
    let stopped = known_workspaces(state)
        .await?
        .into_iter()
        .filter(|row| !row.running)
        .map(|row| row.project_root)
        .collect::<Vec<_>>();
    let recents = recent_file();
    tokio::task::spawn_blocking(move || clear_recent_workspaces_in(recents.as_deref(), &stopped))
        .await?
}

/// Renames a known workspace for non-editor clients such as `--session-rename`.
///
/// This is the same operation the editor's session list performs, so the two
/// agree about what a name means: a running host still owns and validates its
/// persisted name, while a stopped workspace is renamed in the visited history
/// it is listed from.
pub async fn rename_known_workspace(
    selector: &Path,
    name: &str,
    state: &Path,
    config: Option<&Path>,
) -> Result<()> {
    let working_directory =
        std::env::current_dir().context("the current directory is unavailable")?;
    rename(
        &registry_roots(),
        recent_file().as_deref(),
        selector,
        &working_directory,
        name,
        state,
        config,
    )
    .await
}

/// Resolves a known workspace selector, including stopped recents.
pub async fn resolve_known_workspace(selector: &Path, state: &Path) -> Result<Option<PathBuf>> {
    let rows = known_workspaces(state).await?;
    let working_directory = std::env::current_dir().ok();
    resolve_known_workspace_from_rows(&rows, selector, working_directory.as_deref())
}

/// Resolves a known workspace while interpreting a relative directory
/// selector from the editor's working directory.
pub async fn resolve_known_workspace_from_directory(
    selector: &Path,
    working_directory: &Path,
    state: &Path,
) -> Result<Option<PathBuf>> {
    let rows = known_workspaces(state).await?;
    resolve_known_workspace_from_rows(&rows, selector, Some(working_directory))
}

fn resolve_known_workspace_from_rows(
    rows: &[WorkspaceRow],
    selector: &Path,
    working_directory: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let text = selector.to_str();
    let lower = text.map(str::to_ascii_lowercase);
    let supplied = if selector.is_absolute() {
        Some(selector.to_path_buf())
    } else {
        working_directory.map(|cwd| cwd.join(selector))
    };
    let directory = supplied.map(|path| path.canonicalize().unwrap_or(path));
    let mut matches = rows
        .iter()
        .filter(|row| {
            lower.as_ref().is_some_and(|id| row.id == *id)
                || text.is_some_and(|name| row.name.as_deref() == Some(name))
                || directory.as_ref() == Some(&row.project_root)
        })
        .map(|row| row.project_root.clone())
        .collect::<Vec<_>>();
    if matches.is_empty()
        && let Some(prefix) = lower
            .as_deref()
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        matches.extend(
            rows.iter()
                .filter(|row| row.id.starts_with(prefix))
                .map(|row| row.project_root.clone()),
        );
    }
    matches.sort();
    matches.dedup();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => anyhow::bail!(
            "workspace selector {} is ambiguous; use its ID or directory",
            selector.display()
        ),
    }
}

/// Builds the workspace inventory.
///
/// `runtime` overrides the user runtime directory endpoints are published in.
/// Production passes `None`, which means the one the environment names; tests
/// pass their own so a scan never reads or writes the real one.
async fn refresh(
    roots: &[PathBuf],
    recents: Option<&Path>,
    state: &Path,
    runtime: Option<&Path>,
) -> Result<Vec<WorkspaceRow>> {
    let scan_roots = roots.to_vec();
    let recent_path = recents.map(Path::to_path_buf);
    let (hosts, mut recent_entries) = tokio::task::spawn_blocking(move || {
        Ok::<_, anyhow::Error>((
            registered_hosts_in(&scan_roots)?,
            read_recents(recent_path.as_deref())?,
        ))
    })
    .await??;
    let recent_snapshot = recent_entries.clone();
    assign_missing_default_workspace_names(&mut recent_entries);
    let mut rows = Vec::with_capacity(hosts.len());
    for host in hosts {
        rows.push(inspect_host(host).await);
    }
    apply_recent_names(&mut rows, &recent_entries);
    for RecentEntry {
        project_root,
        name,
        number: _,
    } in recent_entries.clone()
    {
        if rows.iter().any(|row| row.project_root == project_root) {
            continue;
        }
        let id = workspace_id(&project_root);
        // The registry did not account for this workspace, which does not
        // settle whether a host is running in it: a host of another version
        // publishes an identity this build's registry scan discards, and a
        // registration can be lost while its host keeps the endpoint. Reading
        // the endpoint itself is the only answer that agrees with what a
        // connecting client will find there.
        if let Some(row) = published_row(&project_root, name.clone(), state, runtime).await {
            rows.push(row);
            continue;
        }
        rows.push(WorkspaceRow {
            id,
            name,
            number: None,
            project_root,
            running: false,
            incompatible_protocol: None,
            // A history-only row has no running process that can answer
            // host-owned state. Listings leave those columns blank.
            unsaved_buffers: None,
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: None,
        });
    }
    apply_recent_numbers(&mut rows, &recent_entries);
    // Most recently visited first, which is the order the recents file already
    // holds. A workspace absent from that history is a running host whose
    // record was pruned or never written; it sorts after the remembered ones,
    // by path, so the listing stays stable between refreshes.
    rows.sort_by_cached_key(|row| {
        let recency = recent_snapshot
            .iter()
            .position(|entry| entry.project_root == row.project_root)
            .unwrap_or(usize::MAX);
        (recency, row.project_root.clone())
    });
    if let Some(path) = recents {
        let path = path.to_path_buf();
        let refreshed_rows = rows.clone();
        tokio::task::spawn_blocking(move || {
            merge_refreshed_names(&path, &recent_snapshot, &refreshed_rows)
        })
        .await??;
    }
    Ok(rows)
}

/// Supplies catalog names to running hosts which have never been explicitly
/// renamed. An explicit host name remains authoritative and is merged back
/// into recents after inspection.
fn apply_recent_names(rows: &mut [WorkspaceRow], recent_entries: &[RecentEntry]) {
    for row in rows.iter_mut().filter(|row| row.name.is_none()) {
        row.name = recent_entries
            .iter()
            .find(|entry| entry.project_root == row.project_root)
            .and_then(|entry| entry.name.clone());
    }
}

/// Supplies catalog numbers to rows discovered as running hosts.
///
/// A number is per-user history rather than host state, so a running host
/// never answers one and the catalog is the only place it can come from.
fn apply_recent_numbers(rows: &mut [WorkspaceRow], recent_entries: &[RecentEntry]) {
    for row in rows.iter_mut() {
        row.number = recent_entries
            .iter()
            .find(|entry| entry.project_root == row.project_root)
            .and_then(|entry| entry.number);
    }
}

/// Describes the host a project root publishes, when one is there and the
/// registry did not already account for it.
async fn published_row(
    project_root: &Path,
    name: Option<String>,
    state: &Path,
    runtime: Option<&Path>,
) -> Option<WorkspaceRow> {
    let endpoint = published_endpoint(project_root, state, runtime).ok()?;
    let host = endpoint.published_host().ok().flatten()?;
    if !host.speaks_current_protocol() {
        return Some(WorkspaceRow {
            id: host.id,
            // Nothing can ask a host of another protocol anything, so its
            // buffer counts stay unknown rather than being reported as zero.
            name: host.name.or(name),
            // Filled from the catalog once every row exists.
            number: None,
            project_root: host.project_root,
            running: true,
            incompatible_protocol: Some(host.protocol),
            unsaved_buffers: None,
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: None,
        });
    }
    let inspection = inspect_endpoint(&endpoint).await;
    Some(WorkspaceRow {
        id: host.id,
        name: host.name.or(name),
        number: None,
        project_root: host.project_root,
        running: true,
        incompatible_protocol: None,
        unsaved_buffers: inspection.unsaved_buffers,
        pending_wait_requests: inspection.pending_wait_requests,
        live_terminals: inspection.live_terminals,
        terminal_sessions: inspection.terminal_sessions,
        interactive_attached: inspection.interactive_attached,
    })
}

/// Inspects one exact workspace endpoint for a destructive operation.
///
/// Unlike the catalog refresh, this does not depend on registry or recent
/// history membership and does not turn an endpoint it cannot verify into a
/// stopped row. Endpoint artifacts with an unverifiable owner fail closed.
async fn inspect_workspace_target(
    project_root: &Path,
    state: &Path,
    runtime: Option<&Path>,
) -> Result<Option<WorkspaceRow>> {
    let project_root = project_root
        .canonicalize()
        .with_context(|| format!("cannot resolve workspace {}", project_root.display()))?;
    let endpoint = published_endpoint(&project_root, state, runtime)?;
    if !endpoint.metadata().exists() && !endpoint.socket().exists() {
        return Ok(None);
    }
    let host = endpoint.published_host()?.with_context(|| {
        format!(
            "workspace endpoint for {} exists but its owner or health cannot be verified",
            project_root.display()
        )
    })?;
    anyhow::ensure!(
        host.project_root == project_root,
        "workspace endpoint identity does not match {}",
        project_root.display()
    );
    if !host.speaks_current_protocol() {
        return Ok(Some(WorkspaceRow {
            id: host.id,
            name: host.name,
            number: None,
            project_root,
            running: true,
            incompatible_protocol: Some(host.protocol),
            unsaved_buffers: None,
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: None,
        }));
    }
    let inspection = inspect_endpoint_strict(&endpoint).await?;
    Ok(Some(WorkspaceRow {
        id: host.id,
        name: host.name,
        number: None,
        project_root,
        running: true,
        incompatible_protocol: None,
        unsaved_buffers: Some(inspection.unsaved_buffers),
        pending_wait_requests: Some(inspection.pending_wait_requests),
        live_terminals: Some(inspection.live_terminals),
        terminal_sessions: Some(inspection.terminal_sessions),
        interactive_attached: Some(inspection.interactive_attached),
    }))
}

/// Resolves the endpoint a project root publishes, the same way a connecting
/// client resolves it.
fn published_endpoint(
    project_root: &Path,
    state: &Path,
    runtime: Option<&Path>,
) -> Result<LocalEndpoint> {
    let state_root = project_root::resolve_state_root(project_root, state);
    match runtime {
        Some(runtime) => {
            LocalEndpoint::discover_with_runtime(&state_root, project_root, Some(runtime))
        }
        None => LocalEndpoint::discover(&state_root, project_root),
    }
}

async fn inspect_host(host: RegisteredHost) -> WorkspaceRow {
    let inspection = inspect_endpoint(host.endpoint()).await;
    WorkspaceRow {
        id: host.id,
        name: host.name,
        number: None,
        project_root: host.project_root,
        running: true,
        incompatible_protocol: None,
        unsaved_buffers: inspection.unsaved_buffers,
        pending_wait_requests: inspection.pending_wait_requests,
        live_terminals: inspection.live_terminals,
        terminal_sessions: inspection.terminal_sessions,
        interactive_attached: inspection.interactive_attached,
    }
}

/// Asks a reachable host for the counts a listing shows. An unreachable or slow
/// host leaves them unknown rather than delaying the whole inventory.
///
/// The host reports its own unsaved-buffer count rather than being handed its
/// buffer list to count from, so a row can never call a workspace clean that
/// the same host would refuse to stop.
#[derive(Default)]
struct HostInspection {
    unsaved_buffers: Option<usize>,
    pending_wait_requests: Option<usize>,
    live_terminals: Option<usize>,
    terminal_sessions: Option<usize>,
    interactive_attached: Option<bool>,
}

struct StrictHostInspection {
    unsaved_buffers: usize,
    pending_wait_requests: usize,
    live_terminals: usize,
    terminal_sessions: usize,
    interactive_attached: bool,
}

async fn inspect_endpoint_strict(endpoint: &LocalEndpoint) -> Result<StrictHostInspection> {
    tokio::time::timeout(CONTROL_TIMEOUT, async {
        let mut client = connect_control(endpoint).await?;
        client.send(&ClientRequest::Health).await?;
        match client.recv().await? {
            Some(HostResponse::Health {
                interactive_attached,
                unsaved_buffers,
                pending_wait_requests,
                live_terminals,
                terminal_sessions,
                ..
            }) => Ok(StrictHostInspection {
                unsaved_buffers,
                pending_wait_requests,
                live_terminals,
                terminal_sessions,
                interactive_attached,
            }),
            Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                anyhow::bail!(message)
            }
            Some(_) => anyhow::bail!("workspace host returned the wrong health response"),
            None => anyhow::bail!("workspace host closed before returning its health"),
        }
    })
    .await
    .context("workspace health check timed out")?
}

async fn inspect_endpoint(endpoint: &LocalEndpoint) -> HostInspection {
    let mut result = HostInspection::default();
    let inspection = tokio::time::timeout(CONTROL_TIMEOUT, async {
        let mut client = connect_control(endpoint).await?;
        client.send(&ClientRequest::Health).await?;
        if let Some(HostResponse::Health {
            interactive_attached: attached,
            unsaved_buffers: unsaved,
            pending_wait_requests,
            live_terminals,
            terminal_sessions,
            ..
        }) = client.recv().await?
        {
            result.interactive_attached = Some(attached);
            result.unsaved_buffers = Some(unsaved);
            result.pending_wait_requests = Some(pending_wait_requests);
            result.live_terminals = Some(live_terminals);
            result.terminal_sessions = Some(terminal_sessions);
        }
        Ok::<_, anyhow::Error>(())
    })
    .await;
    let _ = inspection;
    result
}

/// Reads only the live host selected in the session manager. Unlike catalog
/// health, this request is intentionally lazy because it contains editor text
/// and terminal output rather than a few scalar counts.
async fn preview_session(project_root: &Path, state: &Path) -> Result<SessionPreview> {
    let endpoint = published_endpoint(project_root, state, None)?;
    tokio::time::timeout(CONTROL_TIMEOUT, async {
        let mut client = connect_control(&endpoint).await?;
        client.send(&ClientRequest::SessionPreview).await?;
        match client.recv().await? {
            Some(HostResponse::SessionPreview { preview }) => Ok(preview.into()),
            Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                anyhow::bail!(message)
            }
            Some(_) => anyhow::bail!("workspace host returned the wrong session preview response"),
            None => anyhow::bail!("workspace host closed before returning a session preview"),
        }
    })
    .await
    .context("session preview timed out")?
}

async fn start(
    roots: &[PathBuf],
    recents: Option<&Path>,
    selector: &Path,
    working_directory: &Path,
    state: &Path,
    config: Option<&Path>,
    startup: HostStartup,
) -> Result<()> {
    let rows = refresh(roots, recents, state, None).await?;
    let requested = resolve_known_workspace_from_rows(&rows, selector, Some(working_directory))?
        .unwrap_or_else(|| {
            if selector.is_absolute() {
                selector.to_path_buf()
            } else {
                working_directory.join(selector)
            }
        });
    ensure_workspace_host(&requested, state, config, startup)
        .await
        .map(|_| ())
}

async fn stop(
    roots: &[PathBuf],
    selector: &std::path::Path,
    working_directory: &Path,
    state: &Path,
    config: Option<&Path>,
    force: bool,
) -> Result<()> {
    let scan_roots = roots.to_vec();
    let owned_selector = selector.to_path_buf();
    let owned_working_directory = working_directory.to_path_buf();
    let host = tokio::task::spawn_blocking(move || {
        let hosts = registered_hosts_in(&scan_roots)?;
        resolve_registered_host_from(&owned_selector, Some(&owned_working_directory), hosts)
    })
    .await?;
    let host = match host {
        Ok(host) => host,
        Err(error) => {
            if force {
                return stop_incompatible(selector, state, config)
                    .await
                    .map_err(|_| error);
            }
            anyhow::bail!(
                "{error}; this host may own live terminals or unsaved buffers; choose Force close explicitly or use a compatible client"
            );
        }
    };
    let shutdown = async {
        if force {
            force_shutdown_host(host.endpoint()).await
        } else {
            shutdown_host(host.endpoint()).await
        }
    };
    tokio::time::timeout(CONTROL_TIMEOUT, shutdown)
        .await
        .map_err(|_| anyhow::anyhow!("workspace host did not answer the stop request"))??;
    Ok(())
}

/// Renames the currently known form of one workspace. A running host owns its
/// persisted name and validates it against the live registry; a stopped row
/// has only its recent-history record to update.
async fn rename(
    roots: &[PathBuf],
    recents: Option<&Path>,
    selector: &Path,
    working_directory: &Path,
    name: &str,
    state: &Path,
    config: Option<&Path>,
) -> Result<()> {
    validate_host_name(name)?;
    let rows = refresh(roots, recents, state, None).await?;
    let project_root = resolve_known_workspace_from_rows(&rows, selector, Some(working_directory))?
        .with_context(|| format!("no session matches {}", selector.display()))?;
    let row = rows
        .iter()
        .find(|row| row.project_root == project_root)
        .with_context(|| format!("workspace {} is no longer known", project_root.display()))?;
    if row.running {
        let scan_roots = roots.to_vec();
        let selector = row.project_root.clone();
        let endpoint = tokio::task::spawn_blocking(move || {
            resolve_registered_host_from(&selector, None, registered_hosts_in(&scan_roots)?)
                .map(|host| host.endpoint().clone())
        })
        .await?
        .or_else(|_| resolve_workspace_endpoint(&row.project_root, state, config))?;
        rename_host(&endpoint, name).await
    } else {
        let recents = recents.context("workspace recent history is unavailable")?;
        rename_recent_workspace_in(recents, &row.project_root, name)
    }
}

/// Gives one workspace a number shortcut, or clears it.
///
/// Unlike a name, a number is never host state: it is this user's shortcut for
/// reaching a project, so a running host is neither consulted nor required and
/// a stopped workspace can be numbered exactly like a running one.
async fn number_workspace(
    roots: &[PathBuf],
    recents: Option<&Path>,
    selector: &Path,
    working_directory: &Path,
    number: Option<u8>,
    state: &Path,
) -> Result<Option<PathBuf>> {
    let rows = refresh(roots, recents, state, None).await?;
    let project_root = resolve_known_workspace_from_rows(&rows, selector, Some(working_directory))?
        .with_context(|| format!("no session matches {}", selector.display()))?;
    set_recent_workspace_number_in(recents, &project_root, number)
}

/// Stops the host a directory publishes when this build cannot speak to it.
/// Only a directory selector can be resolved this way: an ID or a name is a
/// registry lookup, and the registry is exactly what such a host is missing
/// from.
async fn stop_incompatible(selector: &Path, state: &Path, config: Option<&Path>) -> Result<()> {
    let endpoint = resolve_workspace_endpoint(selector, state, config)?;
    terminate_incompatible_host(&endpoint).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Path and name only, for the assertions written before numbering. A test
    /// about numbers reads [`RecentEntry::number`] directly instead.
    fn named(entries: Vec<RecentEntry>) -> Vec<(PathBuf, Option<String>)> {
        entries
            .into_iter()
            .map(|entry| (entry.project_root, entry.name))
            .collect()
    }

    fn entry(project_root: PathBuf, name: Option<String>) -> RecentEntry {
        RecentEntry::new(project_root, name, None)
    }

    #[test]
    fn abbreviated_ids_stay_six_characters_while_they_tell_workspaces_apart() {
        let ids = [
            "96ceecd6a1f66da1b4ef385dbb62328a",
            "7862cb247950d6d2435bd7545273d79f",
            "22b80e1b3b4ca1b84282af9e467983de",
            "d4db0b5604ea856609369870185fc36a",
        ];

        assert_eq!(abbreviated_id_width(ids), ABBREVIATED_WORKSPACE_ID);
    }

    #[test]
    fn abbreviated_ids_grow_only_far_enough_to_separate_a_shared_prefix() {
        let ids = [
            "aaaaaaaa1111111111111111111111ff",
            "aaaaaaaa2222222222222222222222ff",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ];

        assert_eq!(abbreviated_id_width(ids), 9);
    }

    #[test]
    fn ids_that_never_separate_abbreviate_to_their_whole_length() {
        assert_eq!(abbreviated_id_width(["abcdef0123", "abcdef0123"]), 10);
        assert_eq!(
            abbreviated_id_width(["abc", "abc"]),
            ABBREVIATED_WORKSPACE_ID
        );
        assert_eq!(abbreviated_id_width([]), ABBREVIATED_WORKSPACE_ID);
    }

    #[test]
    fn an_abbreviated_id_still_resolves_to_the_row_it_was_printed_from() {
        let ids = [
            "96ceecd6a1f66da1b4ef385dbb62328a",
            "7862cb247950d6d2435bd7545273d79f",
        ];
        let width = abbreviated_id_width(ids);
        let rows = ids
            .iter()
            .map(|id| WorkspaceRow {
                id: (*id).to_owned(),
                name: None,
                number: None,
                project_root: PathBuf::from(format!("/projects/{}", &id[..4])),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: None,
            })
            .collect::<Vec<_>>();

        for row in &rows {
            assert_eq!(
                resolve_known_workspace_from_rows(&rows, Path::new(&row.id[..width]), None)
                    .unwrap(),
                Some(row.project_root.clone())
            );
        }
    }

    #[tokio::test]
    async fn empty_injected_registry_refreshes_without_touching_user_state() {
        let root =
            std::env::temp_dir().join(format!("runyte-workspace-catalog-{}", std::process::id()));
        let (service, mut events) = WorkspaceService::spawn_with(
            vec![root],
            None,
            PathBuf::from("runyte-does-not-run"),
            PathBuf::from(".runyte"),
            None,
            None,
        );
        service.try_refresh(7).unwrap();
        let Some(WorkspaceEvent::Refreshed { generation, result }) = events.recv().await else {
            panic!("workspace service ended")
        };
        assert_eq!(generation, 7);
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn targeted_inspection_probes_a_live_host_absent_from_registry_and_recents() {
        use std::collections::HashMap;

        use crate::{
            protocol::FeatureGroup,
            workspace::transport::{LocalServer, PROTOCOL_VERSION, ServerEvent},
        };

        let root = unique_test_root("targeted-unregistered-inspection");
        let project = root.join("project");
        let runtime = std::env::temp_dir().join(format!(
            "ryt-ti-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(project.join(".runyte")).unwrap();
        fs::create_dir_all(&runtime).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let project = project.canonicalize().unwrap();
        let endpoint = LocalEndpoint::discover_with_runtime(
            &project.join(".runyte"),
            &project,
            Some(&runtime),
        )
        .unwrap();
        let mut server = match LocalServer::bind(&endpoint).await {
            Ok(server) => server,
            Err(error)
                if error.chain().any(|cause| {
                    cause
                        .downcast_ref::<io::Error>()
                        .is_some_and(|error| error.raw_os_error() == Some(libc::EPERM))
                }) =>
            {
                fs::remove_dir_all(runtime).unwrap();
                fs::remove_dir_all(root).unwrap();
                return;
            }
            Err(error) => panic!("cannot bind test transport: {error:#}"),
        };
        fs::remove_file(
            runtime
                .join("runyte/hosts")
                .join(format!("{}.json", endpoint.id())),
        )
        .unwrap();
        let host = tokio::spawn(async move {
            let mut clients = HashMap::new();
            while let Some(event) = server.recv().await {
                match event {
                    ServerEvent::Connected { id, responses, .. } => {
                        let _ = responses
                            .send(HostResponse::Welcome {
                                protocol: PROTOCOL_VERSION,
                                pid: std::process::id(),
                                features: vec![
                                    FeatureGroup::Control,
                                    FeatureGroup::Buffers,
                                    FeatureGroup::Wait,
                                ],
                                host_version: env!("CARGO_PKG_VERSION").to_owned(),
                            })
                            .await;
                        clients.insert(id, responses);
                    }
                    ServerEvent::Request {
                        id,
                        request: ClientRequest::Health,
                    } => {
                        if let Some(responses) = clients.get(&id) {
                            let _ = responses
                                .send(HostResponse::Health {
                                    protocol: PROTOCOL_VERSION,
                                    pid: std::process::id(),
                                    interactive_attached: false,
                                    unsaved_buffers: 3,
                                    pending_wait_requests: 0,
                                    live_terminals: 0,
                                    terminal_sessions: 0,
                                })
                                .await;
                        }
                    }
                    ServerEvent::Disconnected { id } => {
                        clients.remove(&id);
                    }
                    ServerEvent::ProtocolError { id, message } => {
                        if let Some(responses) = clients.get(&id) {
                            let _ = responses.send(HostResponse::Error { message }).await;
                        }
                    }
                    ServerEvent::Request { .. } => {}
                }
            }
        });
        let (service, mut events) = WorkspaceService::spawn_with(
            vec![root.join("empty-registry")],
            None,
            PathBuf::from("runyte-does-not-run"),
            PathBuf::from(".runyte"),
            None,
            Some(runtime.clone()),
        );
        service.try_inspect(9, project.clone()).unwrap();
        let Some(WorkspaceEvent::Inspected {
            generation,
            path,
            result,
        }) = events.recv().await
        else {
            panic!("workspace service ended")
        };
        assert_eq!(generation, 9);
        assert_eq!(path, project);
        let row = result.unwrap().expect("the unregistered host is running");
        assert!(row.running);
        assert_eq!(row.unsaved_buffers, Some(3));
        host.abort();
        let _ = host.await;
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn refresh_rejects_invalid_recents_without_rewriting_them() {
        let root = std::env::temp_dir().join(format!(
            "runyte-workspace-invalid-recents-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let registry = root.join("registry");
        let path = root.join("cache/workspaces.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        for invalid in [
            b"[{\"project_root_bytes\":[47]".as_slice(),
            b"not json".as_slice(),
            b"{\"project_root_bytes\":[]}".as_slice(),
        ] {
            fs::write(&path, invalid).unwrap();

            let error = refresh(
                std::slice::from_ref(&registry),
                Some(&path),
                Path::new(".runyte"),
                None,
            )
            .await
            .unwrap_err();

            assert!(
                error.downcast_ref::<serde_json::Error>().is_some(),
                "unexpected refresh error: {error:#}"
            );
            assert_eq!(fs::read(&path).unwrap(), invalid);
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn unusable_optional_recents_are_omitted_from_catalog_refresh() {
        let root = unique_test_root("unusable-recents");
        let cache_parent = root.join("cache-is-a-file");
        fs::create_dir_all(&root).unwrap();
        fs::write(&cache_parent, b"not a directory").unwrap();
        let cache_root = cache_parent.join("runyte");

        let recents = recent_file_in(Some(cache_root));
        assert_eq!(recents, None);
        assert!(
            refresh(&[], recents.as_deref(), Path::new(".runyte"), None)
                .await
                .unwrap()
                .is_empty()
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn usable_optional_recents_resolve_inside_the_cache_root() {
        let root = unique_test_root("usable-recents");
        let cache_root = root.join("cache");

        assert_eq!(
            recent_file_in(Some(cache_root.clone())),
            Some(cache_root.join("workspaces.json"))
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn stopped_workspace_id_matches_the_running_endpoint_identity() {
        let root = unique_test_root("stopped-id");
        let project_root = root.join("project");
        fs::create_dir_all(&project_root).unwrap();
        let project_root = project_root.canonicalize().unwrap();
        let recents = root.join("cache/workspaces.json");
        record_recent_workspace_in(&recents, &project_root).unwrap();

        let endpoint = super::super::transport::LocalEndpoint::new(
            &project_root.join(".runyte"),
            &project_root,
        )
        .unwrap();
        let rows = refresh(&[], Some(&recents), Path::new(".runyte"), None)
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert!(!rows[0].running);
        assert_eq!(rows[0].unsaved_buffers, None);
        assert_eq!(rows[0].interactive_attached, None);
        assert_eq!(rows[0].id, endpoint.id());
        assert_eq!(rows[0].id.len(), 32);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn refresh_lists_workspaces_from_most_recently_visited_to_oldest() {
        let root = unique_test_root("recency-order");
        let recents = root.join("cache/workspaces.json");
        let mut visited = Vec::new();
        for name in ["oldest", "middle", "newest"] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            let directory = directory.canonicalize().unwrap();
            record_recent_workspace_in(&recents, &directory).unwrap();
            visited.push(directory);
        }

        let rows = refresh(&[], Some(&recents), Path::new(".runyte"), None)
            .await
            .unwrap();

        visited.reverse();
        assert_eq!(
            rows.iter()
                .map(|row| row.project_root.clone())
                .collect::<Vec<_>>(),
            visited
        );

        // Visiting the oldest one again moves it to the top rather than
        // leaving the listing in whatever order the first visits produced.
        record_recent_workspace_in(&recents, visited.last().unwrap()).unwrap();
        let rows = refresh(&[], Some(&recents), Path::new(".runyte"), None)
            .await
            .unwrap();
        assert_eq!(rows[0].project_root, *visited.last().unwrap());

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn forgetting_a_workspace_drops_only_its_history_entry() {
        let root = unique_test_root("forget-recent");
        let recents = root.join("cache/workspaces.json");
        let kept = root.join("kept");
        let cleared = root.join("cleared");
        fs::create_dir_all(&kept).unwrap();
        fs::create_dir_all(&cleared).unwrap();
        let kept = kept.canonicalize().unwrap();
        let cleared = cleared.canonicalize().unwrap();
        record_recent_workspace_in(&recents, &kept).unwrap();
        record_recent_workspace_in(&recents, &cleared).unwrap();

        assert!(forget_recent_workspace_in(Some(&recents), &cleared).unwrap());
        assert_eq!(
            named(read_recents(Some(&recents)).unwrap()),
            vec![(kept.clone(), Some("kept".to_owned()))]
        );
        // The directory is untouched, so the workspace is exactly as reachable
        // as one that had never been opened.
        assert!(cleared.is_dir());
        assert!(
            refresh(&[], Some(&recents), Path::new(".runyte"), None)
                .await
                .unwrap()
                .iter()
                .all(|row| row.project_root == kept)
        );
        assert!(!forget_recent_workspace_in(Some(&recents), &cleared).unwrap());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_stopped_sessions_keeps_running_and_concurrent_history() {
        let root = unique_test_root("clear-stopped");
        let recents = root.join("cache/workspaces.json");
        let running = root.join("running");
        let stopped_one = root.join("stopped-one");
        let stopped_two = root.join("stopped-two");
        let concurrent = root.join("concurrent");
        for path in [&running, &stopped_one, &stopped_two, &concurrent] {
            fs::create_dir_all(path).unwrap();
        }
        for path in [&running, &stopped_one, &stopped_two] {
            record_recent_workspace_in(&recents, path).unwrap();
        }
        // A separate process may record another workspace after the listing
        // snapshot that selected the stopped entries.
        record_recent_workspace_in(&recents, &concurrent).unwrap();

        assert_eq!(
            clear_recent_workspaces_in(Some(&recents), &[stopped_one.clone(), stopped_two.clone()])
                .unwrap(),
            2
        );
        let paths = read_recents(Some(&recents))
            .unwrap()
            .into_iter()
            .map(|entry| entry.project_root)
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                concurrent.canonicalize().unwrap(),
                running.canonicalize().unwrap()
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stopped_workspace_rename_validates_and_preserves_its_identity() {
        let root = unique_test_root("rename-stopped");
        let recents = root.join("cache/workspaces.json");
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        record_recent_workspace_in(&recents, &first).unwrap();
        record_recent_workspace_in(&recents, &second).unwrap();

        rename_recent_workspace_in(&recents, &first, "archive").unwrap();
        let entries = read_recents(Some(&recents)).unwrap();
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.project_root == first.canonicalize().unwrap())
                .and_then(|entry| entry.name.as_deref()),
            Some("archive")
        );
        assert!(rename_recent_workspace_in(&recents, &second, "archive").is_err());
        assert!(rename_recent_workspace_in(&recents, &second, " bad ").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    /// `--session-rename` goes through the same `rename` the editor's session
    /// list uses, so a stopped session is renamed in visited history rather
    /// than refused for having no host to ask.
    #[tokio::test]
    async fn stopped_session_rename_updates_history_and_reports_unknown_selectors() {
        let root = unique_test_root("rename-stopped-selector");
        let registry = root.join("registry");
        let recents = root.join("cache/workspaces.json");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        record_recent_workspace_in(&recents, &workspace).unwrap();

        rename(
            std::slice::from_ref(&registry),
            Some(&recents),
            Path::new("project"),
            &root,
            "archive",
            Path::new(".runyte"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            read_recents(Some(&recents))
                .unwrap()
                .iter()
                .find(|entry| entry.project_root == workspace)
                .and_then(|entry| entry.name.as_deref()),
            Some("archive")
        );

        let error = rename(
            std::slice::from_ref(&registry),
            Some(&recents),
            Path::new("missing"),
            &root,
            "elsewhere",
            Path::new(".runyte"),
            None,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("no session matches"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn session_service_rename_resolves_id_name_and_editor_relative_directory() {
        let root = unique_test_root("session-rename-selectors");
        let registry = root.join("registry");
        let recents = root.join("cache/workspaces.json");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        record_recent_workspace_in(&recents, &workspace).unwrap();
        let row = refresh(
            std::slice::from_ref(&registry),
            Some(&recents),
            Path::new(".runyte"),
            None,
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
        assert!(row.name.is_some());

        let (service, mut events) = WorkspaceService::spawn_with(
            vec![registry],
            Some(recents.clone()),
            PathBuf::from("runyte-does-not-run"),
            PathBuf::from(".runyte"),
            None,
            None,
        );
        for (generation, selector, name) in [
            (1, PathBuf::from(row.id), "by-id"),
            (2, PathBuf::from("by-id"), "by-name"),
            (3, PathBuf::from("project"), "by-directory"),
        ] {
            service
                .try_rename(generation, selector.clone(), root.clone(), name.to_owned())
                .unwrap();
            let Some(WorkspaceEvent::Renamed {
                generation: completed,
                path,
                name: completed_name,
                result,
            }) = events.recv().await
            else {
                panic!("session rename service ended")
            };
            assert_eq!(completed, generation);
            assert_eq!(path, selector);
            assert_eq!(completed_name, name);
            result.unwrap();
        }
        assert_eq!(
            read_recents(Some(&recents)).unwrap()[0].name.as_deref(),
            Some("by-directory")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn session_service_start_resolves_id_name_and_editor_relative_directory() {
        let root = unique_test_root("session-start-selectors");
        let registry = root.join("registry");
        let recents = root.join("cache/workspaces.json");
        let workspace = root.join("project");
        fs::create_dir_all(workspace.join(".runyte")).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        record_recent_workspace_in(&recents, &workspace).unwrap();
        let row = refresh(
            std::slice::from_ref(&registry),
            Some(&recents),
            Path::new(".runyte"),
            None,
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
        let name = row.name.unwrap();

        let (service, mut events) = WorkspaceService::spawn_with(
            vec![registry],
            Some(recents),
            PathBuf::from("runyte-does-not-run"),
            PathBuf::from(".runyte"),
            None,
            None,
        );
        for (generation, selector) in [
            (1, PathBuf::from(row.id)),
            (2, PathBuf::from(name)),
            (3, PathBuf::from("project")),
        ] {
            service
                .try_start(generation, selector.clone(), root.clone())
                .unwrap();
            let Some(WorkspaceEvent::Started {
                generation: completed,
                path,
                result,
            }) = events.recv().await
            else {
                panic!("session start service ended")
            };
            assert_eq!(completed, generation);
            assert_eq!(path, selector);
            let error = result.unwrap_err();
            assert!(
                error.contains(workspace.to_string_lossy().as_ref()),
                "{error}"
            );
            assert!(!error.contains("workspace is unavailable"), "{error}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recents_are_deduplicated_most_recent_first_in_injected_storage() {
        let root = std::env::temp_dir().join(format!(
            "runyte-workspace-recents-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let path = root.join("cache/workspaces.json");

        record_recent_workspace_in(&path, &first).unwrap();
        record_recent_workspace_in(&path, &second).unwrap();
        record_recent_workspace_in(&path, &first).unwrap();

        assert_eq!(
            named(read_recents(Some(&path)).unwrap()),
            vec![
                (first.canonicalize().unwrap(), Some("first".to_owned())),
                (second.canonicalize().unwrap(), Some("second".to_owned()))
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_recents_receive_unique_directory_names_and_keep_them_when_revisited() {
        let root = unique_test_root("default-names");
        let path = root.join("cache/workspaces.json");
        let workspaces =
            ["one", "two", "three", "four"].map(|parent| root.join(parent).join("runyte"));
        for workspace in &workspaces {
            fs::create_dir_all(workspace).unwrap();
        }

        // Model an existing workspace recorded by a version before automatic
        // names, then visit two new workspaces with the same directory name.
        let first = workspaces[0].canonicalize().unwrap();
        update_recents(&path, |paths| paths.push(entry(first, None))).unwrap();
        for workspace in &workspaces[1..3] {
            record_recent_workspace_in(&path, workspace).unwrap();
        }
        assert_eq!(
            named(read_recents(Some(&path)).unwrap()),
            vec![
                (
                    workspaces[2].canonicalize().unwrap(),
                    Some("runyte-3".to_owned()),
                ),
                (
                    workspaces[1].canonicalize().unwrap(),
                    Some("runyte-2".to_owned()),
                ),
                (
                    workspaces[0].canonicalize().unwrap(),
                    Some("runyte".to_owned()),
                ),
            ]
        );

        record_recent_workspace_in(&path, &workspaces[1]).unwrap();
        let second = workspaces[1].canonicalize().unwrap();
        assert_eq!(
            named(read_recents(Some(&path)).unwrap())[0],
            (second.clone(), Some("runyte-2".to_owned()))
        );

        assert!(forget_recent_workspace_in(Some(&path), &second).unwrap());
        record_recent_workspace_in(&path, &workspaces[3]).unwrap();
        assert_eq!(
            named(read_recents(Some(&path)).unwrap())[0],
            (
                workspaces[3].canonicalize().unwrap(),
                Some("runyte-2".to_owned()),
            )
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recent_names_fill_unnamed_running_rows_without_overriding_explicit_names() {
        let unnamed_root = PathBuf::from("/workspace/unnamed");
        let explicit_root = PathBuf::from("/workspace/explicit");
        let mut rows = vec![
            WorkspaceRow {
                id: "11111111111111111111111111111111".to_owned(),
                name: None,
                number: None,
                project_root: unnamed_root.clone(),
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(false),
            },
            WorkspaceRow {
                id: "22222222222222222222222222222222".to_owned(),
                name: Some("chosen".to_owned()),
                number: None,
                project_root: explicit_root.clone(),
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(false),
            },
        ];

        apply_recent_names(
            &mut rows,
            &[
                entry(unnamed_root, Some("default".to_owned())),
                entry(explicit_root, Some("stale-default".to_owned())),
            ],
        );

        assert_eq!(rows[0].name.as_deref(), Some("default"));
        assert_eq!(rows[1].name.as_deref(), Some("chosen"));
    }

    #[test]
    fn default_names_are_valid_bounded_host_names() {
        assert_eq!(
            unique_default_workspace_name(Path::new("/workspace/ \n "), &[]),
            "workspace"
        );

        let long = format!("{}-tail", "ż".repeat(40));
        let first = unique_default_workspace_name(&Path::new("/workspace").join(&long), &[]);
        let second = unique_default_workspace_name(
            &Path::new("/another").join(&long),
            &[entry(
                PathBuf::from("/workspace/first"),
                Some(first.clone()),
            )],
        );
        assert!(first.len() <= MAX_HOST_NAME_BYTES);
        assert!(second.len() <= MAX_HOST_NAME_BYTES);
        assert!(second.ends_with("-2"));
        assert!(first.is_char_boundary(first.len()));
        assert!(second.is_char_boundary(second.len()));
    }

    #[test]
    fn known_selector_paths_use_the_supplied_editor_directory_and_ids_and_names_stay_exact() {
        let root = std::env::temp_dir().join(format!(
            "runyte-workspace-selector-directory-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let editor_directory = root.join("current/nested");
        let relative_target = root.join("current/project");
        let named_target = root.join("named");
        let id_target = root.join("identified");
        for directory in [
            &editor_directory,
            &relative_target,
            &named_target,
            &id_target,
        ] {
            fs::create_dir_all(directory).unwrap();
        }
        let editor_directory = editor_directory.canonicalize().unwrap();
        assert_ne!(std::env::current_dir().unwrap(), editor_directory);
        let relative_target = relative_target.canonicalize().unwrap();
        let named_target = named_target.canonicalize().unwrap();
        let id_target = id_target.canonicalize().unwrap();
        let rows = vec![
            WorkspaceRow {
                id: "11111111111111111111111111111111".to_owned(),
                name: None,
                number: None,
                project_root: relative_target.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: None,
            },
            WorkspaceRow {
                id: "22222222222222222222222222222222".to_owned(),
                name: Some("archive".to_owned()),
                number: None,
                project_root: named_target.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: None,
            },
            WorkspaceRow {
                id: "abcdef0123456789abcdef0123456789".to_owned(),
                name: None,
                number: None,
                project_root: id_target.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: None,
            },
        ];

        assert_eq!(
            resolve_known_workspace_from_rows(
                &rows,
                Path::new("../project"),
                Some(&editor_directory)
            )
            .unwrap(),
            Some(relative_target)
        );
        assert_eq!(
            resolve_known_workspace_from_rows(&rows, Path::new("archive"), Some(&editor_directory))
                .unwrap(),
            Some(named_target)
        );
        assert_eq!(
            resolve_known_workspace_from_rows(
                &rows,
                Path::new("ABCDEF0123456789ABCDEF0123456789"),
                Some(&editor_directory)
            )
            .unwrap(),
            Some(id_target.clone())
        );
        assert_eq!(
            resolve_known_workspace_from_rows(&rows, Path::new("abcdef"), Some(&editor_directory))
                .unwrap(),
            Some(id_target)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_merge_preserves_a_workspace_recorded_after_its_snapshot() {
        let root = unique_test_root("refresh-race");
        let first = root.join("first");
        let recorded_during_refresh = root.join("recorded-during-refresh");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&recorded_during_refresh).unwrap();
        let path = root.join("cache/workspaces.json");

        record_recent_workspace_in(&path, &first).unwrap();
        let first = first.canonicalize().unwrap();
        let snapshot = read_recents(Some(&path)).unwrap();
        let stale_rows = vec![WorkspaceRow {
            id: "11111111111111111111111111111111".to_owned(),
            name: Some("named-first".to_owned()),
            number: None,
            project_root: first.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: Some(false),
        }];

        // Model a second process recording a workspace while refresh is
        // inspecting the hosts represented by `stale_rows`.
        record_recent_workspace_in(&path, &recorded_during_refresh).unwrap();
        merge_refreshed_names(&path, &snapshot, &stale_rows).unwrap();

        assert_eq!(
            named(read_recents(Some(&path)).unwrap()),
            vec![
                (
                    recorded_during_refresh.canonicalize().unwrap(),
                    Some("recorded-during-refresh".to_owned()),
                ),
                (first, Some("named-first".to_owned())),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_merge_preserves_a_concurrently_changed_existing_name() {
        let root = unique_test_root("refresh-name-race");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let path = root.join("cache/workspaces.json");
        record_recent_workspace_in(&path, &workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let snapshot = read_recents(Some(&path)).unwrap();
        let stale_rows = vec![WorkspaceRow {
            id: "11111111111111111111111111111111".to_owned(),
            name: Some("stale-inspection".to_owned()),
            number: None,
            project_root: workspace.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: Some(false),
        }];

        update_recents(&path, |paths| {
            paths[0].name = Some("concurrent-name".to_owned());
        })
        .unwrap();
        merge_refreshed_names(&path, &snapshot, &stale_rows).unwrap();

        assert_eq!(
            named(read_recents(Some(&path)).unwrap()),
            vec![(workspace, Some("concurrent-name".to_owned()))]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recents_lock_secures_a_preexisting_broad_lock_file() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let root = unique_test_root("lock-mode");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let path = root.join("cache/workspaces.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lock_path = path.with_extension("lock");
        fs::write(&lock_path, []).unwrap();
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o666)).unwrap();

        record_recent_workspace_in(&path, &workspace).unwrap();

        assert_eq!(fs::metadata(&lock_path).unwrap().mode() & 0o777, 0o600);
        fs::remove_dir_all(root).unwrap();
    }

    const LOCK_HELPER_RECENTS: &str = "RUNYTE_TEST_RECENTS_LOCK_PATH";
    const LOCK_HELPER_WORKSPACE: &str = "RUNYTE_TEST_RECENTS_LOCK_WORKSPACE";
    const LOCK_HELPER_READY: &str = "RUNYTE_TEST_RECENTS_LOCK_BLOCKED";

    #[test]
    #[ignore = "subprocess helper for recents_writers_are_serialized_between_processes"]
    fn recents_lock_process_helper() {
        use std::io::Write;

        let Some(path) = std::env::var_os(LOCK_HELPER_RECENTS).map(PathBuf::from) else {
            return;
        };
        let workspace = PathBuf::from(
            std::env::var_os(LOCK_HELPER_WORKSPACE).expect("helper workspace was not supplied"),
        );
        assert!(
            RecentFileLock::try_acquire(&path).unwrap().is_none(),
            "the parent process should hold the recents lock"
        );
        println!("{LOCK_HELPER_READY}");
        std::io::stdout().flush().unwrap();
        record_recent_workspace_in(&path, &workspace).unwrap();
    }

    #[test]
    fn recents_writers_are_serialized_between_processes() {
        use std::{
            io::BufRead,
            process::{Command, Stdio},
        };

        let root = unique_test_root("process-lock");
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let path = root.join("cache/workspaces.json");
        record_recent_workspace_in(&path, &first).unwrap();

        let lock = RecentFileLock::acquire(&path).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("recents_lock_process_helper")
            .arg("--ignored")
            .arg("--nocapture")
            .env(LOCK_HELPER_RECENTS, &path)
            .env(LOCK_HELPER_WORKSPACE, &second)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(
                stdout.read_line(&mut line).unwrap(),
                0,
                "lock helper exited before observing the held lock"
            );
            if line.contains(LOCK_HELPER_READY) {
                break;
            }
        }

        assert_eq!(
            named(read_recents(Some(&path)).unwrap()),
            vec![(first.canonicalize().unwrap(), Some("first".to_owned()))],
            "the blocked child must not publish its row before lock release"
        );
        drop(lock);

        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "lock helper failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            named(read_recents(Some(&path)).unwrap()),
            vec![
                (second.canonicalize().unwrap(), Some("second".to_owned())),
                (first.canonicalize().unwrap(), Some("first".to_owned())),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspaces_are_numbered_in_the_order_they_are_first_recorded() {
        let root = unique_test_root("number-order");
        let path = root.join("cache/workspaces.json");
        let workspaces = (0..3)
            .map(|index| {
                let workspace = root.join(format!("project-{index}"));
                fs::create_dir_all(&workspace).unwrap();
                record_recent_workspace_in(&path, &workspace).unwrap();
                workspace.canonicalize().unwrap()
            })
            .collect::<Vec<_>>();

        // The file is ordered most-recently-visited first, so the numbers run
        // the other way. That is the point: a number follows the workspace,
        // not its position.
        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.project_root.clone(), entry.number))
                .collect::<Vec<_>>(),
            vec![
                (workspaces[2].clone(), Some(3)),
                (workspaces[1].clone(), Some(2)),
                (workspaces[0].clone(), Some(1)),
            ]
        );

        // Revisiting moves a workspace to the front and leaves its number be.
        record_recent_workspace_in(&path, &workspaces[0]).unwrap();
        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(entries[0].project_root, workspaces[0]);
        assert_eq!(entries[0].number, Some(1));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_the_first_nine_workspaces_receive_a_number() {
        let root = unique_test_root("number-limit");
        let path = root.join("cache/workspaces.json");
        let mut workspaces = Vec::new();
        for index in 0..(MAX_WORKSPACE_NUMBER as usize + 2) {
            let workspace = root.join(format!("project-{index}"));
            fs::create_dir_all(&workspace).unwrap();
            record_recent_workspace_in(&path, &workspace).unwrap();
            workspaces.push(workspace.canonicalize().unwrap());
        }

        let entries = read_recents(Some(&path)).unwrap();
        let numbered = entries
            .iter()
            .filter(|entry| entry.number.is_some())
            .count();
        assert_eq!(numbered, MAX_WORKSPACE_NUMBER as usize);
        // The tenth and eleventh are reachable by name or path instead.
        for overflow in &workspaces[MAX_WORKSPACE_NUMBER as usize..] {
            let entry = entries
                .iter()
                .find(|entry| &entry.project_root == overflow)
                .unwrap();
            assert_eq!(entry.number, None);
            assert!(entry.name.is_some());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_a_workspace_frees_its_number_for_the_next_one() {
        let root = unique_test_root("number-free");
        let path = root.join("cache/workspaces.json");
        let first = root.join("first");
        let second = root.join("second");
        let third = root.join("third");
        for workspace in [&first, &second] {
            fs::create_dir_all(workspace).unwrap();
            record_recent_workspace_in(&path, workspace).unwrap();
        }
        let first = first.canonicalize().unwrap();

        assert!(forget_recent_workspace_in(Some(&path), &first).unwrap());
        fs::create_dir_all(&third).unwrap();
        record_recent_workspace_in(&path, &third).unwrap();

        let entries = read_recents(Some(&path)).unwrap();
        let third = third.canonicalize().unwrap();
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.project_root == third)
                .unwrap()
                .number,
            Some(1),
            "the freed number is the lowest available one"
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.project_root == second.canonicalize().unwrap())
                .unwrap()
                .number,
            Some(2),
            "a workspace that kept its place keeps its number"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn assigning_a_taken_number_swaps_the_pair() {
        let root = unique_test_root("number-swap");
        let path = root.join("cache/workspaces.json");
        let first = root.join("first");
        let second = root.join("second");
        for workspace in [&first, &second] {
            fs::create_dir_all(workspace).unwrap();
            record_recent_workspace_in(&path, workspace).unwrap();
        }
        let first = first.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();

        let displaced = set_recent_workspace_number_in(Some(&path), &second, Some(1)).unwrap();
        assert_eq!(displaced.as_deref(), Some(first.as_path()));

        let entries = read_recents(Some(&path)).unwrap();
        let number_of = |target: &Path| {
            entries
                .iter()
                .find(|entry| entry.project_root == target)
                .unwrap()
                .number
        };
        assert_eq!(number_of(&second), Some(1));
        assert_eq!(
            number_of(&first),
            Some(2),
            "the displaced workspace takes the number the other gave up"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_number_can_be_cleared_and_the_range_is_enforced() {
        let root = unique_test_root("number-clear");
        let path = root.join("cache/workspaces.json");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).unwrap();
        record_recent_workspace_in(&path, &workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();

        assert!(
            set_recent_workspace_number_in(Some(&path), &workspace, None)
                .unwrap()
                .is_none()
        );
        assert_eq!(recorded_number(&path, &workspace), None);

        assert!(
            set_recent_workspace_number_in(Some(&path), &workspace, Some(MAX_WORKSPACE_NUMBER + 1))
                .is_err()
        );
        assert!(set_recent_workspace_number_in(Some(&path), &workspace, Some(0)).is_err());
        assert_eq!(recorded_number(&path, &workspace), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_catalog_written_before_numbering_is_numbered_on_the_next_visit() {
        let root = unique_test_root("number-backfill");
        let path = root.join("cache/workspaces.json");
        let older = root.join("older");
        let newer = root.join("newer");
        for workspace in [&older, &newer] {
            fs::create_dir_all(workspace).unwrap();
        }
        // Exactly what an older release wrote: names but no numbers.
        update_recents(&path, |paths| {
            paths.push(entry(
                newer.canonicalize().unwrap(),
                Some("newer".to_owned()),
            ));
            paths.push(entry(
                older.canonicalize().unwrap(),
                Some("older".to_owned()),
            ));
        })
        .unwrap();
        assert_eq!(recorded_number(&path, &older.canonicalize().unwrap()), None);

        let visited = root.join("visited");
        fs::create_dir_all(&visited).unwrap();
        record_recent_workspace_in(&path, &visited).unwrap();

        // The backfill runs most-recently-visited first, because a catalog
        // without numbers has no creation order left to recover.
        assert_eq!(
            recorded_number(&path, &newer.canonicalize().unwrap()),
            Some(1)
        );
        assert_eq!(
            recorded_number(&path, &older.canonicalize().unwrap()),
            Some(2)
        );
        assert_eq!(
            recorded_number(&path, &visited.canonicalize().unwrap()),
            Some(3),
            "the newcomer claims the next free number rather than a taken one"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_duplicate_number_in_a_hand_edited_catalog_is_repaired_on_read() {
        let root = unique_test_root("number-duplicate");
        let path = root.join("cache/workspaces.json");
        let first = root.join("first");
        let second = root.join("second");
        for workspace in [&first, &second] {
            fs::create_dir_all(workspace).unwrap();
        }
        update_recents(&path, |paths| {
            paths.push(RecentEntry::new(
                first.canonicalize().unwrap(),
                Some("first".to_owned()),
                Some(1),
            ));
            paths.push(RecentEntry::new(
                second.canonicalize().unwrap(),
                Some("second".to_owned()),
                Some(1),
            ));
        })
        .unwrap();

        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(entries[0].number, Some(1), "the first claim stands");
        assert_eq!(
            entries[1].number, None,
            "the duplicate is dropped rather than letting one key select two rows"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// The number the catalog at `path` records for `project_root`.
    fn recorded_number(path: &Path, project_root: &Path) -> Option<u8> {
        read_recents(Some(path))
            .unwrap()
            .into_iter()
            .find(|entry| entry.project_root == project_root)
            .and_then(|entry| entry.number)
    }

    fn unique_test_root(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "runyte-workspace-{label}-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RecentWorkspace {
    project_root_bytes: Vec<u8>,
    #[serde(default)]
    name: Option<String>,
    /// Absent in catalogs written before workspaces were numbered, which is
    /// why it defaults rather than failing the whole file: an older history
    /// stays readable and is numbered on the next listing.
    #[serde(default)]
    number: Option<u8>,
}

/// The largest number a workspace can carry.
///
/// A number is a shortcut pressed as one key in the session manager, so the
/// range is exactly the digits `1`-`9`. A tenth remembered workspace is
/// reached by name or path instead of by number.
pub const MAX_WORKSPACE_NUMBER: u8 = 9;

/// One remembered workspace: where it is, what it is called, and the digit
/// that selects it in the session manager.
///
/// The recents file is ordered most-recently-visited first, so an entry's
/// position is deliberately not its number. A number is claimed once, when
/// the workspace is first recorded, and then stays put even as the workspace
/// moves up and down the history. That is what makes it a shortcut somebody
/// can learn rather than a label that moves under them between two visits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentEntry {
    pub project_root: PathBuf,
    pub name: Option<String>,
    /// `1` through [`MAX_WORKSPACE_NUMBER`], or `None` when every number was
    /// already taken as this workspace was first recorded.
    pub number: Option<u8>,
}

impl RecentEntry {
    fn new(project_root: PathBuf, name: Option<String>, number: Option<u8>) -> Self {
        Self {
            project_root,
            name,
            number,
        }
    }
}

fn recent_file() -> Option<PathBuf> {
    recent_file_in(external_open::cache_root())
}

/// Finds usable optional storage for stopped-workspace history.
///
/// Running-host discovery has its own runtime registry fallback and must not
/// become unavailable merely because the regenerable cache root cannot be
/// created. Once a cache directory is usable, errors from the recents file
/// itself remain observable so malformed history is never silently erased.
fn recent_file_in(cache_root: Option<PathBuf>) -> Option<PathBuf> {
    let root = cache_root?;
    prepare_recents_parent(&root).ok()?;
    Some(root.join("workspaces.json"))
}

/// Remembers a workspace after startup so stopped hosts remain discoverable.
pub fn record_recent_workspace(project_root: &Path) -> Result<Option<RecordedWorkspace>> {
    let Some(path) = recent_file() else {
        return Ok(None);
    };
    record_recent_workspace_name_in(&path, project_root)
}

/// The number the catalog currently records for one workspace, if any.
///
/// A direct read rather than a listing: the status line needs this before any
/// refresh has run, and asking for the whole inventory to learn one digit
/// would make drawing the first frame wait on scanning every workspace.
pub fn recorded_workspace_number(project_root: &Path) -> Option<u8> {
    let path = recent_file()?;
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    read_recents(Some(&path))
        .ok()?
        .into_iter()
        .find(|entry| entry.project_root == canonical)
        .and_then(|entry| entry.number)
}

/// Gives one workspace a number shortcut, or takes its number away.
///
/// A number identifies exactly one workspace, so assigning one that another
/// workspace already holds swaps the pair rather than leaving a duplicate or
/// quietly unnumbering the other. Both keep a shortcut, and the returned path
/// lets the caller say where the old one went instead of leaving somebody to
/// discover it by pressing the key.
fn set_recent_workspace_number_in(
    path: Option<&Path>,
    project_root: &Path,
    number: Option<u8>,
) -> Result<Option<PathBuf>> {
    if let Some(number) = number {
        anyhow::ensure!(
            (1..=MAX_WORKSPACE_NUMBER).contains(&number),
            "a session number must be between 1 and {MAX_WORKSPACE_NUMBER}"
        );
    }
    let Some(path) = path else {
        return Ok(None);
    };
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut displaced = None;
    update_recents_result(path, |paths| {
        assign_missing_default_workspace_names(paths);
        assign_missing_default_workspace_numbers(paths);
        let Some(index) = paths
            .iter()
            .position(|entry| entry.project_root == canonical)
        else {
            anyhow::bail!("that workspace is not in the visited history")
        };
        let vacated = paths[index].number;
        if let Some(number) = number
            && let Some(holder) = paths
                .iter()
                .position(|entry| entry.number == Some(number) && entry.project_root != canonical)
        {
            // The swap hands the asking workspace's old number over, which is
            // why an unnumbered one leaves the other without a number rather
            // than duplicating the one it just gave away.
            paths[holder].number = vacated;
            displaced = Some(paths[holder].project_root.clone());
        }
        paths[index].number = number;
        Ok(())
    })?;
    Ok(displaced)
}

#[cfg(test)]
fn record_recent_workspace_in(path: &Path, project_root: &Path) -> Result<()> {
    record_recent_workspace_name_in(path, project_root).map(drop)
}

fn record_recent_workspace_name_in(
    path: &Path,
    project_root: &Path,
) -> Result<Option<RecordedWorkspace>> {
    let canonical = project_root.canonicalize()?;
    let mut recorded = None;
    update_recents(path, |paths| {
        // Older catalogs predate automatic names and numbers. Claim both for
        // those rows before inserting a new workspace so an established
        // directory keeps the unsuffixed form and the low number, and the
        // newcomer receives `-2` and the next number up.
        assign_missing_default_workspace_names(paths);
        assign_missing_default_workspace_numbers(paths);
        let previous = paths
            .iter()
            .find(|entry| entry.project_root == canonical)
            .map(|entry| (entry.name.clone(), entry.number));
        paths.retain(|entry| entry.project_root != canonical);
        let (previous_name, previous_number) = previous.unzip();
        let name = previous_name
            .flatten()
            .unwrap_or_else(|| unique_default_workspace_name(&canonical, paths.as_slice()));
        // Revisiting keeps the number this workspace already answered to.
        // Only a genuinely new record claims one, which is what makes the
        // default assignment order the order workspaces were created in.
        let number = previous_number
            .flatten()
            .or_else(|| lowest_free_workspace_number(paths));
        recorded = Some(RecordedWorkspace {
            name: name.clone(),
            number,
        });
        paths.insert(0, RecentEntry::new(canonical, Some(name), number));
        // Truncation drops the least recently visited tail, which can free a
        // number. The next new workspace claims it; the survivors keep theirs.
        paths.truncate(RECENT_LIMIT);
    })?;
    Ok(recorded)
}

/// What recording a visit settled about a workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedWorkspace {
    pub name: String,
    pub number: Option<u8>,
}

fn assign_missing_default_workspace_names(paths: &mut [RecentEntry]) {
    for index in 0..paths.len() {
        if paths[index].name.is_some() {
            continue;
        }
        let project_root = paths[index].project_root.clone();
        paths[index].name = Some(unique_default_workspace_name(&project_root, paths));
    }
}

/// Claims the lowest free number for every remembered workspace without one.
///
/// Older catalogs predate numbering entirely, and a workspace recorded while
/// all nine were taken carries none. Both are answered here, on the way to a
/// listing, so numbering never depends on having been present for a
/// particular release.
///
/// Numbers are meant to follow the order workspaces were created, and a new
/// record claims its number at exactly that moment. A catalog written before
/// numbering existed has no creation order to recover -- it is ordered by
/// recency and nothing else -- so this one-time backfill numbers those rows
/// most-recently-visited first and says so rather than inventing a history.
fn assign_missing_default_workspace_numbers(paths: &mut [RecentEntry]) {
    for index in 0..paths.len() {
        if paths[index].number.is_some() {
            continue;
        }
        paths[index].number = lowest_free_workspace_number(paths);
    }
}

/// The smallest number no remembered workspace holds, if any is left.
fn lowest_free_workspace_number(paths: &[RecentEntry]) -> Option<u8> {
    (1..=MAX_WORKSPACE_NUMBER)
        .find(|candidate| paths.iter().all(|entry| entry.number != Some(*candidate)))
}

/// Derives a stable catalog name from the workspace directory and adds the
/// first free numeric suffix when another recorded workspace already owns it.
fn unique_default_workspace_name(project_root: &Path, paths: &[RecentEntry]) -> String {
    let raw_base = project_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| project_root.display().to_string());
    let sanitized = raw_base
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() {
                '-'
            } else {
                character
            }
        })
        .collect::<String>();
    let sanitized = if sanitized.is_empty() {
        "workspace"
    } else {
        &sanitized
    };
    let base = truncate_utf8(sanitized, MAX_HOST_NAME_BYTES).to_owned();
    let available = |candidate: &str| {
        paths
            .iter()
            .all(|entry| entry.name.as_deref() != Some(candidate))
    };
    if available(&base) {
        return base;
    }
    (2_u64..)
        .map(|suffix| {
            let suffix = format!("-{suffix}");
            let prefix = truncate_utf8(&base, MAX_HOST_NAME_BYTES - suffix.len());
            format!("{prefix}{suffix}")
        })
        .find(|candidate| available(candidate))
        .expect("an unbounded numeric suffix has an available value")
}

fn truncate_utf8(value: &str, maximum_bytes: usize) -> &str {
    let mut end = value.len().min(maximum_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Drops a workspace from the visited history, so a stopped one stops being
/// listed. Answers whether history held it at all.
///
/// Only the per-user recents record is removed. Nothing under the project's own
/// state root is touched, so a workspace cleared here is exactly as reachable as
/// one that was never opened: naming it starts a host there again.
fn forget_recent_workspace_in(path: Option<&Path>, project_root: &Path) -> Result<bool> {
    let Some(path) = path else {
        return Ok(false);
    };
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut removed = false;
    update_recents(path, |paths| {
        let before = paths.len();
        paths.retain(|entry| entry.project_root != canonical);
        removed = paths.len() != before;
    })?;
    Ok(removed)
}

/// Removes exactly the named stopped rows from recent history in one locked
/// update. A workspace recorded concurrently at another path is preserved.
fn clear_recent_workspaces_in(path: Option<&Path>, stopped: &[PathBuf]) -> Result<usize> {
    let Some(path) = path else {
        return Ok(0);
    };
    let stopped = stopped
        .iter()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .collect::<Vec<_>>();
    let mut removed = 0;
    update_recents(path, |paths| {
        let before = paths.len();
        paths.retain(|entry| !stopped.contains(&entry.project_root));
        removed = before - paths.len();
    })?;
    Ok(removed)
}

fn rename_recent_workspace_in(path: &Path, project_root: &Path, name: &str) -> Result<()> {
    validate_host_name(name)?;
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    update_recents_result(path, |paths| {
        anyhow::ensure!(
            paths.iter().all(|entry| entry.project_root == canonical
                || entry.name.as_deref() != Some(name)),
            "session name {name:?} is already in use"
        );
        let entry = paths
            .iter_mut()
            .find(|entry| entry.project_root == canonical)
            .with_context(|| {
                format!(
                    "workspace {} is not in recent history",
                    project_root.display()
                )
            })?;
        entry.name = Some(name.to_owned());
        Ok(())
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
fn merge_refreshed_names(
    path: &Path,
    snapshot: &[RecentEntry],
    rows: &[WorkspaceRow],
) -> Result<()> {
    update_recents(path, |paths| {
        for entry in paths {
            let Some(snapshot_entry) = snapshot
                .iter()
                .find(|candidate| candidate.project_root == entry.project_root)
            else {
                continue;
            };
            if entry.name != snapshot_entry.name {
                continue;
            }
            let Some(refreshed_name) = rows
                .iter()
                .find(|row| row.project_root == entry.project_root)
                .and_then(|row| row.name.as_ref())
            else {
                continue;
            };
            entry.name = Some(refreshed_name.clone());
        }
    })
}

fn update_recents(path: &Path, update: impl FnOnce(&mut Vec<RecentEntry>)) -> Result<()> {
    update_recents_result(path, |paths| {
        update(paths);
        Ok(())
    })
}

fn update_recents_result(
    path: &Path,
    update: impl FnOnce(&mut Vec<RecentEntry>) -> Result<()>,
) -> Result<()> {
    let lock = RecentFileLock::acquire(path)?;
    let mut paths = read_recents(Some(path))?;
    update(&mut paths)?;
    write_recents(path, &paths, &lock)
}

/// A dedicated advisory lock for the recents file, not for the cache or user
/// directory around it. The kernel releases `flock` when this descriptor is
/// closed, including process exit after a crash; the persistent lock file is
/// inert and can safely be reused by the next process.
struct RecentFileLock(fs::File);

impl RecentFileLock {
    fn acquire(recents: &Path) -> Result<Self> {
        Self::acquire_with_operation(recents, libc::LOCK_EX)?.ok_or_else(|| {
            anyhow::anyhow!("blocking workspace recents lock unexpectedly was unavailable")
        })
    }

    fn acquire_with_operation(recents: &Path, operation: libc::c_int) -> Result<Option<Self>> {
        use std::os::{
            fd::AsRawFd,
            unix::fs::{OpenOptionsExt, PermissionsExt},
        };

        let Some(parent) = recents.parent() else {
            anyhow::bail!("workspace recents path has no parent")
        };
        prepare_recents_parent(parent)?;
        let path = recents.with_extension("lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .with_context(|| format!("cannot open workspace recents lock {}", path.display()))?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .with_context(|| format!("cannot secure workspace recents lock {}", path.display()))?;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
                return Ok(Some(Self(file)));
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if operation & libc::LOCK_NB != 0 && error.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(error)
                .with_context(|| format!("cannot lock workspace recents file {}", path.display()));
        }
    }

    #[cfg(test)]
    fn try_acquire(recents: &Path) -> Result<Option<Self>> {
        Self::acquire_with_operation(recents, libc::LOCK_EX | libc::LOCK_NB)
    }
}

impl Drop for RecentFileLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn prepare_recents_parent(parent: &Path) -> Result<()> {
    fs::create_dir_all(parent)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn write_recents(path: &Path, paths: &[RecentEntry], _lock: &RecentFileLock) -> Result<()> {
    let entries = paths
        .iter()
        .map(|entry| RecentWorkspace {
            project_root_bytes: encode_path(&entry.project_root),
            name: entry.name.clone(),
            number: entry.number,
        })
        .collect::<Vec<_>>();
    let Some(parent) = path.parent() else {
        anyhow::bail!("workspace recents path has no parent")
    };
    prepare_recents_parent(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, serde_json::to_vec_pretty(&entries)?)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn read_recents(path: Option<&Path>) -> Result<Vec<RecentEntry>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let entries: Vec<RecentWorkspace> = serde_json::from_slice(&bytes)?;
    let mut entries = entries
        .into_iter()
        .map(|entry| {
            RecentEntry::new(
                decode_path(entry.project_root_bytes),
                entry.name,
                entry
                    .number
                    .filter(|number| (1..=MAX_WORKSPACE_NUMBER).contains(number)),
            )
        })
        .filter(|entry| entry.project_root.is_dir())
        .take(RECENT_LIMIT)
        .collect::<Vec<_>>();
    // A number identifies one workspace, so a file hand-edited into holding a
    // duplicate is repaired on the way in rather than reaching a listing where
    // one digit would select whichever row happened to be first.
    let mut claimed = Vec::new();
    for entry in &mut entries {
        match entry.number {
            Some(number) if claimed.contains(&number) => entry.number = None,
            Some(number) => claimed.push(number),
            None => {}
        }
    }
    Ok(entries)
}
