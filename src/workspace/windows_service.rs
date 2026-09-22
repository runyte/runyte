// SPDX-License-Identifier: MPL-2.0

//! Owned native catalog work. The editor holds only a bounded sender; a
//! dedicated thread owns the runtime, complete snapshots and recovery ledger.

use super::{
    catalog_values::{WorkspaceEvent, WorkspaceRow, WorkspaceSelection},
    windows_catalog::HistoryTarget,
    windows_control::{ControlSnapshot, UserSelector},
    windows_endpoint::MAX_PERSISTED_PATH_BYTES,
    windows_lifecycle::connect_control,
    windows_location::{DiscoveryScope, KnownReadLocation},
};
use crate::{
    git::{GitCliProvider, GitProvider},
    protocol::{ClientRequest, HostResponse},
};
use anyhow::{Context, Result, ensure};
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};

const REQUEST_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 16;
const MAX_ERROR_BYTES: usize = 1024;
const MAX_WARNING_DETAILS: usize = 8;
const MAX_WARNING_BYTES: usize = 4096;
const PREVIEW_BUDGET: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
enum Target {
    UserSelector {
        selector: PathBuf,
        working_directory: Option<PathBuf>,
    },
    SelectedRow(WorkspaceSelection),
}

impl Target {
    fn path(&self) -> &Path {
        match self {
            Self::UserSelector { selector, .. } => selector,
            Self::SelectedRow(selection) => selection.project_root(),
        }
    }

    fn selection(&self) -> Option<WorkspaceSelection> {
        match self {
            Self::UserSelector { .. } => None,
            Self::SelectedRow(selection) => Some(selection.clone()),
        }
    }
}

#[derive(Debug)]
enum Request {
    Refresh {
        generation: u64,
        include_hidden: bool,
    },
    Poll,
    Observe,
    Stop {
        generation: u64,
        target: Target,
        force: bool,
    },
    Rename {
        generation: u64,
        target: Target,
        name: String,
    },
    Clean {
        generation: u64,
    },
    DirectoryWorktrees {
        generation: u64,
        path: PathBuf,
    },
    #[cfg(test)]
    Hold {
        entered: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    },
    #[cfg(test)]
    Emit {
        generation: u64,
        entered: Option<oneshot::Sender<()>>,
    },
    #[cfg(test)]
    Panic {
        entered: oneshot::Sender<()>,
    },
    #[cfg(test)]
    EmitWarnings {
        entered: oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
struct PreviewRequest {
    generation: u64,
    selection: WorkspaceSelection,
}

/// Clonable admission only. It never owns the worker or a joining destructor.
#[derive(Clone)]
pub struct WorkspaceServiceHandle {
    requests: mpsc::Sender<Request>,
    previews: watch::Sender<Option<PreviewRequest>>,
    stop: watch::Receiver<bool>,
}

impl WorkspaceServiceHandle {
    pub fn try_refresh(&self, generation: u64, include_hidden: bool) -> Result<(), &'static str> {
        self.submit(Request::Refresh {
            generation,
            include_hidden,
        })
    }

    pub fn try_poll(&self) -> Result<(), &'static str> {
        self.submit(Request::Poll)
    }

    pub fn try_observe(&self) -> Result<(), &'static str> {
        self.submit(Request::Observe)
    }

    pub fn try_stop_selected(
        &self,
        generation: u64,
        selection: WorkspaceSelection,
        force: bool,
    ) -> Result<(), &'static str> {
        self.check_open()?;
        validate_selection(&selection)?;
        self.submit(Request::Stop {
            generation,
            target: Target::SelectedRow(selection),
            force,
        })
    }

    pub fn try_stop_selector(
        &self,
        generation: u64,
        selector: &Path,
        working_directory: Option<&Path>,
        force: bool,
    ) -> Result<(), &'static str> {
        let target = user_target(selector, working_directory)?;
        self.submit(Request::Stop {
            generation,
            target,
            force,
        })
    }

    pub fn try_rename_selected(
        &self,
        generation: u64,
        selection: WorkspaceSelection,
        name: &str,
    ) -> Result<(), &'static str> {
        validate_selection(&selection)?;
        let name = checked_name(name)?;
        self.submit(Request::Rename {
            generation,
            target: Target::SelectedRow(selection),
            name,
        })
    }

    pub fn try_rename_selector(
        &self,
        generation: u64,
        selector: &Path,
        working_directory: Option<&Path>,
        name: &str,
    ) -> Result<(), &'static str> {
        let target = user_target(selector, working_directory)?;
        let name = checked_name(name)?;
        self.submit(Request::Rename {
            generation,
            target,
            name,
        })
    }

    pub fn try_clean(&self, generation: u64) -> Result<(), &'static str> {
        self.submit(Request::Clean { generation })
    }

    pub fn try_directory_worktrees(
        &self,
        generation: u64,
        path: &Path,
    ) -> Result<(), &'static str> {
        validate_path(path)?;
        if !path.is_absolute() {
            return Err("worktree discovery requires an absolute project path");
        }
        self.submit(Request::DirectoryWorktrees {
            generation,
            path: path.to_owned(),
        })
    }

    pub fn try_preview(
        &self,
        generation: u64,
        selection: WorkspaceSelection,
    ) -> Result<(), &'static str> {
        self.check_open()?;
        validate_selection(&selection)?;
        self.previews
            .send(Some(PreviewRequest {
                generation,
                selection,
            }))
            .map_err(|_| "native session preview service is unavailable")
    }

    fn submit(&self, request: Request) -> Result<(), &'static str> {
        self.check_open()?;
        self.requests
            .try_send(request)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "native session service queue is full",
                mpsc::error::TrySendError::Closed(_) => "native session service is unavailable",
            })
    }

    fn check_open(&self) -> Result<(), &'static str> {
        if *self.stop.borrow() {
            Err("native session service is shutting down")
        } else {
            Ok(())
        }
    }
}

fn validate_path(path: &Path) -> Result<(), &'static str> {
    if path
        .as_os_str()
        .encode_wide()
        .take(MAX_PERSISTED_PATH_BYTES / 2 + 1)
        .count()
        > MAX_PERSISTED_PATH_BYTES / 2
    {
        Err("native session path is too long")
    } else {
        Ok(())
    }
}

fn validate_selection(selection: &WorkspaceSelection) -> Result<(), &'static str> {
    validate_path(selection.project_root())
}

fn user_target(selector: &Path, working_directory: Option<&Path>) -> Result<Target, &'static str> {
    validate_path(selector)?;
    if let Some(path) = working_directory {
        validate_path(path)?;
    }
    Ok(Target::UserSelector {
        selector: selector.to_owned(),
        working_directory: working_directory.map(Path::to_owned),
    })
}

fn checked_name(name: &str) -> Result<String, &'static str> {
    if name.len() > crate::workspace::session_name::MAX_HOST_NAME_BYTES {
        return Err("session name is too long");
    }
    let normalized = super::normalize_session_name(name);
    super::session_name::validate_host_name(&normalized).map_err(|_| "session name is invalid")?;
    Ok(normalized)
}

/// The sole thread/runtime owner. A cancelled `shutdown` call leaves its
/// completion receiver and join handle here for a later call or Drop.
pub struct WorkspaceServiceOwner {
    stop: watch::Sender<bool>,
    completed: Option<oneshot::Receiver<()>>,
    thread: Option<JoinHandle<Result<()>>>,
}

impl WorkspaceServiceOwner {
    pub fn spawn(
        scope: DiscoveryScope,
        current: Option<KnownReadLocation>,
        configured_state: PathBuf,
    ) -> std::io::Result<(WorkspaceServiceHandle, Self, mpsc::Receiver<WorkspaceEvent>)> {
        Self::spawn_with_snapshot(scope, current, configured_state, None)
    }

    fn spawn_with_snapshot(
        scope: DiscoveryScope,
        current: Option<KnownReadLocation>,
        configured_state: PathBuf,
        snapshot: Option<ControlSnapshot>,
    ) -> std::io::Result<(WorkspaceServiceHandle, Self, mpsc::Receiver<WorkspaceEvent>)> {
        validate_path(&configured_state).map_err(std::io::Error::other)?;
        let (requests, request_rx) = mpsc::channel(REQUEST_CAPACITY);
        let (previews, preview_rx) = watch::channel(None);
        let (events, event_rx) = mpsc::channel(EVENT_CAPACITY);
        let (stop, stop_rx) = watch::channel(false);
        let (completed_tx, completed_rx) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("runyte-native-catalog".to_owned())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("native catalog runtime could not start")
                    .and_then(|runtime| {
                        runtime.block_on(
                            Worker {
                                scope,
                                current,
                                configured_state,
                                snapshot,
                                include_hidden: false,
                                requests: request_rx,
                                previews: preview_rx,
                                events,
                                stop: stop_rx,
                            }
                            .run(),
                        )
                    });
                let _ = completed_tx.send(());
                result
            })?;
        Ok((
            WorkspaceServiceHandle {
                requests,
                previews,
                stop: stop.subscribe(),
            },
            Self {
                stop,
                completed: Some(completed_rx),
                thread: Some(thread),
            },
            event_rx,
        ))
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        let _ = self.stop.send(true);
        if let Some(completed) = self.completed.as_mut() {
            let _ = completed.await;
            self.completed = None;
        }
        self.join()
    }

    fn join(&mut self) -> Result<()> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("native catalog worker panicked"))??;
        }
        Ok(())
    }
}

impl Drop for WorkspaceServiceOwner {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        // Drop is called by the service owner after its event loop, never by
        // App rendering/input. A panicked worker still has its handle joined.
        let _ = self.join();
    }
}

struct Worker {
    scope: DiscoveryScope,
    current: Option<KnownReadLocation>,
    configured_state: PathBuf,
    snapshot: Option<ControlSnapshot>,
    include_hidden: bool,
    requests: mpsc::Receiver<Request>,
    previews: watch::Receiver<Option<PreviewRequest>>,
    events: mpsc::Sender<WorkspaceEvent>,
    stop: watch::Receiver<bool>,
}

impl Worker {
    async fn run(mut self) -> Result<()> {
        let mut worker_result = Ok(());
        let mut preview_open = true;
        loop {
            if *self.stop.borrow() {
                break;
            }
            let result = tokio::select! {
                biased;
                _ = self.stop.changed() => break,
                request = self.requests.recv() => match request {
                    Some(request) => self.handle_request(request).await,
                    None => break,
                },
                changed = self.previews.changed(), if preview_open => {
                    if changed.is_err() {
                        preview_open = false;
                        continue;
                    }
                    let request = self.previews.borrow_and_update().clone();
                    if let Some(request) = request {
                        self.handle_preview(request).await
                    } else {
                        Ok(())
                    }
                }
            };
            if let Err(error) = result {
                worker_result = Err(error);
                break;
            }
        }
        // A stopped-name transaction is synchronous and worker-owned. Try its
        // retained rollback before dropping the snapshot or runtime.
        if let Some(snapshot) = self.snapshot.as_mut()
            && let Err(error) = snapshot.retry_name_recovery()
        {
            return Err(match worker_result {
                Ok(()) => error,
                Err(primary) => {
                    primary.context(format!("stopped-name recovery also failed: {error}"))
                }
            });
        }
        worker_result
    }

    async fn observe(&self, include_hidden: bool) -> Result<ControlSnapshot> {
        ControlSnapshot::observe_at(
            &self.scope,
            self.current.as_ref(),
            &self.configured_state,
            include_hidden,
        )
        .await
    }

    async fn observe_cancellable(&self, include_hidden: bool) -> Result<ControlSnapshot> {
        let mut stop = self.stop.clone();
        ensure!(!*stop.borrow(), "native catalog is shutting down");
        tokio::select! {
            biased;
            _ = stop.changed() => anyhow::bail!("native catalog is shutting down"),
            result = self.observe(include_hidden) => result,
        }
    }

    fn recover(&mut self) -> Result<()> {
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.retry_name_recovery()?;
        }
        Ok(())
    }

    async fn refresh(&mut self, include_hidden: bool) -> Result<Vec<WorkspaceRow>> {
        self.recover()?;
        let snapshot = self.observe_cancellable(include_hidden).await?;
        let rows = snapshot
            .history()
            .entries()
            .iter()
            .map(|entry| entry.row().clone())
            .collect();
        self.snapshot = Some(snapshot);
        self.include_hidden = include_hidden;
        Ok(rows)
    }

    async fn select_target(&mut self, target: &Target) -> Result<usize> {
        self.recover()?;
        match target {
            Target::SelectedRow(selection) => self
                .snapshot
                .as_ref()
                .context("selected session has no complete catalog")?
                .history()
                .select_selection(selection)?
                .context("selected session changed; choose it again"),
            Target::UserSelector {
                selector,
                working_directory,
            } => {
                // A user-authored selector is never resolved against a cached
                // display snapshot. This observation must complete first.
                self.snapshot = Some(self.observe_cancellable(self.include_hidden).await?);
                self.snapshot
                    .as_ref()
                    .expect("complete observation installed")
                    .select(UserSelector {
                        selector,
                        working_directory: working_directory.as_deref(),
                    })
            }
        }
    }

    async fn handle_request(&mut self, request: Request) -> Result<()> {
        let event = match request {
            Request::Refresh {
                generation,
                include_hidden,
            } => WorkspaceEvent::Refreshed {
                generation,
                result: self.refresh(include_hidden).await.map_err(error_text),
            },
            Request::Poll => WorkspaceEvent::Polled {
                result: self.refresh(self.include_hidden).await.map_err(error_text),
            },
            Request::Observe => WorkspaceEvent::Observed {
                result: self.refresh(self.include_hidden).await.map_err(error_text),
            },
            Request::Stop {
                generation,
                target,
                force,
            } => {
                let selector = target.path().to_owned();
                let selection = target.selection();
                let result = async {
                    let index = self.select_target(&target).await?;
                    self.snapshot
                        .as_ref()
                        .expect("selected snapshot")
                        .stop(index, force)
                        .await
                }
                .await;
                let result = match result {
                    Ok(outcome) => {
                        self.publish_warnings(generation, outcome.cleanup_issues)
                            .await?;
                        Ok(())
                    }
                    Err(error) => Err(error_text(error)),
                };
                WorkspaceEvent::Stopped {
                    generation,
                    selector,
                    selection,
                    result,
                }
            }
            Request::Rename {
                generation,
                target,
                name,
            } => {
                let path = target.path().to_owned();
                let selection = target.selection();
                let result = async {
                    let index = self.select_target(&target).await?;
                    self.snapshot
                        .as_mut()
                        .expect("selected snapshot")
                        .rename(index, &name)
                        .await
                }
                .await;
                let result = match result {
                    Ok(outcome) => {
                        if let Some(issue) = outcome.cache_issue {
                            self.publish_warnings(generation, vec![issue]).await?;
                        }
                        Ok(())
                    }
                    Err(error) => Err(error_text(error)),
                };
                WorkspaceEvent::Renamed {
                    generation,
                    path,
                    selection,
                    name,
                    result,
                }
            }
            Request::Clean { generation } => WorkspaceEvent::Cleaned {
                generation,
                result: async {
                    self.recover()?;
                    self.snapshot = Some(self.observe_cancellable(self.include_hidden).await?);
                    self.snapshot
                        .as_ref()
                        .expect("complete catalog installed")
                        .clean()
                }
                .await
                .map_err(error_text),
            },
            Request::DirectoryWorktrees { generation, path } => {
                let git = GitCliProvider::from_environment();
                WorkspaceEvent::DirectoryWorktrees {
                    generation,
                    result: discover_worktrees(git.as_ref(), &path),
                }
            }
            #[cfg(test)]
            Request::Hold { entered, release } => {
                let _ = entered.send(());
                let _ = release.await;
                return Ok(());
            }
            #[cfg(test)]
            Request::Emit {
                generation,
                entered,
            } => {
                if let Some(entered) = entered {
                    let _ = entered.send(());
                }
                WorkspaceEvent::Refreshed {
                    generation,
                    result: Ok(Vec::new()),
                }
            }
            #[cfg(test)]
            Request::Panic { entered } => {
                let _ = entered.send(());
                panic!("native catalog fixture panic")
            }
            #[cfg(test)]
            Request::EmitWarnings { entered } => {
                let _ = entered.send(());
                self.publish_warnings(100, vec!["retirement warning".to_owned()])
                    .await?;
                WorkspaceEvent::Refreshed {
                    generation: 100,
                    result: Ok(Vec::new()),
                }
            }
        };
        self.publish(event).await
    }

    async fn handle_preview(&mut self, request: PreviewRequest) -> Result<()> {
        let path = request.selection.project_root().to_owned();
        let selection = request.selection;
        let result = tokio::select! {
            biased;
            _ = self.stop.changed() => return Ok(()),
            result = async {
                let snapshot = self.snapshot.as_ref().context("selected session has no complete catalog")?;
                let index = snapshot.history().select_selection(&selection)?
                    .context("selected session changed; choose it again")?;
                let HistoryTarget::Live { publication, .. } = snapshot.history().target(index)
                    .context("selected session is unavailable")? else {
                    anyhow::bail!("stopped session has no live preview")
                };
                ensure!(publication.metadata().protocol == crate::protocol::VERSION,
                    "incompatible session has no live preview");
                tokio::time::timeout(PREVIEW_BUDGET, async {
                    let mut client = connect_control(publication.metadata()).await?;
                    client.send(&ClientRequest::SessionPreview).await?;
                    match client.recv().await? {
                        Some(HostResponse::SessionPreview { preview }) => Ok(preview.into()),
                        Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                            anyhow::bail!(message)
                        }
                        Some(_) => anyhow::bail!("native host returned an unexpected preview response"),
                        None => anyhow::bail!("native host disconnected during preview"),
                    }
                }).await.context("native session preview timed out")?
            } => result.map_err(error_text),
        };
        if self.previews.has_changed().unwrap_or(false) {
            return Ok(());
        }
        self.publish(WorkspaceEvent::Previewed {
            generation: request.generation,
            path,
            selection,
            result,
        })
        .await
    }

    async fn publish_warnings(&mut self, generation: u64, issues: Vec<String>) -> Result<()> {
        if issues.is_empty() {
            return Ok(());
        }
        let total = issues.len();
        let mut bytes = 0;
        let mut details = Vec::new();
        for issue in issues {
            let issue = bounded(issue, MAX_ERROR_BYTES);
            if details.len() == MAX_WARNING_DETAILS || bytes + issue.len() > MAX_WARNING_BYTES {
                break;
            }
            bytes += issue.len();
            details.push(issue);
        }
        self.publish(WorkspaceEvent::ControlWarnings {
            generation,
            omitted: total - details.len(),
            details,
        })
        .await
    }

    async fn publish(&mut self, event: WorkspaceEvent) -> Result<()> {
        if *self.stop.borrow() {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = self.stop.changed() => Ok(()),
            result = self.events.send(event) => {
                if result.is_err() { anyhow::bail!("native session event receiver closed") }
                Ok(())
            },
        }
    }
}

fn discover_worktrees(git: Option<&GitCliProvider>, path: &Path) -> Result<Vec<PathBuf>, String> {
    let Some(git) = git else {
        return Ok(Vec::new());
    };
    match git.discover(path).map_err(error_text)? {
        Some(repository) => git
            .worktrees(&repository)
            .map(|rows| {
                rows.into_iter()
                    .filter(|row| !row.bare && !row.missing)
                    .map(|row| row.path)
                    .take(256)
                    .collect()
            })
            .map_err(error_text),
        None => Ok(Vec::new()),
    }
}

fn error_text(error: impl std::fmt::Display) -> String {
    bounded(error.to_string(), MAX_ERROR_BYTES)
}

fn bounded(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max - 3;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }
    text
}

#[cfg(test)]
mod tests;
