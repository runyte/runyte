// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Host-lifetime identity of one service request.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ServiceRequestId(u64);

impl ServiceRequestId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Shared scheduler class. Domain operations and results stay in their own
/// modules instead of being flattened into one serialized mega-enum.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ServiceKind {
    Git,
    Lsp,
    FileScan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServicePhase {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    CompletedWithUncertainState,
}

impl ServicePhase {
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::CompletedWithUncertainState
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceOutcome {
    Completed,
    Failed(String),
    Cancelled,
    CompletedWithUncertainState(String),
}

impl ServiceOutcome {
    const fn phase(&self) -> ServicePhase {
        match self {
            Self::Completed => ServicePhase::Completed,
            Self::Failed(_) => ServicePhase::Failed,
            Self::Cancelled => ServicePhase::Cancelled,
            Self::CompletedWithUncertainState(_) => ServicePhase::CompletedWithUncertainState,
        }
    }
}

/// Cooperative cancellation observed by a worker it was explicitly given to.
/// Dropping a request does not imply rollback and does not mutate editor state.
#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// One concrete typed service implementation. Requests and returned events
/// remain domain values; this is intentionally not a boxed closure API.
pub trait ServiceWorker: Send + 'static {
    type Request: Send + 'static;
    type Event: Send + 'static;

    fn execute(
        &mut self,
        request: Self::Request,
        cancellation: &CancellationToken,
    ) -> (Option<Self::Event>, ServiceOutcome);
}

#[derive(Debug)]
struct ServiceTask<R> {
    id: ServiceRequestId,
    request: R,
    cancellation: CancellationToken,
}

/// Progress plus an optional owned domain event returned by a worker.
#[derive(Debug)]
pub enum ServiceUpdate<E> {
    Started(ServiceRequestId),
    Finished {
        id: ServiceRequestId,
        event: Option<E>,
        outcome: ServiceOutcome,
    },
}

/// A bounded typed background lane. The host allocates identities and applies
/// these updates; the worker thread can never reach live editor state.
pub struct ServiceLane<W: ServiceWorker> {
    requests: SyncSender<ServiceTask<W::Request>>,
    updates: Receiver<ServiceUpdate<W::Event>>,
}

impl<W: ServiceWorker> ServiceLane<W> {
    pub fn spawn(mut worker: W, capacity: usize) -> Self {
        assert!(capacity > 0, "service lane capacity must be non-zero");
        let (requests, request_rx) = sync_channel::<ServiceTask<W::Request>>(capacity);
        let (update_tx, updates) = sync_channel(capacity.saturating_mul(2));
        std::thread::spawn(move || {
            while let Ok(task) = request_rx.recv() {
                if task.cancellation.is_cancelled() {
                    if update_tx
                        .send(ServiceUpdate::Finished {
                            id: task.id,
                            event: None,
                            outcome: ServiceOutcome::Cancelled,
                        })
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                if update_tx.send(ServiceUpdate::Started(task.id)).is_err() {
                    break;
                }
                let (event, outcome) = worker.execute(task.request, &task.cancellation);
                if update_tx
                    .send(ServiceUpdate::Finished {
                        id: task.id,
                        event,
                        outcome,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self { requests, updates }
    }

    pub fn try_submit(
        &self,
        id: ServiceRequestId,
        request: W::Request,
        cancellation: CancellationToken,
    ) -> Result<(), ServiceSubmitError> {
        self.requests
            .try_send(ServiceTask {
                id,
                request,
                cancellation,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => ServiceSubmitError::Full,
                TrySendError::Disconnected(_) => ServiceSubmitError::Stopped,
            })
    }

    pub fn try_recv(&self) -> Result<ServiceUpdate<W::Event>, TryRecvError> {
        self.updates.try_recv()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceSubmitError {
    Full,
    Stopped,
}

impl fmt::Display for ServiceSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => formatter.write_str("service lane is full"),
            Self::Stopped => formatter.write_str("service lane has stopped"),
        }
    }
}

impl std::error::Error for ServiceSubmitError {}

#[derive(Clone, Debug)]
pub struct ServiceProgress {
    pub id: ServiceRequestId,
    pub kind: ServiceKind,
    pub operation: String,
    pub target: String,
    pub phase: ServicePhase,
    pub cancellable: bool,
    pub queued_at: Instant,
    pub started_at: Option<Instant>,
    pub finished_at: Option<Instant>,
    pub outcome: Option<ServiceOutcome>,
}

impl ServiceProgress {
    pub fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started_at.unwrap_or(self.queued_at))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceStateError {
    QueueFull {
        limit: usize,
    },
    Unknown(ServiceRequestId),
    AlreadyTerminal(ServiceRequestId),
    InvalidTransition {
        id: ServiceRequestId,
        from: ServicePhase,
        to: ServicePhase,
    },
    NotCancellable(ServiceRequestId),
}

impl fmt::Display for ServiceStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull { limit } => write!(formatter, "service queue is full ({limit})"),
            Self::Unknown(id) => write!(formatter, "unknown service request {}", id.get()),
            Self::AlreadyTerminal(id) => {
                write!(formatter, "service request {} already finished", id.get())
            }
            Self::InvalidTransition { id, from, to } => write!(
                formatter,
                "service request {} cannot move from {from:?} to {to:?}",
                id.get()
            ),
            Self::NotCancellable(id) => {
                write!(
                    formatter,
                    "service request {} cannot be cancelled",
                    id.get()
                )
            }
        }
    }
}

impl std::error::Error for ServiceStateError {}

#[derive(Clone, Debug)]
struct Entry {
    progress: ServiceProgress,
    cancellation: CancellationToken,
}

/// Bounded host-owned lifecycle table shared by concrete service schedulers.
///
/// Workers receive only their domain request and a cancellation token. They
/// return owned events; this value never grants them access to `App`.
#[derive(Debug)]
pub struct ServiceLifecycle {
    limit: usize,
    next_id: u64,
    entries: HashMap<ServiceRequestId, Entry>,
    terminal_order: VecDeque<ServiceRequestId>,
}

impl ServiceLifecycle {
    pub fn new(limit: usize) -> Self {
        assert!(limit > 0, "service lifecycle limit must be non-zero");
        Self {
            limit,
            next_id: 1,
            entries: HashMap::new(),
            terminal_order: VecDeque::new(),
        }
    }

    pub fn queue(
        &mut self,
        kind: ServiceKind,
        operation: impl Into<String>,
        target: impl Into<String>,
        cancellable: bool,
    ) -> Result<(ServiceRequestId, CancellationToken), ServiceStateError> {
        let live = self
            .entries
            .values()
            .filter(|entry| !entry.progress.phase.terminal())
            .count();
        if live >= self.limit {
            return Err(ServiceStateError::QueueFull { limit: self.limit });
        }
        let id = ServiceRequestId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("service request identity exhausted");
        let cancellation = CancellationToken::new();
        self.entries.insert(
            id,
            Entry {
                progress: ServiceProgress {
                    id,
                    kind,
                    operation: operation.into(),
                    target: target.into(),
                    phase: ServicePhase::Queued,
                    cancellable,
                    queued_at: Instant::now(),
                    started_at: None,
                    finished_at: None,
                    outcome: None,
                },
                cancellation: cancellation.clone(),
            },
        );
        Ok((id, cancellation))
    }

    pub fn start(&mut self, id: ServiceRequestId) -> Result<(), ServiceStateError> {
        let entry = self.entry_mut(id)?;
        if entry.progress.phase != ServicePhase::Queued {
            return Err(ServiceStateError::InvalidTransition {
                id,
                from: entry.progress.phase,
                to: ServicePhase::Running,
            });
        }
        entry.progress.phase = ServicePhase::Running;
        entry.progress.started_at = Some(Instant::now());
        Ok(())
    }

    pub fn cancel(&mut self, id: ServiceRequestId) -> Result<(), ServiceStateError> {
        let entry = self.entry_mut(id)?;
        if entry.progress.phase.terminal() {
            return Err(ServiceStateError::AlreadyTerminal(id));
        }
        if !entry.progress.cancellable {
            return Err(ServiceStateError::NotCancellable(id));
        }
        entry.cancellation.cancel();
        Ok(())
    }

    pub fn finish(
        &mut self,
        id: ServiceRequestId,
        outcome: ServiceOutcome,
    ) -> Result<(), ServiceStateError> {
        let entry = self.entry_mut(id)?;
        if entry.progress.phase.terminal() {
            return Err(ServiceStateError::AlreadyTerminal(id));
        }
        entry.progress.phase = outcome.phase();
        entry.progress.finished_at = Some(Instant::now());
        entry.progress.outcome = Some(outcome);
        self.terminal_order.push_back(id);
        while self.terminal_order.len() > self.limit {
            if let Some(retired) = self.terminal_order.pop_front() {
                self.entries.remove(&retired);
            }
        }
        Ok(())
    }

    pub fn progress(&self, id: ServiceRequestId) -> Option<&ServiceProgress> {
        self.entries.get(&id).map(|entry| &entry.progress)
    }

    pub fn active(&self) -> impl Iterator<Item = &ServiceProgress> {
        self.entries
            .values()
            .filter(|entry| !entry.progress.phase.terminal())
            .map(|entry| &entry.progress)
    }

    pub fn prune_terminal(&mut self) {
        self.entries
            .retain(|_, entry| !entry.progress.phase.terminal());
        self.terminal_order.clear();
    }

    fn entry_mut(&mut self, id: ServiceRequestId) -> Result<&mut Entry, ServiceStateError> {
        self.entries
            .get_mut(&id)
            .ok_or(ServiceStateError::Unknown(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_non_reused_and_the_live_queue_is_bounded() {
        let mut lifecycle = ServiceLifecycle::new(1);
        let (first, _) = lifecycle
            .queue(ServiceKind::FileScan, "scan", "/project", true)
            .unwrap();
        assert!(matches!(
            lifecycle.queue(ServiceKind::Lsp, "symbols", "main.rs", true),
            Err(ServiceStateError::QueueFull { limit: 1 })
        ));
        lifecycle.start(first).unwrap();
        lifecycle.finish(first, ServiceOutcome::Completed).unwrap();
        let (second, _) = lifecycle
            .queue(ServiceKind::Lsp, "symbols", "main.rs", true)
            .unwrap();
        assert!(second > first);
    }

    #[test]
    fn cancellation_is_cooperative_and_never_claims_a_terminal_outcome() {
        let mut lifecycle = ServiceLifecycle::new(2);
        let (id, cancellation) = lifecycle
            .queue(ServiceKind::Git, "commit", "/project", true)
            .unwrap();
        lifecycle.start(id).unwrap();
        lifecycle.cancel(id).unwrap();
        assert!(cancellation.is_cancelled());
        assert_eq!(
            lifecycle.progress(id).unwrap().phase,
            ServicePhase::Running,
            "the worker still decides whether state is uncertain"
        );
        lifecycle
            .finish(
                id,
                ServiceOutcome::CompletedWithUncertainState(
                    "commit hook may have completed".to_owned(),
                ),
            )
            .unwrap();
        assert_eq!(
            lifecycle.progress(id).unwrap().phase,
            ServicePhase::CompletedWithUncertainState
        );
    }

    #[test]
    fn invalid_transitions_and_non_cancellable_work_are_typed() {
        let mut lifecycle = ServiceLifecycle::new(2);
        let (id, _) = lifecycle
            .queue(ServiceKind::Git, "update index", "repository", false)
            .unwrap();
        assert_eq!(
            lifecycle.cancel(id),
            Err(ServiceStateError::NotCancellable(id))
        );
        lifecycle.start(id).unwrap();
        assert!(matches!(
            lifecycle.start(id),
            Err(ServiceStateError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn terminal_history_is_bounded_without_reusing_identities() {
        let mut lifecycle = ServiceLifecycle::new(2);
        let mut ids = Vec::new();
        for _ in 0..4 {
            let (id, _) = lifecycle
                .queue(ServiceKind::Git, "status", "/project", true)
                .unwrap();
            lifecycle.start(id).unwrap();
            lifecycle.finish(id, ServiceOutcome::Completed).unwrap();
            ids.push(id);
        }
        assert!(lifecycle.progress(ids[0]).is_none());
        assert!(lifecycle.progress(ids[1]).is_none());
        assert!(lifecycle.progress(ids[2]).is_some());
        assert!(lifecycle.progress(ids[3]).is_some());
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
