// SPDX-License-Identifier: MPL-2.0

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{error::Error, fmt};

use anyhow::Result;
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, CommandOutcome, FrameGeometry, PointerOutcome, PreparedView},
    buffer::FileObservationEvent,
    command::CommandInvocation,
    file_picker::{FilePickerEvent, FileScanner},
    git::{
        GitOperation, GitRequestId, GitResponse, GitServiceEvent, GitServiceState, RefreshSpec,
        RepositoryGeneration,
    },
    git_monitor::GitInvalidation,
    input::{InputEvent, PointerEvent},
    key_hints::{KeyHintState, key_hint_description, key_hint_keys, key_hint_layout},
    lsp::{LspEvent, LspHandle},
    snapshot::{EditorSnapshot, OverlayIdentity, OverlayKind, OverlayRow, OverlaySnapshot},
    syntax::{SyntaxEvent, SyntaxHandle},
    text::Transaction,
    workspace_search::WorkspaceSearchEvent,
};

use super::buffers::WaitRequest;
use super::{
    BufferContents, BufferId, BufferMetadata, BufferRevision, CancellationToken, ServiceKind,
    ServiceLane, ServiceLifecycle, ServiceRequestId, ServiceStateError, ServiceSubmitError,
    ServiceUpdate, ServiceWorker, WaitStatus, WaitToken, WorkspaceIdentity,
};

const MAX_BUFFER_READ_BYTES: usize = 1024 * 1024;
const MAX_WAIT_REQUESTS: usize = 256;
const SESSION_PREVIEW_PANES: usize = 8;
const SESSION_PREVIEW_LINES: usize = 8;
const SESSION_PREVIEW_COLUMNS: usize = 240;
const SESSION_PREVIEW_OTHER_RESOURCES: usize = 8;
const AUTOMATIC_GIT_RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BufferRequestError {
    Unknown(BufferId),
    Closed(BufferId),
    Stale {
        expected: BufferRevision,
        actual: BufferRevision,
    },
    InvalidTransaction(String),
    Refused(String),
}

impl fmt::Display for BufferRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(formatter, "unknown buffer {id}"),
            Self::Closed(id) => write!(formatter, "buffer {id} is closed"),
            Self::Stale { expected, actual } => write!(
                formatter,
                "stale buffer revision: expected {}, current {}",
                expected.get(),
                actual.get()
            ),
            Self::InvalidTransaction(reason) | Self::Refused(reason) => formatter.write_str(reason),
        }
    }
}

impl Error for BufferRequestError {}

#[derive(Debug)]
pub enum HostServiceSubmitError {
    Lifecycle(ServiceStateError),
    Lane(ServiceSubmitError),
}

impl fmt::Display for HostServiceSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lifecycle(error) => error.fmt(formatter),
            Self::Lane(error) => error.fmt(formatter),
        }
    }
}

impl Error for HostServiceSubmitError {}

/// Monotonic identity of prepared pointer-hit-test geometry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameId(u64);

impl FrameId {
    pub(crate) const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One immutable normal-editor frame returned to a frontend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostFrame {
    pub id: FrameId,
    pub active_buffer: BufferId,
    pub active_revision: BufferRevision,
    pub editor: EditorSnapshot,
    pub overlays: Vec<OverlaySnapshot>,
}

/// A bounded, presentation-neutral reading of the live panes a person would
/// return to after attaching to a persistent session.
///
/// This is deliberately not a miniature [`HostFrame`]. Preparing a frame for
/// synthetic thumbnail geometry would mutate viewport state and resize PTYs;
/// a session preview only reads the pane state the host already owns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionPreview {
    /// Every pane in the retained split layout, including panes hidden behind
    /// a maximized presentation.
    pub layout_panes: usize,
    /// Visible panes included below. Bounded independently from the layout so
    /// an unusually split workspace cannot make a control response unbounded.
    pub panes: Vec<SessionPreviewPane>,
    pub omitted_panes: usize,
    /// Open resources not represented by a visible pane, as structural names
    /// such as `[file] path` and `[terminal] shell`.
    pub other_resources: Vec<String>,
    pub omitted_resources: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionPreviewPane {
    pub active: bool,
    pub title: String,
    pub kind: SessionPreviewPaneKind,
    /// One-based document row of `lines[0]` for a buffer, and `None` for a
    /// terminal tail whose retained scrollback has no document coordinates.
    pub start_line: Option<usize>,
    /// Buffer rows begin at the pane's retained viewport anchor; terminal
    /// rows are the tail of its parsed plain-text screen and scrollback.
    pub lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionPreviewPaneKind {
    Buffer { dirty: bool, read_only: bool },
    Terminal { live: bool },
}

fn session_preview_line(line: &str) -> String {
    line.trim_end_matches(['\r', '\n'])
        .chars()
        .take(SESSION_PREVIEW_COLUMNS)
        .map(|character| {
            if character == '\t' || !character.is_control() {
                character
            } else {
                '\u{fffd}'
            }
        })
        .collect()
}

/// Typed direct-host commands. Serialization DTOs are deliberately separate.
#[derive(Clone, Debug)]
pub enum HostCommand {
    Input(InputEvent),
    Pointer {
        event: PointerEvent,
        frame: FrameId,
        repetitions: u16,
    },
}

/// Owned service events accepted on the host thread.
#[derive(Debug)]
pub enum HostEvent {
    Syntax(SyntaxEvent),
    Lsp(LspEvent),
    FilePicker(FilePickerEvent),
    WorkspaceSearch(WorkspaceSearchEvent),
    FileObservation(FileObservationEvent),
    GitInvalidation(GitInvalidation),
    Git(GitServiceEvent),
    Terminal(crate::terminal::TerminalOutput),
    #[cfg(unix)]
    Workspace(super::WorkspaceEvent),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostInputOutcome {
    Applied,
    AppliedWithoutVisualChange,
    IgnoredStaleFrame,
}

#[derive(Clone, Debug)]
struct PreparedFrame {
    id: FrameId,
    pointer_compatible_since: FrameId,
    view: PreparedView,
}

#[derive(Clone, Debug)]
struct PendingAutomaticGitRefresh {
    id: GitRequestId,
    spec: RefreshSpec,
    fallback: bool,
}

#[derive(Clone, Debug)]
struct AppliedGitReconciliation {
    repository: PathBuf,
    started_at: Instant,
    spec: RefreshSpec,
}

#[derive(Clone, Debug)]
struct CompletedGitSnapshot {
    repository: PathBuf,
    generation: RepositoryGeneration,
    started_at: Instant,
    spec: RefreshSpec,
    state: GitServiceState,
    coalesced: bool,
    mutation: bool,
}

/// The only owner allowed to mutate one live editor/application workspace.
///
/// Standalone mode uses this value directly. Persistent mode will keep the
/// same owner and place a bounded transport adapter in front of it.
pub struct WorkspaceHost {
    identity: WorkspaceIdentity,
    app: App,
    services: ServiceLifecycle,
    next_frame: u64,
    prepared: Option<PreparedFrame>,
    last_git_refresh: Instant,
    last_automatic_git_refresh_request: Option<Instant>,
    git_dirty: bool,
    git_reconciled: Option<RefreshSpec>,
    pending_git_refresh: Option<PendingAutomaticGitRefresh>,
    last_git_reconciliation: Option<AppliedGitReconciliation>,
    latest_git_observation: Option<Instant>,
    next_git_retry: Option<Instant>,
    next_wait_token: u64,
    wait_requests: HashMap<WaitToken, WaitRequest>,
    wait_order: VecDeque<WaitToken>,
}

/// Live host state that an automatic or ordinary shutdown must not abandon.
///
/// This is the authoritative lifecycle answer. Callers may present its fields
/// differently, but must not independently reconstruct whether the host is
/// safe to retire or stop.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProtectedHostState {
    pub unsaved_buffers: usize,
    pub pending_wait_requests: usize,
    pub live_terminals: usize,
}

impl ProtectedHostState {
    pub const fn is_empty(self) -> bool {
        self.unsaved_buffers == 0 && self.pending_wait_requests == 0 && self.live_terminals == 0
    }

    pub fn refusal(self) -> String {
        let mut parts = Vec::new();
        if self.unsaved_buffers > 0 {
            parts.push(format!(
                "{} unsaved buffer{}",
                self.unsaved_buffers,
                if self.unsaved_buffers == 1 { "" } else { "s" }
            ));
        }
        if self.pending_wait_requests > 0 {
            parts.push(format!(
                "{} pending --wait request{}",
                self.pending_wait_requests,
                if self.pending_wait_requests == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if self.live_terminals > 0 {
            parts.push(format!(
                "{} live terminal{}",
                self.live_terminals,
                if self.live_terminals == 1 { "" } else { "s" }
            ));
        }
        format!("workspace has {}", parts.join(" and "))
    }
}

impl WorkspaceHost {
    pub fn new(app: App) -> Self {
        let identity = WorkspaceIdentity::from_canonical(app.project_root.clone());
        Self {
            identity,
            app,
            services: ServiceLifecycle::new(256),
            next_frame: 1,
            prepared: None,
            last_git_refresh: Instant::now(),
            last_automatic_git_refresh_request: None,
            git_dirty: false,
            git_reconciled: None,
            pending_git_refresh: None,
            last_git_reconciliation: None,
            latest_git_observation: None,
            next_git_retry: None,
            next_wait_token: 1,
            wait_requests: HashMap::new(),
            wait_order: VecDeque::new(),
        }
    }

    pub fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub fn services(&self) -> &ServiceLifecycle {
        &self.services
    }

    pub fn services_mut(&mut self) -> &mut ServiceLifecycle {
        &mut self.services
    }

    pub fn buffer_metadata(&self) -> Vec<BufferMetadata> {
        self.app
            .buffers
            .iter()
            .enumerate()
            .map(|(index, buffer)| BufferMetadata {
                id: BufferId::from_index(index),
                revision: BufferRevision::from_raw(buffer.revision()),
                path: buffer.path.clone(),
                name: buffer.display_name(),
                dirty: buffer.dirty,
                read_only: buffer.is_read_only(),
                closed: self.app.host_buffer_is_closed(index),
            })
            .collect()
    }

    /// Reads a compact overview without preparing new viewport geometry.
    ///
    /// Buffer snippets begin where each pane was last scrolled. Terminal
    /// snippets use parsed plain text rather than raw escape sequences. The
    /// active/maximized pane rules mirror what attachment will show, while
    /// hidden resources remain names only.
    pub fn session_preview(&self) -> SessionPreview {
        let mut layout_panes = Vec::new();
        self.app.layout.panes(&mut layout_panes);
        let mut visible_panes = layout_panes
            .iter()
            .copied()
            .find(|pane| self.app.maximized_view(*pane).is_some())
            .map_or_else(|| layout_panes.clone(), |pane| vec![pane]);
        if let Some(position) = visible_panes
            .iter()
            .position(|pane| *pane == self.app.active_pane)
        {
            let active = visible_panes.remove(position);
            visible_panes.insert(0, active);
        }

        let represented_buffers = visible_panes
            .iter()
            .filter_map(|pane_id| self.app.panes.get(pane_id))
            .filter(|pane| pane.terminal.is_none())
            .map(|pane| pane.buffer)
            .collect::<HashSet<_>>();
        let represented_terminals = visible_panes
            .iter()
            .filter_map(|pane_id| self.app.panes.get(pane_id)?.terminal)
            .collect::<HashSet<_>>();

        let panes = visible_panes
            .iter()
            .take(SESSION_PREVIEW_PANES)
            .filter_map(|pane_id| {
                let pane = self.app.panes.get(pane_id)?;
                if let Some(id) = pane.terminal
                    && let Some(terminal) = self.app.terminals.get(id)
                {
                    let text = terminal.plain_text();
                    let mut lines = text
                        .lines()
                        .rev()
                        .take(SESSION_PREVIEW_LINES)
                        .map(session_preview_line)
                        .collect::<Vec<_>>();
                    lines.reverse();
                    return Some(SessionPreviewPane {
                        active: *pane_id == self.app.active_pane,
                        title: session_preview_line(&terminal.display_name()),
                        kind: SessionPreviewPaneKind::Terminal {
                            live: terminal.live(),
                        },
                        start_line: None,
                        lines,
                    });
                }

                let buffer = self.app.buffers.get(pane.buffer)?;
                let start = pane.scroll_row.min(buffer.len_lines().saturating_sub(1));
                let lines = (start..buffer.len_lines())
                    .take(SESSION_PREVIEW_LINES)
                    .map(|row| session_preview_line(&buffer.line_string(row)))
                    .collect();
                Some(SessionPreviewPane {
                    active: *pane_id == self.app.active_pane,
                    title: session_preview_line(&buffer.pane_title()),
                    kind: SessionPreviewPaneKind::Buffer {
                        dirty: buffer.dirty,
                        read_only: buffer.is_read_only(),
                    },
                    start_line: Some(start + 1),
                    lines,
                })
            })
            .collect::<Vec<_>>();

        let mut resources = self
            .app
            .buffers
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                !represented_buffers.contains(index) && !self.app.host_buffer_is_closed(*index)
            })
            .map(|(_, buffer)| {
                session_preview_line(&format!(
                    "{}{}",
                    buffer.pane_title(),
                    if buffer.dirty { " [+]" } else { "" }
                ))
            })
            .chain(
                self.app
                    .terminals
                    .iter()
                    .filter(|terminal| !represented_terminals.contains(&terminal.id()))
                    .map(|terminal| session_preview_line(&terminal.display_name())),
            )
            .collect::<Vec<_>>();
        let omitted_resources = resources
            .len()
            .saturating_sub(SESSION_PREVIEW_OTHER_RESOURCES);
        resources.truncate(SESSION_PREVIEW_OTHER_RESOURCES);

        SessionPreview {
            layout_panes: layout_panes.len(),
            omitted_panes: visible_panes.len().saturating_sub(panes.len()),
            panes,
            other_resources: resources,
            omitted_resources,
        }
    }

    /// How many live buffers hold unsaved work.
    ///
    /// The one measure of whether this workspace is clean, so the refusal to
    /// stop, idle retirement, and every listing that reports a count agree by
    /// construction. See [`crate::buffer::Buffer::holds_unsaved_work`] for what
    /// it excludes.
    pub fn unsaved_buffers(&self) -> usize {
        self.app
            .buffers
            .iter()
            .enumerate()
            .filter(|(index, buffer)| {
                buffer.holds_unsaved_work() && !self.app.host_buffer_is_closed(*index)
            })
            .count()
    }

    /// How many buffers this host holds open, unsaved or not.
    ///
    /// The count a session manager states as a fact about the session, next to
    /// [`Self::unsaved_buffers`]. Buffers the host has closed are excluded, so
    /// the two counts are read against the same set.
    pub fn open_buffer_count(&self) -> usize {
        self.app
            .buffers
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.app.host_buffer_is_closed(*index))
            .count()
    }

    /// The single lifecycle summary used by retirement, inspection and stop.
    pub fn protected_state(&self) -> ProtectedHostState {
        ProtectedHostState {
            unsaved_buffers: self.unsaved_buffers(),
            pending_wait_requests: self
                .wait_requests
                .values()
                .filter(|request| matches!(request.status, WaitStatus::Pending { .. }))
                .count(),
            live_terminals: self
                .app
                .terminals
                .iter()
                .filter(|session| session.live())
                .count(),
        }
    }

    /// Whether an unattached persistent host may retire without losing work
    /// or abandoning a caller waiting for a buffer.
    pub fn may_retire_idle(&self) -> bool {
        self.protected_state().is_empty() && self.app.terminals.is_empty()
    }

    pub fn read_buffer(&self, id: BufferId) -> Result<BufferContents, BufferRequestError> {
        let index = self.live_buffer_index(id)?;
        let buffer = &self.app.buffers[index];
        let text = buffer.to_string();
        let (text, truncated) = bounded_utf8(&text, MAX_BUFFER_READ_BYTES);
        Ok(BufferContents {
            metadata: BufferMetadata {
                id,
                revision: BufferRevision::from_raw(buffer.revision()),
                path: buffer.path.clone(),
                name: buffer.display_name(),
                dirty: buffer.dirty,
                read_only: buffer.is_read_only(),
                closed: false,
            },
            text,
            truncated,
        })
    }

    pub fn open_buffer(&mut self, path: PathBuf, activate: bool) -> Result<BufferId> {
        let index = self.app.host_open_file(path, activate)?;
        Ok(BufferId::from_index(index))
    }

    /// Opens every path or none of them. See `App::host_open_files`.
    pub fn open_buffers(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        activate: bool,
    ) -> Result<Vec<BufferId>> {
        let opened = self
            .app
            .host_open_files(paths.into_iter().collect(), activate)?;
        Ok(opened.into_iter().map(BufferId::from_index).collect())
    }

    pub fn apply_expected_transaction(
        &mut self,
        id: BufferId,
        expected: BufferRevision,
        transaction: Transaction,
    ) -> Result<BufferRevision, BufferRequestError> {
        let index = self.live_buffer_index(id)?;
        let actual = BufferRevision::from_raw(self.app.buffers[index].revision());
        if actual != expected {
            return Err(BufferRequestError::Stale { expected, actual });
        }
        let text_len = self.app.buffers[index].len_chars();
        if let Some((change_index, change)) = transaction
            .changes()
            .iter()
            .enumerate()
            .find(|(_, change)| change.from > text_len || change.to > text_len)
        {
            return Err(BufferRequestError::InvalidTransaction(format!(
                "change {change_index} range {}..{} exceeds buffer length {text_len}",
                change.from, change.to
            )));
        }
        if !self.app.apply_to_buffer(index, &transaction) {
            return Err(BufferRequestError::Refused(
                "transaction was empty or the buffer is read-only".to_owned(),
            ));
        }
        Ok(BufferRevision::from_raw(self.app.buffers[index].revision()))
    }

    pub fn save_buffer(&mut self, id: BufferId) -> Result<BufferRevision> {
        let index = self.live_buffer_index(id).map_err(anyhow::Error::from)?;
        self.app.host_save_buffer(index)?;
        Ok(BufferRevision::from_raw(self.app.buffers[index].revision()))
    }

    pub fn close_buffer(&mut self, id: BufferId, discard: bool) -> Result<()> {
        let index = self.live_buffer_index(id).map_err(anyhow::Error::from)?;
        self.app.host_close_buffer(index, discard)?;
        self.reconcile_wait_requests();
        Ok(())
    }

    pub fn create_wait_request(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        activate: bool,
    ) -> Result<(WaitToken, Vec<BufferId>)> {
        self.prune_terminal_wait_requests();
        anyhow::ensure!(
            self.wait_requests.len() < MAX_WAIT_REQUESTS,
            "workspace host has {MAX_WAIT_REQUESTS} live wait requests; complete or cancel one before creating another"
        );
        let paths = paths.into_iter().collect::<Vec<_>>();
        anyhow::ensure!(!paths.is_empty(), "a wait request needs at least one path");
        // Atomic, so a later path that cannot be opened leaves no buffer behind
        // for a token that was never allocated to account for it.
        let mut buffers = Vec::with_capacity(paths.len());
        let pending_wait_buffers = self
            .wait_requests
            .values()
            .filter(|request| matches!(request.status, WaitStatus::Pending { .. }))
            .flat_map(|request| &request.buffers)
            .filter_map(|buffer| buffer.index())
            .collect::<HashSet<_>>();
        let opened = self
            .app
            .host_open_wait_files(paths, activate, &pending_wait_buffers)?;
        for buffer in opened.into_iter().map(BufferId::from_index) {
            if !buffers.contains(&buffer) {
                buffers.push(buffer);
            }
        }
        let token = WaitToken::new(self.next_wait_token);
        self.next_wait_token = self
            .next_wait_token
            .checked_add(1)
            .expect("wait token identity exhausted");
        self.wait_requests.insert(
            token,
            WaitRequest {
                buffers: buffers.clone(),
                completed: Vec::new(),
                status: WaitStatus::Pending {
                    buffers: buffers.clone(),
                    remaining: buffers.clone(),
                },
            },
        );
        self.wait_order.push_back(token);
        Ok((token, buffers))
    }

    pub fn wait_status(&self, token: WaitToken) -> Option<WaitStatus> {
        self.wait_requests
            .get(&token)
            .map(|request| request.status.clone())
    }

    pub fn complete_wait_buffer(&mut self, token: WaitToken, buffer: BufferId) -> Result<()> {
        let request = self
            .wait_requests
            .get(&token)
            .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}"))?;
        anyhow::ensure!(
            matches!(request.status, WaitStatus::Pending { .. }),
            "wait request {token} is already terminal"
        );
        anyhow::ensure!(
            request.buffers.contains(&buffer),
            "buffer is not part of this wait request"
        );
        if request.completed.contains(&buffer) {
            return Ok(());
        }
        let index = self
            .live_buffer_index(buffer)
            .map_err(anyhow::Error::from)?;
        anyhow::ensure!(
            !self.app.buffers[index].dirty,
            "modified wait buffers must be saved, closed with confirmation, or explicitly discarded before completion"
        );
        let request = self
            .wait_requests
            .get_mut(&token)
            .expect("wait request was checked before buffer validation");
        request.completed.push(buffer);
        update_wait_status(request);
        Ok(())
    }

    pub fn complete_wait_request(&mut self, token: WaitToken) -> Result<()> {
        self.reconcile_wait_requests();
        let request = self
            .wait_requests
            .get(&token)
            .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}"))?;
        match &request.status {
            WaitStatus::Completed => return Ok(()),
            WaitStatus::Cancelled { .. } => {
                anyhow::bail!("wait request {token} is already cancelled")
            }
            WaitStatus::Pending { remaining, .. } => {
                for buffer in remaining {
                    let index = self
                        .live_buffer_index(*buffer)
                        .map_err(anyhow::Error::from)?;
                    anyhow::ensure!(
                        !self.app.buffers[index].dirty,
                        "modified wait buffers must be saved before completing the request"
                    );
                }
            }
        }
        let request = self
            .wait_requests
            .get_mut(&token)
            .expect("wait request was checked before buffer validation");
        request.completed = request.buffers.clone();
        update_wait_status(request);
        Ok(())
    }

    pub fn cancel_wait(&mut self, token: WaitToken, reason: impl Into<String>) -> Result<()> {
        let request = self
            .wait_requests
            .get_mut(&token)
            .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}"))?;
        if matches!(request.status, WaitStatus::Pending { .. }) {
            request.status = WaitStatus::Cancelled {
                reason: reason.into(),
            };
        }
        Ok(())
    }

    pub fn reconcile_wait_requests(&mut self) {
        for request in self.wait_requests.values_mut() {
            if !matches!(request.status, WaitStatus::Pending { .. }) {
                continue;
            }
            for buffer in &request.buffers {
                if buffer
                    .index()
                    .is_some_and(|index| self.app.host_buffer_is_closed(index))
                    && !request.completed.contains(buffer)
                {
                    request.completed.push(*buffer);
                }
            }
            update_wait_status(request);
        }
    }

    pub fn cancel_all_waits(&mut self, reason: &str) {
        for request in self.wait_requests.values_mut() {
            if matches!(request.status, WaitStatus::Pending { .. }) {
                request.status = WaitStatus::Cancelled {
                    reason: reason.to_owned(),
                };
            }
        }
    }

    fn prune_terminal_wait_requests(&mut self) {
        while self.wait_requests.len() >= MAX_WAIT_REQUESTS {
            let Some(index) = self.wait_order.iter().position(|token| {
                self.wait_requests
                    .get(token)
                    .is_some_and(|request| !matches!(request.status, WaitStatus::Pending { .. }))
            }) else {
                break;
            };
            let token = self
                .wait_order
                .remove(index)
                .expect("wait order index came from the same deque");
            self.wait_requests.remove(&token);
        }
    }

    fn live_buffer_index(&self, id: BufferId) -> Result<usize, BufferRequestError> {
        let Some(index) = id.index().filter(|index| *index < self.app.buffers.len()) else {
            return Err(BufferRequestError::Unknown(id));
        };
        if self.app.host_buffer_is_closed(index) {
            return Err(BufferRequestError::Closed(id));
        }
        Ok(index)
    }

    fn queue_service(
        &mut self,
        kind: ServiceKind,
        operation: impl Into<String>,
        target: impl Into<String>,
        cancellable: bool,
    ) -> Result<(ServiceRequestId, CancellationToken), ServiceStateError> {
        self.services.queue(kind, operation, target, cancellable)
    }

    /// Atomically registers and submits one typed background request. A lane
    /// refusal is recorded as a terminal failure immediately, so overload can
    /// never strand phantom queued work or consume the live-request bound.
    pub fn submit_service<W: ServiceWorker>(
        &mut self,
        lane: &ServiceLane<W>,
        kind: ServiceKind,
        operation: impl Into<String>,
        target: impl Into<String>,
        cancellable: bool,
        request: W::Request,
    ) -> Result<ServiceRequestId, HostServiceSubmitError> {
        let (id, cancellation) = self
            .queue_service(kind, operation, target, cancellable)
            .map_err(HostServiceSubmitError::Lifecycle)?;
        if let Err(error) = lane.try_submit(id, request, cancellation) {
            self.services
                .finish(
                    id,
                    crate::workspace::ServiceOutcome::Failed(error.to_string()),
                )
                .expect("a newly queued service request can be failed");
            return Err(HostServiceSubmitError::Lane(error));
        }
        Ok(id)
    }

    /// Applies worker progress on the host thread and returns the owned domain
    /// event, if one accompanied the terminal update.
    pub fn apply_service_update<E>(
        &mut self,
        update: ServiceUpdate<E>,
    ) -> Result<Option<E>, ServiceStateError> {
        match update {
            ServiceUpdate::Started(id) => {
                self.services.start(id)?;
                Ok(None)
            }
            ServiceUpdate::Finished { id, event, outcome } => {
                self.services.finish(id, outcome)?;
                Ok(event)
            }
        }
    }

    pub fn prepare_frame(&mut self, geometry: FrameGeometry) -> HostFrame {
        self.prepare_frame_with_hints(geometry, None)
    }

    pub fn prepare_frame_with_hints(
        &mut self,
        geometry: FrameGeometry,
        key_hints: Option<&KeyHintState>,
    ) -> HostFrame {
        let id = FrameId(self.next_frame);
        self.next_frame = self
            .next_frame
            .checked_add(1)
            .expect("workspace frame identity exhausted");
        let view = self.app.prepare_view(geometry);
        let editor = self.app.snapshot(&view);
        let mut overlays = self.app.overlay_snapshots();
        if let Some(key_hints) = key_hints
            && key_hints.is_visible()
            && let Some(mode) = self.app.key_hint_mode()
        {
            let capabilities = self.app.command_capabilities();
            let mut hint_rows =
                key_hints.rows_in(self.app.keymap(), mode, self.app.key_binding_scope());
            for row in &mut hint_rows {
                row.apply_capabilities(&capabilities);
            }
            let widest_key = hint_rows
                .iter()
                .map(|hint| UnicodeWidthStr::width(key_hint_keys(hint).as_str()))
                .max()
                .unwrap_or_default();
            let widest_description = hint_rows
                .iter()
                .map(|hint| UnicodeWidthStr::width(key_hint_description(hint).as_str()))
                .max()
                .unwrap_or_default();
            let layout = key_hint_layout(
                geometry.editor.width,
                geometry.editor.height,
                hint_rows.len(),
                widest_key,
                widest_description,
                key_hints.scroll_offset(),
            );
            key_hints.note_scroll_limit(hint_rows.len().saturating_sub(layout.capacity));
            let total_rows = hint_rows.len();
            const ROW_LIMIT: usize = 512;
            let rows = hint_rows
                .into_iter()
                .take(ROW_LIMIT)
                .map(|hint| {
                    let available =
                        hint.availability.is_implemented() && hint.unavailable_reason.is_none();
                    OverlayRow {
                        identity: OverlayIdentity::Text(hint.sequence.to_string()),
                        label: key_hint_keys(&hint),
                        detail: key_hint_description(&hint),
                        trailing_detail: String::new(),
                        available,
                        dimmed: false,
                        muted: Vec::new(),
                        emphasis: Vec::new(),
                        detail_emphasis: Vec::new(),
                    }
                })
                .collect::<Vec<_>>();
            let arrows_are_free = key_hints.scrolls_with_arrow_in(
                crate::input::KeyCode::Up,
                mode,
                self.app.key_binding_scope(),
                self.app.keymap(),
            ) && key_hints.scrolls_with_arrow_in(
                crate::input::KeyCode::Down,
                mode,
                self.app.key_binding_scope(),
                self.app.keymap(),
            );
            let scroll_keys = if arrows_are_free {
                "Ctrl-n/p ↑/↓"
            } else {
                "Ctrl-n/p Alt-j/k"
            };
            let sequence = if key_hints.is_pending() {
                format!("{} …", key_hints.display_pending())
            } else {
                String::new()
            };
            overlays.push(OverlaySnapshot {
                kind: OverlayKind::KeyHints,
                purpose: crate::snapshot::OverlayPurpose::Context,
                input: crate::snapshot::OverlayInput::None,
                layout: crate::snapshot::OverlayLayout::Bottom,
                actions: vec![
                    crate::snapshot::OverlayAction::new(scroll_keys, "scroll"),
                    crate::snapshot::OverlayAction::new("Esc", "dismiss"),
                ],
                title: format!("Keys: {sequence}"),
                query: String::new(),
                column_header: None,
                rows,
                selected: None,
                scroll_anchor: Some(layout.offset),
                row_offset: 0,
                message: key_hints.message().map(str::to_owned),
                omitted_rows: total_rows.saturating_sub(ROW_LIMIT),
                total_rows,
                query_cursor: None,
                show_preview: false,
                preview_title: None,
                preview: None,
            });
        }
        let pointer_compatible_since = self
            .prepared
            .as_ref()
            .filter(|prepared| prepared.view == view)
            .map_or(id, |prepared| prepared.pointer_compatible_since);
        self.prepared = Some(PreparedFrame {
            id,
            pointer_compatible_since,
            view,
        });
        let active_buffer = self.app.active().buffer;
        HostFrame {
            id,
            active_buffer: BufferId::from_index(active_buffer),
            active_revision: BufferRevision::from_raw(self.app.buffers[active_buffer].revision()),
            editor,
            overlays,
        }
    }

    pub fn current_frame_id(&self) -> Option<FrameId> {
        self.prepared.as_ref().map(|frame| frame.id)
    }

    /// Runs a semantic command only against the exact editor state the
    /// interactive client observed. This keeps layout- and selection-sensitive
    /// commands from acting on a newer frame or buffer revision.
    pub fn execute_expected_command(
        &mut self,
        frame: FrameId,
        buffer: BufferId,
        revision: BufferRevision,
        invocation: CommandInvocation,
    ) -> Result<CommandOutcome, BufferRequestError> {
        if self.app.macro_replay_pending() {
            return Err(BufferRequestError::Refused(
                "macro replay owns editor input; cancel it before invoking a command".to_owned(),
            ));
        }
        let Some(prepared) = self.prepared.as_ref() else {
            return Err(BufferRequestError::Refused(
                "no prepared editor frame is available".to_owned(),
            ));
        };
        if prepared.id != frame {
            return Err(BufferRequestError::Refused(format!(
                "stale editor frame: expected {}, current {}",
                frame.get(),
                prepared.id.get()
            )));
        }
        let active = self.app.active().buffer;
        let actual_buffer = BufferId::from_index(active);
        if actual_buffer != buffer {
            return Err(BufferRequestError::Refused(format!(
                "inactive command buffer: expected {buffer}, current {actual_buffer}"
            )));
        }
        let actual_revision = BufferRevision::from_raw(self.app.buffers[active].revision());
        if actual_revision != revision {
            return Err(BufferRequestError::Stale {
                expected: revision,
                actual: actual_revision,
            });
        }
        self.app
            .execute(invocation)
            .map_err(|error| BufferRequestError::Refused(error.to_string()))
    }

    pub fn execute(&mut self, command: HostCommand) -> Result<HostInputOutcome> {
        match command {
            HostCommand::Input(input) => {
                self.app.handle_input(input)?;
                Ok(HostInputOutcome::Applied)
            }
            HostCommand::Pointer {
                event,
                frame,
                repetitions,
            } => {
                let Some(prepared) = self.prepared.as_ref() else {
                    return Ok(HostInputOutcome::IgnoredStaleFrame);
                };
                if frame < prepared.pointer_compatible_since || frame > prepared.id {
                    return Ok(HostInputOutcome::IgnoredStaleFrame);
                }
                match self
                    .app
                    .handle_pointer_repeated(event, &prepared.view, repetitions)?
                {
                    PointerOutcome::Changed => Ok(HostInputOutcome::Applied),
                    PointerOutcome::Unchanged => Ok(HostInputOutcome::AppliedWithoutVisualChange),
                }
            }
        }
    }

    pub fn apply_event(&mut self, event: HostEvent) {
        match event {
            HostEvent::Syntax(event) => {
                self.app.apply_syntax_event(event);
            }
            HostEvent::Lsp(event) => self.app.apply_lsp_event(event),
            HostEvent::FilePicker(event) => self.app.apply_file_picker_event(event),
            HostEvent::WorkspaceSearch(event) => self.app.apply_workspace_search_event(event),
            HostEvent::FileObservation(event) => self.app.apply_file_observation(event),
            HostEvent::GitInvalidation(event) => self.apply_git_invalidation(event),
            HostEvent::Git(event) => self.apply_git_service_event(event),
            HostEvent::Terminal(output) => self.app.apply_terminal_output(output),
            #[cfg(unix)]
            HostEvent::Workspace(event) => self.app.apply_workspace_event(event),
        }
    }

    /// Applies terminal output with attachment awareness. Persistent hosts
    /// call this directly because a retained pane is not itself an observer.
    pub fn apply_terminal_output(
        &mut self,
        output: crate::terminal::TerminalOutput,
        observed: bool,
    ) {
        self.app.apply_terminal_output_observed(output, observed);
    }

    pub fn mark_visible_terminals_viewed(&mut self) {
        self.app.mark_visible_terminals_viewed();
    }

    /// Hands the terminal output stream to whichever loop will drive it.
    pub fn take_terminal_events(&mut self) -> Option<crate::terminal::TerminalEvents> {
        self.app.terminals.take_events()
    }

    pub fn attach_lsp(&mut self, handle: LspHandle) {
        self.app.attach_lsp(handle);
    }

    pub fn attach_syntax_worker(&mut self, handle: SyntaxHandle) {
        self.app.attach_syntax_worker(handle);
    }

    pub fn attach_file_scanner(&mut self, scanner: FileScanner) {
        self.app.attach_file_scanner(scanner);
    }

    fn completed_git_snapshot(event: &GitServiceEvent) -> Option<CompletedGitSnapshot> {
        let GitServiceEvent::Completed {
            id: _,
            operation,
            result,
            state,
            coalesced,
        } = event
        else {
            return None;
        };
        let response = result.as_ref().as_ref().ok()?;
        let (snapshot, mutation) = match response {
            GitResponse::Snapshot(snapshot) => (snapshot.as_ref(), false),
            GitResponse::Mutation { snapshot, .. } => (snapshot.as_ref().as_ref().ok()?, true),
            _ => return None,
        };
        Some(CompletedGitSnapshot {
            repository: snapshot.repository.workdir().to_path_buf(),
            generation: snapshot.generation,
            started_at: snapshot.started_at,
            spec: snapshot.requested.clone(),
            state: *state,
            coalesced: *coalesced,
            mutation: mutation || matches!(operation, GitOperation::Mutate { .. }),
        })
    }

    fn apply_git_service_event(&mut self, event: GitServiceEvent) {
        let generation_before = self.app.git_snapshot_generation();
        let completed_id = match &event {
            GitServiceEvent::Completed { id, .. } => Some(*id),
            GitServiceEvent::Progress(_) => None,
        };
        let completed = Self::completed_git_snapshot(&event);
        self.app.apply_git_service_event(event);
        let pending = completed_id
            .and_then(|id| self.pending_git_refresh.take_if(|pending| pending.id == id));
        let Some(completed) = completed else {
            if pending.is_some() {
                self.next_git_retry = Some(Instant::now() + AUTOMATIC_GIT_RETRY_DELAY);
                if self.git_dirty {
                    self.app.mark_git_snapshot_stale();
                }
            }
            return;
        };
        let accepted = completed.generation >= generation_before
            && (completed.state == GitServiceState::Completed
                || (completed.mutation && completed.state == GitServiceState::Failed));
        if let Some(pending) = pending {
            if accepted && completed.state == GitServiceState::Completed && !completed.coalesced {
                debug_assert_eq!(pending.spec, completed.spec);
                self.record_git_reconciliation(completed, pending.fallback);
            } else {
                self.next_git_retry = Some(Instant::now() + AUTOMATIC_GIT_RETRY_DELAY);
                if self.git_dirty {
                    self.app.mark_git_snapshot_stale();
                }
            }
        } else if accepted && !completed.coalesced {
            // Initial, explicit, and mutation-owned snapshots are freshness
            // barriers too. Resetting coverage matters because each one is a
            // deliberately narrow read of the consumers visible at that time.
            self.record_git_reconciliation(completed, true);
        }
    }

    fn record_git_reconciliation(&mut self, completed: CompletedGitSnapshot, reset: bool) {
        if self
            .latest_git_observation
            .is_some_and(|observed| observed >= completed.started_at)
        {
            self.app.mark_git_snapshot_stale();
            return;
        }
        self.last_git_refresh = Instant::now();
        self.next_git_retry = None;
        self.last_git_reconciliation = Some(AppliedGitReconciliation {
            repository: completed.repository,
            started_at: completed.started_at,
            spec: completed.spec.clone(),
        });
        if self.app.automatic_git_refresh_interval_seconds() == 0 {
            self.git_dirty = false;
            self.git_reconciled = None;
            return;
        }
        self.git_dirty = true;
        if reset {
            self.git_reconciled = Some(completed.spec);
        } else {
            self.git_reconciled
                .get_or_insert_with(Default::default)
                .merge(&completed.spec);
        }
    }

    fn apply_git_invalidation(&mut self, event: GitInvalidation) {
        if self
            .app
            .git_monitor_repository()
            .is_some_and(|repository| repository.workdir() == event.repository)
        {
            self.latest_git_observation = Some(
                self.latest_git_observation
                    .map_or(event.observed_at, |latest| latest.max(event.observed_at)),
            );
            self.next_git_retry = None;
            self.git_dirty = true;
            let covered = self
                .last_git_reconciliation
                .as_ref()
                .filter(|reconciliation| reconciliation.repository == event.repository)
                .filter(|reconciliation| event.observed_at < reconciliation.started_at);
            if let Some(reconciliation) = covered {
                self.git_reconciled = Some(reconciliation.spec.clone());
            } else {
                self.git_reconciled = None;
                self.app.mark_git_snapshot_stale();
            }
        }
    }

    /// Reconciles observed invalidation after the configured automatic-refresh
    /// cooldown and uses the same interval as a maximum-staleness fallback.
    /// Dirty state is retained while no Git consumer is visible, while the
    /// cooldown is active, or while interaction defers a projection rewrite.
    pub fn refresh_git_if_due(&mut self, now: Instant) -> bool {
        // A confirmed filesystem plan is a stronger boundary than automatic
        // monitoring and remains required when that monitoring is disabled.
        let _ = self.app.retry_pending_git_reconciliation(now);
        let seconds = self.app.automatic_git_refresh_interval_seconds();
        if seconds == 0 {
            self.git_dirty = false;
            self.git_reconciled = None;
            self.pending_git_refresh = None;
            self.next_git_retry = None;
            return false;
        }
        if self.pending_git_refresh.is_some()
            || self.next_git_retry.is_some_and(|retry| now < retry)
        {
            return false;
        }
        if !self.app.has_visible_git_state() {
            return false;
        }
        let Some(spec) = self.app.visible_git_refresh_spec() else {
            return false;
        };
        let interval = Duration::from_secs(seconds as u64);
        let last_automatic_boundary = self
            .last_automatic_git_refresh_request
            .map_or(self.last_git_refresh, |request| {
                request.max(self.last_git_refresh)
            });
        let cooldown_elapsed = now.saturating_duration_since(last_automatic_boundary) >= interval;
        let fallback_due = now.saturating_duration_since(self.last_git_refresh) >= interval;
        let dirty_due = self.git_dirty
            && self
                .git_reconciled
                .as_ref()
                .is_none_or(|reconciled| !reconciled.covers(&spec));
        if (dirty_due || fallback_due) && !cooldown_elapsed {
            return false;
        }
        if !dirty_due && !fallback_due {
            return false;
        }
        if let Some(id) = self.app.request_automatic_git_refresh(spec.clone()) {
            self.last_automatic_git_refresh_request = Some(now);
            self.pending_git_refresh = Some(PendingAutomaticGitRefresh {
                id,
                spec,
                fallback: fallback_due && !dirty_due,
            });
            true
        } else {
            false
        }
    }

    /// Refreshes the session manager only when one of its rounded activity
    /// values crossed a visible boundary.
    pub fn refresh_session_activity(&mut self) -> bool {
        #[cfg(unix)]
        {
            self.app.refresh_workspace_activity()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

fn bounded_utf8(value: &str, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value.to_owned(), false);
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true)
}

fn update_wait_status(request: &mut WaitRequest) {
    let remaining = request
        .buffers
        .iter()
        .filter(|buffer| !request.completed.contains(buffer))
        .copied()
        .collect::<Vec<_>>();
    request.status = if remaining.is_empty() {
        WaitStatus::Completed
    } else {
        WaitStatus::Pending {
            buffers: request.buffers.clone(),
            remaining,
        }
    };
}

// Temporary migration convenience: application integrations still call
// narrow `App` methods while coherent ownership moves behind the host. The
// host remains the owner and transport code never receives this dereference.
impl Deref for WorkspaceHost {
    type Target = App;

    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

impl DerefMut for WorkspaceHost {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.app
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::HostPorts,
        buffer::BufferKind,
        clipboard::SystemClipboard,
        command::{CommandExecutionContext, EditorCommand, Mode},
        config::Config,
        git::{
            BaseContent, Divergence, GitCliProvider, GitError, GitMutation, GitOperation,
            GitRequestId, GitResponse, GitService, GitServiceEvent, Head, Repository,
            RepositorySnapshot, RepositoryStatus,
        },
        git_monitor::GitInvalidation,
        input::{KeyStroke, Modifiers, PointerButton, PointerEventKind},
        launch::LaunchTarget,
        layout::Rect,
        text::Transaction,
        workspace::{ServiceLane, ServiceOutcome, ServiceWorker},
    };
    use anyhow::{Result, bail};
    use std::{
        fs,
        process::Command,
        sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        thread,
        time::SystemTime,
    };

    fn unique_test_id() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    #[derive(Default)]
    struct InertClipboard;

    impl SystemClipboard for InertClipboard {
        fn read(&mut self) -> Result<String> {
            bail!("clipboard is unavailable")
        }

        fn write(&mut self, _text: &str) -> Result<()> {
            bail!("clipboard is unavailable")
        }
    }

    fn host() -> WorkspaceHost {
        let app = App::new_in_isolated_project(
            std::env::temp_dir(),
            HostPorts::isolated(Box::new(InertClipboard)),
        )
        .unwrap();
        WorkspaceHost::new(app)
    }

    fn geometry() -> FrameGeometry {
        FrameGeometry {
            screen: Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            editor: Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 22,
            },
            status: Rect {
                x: 0,
                y: 22,
                width: 80,
                height: 1,
            },
            message: Rect {
                x: 0,
                y: 23,
                width: 80,
                height: 1,
            },
        }
    }

    fn complete_git_refresh(
        host: &mut WorkspaceHost,
        events: &mut tokio::sync::mpsc::Receiver<GitServiceEvent>,
    ) {
        loop {
            let event = events.blocking_recv().expect("Git service stayed live");
            let complete = matches!(
                &event,
                GitServiceEvent::Completed {
                    operation: GitOperation::Refresh { .. },
                    ..
                }
            );
            host.apply_event(HostEvent::Git(event));
            if complete {
                break;
            }
        }
    }

    #[test]
    fn a_snapshot_covers_only_observations_before_its_first_read() {
        let mut host = host();
        let root = host.project_root.clone();
        let path = root.join("covered-by-mutation.rs");
        host.buffers[0].kind = BufferKind::File;
        host.buffers[0].path = Some(path.clone());
        let repository = Repository::new(&root);
        let spec = RefreshSpec {
            staged_paths: vec![path.clone()],
            ..RefreshSpec::default()
        };
        let started_at = Instant::now();
        let mutation = GitMutation::Stage(vec![path.clone()]);
        host.apply_event(HostEvent::Git(GitServiceEvent::Completed {
            id: GitRequestId::from_raw(91),
            operation: GitOperation::Mutate {
                repository: repository.clone(),
                mutation: mutation.clone(),
                refresh: spec.clone(),
            },
            result: Box::new(Ok(GitResponse::Mutation {
                mutation,
                applied_paths: vec![path.clone()],
                summary: None,
                failure: None,
                snapshot: Box::new(Ok(RepositorySnapshot {
                    repository,
                    generation: RepositoryGeneration::from_raw(1),
                    started_at,
                    requested: spec.clone(),
                    status: RepositoryStatus {
                        head: Head::Branch("main".to_owned()),
                        upstream: None,
                        divergence: Divergence::default(),
                        files: Vec::new(),
                    },
                    stats: Default::default(),
                    head_oid: Some("a".repeat(40)),
                    staged: vec![(path, BaseContent::Text(String::new()))],
                    branches: None,
                    staged_diff: None,
                    file_diffs: Vec::new(),
                    worktrees: None,
                    log: None,
                    requested_log_anchors: Vec::new(),
                    reachable_log_anchors: Vec::new(),
                    stashes: None,
                })),
            })),
            state: GitServiceState::Completed,
            coalesced: false,
        }));

        host.apply_event(HostEvent::GitInvalidation(GitInvalidation {
            repository: root,
            observed_at: started_at.checked_sub(Duration::from_millis(1)).unwrap(),
            overflowed: false,
        }));

        assert!(host.git_dirty);
        assert!(host.git_reconciled.as_ref().unwrap().covers(&spec));

        host.apply_event(HostEvent::GitInvalidation(GitInvalidation {
            repository: host.project_root.clone(),
            observed_at: started_at,
            overflowed: false,
        }));

        assert!(host.git_dirty);
        assert!(host.git_reconciled.is_none());
    }

    #[test]
    fn a_failed_automatic_refresh_does_not_claim_reconciliation() {
        let mut host = host();
        let repository = Repository::new(host.project_root.clone());
        let spec = RefreshSpec::default();
        let last_success = host.last_git_refresh;
        host.git_dirty = true;
        host.pending_git_refresh = Some(PendingAutomaticGitRefresh {
            id: GitRequestId::from_raw(92),
            spec,
            fallback: false,
        });

        host.apply_event(HostEvent::Git(GitServiceEvent::Completed {
            id: GitRequestId::from_raw(92),
            operation: GitOperation::Refresh {
                repository,
                spec: RefreshSpec::default(),
            },
            result: Box::new(Err(GitError::Failed {
                command: "git status".to_owned(),
                code: Some(1),
                signal: None,
                stderr: "temporary failure".to_owned(),
            })),
            state: GitServiceState::Failed,
            coalesced: false,
        }));

        assert!(host.pending_git_refresh.is_none());
        assert!(host.git_reconciled.is_none());
        assert_eq!(host.last_git_refresh, last_success);
        assert!(host.next_git_retry.is_some());
    }

    #[test]
    fn git_invalidation_is_retained_until_visible_and_rate_limited_between_fallbacks() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-git-invalidation-{}-{}-{}",
            std::process::id(),
            unique_test_id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let run = |arguments: &[&str]| {
            let output = Command::new("git")
                .args(arguments)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {arguments:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.name", "Runyte Test"]);
        run(&["config", "user.email", "runyte@example.invalid"]);
        let tracked = root.join("tracked.rs");
        let second = root.join("second.rs");
        fs::write(&tracked, "fn main() {}\n").unwrap();
        fs::write(&second, "fn second() {}\n").unwrap();
        run(&["add", "tracked.rs", "second.rs"]);
        run(&["commit", "-qm", "base"]);
        let root = root.canonicalize().unwrap();
        let tracked = tracked.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();

        let mut app = App::new_in_project_with_targets(
            Config::default(),
            vec![
                LaunchTarget::new(root.clone()),
                LaunchTarget::new(tracked.clone()),
                LaunchTarget::new(second.clone()),
            ],
            &root,
        )
        .unwrap();
        let directory_buffer = app
            .buffers
            .iter()
            .position(|buffer| buffer.path.as_deref() == Some(root.as_path()))
            .unwrap();
        let tracked_buffer = app
            .buffers
            .iter()
            .position(|buffer| buffer.path.as_deref() == Some(tracked.as_path()))
            .unwrap();
        let second_buffer = app
            .buffers
            .iter()
            .position(|buffer| buffer.path.as_deref() == Some(second.as_path()))
            .unwrap();
        app.panes.get_mut(&app.active_pane).unwrap().buffer = tracked_buffer;
        let (service, mut events) = GitService::spawn(GitCliProvider::new("git"));
        app.attach_git_service(service);
        let mut host = WorkspaceHost::new(app);
        complete_git_refresh(&mut host, &mut events);
        thread::sleep(Duration::from_millis(275));

        let before_fallback = host.last_git_refresh + Duration::from_secs(59);
        assert!(
            !host.refresh_git_if_due(before_fallback),
            "an unchanged visible file triggered routine polling"
        );

        let active_pane = host.active_pane;
        host.panes.get_mut(&active_pane).unwrap().buffer = directory_buffer;
        host.apply_event(HostEvent::GitInvalidation(GitInvalidation {
            repository: root.clone(),
            observed_at: Instant::now(),
            overflowed: false,
        }));
        assert!(host.git_dirty);
        assert!(host.git_summary().unwrap().contains("stale"));
        assert!(
            !host.refresh_git_if_due(before_fallback),
            "a hidden invalidation ran Git with no visible consumer"
        );
        assert!(host.git_dirty, "the hidden invalidation was forgotten");

        host.panes.get_mut(&active_pane).unwrap().buffer = tracked_buffer;
        thread::sleep(Duration::from_millis(275));
        assert!(
            !host.refresh_git_if_due(before_fallback),
            "revealing the dirty tracked gutter bypassed the refresh cooldown"
        );
        assert!(
            host.git_dirty,
            "the rate-limited invalidation was forgotten"
        );
        let interval_due = host.last_git_refresh + Duration::from_secs(60);
        assert!(
            host.refresh_git_if_due(interval_due),
            "the retained invalidation was not refreshed after the cooldown"
        );
        complete_git_refresh(&mut host, &mut events);

        host.panes.get_mut(&active_pane).unwrap().buffer = second_buffer;
        assert!(
            !host.refresh_git_if_due(interval_due),
            "revealing a different dirty gutter bypassed the refresh cooldown"
        );
        let coverage_due = interval_due + Duration::from_secs(60);
        assert!(host.refresh_git_if_due(coverage_due));
        complete_git_refresh(&mut host, &mut events);
        let reconciled = host.git_reconciled.as_ref().unwrap();
        assert!(reconciled.staged_paths.contains(&tracked));
        assert!(reconciled.staged_paths.contains(&second));
        assert!(host.git_tracks(tracked_buffer));
        assert!(host.git_tracks(second_buffer));
        host.panes.get_mut(&active_pane).unwrap().buffer = tracked_buffer;
        assert!(
            !host.refresh_git_if_due(coverage_due),
            "returning to a reconciled gutter scheduled a redundant refresh"
        );

        host.apply_event(HostEvent::GitInvalidation(GitInvalidation {
            repository: root.clone(),
            observed_at: Instant::now(),
            overflowed: false,
        }));
        let automatic_boundary = host
            .last_automatic_git_refresh_request
            .unwrap()
            .max(host.last_git_refresh);
        assert!(
            !host.refresh_git_if_due(automatic_boundary + Duration::from_secs(59)),
            "a sustained invalidation bypassed the refresh cooldown"
        );
        let fallback = automatic_boundary + Duration::from_secs(60);
        assert!(host.refresh_git_if_due(fallback));
        complete_git_refresh(&mut host, &mut events);

        host.config.git.refresh_interval_seconds = 0;
        host.apply_event(HostEvent::GitInvalidation(GitInvalidation {
            repository: root.clone(),
            observed_at: Instant::now(),
            overflowed: true,
        }));
        assert!(!host.refresh_git_if_due(fallback + Duration::from_secs(600)));
        assert!(!host.git_dirty);

        drop(host);
        drop(events);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pointer_frames_remain_compatible_until_the_prepared_view_changes() {
        let mut host = host();
        host.app_mut().buffers[0].apply(&Transaction::insert(0, "alpha\nbeta"));
        let first = host.prepare_frame(geometry());
        let second = host.prepare_frame(geometry());
        assert!(second.id > first.id);

        let event = PointerEvent {
            kind: PointerEventKind::Down(PointerButton::Left),
            column: 10,
            row: 1,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(
            host.execute(HostCommand::Pointer {
                event,
                frame: first.id,
                repetitions: 1,
            })
            .unwrap(),
            HostInputOutcome::Applied
        );
        let mut changed_geometry = geometry();
        changed_geometry.editor.width -= 1;
        host.prepare_frame(changed_geometry);
        let after_compatible_input = host.app().active().selection.clone();
        assert_eq!(
            host.execute(HostCommand::Pointer {
                event,
                frame: first.id,
                repetitions: 1,
            })
            .unwrap(),
            HostInputOutcome::IgnoredStaleFrame
        );
        assert_eq!(&host.app().active().selection, &after_compatible_input);
    }

    #[test]
    fn direct_host_input_uses_the_same_semantic_editor() {
        let mut host = host();
        host.execute(HostCommand::Input(InputEvent::Key(KeyStroke::char('i'))))
            .unwrap();
        host.execute(HostCommand::Input(InputEvent::Text("hello".to_owned())))
            .unwrap();
        assert_eq!(host.app().active_buffer().to_string(), "hello");
    }

    #[test]
    fn host_macro_replay_returns_between_input_and_cooperative_playback() {
        let mut host = host();
        for character in [' ', 'm', 'm', 'i', 'x'] {
            host.execute(HostCommand::Input(InputEvent::Key(KeyStroke::char(
                character,
            ))))
            .unwrap();
        }
        host.execute(HostCommand::Input(InputEvent::Key(KeyStroke::new(
            crate::input::KeyCode::Escape,
            Modifiers::NONE,
        ))))
        .unwrap();
        for character in [' ', 'm', 'm', ' ', 'm', 'r'] {
            host.execute(HostCommand::Input(InputEvent::Key(KeyStroke::char(
                character,
            ))))
            .unwrap();
        }

        assert!(host.macro_replay_pending());
        assert_eq!(host.app().active_buffer().to_string(), "x");

        host.advance_macro_replay().unwrap();

        assert!(!host.macro_replay_pending());
        assert_eq!(host.app().active_buffer().to_string(), "xx");
    }

    #[test]
    fn attached_semantic_commands_cannot_interleave_with_macro_replay() {
        let mut host = host();
        host.app_mut().buffers[0].apply(&Transaction::insert(0, "abcdef"));
        for character in [' ', 'm', 'm', 'l', ' ', 'm', 'm', ' ', 'm', 'r'] {
            host.execute(HostCommand::Input(InputEvent::Key(KeyStroke::char(
                character,
            ))))
            .unwrap();
        }
        assert!(host.macro_replay_pending());
        let head = host.app().active().head();
        let frame = host.prepare_frame(geometry());
        let invocation =
            CommandInvocation::editor(EditorCommand::MoveRight, CommandExecutionContext::default())
                .unwrap();

        let result = host.execute_expected_command(
            frame.id,
            frame.active_buffer,
            frame.active_revision,
            invocation,
        );

        assert!(matches!(
            result,
            Err(BufferRequestError::Refused(message))
                if message == "macro replay owns editor input; cancel it before invoking a command"
        ));
        assert!(host.macro_replay_pending());
        assert_eq!(host.app().active().head(), head);
    }

    #[test]
    fn session_preview_reads_the_retained_viewport_without_preparing_a_frame() {
        let mut host = host();
        let text = (1..=20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        host.app_mut().buffers[0].apply(&Transaction::insert(0, text));
        host.app_mut().panes.get_mut(&0).unwrap().scroll_row = 9;

        let preview = host.session_preview();

        assert_eq!(preview.layout_panes, 1);
        assert_eq!(preview.omitted_panes, 0);
        assert!(preview.other_resources.is_empty());
        let pane = &preview.panes[0];
        assert!(pane.active);
        assert_eq!(pane.start_line, Some(10));
        assert_eq!(pane.lines.first().map(String::as_str), Some("line 10"));
        assert_eq!(pane.lines.last().map(String::as_str), Some("line 17"));
        assert!(matches!(
            pane.kind,
            SessionPreviewPaneKind::Buffer {
                dirty: true,
                read_only: false
            }
        ));
        // Reading a preview must not replace the geometry/pointer witness the
        // interactive frontend last received.
        assert!(host.current_frame_id().is_none());
    }

    #[test]
    fn session_preview_lines_are_bounded_and_strip_terminal_controls() {
        let line = format!("abc\u{0}{}", "x".repeat(SESSION_PREVIEW_COLUMNS + 20));
        let preview = session_preview_line(&line);

        assert_eq!(preview.chars().count(), SESSION_PREVIEW_COLUMNS);
        assert!(preview.starts_with("abc\u{fffd}"));
        assert!(!preview.contains('\u{0}'));
    }

    #[test]
    fn frames_own_bounded_overlay_state() {
        use crate::picker::{ListPicker, PickerItem};
        use crate::snapshot::OverlayKind;

        let mut host = host();
        let mut list = ListPicker::new(
            "Symbols",
            (0..600)
                .map(|index| PickerItem::new(format!("symbol-{index}"), "function", index))
                .collect(),
        );
        list.selected = 599;
        host.app_mut().list = Some(list);
        let frame = host.prepare_frame(geometry());
        let overlay = frame
            .overlays
            .iter()
            .find(|overlay| overlay.kind == OverlayKind::ResultList)
            .unwrap();
        assert_eq!(overlay.rows.len(), 512);
        assert_eq!(overlay.omitted_rows, 88);
        assert_eq!(overlay.row_offset, 88);
        assert_eq!(overlay.selected, Some(511));

        host.app_mut().list = None;
        assert_eq!(frame.overlays[0].rows[0].label, "symbol-88");
    }

    #[test]
    fn host_frame_includes_owned_key_hint_rows() {
        let mut host = host();
        let mut hints = KeyHintState::default();
        hints.push(KeyStroke::char('g'));
        let frame = host.prepare_frame_with_hints(geometry(), Some(&hints));
        let overlay = frame
            .overlays
            .iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::KeyHints)
            .expect("the pending prefix has an immutable overlay");
        assert_eq!(overlay.title, "Keys: g …");
        assert_eq!(overlay.query, "");
        assert_eq!(overlay.row_offset, 0);
        assert_eq!(overlay.scroll_anchor, Some(0));
        assert!(!overlay.rows.is_empty());
    }

    #[test]
    fn host_key_hint_rows_include_active_buffer_capability_availability() {
        let mut host = host();
        let mut hints = KeyHintState::default();
        hints.push(KeyStroke::char(' '));

        let frame = host.prepare_frame_with_hints(geometry(), Some(&hints));
        let overlay = frame
            .overlays
            .iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::KeyHints)
            .expect("Space opens the key-hint overlay");
        for label in ["Language (LSP)", "Syntax (Tree-sitter)"] {
            let row = overlay
                .rows
                .iter()
                .find(|row| row.detail.starts_with(label))
                .unwrap_or_else(|| panic!("the Space menu lists {label}"));
            assert!(!row.available, "{label}");
            assert!(
                row.detail.contains("no LSP") || row.detail.contains("no syntax"),
                "{}",
                row.detail
            );
        }
    }

    #[derive(Clone)]
    struct HeldWorker {
        state: Arc<(Mutex<(bool, bool)>, Condvar)>,
    }

    impl ServiceWorker for HeldWorker {
        type Request = String;
        type Event = String;

        fn execute(
            &mut self,
            request: Self::Request,
            cancellation: &CancellationToken,
        ) -> (Option<Self::Event>, ServiceOutcome) {
            let (lock, changed) = &*self.state;
            let mut state = lock.lock().unwrap();
            state.0 = true;
            changed.notify_all();
            while !state.1 && !cancellation.is_cancelled() {
                state = changed.wait(state).unwrap();
            }
            if cancellation.is_cancelled() {
                (None, ServiceOutcome::Cancelled)
            } else {
                (
                    Some(format!("finished {request}")),
                    ServiceOutcome::Completed,
                )
            }
        }
    }

    #[test]
    fn held_typed_service_leaves_host_input_and_frames_available() {
        let state = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let lane = ServiceLane::spawn(
            HeldWorker {
                state: Arc::clone(&state),
            },
            1,
        );
        let mut host = host();
        let id = host
            .submit_service(
                &lane,
                ServiceKind::Git,
                "status",
                "/project",
                true,
                "status".to_owned(),
            )
            .unwrap();

        let (lock, changed) = &*state;
        let mut held = lock.lock().unwrap();
        while !held.0 {
            held = changed.wait(held).unwrap();
        }
        drop(held);

        host.submit_service(
            &lane,
            ServiceKind::Git,
            "branches",
            "/project",
            true,
            "branches".to_owned(),
        )
        .unwrap();
        assert!(matches!(
            host.submit_service(
                &lane,
                ServiceKind::Git,
                "diff",
                "/project/file",
                true,
                "diff".to_owned(),
            ),
            Err(HostServiceSubmitError::Lane(ServiceSubmitError::Full))
        ));
        assert_eq!(
            host.services().active().count(),
            2,
            "the refused request must already be terminal"
        );

        while let Ok(update) = lane.try_recv() {
            assert!(host.apply_service_update(update).unwrap().is_none());
        }
        host.execute(HostCommand::Input(InputEvent::Key(KeyStroke::char('i'))))
            .unwrap();
        host.execute(HostCommand::Input(InputEvent::Text(
            "responsive".to_owned(),
        )))
        .unwrap();
        let frame = host.prepare_frame(geometry());
        assert_eq!(host.app().active_buffer().to_string(), "responsive");
        assert_eq!(frame.id.get(), 1);
        assert_eq!(
            host.services().progress(id).unwrap().phase,
            crate::workspace::ServicePhase::Running
        );

        let mut held = lock.lock().unwrap();
        held.1 = true;
        changed.notify_all();
        drop(held);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let event = loop {
            match lane.try_recv() {
                Ok(update) => {
                    if let Some(event) = host.apply_service_update(update).unwrap() {
                        break event;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty)
                    if std::time::Instant::now() < deadline =>
                {
                    std::thread::yield_now();
                }
                other => panic!("service did not finish: {other:?}"),
            }
        };
        assert_eq!(event, "finished status");
        assert_eq!(
            host.services().progress(id).unwrap().phase,
            crate::workspace::ServicePhase::Completed
        );
    }

    #[test]
    fn expected_revision_edits_are_atomic_undoable_and_stale_safe() {
        let mut host = host();
        let id = host.buffer_metadata()[0].id;
        let before = host.read_buffer(id).unwrap();
        let revision = host
            .apply_expected_transaction(
                id,
                before.metadata.revision,
                Transaction::insert(0, "alpha"),
            )
            .unwrap();
        assert_ne!(revision, before.metadata.revision);
        let error = host
            .apply_expected_transaction(
                id,
                before.metadata.revision,
                Transaction::insert(0, "stale"),
            )
            .unwrap_err();
        assert!(matches!(error, BufferRequestError::Stale { actual, .. } if actual == revision));
        assert_eq!(host.read_buffer(id).unwrap().text, "alpha");

        host.execute(HostCommand::Input(InputEvent::Key(KeyStroke::char('u'))))
            .unwrap();
        assert_eq!(host.read_buffer(id).unwrap().text, "");
    }

    #[test]
    fn buffer_ids_are_not_reused_and_saving_does_not_change_text_revision() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-buffer-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "base").unwrap();
        let mut host = host();
        let first = host.open_buffer(path.clone(), true).unwrap();
        let revision = host.read_buffer(first).unwrap().metadata.revision;
        assert_eq!(host.save_buffer(first).unwrap(), revision);
        host.close_buffer(first, false).unwrap();
        let reopened = host.open_buffer(path, true).unwrap();
        assert!(reopened.get() > first.get());
        assert!(
            matches!(host.read_buffer(first), Err(BufferRequestError::Closed(id)) if id == first)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn one_host_request_deduplicates_resolved_file_aliases() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "runyte-host-alias-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("target.txt");
        let alias = root.join("alias.txt");
        std::fs::write(&target, "shared").unwrap();
        symlink("target.txt", &alias).unwrap();
        let mut host = host();

        let opened = host
            .open_buffers([alias.clone(), target.clone()], false)
            .unwrap();

        assert_eq!(opened.len(), 2);
        assert_eq!(opened[0], opened[1]);
        assert_eq!(
            live_paths(&host)
                .into_iter()
                .filter(|path| path == &alias || path == &target)
                .count(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failing_later_path_opens_no_buffer_and_moves_no_pane() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-atomic-open-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let valid = root.join("note.txt");
        let binary = root.join("image.bin");
        std::fs::write(&valid, "alpha").unwrap();
        std::fs::write(&binary, b"\x00\x01\x02binary\x00").unwrap();
        let mut host = host();
        let before = host.app.active().buffer;
        let live_before = live_paths(&host);

        let error = host
            .open_buffers([valid.clone(), binary.clone()], true)
            .unwrap_err();
        assert!(
            error.to_string().contains("binary"),
            "unexpected error: {error}"
        );
        assert_eq!(
            live_paths(&host),
            live_before,
            "the valid first path was left open by a request that failed"
        );
        assert_eq!(
            host.app.active().buffer,
            before,
            "a failed request moved the active pane"
        );

        // The request is still available afterwards, so refusing it did not
        // leave the host in a state that rejects the same paths again.
        let opened = host.open_buffers([valid.clone()], true).unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(
            host.read_buffer(opened[0]).unwrap().text,
            "alpha",
            "the valid path could not be opened after the failure"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failing_later_path_allocates_no_wait_token() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-atomic-wait-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let valid = root.join("note.txt");
        let binary = root.join("image.bin");
        std::fs::write(&valid, "alpha").unwrap();
        std::fs::write(&binary, b"\x00\x01\x02binary\x00").unwrap();
        let mut host = host();
        let already_open = host.open_buffer(valid.clone(), false).unwrap();
        let before = host.read_buffer(already_open).unwrap();
        std::fs::write(&valid, "new on disk").unwrap();
        let live_before = live_paths(&host);

        host.create_wait_request([valid, binary], true).unwrap_err();
        assert_eq!(
            live_paths(&host),
            live_before,
            "the valid first path was left open by a wait request that failed"
        );
        assert!(
            host.wait_requests.is_empty() && host.wait_order.is_empty(),
            "a failed wait request left a token behind"
        );
        assert_eq!(
            host.read_buffer(already_open).unwrap(),
            before,
            "the reused clean buffer was refreshed before the whole request was validated"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_later_wait_refreshes_a_clean_reused_file_from_disk() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-refresh-wait-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("MERGE_MSG");
        std::fs::write(&path, "Merge branch 'security'\n").unwrap();
        let mut host = host();

        let (first, first_buffers) = host.create_wait_request([path.clone()], true).unwrap();
        let buffer = first_buffers[0];
        assert_eq!(
            host.read_buffer(buffer).unwrap().text,
            "Merge branch 'security'\n"
        );
        host.complete_wait_buffer(first, buffer).unwrap();

        std::fs::write(&path, "Merge branch 'dev'\n").unwrap();
        let before_refresh = host.read_buffer(buffer).unwrap().metadata.revision;
        let (_, second_buffers) = host.create_wait_request([path.clone()], true).unwrap();

        assert_eq!(second_buffers, vec![buffer]);
        let refreshed = host.read_buffer(buffer).unwrap();
        assert_eq!(refreshed.text, "Merge branch 'dev'\n");
        assert_ne!(refreshed.metadata.revision, before_refresh);
        host.apply_expected_transaction(
            buffer,
            refreshed.metadata.revision,
            Transaction::insert(0, "Complete "),
        )
        .unwrap();
        host.save_buffer(buffer).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Complete Merge branch 'dev'\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_activated_wait_enters_normal_mode() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-normal-wait-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("MERGE_MSG");
        std::fs::write(&path, "Merge branch 'dev'\n").unwrap();
        let mut host = host();
        host.execute(HostCommand::Input(InputEvent::Key(KeyStroke::char('i'))))
            .unwrap();
        assert_eq!(host.app.mode, Mode::Insert);

        let (_, buffers) = host.create_wait_request([path], true).unwrap();

        assert_eq!(host.app.active().buffer, buffers[0].index().unwrap());
        assert_eq!(host.app.mode, Mode::Normal);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_wait_never_refreshes_dirty_or_pending_buffer_text() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-protected-wait-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let pending_path = root.join("pending.txt");
        let dirty_path = root.join("dirty.txt");
        std::fs::write(&pending_path, "pending in memory\n").unwrap();
        std::fs::write(&dirty_path, "saved text\n").unwrap();
        let mut host = host();

        let (_, pending_buffers) = host
            .create_wait_request([pending_path.clone()], false)
            .unwrap();
        let pending = pending_buffers[0];
        std::fs::write(&pending_path, "new disk text\n").unwrap();
        let (_, reused_pending) = host.create_wait_request([pending_path], false).unwrap();
        assert_eq!(reused_pending, vec![pending]);
        assert_eq!(
            host.read_buffer(pending).unwrap().text,
            "pending in memory\n"
        );

        let dirty = host.open_buffer(dirty_path.clone(), false).unwrap();
        let revision = host.read_buffer(dirty).unwrap().metadata.revision;
        host.apply_expected_transaction(dirty, revision, Transaction::insert(0, "unsaved "))
            .unwrap();
        std::fs::write(&dirty_path, "new disk text\n").unwrap();
        let (_, reused_dirty) = host.create_wait_request([dirty_path], false).unwrap();
        assert_eq!(reused_dirty, vec![dirty]);
        assert_eq!(
            host.read_buffer(dirty).unwrap().text,
            "unsaved saved text\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_activated_directory_wait_uses_the_panes_reused_explorer() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-directory-identity-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        let first = root.join("first");
        let second = root.join("second");
        let note = root.join("note.txt");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(&note, "alpha").unwrap();
        let mut host = host();

        let explorer = host.open_buffer(first, true).unwrap();
        host.open_buffer(note, true).unwrap();
        assert_eq!(host.app.active().directory_buffer, explorer.index());

        let (token, buffers) = host
            .create_wait_request([second.clone(), second.clone()], true)
            .unwrap();
        assert_eq!(
            buffers,
            vec![explorer],
            "the repeated path was not deduplicated"
        );
        assert_eq!(
            host.app.active().buffer,
            explorer.index().unwrap(),
            "the wait token does not own the explorer the pane activated"
        );
        assert_eq!(
            live_paths(&host)
                .into_iter()
                .filter(|path| path == &second)
                .count(),
            1,
            "activating the directory left a staged duplicate open"
        );

        host.close_buffer(explorer, false).unwrap();
        assert_eq!(host.wait_status(token), Some(WaitStatus::Completed));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_already_open_repeated_directory_wait_uses_the_activated_explorer() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-live-directory-identity-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        let first = root.join("first");
        let second = root.join("second");
        let note = root.join("note.txt");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(&note, "alpha").unwrap();
        let mut host = host();

        let explorer = host.open_buffer(first, true).unwrap();
        host.open_buffer(note, true).unwrap();
        let already_open = host.open_buffer(second.clone(), false).unwrap();
        assert_ne!(explorer, already_open);

        let (token, buffers) = host
            .create_wait_request([second.clone(), second], true)
            .unwrap();
        assert_eq!(buffers, vec![explorer]);
        assert_eq!(host.app.active().buffer, explorer.index().unwrap());

        host.close_buffer(explorer, false).unwrap();
        assert_eq!(host.wait_status(token), Some(WaitStatus::Completed));
        assert!(host.read_buffer(already_open).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_dirty_reused_explorer_rejects_directory_activation_before_commit() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-dirty-directory-identity-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        let first = root.join("first");
        let second = root.join("second");
        let note = root.join("note.txt");
        let later = root.join("later.txt");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(&note, "alpha").unwrap();
        std::fs::write(&later, "beta").unwrap();
        let mut host = host();

        let explorer = host.open_buffer(first, true).unwrap();
        let revision = host.read_buffer(explorer).unwrap().metadata.revision;
        host.apply_expected_transaction(explorer, revision, Transaction::insert(0, "pending/\n"))
            .unwrap();
        let showing = host.open_buffer(note, true).unwrap();
        let live_before = live_paths(&host);

        let error = host
            .open_buffers([second, later.clone()], true)
            .unwrap_err();
        assert!(error.to_string().contains("unsaved edits"), "{error}");
        assert_eq!(host.app.active().buffer, showing.index().unwrap());
        assert_eq!(live_paths(&host), live_before);
        assert!(
            !host
                .buffer_metadata()
                .into_iter()
                .any(|buffer| !buffer.closed && buffer.path.as_deref() == Some(later.as_path())),
            "a later staged file was committed before directory activation was rejected"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The paths of every buffer the host is currently holding open.
    fn live_paths(host: &WorkspaceHost) -> Vec<PathBuf> {
        host.buffer_metadata()
            .into_iter()
            .filter(|buffer| !buffer.closed)
            .filter_map(|buffer| buffer.path)
            .collect()
    }

    #[test]
    fn wait_tokens_survive_independent_buffer_completion() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-wait-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        std::fs::write(&first, "one").unwrap();
        std::fs::write(&second, "two").unwrap();
        let mut host = host();
        let (token, buffers) = host.create_wait_request([first, second], true).unwrap();
        assert_eq!(buffers.len(), 2);
        host.complete_wait_buffer(token, buffers[0]).unwrap();
        assert!(matches!(
            host.wait_status(token),
            Some(WaitStatus::Pending { remaining, .. }) if remaining == vec![buffers[1]]
        ));
        host.close_buffer(buffers[1], false).unwrap();
        assert_eq!(host.wait_status(token), Some(WaitStatus::Completed));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_wait_history_is_bounded_without_exhausting_a_long_lived_host() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-wait-history-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "one").unwrap();
        let mut host = host();
        let mut last = None;
        for _ in 0..(MAX_WAIT_REQUESTS + 32) {
            let (token, buffers) = host.create_wait_request([path.clone()], false).unwrap();
            host.complete_wait_buffer(token, buffers[0]).unwrap();
            last = Some(token);
        }
        assert_eq!(host.wait_requests.len(), MAX_WAIT_REQUESTS);
        assert_eq!(host.wait_status(last.unwrap()), Some(WaitStatus::Completed));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_wait_completion_refuses_unsaved_text() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-dirty-wait-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "base").unwrap();
        let mut host = host();
        let (token, buffers) = host.create_wait_request([path], false).unwrap();
        let revision = host.read_buffer(buffers[0]).unwrap().metadata.revision;
        host.apply_expected_transaction(buffers[0], revision, Transaction::insert(0, "unsaved"))
            .unwrap();
        let error = host
            .complete_wait_buffer(token, buffers[0])
            .unwrap_err()
            .to_string();
        assert!(error.contains("must be saved"), "{error}");
        assert!(matches!(
            host.wait_status(token),
            Some(WaitStatus::Pending { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_is_terminal_and_cannot_be_revived_by_completion() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-cancelled-wait-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "base").unwrap();
        let mut host = host();
        let (token, buffers) = host.create_wait_request([path], false).unwrap();
        host.cancel_wait(token, "cancelled in test").unwrap();
        assert!(host.complete_wait_buffer(token, buffers[0]).is_err());
        assert!(host.complete_wait_request(token).is_err());
        assert_eq!(
            host.wait_status(token),
            Some(WaitStatus::Cancelled {
                reason: "cancelled in test".to_owned()
            })
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn idle_retirement_requires_clean_buffers_and_no_pending_wait() {
        let root = std::env::temp_dir().join(format!(
            "runyte-host-idle-wait-{}-{}",
            std::process::id(),
            unique_test_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("note.txt");
        std::fs::write(&path, "base").unwrap();
        let mut host = host();
        assert!(host.may_retire_idle());

        let file = host.open_buffer(path.clone(), false).unwrap();
        let revision = host.read_buffer(file).unwrap().metadata.revision;
        host.apply_expected_transaction(file, revision, Transaction::insert(0, "unsaved"))
            .unwrap();
        assert_eq!(host.unsaved_buffers(), 1);
        assert!(!host.may_retire_idle());
        host.save_buffer(file).unwrap();
        assert!(host.may_retire_idle());

        let (token, buffers) = host.create_wait_request([path], false).unwrap();
        assert!(!host.may_retire_idle());
        host.complete_wait_buffer(token, buffers[0]).unwrap();
        assert!(host.may_retire_idle());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn live_terminals_are_reported_and_exited_sessions_still_prevent_idle_retirement() {
        use crate::terminal::TerminalRequest;
        use std::ffi::OsString;

        let mut host = host();
        let id = host
            .app_mut()
            .terminals
            .open(
                TerminalRequest {
                    program: OsString::from("/bin/sh"),
                    arguments: vec!["-c".to_owned(), "sleep 30".to_owned()],
                    directory: std::env::temp_dir(),
                    label: "sleep".to_owned(),
                },
                80,
                24,
            )
            .unwrap();
        assert_eq!(host.protected_state().live_terminals, 1);
        assert!(!host.may_retire_idle());
        assert!(host.protected_state().refusal().contains("1 live terminal"));

        host.app_mut()
            .apply_terminal_output(crate::terminal::TerminalOutput::Exited { id, code: Some(0) });
        assert_eq!(host.protected_state().live_terminals, 0);
        assert!(host.app().terminals.get(id).is_some());
        assert!(!host.may_retire_idle());
    }

    #[test]
    fn an_edited_scratch_buffer_leaves_the_workspace_clean() {
        let mut host = host();
        let scratch = host
            .app()
            .buffers
            .iter()
            .position(|buffer| matches!(buffer.kind, BufferKind::Scratch))
            .expect("a host starts with a scratch buffer");
        host.app_mut().buffers[scratch].apply(&Transaction::insert(0, "a note to self"));

        // Still dirty, so the pane keeps its `[+]`; the workspace is clean all
        // the same, because nothing about the scratchpad can be saved in place.
        assert!(host.app().buffers[scratch].dirty);
        assert!(host.buffer_metadata()[scratch].dirty);
        assert_eq!(host.unsaved_buffers(), 0);
        assert!(host.may_retire_idle());
    }
}
