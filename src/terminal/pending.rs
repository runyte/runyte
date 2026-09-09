// SPDX-License-Identifier: MPL-2.0

//! Bounded, unpublished PTYs. Readers wait until native ownership is installed.
use super::*;
use std::{
    io,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const PENDING_TERMINAL_CHARGE: usize = 4 * 1024 * 1024;
const MAX_PENDING: usize = 8;
const MAX_CELLS: usize = 32768;

#[cfg(test)]
pub(crate) fn pending_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static TESTS: Mutex<()> = Mutex::new(());
    TESTS.lock().unwrap_or_else(|error| error.into_inner())
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalCancellation {
    id: TerminalId,
    events: TerminalEventSender,
    cancelled: Arc<AtomicBool>,
}
impl TerminalCancellation {
    pub(crate) fn cancel(&self) {
        let mut state = self
            .events
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Once installed, this is an ordinary native terminal. A late owner
        // cancellation must not detach its queue or terminate its child.
        if state
            .sessions
            .get(&self.id)
            .is_some_and(|pending| pending.active)
        {
            return;
        }
        self.cancelled.store(true, Ordering::Release);
        state.sessions.remove(&self.id);
        state.ready.retain(|id| *id != self.id);
        self.events.0.space.notify_all();
    }
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    fn is_pending(&self) -> bool {
        !self.is_cancelled()
            && self
                .events
                .0
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sessions
                .get(&self.id)
                .is_some_and(|state| !state.active)
    }
}

struct Reservation {
    cancellation: TerminalCancellation,
    budget: Arc<Budget>,
}
struct Budget {
    _permit: OwnedSemaphorePermit,
    lifetime: Mutex<Option<Box<dyn Send>>>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

struct Cleanup {
    // Field order keeps the permit until existing PTY cleanup has reaped.
    _session: TerminalSession,
    _reservation: Reservation,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(child) = self._session.pty.as_mut() {
            child.terminate_unpublished();
        }
    }
}
fn cleanup_sender() -> io::Result<mpsc::SyncSender<Cleanup>> {
    static CLEANUP: OnceLock<Result<mpsc::SyncSender<Cleanup>, ()>> = OnceLock::new();
    CLEANUP
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Cleanup>(MAX_PENDING);
            std::thread::Builder::new()
                .name("terminal-handoff-reap".into())
                .spawn(move || {
                    while let Ok(cleanup) = receiver.recv() {
                        drop(cleanup);
                    }
                })
                .map(|_| sender)
                .map_err(|_| ())
        })
        .clone()
        .map_err(|_| io::Error::other("Terminal cleanup is unavailable"))
}

pub(crate) struct TerminalPreparation {
    pub(crate) request: TerminalRequest,
    reservation: Reservation,
    columns: usize,
    rows: usize,
    colors: DefaultColors,
    parent_context: Option<String>,
    cleanup: mpsc::SyncSender<Cleanup>,
}
impl std::fmt::Debug for TerminalPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalPreparation")
            .field("id", &self.reservation.cancellation.id)
            .finish_non_exhaustive()
    }
}
impl TerminalPreparation {
    /// Host accounting follows unpublished ownership, including cleanup after
    /// a stale result. Installed terminals return to native terminal budgets.
    pub(crate) fn retain_until_settled(&mut self, guard: Box<dyn Send>) {
        let mut lifetime = self
            .reservation
            .budget
            .lifetime
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(lifetime.is_none(), "one terminal accounting lease");
        *lifetime = Some(guard);
    }
    pub(crate) fn cancellation(&self) -> TerminalCancellation {
        self.reservation.cancellation.clone()
    }
    /// Runs on a bounded blocking worker, after the caller validates cwd.
    pub(crate) fn spawn(self) -> io::Result<PendingTerminal> {
        if !self.reservation.cancellation.is_pending() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Terminal handoff was cancelled",
            ));
        }
        let id = self.reservation.cancellation.id;
        let events = self.reservation.cancellation.events.clone();
        let gate = events.clone();
        let child = pty::Pty::spawn_gated_in_context(
            &self.request.program,
            &self.request.arguments,
            &self.request.directory,
            self.columns as u16,
            self.rows as u16,
            self.parent_context.as_deref(),
            move |event| {
                let output = match event {
                    pty::PtyEvent::Output(bytes) => TerminalOutput::Bytes { id, bytes },
                    pty::PtyEvent::Exited(code) => TerminalOutput::Exited { id, code },
                };
                let _ = events.send(output);
            },
            pty::PendingActivation {
                wait: Arc::new(move || gate.wait_until_active(id)),
                lifetime: self.reservation.budget.clone(),
            },
        )?;
        let mut emulator = Emulator::new(self.columns, self.rows);
        emulator.set_default_colors(self.colors);
        let now = SystemTime::now();
        let pending = PendingTerminal {
            reservation: Some(self.reservation),
            cleanup: self.cleanup,
            session: Some(TerminalSession {
                id,
                label: self.request.label,
                user_name: None,
                directory: self.request.directory.clone(),
                initial_directory: self.request.directory,
                reported_directory: None,
                created_at: now,
                last_activity: now,
                last_completed_line_activity: now,
                unread_activity: false,
                bell: false,
                history_truncated: false,
                content_revision: 1,
                review: None,
                emulator,
                pty: Some(child),
                exit: None,
                sent_text: None,
                scroll: 0,
                revision: 1,
            }),
        };
        if !pending
            .reservation
            .as_ref()
            .unwrap()
            .cancellation
            .is_pending()
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Terminal handoff was cancelled",
            ));
        }
        Ok(pending)
    }
}

pub(crate) struct PendingTerminal {
    reservation: Option<Reservation>,
    session: Option<TerminalSession>,
    cleanup: mpsc::SyncSender<Cleanup>,
}
impl std::fmt::Debug for PendingTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingTerminal")
            .field("id", &self.session.as_ref().map(|s| s.id))
            .finish_non_exhaustive()
    }
}
impl Drop for PendingTerminal {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            reservation.cancellation.cancel();
            if let Some(session) = self.session.take() {
                // Signal while still on the caller's thread: immediate editor
                // exit must not outrun the background reaper's first wakeup.
                if let Some(child) = session.pty.as_ref() {
                    child.signal_unpublished();
                }
                // Each queued cleanup retains one of eight reservations. This
                // additional reservation proves the queue has a free slot.
                self.cleanup
                    .try_send(Cleanup {
                        _session: session,
                        _reservation: reservation,
                    })
                    .unwrap_or_else(|_| unreachable!("reserved terminal cleanup capacity"));
            }
        }
    }
}

impl TerminalSessions {
    pub(crate) fn prepare_open(
        &mut self,
        request: TerminalRequest,
        columns: usize,
        rows: usize,
    ) -> io::Result<TerminalPreparation> {
        let (columns, rows) = (columns.max(1), rows.max(1));
        let cells = columns.saturating_mul(rows);
        // Two grids plus conservative per-line storage, tab stops, parser and
        // a held reader chunk. Tall, narrow panes must count line allocations.
        let allocation = cells
            .saturating_mul(2 * std::mem::size_of::<Cell>())
            .saturating_add(rows.saturating_mul(256))
            .saturating_add(columns.saturating_mul(16))
            .saturating_add(256 * 1024);
        if columns > u16::MAX as usize
            || rows > u16::MAX as usize
            || cells > MAX_CELLS
            || allocation > PENDING_TERMINAL_CHARGE
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Terminal pane exceeds handoff geometry limit",
            ));
        }
        static PENDING: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let permit = PENDING
            .get_or_init(|| Arc::new(Semaphore::new(MAX_PENDING)))
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "Terminal handoff limit reached")
            })?;
        let cleanup = cleanup_sender()?;
        let id = TerminalId(self.next);
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Terminal identities exhausted"))?;
        self.events
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sessions
            .insert(id, PendingOutput::default());
        Ok(TerminalPreparation {
            request,
            reservation: Reservation {
                cancellation: TerminalCancellation {
                    id,
                    events: self.events.clone(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                },
                budget: Arc::new(Budget {
                    _permit: permit,
                    lifetime: Mutex::new(None),
                }),
            },
            columns,
            rows,
            colors: self.default_colors,
            parent_context: self.parent_launch.as_ref().map(|launch| launch.context(id)),
            cleanup,
        })
    }
    pub(crate) fn install_prepared(
        &mut self,
        mut pending: PendingTerminal,
    ) -> io::Result<TerminalId> {
        let cancellation = pending
            .reservation
            .as_ref()
            .expect("pending reservation")
            .cancellation
            .clone();
        if !Arc::ptr_eq(&self.events.0, &cancellation.events.0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Terminal belongs to another workspace",
            ));
        }
        let mut state = self
            .events
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cancellation.is_cancelled() || !state.sessions.contains_key(&cancellation.id) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Terminal handoff was cancelled",
            ));
        }
        let mut session = pending.session.take().expect("pending session");
        session.emulator.set_default_colors(self.default_colors);
        self.sessions.insert(cancellation.id, session);
        state.sessions.get_mut(&cancellation.id).unwrap().active = true;
        drop(state);
        self.events.0.space.notify_all();
        // Dropping its reservation now observes active native ownership.
        pending.reservation.take();
        Ok(cancellation.id)
    }
}

#[cfg(test)]
mod tests;
