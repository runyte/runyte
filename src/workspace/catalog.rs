// SPDX-License-Identifier: MPL-2.0

//! Asynchronous discovery and lifecycle work for switchable workspaces.
//!
//! Registry access and control connections may block or time out, so the
//! editor submits bounded requests and accepts owned completion events. The
//! picker never performs filesystem or transport work on the render path.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use tokio::sync::{mpsc, watch};

use crate::git::read_workspace_git_facts;
use crate::project_root;
use crate::protocol::{ClientRequest, HostResponse};

use super::{
    SessionPreview,
    lifecycle::{
        await_host_stopped, connect_control, force_shutdown_host, rename_host,
        resolve_registered_host_from, resolve_workspace_endpoint,
        resolve_workspace_endpoint_with_runtime, shutdown_host, terminate_incompatible_host,
    },
    transport::{
        LocalEndpoint, RegisteredHost, all_registry_roots, registered_hosts_in, registry_roots,
        workspace_id,
    },
};

use super::catalog_values::WorkspaceSelection;
pub use super::catalog_values::{
    ABBREVIATED_WORKSPACE_ID, DestinationInventory, WorkspaceEvent, WorkspaceRow,
    abbreviated_id_width,
};
use super::catalog_values::{
    apply_recent_activity, apply_recent_names, assign_running_workspace_numbers,
    merge_assigned_numbers, order_workspace_rows, validate_destination_inventory,
};
use super::recent_history::*;
pub use super::recent_history::{
    MAX_WORKSPACE_NUMBER, RecentEntry, RecordedWorkspace, ensure_recent_workspace,
    record_recent_workspace, record_workspace_activity, recorded_workspace_number,
};
#[cfg(test)]
use super::session_name::MAX_HOST_NAME_BYTES;
use super::session_name::{normalize_session_name, validate_host_name};
#[cfg(test)]
use crate::native_path::encode_path;
#[cfg(test)]
use std::{fs, io};

const REQUEST_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 16;
const CONTROL_TIMEOUT: Duration = Duration::from_millis(500);
const NATIVE_SELECTION_UNAVAILABLE: &str =
    "native publication selection is unavailable in the Unix session service";

#[derive(Debug)]
enum WorkspaceRequest {
    Refresh {
        generation: u64,
    },
    Poll,
    Observe {
        attention: bool,
    },
    DirectoryWorktrees {
        generation: u64,
        path: PathBuf,
    },
    Inventory {
        generation: u64,
        selection: WorkspaceSelection,
    },
    Inspect {
        generation: u64,
        path: PathBuf,
    },
    Stop {
        generation: u64,
        target: WorkspaceRequestTarget,
        working_directory: PathBuf,
        force: bool,
    },
    Forget {
        generation: u64,
        path: PathBuf,
    },
    Rename {
        generation: u64,
        target: WorkspaceRequestTarget,
        working_directory: PathBuf,
        name: String,
    },
    Number {
        generation: u64,
        target: WorkspaceRequestTarget,
        working_directory: PathBuf,
        number: Option<u8>,
    },
}

#[derive(Debug)]
enum WorkspaceRequestTarget {
    UserSelector(PathBuf),
    SelectedRow(WorkspaceSelection),
}

impl WorkspaceRequestTarget {
    fn into_parts(self) -> (PathBuf, Option<WorkspaceSelection>) {
        match self {
            Self::UserSelector(selector) => (selector, None),
            Self::SelectedRow(selection) => {
                (selection.project_root().to_path_buf(), Some(selection))
            }
        }
    }
}

#[derive(Clone)]
pub struct WorkspaceServiceHandle {
    requests: mpsc::Sender<WorkspaceRequest>,
    previews: watch::Sender<Option<WorkspacePreviewRequest>>,
}

#[derive(Clone, Debug)]
struct WorkspacePreviewRequest {
    generation: u64,
    selection: WorkspaceSelection,
}

impl WorkspaceServiceHandle {
    pub fn try_directory_worktrees(
        &self,
        generation: u64,
        path: PathBuf,
    ) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::DirectoryWorktrees { generation, path })
            .map_err(|_| "session service is unavailable or busy")
    }

    pub fn try_inventory(
        &self,
        generation: u64,
        selection: WorkspaceSelection,
    ) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Inventory {
                generation,
                selection,
            })
            .map_err(|_| "session service is unavailable or busy")
    }

    pub fn try_refresh(&self, generation: u64) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Refresh { generation })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    /// Quiet strip observation: one bounded request, with no Git metadata work.
    pub fn try_observe(&self, attention: bool) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Observe { attention })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_poll(&self) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Poll)
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
    pub fn try_preview(
        &self,
        generation: u64,
        selection: WorkspaceSelection,
    ) -> Result<(), &'static str> {
        self.previews
            .send(Some(WorkspacePreviewRequest {
                generation,
                selection,
            }))
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
                target: WorkspaceRequestTarget::UserSelector(selector),
                working_directory,
                force,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_stop_selected(
        &self,
        generation: u64,
        selection: WorkspaceSelection,
        working_directory: PathBuf,
        force: bool,
    ) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Stop {
                generation,
                target: WorkspaceRequestTarget::SelectedRow(selection),
                working_directory,
                force,
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
        let name = normalize_session_name(&name);
        self.requests
            .try_send(WorkspaceRequest::Rename {
                generation,
                target: WorkspaceRequestTarget::UserSelector(selector),
                working_directory,
                name,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_rename_selected(
        &self,
        generation: u64,
        selection: WorkspaceSelection,
        working_directory: PathBuf,
        name: String,
    ) -> Result<(), &'static str> {
        let name = normalize_session_name(&name);
        self.requests
            .try_send(WorkspaceRequest::Rename {
                generation,
                target: WorkspaceRequestTarget::SelectedRow(selection),
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
                target: WorkspaceRequestTarget::UserSelector(selector),
                working_directory,
                number,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "session service is unavailable",
            })
    }

    pub fn try_number_selected(
        &self,
        generation: u64,
        selection: WorkspaceSelection,
        working_directory: PathBuf,
        number: Option<u8>,
    ) -> Result<(), &'static str> {
        self.requests
            .try_send(WorkspaceRequest::Number {
                generation,
                target: WorkspaceRequestTarget::SelectedRow(selection),
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
        state: PathBuf,
        config: Option<PathBuf>,
    ) -> (WorkspaceServiceHandle, mpsc::Receiver<WorkspaceEvent>) {
        Self::spawn_with(registry_roots(), recent_file(), state, config, None)
    }

    fn spawn_with(
        roots: Vec<PathBuf>,
        recents: Option<PathBuf>,
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
                let Some(WorkspacePreviewRequest {
                    generation,
                    selection,
                }) = request
                else {
                    continue;
                };
                let path = selection.project_root().to_path_buf();
                let result = if selection.publication_key().is_some() {
                    Err(NATIVE_SELECTION_UNAVAILABLE.to_owned())
                } else {
                    preview_session(&path, &preview_state)
                        .await
                        .map_err(|error| format!("{error:#}"))
                };
                if preview_events
                    .send(WorkspaceEvent::Previewed {
                        generation,
                        path,
                        selection,
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
                    WorkspaceRequest::DirectoryWorktrees { generation, path } => {
                        let result = tokio::task::spawn_blocking(move || {
                            use crate::git::GitProvider;
                            let git = crate::git::GitCliProvider::new("git");
                            match git.discover(&path).map_err(|error| error.to_string())? {
                                Some(repository) => git
                                    .worktrees(&repository)
                                    .map(|rows| {
                                        rows.into_iter()
                                            .filter(|row| !row.bare && !row.missing)
                                            .map(|row| row.path)
                                            .take(256)
                                            .collect()
                                    })
                                    .map_err(|error| error.to_string()),
                                None => Ok(Vec::new()),
                            }
                        })
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|result| result);
                        WorkspaceEvent::DirectoryWorktrees { generation, result }
                    }
                    WorkspaceRequest::Inventory {
                        generation,
                        selection,
                    } => {
                        let path = selection.project_root().to_path_buf();
                        let result = if selection.publication_key().is_some() {
                            Err(NATIVE_SELECTION_UNAVAILABLE.to_owned())
                        } else {
                            read_destination_inventory(&path, &state, runtime.as_deref())
                                .await
                                .map_err(|error| format!("{error:#}"))
                        };
                        WorkspaceEvent::Inventory {
                            generation,
                            path,
                            selection,
                            result,
                        }
                    }
                    WorkspaceRequest::Observe { attention } => WorkspaceEvent::Observed {
                        result: match tokio::time::timeout(
                            Duration::from_secs(2),
                            refresh_options(
                                &roots,
                                recents.as_deref(),
                                &state,
                                runtime.as_deref(),
                                false,
                                false,
                                attention,
                            ),
                        )
                        .await
                        {
                            Ok(result) => result.map_err(|error| format!("{error:#}")),
                            Err(_) => {
                                Err("session observation timed out; host health is unknown"
                                    .to_owned())
                            }
                        },
                    },
                    WorkspaceRequest::Poll => WorkspaceEvent::Polled {
                        result: refresh(&roots, recents.as_deref(), &state, runtime.as_deref())
                            .await
                            .map_err(|error| format!("{error:#}")),
                    },
                    WorkspaceRequest::Inspect { generation, path } => {
                        let result = inspect_workspace_target(
                            &path,
                            recents.as_deref(),
                            &state,
                            runtime.as_deref(),
                        )
                        .await
                        .map_err(|error| format!("{error:#}"));
                        WorkspaceEvent::Inspected {
                            generation,
                            path,
                            result: Box::new(result),
                        }
                    }
                    WorkspaceRequest::Stop {
                        generation,
                        target,
                        working_directory,
                        force,
                    } => {
                        let (selector, selection) = target.into_parts();
                        let result = if selection
                            .as_ref()
                            .is_some_and(|selection| selection.publication_key().is_some())
                        {
                            Err(NATIVE_SELECTION_UNAVAILABLE.to_owned())
                        } else {
                            stop(
                                &roots,
                                &selector,
                                &working_directory,
                                &state,
                                config.as_deref(),
                                runtime.as_deref(),
                                force,
                            )
                            .await
                            .map_err(|error| format!("{error:#}"))
                        };
                        WorkspaceEvent::Stopped {
                            generation,
                            selector,
                            selection,
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
                        target,
                        working_directory,
                        name,
                    } => {
                        let (selector, selection) = target.into_parts();
                        let result = if selection
                            .as_ref()
                            .is_some_and(|selection| selection.publication_key().is_some())
                        {
                            Err(NATIVE_SELECTION_UNAVAILABLE.to_owned())
                        } else {
                            rename(
                                &roots,
                                recents.as_deref(),
                                &selector,
                                &working_directory,
                                &name,
                                &state,
                                config.as_deref(),
                            )
                            .await
                            .map_err(|error| format!("{error:#}"))
                        };
                        WorkspaceEvent::Renamed {
                            generation,
                            path: selector,
                            selection,
                            name,
                            result,
                        }
                    }
                    WorkspaceRequest::Number {
                        generation,
                        target,
                        working_directory,
                        number,
                    } => {
                        let (selector, selection) = target.into_parts();
                        let result = if selection
                            .as_ref()
                            .is_some_and(|selection| selection.publication_key().is_some())
                        {
                            Err(NATIVE_SELECTION_UNAVAILABLE.to_owned())
                        } else {
                            number_workspace(
                                &roots,
                                recents.as_deref(),
                                &selector,
                                &working_directory,
                                number,
                                &state,
                            )
                            .await
                            .map_err(|error| format!("{error:#}"))
                        };
                        WorkspaceEvent::Numbered {
                            generation,
                            path: selector,
                            selection,
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

/// Reads lifecycle and visit information for navigation without collecting
/// Git details for every workspace merely to choose the next attachment.
pub async fn known_workspaces_for_navigation(state: &Path) -> Result<Vec<WorkspaceRow>> {
    refresh_options(
        &registry_roots(),
        recent_file().as_deref(),
        state,
        None,
        true,
        false,
        true,
    )
    .await
}

/// Enumerates the current namespace's recent history together with every live
/// host in the explicit owner-wide inventory.
///
/// Recent history remains namespace-local: a stopped workspace has no host to
/// discover outside the namespace that recorded it.
pub async fn known_workspaces_all_namespaces(state: &Path) -> Result<Vec<WorkspaceRow>> {
    let roots = all_registry_roots()?;
    refresh_with_name_persistence(&roots, recent_file().as_deref(), state, None, false).await
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
    refresh_with_name_persistence(roots, recents, state, runtime, true).await
}

/// Builds the workspace inventory, optionally persisting names learned from
/// the scanned hosts back into this namespace's recent history.
///
/// Explicit owner-wide listing disables that write: a host in another
/// namespace may deliberately use a different name for the same project.
async fn refresh_with_name_persistence(
    roots: &[PathBuf],
    recents: Option<&Path>,
    state: &Path,
    runtime: Option<&Path>,
    persist_host_names: bool,
) -> Result<Vec<WorkspaceRow>> {
    refresh_options(
        roots,
        recents,
        state,
        runtime,
        persist_host_names,
        true,
        true,
    )
    .await
}

async fn refresh_options(
    roots: &[PathBuf],
    recents: Option<&Path>,
    state: &Path,
    runtime: Option<&Path>,
    persist_host_names: bool,
    describe_git: bool,
    attention: bool,
) -> Result<Vec<WorkspaceRow>> {
    let scan_roots = roots.to_vec();
    let recent_path = recents.map(Path::to_path_buf);
    let (hosts, mut remembered) = tokio::task::spawn_blocking(move || {
        Ok::<_, anyhow::Error>((
            registered_hosts_in(&scan_roots)?,
            read_recents(recent_path.as_deref())?,
        ))
    })
    .await??;
    let recent_snapshot = remembered.clone();
    assign_missing_default_workspace_names(&mut remembered);
    // Numbers and names come from every remembered workspace, including one
    // whose directory has gone while its host keeps running. Only the rows a
    // listing offers to open are drawn from the ones still on disk.
    let openable = listable_recents(remembered.clone());
    let mut rows = Vec::with_capacity(hosts.len());
    for host in hosts {
        rows.push(inspect_host_with_attention(host, attention).await);
    }
    apply_recent_names(&mut rows, &remembered);
    for RecentEntry {
        project_root,
        name,
        number: _,
        last_active_unix_seconds,
        number_declined: _,
        number_pinned: _,
    } in openable
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
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id,
            name,
            number: None,
            last_active_unix_seconds,
            project_root,
            running: false,
            incompatible_protocol: None,
            // A history-only row has no running process that can answer
            // host-owned state. Listings leave those columns blank.
            unsaved_buffers: None,
            pending_wait_requests: None,
            plugin_jobs: None,
            activity_leases: None,
            activities: Vec::new(),
            live_terminals: None,
            terminal_sessions: None,
            terminal_line_activity_unix_seconds: None,
            interactive_attached: None,
            open_buffers: None,
            git: None,
            missing_directory: false,
        });
    }
    apply_recent_activity(&mut rows, &remembered);
    // Git facts and directory existence are filesystem reads, and a listing
    // can hold hundreds of rows, so they are gathered once here rather than
    // recomputed by whatever draws them.
    let described = tokio::task::spawn_blocking(move || {
        for row in &mut rows {
            row.missing_directory = !row.project_root.is_dir();
            if describe_git {
                row.git = read_workspace_git_facts(&row.project_root);
            }
        }
        rows
    })
    .await?;
    let mut rows = described;
    // Most recently visited first, which is the order the recents file already
    // holds. A workspace absent from that history is a running host whose
    // record was pruned or never written; it sorts after the remembered ones,
    // by path, so the listing stays stable between refreshes.
    //
    // This is not the order the listing is shown in: it is the order numbering
    // reads, so a digit two running sessions both prefer goes to the more
    // recently visited one and the same sessions always number the same way.
    rows.sort_by_cached_key(|row| {
        let recency = recent_snapshot
            .iter()
            .position(|entry| entry.project_root == row.project_root)
            .unwrap_or(usize::MAX);
        (recency, row.project_root.clone())
    });
    assign_running_workspace_numbers(&mut rows, &remembered);
    // The digits are settled, so the listing can be put in the order it is
    // read in: pinned sessions first.
    order_workspace_rows(&mut rows);
    if persist_host_names && let Some(path) = recents {
        let path = path.to_path_buf();
        let refreshed_rows = rows.clone();
        tokio::task::spawn_blocking(move || {
            merge_refreshed_rows(&path, &recent_snapshot, &refreshed_rows)
        })
        .await??;
    }
    Ok(rows)
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
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: host.id,
            // Nothing can ask a host of another protocol anything, so its
            // buffer counts stay unknown rather than being reported as zero.
            name: host.name.or(name),
            // Filled from the catalog once every row exists.
            number: None,
            last_active_unix_seconds: None,
            project_root: host.project_root,
            running: true,
            incompatible_protocol: Some(host.protocol),
            unsaved_buffers: None,
            pending_wait_requests: None,
            plugin_jobs: None,
            activity_leases: None,
            activities: Vec::new(),
            live_terminals: None,
            terminal_sessions: None,
            terminal_line_activity_unix_seconds: None,
            interactive_attached: None,
            open_buffers: None,
            git: None,
            missing_directory: false,
        });
    }
    let inspection = inspect_endpoint(&endpoint).await;
    Some(WorkspaceRow {
        publication_key: None,
        unread_terminals: None,
        terminal_bell: None,
        id: host.id,
        name: host.name.or(name),
        number: None,
        last_active_unix_seconds: None,
        project_root: host.project_root,
        running: true,
        incompatible_protocol: None,
        unsaved_buffers: inspection.unsaved_buffers,
        pending_wait_requests: inspection.pending_wait_requests,
        plugin_jobs: inspection.plugin_jobs,
        activity_leases: inspection.activity_leases,
        activities: inspection.activities,
        live_terminals: inspection.live_terminals,
        terminal_sessions: inspection.terminal_sessions,
        terminal_line_activity_unix_seconds: inspection.terminal_line_activity_unix_seconds,
        interactive_attached: inspection.interactive_attached,
        open_buffers: inspection.open_buffers,
        git: None,
        missing_directory: false,
    })
}

/// Inspects one exact workspace endpoint for a destructive operation.
///
/// Unlike the catalog refresh, this does not depend on registry or recent
/// history membership and does not turn an endpoint it cannot verify into a
/// stopped row. Endpoint artifacts with an unverifiable owner fail closed.
async fn inspect_workspace_target(
    project_root: &Path,
    recents: Option<&Path>,
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
    // A host never answers a number, and the digit is how somebody knows which
    // session a confirmation is about to stop, so it is read from the catalog
    // here as it is for every listed row. A catalog that cannot be read leaves
    // the row unnumbered rather than failing an inspection that is about
    // whether the host is safe to touch.
    let number = read_recents(recents)
        .unwrap_or_default()
        .into_iter()
        .find(|entry| entry.project_root == project_root)
        .and_then(|entry| entry.number);
    if !host.speaks_current_protocol() {
        return Ok(Some(WorkspaceRow {
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: host.id,
            name: host.name,
            number,
            last_active_unix_seconds: None,
            project_root,
            running: true,
            incompatible_protocol: Some(host.protocol),
            unsaved_buffers: None,
            pending_wait_requests: None,
            plugin_jobs: None,
            activity_leases: None,
            activities: Vec::new(),
            live_terminals: None,
            terminal_sessions: None,
            terminal_line_activity_unix_seconds: None,
            interactive_attached: None,
            open_buffers: None,
            git: None,
            missing_directory: false,
        }));
    }
    let inspection = inspect_endpoint_strict(&endpoint).await?;
    Ok(Some(WorkspaceRow {
        publication_key: None,
        unread_terminals: None,
        terminal_bell: None,
        id: host.id,
        name: host.name,
        number,
        last_active_unix_seconds: None,
        project_root,
        running: true,
        incompatible_protocol: None,
        unsaved_buffers: Some(inspection.unsaved_buffers),
        pending_wait_requests: Some(inspection.pending_wait_requests),
        plugin_jobs: Some(inspection.plugin_jobs),
        activity_leases: Some(inspection.activity_leases),
        activities: inspection.activities,
        live_terminals: Some(inspection.live_terminals),
        terminal_sessions: Some(inspection.terminal_sessions),
        terminal_line_activity_unix_seconds: inspection.terminal_line_activity_unix_seconds,
        interactive_attached: Some(inspection.interactive_attached),
        open_buffers: Some(inspection.open_buffers),
        git: None,
        missing_directory: false,
    }))
}

async fn read_destination_inventory(
    path: &Path,
    state: &Path,
    runtime: Option<&Path>,
) -> Result<DestinationInventory> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let endpoint = published_endpoint(path, state, runtime)?;
        let mut client = connect_control(&endpoint).await?;
        client.send(&ClientRequest::DestinationInventory).await?;
        match client.recv().await? {
            Some(HostResponse::DestinationInventory {
                incarnation,
                entries,
                truncated,
            }) => validate_destination_inventory(incarnation, entries, truncated),
            Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                anyhow::bail!(message)
            }
            Some(_) => anyhow::bail!("this host does not support destination inventories"),
            None => anyhow::bail!("host closed before returning its destinations"),
        }
    })
    .await
    .context("destination inventory timed out")?
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

async fn inspect_host_with_attention(host: RegisteredHost, attention: bool) -> WorkspaceRow {
    if !host.speaks_current_protocol() {
        return WorkspaceRow {
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: host.id,
            name: host.name,
            number: None,
            last_active_unix_seconds: None,
            project_root: host.project_root,
            running: true,
            incompatible_protocol: Some(host.protocol),
            unsaved_buffers: None,
            pending_wait_requests: None,
            plugin_jobs: None,
            activity_leases: None,
            activities: Vec::new(),
            live_terminals: None,
            terminal_sessions: None,
            terminal_line_activity_unix_seconds: None,
            interactive_attached: None,
            open_buffers: None,
            git: None,
            missing_directory: false,
        };
    }
    let inspection = if attention {
        inspect_endpoint(host.endpoint()).await
    } else {
        HostInspection::default()
    };
    WorkspaceRow {
        publication_key: None,
        unread_terminals: inspection.unread_terminals,
        terminal_bell: inspection.terminal_bell,
        id: host.id,
        name: host.name,
        number: None,
        last_active_unix_seconds: None,
        project_root: host.project_root,
        running: true,
        incompatible_protocol: None,
        unsaved_buffers: inspection.unsaved_buffers,
        pending_wait_requests: inspection.pending_wait_requests,
        plugin_jobs: inspection.plugin_jobs,
        activity_leases: inspection.activity_leases,
        activities: inspection.activities,
        live_terminals: inspection.live_terminals,
        terminal_sessions: inspection.terminal_sessions,
        terminal_line_activity_unix_seconds: inspection.terminal_line_activity_unix_seconds,
        interactive_attached: inspection.interactive_attached,
        open_buffers: inspection.open_buffers,
        git: None,
        missing_directory: false,
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
    unread_terminals: Option<usize>,
    terminal_bell: Option<bool>,
    unsaved_buffers: Option<usize>,
    open_buffers: Option<usize>,
    pending_wait_requests: Option<usize>,
    plugin_jobs: Option<usize>,
    activity_leases: Option<usize>,
    activities: Vec<crate::service_health::ActivityLeaseHealth>,
    live_terminals: Option<usize>,
    terminal_sessions: Option<usize>,
    terminal_line_activity_unix_seconds: Option<u64>,
    interactive_attached: Option<bool>,
}

struct StrictHostInspection {
    unsaved_buffers: usize,
    open_buffers: usize,
    pending_wait_requests: usize,
    plugin_jobs: usize,
    activity_leases: usize,
    activities: Vec<crate::service_health::ActivityLeaseHealth>,
    live_terminals: usize,
    terminal_sessions: usize,
    terminal_line_activity_unix_seconds: Option<u64>,
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
                open_buffers,
                pending_wait_requests,
                plugin_jobs,
                activity_leases,
                activities,
                live_terminals,
                terminal_sessions,
                terminal_line_activity_unix_seconds,
                ..
            }) => Ok(StrictHostInspection {
                unsaved_buffers,
                open_buffers,
                pending_wait_requests,
                plugin_jobs,
                activity_leases,
                activities: activities.into_iter().map(Into::into).collect(),
                live_terminals,
                terminal_sessions,
                terminal_line_activity_unix_seconds,
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
            unread_terminals,
            terminal_bell,
            interactive_attached: attached,
            unsaved_buffers: unsaved,
            open_buffers,
            pending_wait_requests,
            plugin_jobs,
            activity_leases,
            activities,
            live_terminals,
            terminal_sessions,
            terminal_line_activity_unix_seconds,
            ..
        }) = client.recv().await?
        {
            result.unread_terminals = Some(unread_terminals);
            result.terminal_bell = Some(terminal_bell);
            result.interactive_attached = Some(attached);
            result.unsaved_buffers = Some(unsaved);
            result.open_buffers = Some(open_buffers);
            result.pending_wait_requests = Some(pending_wait_requests);
            result.plugin_jobs = Some(plugin_jobs);
            result.activity_leases = Some(activity_leases);
            result.activities = activities.into_iter().map(Into::into).collect();
            result.live_terminals = Some(live_terminals);
            result.terminal_sessions = Some(terminal_sessions);
            result.terminal_line_activity_unix_seconds = terminal_line_activity_unix_seconds;
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

async fn stop(
    roots: &[PathBuf],
    selector: &std::path::Path,
    working_directory: &Path,
    state: &Path,
    config: Option<&Path>,
    runtime: Option<&Path>,
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
    enum StopTarget {
        // The process is carried alongside the endpoint so the wait afterwards
        // can tell the host that was stopped from a replacement started at the
        // same workspace while it was going away.
        Current {
            endpoint: LocalEndpoint,
            pid: u32,
        },
        Incompatible {
            endpoint: LocalEndpoint,
            protocol: u32,
        },
    }
    let target = match host {
        Ok(host) if host.speaks_current_protocol() => StopTarget::Current {
            endpoint: host.endpoint().clone(),
            pid: host.pid,
        },
        Ok(host) => StopTarget::Incompatible {
            endpoint: host.endpoint().clone(),
            protocol: host.protocol,
        },
        Err(registry_error) => {
            let requested = if selector.is_absolute() {
                selector.to_path_buf()
            } else {
                working_directory.join(selector)
            };
            let endpoint = match resolve_workspace_endpoint_with_runtime(
                &requested, state, config, runtime,
            ) {
                Ok(endpoint) => endpoint,
                Err(_) => {
                    anyhow::bail!(
                        "{registry_error}; this host may own live terminals or unsaved buffers; choose Force close explicitly or use a compatible client"
                    )
                }
            };
            let published = endpoint
                .published_host()?
                .with_context(|| format!("no running session matches {}", selector.display()))?;
            if published.speaks_current_protocol() {
                StopTarget::Current {
                    endpoint,
                    pid: published.pid,
                }
            } else {
                StopTarget::Incompatible {
                    endpoint,
                    protocol: published.protocol,
                }
            }
        }
    };
    match target {
        StopTarget::Current { endpoint, pid } => {
            tokio::time::timeout(CONTROL_TIMEOUT, async {
                if force {
                    force_shutdown_host(&endpoint).await
                } else {
                    shutdown_host(&endpoint).await
                }
            })
            .await
            .map_err(|_| anyhow::anyhow!("workspace host did not answer the stop request"))??;
            // The host acknowledges before it exits, and every client answers
            // a stop by listing again. Returning on the acknowledgement alone
            // hands that listing a session still holding its endpoint, so the
            // manager shows the row it was just asked to close as running
            // until something refreshes it later.
            await_host_stopped(&endpoint, pid).await
        }
        StopTarget::Incompatible { endpoint, .. } if force => {
            terminate_incompatible_host(&endpoint).await.map(drop)
        }
        StopTarget::Incompatible { protocol, .. } => anyhow::bail!(
            "workspace host protocol {protocol} is incompatible with client protocol {}; this host may own live terminals or unsaved buffers; choose Force close explicitly or use a compatible client",
            super::transport::PROTOCOL_VERSION
        ),
    }
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
    let name = normalize_session_name(name);
    validate_host_name(&name)?;
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
        rename_host(&endpoint, &name).await
    } else {
        let recents = recents.context("workspace recent history is unavailable")?;
        rename_recent_workspace_in(recents, &row.project_root, &name)
    }
}

/// Gives one workspace a number shortcut, or clears it.
///
/// Unlike a name, a number is never host state: it is this user's shortcut for
/// reaching a running session. The inventory is re-read before changing it so
/// a session that stopped after its action menu opened is refused.
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
    anyhow::ensure!(
        rows.iter()
            .any(|row| row.project_root == project_root && row.running),
        "cannot renumber a stopped session"
    );
    set_recent_workspace_number_in(recents, &project_root, number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestRuntimeRoot;

    /// Path and name only, for the assertions written before numbering. A test
    /// about numbers reads [`RecentEntry::number`] directly instead.
    fn named(entries: Vec<RecentEntry>) -> Vec<(PathBuf, Option<String>)> {
        entries
            .into_iter()
            .map(|entry| (entry.project_root, entry.name))
            .collect()
    }

    fn entry(project_root: PathBuf, name: Option<String>) -> RecentEntry {
        RecentEntry::new(project_root, name, None, None)
    }

    #[test]
    fn workspace_service_handle_distinguishes_full_and_closed_queues() {
        let path = PathBuf::from("/workspace");
        let invoke_all = |handle: &WorkspaceServiceHandle| {
            [
                handle.try_refresh(1),
                handle.try_poll(),
                handle.try_inspect(2, path.clone()),
                handle.try_stop(3, path.clone(), path.clone(), false),
                handle.try_forget(4, path.clone()),
                handle.try_rename(5, path.clone(), path.clone(), "  named  ".to_owned()),
                handle.try_number(6, path.clone(), path.clone(), Some(7)),
            ]
        };

        let (full_tx, _full_rx) = mpsc::channel(1);
        full_tx
            .try_send(WorkspaceRequest::Refresh { generation: 0 })
            .unwrap();
        let (preview_tx, _preview_rx) = watch::channel(None);
        let full = WorkspaceServiceHandle {
            requests: full_tx,
            previews: preview_tx,
        };
        for result in invoke_all(&full) {
            assert_eq!(result, Err("session service queue is full"));
        }

        let (closed_tx, closed_rx) = mpsc::channel(1);
        drop(closed_rx);
        let (closed_preview_tx, closed_preview_rx) = watch::channel(None);
        drop(closed_preview_rx);
        let closed = WorkspaceServiceHandle {
            requests: closed_tx,
            previews: closed_preview_tx,
        };
        for result in invoke_all(&closed) {
            assert_eq!(result, Err("session service is unavailable"));
        }
        assert_eq!(
            closed.try_preview(7, WorkspaceSelection::project_only(path)),
            Err("session preview service is unavailable")
        );
    }

    #[tokio::test]
    async fn unix_worker_rejects_native_selected_keys_before_path_resolution() {
        let root = TestRuntimeRoot::new("selected-native-key").unwrap();
        let mut row = numbering_row(&root.join("project"), true);
        row.publication_key = Some(crate::workspace::PublicationKey::for_test(b"native-key"));
        let selection = row.selection();
        let (service, mut events) =
            WorkspaceService::spawn_with(Vec::new(), None, root.join("state"), None, None);
        service.try_preview(1, selection.clone()).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkspaceEvent::Previewed {
            selection: returned,
            result: Err(error),
            ..
        } = event
        else {
            panic!("native-key preview should be refused")
        };
        assert_eq!(returned, selection);
        assert_eq!(error, NATIVE_SELECTION_UNAVAILABLE);

        service.try_inventory(2, selection.clone()).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkspaceEvent::Inventory {
            selection: returned,
            result: Err(error),
            ..
        } = event
        else {
            panic!("native-key inventory should be refused")
        };
        assert_eq!(returned, selection);
        assert_eq!(error, NATIVE_SELECTION_UNAVAILABLE);

        service
            .try_stop_selected(3, selection.clone(), root.to_path_buf(), true)
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkspaceEvent::Stopped {
            selection: Some(returned),
            result: Err(error),
            ..
        } = event
        else {
            panic!("native-key stop should be refused")
        };
        assert_eq!(returned, selection);
        assert_eq!(error, NATIVE_SELECTION_UNAVAILABLE);

        service
            .try_rename_selected(4, selection.clone(), root.to_path_buf(), "new".to_owned())
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkspaceEvent::Renamed {
            selection: Some(returned),
            result: Err(error),
            ..
        } = event
        else {
            panic!("native-key rename should be refused")
        };
        assert_eq!(returned, selection);
        assert_eq!(error, NATIVE_SELECTION_UNAVAILABLE);

        service
            .try_number_selected(5, selection.clone(), root.to_path_buf(), Some(1))
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkspaceEvent::Numbered {
            selection: Some(returned),
            result: Err(error),
            ..
        } = event
        else {
            panic!("native-key number should be refused")
        };
        assert_eq!(returned, selection);
        assert_eq!(error, NATIVE_SELECTION_UNAVAILABLE);
        assert!(!root.join("state").exists());
        assert!(!root.join("project").exists());
    }

    #[test]
    fn an_abbreviated_id_still_resolves_to_the_row_it_was_printed_from() {
        let ids = [
            "658471a65ca7c48244bef5867d3e80bc",
            "fe973e03d42260785cd4cb9386a8168d",
        ];
        let width = abbreviated_id_width(ids);
        let rows = ids
            .iter()
            .map(|id| WorkspaceRow {
                publication_key: None,
                unread_terminals: None,
                terminal_bell: None,
                id: (*id).to_owned(),
                name: None,
                number: None,
                last_active_unix_seconds: None,
                project_root: PathBuf::from(format!("/projects/{}", &id[..4])),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                plugin_jobs: None,
                activity_leases: None,
                activities: Vec::new(),
                live_terminals: None,
                terminal_sessions: None,
                terminal_line_activity_unix_seconds: None,
                interactive_attached: None,
                open_buffers: None,
                git: None,
                missing_directory: false,
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
        let (service, mut events) =
            WorkspaceService::spawn_with(vec![root], None, PathBuf::from(".runyte"), None, None);
        service.try_refresh(7).unwrap();
        let Some(WorkspaceEvent::Refreshed { generation, result }) = events.recv().await else {
            panic!("workspace service ended")
        };
        assert_eq!(generation, 7);
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn targeted_directory_operations_reach_a_live_host_absent_from_the_registry() {
        use std::collections::HashMap;

        use crate::{
            protocol::FeatureGroup,
            workspace::transport::{LocalServer, PROTOCOL_VERSION, ServerEvent},
        };

        let root = unique_test_root("targeted-unregistered-inspection");
        let project = root.join("project");
        let runtime = unique_test_root("targeted-runtime");
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
                drop(runtime);
                drop(root);
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
                                    unread_terminals: 0,
                                    terminal_bell: false,
                                    protocol: PROTOCOL_VERSION,
                                    pid: std::process::id(),
                                    interactive_attached: false,
                                    unsaved_buffers: 3,
                                    open_buffers: 9,
                                    pending_wait_requests: 0,
                                    plugin_jobs: 2,
                                    activity_leases: 1,
                                    activities: vec![crate::protocol::ActivityLeaseHealth {
                                        owner: "watcher".into(),
                                        title: "Remote watcher".into(),
                                        state: crate::protocol::ActivityLeaseState::Cancelling,
                                    }]
                                    .into_boxed_slice(),
                                    live_terminals: 0,
                                    terminal_sessions: 0,
                                    terminal_line_activity_unix_seconds: None,
                                })
                                .await;
                        }
                    }
                    ServerEvent::Request {
                        id,
                        request: ClientRequest::Shutdown,
                    } => {
                        if let Some(responses) = clients.get(&id) {
                            let _ = responses.send(HostResponse::ShuttingDown).await;
                        }
                        break;
                    }
                    ServerEvent::Disconnected { id } => {
                        clients.remove(&id);
                    }
                    ServerEvent::ProtocolError { id, message } => {
                        if let Some(responses) = clients.get(&id) {
                            let _ = responses.send(HostResponse::Error { message }).await;
                        }
                    }
                    ServerEvent::Request { .. } | ServerEvent::TransportFailure { .. } => {}
                }
            }
        });
        // A targeted inspection reads the number from the same catalog a
        // listing does. A host never answers one, and the digit is how a
        // confirmation names the session it is about to stop.
        let recents = root.join("cache/workspaces.json");
        record_recent_workspace_in(&recents, &project).unwrap();
        let (service, mut events) = WorkspaceService::spawn_with(
            vec![root.join("empty-registry")],
            Some(recents.clone()),
            PathBuf::from(".runyte"),
            None,
            Some(runtime.to_path_buf()),
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
        assert_eq!(row.open_buffers, Some(9));
        assert_eq!(row.plugin_jobs, Some(2));
        assert_eq!(row.activity_leases, Some(1));
        assert_eq!(row.activities[0].owner, "watcher");
        assert_eq!(
            row.activities[0].state,
            crate::service_health::ActivityLeaseState::Cancelling
        );
        let optional = inspect_endpoint(&endpoint).await;
        assert_eq!(optional.plugin_jobs, Some(2));
        assert_eq!(optional.activity_leases, Some(1));
        assert_eq!(optional.activities, row.activities);
        assert_eq!(
            row.number,
            recorded_number(&recents, &project),
            "a targeted inspection must recover the session's number"
        );
        assert_eq!(row.number, Some(1));
        service
            .try_stop(10, project.clone(), root.to_path_buf(), false)
            .unwrap();
        let Some(WorkspaceEvent::Stopped {
            generation,
            selector,
            selection: None,
            result,
        }) = events.recv().await
        else {
            panic!("workspace service ended")
        };
        assert_eq!(generation, 10);
        assert_eq!(selector, project);
        result.expect("directory stop should reach the unregistered current host");
        host.await.unwrap();
        drop(runtime);
        drop(root);
    }

    /// A stop is only over once the host is gone.
    ///
    /// Every client answers a stop by listing again, so a stop that returns on
    /// the host's acknowledgement hands that listing an endpoint the exiting
    /// host still holds, and the session manager redraws the row it was just
    /// asked to close as running.
    #[tokio::test]
    async fn a_refresh_taken_after_a_stop_reports_the_session_stopped() {
        use std::collections::HashMap;

        use crate::{
            protocol::FeatureGroup,
            workspace::transport::{LocalServer, PROTOCOL_VERSION, ServerEvent},
        };

        let root = unique_test_root("stop-then-refresh");
        let project = root.join("project");
        let runtime = unique_test_root("stop-then-refresh-runtime");
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
                drop(runtime);
                drop(root);
                return;
            }
            Err(error) => panic!("cannot bind test transport: {error:#}"),
        };
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
                                    unread_terminals: 0,
                                    terminal_bell: false,
                                    protocol: PROTOCOL_VERSION,
                                    pid: std::process::id(),
                                    interactive_attached: false,
                                    unsaved_buffers: 0,
                                    open_buffers: 1,
                                    pending_wait_requests: 0,
                                    plugin_jobs: 0,
                                    activity_leases: 0,
                                    activities: Box::new([]),
                                    live_terminals: 0,
                                    terminal_sessions: 0,
                                    terminal_line_activity_unix_seconds: None,
                                })
                                .await;
                        }
                    }
                    ServerEvent::Request {
                        id,
                        request: ClientRequest::Shutdown,
                    } => {
                        if let Some(responses) = clients.get(&id) {
                            let _ = responses.send(HostResponse::ShuttingDown).await;
                        }
                        break;
                    }
                    ServerEvent::Disconnected { id } => {
                        clients.remove(&id);
                    }
                    ServerEvent::ProtocolError { id, message } => {
                        if let Some(responses) = clients.get(&id) {
                            let _ = responses.send(HostResponse::Error { message }).await;
                        }
                    }
                    ServerEvent::Request { .. } | ServerEvent::TransportFailure { .. } => {}
                }
            }
            // A real host answers first and unpublishes as it winds down. The
            // gap is what a stop has to outlast.
            tokio::time::sleep(Duration::from_millis(250)).await;
            drop(server);
        });

        let recents = root.join("cache/workspaces.json");
        record_recent_workspace_in(&recents, &project).unwrap();
        let (service, mut events) = WorkspaceService::spawn_with(
            vec![root.join("empty-registry")],
            Some(recents.clone()),
            PathBuf::from(".runyte"),
            None,
            Some(runtime.to_path_buf()),
        );
        service
            .try_stop(1, project.clone(), root.to_path_buf(), false)
            .unwrap();
        let Some(WorkspaceEvent::Stopped { result, .. }) = events.recv().await else {
            panic!("workspace service ended")
        };
        result.expect("the running host should accept the stop");

        service.try_refresh(2).unwrap();
        let Some(WorkspaceEvent::Refreshed { result, .. }) = events.recv().await else {
            panic!("workspace service ended")
        };
        let rows = result.unwrap();
        let row = rows
            .iter()
            .find(|row| row.project_root == project)
            .expect("the stopped workspace is still listed from history");
        assert!(
            !row.running,
            "a listing taken after a stop must not report the session running"
        );
        host.await.unwrap();
        drop(runtime);
        drop(root);
    }

    #[tokio::test]
    async fn a_registered_incompatible_host_lists_without_a_handshake_and_requires_force_to_stop() {
        use crate::workspace::transport::{EndpointMetadata, LocalServer, PROTOCOL_VERSION};

        let root = unique_test_root("registered-incompatible-host");
        let project = root.join("project");
        let runtime = unique_test_root("incompatible-runtime");
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
        let server = match LocalServer::bind(&endpoint).await {
            Ok(server) => server,
            Err(error)
                if error.chain().any(|cause| {
                    cause
                        .downcast_ref::<io::Error>()
                        .is_some_and(|error| error.raw_os_error() == Some(libc::EPERM))
                }) =>
            {
                drop(runtime);
                drop(root);
                return;
            }
            Err(error) => panic!("cannot bind test transport: {error:#}"),
        };

        let child_ready = root.join("delayed-child-ready");
        let mut child = std::process::Command::new("/bin/sh")
            .env("RUNYTE_DELAY_READY", &child_ready)
            .args([
                "-c",
                "trap 'sleep 0.7; exit 0' TERM; : > \"$RUNYTE_DELAY_READY\"; while :; do sleep 0.05; done",
            ])
            .spawn()
            .unwrap();
        let child_pid = child.id();
        let reaper = std::thread::spawn(move || child.wait());
        for _ in 0..100 {
            if child_ready.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(child_ready.exists(), "delayed child never became ready");
        let older_protocol = PROTOCOL_VERSION.checked_sub(1).unwrap();
        let mut metadata: EndpointMetadata =
            serde_json::from_slice(&fs::read(endpoint.metadata()).unwrap()).unwrap();
        metadata.protocol = older_protocol;
        metadata.pid = child_pid;
        let metadata_bytes = serde_json::to_vec_pretty(&metadata).unwrap();
        fs::write(endpoint.metadata(), &metadata_bytes).unwrap();
        let registry = runtime.join("runyte/hosts");
        let registration = registry.join(format!("{}.json", endpoint.id()));
        fs::write(&registration, metadata_bytes).unwrap();

        let rows = refresh(
            std::slice::from_ref(&registry),
            None,
            Path::new(".runyte"),
            Some(&runtime),
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].running);
        assert_eq!(rows[0].project_root, project);
        assert_eq!(rows[0].incompatible_protocol, Some(older_protocol));
        assert_eq!(rows[0].unsaved_buffers, None);

        let error = stop(
            std::slice::from_ref(&registry),
            &project,
            &root,
            Path::new(".runyte"),
            None,
            Some(&runtime),
            false,
        )
        .await
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("Force close"), "{message}");
        assert!(
            message.contains("live terminals or unsaved buffers"),
            "{message}"
        );
        assert!(super::super::transport::process_is_alive(child_pid).unwrap());

        let force_started = std::time::Instant::now();
        stop(
            std::slice::from_ref(&registry),
            &project,
            &root,
            Path::new(".runyte"),
            None,
            Some(&runtime),
            true,
        )
        .await
        .unwrap();
        assert!(
            force_started.elapsed() >= Duration::from_millis(600),
            "force stop returned before the incompatible host completed its delayed exit"
        );
        reaper.join().unwrap().unwrap();
        assert!(!endpoint.metadata().exists());
        assert!(!endpoint.socket().exists());
        assert!(!registration.exists());

        drop(server);
        drop(runtime);
        drop(root);
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

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn recents_reject_oversized_files_and_semantically_unbounded_entries() {
        let root = unique_test_root("bounded-recents");
        let path = root.join("cache/workspaces.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        fs::write(&path, vec![b' '; MAX_RECENTS_BYTES + 1]).unwrap();
        let error = read_recents(Some(&path)).unwrap_err().to_string();
        assert!(error.contains("exceed"), "{error}");

        let repeated = (0..=RECENT_LIMIT)
            .map(|_| RecentWorkspace {
                project_root_bytes: encode_path(Path::new("/")),
                name: None,
                number: None,
                last_active_unix_seconds: None,
                number_declined: false,
                number_pinned: false,
            })
            .collect::<Vec<_>>();
        fs::write(&path, serde_json::to_vec(&repeated).unwrap()).unwrap();
        let error = read_recents(Some(&path)).unwrap_err().to_string();
        assert!(error.contains("more than"), "{error}");

        let invalid_path = [RecentWorkspace {
            project_root_bytes: vec![b'/'; MAX_PERSISTED_PATH_BYTES + 1],
            name: None,
            number: None,
            last_active_unix_seconds: None,
            number_declined: false,
            number_pinned: false,
        }];
        fs::write(&path, serde_json::to_vec(&invalid_path).unwrap()).unwrap();
        let error = read_recents(Some(&path)).unwrap_err().to_string();
        assert!(error.contains("project directory exceeds"), "{error}");

        let invalid_name = [RecentWorkspace {
            project_root_bytes: encode_path(Path::new("/")),
            name: Some("x".repeat(MAX_HOST_NAME_BYTES + 1)),
            number: None,
            last_active_unix_seconds: None,
            number_declined: false,
            number_pinned: false,
        }];
        fs::write(&path, serde_json::to_vec(&invalid_name).unwrap()).unwrap();
        let error = read_recents(Some(&path)).unwrap_err().to_string();
        assert!(error.contains("session name cannot exceed"), "{error}");

        drop(root);
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

        drop(root);
    }

    #[test]
    fn usable_optional_recents_resolve_inside_the_cache_root() {
        let root = unique_test_root("usable-recents");
        let cache_root = root.join("cache");

        assert_eq!(
            recent_file_in(Some(cache_root.clone())),
            Some(cache_root.join("workspaces.json"))
        );

        drop(root);
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
        assert_eq!(rows[0].last_active_unix_seconds, None);
        drop(root);
    }

    #[tokio::test]
    async fn refresh_lists_the_least_recently_visited_first_and_never_visited_last() {
        let root = unique_test_root("visit-order");
        let recents = root.join("cache/workspaces.json");
        let mut workspaces = Vec::new();
        for name in ["oldest", "newest", "unvisited"] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            let directory = directory.canonicalize().unwrap();
            record_recent_workspace_in(&recents, &directory).unwrap();
            workspaces.push(directory);
        }
        // Stamped rather than attached to, so the order under test is the one
        // the values describe and not the second the test happened to run in.
        let stamp = |entries: &mut Vec<RecentEntry>, project_root: &Path, seconds: u64| {
            entries
                .iter_mut()
                .find(|entry| entry.project_root == project_root)
                .unwrap()
                .last_active_unix_seconds = Some(seconds);
        };
        update_recents(&recents, |entries| {
            stamp(entries, &workspaces[0], 1_000);
            stamp(entries, &workspaces[1], 9_000);
        })
        .unwrap();

        // No host is running in any of them, so nothing is numbered and the
        // visits are the whole answer.
        let rows = refresh(&[], Some(&recents), Path::new(".runyte"), None)
            .await
            .unwrap();
        assert!(rows.iter().all(|row| row.number.is_none()));
        assert_eq!(
            rows.iter()
                .map(|row| row.project_root.clone())
                .collect::<Vec<_>>(),
            workspaces
        );

        // Visiting the oldest one again moves it below the other visited row
        // rather than leaving the listing in the order the first visits made.
        update_recents(&recents, |entries| {
            stamp(entries, &workspaces[0], 12_000);
        })
        .unwrap();
        let rows = refresh(&[], Some(&recents), Path::new(".runyte"), None)
            .await
            .unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.project_root.clone())
                .collect::<Vec<_>>(),
            vec![
                workspaces[1].clone(),
                workspaces[0].clone(),
                workspaces[2].clone(),
            ]
        );

        drop(root);
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

        drop(root);
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
        drop(root);
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
        drop(root);
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
            "  release candidate  ",
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
            Some("release-candidate")
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
        drop(root);
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
            PathBuf::from(".runyte"),
            None,
            None,
        );
        for (generation, selector, name, expected) in [
            (1, PathBuf::from(row.id), "  by id  ", "by-id"),
            (2, PathBuf::from("by-id"), "by-name", "by-name"),
            (3, PathBuf::from("project"), "by-directory", "by-directory"),
        ] {
            service
                .try_rename(
                    generation,
                    selector.clone(),
                    root.to_path_buf(),
                    name.to_owned(),
                )
                .unwrap();
            let Some(WorkspaceEvent::Renamed {
                generation: completed,
                path,
                selection: None,
                name: completed_name,
                result,
            }) = events.recv().await
            else {
                panic!("session rename service ended")
            };
            assert_eq!(completed, generation);
            assert_eq!(path, selector);
            assert_eq!(completed_name, expected);
            result.unwrap();
        }
        assert_eq!(
            read_recents(Some(&recents)).unwrap()[0].name.as_deref(),
            Some("by-directory")
        );
        drop(root);
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
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn only_successful_attachment_backfills_an_older_catalog_activity_time() {
        let root = unique_test_root("activity-backfill");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let path = root.join("cache/workspaces.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old_catalog = serde_json::json!([{
            "project_root_bytes": encode_path(&workspace),
            "name": "workspace",
            "number": 1
        }]);
        fs::write(&path, serde_json::to_vec(&old_catalog).unwrap()).unwrap();

        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(entries[0].last_active_unix_seconds, None);
        assert!(!entries[0].number_pinned);

        record_recent_workspace_in(&path, &workspace).unwrap();
        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(entries[0].last_active_unix_seconds, None);

        record_workspace_activity_in(&path, &workspace).unwrap();
        let entries = read_recents(Some(&path)).unwrap();
        assert!(entries[0].last_active_unix_seconds.is_some());
        drop(root);
    }

    #[test]
    fn lifecycle_metadata_does_not_reorder_or_activate_an_existing_workspace() {
        let root = unique_test_root("metadata-without-visit");
        let path = root.join("cache/workspaces.json");
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        record_recent_workspace_in(&path, &first).unwrap();
        record_recent_workspace_in(&path, &second).unwrap();
        let first = first.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();

        ensure_recent_workspace_in(&path, &first).unwrap();
        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(entries[0].project_root, second);
        assert_eq!(entries[1].project_root, first);
        assert_eq!(entries[1].last_active_unix_seconds, None);

        record_workspace_activity_in(&path, &first).unwrap();
        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(entries[0].project_root, first);
        assert!(entries[0].last_active_unix_seconds.is_some());
        drop(root);
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
        drop(root);
    }

    #[test]
    fn default_names_are_valid_bounded_host_names() {
        assert_eq!(
            unique_default_workspace_name(Path::new("/workspace/ \n "), &[]),
            "workspace"
        );
        assert_eq!(
            unique_default_workspace_name(Path::new("/workspace/release candidate"), &[]),
            "release-candidate"
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
                publication_key: None,
                unread_terminals: None,
                terminal_bell: None,
                id: "11111111111111111111111111111111".to_owned(),
                name: None,
                number: None,
                last_active_unix_seconds: None,
                project_root: relative_target.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                plugin_jobs: None,
                activity_leases: None,
                activities: Vec::new(),
                live_terminals: None,
                terminal_sessions: None,
                terminal_line_activity_unix_seconds: None,
                interactive_attached: None,
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
            WorkspaceRow {
                publication_key: None,
                unread_terminals: None,
                terminal_bell: None,
                id: "22222222222222222222222222222222".to_owned(),
                name: Some("archive".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: named_target.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                plugin_jobs: None,
                activity_leases: None,
                activities: Vec::new(),
                live_terminals: None,
                terminal_sessions: None,
                terminal_line_activity_unix_seconds: None,
                interactive_attached: None,
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
            WorkspaceRow {
                publication_key: None,
                unread_terminals: None,
                terminal_bell: None,
                id: "abcdef0123456789abcdef0123456789".to_owned(),
                name: None,
                number: None,
                last_active_unix_seconds: None,
                project_root: id_target.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                plugin_jobs: None,
                activity_leases: None,
                activities: Vec::new(),
                live_terminals: None,
                terminal_sessions: None,
                terminal_line_activity_unix_seconds: None,
                interactive_attached: None,
                open_buffers: None,
                git: None,
                missing_directory: false,
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
        fs::remove_dir_all(&root).unwrap();
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
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: "11111111111111111111111111111111".to_owned(),
            name: Some("named-first".to_owned()),
            number: None,
            last_active_unix_seconds: None,
            project_root: first.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: None,
            plugin_jobs: None,
            activity_leases: None,
            activities: Vec::new(),
            live_terminals: None,
            terminal_sessions: None,
            terminal_line_activity_unix_seconds: None,
            interactive_attached: Some(false),
            open_buffers: None,
            git: None,
            missing_directory: false,
        }];

        // Model a second process recording a workspace while refresh is
        // inspecting the hosts represented by `stale_rows`.
        record_recent_workspace_in(&path, &recorded_during_refresh).unwrap();
        merge_refreshed_rows(&path, &snapshot, &stale_rows).unwrap();

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
        drop(root);
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
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: "11111111111111111111111111111111".to_owned(),
            name: Some("stale-inspection".to_owned()),
            number: None,
            last_active_unix_seconds: None,
            project_root: workspace.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: None,
            plugin_jobs: None,
            activity_leases: None,
            activities: Vec::new(),
            live_terminals: None,
            terminal_sessions: None,
            terminal_line_activity_unix_seconds: None,
            interactive_attached: Some(false),
            open_buffers: None,
            git: None,
            missing_directory: false,
        }];

        update_recents(&path, |paths| {
            paths[0].name = Some("concurrent-name".to_owned());
        })
        .unwrap();
        merge_refreshed_rows(&path, &snapshot, &stale_rows).unwrap();

        assert_eq!(
            named(read_recents(Some(&path)).unwrap()),
            vec![(workspace, Some("concurrent-name".to_owned()))]
        );
        drop(root);
    }

    #[tokio::test]
    async fn owner_wide_refresh_does_not_merge_host_names_into_local_recents() {
        use crate::workspace::transport::{EndpointMetadata, LocalServer, PROTOCOL_VERSION};

        let root = unique_test_root("broad-name-isolation");
        let project = root.join("project");
        let recents = root.join("cache/workspaces.json");
        let runtime = unique_test_root("broad-runtime");
        fs::create_dir_all(project.join(".runyte")).unwrap();
        fs::create_dir_all(&runtime).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let project = project.canonicalize().unwrap();
        record_recent_workspace_in(&recents, &project).unwrap();
        rename_recent_workspace_in(&recents, &project, "local-name").unwrap();
        let endpoint = LocalEndpoint::discover_with_runtime(
            &project.join(".runyte"),
            &project,
            Some(&runtime),
        )
        .unwrap();
        let server = match LocalServer::bind(&endpoint).await {
            Ok(server) => server,
            Err(error)
                if error.chain().any(|cause| {
                    cause
                        .downcast_ref::<io::Error>()
                        .is_some_and(|error| error.raw_os_error() == Some(libc::EPERM))
                }) =>
            {
                drop(runtime);
                drop(root);
                return;
            }
            Err(error) => panic!("cannot bind test transport: {error:#}"),
        };
        endpoint.rename("outside-name").unwrap();
        let mut metadata: EndpointMetadata =
            serde_json::from_slice(&fs::read(endpoint.metadata()).unwrap()).unwrap();
        metadata.protocol = PROTOCOL_VERSION.checked_sub(1).unwrap();
        let metadata = serde_json::to_vec_pretty(&metadata).unwrap();
        fs::write(endpoint.metadata(), &metadata).unwrap();
        let registry = runtime.join("runyte/hosts");
        fs::write(registry.join(format!("{}.json", endpoint.id())), &metadata).unwrap();

        let rows = refresh_with_name_persistence(
            std::slice::from_ref(&registry),
            Some(&recents),
            Path::new(".runyte"),
            Some(&runtime),
            false,
        )
        .await
        .unwrap();
        assert_eq!(rows[0].name.as_deref(), Some("outside-name"));
        assert_eq!(
            read_recents(Some(&recents)).unwrap()[0].name.as_deref(),
            Some("local-name")
        );

        drop(server);
        drop(runtime);
        drop(root);
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
        drop(root);
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
        drop(root);
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
        drop(root);
    }

    /// A worktree removed outside Runyte used to take its session's number with
    /// it: the entry was filtered out on read, and the next write persisted
    /// that filtered view, so a host still running there listed as unnumbered.
    #[test]
    fn a_vanished_directory_keeps_its_record_and_its_number() {
        let root = unique_test_root("number-missing-directory");
        let path = root.join("cache/workspaces.json");
        let kept = root.join("kept");
        let vanishing = root.join("vanishing");
        for workspace in [&kept, &vanishing] {
            fs::create_dir_all(workspace).unwrap();
            record_recent_workspace_in(&path, workspace).unwrap();
        }
        let vanishing = vanishing.canonicalize().unwrap();
        assert_eq!(recorded_number(&path, &vanishing), Some(2));

        fs::remove_dir_all(&vanishing).unwrap();
        // Any later write goes back through the reader, so this is where the
        // record used to be erased.
        let later = root.join("later");
        fs::create_dir_all(&later).unwrap();
        record_recent_workspace_in(&path, &later).unwrap();

        assert_eq!(
            recorded_number(&path, &vanishing),
            Some(2),
            "the number of a workspace whose directory went is still its own"
        );
        // The freed digit is not handed out again while the record holds it.
        assert_eq!(
            recorded_number(&path, &later.canonicalize().unwrap()),
            Some(3)
        );

        // A host still running there therefore keeps its digit in a listing,
        // while a stopped row with nothing left to open stays out of one.
        let entries = read_recents(Some(&path)).unwrap();
        let mut rows = vec![WorkspaceRow {
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("vanishing".to_owned()),
            number: None,
            last_active_unix_seconds: None,
            project_root: vanishing.clone(),
            running: true,
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
            missing_directory: true,
        }];
        assign_running_workspace_numbers(&mut rows, &entries);
        assert_eq!(
            rows[0].number,
            Some(1),
            "an automatic assignment compacts while the host remains reachable"
        );
        assert!(
            !listable_recents(entries)
                .iter()
                .any(|entry| entry.project_root == vanishing)
        );
        drop(root);
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
        drop(root);
    }

    /// One listing row, in whatever running state the numbering is about.
    fn numbering_row(project_root: &Path, running: bool) -> WorkspaceRow {
        WorkspaceRow {
            publication_key: None,
            unread_terminals: None,
            terminal_bell: None,
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: None,
            number: None,
            last_active_unix_seconds: None,
            project_root: project_root.to_path_buf(),
            running,
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

    #[test]
    fn a_cleared_digit_is_remembered_rather_than_handed_out_again() {
        let root = unique_test_root("number-declined");
        let path = root.join("cache/workspaces.json");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        record_recent_workspace_in(&path, &workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        assert_eq!(recorded_number(&path, &workspace), Some(1));

        set_recent_workspace_number_in(Some(&path), &workspace, None).unwrap();
        assert_eq!(recorded_number(&path, &workspace), None);

        // The running session it belongs to is left unnumbered, where it would
        // otherwise be given the lowest free digit by the next listing.
        let entries = read_recents(Some(&path)).unwrap();
        let mut rows = vec![numbering_row(&workspace, true)];
        assign_running_workspace_numbers(&mut rows, &entries);
        assert_eq!(rows[0].number, None);

        // Nor does revisiting the workspace, or a listing that backfills an
        // older catalog, quietly claim one for it.
        record_recent_workspace_in(&path, &workspace).unwrap();
        assert_eq!(recorded_number(&path, &workspace), None);
        let mut entries = read_recents(Some(&path)).unwrap();
        assign_missing_default_workspace_numbers(&mut entries);
        assert_eq!(entries[0].number, None);

        // Numbering it again ends the decision.
        set_recent_workspace_number_in(Some(&path), &workspace, Some(4)).unwrap();
        assert_eq!(recorded_number(&path, &workspace), Some(4));
        let entries = read_recents(Some(&path)).unwrap();
        assert!(!entries[0].number_declined);
        assert!(entries[0].number_pinned);
        let mut rows = vec![numbering_row(&workspace, true)];
        assign_running_workspace_numbers(&mut rows, &entries);
        assert_eq!(rows[0].number, Some(4));
        drop(root);
    }

    /// The mirror of `refresh_merge_preserves_a_concurrently_changed_existing_name`
    /// for digits: a refresh must not undo a renumbering, or an unpinning, that
    /// landed while it was gathering the listing.
    #[test]
    fn a_refresh_does_not_overwrite_a_concurrently_changed_number() {
        let root = unique_test_root("number-writeback-race");
        let path = root.join("cache/workspaces.json");
        let renumbered = root.join("renumbered");
        let unpinned = root.join("unpinned");
        for workspace in [&renumbered, &unpinned] {
            fs::create_dir_all(workspace).unwrap();
            record_recent_workspace_in(&path, workspace).unwrap();
        }
        let renumbered = renumbered.canonicalize().unwrap();
        let unpinned = unpinned.canonicalize().unwrap();

        let snapshot = read_recents(Some(&path)).unwrap();
        let mut rows = vec![
            numbering_row(&renumbered, true),
            numbering_row(&unpinned, true),
        ];
        assign_running_workspace_numbers(&mut rows, &snapshot);
        assert_eq!(rows[0].number, Some(1));
        assert_eq!(rows[1].number, Some(2));

        // Another process answers for both while this listing is in flight.
        set_recent_workspace_number_in(Some(&path), &renumbered, Some(7)).unwrap();
        set_recent_workspace_number_in(Some(&path), &unpinned, None).unwrap();
        merge_refreshed_rows(&path, &snapshot, &rows).unwrap();

        assert_eq!(
            recorded_number(&path, &renumbered),
            Some(7),
            "the digit somebody just chose survives a refresh that read the old one"
        );
        assert_eq!(recorded_number(&path, &unpinned), None);
        let entries = read_recents(Some(&path)).unwrap();
        assert!(
            entries
                .iter()
                .find(|entry| entry.project_root == unpinned)
                .unwrap()
                .number_declined,
            "an unpinned workspace is not given its digit back by a stale refresh"
        );
        drop(root);
    }

    #[test]
    fn a_workspace_displaced_by_a_swap_may_still_be_numbered_again() {
        let root = unique_test_root("number-swap-declined");
        let path = root.join("cache/workspaces.json");
        let first = root.join("first");
        let second = root.join("second");
        for workspace in [&first, &second] {
            fs::create_dir_all(workspace).unwrap();
            record_recent_workspace_in(&path, workspace).unwrap();
        }
        let first = first.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();

        // The asking workspace has no digit to hand over, so the swap leaves
        // the holder without one -- but it never asked for that, so a listing
        // is still free to give it another.
        set_recent_workspace_number_in(Some(&path), &second, None).unwrap();
        let displaced = set_recent_workspace_number_in(Some(&path), &second, Some(1)).unwrap();
        assert_eq!(displaced.as_deref(), Some(first.as_path()));
        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(recorded_number(&path, &first), None);
        assert!(
            !entries
                .iter()
                .find(|entry| entry.project_root == first)
                .unwrap()
                .number_declined
        );
        let mut rows = vec![numbering_row(&first, true), numbering_row(&second, true)];
        assign_running_workspace_numbers(&mut rows, &entries);
        assert_eq!(rows[1].number, Some(1));
        assert_eq!(rows[0].number, Some(2));
        drop(root);
    }

    #[test]
    fn a_refresh_records_the_digit_a_running_session_was_given() {
        let root = unique_test_root("number-writeback");
        let path = root.join("cache/workspaces.json");
        let stopped = root.join("stopped");
        let started = root.join("started");
        for workspace in [&stopped, &started] {
            fs::create_dir_all(workspace).unwrap();
            record_recent_workspace_in(&path, workspace).unwrap();
        }
        let stopped = stopped.canonicalize().unwrap();
        let started = started.canonicalize().unwrap();
        assert_eq!(recorded_number(&path, &stopped), Some(1));
        assert_eq!(recorded_number(&path, &started), Some(2));

        let snapshot = read_recents(Some(&path)).unwrap();
        let mut rows = vec![
            numbering_row(&started, true),
            numbering_row(&stopped, false),
        ];
        // Automatic running numbers close gaps, and stopped rows retain no
        // numbering state after the refresh is persisted.
        assign_running_workspace_numbers(&mut rows, &snapshot);
        merge_refreshed_rows(&path, &snapshot, &rows).unwrap();
        assert_eq!(recorded_number(&path, &started), Some(1));
        assert_eq!(recorded_number(&path, &stopped), None);
        drop(root);
    }

    #[test]
    fn a_new_running_session_takes_the_gap_and_the_stopped_session_forgets_it() {
        let first = PathBuf::from("/w/first");
        let second = PathBuf::from("/w/second");
        let third = PathBuf::from("/w/third");
        let stopped = PathBuf::from("/w/stopped");
        let new = PathBuf::from("/w/new");
        let mut stopped_entry = RecentEntry::new(stopped.clone(), None, Some(4), None);
        stopped_entry.number_pinned = true;
        let snapshot = vec![
            RecentEntry::new(new.clone(), None, Some(5), None),
            stopped_entry,
            RecentEntry::new(third.clone(), None, Some(3), None),
            RecentEntry::new(second.clone(), None, Some(2), None),
            RecentEntry::new(first.clone(), None, Some(1), None),
        ];
        let mut rows = vec![
            numbering_row(&new, true),
            numbering_row(&stopped, false),
            numbering_row(&third, true),
            numbering_row(&second, true),
            numbering_row(&first, true),
        ];

        assign_running_workspace_numbers(&mut rows, &snapshot);

        let number_of = |target: &Path| {
            rows.iter()
                .find(|row| row.project_root == target)
                .and_then(|row| row.number)
        };
        assert_eq!(number_of(&first), Some(1));
        assert_eq!(number_of(&second), Some(2));
        assert_eq!(number_of(&third), Some(3));
        assert_eq!(number_of(&new), Some(4));
        assert_eq!(number_of(&stopped), None);

        let mut persisted = snapshot.clone();
        merge_assigned_numbers(&mut persisted, &snapshot, &rows);
        let stopped = persisted
            .iter()
            .find(|entry| entry.project_root == stopped)
            .unwrap();
        assert_eq!(stopped.number, None);
        assert!(!stopped.number_pinned);
        assert!(!stopped.number_declined);
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
        drop(root);
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
        drop(root);
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
        drop(root);
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
        drop(root);
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
                None,
            ));
            paths.push(RecentEntry::new(
                second.canonicalize().unwrap(),
                Some("second".to_owned()),
                Some(1),
                None,
            ));
        })
        .unwrap();

        let entries = read_recents(Some(&path)).unwrap();
        assert_eq!(entries[0].number, Some(1), "the first claim stands");
        assert_eq!(
            entries[1].number, None,
            "the duplicate is dropped rather than letting one key select two rows"
        );
        drop(root);
    }

    /// The number the catalog at `path` records for `project_root`.
    fn recorded_number(path: &Path, project_root: &Path) -> Option<u8> {
        read_recents(Some(path))
            .unwrap()
            .into_iter()
            .find(|entry| entry.project_root == project_root)
            .and_then(|entry| entry.number)
    }

    fn unique_test_root(label: &str) -> TestRuntimeRoot {
        TestRuntimeRoot::new(label).unwrap()
    }
}

fn merge_refreshed_rows(
    path: &Path,
    snapshot: &[RecentEntry],
    rows: &[WorkspaceRow],
) -> Result<()> {
    update_recents(path, |paths| {
        merge_assigned_numbers(paths, snapshot, rows);
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

#[cfg(test)]
mod destination_inventory_tests {
    use super::*;

    #[tokio::test]
    async fn reattachment_replaces_an_observation_started_before_it() {
        let root = crate::test_support::TestRuntimeRoot::new("reattach-observation").unwrap();
        let registry = root.create_private_dir("registry").unwrap();
        let runtime = root.create_private_dir("runtime").unwrap();
        let (service, mut events) = WorkspaceService::spawn_with(
            vec![registry],
            None,
            PathBuf::from(".runyte"),
            None,
            Some(runtime),
        );
        let mut app = crate::app::App::new(crate::config::Config::default(), None).unwrap();
        app.enable_persistent_session();
        app.attach_workspace_service(service);
        app.note_frontend_attached();
        let stale = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(stale, WorkspaceEvent::Observed { .. }));
        // The old response is already queued when the same frontend returns.
        // It must prompt a fresh scan instead of consuming the new deadline.
        app.note_frontend_attached();
        app.apply_workspace_event(stale);
        let fresh = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("no replacement scan after reattachment")
            .unwrap();
        assert!(matches!(fresh, WorkspaceEvent::Observed { .. }));
        app.apply_workspace_event(fresh);
        assert!(
            events.try_recv().is_err(),
            "replacement scans must coalesce"
        );
    }

    #[tokio::test]
    async fn navigation_service_keeps_discovery_and_failures_inside_explicit_runtime_scope() {
        let root = crate::test_support::TestRuntimeRoot::new("navigation-service").unwrap();
        let project = root.create_private_dir("project").unwrap();
        let registry = root.create_private_dir("registry").unwrap();
        let runtime = root.create_private_dir("runtime").unwrap();
        let (service, mut events) = WorkspaceService::spawn_with(
            vec![registry],
            None,
            PathBuf::from(".runyte"),
            None,
            Some(runtime),
        );
        service.try_observe(false).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event,WorkspaceEvent::Observed { result:Ok(rows) } if rows.is_empty()));
        service.try_directory_worktrees(7, project.clone()).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(event,WorkspaceEvent::DirectoryWorktrees {generation:7,result:Ok(rows)} if rows.is_empty())
        );
        service
            .try_inventory(8, WorkspaceSelection::project_only(project.clone()))
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(event,WorkspaceEvent::Inventory {generation:8,path,result:Err(_), ..} if path==project)
        );
    }
}
