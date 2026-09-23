// SPDX-License-Identifier: MPL-2.0

//! Owned native catalog work. The editor holds only a bounded sender; a
//! dedicated thread owns the runtime, complete snapshots and recovery ledger.

use super::{
    catalog_values::{WorkspaceEvent, WorkspaceRow, WorkspaceSelection},
    windows_catalog::HistoryTarget,
    windows_control::{ControlSnapshot, UserSelector},
    windows_endpoint::{EndpointMetadata, MAX_PERSISTED_PATH_BYTES},
    windows_lifecycle::connect_control,
    windows_location::{DiscoveryScope, KnownReadLocation, ResolvedLayout},
    windows_process_identity::PinnedProcess,
    windows_startup::{self, HostStartup, StartDisposition},
};
use crate::{
    git::{GitCliProvider, GitProvider},
    protocol::{ClientRequest, HostResponse},
};
use anyhow::{Context, Result, ensure};
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, watch};

const REQUEST_CAPACITY: usize = 16;
const EVENT_CAPACITY: usize = 16;
const MAX_ERROR_BYTES: usize = 1024;
const MAX_WARNING_DETAILS: usize = 8;
const MAX_WARNING_BYTES: usize = 4096;
const PREVIEW_BUDGET: Duration = Duration::from_secs(2);
const PREPARE_BUDGET: Duration = Duration::from_secs(3);
const PARENT_ATTACH_BUDGET: Duration = Duration::from_secs(12);

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
    PrepareSelectedLive {
        selection: WorkspaceSelection,
        reply: oneshot::Sender<Result<PreparedLiveTarget>>,
    },
    PrepareParentAttach {
        selector: PathBuf,
        working_directory: PathBuf,
        reply: oneshot::Sender<Result<PreparedLiveTarget>>,
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

/// One exact live publication prepared by the catalog worker that observed it.
/// Metadata alone is never authority: the authenticated process object stays
/// retained through the future frontend handoff.
#[derive(Debug)]
pub struct PreparedLiveTarget {
    metadata: EndpointMetadata,
    peer: Arc<PinnedProcess>,
    acceptance: Option<oneshot::Sender<StartupAcceptance>>,
}

impl PreparedLiveTarget {
    pub fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }

    pub fn peer(&self) -> &Arc<PinnedProcess> {
        &self.peer
    }

    pub async fn accept(self, deadline: Instant) -> Result<()> {
        self.prepare_acceptance(deadline)
            .await?
            .commit()?
            .finish()
            .await
    }

    /// Starts the final release handshake without committing it. Dropping the
    /// returned decision closes the worker-owned request and settles an armed
    /// provisional host. `commit` is the single irreversible boundary.
    pub async fn prepare_acceptance(
        mut self,
        deadline: std::time::Instant,
    ) -> Result<PreparedAcceptanceDecision> {
        let Some(acceptance) = self.acceptance.take() else {
            return Ok(PreparedAcceptanceDecision {
                decision: None,
                result: None,
            });
        };
        ensure!(
            std::time::Instant::now() < deadline,
            "native parent attach admission deadline expired before destination acceptance"
        );
        let (reply, ready) = oneshot::channel();
        acceptance
            .send(StartupAcceptance { deadline, reply })
            .map_err(|_| anyhow::anyhow!("native parent attach startup owner is unavailable"))?;
        ready
            .await
            .context("native parent attach startup owner stopped before commit decision")?
            .map_err(anyhow::Error::msg)
    }
}

/// One worker-owned startup waiting for the host's final atomic decision.
/// Dropping before `commit` cancels and settles it. After `commit`, only the
/// returned result can describe whether the destination was released.
#[derive(Debug)]
pub struct PreparedAcceptanceDecision {
    decision: Option<oneshot::Sender<StartupDecision>>,
    result: Option<oneshot::Receiver<Result<(), String>>>,
}

impl PreparedAcceptanceDecision {
    pub fn commit(mut self) -> Result<CommittedAcceptance> {
        if let Some(decision) = self.decision.take() {
            decision.send(StartupDecision::Commit).map_err(|_| {
                anyhow::anyhow!("native parent attach startup owner is unavailable")
            })?;
        }
        Ok(CommittedAcceptance {
            result: self.result.take(),
        })
    }
}

pub struct CommittedAcceptance {
    result: Option<oneshot::Receiver<Result<(), String>>>,
}

impl CommittedAcceptance {
    pub async fn finish(self) -> Result<()> {
        let Some(result) = self.result else {
            return Ok(());
        };
        result
            .await
            .context("native parent attach startup owner stopped during committed release")?
            .map_err(anyhow::Error::msg)
    }
}

#[derive(Debug)]
struct StartupAcceptance {
    deadline: std::time::Instant,
    reply: oneshot::Sender<Result<PreparedAcceptanceDecision, String>>,
}

#[derive(Debug)]
enum StartupDecision {
    Commit,
}

struct ParentAttachPreparation {
    target: PreparedLiveTarget,
    startup: Option<windows_startup::PreparedHost>,
}

/// Process/configuration choices captured before the catalog worker starts.
/// Destination-specific storage environment is derived later from the same
/// frozen discovery scope; no request reads process environment or cwd.
#[derive(Clone, Debug)]
pub struct ParentAttachStartup {
    executable: PathBuf,
    config: Option<PathBuf>,
    verbosity: u8,
    log: Option<PathBuf>,
    test_harness_helper: Option<String>,
}

impl ParentAttachStartup {
    pub fn capture(
        executable: &Path,
        config: Option<&Path>,
        verbosity: u8,
        log: Option<&Path>,
    ) -> std::io::Result<Self> {
        let directory = std::env::current_dir()?;
        let startup = Self {
            executable: absolute_from(&directory, executable),
            config: config.map(|path| absolute_from(&directory, path)),
            verbosity,
            log: log.map(|path| absolute_from(&directory, path)),
            test_harness_helper: None,
        };
        validate_path(&startup.executable).map_err(std::io::Error::other)?;
        if let Some(path) = startup.config.as_deref() {
            validate_path(path).map_err(std::io::Error::other)?;
        }
        if let Some(path) = startup.log.as_deref() {
            validate_path(path).map_err(std::io::Error::other)?;
        }
        Ok(startup)
    }

    #[doc(hidden)]
    pub fn with_test_harness_helper(mut self, helper: impl Into<String>) -> Self {
        self.test_harness_helper = Some(helper.into());
        self
    }

    fn for_layout(&self, layout: &ResolvedLayout) -> Result<HostStartup> {
        let mut startup = HostStartup::new(self.executable.clone());
        startup.config = self.config.clone();
        startup.verbosity = self.verbosity;
        startup.log = self.log.clone();
        startup.env = layout.detached_environment()?;
        if let Some(helper) = &self.test_harness_helper {
            startup = startup.with_test_harness_helper(helper.clone());
            startup.env.push((
                "RUNYTE_TEST_PARENT_ATTACH_PROJECT".into(),
                Some(layout.project_root().as_os_str().to_owned()),
            ));
            startup.env.push((
                "RUNYTE_TEST_PARENT_ATTACH_STATE".into(),
                Some(layout.state_root().as_os_str().to_owned()),
            ));
            startup.env.push((
                "XDG_CONFIG_HOME".into(),
                Some(layout.project_root().join(".test-config").into_os_string()),
            ));
        }
        Ok(startup)
    }
}

fn absolute_from(directory: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        directory.join(path)
    }
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
    /// Resolves a terminal-authored selector against a fresh complete catalog.
    /// A stopped or previously unseen project is initialized and started by the
    /// parent-side worker, outside the requesting terminal's ConPTY job.
    pub async fn prepare_parent_attach(
        &self,
        selector: &Path,
        working_directory: &Path,
    ) -> Result<PreparedLiveTarget> {
        self.check_open().map_err(anyhow::Error::msg)?;
        validate_path(selector).map_err(anyhow::Error::msg)?;
        validate_path(working_directory).map_err(anyhow::Error::msg)?;
        ensure!(
            working_directory.is_absolute(),
            "parent attach working directory must be absolute"
        );
        let (reply, result) = oneshot::channel();
        self.requests
            .try_send(Request::PrepareParentAttach {
                selector: selector.to_owned(),
                working_directory: working_directory.to_owned(),
                reply,
            })
            .map_err(request_error)?;
        let mut stop = self.stop.clone();
        ensure!(!*stop.borrow(), "native session service is shutting down");
        tokio::time::timeout(PARENT_ATTACH_BUDGET, async {
            tokio::select! {
                biased;
                changed = stop.changed() => {
                    match changed {
                        Ok(()) => anyhow::bail!("native session service is shutting down"),
                        Err(_) => anyhow::bail!("native session service is unavailable"),
                    }
                }
                result = result => result
                    .context("native session service stopped before preparing parent attach")?,
            }
        })
        .await
        .context("native parent attach preparation timed out")?
    }

    /// Prepares only the exact live publication named by a displayed native
    /// row. It never resolves a replacement through its project, name or PID.
    pub async fn prepare_selected_live(
        &self,
        selection: WorkspaceSelection,
    ) -> Result<PreparedLiveTarget> {
        self.check_open().map_err(anyhow::Error::msg)?;
        validate_selection(&selection).map_err(anyhow::Error::msg)?;
        ensure!(
            selection.publication_key().is_some(),
            "stopped session has no live publication"
        );
        let (reply, result) = oneshot::channel();
        self.requests
            .try_send(Request::PrepareSelectedLive { selection, reply })
            .map_err(request_error)?;
        let mut stop = self.stop.clone();
        ensure!(!*stop.borrow(), "native session service is shutting down");
        tokio::time::timeout(PREPARE_BUDGET, async {
            tokio::select! {
                biased;
                changed = stop.changed() => {
                    match changed {
                        Ok(()) => anyhow::bail!("native session service is shutting down"),
                        Err(_) => anyhow::bail!("native session service is unavailable"),
                    }
                }
                result = result => result
                    .context("native session service stopped before preparing selection")?,
            }
        })
        .await
        .context("native session preparation timed out")?
    }

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

fn request_error<T>(error: mpsc::error::TrySendError<T>) -> anyhow::Error {
    match error {
        mpsc::error::TrySendError::Full(_) => {
            anyhow::anyhow!("native session service queue is full")
        }
        mpsc::error::TrySendError::Closed(_) => {
            anyhow::anyhow!("native session service is unavailable")
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
        Self::spawn_inner(scope, current, configured_state, None, None)
    }

    pub fn spawn_with_parent_attach(
        scope: DiscoveryScope,
        current: Option<KnownReadLocation>,
        configured_state: PathBuf,
        startup: ParentAttachStartup,
    ) -> std::io::Result<(WorkspaceServiceHandle, Self, mpsc::Receiver<WorkspaceEvent>)> {
        Self::spawn_inner(scope, current, configured_state, Some(startup), None)
    }

    #[cfg(test)]
    fn spawn_with_snapshot(
        scope: DiscoveryScope,
        current: Option<KnownReadLocation>,
        configured_state: PathBuf,
        snapshot: Option<ControlSnapshot>,
    ) -> std::io::Result<(WorkspaceServiceHandle, Self, mpsc::Receiver<WorkspaceEvent>)> {
        Self::spawn_inner(scope, current, configured_state, None, snapshot)
    }

    fn spawn_inner(
        scope: DiscoveryScope,
        current: Option<KnownReadLocation>,
        configured_state: PathBuf,
        parent_attach: Option<ParentAttachStartup>,
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
                                parent_attach,
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
    parent_attach: Option<ParentAttachStartup>,
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

    async fn prepare_selected_live(
        &mut self,
        selection: &WorkspaceSelection,
    ) -> Result<PreparedLiveTarget> {
        self.recover()?;
        ensure!(
            selection.publication_key().is_some(),
            "stopped session has no live publication"
        );
        // Observe again through this worker's frozen scope, current known-ready
        // location and hidden-publication choice. A cached row cannot prove its
        // publication still exists, and another scope must not replace it.
        let snapshot = self.observe_cancellable(self.include_hidden).await?;
        let index = snapshot
            .history()
            .select_selection(selection)?
            .context("selected session changed; choose it again")?;
        let HistoryTarget::Live { publication, .. } = snapshot
            .history()
            .target(index)
            .context("selected session is unavailable")?
        else {
            anyhow::bail!("stopped session has no live publication");
        };
        ensure!(
            publication.metadata().protocol == crate::protocol::VERSION,
            "incompatible session cannot be attached"
        );
        let prepared = PreparedLiveTarget {
            metadata: publication.metadata().clone(),
            peer: Arc::clone(publication.peer()),
            acceptance: None,
        };
        self.snapshot = Some(snapshot);
        Ok(prepared)
    }

    async fn prepare_parent_attach(
        &mut self,
        selector: &Path,
        working_directory: &Path,
        reply: &mut oneshot::Sender<Result<PreparedLiveTarget>>,
    ) -> Result<ParentAttachPreparation> {
        self.recover()?;
        let mut stop = self.stop.clone();
        ensure!(!reply.is_closed(), "parent attach requester disconnected");
        ensure!(!*stop.borrow(), "native session service is shutting down");
        let snapshot = tokio::select! {
            biased;
            _ = reply.closed() => anyhow::bail!("parent attach requester disconnected"),
            changed = stop.changed() => {
                match changed {
                    Ok(()) => anyhow::bail!("native session service is shutting down"),
                    Err(_) => anyhow::bail!("native session service is unavailable"),
                }
            }
            result = self.observe(self.include_hidden) => result?,
        };
        let target = snapshot
            .history()
            .select(selector, Some(working_directory))?;
        let project = match target {
            Some(HistoryTarget::Live { publication, .. }) => {
                ensure!(
                    publication.metadata().protocol == crate::protocol::VERSION,
                    "incompatible session cannot be attached"
                );
                let prepared = PreparedLiveTarget {
                    metadata: publication.metadata().clone(),
                    peer: Arc::clone(publication.peer()),
                    acceptance: None,
                };
                ensure!(!reply.is_closed(), "parent attach requester disconnected");
                ensure!(!*stop.borrow(), "native session service is shutting down");
                self.snapshot = Some(snapshot);
                return Ok(ParentAttachPreparation {
                    target: prepared,
                    startup: None,
                });
            }
            Some(HistoryTarget::Stopped { row }) => row.project_root.clone(),
            None if selector.is_absolute() => selector.to_owned(),
            None => working_directory.join(selector),
        };
        self.snapshot = Some(snapshot);
        let startup = self
            .parent_attach
            .as_ref()
            .context("native parent attach startup is unavailable")?;
        let layout = self
            .scope
            .initialize_layout(&project, &self.configured_state)?;
        let location = layout.publication_location()?;
        ensure!(!reply.is_closed(), "parent attach requester disconnected");
        ensure!(!*stop.borrow(), "native session service is shutting down");
        let cancellation = async {
            tokio::select! {
                biased;
                _ = reply.closed() => "parent attach requester disconnected",
                changed = stop.changed() => if changed.is_ok() {
                    "native session service is shutting down"
                } else {
                    "native session service is unavailable"
                },
            }
        };
        let startup = windows_startup::prepare_detached_host_cancellable(
            &location,
            startup.for_layout(&layout)?,
            cancellation,
        )
        .await?;
        let target = PreparedLiveTarget {
            metadata: startup.metadata().clone(),
            peer: Arc::clone(startup.peer()),
            acceptance: None,
        };
        let startup = (startup.disposition() == StartDisposition::Started).then_some(startup);
        Ok(ParentAttachPreparation { target, startup })
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
            Request::PrepareSelectedLive {
                selection,
                mut reply,
            } => {
                if reply.is_closed() {
                    return Ok(());
                }
                let result = tokio::select! {
                    biased;
                    _ = reply.closed() => return Ok(()),
                    result = self.prepare_selected_live(&selection) => result,
                };
                let _ = reply.send(result);
                return Ok(());
            }
            Request::PrepareParentAttach {
                selector,
                working_directory,
                mut reply,
            } => {
                // Skip requests abandoned while queued. Once startup begins,
                // cancellation remains worker-owned until a provisional host
                // is settled; an authenticated existing winner is preserved.
                if reply.is_closed() {
                    return Ok(());
                }
                let result = self
                    .prepare_parent_attach(&selector, &working_directory, &mut reply)
                    .await;
                let mut prepared = match result {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        let _ = reply.send(Err(error));
                        return Ok(());
                    }
                };
                let Some(startup) = prepared.startup.take() else {
                    let _ = reply.send(Ok(prepared.target));
                    return Ok(());
                };
                let (acceptance, decision) = oneshot::channel();
                prepared.target.acceptance = Some(acceptance);
                if reply.send(Ok(prepared.target)).is_err() {
                    startup.settle().await?;
                    return Ok(());
                }
                settle_startup_decision(startup, decision, &mut self.stop).await?;
                return Ok(());
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

async fn settle_startup_decision(
    startup: windows_startup::PreparedHost,
    decision: oneshot::Receiver<StartupAcceptance>,
    stop: &mut watch::Receiver<bool>,
) -> Result<()> {
    let acceptance = tokio::select! {
        biased;
        _ = stop.changed() => None,
        decision = decision => decision.ok(),
    };
    let Some(acceptance) = acceptance else {
        return startup.settle().await;
    };
    if *stop.borrow() || acceptance.reply.is_closed() || Instant::now() >= acceptance.deadline {
        let _ = acceptance.reply.send(Err(
            "native parent attach authority ended before destination acceptance".to_owned(),
        ));
        return startup.settle().await;
    }
    let (decision, decision_rx) = oneshot::channel();
    let (result, result_rx) = oneshot::channel();
    if acceptance
        .reply
        .send(Ok(PreparedAcceptanceDecision {
            decision: Some(decision),
            result: Some(result_rx),
        }))
        .is_err()
    {
        return startup.settle().await;
    }
    let commit = tokio::select! {
        biased;
        decision = decision_rx => matches!(decision, Ok(StartupDecision::Commit)),
        _ = stop.changed() => false,
    };
    if !commit {
        let _ = result.send(Err(
            "native parent attach authority ended before destination commit".to_owned(),
        ));
        return startup.settle().await;
    }
    #[cfg(test)]
    let project = startup.metadata().project_root().ok();
    match startup.accept_or_settle().await {
        windows_startup::PreparedAcceptance::Accepted => {
            #[cfg(test)]
            if let Some(project) = project.as_deref() {
                wait_after_parent_release(project).await?;
            }
            let _ = result.send(Ok(()));
            Ok(())
        }
        windows_startup::PreparedAcceptance::Refused { error, cleanup } => {
            let _ = result.send(Err(error.to_string()));
            cleanup
        }
    }
}

#[cfg(test)]
async fn wait_after_parent_release(project: &Path) -> Result<()> {
    if !project.join("hold-parent-commit-ack").exists() {
        return Ok(());
    }
    let pending = project.join("parent-released.pending");
    std::fs::write(&pending, b"released")?;
    std::fs::rename(pending, project.join("parent-released"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !project.join("allow-parent-commit-ack").exists() {
        ensure!(
            Instant::now() < deadline,
            "parent commit acknowledgement fixture was not released"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
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
