// SPDX-License-Identifier: MPL-2.0

//! Process-wide FIFO ordering between editor Git operations.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Condvar, Mutex, OnceLock, atomic::AtomicBool, atomic::Ordering},
    time::Duration,
};

#[derive(Default)]
struct RepositoryState {
    active: bool,
    queue: VecDeque<u64>,
}

#[derive(Default)]
struct LockState {
    next_ticket: u64,
    repositories: HashMap<PathBuf, RepositoryState>,
}

fn state() -> &'static (Mutex<LockState>, Condvar) {
    static STATE: OnceLock<(Mutex<LockState>, Condvar)> = OnceLock::new();
    STATE.get_or_init(|| (Mutex::new(LockState::default()), Condvar::new()))
}

thread_local! {
    static HELD: RefCell<HashMap<PathBuf, usize>> = RefCell::new(HashMap::new());
}

/// A FIFO place allocated when a service accepts a request, before its worker
/// can race another service's worker to the repository.
pub(crate) struct RepositoryReservation {
    common_directory: PathBuf,
    ticket: Option<u64>,
    consumed: bool,
}

impl RepositoryReservation {
    /// Waits for this reservation, abandoning it promptly when cancellation is
    /// requested. `None` proves that no repository operation began.
    pub(crate) fn acquire(mut self, cancellation: Option<&AtomicBool>) -> Option<RepositoryGuard> {
        if self.ticket.is_none() {
            HELD.with(|held| {
                *held
                    .borrow_mut()
                    .entry(self.common_directory.clone())
                    .or_default() += 1;
            });
            self.consumed = true;
            return Some(RepositoryGuard {
                common_directory: self.common_directory.clone(),
                owns_repository: false,
            });
        }
        let ticket = self.ticket.expect("non-reentrant reservation has a ticket");
        let (state, changed) = state();
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                remove_waiter(&mut state, &self.common_directory, ticket);
                self.consumed = true;
                changed.notify_all();
                return None;
            }
            let repository = state
                .repositories
                .get_mut(&self.common_directory)
                .expect("reserved repository state exists");
            if !repository.active && repository.queue.front() == Some(&ticket) {
                repository.queue.pop_front();
                repository.active = true;
                HELD.with(|held| {
                    *held
                        .borrow_mut()
                        .entry(self.common_directory.clone())
                        .or_default() += 1;
                });
                self.consumed = true;
                return Some(RepositoryGuard {
                    common_directory: self.common_directory.clone(),
                    owns_repository: true,
                });
            }
            let waited = changed
                .wait_timeout(state, Duration::from_millis(10))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = waited.0;
        }
    }
}

impl Drop for RepositoryReservation {
    fn drop(&mut self) {
        let Some(ticket) = self.ticket.filter(|_| !self.consumed) else {
            return;
        };
        let (state, changed) = state();
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        remove_waiter(&mut state, &self.common_directory, ticket);
        changed.notify_all();
    }
}

pub(crate) struct RepositoryGuard {
    common_directory: PathBuf,
    owns_repository: bool,
}

impl Drop for RepositoryGuard {
    fn drop(&mut self) {
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            if let Some(depth) = held.get_mut(&self.common_directory) {
                *depth -= 1;
                if *depth == 0 {
                    held.remove(&self.common_directory);
                }
            }
        });
        if !self.owns_repository {
            return;
        }
        let (state, changed) = state();
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let remove = if let Some(repository) = state.repositories.get_mut(&self.common_directory) {
            repository.active = false;
            repository.queue.is_empty()
        } else {
            false
        };
        if remove {
            state.repositories.remove(&self.common_directory);
        }
        changed.notify_all();
    }
}

pub(crate) fn reserve(common_directory: &Path) -> RepositoryReservation {
    let common_directory = common_directory.to_path_buf();
    let reentrant = HELD.with(|held| held.borrow().contains_key(&common_directory));
    if reentrant {
        return RepositoryReservation {
            common_directory,
            ticket: None,
            consumed: false,
        };
    }
    let (state, _) = state();
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let ticket = state.next_ticket;
    state.next_ticket = state.next_ticket.wrapping_add(1);
    state
        .repositories
        .entry(common_directory.clone())
        .or_default()
        .queue
        .push_back(ticket);
    RepositoryReservation {
        common_directory,
        ticket: Some(ticket),
        consumed: false,
    }
}

#[cfg(test)]
pub(crate) fn acquire(common_directory: &Path) -> RepositoryGuard {
    reserve(common_directory)
        .acquire(None)
        .expect("an uncancelled repository reservation is acquired")
}

fn remove_waiter(state: &mut LockState, common_directory: &Path, ticket: u64) {
    let mut remove = false;
    if let Some(repository) = state.repositories.get_mut(common_directory)
        && let Some(position) = repository.queue.iter().position(|queued| *queued == ticket)
    {
        repository.queue.remove(position);
        remove = !repository.active && repository.queue.is_empty();
    }
    if remove {
        state.repositories.remove(common_directory);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, atomic::AtomicBool, mpsc},
        thread,
        time::Duration,
    };

    use super::*;

    #[test]
    fn reservations_are_fifo_even_when_workers_start_out_of_order() {
        let repository = PathBuf::from("/fifo/.git");
        let first = reserve(&repository);
        let second = reserve(&repository);
        let third = reserve(&repository);
        let (entered_tx, entered_rx) = mpsc::channel();
        let third_tx = entered_tx.clone();
        let third_thread = thread::spawn(move || {
            let _guard = third.acquire(None).unwrap();
            third_tx.send(3).unwrap();
        });
        let second_tx = entered_tx.clone();
        let second_thread = thread::spawn(move || {
            let _guard = second.acquire(None).unwrap();
            second_tx.send(2).unwrap();
        });
        thread::sleep(Duration::from_millis(20));
        let first_guard = first.acquire(None).unwrap();
        entered_tx.send(1).unwrap();
        assert_eq!(entered_rx.recv().unwrap(), 1);
        drop(first_guard);
        assert_eq!(entered_rx.recv().unwrap(), 2);
        second_thread.join().unwrap();
        assert_eq!(entered_rx.recv().unwrap(), 3);
        third_thread.join().unwrap();
    }

    #[test]
    fn a_cancelled_waiter_leaves_the_fifo_without_entering() {
        let repository = PathBuf::from("/cancel/.git");
        let holder = reserve(&repository).acquire(None).unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = Arc::clone(&cancellation);
        let worker_repository = repository.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let waiting = reserve(&worker_repository);
            ready_tx.send(()).unwrap();
            waiting.acquire(Some(&worker_cancellation))
        });
        ready_rx.recv().unwrap();
        cancellation.store(true, Ordering::Release);
        assert!(worker.join().unwrap().is_none());
        drop(holder);
        assert!(reserve(&repository).acquire(None).is_some());
    }

    #[test]
    fn nested_repository_authority_is_reentrant_on_one_worker() {
        let repository = PathBuf::from("/reentrant/.git");
        let outer = reserve(&repository).acquire(None).unwrap();
        let inner = reserve(&repository).acquire(None).unwrap();
        drop(inner);
        drop(outer);
    }

    #[test]
    fn an_idle_repository_does_not_leave_lock_state_behind() {
        let repository = PathBuf::from(format!(
            "/released-{}/.git",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let guard = reserve(&repository).acquire(None).unwrap();
        drop(guard);

        let (state, _) = state();
        let state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(!state.repositories.contains_key(&repository));
    }
}
